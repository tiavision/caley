// @vitest-environment node
// Real openpgp needs node (see src/lib/pgp/sign-verify.test.ts).
// body() on encrypted and signed mail against a server that cuts large attachments out (here sinter-mail's reference
// cutter, mail.cut; backend/src/email/cut.rs follows its rules). A signed mail is shown from the cut message at once,
// marked signed, its signature checked when the reader asks (verify()). Every message is fetched cut first: an
// armored ciphertext is never cut, so it comes whole in that request; a cut (base64) one is never decrypted from the
// cut message: the reader fetches the whole message at once.
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import * as openpgp from 'openpgp';
import * as mail from '@tiavision/sinter-mail';
import type { CallContext } from '@tiaforge-ui/sinter';
import { body, openPart, verify, wipe } from './mail.ts';
import { buildProtectedMime, type ProtectedComposeMessage } from '../lib/pgp/compose.ts';
import { lock, openVault } from '../lib/pgp/session.ts';
import type { Message } from '../lib/mail.ts';

const keys = (email: string) => openpgp.generateKey({ userIDs: [{ name: email.split('@')[0], email }], format: 'object' });
const pdf = () => new File([new Uint8Array(300_000).fill(0x25)], 'report.pdf', { type: 'application/pdf' });

function draft(over: Partial<ProtectedComposeMessage>): ProtectedComposeMessage {
  return {
    identity: { name: 'Alice', email: 'alice@example.com' }, to: ['bob@example.com'], cc: [], subject: 'Report',
    body: 'The report is attached.', inReplyTo: null, references: null, attachments: [pdf()],
    encrypt: true, sign: false, encryptionKeys: [], signingKey: null, ...over,
  } as ProtectedComposeMessage;
}

/** PGP/MIME as some clients send it: the ciphertext binary, base64-encoded (so it can be cut, unlike armor). */
async function binaryEncrypted(to: openpgp.PublicKey): Promise<Uint8Array> {
  const inner = ['Content-Type: multipart/mixed; boundary="i"', '', '--i', 'Content-Type: text/plain; charset=utf-8', '',
    'The report is attached.', '--i', 'Content-Type: application/pdf; name="report.pdf"',
    'Content-Disposition: attachment; filename="report.pdf"', 'Content-Transfer-Encoding: base64', '',
    Buffer.from(new Uint8Array(300_000).fill(0x25)).toString('base64').replace(/.{76}/g, '$&\r\n'), '--i--', ''].join('\r\n');
  const ciphertext = await openpgp.encrypt({ message: await openpgp.createMessage({ text: inner }), encryptionKeys: to, format: 'binary' });
  return Uint8Array.from(Buffer.from([
    'From: Alice <alice@example.com>', 'To: bob@example.com', 'Subject: ...', 'MIME-Version: 1.0',
    'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary="e"', '',
    '--e', 'Content-Type: application/pgp-encrypted', '', 'Version: 1', '',
    '--e', 'Content-Type: application/octet-stream; name="encrypted.pgp"', 'Content-Transfer-Encoding: base64', '',
    Buffer.from(ciphertext as Uint8Array).toString('base64').replace(/.{76}/g, '$&\r\n'), '--e--', '',
  ].join('\r\n')));
}

let whole = new Uint8Array();
let flags = { is_encrypted: false, is_signed: false };
let trusted: { fingerprint: string; email: string; armored: string; is_own: boolean; source: string; trust: string }[] = [];
let calls: string[] = [];
let uid = 100;
let cutOptions: { rootCiphertext?: boolean } = {};   // the stand-in server's mail.cut options
let serverFrom = 'Alice <alice@example.com>';        // the From the stand-in server reports (Message.from)

beforeEach(() => {
  calls = [];
  trusted = [];
  cutOptions = {};
  serverFrom = 'Alice <alice@example.com>';
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
    const range = new Headers(init?.headers).get('Range');
    calls.push(range ? `${url} ${range}` : url);
    if (url.startsWith('/api/email/message?')) {
      return Response.json({
        uid: Number(new URL(url, 'http://x').searchParams.get('uid')), account_id: 'work', mailbox: 'INBOX', rowid: 1,
        message_id: null, thread_id: null, from: serverFrom, to: ['bob@example.com'], cc: [], subject: '...',
        sent_at: 1, flags: [], has_attachments: true, body_cached: true, is_read: true, is_flagged: false, attachments: [], ...flags,
      });
    }
    if (url === '/api/pgp/keys') return Response.json(trusted);
    if (!url.startsWith('/api/email/message/raw?')) return new Response('', { status: 404 });
    if (range) {
      const [, a, b] = /bytes=(\d+)-(\d+)/.exec(range)!;
      return new Response(whole.slice(Number(a), Number(b) + 1), { status: 206 });
    }
    if (!url.includes('omit=')) return new Response(whole, { headers: { ETag: '"e"' } });
    const cut = await mail.cut(whole, cutOptions);
    return new Response(cut.raw, { headers: { ETag: '"e"', 'Mail-Holes': JSON.stringify(cut.holes) } });
  });
});
afterEach(() => { wipe(); lock(); vi.unstubAllGlobals(); });

const ctx = () => ({ signal: new AbortController().signal, request: (url: string) => fetch(url).then((r) => r.json()) }) as unknown as CallContext;
const read = () => body({ account_id: 'work', mailbox: 'INBOX', uid: ++uid }, ctx());
const again = () => body({ account_id: 'work', mailbox: 'INBOX', uid }, ctx());   // the reader's refresh(body)
const ref = () => ({ account_id: 'work', mailbox: 'INBOX', uid });
const raws = () => calls.filter((c) => c.startsWith('/api/email/message/raw')).map((c) => (c.includes('bytes=') ? 'range' : c.includes('omit=') ? 'cut' : 'whole'));
async function unlockAs(key: openpgp.PrivateKey) {
  openVault(await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt']), new Map([[key.getFingerprint(), key]]));
}

test('caley\'s encrypted mail (armored) is never cut: one request, decrypted with its attachment', async () => {
  const bob = await keys('bob@example.com');
  whole = await buildProtectedMime(draft({ encryptionKeys: [bob.publicKey] }));
  expect((await mail.cut(whole)).holes).toEqual([]);                     // armor's lines start with `--`
  await unlockAs(bob.privateKey);
  flags = { is_encrypted: true, is_signed: false };
  const b = await read();
  expect(raws()).toEqual(['cut']);
  expect(b.parts.map((p) => [p.kind, p.text.trim()])).toEqual([['text', 'The report is attached.']]);
  expect(b.files.map((f) => [f.filename, f.size])).toEqual([['report.pdf', 300_000]]);
  expect(b.badges).toContain('encrypted');
});

test('a root base64 ciphertext a server cut anyway: the decrypt step is incomplete, so the reader fetches it whole at once and decrypts', async () => {
  const bob = await keys('bob@example.com');
  whole = await binaryEncrypted(bob.publicKey);
  expect((await mail.cut(whole)).holes).toEqual([]);                    // a cutter leaves the root's ciphertext whole (as cut.rs)
  cutOptions = { rootCiphertext: true };                                // a server that cuts it anyway
  expect((await mail.cut(whole, cutOptions)).holes).toHaveLength(1);
  await unlockAs(bob.privateKey);
  flags = { is_encrypted: true, is_signed: false };
  const b = await read();
  expect(raws()).toEqual(['cut', 'whole']);                             // at once, never a range of the ciphertext
  expect(b.parts.map((p) => [p.kind, p.text.trim()])).toEqual([['text', 'The report is attached.']]);
  expect(b.files.map((f) => f.filename)).toEqual(['report.pdf']);
  expect(b.error).toBe('');
});

test('a signed mail whose attachment the server cut: shown at once, marked signed, verified when the reader asks', async () => {
  const alice = await keys('alice@example.com');
  whole = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey }));
  expect((await mail.cut(whole)).holes).toHaveLength(1);                 // the PDF, under the signature
  trusted = [{ fingerprint: alice.publicKey.getFingerprint(), email: 'alice@example.com', armored: alice.publicKey.armor(), is_own: false, source: 'manual', trust: 'trusted' }];
  flags = { is_encrypted: false, is_signed: true };
  const b = await read();
  expect(raws()).toEqual(['cut']);                                      // shown without downloading the PDF
  expect(b.parts.map((p) => p.text.trim())).toEqual(['The report is attached.']);
  expect(b.pending).toBe(true);
  expect(b.badges).toEqual(['signed']);
  expect(b.wholeSize).toBe(whole.length);

  await again();                                                        // a reload alone downloads nothing more
  expect(raws()).toEqual(['cut']);
  verify(ref());
  const full = await again();
  expect(raws()).toEqual(['cut', 'whole']);
  expect(full.pending).toBe(false);
  expect(full.badges).toEqual(['verified']);
  expect(full.parts.map((p) => p.text.trim())).toEqual(['The report is attached.']);
  await again();                                                        // verified: the whole message is kept
  expect(raws()).toEqual(['cut', 'whole']);
});

test('a signed mail with nothing cut: verified at once, in one request', async () => {
  const alice = await keys('alice@example.com');
  whole = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey, attachments: [] }));
  trusted = [{ fingerprint: alice.publicKey.getFingerprint(), email: 'alice@example.com', armored: alice.publicKey.armor(), is_own: false, source: 'manual', trust: 'trusted' }];
  flags = { is_encrypted: false, is_signed: true };
  const b = await read();
  expect(raws()).toEqual(['cut']);
  expect(b.pending).toBe(false);
  expect(b.badges).toEqual(['verified']);
});

test('a bad signature over a cut attachment: shown as signed, the warning once verified', async () => {
  const alice = await keys('alice@example.com');
  const signed = new TextDecoder().decode(await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey })));
  whole = new TextEncoder().encode(signed.replace('JSUl', 'JSUm'));      // one byte of the PDF changed after signing
  trusted = [{ fingerprint: alice.publicKey.getFingerprint(), email: 'alice@example.com', armored: alice.publicKey.armor(), is_own: false, source: 'manual', trust: 'trusted' }];
  flags = { is_encrypted: false, is_signed: true };
  const b = await read();
  expect(b.badges).toEqual(['signed']);
  verify(ref());
  const full = await again();
  expect(full.badges).not.toContain('verified');
  expect(full.parts.map((p) => p.mark)).toEqual(['verification_failed']);
});

test('an encrypted message forwarded in a plain one, its ciphertext cut: shown closed, opened with one click, read whole', async () => {
  const bob = await keys('bob@example.com');
  const forward = new TextDecoder().decode(await binaryEncrypted(bob.publicKey));
  whole = Uint8Array.from(Buffer.from([
    'From: Carol <carol@example.com>', 'To: bob@example.com', 'Subject: Fwd: report', 'MIME-Version: 1.0',
    'Content-Type: multipart/mixed; boundary="f"', '',
    '--f', 'Content-Type: text/plain; charset=utf-8', '', 'See the forward below.',
    '--f', 'Content-Type: message/rfc822', '', forward.trimEnd(), '--f--', '',
  ].join('\r\n')));
  expect((await mail.cut(whole)).holes).toHaveLength(1);                 // not the root's: opened only on request
  await unlockAs(bob.privateKey);
  flags = { is_encrypted: false, is_signed: false };
  const b = await read();
  expect(raws()).toEqual(['cut']);
  const part = b.parts.find((p) => p.kind === 'encrypted')!;
  expect(part.reason).toBe('incomplete');
  expect(part.step === '' || part.step.endsWith('#cut')).toBe(true);   // a cut message's: no step of the whole message matches it
  expect(b.parts.filter((p) => p.kind === 'text').map((p) => p.text.trim())).toEqual(['See the forward below.']);

  openPart(ref(), part.step, part.id);
  const opened = await again();
  expect(raws()).toEqual(['cut', 'whole']);
  expect(opened.parts.filter((p) => p.kind === 'text').map((p) => p.text.trim())).toEqual(['See the forward below.', 'The report is attached.']);
  expect(opened.parts.some((p) => p.kind === 'encrypted')).toBe(false);
});

test('a signature by a key found in mail that the user has not accepted is not verified', async () => {
  const alice = await keys('alice@example.com');
  whole = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey, attachments: [] }));
  trusted = [{ fingerprint: alice.publicKey.getFingerprint(), email: 'alice@example.com', armored: alice.publicKey.armor(), is_own: false, source: 'harvested_autocrypt', trust: 'unverified' }];
  flags = { is_encrypted: false, is_signed: true };
  const b = await read();
  expect(b.badges).not.toContain('verified');
  expect(b.parts.map((p) => p.mark)).toEqual(['unverified']);
});

test('verified for another sender than the server reports: the reader gets the checked sender and the warning', async () => {
  const alice = await keys('alice@example.com');
  whole = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey, attachments: [] }));
  trusted = [{ fingerprint: alice.publicKey.getFingerprint(), email: 'alice@example.com', armored: alice.publicKey.armor(), is_own: false, source: 'imported', trust: 'trusted' }];
  flags = { is_encrypted: false, is_signed: true };
  serverFrom = 'Mallory <mallory@evil.example>';                      // the server parsed another From than the reader
  const b = await read();
  expect(b.badges).toEqual(['verified', 'from_differs']);
  expect(b.sender).toBe('Alice <alice@example.com>');

  serverFrom = 'alice@example.com';
  const same = await read();
  expect(same.badges).toEqual(['verified']);
  expect(same.sender).toBe('');
});

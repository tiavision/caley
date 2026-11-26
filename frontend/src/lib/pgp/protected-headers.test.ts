// @vitest-environment node
// Integration cases use real openpgp crypto, which needs node (see sign-verify.test.ts).
// Caley's composer (buildProtectedMime) against its reader (settle: sinter-mail's mail.mime with the OpenPGP steps):
// what one writes, the other reads, and what it proves.
import { describe, expect, it } from 'vitest';
import * as openpgp from 'openpgp';
import { buildProtectedMime, type ProtectedComposeMessage } from './compose';
import { settle } from './steps';
import {
  checkedSender, messageBadges, protectedReply, protectedSubject, quotable, readerParts, replyAddresses, replyAllAddresses, senderDiffers,
} from '../mail-reader';

async function keys(email: string) {
  return openpgp.generateKey({ userIDs: [{ name: email.split('@')[0], email }], format: 'object' });
}

function draft(over: Partial<ProtectedComposeMessage>): ProtectedComposeMessage {
  return {
    identity: { name: 'Alice', email: 'alice@example.com' },
    to: ['bob@example.com'], cc: [], subject: 'The real secret subject', body: 'Encrypted body',
    inReplyTo: null, references: null, attachments: [], encrypt: true, sign: false, encryptionKeys: [], signingKey: null,
    ...over,
  } as ProtectedComposeMessage;
}

const empty = () => ({ decrypted: {}, verdicts: {}, open: [] });

describe('reading what caley sends', () => {
  it('an encrypted message: locked without a key, then the protected subject, the body and the badge', async () => {
    const bob = await keys('bob@example.com');
    const inner = [
      'Content-Type: text/plain; charset=utf-8; protected-headers="v1"',
      'Subject: The real secret subject',
      'From: Alice <alice@example.com>',
      'To: bob@example.com',
      '',
      'Encrypted body',
      '',
    ].join('\r\n');
    const ciphertext = await openpgp.encrypt({ message: await openpgp.createMessage({ text: inner }), encryptionKeys: bob.publicKey });
    const raw = new TextEncoder().encode([
      'From: Alice <alice@example.com>', 'To: bob@example.com', 'Subject: ...', 'MIME-Version: 1.0',
      'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary="e"', '',
      '--e', 'Content-Type: application/pgp-encrypted', '', 'Version: 1', '',
      '--e', 'Content-Type: application/octet-stream; name="encrypted.asc"', '', String(ciphertext).replace(/\r?\n/g, '\r\n'),
      '--e--', '',
    ].join('\r\n'));

    const locked = await settle(raw, empty(), [], async () => []);
    expect(locked.parts.map((p) => p.type)).toEqual(['encrypted']);
    expect(protectedSubject(locked)).toBe('');

    const msg = await settle(raw, empty(), [bob.privateKey], async () => []);
    expect(protectedSubject(msg)).toBe('The real secret subject');
    expect(readerParts(msg).map((p) => [p.kind, p.text.trim(), p.encrypted])).toEqual([['text', 'Encrypted body', true]]);
    expect(messageBadges(msg)).toEqual(['encrypted', 'from_unknown']);   // anyone can encrypt: the sender is not proven
    expect(quotable(msg)).toBe(false);
  });

  // Every encrypted message caley sends protects its headers: outside, the Subject is "[...]"; the reader shows the
  // real one from inside, for each shape of the payload (text, text and HTML, with attachments).
  const shapes: [string, Partial<ProtectedComposeMessage>, string][] = [
    ['text', {}, 'text/plain'],
    ['text and HTML', { bodyHtml: '<p>Encrypted <b>body</b></p>' }, 'multipart/alternative'],
    ['with an attachment', { attachments: [new File(['x'], 'plan.txt', { type: 'text/plain' })] }, 'multipart/mixed'],
    ['text and HTML with an attachment', { bodyHtml: '<p>Encrypted <b>body</b></p>', attachments: [new File(['x'], 'plan.txt', { type: 'text/plain' })] }, 'multipart/mixed'],
  ];
  for (const [shape, over, root] of shapes) {
    it(`caley's own encrypted message (${shape}): the subject only inside, and the reader shows it`, async () => {
      const bob = await keys('bob@example.com');
      const raw = await buildProtectedMime(draft({
        encryptionKeys: [bob.publicKey], cc: ['carol@example.com'], inReplyTo: '<0@example.com>', references: '<0@example.com>', ...over,
      }));
      const text = new TextDecoder().decode(raw);
      expect(text).toMatch(/^Subject: \[\.\.\.\]\r$/m);
      expect(text).not.toContain('secret subject');
      expect(text).not.toMatch(/^Bcc:/im);

      const locked = await settle(raw, empty(), [], async () => []);
      expect(locked.headers.subject).toBe('[...]');
      expect(protectedSubject(locked)).toBe('');

      const msg = await settle(raw, empty(), [bob.privateKey], async () => []);
      expect(protectedSubject(msg)).toBe('The real secret subject');
      expect(msg.headers.subject).toBe('The real secret subject');
      expect(msg.outer.subject).toBe('[...]');
      expect(msg.sources.subject).toBe('protected');
      expect(msg.headers.to.map((a) => a.address)).toEqual(['bob@example.com']);
      expect(msg.sources.cc).toBe('protected');
      expect(msg.headers.from.map((a) => a.address)).toEqual(['alice@example.com']);
      expect(msg.summary.from).toBe('unknown');                         // unsigned: the same From, but nothing proves it
      expect(readerParts(msg).some((p) => p.text.trim() === 'Encrypted body' || p.html.includes('Encrypted'))).toBe(true);

      // the payload's root carries the real headers, and nothing the sender left out
      const decrypted = await openpgp.decrypt({
        message: await openpgp.readMessage({ armoredMessage: /-----BEGIN PGP MESSAGE-----[\s\S]*?-----END PGP MESSAGE-----/.exec(text)![0] }),
        decryptionKeys: bob.privateKey, format: 'binary',
      });
      const payload = new TextDecoder().decode(decrypted.data as Uint8Array);
      const head = payload.slice(0, payload.indexOf('\r\n\r\n'));
      expect(head).toMatch(new RegExp(`^Content-Type: ${root};[^]*protected-headers="v1"`));
      for (const field of ['Date', 'Message-ID', 'From', 'To', 'Cc', 'Subject', 'In-Reply-To', 'References']) expect(head).toMatch(new RegExp(`^${field}: `, 'm'));
      expect(head).not.toMatch(/^(Bcc|Autocrypt|MIME-Version):/im);
    });
  }

  it('signed only, without attachments: the subject stays outside, as before', async () => {
    const alice = await keys('alice@example.com');
    const text = new TextDecoder().decode(await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey })));
    expect(text).toMatch(/^Subject: The real secret subject\r$/m);
    expect(text).not.toContain('protected-headers');
  });

  it('encrypted and signed by a key trusted for the sender: verified, and a reply may quote it', async () => {
    const alice = await keys('alice@example.com');
    const bob = await keys('bob@example.com');
    const raw = await buildProtectedMime(draft({ encryptionKeys: [bob.publicKey], sign: true, signingKey: alice.privateKey }));
    const msg = await settle(raw, empty(), [bob.privateKey], async () => [{ key: alice.publicKey, addresses: ['alice@example.com'] }]);
    expect(messageBadges(msg)).toEqual(['encrypted', 'verified']);
    expect(quotable(msg)).toBe(true);
    expect(protectedSubject(msg)).toBe('The real secret subject');
    expect(msg.sources.from).toBe('protected');                       // the signature binds the protected From
    expect(protectedReply(msg)).toEqual(['Alice <alice@example.com>']);
    expect(replyAddresses(msg)).toEqual(['Alice <alice@example.com>']);
  });

  it('signed by a key trusted for someone else: no verified badge, the part marked, a warning on the sender', async () => {
    const alice = await keys('alice@example.com');
    const raw = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey }));
    const msg = await settle(raw, empty(), [], async () => [{ key: alice.publicKey, addresses: ['mallory@example.com'] }]);
    expect(messageBadges(msg)).toEqual(['from_unknown']);
    expect(readerParts(msg).map((p) => p.mark)).toEqual(['signed_part']);
  });
});

/** A ciphertext for Bob, re-sent by Mallory under outer headers of his own. */
async function resent(bob: openpgp.PublicKey, inner: string): Promise<Uint8Array> {
  const ciphertext = await openpgp.encrypt({ message: await openpgp.createMessage({ text: inner }), encryptionKeys: bob });
  return new TextEncoder().encode([
    'From: Mallory <mallory@evil.example>', 'Reply-To: mallory@evil.example', 'To: bob@example.com', 'Cc: mallory2@evil.example',
    'Subject: ...', 'MIME-Version: 1.0',
    'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary="e"', '',
    '--e', 'Content-Type: application/pgp-encrypted', '', 'Version: 1', '',
    '--e', 'Content-Type: application/octet-stream; name="encrypted.asc"', '', String(ciphertext).replace(/\r?\n/g, '\r\n'),
    '--e--', '',
  ].join('\r\n'));
}

describe('whom a reply to decrypted mail goes to: only the protected message, never the outer headers', () => {
  it('protected headers: the protected From (no Reply-To inside), reply all the protected To and Cc', async () => {
    const bob = await keys('bob@example.com');
    const raw = await resent(bob.publicKey, [
      'Content-Type: text/plain; charset=utf-8; protected-headers="v1"',
      'Subject: Plan', 'From: Alice <alice@example.com>', 'To: bob@example.com', 'Cc: Carol <carol@example.com>', '',
      'Secret plan', '',
    ].join('\r\n'));
    const msg = await settle(raw, empty(), [bob.privateKey], async () => []);
    expect(msg.summary.encrypted).toBe('all');
    expect(replyAddresses(msg)).toEqual(['Alice <alice@example.com>']);
    expect(replyAllAddresses(msg)).toEqual(['bob@example.com', 'Carol <carol@example.com>']);
    expect(quotable(msg)).toBe(false);                                  // unsigned: nothing binds Alice
  });

  it('no protected headers and no signature inside: nobody (the user fills it in), and nothing quoted', async () => {
    const bob = await keys('bob@example.com');
    const raw = await resent(bob.publicKey, 'Content-Type: text/plain; charset=utf-8\r\n\r\nSecret plan\r\n');
    const msg = await settle(raw, empty(), [bob.privateKey], async () => []);
    expect(replyAddresses(msg)).toEqual([]);
    expect(replyAllAddresses(msg)).toEqual([]);
    expect(quotable(msg)).toBe(false);
  });

  it('plain mail: as before (the server\'s From and Reply-To, To and Cc)', async () => {
    const alice = await keys('alice@example.com');
    const raw = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey }));
    const msg = await settle(raw, empty(), [], async () => []);
    expect(replyAddresses(msg)).toBeNull();
    expect(replyAllAddresses(msg)).toBeNull();
  });
});

describe('"verified" is shown beside the sender the signature was checked for', () => {
  /** A signed message (PGP/MIME) whose payload names `innerFrom` in protected headers, sent under `outerFrom`. */
  async function signedAs(signer: openpgp.PrivateKey, outerFrom: string, innerFrom: string): Promise<Uint8Array> {
    const inner = [`Content-Type: text/plain; charset=utf-8; protected-headers="v1"`, `From: ${innerFrom}`, 'Subject: Hi', '', 'Pay the invoice.'].join('\r\n');
    const signature = await openpgp.sign({ message: await openpgp.createMessage({ binary: new TextEncoder().encode(inner) }), signingKeys: signer, detached: true });
    return new TextEncoder().encode([
      `From: ${outerFrom}`, 'To: bob@example.com', 'Subject: Hi', 'MIME-Version: 1.0',
      'Content-Type: multipart/signed; micalg=pgp-sha512; protocol="application/pgp-signature"; boundary="s"', '',
      '--s', inner, '--s', 'Content-Type: application/pgp-signature', '', String(signature).replace(/\r?\n/g, '\r\n'), '--s--', '',
    ].join('\r\n'));
  }

  it('a protected From the signature binds, under another outer From: the checked sender is the protected one, with the warning', async () => {
    const alice = await keys('alice@example.com');
    const raw = await signedAs(alice.privateKey, 'Mallory <mallory@evil.example>', 'Alice <alice@example.com>');
    const msg = await settle(raw, empty(), [], async () => [{ key: alice.publicKey, addresses: ['alice@example.com'] }]);
    expect(msg.summary.signed).toBe('all');
    expect(msg.sources.from).toBe('protected');
    expect(checkedSender(msg)!.address).toBe('alice@example.com');
    // the server reports the outer From's address: "verified" never stands alone beside it
    expect(senderDiffers(msg, 'mallory@evil.example')).toBe(true);
    expect(messageBadges(msg, 'mallory@evil.example')).toEqual(['verified', 'from_differs']);
    expect(messageBadges(msg, 'alice@example.com')).toEqual(['verified']);
    // the server's address, never one parsed from a display name; no single sender matches no checked one
    expect(messageBadges(msg, '"<alice@example.com>" <mallory@evil.example>')).toEqual(['verified', 'from_differs']);
    expect(messageBadges(msg, null)).toEqual(['verified', 'from_differs']);
  });

  it('the same From outside and in: verified, no warning', async () => {
    const alice = await keys('alice@example.com');
    const raw = await signedAs(alice.privateKey, 'Alice <alice@example.com>', 'Alice <alice@example.com>');
    const msg = await settle(raw, empty(), [], async () => [{ key: alice.publicKey, addresses: ['alice@example.com'] }]);
    expect(messageBadges(msg, 'alice@example.com')).toEqual(['verified']);
    expect(senderDiffers(msg, 'ALICE@example.com')).toBe(false);
  });
});

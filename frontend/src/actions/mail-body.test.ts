// @vitest-environment jsdom
// body() against a server that cuts large attachments out (backend/src/email/cut.rs; here sinter-mail's reference
// cutter, mail.cut): the text renders from the cut message, and a cut the reader does not accept costs a full
// download, never a body.
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import * as mail from '@tiavision/sinter-mail';
import type { CallContext } from '@tiaforge-ui/sinter';
import { body, wipe } from './mail.ts';
import type { Message } from '../lib/mail.ts';

const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const b64 = (bytes: Uint8Array) => Buffer.from(bytes).toString('base64').replace(/.{76}/g, '$&\r\n');

/** A mail with a text and an HTML part, a 600 kB inline PNG and a 200 kB PDF. */
function message(): Uint8Array {
  const png = new Uint8Array(600_000); png.set(PNG);
  const pdf = new Uint8Array(200_000).fill(0x25);
  return Uint8Array.from(new TextEncoder().encode([
    'From: Ada <ada@example.com>', 'To: me@example.com', 'Subject: Report', 'MIME-Version: 1.0',
    'Content-Type: multipart/mixed; boundary="m"', '',
    '--m', 'Content-Type: multipart/related; boundary="r"', '',
    '--r', 'Content-Type: text/html; charset=utf-8', '', '<p>The report is attached.</p><img src="cid:chart@x">',
    '--r', 'Content-Type: image/png', 'Content-ID: <chart@x>', 'Content-Transfer-Encoding: base64', '', b64(png),
    '--r--',
    '--m', 'Content-Type: application/pdf; name="report.pdf"', 'Content-Disposition: attachment; filename="report.pdf"',
    'Content-Transfer-Encoding: base64', '', b64(pdf),
    '--m--', '',
  ].join('\r\n')));   // this realm's Uint8Array (jsdom's TextEncoder returns another's)
}

const meta = {
  uid: 7, account_id: 'work', mailbox: 'INBOX', rowid: 7, message_id: null, thread_id: null, from: 'Ada <ada@example.com>',
  to: ['me@example.com'], cc: [], subject: 'Report', sent_at: 1, flags: [], has_attachments: true, body_cached: true,
  is_read: true, is_flagged: false, is_encrypted: false, is_signed: false, attachments: [],
} as unknown as Message;

let calls: string[] = [];
let holesOf: (raw: Uint8Array) => Promise<{ raw: Uint8Array; holes: { start: number; end: number; size: number }[] }>;

beforeEach(() => {
  calls = [];
  holesOf = async (raw) => await mail.cut(raw);
  const whole = message();
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
    const range = new Headers(init?.headers).get('Range');
    calls.push(range ? `${url} ${range}` : url);
    if (url.startsWith('/api/email/message?')) return Response.json({ ...meta, uid: Number(new URL(url, 'http://x').searchParams.get('uid')) });
    if (!url.startsWith('/api/email/message/raw?')) return new Response('', { status: 404 });
    if (range) {
      const [, a, b] = /bytes=(\d+)-(\d+)/.exec(range)!;
      return new Response(whole.slice(Number(a), Number(b) + 1), { status: 206 });
    }
    if (!url.includes('omit=')) return new Response(whole, { headers: { ETag: '"e"' } });
    const cut = await holesOf(whole);
    return new Response(cut.raw, { headers: { ETag: '"e"', 'Mail-Holes': JSON.stringify(cut.holes) } });
  });
});
afterEach(() => { wipe(); vi.unstubAllGlobals(); });
// each test reads its own message (uid): body() keeps the open message's source for a reload

const ctx = () => ({ signal: new AbortController().signal, request: (url: string) => fetch(url).then((r) => r.json()) }) as unknown as CallContext;

test('a plain mail is read cut: the text at once, its inline image and PDF left to the server', async () => {
  const b = await body({ account_id: 'work', mailbox: 'INBOX', uid: 7 }, ctx());
  expect(b.error).toBe('');
  expect(b.parts.map((p) => p.kind)).toEqual(['html']);
  expect(b.parts[0]!.html).toContain('The report is attached.');
  expect(b.cids).toEqual({});                                    // the server serves the cid: image (bodyImages)
  expect(calls.filter((c) => c.startsWith('/api/email/message/raw'))).toEqual(['/api/email/message/raw?account_id=work&mailbox=INBOX&uid=7&omit=65536']);
});

test('a cut the reader does not accept: the whole message, read as usual', async () => {
  holesOf = async (raw) => {
    const cut = await mail.cut(raw);
    return { raw: cut.raw, holes: cut.holes.map((h) => ({ ...h, start: h.start + 1, end: h.end + 1 })) };
  };
  const b = await body({ account_id: 'work', mailbox: 'INBOX', uid: 8 }, ctx());
  expect(b.parts[0]!.html).toContain('The report is attached.');
  const raw = calls.filter((c) => c.startsWith('/api/email/message/raw'));
  expect(raw.at(-1)).not.toContain('omit=');
  expect(raw.some((c) => c.includes('bytes='))).toBe(false);
});

test('holes the reader refuses (malformed): the whole message, read as usual', async () => {
  holesOf = async (raw) => ({ raw: (await mail.cut(raw)).raw, holes: [{ start: -5, end: 3, size: 1 }] });
  const b = await body({ account_id: 'work', mailbox: 'INBOX', uid: 10 }, ctx());
  expect(b.parts[0]!.html).toContain('The report is attached.');
  expect(calls.filter((c) => c.startsWith('/api/email/message/raw')).at(-1)).not.toContain('omit=');
});

test('a signed or encrypted message is fetched cut too: the reader decides what it needs whole', async () => {
  Object.assign(meta, { is_signed: true, is_encrypted: true });
  try {
    await body({ account_id: 'work', mailbox: 'INBOX', uid: 9 }, ctx());
  } finally {
    Object.assign(meta, { is_signed: false, is_encrypted: false });
  }
  expect(calls.filter((c) => c.startsWith('/api/email/message/raw'))).toEqual(['/api/email/message/raw?account_id=work&mailbox=INBOX&uid=9&omit=65536']);
});

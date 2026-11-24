// @vitest-environment node
// Integration cases use real openpgp crypto, which needs node (see sign-verify.test.ts).
// Caley's composer (buildProtectedMime) against its reader (settle: sinter-mail's mail.mime with the OpenPGP steps):
// what one writes, the other reads, and what it proves.
import { describe, expect, it } from 'vitest';
import * as openpgp from 'openpgp';
import { buildProtectedMime, type ProtectedComposeMessage } from './compose';
import { settle } from './steps';
import { messageBadges, protectedReply, protectedSubject, quotable, readerParts } from '../mail-reader';

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

  it('caley\'s own encrypted message reads back (it carries no protected headers yet: the outer ones show)', async () => {
    const bob = await keys('bob@example.com');
    const raw = await buildProtectedMime(draft({ encryptionKeys: [bob.publicKey] }));
    const msg = await settle(raw, empty(), [bob.privateKey], async () => []);
    expect(readerParts(msg).map((p) => [p.kind, p.text.trim()])).toEqual([['text', 'Encrypted body']]);
    expect(msg.headers.subject).toBe('The real secret subject');
  });

  it('encrypted and signed by a key trusted for the sender: verified, and a reply may quote it', async () => {
    const alice = await keys('alice@example.com');
    const bob = await keys('bob@example.com');
    const raw = await buildProtectedMime(draft({ encryptionKeys: [bob.publicKey], sign: true, signingKey: alice.privateKey }));
    const msg = await settle(raw, empty(), [bob.privateKey], async () => [{ key: alice.publicKey, addresses: ['alice@example.com'] }]);
    expect(messageBadges(msg)).toEqual(['encrypted', 'verified']);
    expect(quotable(msg)).toBe(true);
    expect(protectedReply(msg)).toBeNull();   // no protected headers: the message's own From and Reply-To
  });

  it('signed by a key trusted for someone else: no verified badge, the part marked, a warning on the sender', async () => {
    const alice = await keys('alice@example.com');
    const raw = await buildProtectedMime(draft({ encrypt: false, sign: true, signingKey: alice.privateKey }));
    const msg = await settle(raw, empty(), [], async () => [{ key: alice.publicKey, addresses: ['mallory@example.com'] }]);
    expect(messageBadges(msg)).toEqual(['from_unknown']);
    expect(readerParts(msg).map((p) => p.mark)).toEqual(['signed_part']);
  });
});

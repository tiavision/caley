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
    expect(protectedReply(msg)).toBeNull();   // no protected headers
    expect(replyAddresses(msg)).toEqual(['Alice <alice@example.com>']);   // the outer From, which the signature inside binds
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

// @vitest-environment node
// Real openpgp crypto must run under node: jsdom's TextEncoder returns a
// cross-realm Uint8Array that fails openpgp's internal type checks.
import { describe, expect, it } from 'vitest';
import * as openpgp from 'openpgp';
import { buildProtectedMime, type ProtectedComposeMessage } from './compose';
import { detectPgpMessage, verifySignedMessage } from './keyring';
import { autocryptKeys } from './autocrypt';

// Uses real openpgp crypto (no mock) to guard the PGP/MIME signing pipeline:
// the signed octets must match exactly what other clients verify, and our
// verifier must not "repair" mangled content into a false-positive signature.
async function signingKey() {
  const { privateKey, publicKey } = await openpgp.generateKey({
    userIDs: [{ name: 'Tester', email: 'tester@example.com' }],
    format: 'object',
  });
  return { privateKey, publicKey };
}

function baseMessage(privateKey: openpgp.PrivateKey): ProtectedComposeMessage {
  return {
    identity: { name: 'Tester', email: 'tester@example.com' },
    to: ['rcpt@example.com'],
    cc: [],
    subject: 'Hi',
    body: 'Hello world\nLine two',
    inReplyTo: null,
    references: null,
    attachments: [],
    encrypt: false,
    sign: true,
    encryptionKeys: [],
    signingKey: privateKey,
  };
}

describe('PGP/MIME sign + verify', () => {
  it('signs a body part that verifies against the signer key', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime(baseMessage(privateKey));

    expect(await verifySignedMessage(mime, [publicKey])).toContain('verified');
  });

  it('advertises the key via Autocrypt header, not an attachment (K-9 #6511)', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime(baseMessage(privateKey));
    const text = new TextDecoder().decode(mime);

    // The key-attachment is what breaks Thunderbird mobile — it must be gone.
    expect(text).not.toContain('application/pgp-keys');
    expect(text).not.toContain('openpgp-key.asc');

    // It must instead ride in an Autocrypt header that round-trips to our key.
    expect(text).toMatch(/^Autocrypt: addr=/m);
    const harvested = await autocryptKeys(mime, baseMessage(privateKey).identity.email);
    expect(harvested.map(k => k.getFingerprint())).toContain(publicKey.getFingerprint());
  });

  it('declares a micalg matching the real hash and emits CRLF-only armor', async () => {
    const { privateKey } = await signingKey();
    const text = new TextDecoder().decode(await buildProtectedMime(baseMessage(privateKey)));

    const micalg = /micalg=([^;]+);/.exec(text)![1];
    const sigArmor = text.slice(
      text.indexOf('-----BEGIN PGP SIGNATURE-----'),
      text.indexOf('-----END PGP SIGNATURE-----') + '-----END PGP SIGNATURE-----'.length,
    );
    const hashId = (await openpgp.readSignature({ armoredSignature: sigArmor })).packets[0].hashAlgorithm;
    const expected: Record<number, string> = { 8: 'pgp-sha256', 9: 'pgp-sha384', 10: 'pgp-sha512' };
    expect(micalg).toBe(expected[hashId!]);
    // No bare LF anywhere in the MIME entity.
    expect(/[^\r]\n/.test(text) || text.startsWith('\n')).toBe(false);
  });

  it('does not include the boundary delimiter CRLF in the signed content', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime(baseMessage(privateKey));
    const text = new TextDecoder().decode(mime);

    // Appending an extra blank line to the signed part (delimiter drift / a
    // forwarder rewriting trailing whitespace) must invalidate the signature.
    const boundary = /boundary="([^"]+)"/.exec(text)![1];
    const mangled = text.replace(`\r\n--${boundary}\r\nContent-Type: application/pgp-signature`,
      `\r\n\r\n--${boundary}\r\nContent-Type: application/pgp-signature`);
    expect(await verifySignedMessage(new TextEncoder().encode(mangled), [publicKey]))
      .not.toContain('verified');
  });

  it('rejects a tampered body instead of re-canonicalizing it to valid', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime(baseMessage(privateKey));
    const tampered = new TextDecoder().decode(mime).replace('Hello world', 'Hello WORLD');

    expect(await verifySignedMessage(new TextEncoder().encode(tampered), [publicKey]))
      .toContain('verification_failed');
  });

  it('signs multipart/mixed messages with attachments as exact MIME octets', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime({
      ...baseMessage(privateKey),
      attachments: [new File(['attached content'], 'note.txt', { type: 'text/plain' })],
    });
    const text = new TextDecoder().decode(mime);

    expect(text).toContain('Content-Type: multipart/mixed;');
    expect(text).toContain('protected-headers="v1"');
    expect(text).toMatch(/\r\nFrom: Tester <tester@example\.com>\r\nTo: rcpt@example\.com\r\nSubject: Hi\r\n\r\n/);
    expect(text).toContain('filename="note.txt"');
    expect(text).toContain('name="OpenPGP_signature.asc"');
    expect(text).toContain('Content-Description: OpenPGP digital signature');
    expect(text).toContain('filename="OpenPGP_signature"');

    const signedBoundary = /^Content-Type: multipart\/signed;[^\r\n]+boundary="([^"]+)"/m.exec(text)![1];
    const mixedBoundary = /Content-Type: multipart\/mixed; boundary="([^"]+)"/.exec(text)![1];
    expect(text).toContain(`--${mixedBoundary}--\r\n\r\n--${signedBoundary}\r\nContent-Type: application/pgp-signature`);
    expect(await verifySignedMessage(mime, [publicKey])).toContain('verified');

    const tampered = text.replace('YXR0YWNoZWQgY29udGVudA==', 'YXR0YWNoZWQgY29udGVuZA==');
    expect(await verifySignedMessage(new TextEncoder().encode(tampered), [publicKey]))
      .toContain('verification_failed');
  });

  it('keeps a forwarded signature.asc attachment inside the signed content', async () => {
    const { privateKey, publicKey } = await signingKey();
    const mime = await buildProtectedMime({
      ...baseMessage(privateKey),
      attachments: [new File(['-----BEGIN PGP SIGNATURE-----\nold\n-----END PGP SIGNATURE-----'], 'signature.asc', { type: 'application/pgp-signature' })],
    });
    const text = new TextDecoder().decode(mime);

    expect(text.match(/filename="signature\.asc"/g)).toHaveLength(1);
    expect(text).toContain('filename="OpenPGP_signature"');
    expect(await verifySignedMessage(mime, [publicKey])).toContain('verified');
  });
});

describe('PGP message detection', () => {
  const enc = new TextEncoder();

  it('detects PGP/MIME signed mail', () => {
    const raw = enc.encode(
      'From: tester@example.com\r\nContent-Type: multipart/signed; boundary="s"; protocol="application/pgp-signature"\r\n\r\n' +
      '--s\r\nContent-Type: text/plain\r\n\r\nHello\r\n' +
      '--s\r\nContent-Type: application/pgp-signature\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\nx\r\n-----END PGP SIGNATURE-----\r\n--s--\r\n',
    );

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: false, isSigned: true });
  });

  it('detects inline clear-signed mail', () => {
    const raw = enc.encode('Content-Type: text/plain\r\n\r\n-----BEGIN PGP SIGNED MESSAGE-----\r\nHash: SHA256\r\n\r\nHello');

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: false, isSigned: true });
  });

  it('detects PGP/MIME encrypted mail', () => {
    const raw = enc.encode(
      'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary="e"\r\n\r\n' +
      '--e\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n--e--\r\n',
    );

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: true, isSigned: false });
  });

  it('detects inline encrypted mail', () => {
    const raw = enc.encode('Content-Type: text/plain\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nx\r\n-----END PGP MESSAGE-----');

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: true, isSigned: false });
  });

  it('ignores ordinary mail', () => {
    const raw = enc.encode('Content-Type: text/plain\r\n\r\nHello');

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: false, isSigned: false });
  });

  it('scans beyond the initial header block', () => {
    const raw = enc.encode(`${'X-Filler: value\r\n'.repeat(400)}\r\n-----BEGIN PGP SIGNED MESSAGE-----\r\nHash: SHA256\r\n\r\nHello`);

    expect(detectPgpMessage(raw)).toEqual({ isEncrypted: false, isSigned: true });
  });
});

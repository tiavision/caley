// @vitest-environment node
// Real openpgp crypto must run under node: jsdom's TextEncoder returns a
// cross-realm Uint8Array that fails openpgp's internal type checks.
import { describe, expect, it } from 'vitest';
import * as openpgp from 'openpgp';
import { buildProtectedMime, type ProtectedComposeMessage } from './compose';
import { verifySignedMessage } from './keyring';

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
});

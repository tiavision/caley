// openpgp.js wrappers for key import, decryption, and signature verification.
// CSP constraint: no 'unsafe-eval', no wasm — Argon2 S2K is explicitly rejected.

import * as openpgp from 'openpgp';
import { decodeText, extractDetachedSignedMime, extractPgpBlock, extractSignedBlock, hasAsciiMarker, parseDecryptedMime, parseHeaders, readParameter, splitMultipart, type DecryptedAttachment } from './mime.ts';

export type { DecryptedAttachment, PgpMessageDetection } from './mime.ts';
export { detectPgpMessage, parseMimeMessage } from './mime.ts';

export type DecryptBadge =
  | 'encrypted'       // decrypted successfully
  | 'signed'          // signature present
  | 'verified'        // signature cryptographically valid
  | 'unverified'      // signed but key not available
  | 'missing_key'     // encrypted but our private key not present
  | 'unsupported_alg' // Argon2 or other unsupported S2K
  | 'verification_failed' // signature invalid
  | 'decrypt_failed'; // decryption error

export interface DecryptResult {
  text: string | null;
  html: string | null;
  badges: DecryptBadge[];
  attachments: DecryptedAttachment[];
  /** Protected subject recovered from headers inside the encrypted payload, if any. */
  subject: string | null;
}

function hasArgon2(err: unknown): boolean {
  return err instanceof Error && err.message.toLowerCase().includes('argon2');
}

/** Import and decrypt a private key from armored text + passphrase. */
export async function importPrivateKey(armored: string, passphrase: string): Promise<openpgp.PrivateKey> {
  const key = await openpgp.readPrivateKey({ armoredKey: armored });
  if (key.isDecrypted()) return key;
  const decrypted = await openpgp.decryptKey({ privateKey: key, passphrase });
  return decrypted;
}

/** Generate a new private key (Ed25519 signing, Curve25519 encryption subkey, no expiry) for one User ID. */
export async function generatePrivateKey(identity: openpgp.UserID): Promise<openpgp.PrivateKey> {
  const { privateKey } = await openpgp.generateKey({ type: 'ecc', curve: 'curve25519Legacy', userIDs: [identity], format: 'object' });
  return privateKey;
}

/** The first key of a binary (unarmored) key file, ASCII-armored; a protected private key stays protected. */
export async function armorBinaryKey(binaryKey: Uint8Array): Promise<string> {
  const [key] = await openpgp.readKeys({ binaryKeys: binaryKey });
  if (!key) throw new Error('no key in file');
  return key.armor();
}

/** Return the primary fingerprint (uppercase hex) of a private or public key. */
export function getFingerprint(key: openpgp.Key): string {
  return key.getFingerprint().toUpperCase();
}

/** Return the primary email from the key's User ID. */
export function getPrimaryEmail(key: openpgp.Key): string {
  const uid = key.users[0]?.userID?.email ?? '';
  return uid;
}

export interface KeyIdentity {
  name: string;
  email: string;
  comment: string;
  userID: string;
}

/** Return all textual User IDs certified on a key. */
export function getKeyIdentities(key: openpgp.Key): KeyIdentity[] {
  return key.users.flatMap(user => user.userID ? [{
    name: user.userID.name,
    email: user.userID.email,
    comment: user.userID.comment,
    userID: user.userID.userID,
  }] : []);
}

/** Self-certify another User ID while retaining existing key certifications. */
export async function addKeyIdentity(
  privateKey: openpgp.PrivateKey,
  identity: openpgp.UserID,
): Promise<openpgp.PrivateKey> {
  const reformatted = await openpgp.reformatKey({ privateKey, userIDs: [identity], format: 'object' });
  return privateKey.update(reformatted.publicKey);
}

/** Return armored public key from a private key. */
export function extractPublicKey(privateKey: openpgp.PrivateKey): string {
  return privateKey.toPublic().armor();
}

/**
 * Decrypt a PGP/MIME or inline PGP message from raw MIME bytes.
 * privateKeys — unlocked in-memory private keys (tried in order)
 * rawMime     — the raw .eml bytes fetched from /api/email/raw
 * publicKeys  — trusted public keys available for signer verification
 * Returns decrypted text/html bodies plus signature status badges.
 */
export async function decryptMessage(
  privateKeys: openpgp.PrivateKey[],
  rawMime: Uint8Array,
  publicKeys: openpgp.PublicKey[] = [],
): Promise<DecryptResult> {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawMime);
  const badges: DecryptBadge[] = [];

  // Extract the PGP block from the MIME structure.
  const pgpBlock = extractPgpBlock(mimeStr);
  if (!pgpBlock) {
    return { text: mimeStr, html: null, badges: [], attachments: [], subject: null };
  }

  let message: openpgp.Message<string | Uint8Array>;
  try {
    message = await openpgp.readMessage({ armoredMessage: pgpBlock.ciphertext });
  } catch (e) {
    if (hasArgon2(e)) {
      return { text: null, html: null, badges: ['unsupported_alg'], attachments: [], subject: null };
    }
    return { text: null, html: null, badges: ['decrypt_failed'], attachments: [], subject: null };
  }

  let decrypted: openpgp.DecryptMessageResult;
  try {
    decrypted = await openpgp.decrypt({
      message,
      decryptionKeys: privateKeys,
      ...(publicKeys.length > 0 ? { verificationKeys: publicKeys } : {}),
      config: { allowUnauthenticatedMessages: false },
    });
  } catch (e) {
    if (hasArgon2(e)) {
      return { text: null, html: null, badges: ['unsupported_alg'], attachments: [], subject: null };
    }
    return { text: null, html: null, badges: ['decrypt_failed'], attachments: [], subject: null };
  }

  badges.push('encrypted');

  // Check signatures
  if (decrypted.signatures.length > 0) {
    badges.push('signed');
    const matchingKeys = publicKeys.filter(key =>
      decrypted.signatures.some(sig =>
        key.getKeyIDs().some(keyId => keyId.equals(sig.keyID))
      )
    );

    if (matchingKeys.length === 0) {
      badges.push('unverified');
    } else {
      let anyVerified = false;
      let anyFailed = false;
      for (const sig of decrypted.signatures) {
        try {
          await sig.verified;
          anyVerified = true;
        } catch {
          anyFailed = true;
        }
      }
      if (anyVerified) badges.push('verified');
      else if (anyFailed) badges.push('verification_failed');
      else badges.push('unverified');
    }
  }

  const decryptedText = typeof decrypted.data === 'string'
    ? decrypted.data
    : new TextDecoder().decode(decrypted.data as Uint8Array);

  // PGP/MIME plaintext is itself a MIME entity. Parse it in the browser: the
  // server cannot see inner bodies or attachments without the private key.
  const inner = parseDecryptedMime(decryptedText);
  if (inner) {
    return { text: inner.text, html: inner.html, badges, attachments: inner.attachments, subject: inner.subject };
  }

  // Inline PGP commonly decrypts to an ordinary body rather than a MIME entity.
  const isHtml = decryptedText.trimStart().toLowerCase().startsWith('<!doctype html') ||
    decryptedText.trimStart().toLowerCase().startsWith('<html');

  return {
    text: isHtml ? null : decryptedText,
    html: isHtml ? decryptedText : null,
    badges,
    attachments: [],
    subject: null,
  };
}

/**
 * Verify a PGP/MIME or inline signed message.
 */
export async function verifySignedMessage(
  rawMime: Uint8Array,
  publicKeys: openpgp.PublicKey[],
): Promise<DecryptBadge[]> {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawMime);
  const badges: DecryptBadge[] = ['signed'];

  const signedBlock = extractSignedBlock(mimeStr);
  const detached = extractDetachedSignedMime(mimeStr);
  if (!signedBlock && !detached) return badges;
  if (publicKeys.length === 0) return [...badges, 'unverified'];

  try {
    const result = detached
      ? await openpgp.verify({
        message: await openpgp.createMessage({ binary: new TextEncoder().encode(detached.text) }),
        signature: await openpgp.readSignature({ armoredSignature: detached.signature }),
        verificationKeys: publicKeys,
      })
      : await openpgp.verify({
        message: await openpgp.createCleartextMessage({ text: signedBlock!.text }),
        verificationKeys: publicKeys,
      });
    const hasMatchingKey = result.signatures.some(sig =>
      publicKeys.some(key => key.getKeyIDs().some(keyId => keyId.equals(sig.keyID)))
    );
    if (!hasMatchingKey) return [...badges, 'unverified'];

    let anyVerified = false;
    let anyFailed = false;
    for (const sig of result.signatures) {
      try {
        await sig.verified;
        anyVerified = true;
      } catch {
        anyFailed = true;
      }
    }
    if (anyVerified) badges.push('verified');
    else if (anyFailed) badges.push('verification_failed');
    else badges.push('unverified');
  } catch {
    badges.push('verification_failed');
  }

  return badges;
}

export async function verifiesWithKey(rawMime: Uint8Array, key: openpgp.PublicKey): Promise<boolean> {
  return (await verifySignedMessage(rawMime, [key])).includes('verified');
}

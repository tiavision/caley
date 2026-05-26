// openpgp.js wrappers for key import, decryption, and signature verification.
// CSP constraint: no 'unsafe-eval', no wasm — Argon2 S2K is explicitly rejected.

import * as openpgp from 'openpgp';

export type DecryptBadge =
  | 'encrypted'       // decrypted successfully
  | 'signed'          // signature present and key found
  | 'verified'        // signature cryptographically valid
  | 'unverified'      // signed but key not available
  | 'missing_key'     // encrypted but our private key not present
  | 'unsupported_alg' // Argon2 or other unsupported S2K
  | 'verification_failed' // signature invalid
  | 'decrypt_failed'; // decryption error

export interface DecryptResult {
  plaintext: string;
  isHtml: boolean;
  badges: DecryptBadge[];
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

/** Re-encrypt a private key with a new random passphrase (for vault storage). */
export async function reencryptPrivateKey(
  privateKey: openpgp.PrivateKey,
  newPassphrase: string,
): Promise<string> {
  const protected_ = await openpgp.encryptKey({ privateKey, passphrase: newPassphrase });
  return protected_.armor();
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

/** Return armored public key from a private key. */
export function extractPublicKey(privateKey: openpgp.PrivateKey): string {
  return privateKey.toPublic().armor();
}

/**
 * Decrypt a PGP/MIME or inline PGP message from raw MIME bytes.
 * privateKeys — unlocked in-memory private keys (tried in order)
 * rawMime     — the raw .eml bytes fetched from /api/email/raw
 * Returns the decrypted plaintext + signature status badges.
 */
export async function decryptMessage(
  privateKeys: openpgp.PrivateKey[],
  rawMime: Uint8Array,
): Promise<DecryptResult> {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawMime);
  const badges: DecryptBadge[] = [];

  // Extract the PGP block from the MIME structure.
  const pgpBlock = extractPgpBlock(mimeStr);
  if (!pgpBlock) {
    return { plaintext: mimeStr, isHtml: false, badges: [] };
  }

  let message: openpgp.Message<string | Uint8Array>;
  try {
    message = await openpgp.readMessage({ armoredMessage: pgpBlock.ciphertext });
  } catch (e) {
    if (hasArgon2(e)) {
      return { plaintext: '', isHtml: false, badges: ['unsupported_alg'] };
    }
    return { plaintext: '', isHtml: false, badges: ['decrypt_failed'] };
  }

  let decrypted: openpgp.DecryptMessageResult;
  try {
    decrypted = await openpgp.decrypt({
      message,
      decryptionKeys: privateKeys,
      config: { allowUnauthenticatedMessages: false },
    });
  } catch (e) {
    if (hasArgon2(e)) {
      return { plaintext: '', isHtml: false, badges: ['unsupported_alg'] };
    }
    return { plaintext: '', isHtml: false, badges: ['decrypt_failed'] };
  }

  badges.push('encrypted');

  // Check signatures
  if (decrypted.signatures.length > 0) {
    badges.push('signed');
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

  const plaintext = typeof decrypted.data === 'string'
    ? decrypted.data
    : new TextDecoder().decode(decrypted.data as Uint8Array);

  // Detect if decrypted payload is HTML (PGP/MIME inner part)
  const isHtml = plaintext.trimStart().toLowerCase().startsWith('<!doctype html') ||
    plaintext.trimStart().toLowerCase().startsWith('<html');

  return { plaintext, isHtml, badges };
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
  if (!signedBlock) return badges;

  try {
    const message = await openpgp.createCleartextMessage({ text: signedBlock.text });
    const result = await openpgp.verify({
      message,
      verificationKeys: publicKeys,
    });
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

// ── MIME parsing helpers ──────────────────────────────────────────────────────

interface PgpBlock {
  ciphertext: string;
}

/** Extract the PGP armored ciphertext from a MIME message string. */
function extractPgpBlock(mime: string): PgpBlock | null {
  const begin = mime.indexOf('-----BEGIN PGP MESSAGE-----');
  const end = mime.indexOf('-----END PGP MESSAGE-----');
  if (begin === -1 || end === -1) return null;
  return { ciphertext: mime.slice(begin, end + '-----END PGP MESSAGE-----'.length) };
}

interface SignedBlock {
  text: string;
  signature: string;
}

function extractSignedBlock(mime: string): SignedBlock | null {
  const begin = mime.indexOf('-----BEGIN PGP SIGNED MESSAGE-----');
  const end = mime.indexOf('-----END PGP SIGNATURE-----');
  if (begin === -1 || end === -1) return null;
  const full = mime.slice(begin, end + '-----END PGP SIGNATURE-----'.length);
  const sigBegin = full.indexOf('-----BEGIN PGP SIGNATURE-----');
  return {
    text: full.slice(0, sigBegin).trim(),
    signature: full.slice(sigBegin),
  };
}

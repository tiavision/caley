// openpgp.js wrappers for keys: import, generation, identities. Reading mail (decrypting, verifying) is
// steps.ts, for the steps sinter-mail's mail.mime asks for.

import * as openpgp from 'openpgp';

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

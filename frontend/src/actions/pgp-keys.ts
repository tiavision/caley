// `pgpKeys` actions: senders' public keys on the settings page (src/components/pgp-settings.html).
// keyFile() reads a key file for the import dialogs' textareas. Parsing a key needs openpgp (a lazy package, loaded by
// sinter on the first call); listing, storing, accepting and deleting are plain requests in the markup.
import { armorBinaryKey } from '../lib/pgp/keyring.ts';
import { keyFacts, parsePublicKey, publicKeyBody, type StoredPublicKey } from '../lib/pgp/public-keys.ts';
import type { KeyCandidate, KeyFacts } from '../lib/pgp-keys.ts';
import { m } from '@tiaforge-ui/sinter/messages';

/**
 * Reads an armored public key for the import dialog, which shows its fingerprint and user IDs before it is stored
 * (POST /pgp/key, trusted) for the address chosen there: the key's first, unless the user picks another of its own.
 */
export async function publicKey(armored: string): Promise<KeyCandidate> {
  const key = await parsePublicKey(armored);
  const facts = keyFacts(key);
  return { body: { ...publicKeyBody(key, 'imported'), email: facts.emails[0] ?? '' }, facts, sender: '', claimsAddress: true };
}

/** What a stored key says of itself, for the dialog that accepts it; rejects a record whose armor is another key. */
export async function storedKeyFacts(record: StoredPublicKey): Promise<KeyFacts> {
  const facts = keyFacts(await parsePublicKey(record.armored));
  if (facts.fingerprint !== record.fingerprint.toUpperCase()) throw new Error(m.pgp_storedKeyMismatch());
  return facts;
}

/**
 * The armored text of a chosen key file (.asc as it is, a binary .gpg/.pgp key armored here), for an import
 * dialog's textarea; '' when the picker was cancelled. Rejects when the file holds no OpenPGP key.
 */
export async function keyFile(file: File | undefined): Promise<string> {
  if (!file) return '';
  const bytes = new Uint8Array(await file.arrayBuffer());
  const text = new TextDecoder().decode(bytes);
  if (text.includes('-----BEGIN PGP ')) return text.trim();
  return armorBinaryKey(bytes);
}

// `pgpKeys` actions: trusted senders' public keys on the settings page (src/components/pgp-settings.html).
// keyFile() reads a key file for the import dialogs' textareas. Parsing a key needs openpgp (a lazy package, loaded by
// sinter on the first call); listing and deleting are plain requests in the markup.
import { armorBinaryKey } from '../lib/pgp/keyring.ts';
import { parsePublicKey, savePublicKey, type StoredPublicKey } from '../lib/pgp/public-keys.ts';

/** Reads an armored public key and stores it as trusted; returns the stored record for `publicKeys`. */
export async function importPublic(armored: string): Promise<StoredPublicKey> {
  return savePublicKey(await parsePublicKey(armored), 'imported');
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

// `pgpKeys` actions: trusted senders' public keys on the settings page (partials/pgp-settings.html).
// Parsing an armored key needs OpenPGP.js (loaded on first use); listing and deleting are plain requests in the markup.
import { loadPgp } from '../lib/pgp/load.ts';
import type { StoredPublicKey } from '../lib/pgp/public-keys.ts';

/** Reads an armored public key and stores it as trusted; returns the stored record for `publicKeys`. */
export async function importPublic(armored: string): Promise<StoredPublicKey> {
  const { parsePublicKey, savePublicKey } = await loadPgp();
  return savePublicKey(await parsePublicKey(armored), 'imported');
}

import * as openpgp from 'openpgp';
import { getFingerprint, getPrimaryEmail } from './keyring.ts';
import { apiFetch } from '../api.ts';

export interface StoredPublicKey {
  fingerprint: string;
  email: string;
  armored: string;
  is_own: boolean;
  source: string;
  trust: string;
}

export async function parsePublicKey(armored: string): Promise<openpgp.PublicKey> {
  return openpgp.readKey({ armoredKey: armored });
}

export function describePublicKey(key: openpgp.PublicKey): Pick<StoredPublicKey, 'fingerprint' | 'email' | 'armored'> {
  return {
    fingerprint: getFingerprint(key),
    email: getPrimaryEmail(key),
    armored: key.armor(),
  };
}

export async function listPublicKeys(): Promise<StoredPublicKey[]> {
  const res = await apiFetch('/api/pgp/keys');
  if (!res.ok) throw new Error('list public keys failed');
  return res.json() as Promise<StoredPublicKey[]>;
}

export async function readVerificationKeys(records: StoredPublicKey[]): Promise<openpgp.PublicKey[]> {
  const keys: openpgp.PublicKey[] = [];
  for (const record of records) {
    if (record.trust !== 'trusted') continue;
    try {
      keys.push(await parsePublicKey(record.armored));
    } catch {
      // Ignore corrupt stored records; they cannot be used for verification.
    }
  }
  return keys;
}

export type PublicKeySource = 'imported' | 'message_attachment' | 'harvested_attachment' | 'harvested_autocrypt';

/** The body of POST /api/pgp/key for a sender's key (stored as trusted). */
export interface PublicKeyBody { fingerprint: string; email: string; armored: string; is_own: false; source: PublicKeySource }

export function publicKeyBody(key: openpgp.PublicKey, source: PublicKeySource, associatedEmail?: string): PublicKeyBody {
  return { ...describePublicKey(key), ...(associatedEmail ? { email: associatedEmail } : {}), is_own: false, source };
}

/** Stores a sender's key found in the background (Autocrypt); the markup stores the keys a user imports. */
export async function savePublicKey(key: openpgp.PublicKey, source: PublicKeySource, associatedEmail?: string): Promise<void> {
  const res = await apiFetch('/api/pgp/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(publicKeyBody(key, source, associatedEmail)),
  });
  if (!res.ok) throw new Error('save public key failed');
}

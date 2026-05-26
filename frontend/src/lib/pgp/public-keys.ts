import * as openpgp from 'openpgp';
import { getFingerprint, getPrimaryEmail } from './keyring.ts';

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
  const res = await fetch('/api/pgp/keys');
  if (!res.ok) throw new Error('list public keys failed');
  return res.json() as Promise<StoredPublicKey[]>;
}

export async function listPublicKeysForEmail(email: string): Promise<StoredPublicKey[]> {
  const res = await fetch(`/api/pgp/keys?email=${encodeURIComponent(email)}`);
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

export async function savePublicKey(
  key: openpgp.PublicKey,
  source: 'imported' | 'message_attachment' | 'harvested_attachment' | 'harvested_autocrypt',
  associatedEmail?: string,
): Promise<StoredPublicKey> {
  const fields = { ...describePublicKey(key), ...(associatedEmail ? { email: associatedEmail } : {}) };
  const record: StoredPublicKey = {
    ...fields,
    is_own: false,
    source,
    trust: 'trusted',
  };
  const res = await fetch('/api/pgp/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(record),
  });
  if (!res.ok) throw new Error('save public key failed');
  return record;
}

export async function deletePublicKey(fingerprint: string): Promise<void> {
  const res = await fetch('/api/pgp/key', {
    method: 'DELETE',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ fingerprint }),
  });
  if (!res.ok) throw new Error('delete public key failed');
}

import * as openpgp from 'openpgp';
import { getFingerprint, getPrimaryEmail } from './keyring.ts';
import { apiFetch } from '../api.ts';
import type { KeyFacts } from '../pgp-keys.ts';

/**
 * GET /api/pgp/keys: a stored key. `trust` is `trusted` (imported, or accepted by the user) or `unverified` (found in
 * mail: counts for nothing until accepted); a key is trusted only for `email`, the address it was stored for.
 */
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

/** The key's fingerprint, its user IDs as text, and the addresses among them (lowercase, each once). */
export function keyFacts(key: openpgp.PublicKey): KeyFacts {
  const userIds = key.users.map((u) => u.userID?.userID ?? '').filter(Boolean);
  const emails = [...new Set(key.users.map((u) => u.userID?.email?.trim().toLowerCase() ?? '').filter(Boolean))];
  return { fingerprint: getFingerprint(key), userIds, emails };
}

export async function listPublicKeys(): Promise<StoredPublicKey[]> {
  const res = await apiFetch('/api/pgp/keys');
  if (!res.ok) throw new Error('list public keys failed');
  return res.json() as Promise<StoredPublicKey[]>;
}

/** The keys that count: the ones the user trusts. */
export function trustedKeys(records: StoredPublicKey[]): StoredPublicKey[] {
  return records.filter((r) => r.trust === 'trusted');
}

export async function readVerificationKeys(records: StoredPublicKey[]): Promise<openpgp.PublicKey[]> {
  const keys: openpgp.PublicKey[] = [];
  for (const record of trustedKeys(records)) {
    try {
      keys.push(await parsePublicKey(record.armored));
    } catch {
      // Ignore corrupt stored records; they cannot be used for verification.
    }
  }
  return keys;
}

/**
 * Where a posted key came from; the server decides its trust from it: `imported` and `message_attachment` (the user
 * chose to store it, after seeing it) are trusted, `harvested_*` (found in the background) wait to be accepted.
 */
export type PublicKeySource = 'imported' | 'message_attachment' | 'harvested_attachment' | 'harvested_autocrypt';

/** The body of POST /api/pgp/key for a sender's key. */
export interface PublicKeyBody { fingerprint: string; email: string; armored: string; is_own: false; source: PublicKeySource }

export function publicKeyBody(key: openpgp.PublicKey, source: PublicKeySource, associatedEmail?: string): PublicKeyBody {
  return { ...describePublicKey(key), ...(associatedEmail ? { email: associatedEmail } : {}), is_own: false, source };
}

/**
 * Stores a sender's key found in the background (Autocrypt): the server keeps it `unverified` until the user accepts
 * it, and never lets it change a key the user trusts. The markup stores the keys a user imports.
 */
export async function savePublicKey(key: openpgp.PublicKey, source: PublicKeySource, associatedEmail?: string): Promise<void> {
  const res = await apiFetch('/api/pgp/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(publicKeyBody(key, source, associatedEmail)),
  });
  if (!res.ok) throw new Error('save public key failed');
}

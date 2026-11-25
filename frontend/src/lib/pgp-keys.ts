// What the pages show of OpenPGP public keys before one is trusted (src/components/pgp-key-facts.html, the settings'
// key list, the reader's key import, compose's waiting keys): pure, no openpgp (every page imports the helpers).
// A key counts (signatures, encryption) only once the user trusted it: imported it, or accepted one found in mail,
// after seeing its full fingerprint and user IDs; and only for the address it was stored for.
import { m } from '@tiaforge-ui/sinter/messages';
import type { PublicKeyBody, StoredPublicKey } from './pgp/public-keys.ts';

/** What a key says of itself: its fingerprint and user IDs (`Name <addr>`), and the addresses among them. */
export interface KeyFacts { fingerprint: string; userIds: string[]; emails: string[] }

/**
 * A key about to be stored as trusted (the settings' import, the reader's import button): the body of POST /pgp/key,
 * what the key says of itself, and whether it claims the address it is offered for (`email` set to it) or the user
 * has to choose one of `facts.emails`.
 */
export interface KeyCandidate { body: PublicKeyBody; facts: KeyFacts; claimsAddress: boolean }

/** A key found in mail that waits to be accepted; `change`: another key is trusted for its address. */
export interface PendingKey { fingerprint: string; email: string; userIds: string[]; change: boolean }

/** A fingerprint in groups of four, as people compare them: `ABCD 1234 …`. */
export function fingerprintText(fingerprint: string): string {
  return fingerprint.toUpperCase().replace(/[^0-9A-F]/g, '').replace(/(.{4})(?=.)/g, '$1 ');
}

/** The stored key's state: 'own', 'trusted', 'waiting' (found in mail) or 'change' (waiting, while another key is trusted for its address). */
export function keyState(key: StoredPublicKey, all: StoredPublicKey[]): 'own' | 'trusted' | 'waiting' | 'change' {
  if (key.is_own) return 'own';
  if (key.trust === 'trusted') return 'trusted';
  return keyChanges(key, all) ? 'change' : 'waiting';
}

/** Whether another key, trusted and not the user's own, is stored for this key's address. */
export function keyChanges(key: Pick<StoredPublicKey, 'fingerprint' | 'email'>, all: StoredPublicKey[]): boolean {
  const email = key.email.trim().toLowerCase();
  return !!email && all.some((k) => !k.is_own && k.trust === 'trusted' && k.email.trim().toLowerCase() === email &&
    k.fingerprint.toUpperCase() !== key.fingerprint.toUpperCase());
}

export function keyStateLabel(state: 'own' | 'trusted' | 'waiting' | 'change'): string {
  switch (state) {
    case 'trusted': return m.pgp_keyTrusted();
    case 'waiting': return m.pgp_keyWaiting();
    case 'change': return m.pgp_keyChanged();
    default: return '';
  }
}

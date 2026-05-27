// In-memory state for the unlocked private key vault. The vault remains open
// for this page lifetime until the user explicitly locks it or navigates away.

import type * as openpgp from 'openpgp';

let wrappingKey: CryptoKey | null = null;
const privateKeys = new Map<string, openpgp.PrivateKey>();

export function openVault(key: CryptoKey, keys: Map<string, openpgp.PrivateKey>) {
  wrappingKey = key;
  privateKeys.clear();
  for (const [fingerprint, privateKey] of keys) {
    privateKeys.set(fingerprint, privateKey);
  }
}

export function addKey(fingerprint: string, key: openpgp.PrivateKey) {
  if (!wrappingKey) throw new Error('vault is locked');
  privateKeys.set(fingerprint, key);
}

export function removeKey(fingerprint: string) {
  privateKeys.delete(fingerprint);
}

export function getWrappingKey(): CryptoKey | null {
  return wrappingKey;
}

export function getKeys(): openpgp.PrivateKey[] {
  return [...privateKeys.values()];
}

export function lock() {
  wrappingKey = null;
  privateKeys.clear();
}

export function isUnlocked(): boolean {
  return wrappingKey !== null;
}

// IndexedDB persistence for one browser-local PGP private key vault.
// The vault metadata supplies one WebAuthn-derived wrapping key for all records.

import type * as openpgp from 'openpgp';
import { aesDecrypt, aesEncrypt, b64uDecode, b64uEncode, randomBytes } from './kek.ts';
import { getFingerprint, importPrivateKey } from './keyring.ts';

export interface VaultMetadata {
  kind: 'metadata';
  version: 2;
  credentialId: string;
  prfSalt: string;
  hkdfSalt: string;
  deviceLabel: string;
}

export interface VaultKeyRecord {
  kind: 'key';
  iv: string;
  ciphertext: string;
  fingerprint: string;
}

export interface VaultData {
  metadata: VaultMetadata;
  keys: VaultKeyRecord[];
}

const DB_NAME = 'caley-pgp';
const STORE = 'vault';
const VERSION = 2;
const METADATA_ID = 'metadata';
const KEY_PREFIX = 'key:';

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, VERSION);
    req.onupgradeneeded = () => {
      if (req.result.objectStoreNames.contains(STORE)) {
        req.result.deleteObjectStore(STORE);
      }
      req.result.createObjectStore(STORE);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function readVault(): Promise<VaultData | null> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readonly');
    const req = tx.objectStore(STORE).getAll();
    req.onsuccess = () => {
      const records = (req.result as Array<VaultMetadata | VaultKeyRecord>) ?? [];
      const metadata = records.find((record): record is VaultMetadata => record.kind === 'metadata');
      if (!metadata) {
        resolve(null);
        return;
      }
      resolve({
        metadata,
        keys: records.filter((record): record is VaultKeyRecord => record.kind === 'key'),
      });
    };
    req.onerror = () => reject(req.error);
  });
}

function encryptPrivateKey(
  wrappingKey: CryptoKey,
  fingerprint: string,
  privateKey: openpgp.PrivateKey,
): Promise<VaultKeyRecord> {
  const iv = randomBytes(12);
  const payload = new TextEncoder().encode(JSON.stringify({ armored: privateKey.armor() }));
  return aesEncrypt(wrappingKey, iv, payload).then(ciphertext => ({
    kind: 'key',
    iv: b64uEncode(iv),
    ciphertext: b64uEncode(ciphertext),
    fingerprint,
  }));
}

export async function createVault(
  metadata: VaultMetadata,
  wrappingKey: CryptoKey,
  fingerprint: string,
  privateKey: openpgp.PrivateKey,
): Promise<VaultKeyRecord> {
  const record = await encryptPrivateKey(wrappingKey, fingerprint, privateKey);
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    const store = tx.objectStore(STORE);
    store.clear();
    store.put(metadata, METADATA_ID);
    store.put(record, KEY_PREFIX + fingerprint);
    tx.oncomplete = () => resolve(record);
    tx.onerror = () => reject(tx.error);
  });
}

export async function writePrivateKey(
  wrappingKey: CryptoKey,
  fingerprint: string,
  privateKey: openpgp.PrivateKey,
): Promise<VaultKeyRecord> {
  const record = await encryptPrivateKey(wrappingKey, fingerprint, privateKey);
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).put(record, KEY_PREFIX + fingerprint);
    tx.oncomplete = () => resolve(record);
    tx.onerror = () => reject(tx.error);
  });
}

export async function decryptPrivateKeys(
  wrappingKey: CryptoKey,
  records: VaultKeyRecord[],
): Promise<Map<string, openpgp.PrivateKey>> {
  const keys = new Map<string, openpgp.PrivateKey>();
  for (const record of records) {
    const plainBuf = await aesDecrypt(wrappingKey, b64uDecode(record.iv), b64uDecode(record.ciphertext));
    const { armored } = JSON.parse(new TextDecoder().decode(plainBuf)) as { armored: string };
    const key = await importPrivateKey(armored, '');
    if (getFingerprint(key) !== record.fingerprint) throw new Error('vault fingerprint mismatch');
    keys.set(record.fingerprint, key);
  }
  return keys;
}

export async function deletePrivateKey(fingerprint: string): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).delete(KEY_PREFIX + fingerprint);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

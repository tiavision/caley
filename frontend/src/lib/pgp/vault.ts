// IndexedDB vault for the per-device PGP key record.
// The vault record stores everything needed to re-derive the KEK and decrypt
// the stored private key blob, but never the private key or PRF output itself.

export interface VaultRecord {
  credentialId: string;   // base64url WebAuthn credential ID
  prfSalt: string;        // base64url 32-byte PRF input salt
  hkdfSalt: string;       // base64url 32-byte HKDF salt
  iv: string;             // base64url 12-byte AES-GCM nonce
  ciphertext: string;     // base64url encrypted(private key armored + random passphrase)
  fingerprint: string;    // uppercase hex fingerprint
  deviceLabel: string;
}

const DB_NAME = 'caley-pgp';
const STORE = 'vault';
const KEY = 'record';
const VERSION = 1;

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, VERSION);
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export async function readVault(): Promise<VaultRecord | null> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readonly');
    const req = tx.objectStore(STORE).get(KEY);
    req.onsuccess = () => resolve(req.result ?? null);
    req.onerror = () => reject(req.error);
  });
}

export async function writeVault(record: VaultRecord): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).put(record, KEY);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export async function clearVault(): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).delete(KEY);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

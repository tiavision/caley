// IndexedDB vault for per-device PGP key records.
// Each enrolled private key gets its own record keyed by fingerprint.
// All records on the same device share the same credentialId and prfSalt
// so that one WebAuthn touch unlocks all keys.

export interface VaultRecord {
  credentialId: string;   // base64url WebAuthn credential ID
  prfSalt: string;        // base64url 32-byte PRF input salt (shared per device)
  hkdfSalt: string;       // base64url 32-byte HKDF salt (unique per key)
  iv: string;             // base64url 12-byte AES-GCM nonce
  ciphertext: string;     // base64url encrypted(private key armored + random passphrase)
  fingerprint: string;    // uppercase hex fingerprint
  deviceLabel: string;
}

const DB_NAME = 'caley-pgp';
const STORE = 'vault';
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

export async function readAllVaults(): Promise<VaultRecord[]> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readonly');
    const req = tx.objectStore(STORE).getAll();
    req.onsuccess = () => resolve((req.result as VaultRecord[]) ?? []);
    req.onerror = () => reject(req.error);
  });
}

// Write a vault record using the fingerprint as the IDB key.
export async function writeVault(record: VaultRecord): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).put(record, record.fingerprint);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

// Delete a vault record by fingerprint. Also handles the legacy single-record
// format where the record was stored under the fixed key 'record'.
export async function deleteVault(fingerprint: string): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    const store = tx.objectStore(STORE);
    store.delete(fingerprint);
    const legacyReq = store.get('record');
    legacyReq.onsuccess = () => {
      const legacy = legacyReq.result as VaultRecord | undefined;
      if (legacy?.fingerprint === fingerprint) store.delete('record');
    };
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export async function clearVault(): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).clear();
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

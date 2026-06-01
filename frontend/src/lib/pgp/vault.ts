// Client-side cryptography and transport for the synchronized encrypted PGP vault.
// The server stores ciphertext and public device material, but cannot author valid manifests.

import type * as openpgp from 'openpgp';
import { aesDecrypt, aesEncrypt, b64uDecode, b64uEncode, deriveKek, deriveSharedKek, importAesKey, randomBytes } from './kek.ts';
import { apiFetch } from '../api.ts';
import { getFingerprint, importPrivateKey } from './keyring.ts';

export interface EncryptedBlob {
  iv: string;
  ciphertext: string;
}

export interface VaultKeyRecord extends EncryptedBlob {
  fingerprint: string;
}

export interface VaultWrap extends EncryptedBlob {
  cred_id: string;
  ephemeral_public_jwk: JsonWebKey;
  hkdf_salt: string;
}

export interface DeviceRecord {
  cred_id: string;
  device_label: string | null;
  status: 'pending' | 'active';
  public_jwk: JsonWebKey;
  encrypted_private_jwk: string;
  private_iv: string;
  prf_salt: string;
  hkdf_salt: string;
  pairing_nonce: string;
  wrap: VaultWrap | null;
}

export interface DeviceBundle {
  cred_id: string;
  public_jwk: JsonWebKey;
  encrypted_private_jwk: string;
  private_iv: string;
  prf_salt: string;
  hkdf_salt: string;
  pairing_nonce: string;
}

export interface VaultData {
  revision: number;
  manifest: EncryptedBlob;
  keys: VaultKeyRecord[];
  devices: DeviceRecord[];
}

export interface ManifestKey {
  fingerprint: string;
  commitment: string;
}

export interface ManifestDevice {
  cred_id: string;
  device_label: string;
  public_jwk: JsonWebKey;
  bundle_commitment: string;
  wrap_commitment: string;
}

export interface VaultManifest {
  format: 2;
  vault_id: string;
  epoch: number;
  previous_commitment: string | null;
  keys: ManifestKey[];
  devices: ManifestDevice[];
}

interface VaultCheckpoint {
  epoch: number;
  commitment: string;
}

const LOCAL_DEVICE_IDS_KEY = 'caley-pgp-local-device-credential-ids';
const CHECKPOINTS_KEY = 'caley-pgp-vault-checkpoints-v2';
const SAS_EMOJI = [
  '😀', '😎', '🥳', '🤖', '👻', '🧠', '👀', '👍',
  '🦊', '🐼', '🐙', '🦋', '🐝', '🐳', '🦉', '🦖',
  '🍏', '🍋', '🍓', '🍇', '🥑', '🥕', '🍕', '🍪',
  '⚽', '🏀', '🎾', '🎲', '🎸', '🎧', '🎁', '🎈',
  '🚗', '🚲', '🚀', '🚂', '✈️', '⛵', '🏠', '🏰',
  '☀️', '🌙', '⭐', '☁️', '❄️', '🔥', '🌈', '🌊',
  '🌲', '🌻', '🍀', '🌵', '💎', '🔑', '🔒', '🔔',
  '❤️', '💙', '💚', '💜', '✅', '🔷', '🔶', '⚓',
] as const;

function readLocalDeviceIds(): string[] {
  try {
    const value = localStorage.getItem(LOCAL_DEVICE_IDS_KEY);
    const parsed: unknown = value ? JSON.parse(value) : [];
    return Array.isArray(parsed) ? parsed.filter((id): id is string => typeof id === 'string') : [];
  } catch {
    return [];
  }
}

export function rememberLocalDevice(credentialId: string): void {
  try {
    const ids = readLocalDeviceIds().filter(id => id !== credentialId);
    localStorage.setItem(LOCAL_DEVICE_IDS_KEY, JSON.stringify([credentialId, ...ids]));
  } catch {
    // Storage may be unavailable; settings can still unlock devices explicitly.
  }
}

export function clearLocalVaultState(): void {
  try {
    localStorage.removeItem(LOCAL_DEVICE_IDS_KEY);
    localStorage.removeItem(CHECKPOINTS_KEY);
  } catch {
    // A server-side reset remains valid if local storage is unavailable.
  }
}

export function localActiveDevice(vault: VaultData): DeviceRecord | undefined {
  return localActiveDevices(vault)[0];
}

export function localActiveDevices(vault: VaultData): DeviceRecord[] {
  return localDevices(vault).filter(device => device.status === 'active' && device.wrap);
}

export function localPendingDevices(vault: VaultData): DeviceRecord[] {
  return localDevices(vault).filter(device => device.status === 'pending');
}

function localDevices(vault: VaultData): DeviceRecord[] {
  const byId = new Map(vault.devices.map(device => [device.cred_id, device]));
  return readLocalDeviceIds()
    .map(credentialId => byId.get(credentialId))
    .filter((device): device is DeviceRecord => device !== undefined);
}

async function jsonRequest(path: string, init?: RequestInit): Promise<Response> {
  const response = await apiFetch(path, init);
  if (!response.ok && response.status !== 204) {
    throw new Error(`${path} failed: ${await response.text()}`);
  }
  return response;
}

export async function readVault(): Promise<VaultData | null> {
  const response = await jsonRequest('/api/pgp/vault');
  if (response.status === 204) return null;
  return response.json() as Promise<VaultData>;
}

export async function resetVault(): Promise<void> {
  await jsonRequest('/api/pgp/vault', { method: 'DELETE' });
}

export async function generateVaultKey(): Promise<{ raw: Uint8Array; key: CryptoKey }> {
  const raw = randomBytes(32);
  return { raw, key: await importAesKey(raw) };
}

export function eraseBytes(bytes: Uint8Array): void {
  bytes.fill(0);
}

export async function encryptPrivateKey(
  vaultKey: CryptoKey,
  fingerprint: string,
  privateKey: openpgp.PrivateKey,
): Promise<VaultKeyRecord> {
  const iv = randomBytes(12);
  const payload = new TextEncoder().encode(JSON.stringify({ armored: privateKey.armor() }));
  return {
    fingerprint,
    iv: b64uEncode(iv),
    ciphertext: b64uEncode(await aesEncrypt(vaultKey, iv, payload)),
  };
}

export async function decryptPrivateKeys(
  vaultKey: CryptoKey,
  records: VaultKeyRecord[],
): Promise<Map<string, openpgp.PrivateKey>> {
  const keys = new Map<string, openpgp.PrivateKey>();
  for (const record of records) {
    const plain = await aesDecrypt(vaultKey, b64uDecode(record.iv), b64uDecode(record.ciphertext));
    const { armored } = JSON.parse(new TextDecoder().decode(plain)) as { armored: string };
    const key = await importPrivateKey(armored, '');
    if (getFingerprint(key) !== record.fingerprint) throw new Error('vault fingerprint mismatch');
    keys.set(record.fingerprint, key);
  }
  return keys;
}

export async function encryptManifest(vaultKey: CryptoKey, manifest: VaultManifest): Promise<EncryptedBlob> {
  const iv = randomBytes(12);
  const plaintext = new TextEncoder().encode(JSON.stringify(manifest));
  return { iv: b64uEncode(iv), ciphertext: b64uEncode(await aesEncrypt(vaultKey, iv, plaintext)) };
}

export async function decryptManifest(vaultKey: CryptoKey, blob: EncryptedBlob): Promise<VaultManifest> {
  const plaintext = await aesDecrypt(vaultKey, b64uDecode(blob.iv), b64uDecode(blob.ciphertext));
  const manifest = JSON.parse(new TextDecoder().decode(plaintext)) as Partial<VaultManifest>;
  if (manifest.format !== 2 || typeof manifest.vault_id !== 'string' || typeof manifest.epoch !== 'number'
    || !Array.isArray(manifest.keys) || !Array.isArray(manifest.devices)) {
    throw new Error('legacy or invalid encrypted vault; enroll a new vault');
  }
  return manifest as VaultManifest;
}

function canonicalPublicJwk(key: JsonWebKey): string {
  return JSON.stringify({ crv: key.crv, kty: key.kty, x: key.x, y: key.y });
}

function canonicalDeviceBundle(device: DeviceBundle | DeviceRecord): string {
  return JSON.stringify({
    cred_id: device.cred_id,
    public_jwk: canonicalPublicJwk(device.public_jwk),
    encrypted_private_jwk: device.encrypted_private_jwk,
    private_iv: device.private_iv,
    prf_salt: device.prf_salt,
    hkdf_salt: device.hkdf_salt,
    pairing_nonce: device.pairing_nonce,
  });
}

function canonicalWrap(wrap: VaultWrap): string {
  return JSON.stringify({
    cred_id: wrap.cred_id,
    ephemeral_public_jwk: canonicalPublicJwk(wrap.ephemeral_public_jwk),
    hkdf_salt: wrap.hkdf_salt,
    iv: wrap.iv,
    ciphertext: wrap.ciphertext,
  });
}

async function digestCommitment(domain: string, content: string): Promise<string> {
  const bytes = new TextEncoder().encode(`${domain}|${content}`);
  return b64uEncode(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)));
}

export async function keyCommitment(record: VaultKeyRecord): Promise<string> {
  return digestCommitment('caley-vault-key-v2', JSON.stringify({
    fingerprint: record.fingerprint,
    iv: record.iv,
    ciphertext: record.ciphertext,
  }));
}

async function deviceBundleCommitment(device: DeviceBundle | DeviceRecord): Promise<string> {
  return digestCommitment('caley-device-bundle-v2', canonicalDeviceBundle(device));
}

async function wrapCommitment(wrap: VaultWrap): Promise<string> {
  return digestCommitment('caley-vault-wrap-v2', canonicalWrap(wrap));
}

export async function manifestDevice(
  device: DeviceBundle | DeviceRecord,
  label: string,
  wrap: VaultWrap,
): Promise<ManifestDevice> {
  return {
    cred_id: device.cred_id,
    device_label: label,
    public_jwk: device.public_jwk,
    bundle_commitment: await deviceBundleCommitment(device),
    wrap_commitment: await wrapCommitment(wrap),
  };
}

export async function updatedManifestDevice(device: ManifestDevice, wrap: VaultWrap): Promise<ManifestDevice> {
  return { ...device, wrap_commitment: await wrapCommitment(wrap) };
}

export function createVaultId(): string {
  return b64uEncode(randomBytes(32));
}

export async function manifestCommitment(blob: EncryptedBlob): Promise<string> {
  // Do not hash object insertion order: API JSON parsing may reconstruct this
  // object in a different field order from the locally encrypted value.
  return digestCommitment('caley-vault-state-v2', JSON.stringify({
    iv: blob.iv,
    ciphertext: blob.ciphertext,
  }));
}

function checkpointStorageKey(vaultId: string, credentialId: string): string {
  return `${vaultId}|${credentialId}`;
}

function readCheckpoints(): Record<string, VaultCheckpoint> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(CHECKPOINTS_KEY) ?? '{}');
    return typeof parsed === 'object' && parsed !== null ? parsed as Record<string, VaultCheckpoint> : {};
  } catch {
    return {};
  }
}

export async function rememberVaultCheckpoint(
  manifest: VaultManifest,
  encryptedManifest: EncryptedBlob,
  credentialId: string,
): Promise<void> {
  try {
    const checkpoints = readCheckpoints();
    checkpoints[checkpointStorageKey(manifest.vault_id, credentialId)] = {
      epoch: manifest.epoch,
      commitment: await manifestCommitment(encryptedManifest),
    };
    localStorage.setItem(CHECKPOINTS_KEY, JSON.stringify(checkpoints));
  } catch {
    // Checkpoints improve rollback detection but local storage is not always available.
  }
}

export async function validateVault(
  vault: VaultData,
  vaultKey: CryptoKey,
  credentialId: string,
): Promise<VaultManifest> {
  const manifest = await decryptManifest(vaultKey, vault.manifest);
  const manifestKeys = new Map(manifest.keys.map(record => [record.fingerprint, record.commitment]));
  if (manifestKeys.size !== manifest.keys.length || manifestKeys.size !== vault.keys.length) {
    throw new Error('vault key set mismatch');
  }
  for (const record of vault.keys) {
    if (manifestKeys.get(record.fingerprint) !== await keyCommitment(record)) {
      throw new Error('vault key commitment mismatch');
    }
  }

  const activeDevices = vault.devices.filter(device => device.status === 'active');
  const manifestDevices = new Map(manifest.devices.map(device => [device.cred_id, device]));
  if (manifestDevices.size !== manifest.devices.length || activeDevices.length !== manifest.devices.length
    || activeDevices.some(device => !device.wrap)) {
    throw new Error('vault device set mismatch');
  }
  for (const device of activeDevices) {
    const authenticated = manifestDevices.get(device.cred_id);
    if (!authenticated || authenticated.bundle_commitment !== await deviceBundleCommitment(device)
      || authenticated.wrap_commitment !== await wrapCommitment(device.wrap!)) {
      throw new Error('vault device commitment mismatch');
    }
  }
  if (!manifestDevices.has(credentialId)) throw new Error('local device is not authenticated');

  const stateCommitment = await manifestCommitment(vault.manifest);
  const checkpoint = readCheckpoints()[checkpointStorageKey(manifest.vault_id, credentialId)];
  if (checkpoint && (manifest.epoch < checkpoint.epoch
    || (manifest.epoch === checkpoint.epoch && checkpoint.commitment !== stateCommitment))) {
    throw new Error('vault rollback or fork detected');
  }
  if (checkpoint && manifest.epoch === checkpoint.epoch + 1
    && manifest.previous_commitment !== checkpoint.commitment) {
    throw new Error('vault update does not extend the locally accepted state');
  }
  await rememberVaultCheckpoint(manifest, vault.manifest, credentialId);
  return manifest;
}

export async function createDeviceBundle(
  credentialId: string,
  prfSalt: string,
  prfOutput: ArrayBuffer,
): Promise<DeviceBundle> {
  const keys = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']);
  const publicJwk = await crypto.subtle.exportKey('jwk', keys.publicKey);
  const privateJwk = await crypto.subtle.exportKey('jwk', keys.privateKey);
  const hkdfSalt = randomBytes(32);
  const privateIv = randomBytes(12);
  const deviceKek = await deriveKek(prfOutput, hkdfSalt);
  const privateBytes = new TextEncoder().encode(JSON.stringify(privateJwk));
  return {
    cred_id: credentialId,
    public_jwk: publicJwk,
    encrypted_private_jwk: b64uEncode(await aesEncrypt(deviceKek, privateIv, privateBytes)),
    private_iv: b64uEncode(privateIv),
    prf_salt: prfSalt,
    hkdf_salt: b64uEncode(hkdfSalt),
    pairing_nonce: b64uEncode(randomBytes(32)),
  };
}

async function decryptDevicePrivateKey(device: DeviceRecord, prfOutput: ArrayBuffer): Promise<CryptoKey> {
  const deviceKek = await deriveKek(prfOutput, b64uDecode(device.hkdf_salt));
  const plaintext = await aesDecrypt(deviceKek, b64uDecode(device.private_iv), b64uDecode(device.encrypted_private_jwk));
  const jwk = JSON.parse(new TextDecoder().decode(plaintext)) as JsonWebKey;
  return crypto.subtle.importKey('jwk', jwk, { name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
}

export async function wrapVaultKey(rawVaultKey: Uint8Array, device: Pick<DeviceRecord | ManifestDevice, 'cred_id' | 'public_jwk'>): Promise<VaultWrap> {
  const recipient = await crypto.subtle.importKey('jwk', device.public_jwk, { name: 'ECDH', namedCurve: 'P-256' }, false, []);
  const ephemeral = await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']);
  const shared = await crypto.subtle.deriveBits({ name: 'ECDH', public: recipient }, ephemeral.privateKey, 256);
  const salt = randomBytes(32);
  const iv = randomBytes(12);
  const wrappingKey = await deriveSharedKek(shared, salt);
  return {
    cred_id: device.cred_id,
    ephemeral_public_jwk: await crypto.subtle.exportKey('jwk', ephemeral.publicKey),
    hkdf_salt: b64uEncode(salt),
    iv: b64uEncode(iv),
    ciphertext: b64uEncode(await aesEncrypt(wrappingKey, iv, rawVaultKey)),
  };
}

export async function unwrapVaultKey(device: DeviceRecord, prfOutput: ArrayBuffer): Promise<{ raw: Uint8Array; key: CryptoKey }> {
  if (!device.wrap) throw new Error('device is not approved');
  const privateKey = await decryptDevicePrivateKey(device, prfOutput);
  const ephemeral = await crypto.subtle.importKey(
    'jwk',
    device.wrap.ephemeral_public_jwk,
    { name: 'ECDH', namedCurve: 'P-256' },
    false,
    [],
  );
  const shared = await crypto.subtle.deriveBits({ name: 'ECDH', public: ephemeral }, privateKey, 256);
  const wrappingKey = await deriveSharedKek(shared, b64uDecode(device.wrap.hkdf_salt));
  const raw = new Uint8Array(await aesDecrypt(wrappingKey, b64uDecode(device.wrap.iv), b64uDecode(device.wrap.ciphertext)));
  return { raw, key: await importAesKey(raw) };
}

export async function pairingSas(device: DeviceBundle | DeviceRecord): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(`caley-pair-sas-v2|${canonicalDeviceBundle(device)}`),
  ));
  let accumulator = 0;
  let bits = 0;
  let byte = 0;
  const symbols: string[] = [];
  while (symbols.length < 12) {
    while (bits < 6) {
      accumulator = (accumulator << 8) | digest[byte++];
      bits += 8;
    }
    bits -= 6;
    symbols.push(SAS_EMOJI[(accumulator >>> bits) & 0x3f]);
  }
  return [symbols.slice(0, 4), symbols.slice(4, 8), symbols.slice(8, 12)]
    .map(group => group.join(' '))
    .join('   ');
}

export async function initializeVault(
  manifest: EncryptedBlob,
  key: VaultKeyRecord,
  device: DeviceBundle,
  wrap: VaultWrap,
): Promise<void> {
  await jsonRequest('/api/pgp/vault/initialize', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ manifest, key, device, wrap }),
  });
}

export async function storePendingDevice(device: DeviceBundle): Promise<void> {
  await jsonRequest('/api/pgp/devices/pending', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ device }),
  });
}

export interface VaultUpdate {
  expected_revision: number;
  manifest: EncryptedBlob;
  upsert_keys?: VaultKeyRecord[];
  delete_keys?: string[];
  approve_device?: { cred_id: string; wrap: VaultWrap };
  remove_device?: string;
  replace_wraps?: VaultWrap[];
}

export async function updateVault(update: VaultUpdate): Promise<number> {
  const response = await jsonRequest('/api/pgp/vault', {
    method: 'PUT', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(update),
  });
  const result = await response.json() as { revision: number };
  return result.revision;
}

// Client-side cryptography and transport for the synchronized encrypted PGP vault.
// The server stores only ciphertext and public device material.

import type * as openpgp from 'openpgp';
import { aesDecrypt, aesEncrypt, b64uDecode, b64uEncode, deriveKek, deriveSharedKek, importAesKey, randomBytes } from './kek.ts';
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

export interface ManifestDevice {
  cred_id: string;
  device_label: string;
  public_jwk: JsonWebKey;
}

export interface VaultManifest {
  version: 1;
  devices: ManifestDevice[];
}

const LOCAL_DEVICE_IDS_KEY = 'caley-pgp-local-device-credential-ids';

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

export function localActiveDevice(vault: VaultData): DeviceRecord | undefined {
  const activeById = new Map(
    vault.devices
      .filter(device => device.status === 'active' && device.wrap)
      .map(device => [device.cred_id, device]),
  );
  return readLocalDeviceIds()
    .map(credentialId => activeById.get(credentialId))
    .find((device): device is DeviceRecord => device !== undefined);
}

async function jsonRequest(path: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(path, init);
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

export async function generateVaultKey(): Promise<{ raw: Uint8Array; key: CryptoKey }> {
  const raw = randomBytes(32);
  return { raw, key: await importAesKey(raw) };
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
  return JSON.parse(new TextDecoder().decode(plaintext)) as VaultManifest;
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

export async function wrapVaultKey(rawVaultKey: Uint8Array, device: Pick<DeviceRecord, 'cred_id' | 'public_jwk'>): Promise<VaultWrap> {
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

function canonicalPublicJwk(key: JsonWebKey): string {
  return JSON.stringify({ crv: key.crv, kty: key.kty, x: key.x, y: key.y });
}

export async function pairingCode(device: Pick<DeviceRecord | DeviceBundle, 'cred_id' | 'public_jwk' | 'pairing_nonce'>): Promise<string> {
  const data = `caley-pair-v1|${device.cred_id}|${canonicalPublicJwk(device.public_jwk)}|${device.pairing_nonce}`;
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(data)));
  const value = (((digest[0] << 24) >>> 0) + (digest[1] << 16) + (digest[2] << 8) + digest[3]) % 100000000;
  return value.toString().padStart(8, '0');
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

// The steps of the `pgpVault` actions (src/actions/pgp-vault.ts loads this module, with openpgp, on first use): the synced OpenPGP private-key vault of the settings page (partials/pgp-settings.html).
// WebAuthn (PRF) and the vault crypto are src/lib/pgp/*; this module runs one user step each and returns a
// VaultView, which the page upserts into its `vault` resource. Private keys and the vault key never leave
// this browser and never enter sinter state: they live in src/lib/pgp/session.ts (unlocked keys) and here
// (the decrypted manifest, a key being imported). The server only ever sees ciphertext and public data.
import type * as openpgp from 'openpgp';
import {
  readVault,
  resetVault,
  generateVaultKey,
  createDeviceBundle,
  wrapVaultKey,
  unwrapVaultKey,
  pairingSas,
  encryptManifest,
  encryptPrivateKey,
  decryptPrivateKeys,
  initializeVault,
  storePendingDevice,
  updateVault,
  rememberLocalDevice,
  localActiveDevice,
  localActiveDevices,
  localPendingDevices,
  createVaultId,
  keyCommitment,
  manifestDevice,
  updatedManifestDevice,
  manifestCommitment,
  rememberVaultCheckpoint,
  validateVault,
  eraseBytes,
  clearLocalVaultState,
  type VaultData,
  type DeviceRecord,
  type VaultManifest,
  type VaultKeyRecord,
  type VaultWrap,
  type EncryptedBlob,
} from './vault.ts';
import { enroll, unlock as webauthnUnlock, listCredentials } from './webauthn.ts';
import { importPrivateKey, generatePrivateKey, getFingerprint, getKeyIdentities, getPrimaryEmail, addKeyIdentity, extractPublicKey } from './keyring.ts';
import { parsePublicKey, listPublicKeys, type StoredPublicKey } from './public-keys.ts';
import { openVault, addKey as sessionAddKey, removeKey as sessionRemoveKey, getWrappingKey, getKeys, lock as sessionLock, isUnlocked } from './session.ts';
import { apiFetch } from '../api.ts';
import { downloadFile } from '../share.ts';
import { m } from '@tiaforge-ui/sinter/messages';

/** A device (passkey) of the vault, as the page lists it. */
export interface VaultDeviceView {
  cred_id: string;
  label: string | null;
  status: 'pending' | 'active';
  /** this browser's own device (an approved one that can unlock, or one waiting for approval) */
  local: boolean;
  /** pending devices: the pairing code both browsers show */
  sas: string;
}

/** A private key in the vault: its User IDs come from the unlocked key, else from its stored public copy. */
export interface VaultKeyView {
  fingerprint: string;
  identities: string[];
  /** its public key can be downloaded (unlocked, or a public copy is stored) */
  exportable: boolean;
}

/** What the settings page shows of the vault. */
export interface VaultView {
  exists: boolean;
  unlocked: boolean;
  /** passkeys registered without a vault: an interrupted setup */
  hasCredentials: boolean;
  keys: VaultKeyView[];
  active: VaultDeviceView[];
  pending: VaultDeviceView[];
  /** pairing code of this browser's pending device, while it waits for approval */
  pairingSas: string;
}

let vault: VaultData | null = null;
let manifest: VaultManifest | null = null;          // decrypted while unlocked
let imported: openpgp.PrivateKey | null = null;      // a key being imported, until it is added or the dialog closes
let ownPublic = new Map<string, StoredPublicKey>();
let storedIdentities = new Map<string, string[]>();
let hasCredentials = false;
let pairing = '';

async function cacheOwnPublicKeys(keys: StoredPublicKey[]) {
  ownPublic = new Map();
  storedIdentities = new Map();
  for (const record of keys.filter((k) => k.is_own)) {
    ownPublic.set(record.fingerprint, record);
    try {
      storedIdentities.set(record.fingerprint, getKeyIdentities(await parsePublicKey(record.armored)).map((i) => i.userID));
    } catch {
      // A malformed public copy must not prevent the private vault from loading.
    }
  }
}

async function reloadPublicKeys() {
  await cacheOwnPublicKeys(await listPublicKeys().catch(() => [...ownPublic.values()]));
}

function unlockedKey(fingerprint: string): openpgp.PrivateKey | undefined {
  return getKeys().find((k) => getFingerprint(k) === fingerprint);
}

async function view(): Promise<VaultView> {
  const unlocked = isUnlocked();
  if (!vault) return { exists: false, unlocked, hasCredentials, keys: [], active: [], pending: [], pairingSas: '' };
  const localActive = new Set(localActiveDevices(vault).map((d) => d.cred_id));
  const localPending = new Set(localPendingDevices(vault).map((d) => d.cred_id));
  const device = async (d: DeviceRecord): Promise<VaultDeviceView> => ({
    cred_id: d.cred_id,
    label: d.device_label,
    status: d.status,
    local: d.status === 'active' ? localActive.has(d.cred_id) : localPending.has(d.cred_id),
    sas: d.status === 'pending' ? await pairingSas(d) : '',
  });
  const devices = await Promise.all(vault.devices.map(device));
  return {
    exists: true,
    unlocked,
    hasCredentials,
    keys: vault.keys.map(({ fingerprint }) => {
      const key = unlockedKey(fingerprint);
      return {
        fingerprint,
        identities: key ? getKeyIdentities(key).map((i) => i.userID) : (storedIdentities.get(fingerprint) ?? []),
        exportable: unlocked || ownPublic.has(fingerprint),
      };
    }),
    active: devices.filter((d) => d.status === 'active'),
    pending: devices.filter((d) => d.status === 'pending'),
    pairingSas: localPending.size ? pairing : '',
  };
}

function requireUnlocked(): { vault: VaultData; manifest: VaultManifest; vaultKey: CryptoKey } {
  const vaultKey = getWrappingKey();
  if (!vault || !manifest || !vaultKey) throw new Error(m.pgp_unlockApproveFirst());
  return { vault, manifest, vaultKey };
}

function device(credId: string): DeviceRecord {
  const d = vault?.devices.find((x) => x.cred_id === credId);
  if (!d) throw new Error('unknown device');
  return d;
}

async function nextManifest(changes: Partial<Pick<VaultManifest, 'keys' | 'devices'>>): Promise<VaultManifest> {
  const { vault: v, manifest: current } = requireUnlocked();
  return { ...current, ...changes, epoch: current.epoch + 1, previous_commitment: await manifestCommitment(v.manifest) };
}

async function checkpoint(next: VaultManifest, encrypted: EncryptedBlob) {
  const local = vault && localActiveDevice(vault);
  if (local) await rememberVaultCheckpoint(next, encrypted, local.cred_id);
}

async function uploadPublicKey(key: openpgp.PrivateKey, fingerprint: string) {
  await apiFetch('/api/pgp/key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ fingerprint, email: getPrimaryEmail(key), armored: extractPublicKey(key), is_own: true, source: 'imported' }),
  }).catch(() => {});
}

/** Loads the vault, this browser's devices and the stored public copies of the own keys. */
export async function status(): Promise<VaultView> {
  vault = await readVault().catch(() => null);
  const localPending = vault ? localPendingDevices(vault)[0] : undefined;
  pairing = localPending ? await pairingSas(localPending) : '';
  hasCredentials = (await listCredentials().catch(() => [])).length > 0;
  await cacheOwnPublicKeys(await listPublicKeys().catch(() => []));
  return view();
}

/** Unlocks the vault with one of its approved passkeys (WebAuthn PRF). */
export async function unlock(credId: string): Promise<VaultView> {
  const d = device(credId);
  if (!vault || d.status !== 'active') throw new Error('no approved vault device');
  const { prfOutput } = await webauthnUnlock(d.prf_salt, d.cred_id);
  const { raw, key } = await unwrapVaultKey(d, prfOutput);
  try {
    manifest = await validateVault(vault, key, d.cred_id);
    const keys = await decryptPrivateKeys(key, vault.keys);
    openVault(key, keys);
    for (const [fp, privateKey] of keys) await uploadPublicKey(privateKey, fp);
    await reloadPublicKeys();
    rememberLocalDevice(d.cred_id);
    return view();
  } finally {
    eraseBytes(raw);
  }
}

/** Forgets the unlocked keys. */
export async function lock(): Promise<VaultView> {
  sessionLock();
  manifest = null;
  return view();
}

/** Deletes the vault and its passkeys on the server and this browser's vault state. */
export async function reset(): Promise<VaultView> {
  await resetVault();
  sessionLock();
  clearLocalVaultState();
  vault = null;
  manifest = null;
  hasCredentials = false;
  pairing = '';
  return view();
}

/** Decrypts an armored private key with its passphrase and holds it for store(). */
export async function readKey(armored: string, passphrase: string): Promise<{ fingerprint: string }> {
  imported = null;
  const key = await importPrivateKey(armored, passphrase);
  imported = key;
  return { fingerprint: getFingerprint(key) };
}

/** Generates a new private key for `name <email>` and holds it for store(), like readKey(). */
export async function generateKey(name: string, email: string): Promise<{ fingerprint: string }> {
  imported = null;
  const key = await generatePrivateKey({ name: name.trim(), email: email.trim() });
  imported = key;
  return { fingerprint: getFingerprint(key) };
}

/** Drops a key read by readKey() that was not stored (the import dialog closed). */
export function forgetKey(): void {
  imported = null;
}

/** First setup: a passkey (with PRF) for this browser, and a new vault holding the imported or generated key. */
async function create(label: string): Promise<VaultView> {
  const key = imported;
  if (!key) throw new Error('no key');
  const name = label || 'My device';
  const result = await enroll(name);
  if (!result.hasPrf) throw new Error('prf_not_supported');
  const fingerprint = getFingerprint(key);
  const bundle = await createDeviceBundle(result.credentialId, result.prfSalt, result.prfOutput);
  const { raw, key: vaultKey } = await generateVaultKey();
  try {
    const record = await encryptPrivateKey(vaultKey, fingerprint, key);
    const wrap = await wrapVaultKey(raw, bundle);
    const next: VaultManifest = {
      format: 2,
      vault_id: createVaultId(),
      epoch: 1,
      previous_commitment: null,
      keys: [{ fingerprint: record.fingerprint, commitment: await keyCommitment(record) }],
      devices: [await manifestDevice(bundle, name, wrap)],
    };
    const encrypted = await encryptManifest(vaultKey, next);
    await initializeVault(encrypted, record, bundle, wrap);
    rememberLocalDevice(bundle.cred_id);
    await rememberVaultCheckpoint(next, encrypted, bundle.cred_id);
    await uploadPublicKey(key, fingerprint);
    openVault(vaultKey, new Map([[fingerprint, key]]));
    manifest = next;
    vault = { revision: 1, manifest: encrypted, keys: [record], devices: [{ ...bundle, device_label: name, status: 'active', wrap }] };
    imported = null;
    await reloadPublicKeys();
    return view();
  } finally {
    eraseBytes(raw);
  }
}

async function storeKey(key: openpgp.PrivateKey): Promise<void> {
  const { vault: v, manifest: current, vaultKey } = requireUnlocked();
  const fingerprint = getFingerprint(key);
  const record = await encryptPrivateKey(vaultKey, fingerprint, key);
  const next = await nextManifest({
    keys: [...current.keys.filter((k) => k.fingerprint !== fingerprint), { fingerprint, commitment: await keyCommitment(record) }],
  });
  const encrypted = await encryptManifest(vaultKey, next);
  const revision = await updateVault({ expected_revision: v.revision, manifest: encrypted, upsert_keys: [record] });
  await checkpoint(next, encrypted);
  manifest = next;
  vault = { ...v, revision, manifest: encrypted, keys: [...v.keys.filter((k) => k.fingerprint !== fingerprint), record] };
  sessionAddKey(fingerprint, key);
  await uploadPublicKey(key, fingerprint);
  await reloadPublicKeys();
}

/** Stores the key read by readKey() or made by generateKey(): in a new vault with a passkey named `label`, or in the unlocked vault. */
export async function store(label: string): Promise<VaultView> {
  if (!vault) return create(label);
  if (!imported) throw new Error('no key');
  await storeKey(imported);
  imported = null;
  return view();
}

/** Certifies another User ID on an unlocked key. */
export async function addIdentity(fingerprint: string, name: string, email: string, comment: string): Promise<VaultView> {
  const key = unlockedKey(fingerprint);
  if (!key) throw new Error('vault is locked');
  await storeKey(await addKeyIdentity(key, { name: name.trim(), email: email.trim(), ...(comment.trim() ? { comment: comment.trim() } : {}) }));
  return view();
}

/** Merges an updated public key (new User IDs, subkeys, expiry) into its unlocked private key. */
export async function updatePublic(fingerprint: string, armored: string): Promise<VaultView> {
  const key = unlockedKey(fingerprint);
  if (!key) throw new Error('vault is locked');
  const publicKey = await parsePublicKey(armored);
  if (getFingerprint(publicKey) !== fingerprint) throw new Error(m.pgp_publicKeyMismatch());
  await storeKey(await key.update(publicKey));
  return view();
}

/** Saves the key's public part as <fingerprint>.asc. */
export function exportPublic(fingerprint: string): void {
  const key = unlockedKey(fingerprint);
  const armored = key ? extractPublicKey(key) : ownPublic.get(fingerprint)?.armored;
  if (armored) downloadFile(`${fingerprint}.asc`, armored, 'application/pgp-keys');
}

/** Registers a passkey of this browser as a pending device; an unlocked approved device approves it. */
export async function register(label: string): Promise<VaultView> {
  const name = label || 'My device';
  const result = await enroll(name);
  const bundle = await createDeviceBundle(result.credentialId, result.prfSalt, result.prfOutput);
  await storePendingDevice(bundle);
  rememberLocalDevice(bundle.cred_id);
  pairing = await pairingSas(bundle);
  if (vault && manifest && isUnlocked()) {
    vault = {
      ...vault,
      devices: [...vault.devices.filter((d) => d.cred_id !== bundle.cred_id), { ...bundle, device_label: name, status: 'pending', wrap: null }],
    };
    return view();
  }
  return status();
}

/** Approves a pending device: wraps the vault key for it (after re-checking the vault with this browser's passkey). */
export async function approve(credId: string): Promise<VaultView> {
  const { vault: v } = requireUnlocked();
  const d = device(credId);
  const local = localActiveDevice(v);
  if (!local) throw new Error('no local approved device');
  const { prfOutput } = await webauthnUnlock(local.prf_salt, local.cred_id);
  const { raw, key } = await unwrapVaultKey(local, prfOutput);
  try {
    manifest = await validateVault(v, key, local.cred_id);
    const wrap = await wrapVaultKey(raw, d);
    const next = await nextManifest({ devices: [...manifest.devices, await manifestDevice(d, d.device_label ?? 'My device', wrap)] });
    const encrypted = await encryptManifest(key, next);
    const revision = await updateVault({ expected_revision: v.revision, manifest: encrypted, approve_device: { cred_id: d.cred_id, wrap } });
    await rememberVaultCheckpoint(next, encrypted, local.cred_id);
    manifest = next;
    vault = { ...v, revision, manifest: encrypted, devices: v.devices.map((x) => (x.cred_id === d.cred_id ? { ...x, status: 'active', wrap } : x)) };
    return view();
  } finally {
    eraseBytes(raw);
  }
}

/**
 * Re-encrypts the vault under a new vault key for the remaining devices: rotation, removing a key
 * (so its old ciphertext cannot be decrypted with a key a removed device might still hold) or removing a device.
 */
async function rekey(keys: openpgp.PrivateKey[], removeDevice: string | null, deleteKey: string | null): Promise<void> {
  const { vault: v, manifest: current } = requireUnlocked();
  const devices = current.devices.filter((d) => d.cred_id !== removeDevice);
  const { raw, key: vaultKey } = await generateVaultKey();
  try {
    const records: VaultKeyRecord[] = await Promise.all(keys.map((k) => encryptPrivateKey(vaultKey, getFingerprint(k), k)));
    const wraps: VaultWrap[] = await Promise.all(devices.map((d) => wrapVaultKey(raw, d)));
    const next = await nextManifest({
      keys: await Promise.all(records.map(async (r) => ({ fingerprint: r.fingerprint, commitment: await keyCommitment(r) }))),
      devices: await Promise.all(devices.map((d, i) => updatedManifestDevice(d, wraps[i]!))),
    });
    const encrypted = await encryptManifest(vaultKey, next);
    const revision = await updateVault({
      expected_revision: v.revision,
      manifest: encrypted,
      upsert_keys: records,
      replace_wraps: wraps,
      ...(deleteKey ? { delete_keys: [deleteKey] } : {}),
      ...(removeDevice ? { remove_device: removeDevice } : {}),
    });
    await checkpoint(next, encrypted);
    manifest = next;
    const wrapsById = new Map(wraps.map((w) => [w.cred_id, w]));
    vault = {
      ...v,
      revision,
      manifest: encrypted,
      keys: records,
      devices: v.devices
        .filter((d) => d.cred_id !== removeDevice)
        .map((d) => (wrapsById.has(d.cred_id) ? { ...d, wrap: wrapsById.get(d.cred_id)! } : d)),
    };
    openVault(vaultKey, new Map(keys.map((k) => [getFingerprint(k), k])));
  } finally {
    eraseBytes(raw);
  }
}

/** Replaces the vault key (all keys re-encrypted, all devices re-wrapped). */
export async function rotate(): Promise<VaultView> {
  await rekey(getKeys(), null, null);
  return view();
}

/** Removes a private key from the vault. */
export async function removeKey(fingerprint: string): Promise<VaultView> {
  await rekey(getKeys().filter((k) => getFingerprint(k) !== fingerprint), null, fingerprint);
  sessionRemoveKey(fingerprint);
  return view();
}

/** Removes a device (approved or pending); the last approved one cannot be removed. */
export async function removeDevice(credId: string): Promise<VaultView> {
  const { manifest: current } = requireUnlocked();
  const d = device(credId);
  if (d.status === 'active' && current.devices.every((x) => x.cred_id === credId)) throw new Error('cannot remove the last active device');
  await rekey(getKeys(), credId, null);
  return view();
}

<script lang="ts">
  import { onMount } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
  import UiIcon from '../UiIcon.svelte';
  export let locale: string;
  $: tl = makeTl(locale);

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
    type VaultKeyRecord,
    type DeviceRecord,
    type VaultManifest,
  } from '../../lib/pgp/vault.ts';
  import { enroll, unlock, listCredentials } from '../../lib/pgp/webauthn.ts';
  import {
    importPrivateKey,
    getFingerprint,
    getPrimaryEmail,
    getKeyIdentities,
    addKeyIdentity,
    extractPublicKey,
  } from '../../lib/pgp/keyring.ts';
  import {
    parsePublicKey,
    listPublicKeys,
    savePublicKey,
    deletePublicKey,
    type StoredPublicKey,
  } from '../../lib/pgp/public-keys.ts';
  import { openVault, addKey, removeKey, getWrappingKey, getKeys, lock, isUnlocked } from '../../lib/pgp/session.ts';
  import { apiFetch } from '../../lib/api.ts';
  import * as openpgp from 'openpgp';

  // ── Encryption tab state ──────────────────────────────────────────────────
  let pgpVault: VaultData | null = null;
  let pgpManifest: VaultManifest | null = null;
  let pgpVaultRecords: VaultKeyRecord[] = [];
  let pgpPairingSas = '';
  let pgpDeviceActionError = '';
  let pgpHasCredentials = false;
  let pgpResetting = false;
  let pgpPrivateKeyDialog: HTMLDialogElement;
  let pgpPublicKeyDialog: HTMLDialogElement;
  let pgpIdentityDialog: HTMLDialogElement;
  let pgpOwnPublicKeyDialog: HTMLDialogElement;
  let pgpAccessDialog: HTMLDialogElement;
  let pgpUnlockDialog: HTMLDialogElement;
  let pgpUnlockDialogRecovery = false;

  let pgpArmoredKey = '';
  let pgpPassphrase = '';
  let pgpDeviceLabel = '';
  let pgpImporting = false;
  let pgpImportError = '';
  let pgpImportedKey: openpgp.PrivateKey | null = null;
  let pgpImportedFingerprint = '';

  let pgpEnrolling = false;
  let pgpEnrollError = '';
  let pgpJustEnrolled = '';  // fingerprint of key enrolled this session

  let pgpUnlocking = false;
  let pgpUnlockError = '';
  let pgpUnlockRevision = 0;
  $: pgpVaultUnlocked = (pgpUnlockRevision, isUnlocked());

  let pgpPublicKeys: StoredPublicKey[] = [];
  let pgpPublicArmoredKey = '';
  let pgpPublicImporting = false;
  let pgpPublicImportError = '';
  let pgpPublicImportSuccess = '';
  let pgpOwnPublicKeys = new Map<string, StoredPublicKey>();
  let pgpStoredKeyIdentities = new Map<string, ReturnType<typeof getKeyIdentities>>();
  let pgpManagedFingerprint = '';
  let pgpIdentityName = '';
  let pgpIdentityEmail = '';
  let pgpIdentityComment = '';
  let pgpManagedPublicArmored = '';
  let pgpKeyActionSaving = false;
  let pgpKeyActionError = '';
  $: pgpTrustedSenderKeys = pgpPublicKeys.filter(key => !key.is_own);
  $: pgpLocalActiveMethods = pgpVault ? localActiveDevices(pgpVault) : [];
  $: pgpLocalPendingMethods = pgpVault ? localPendingDevices(pgpVault) : [];
  $: pgpUnlockCandidates = pgpUnlockDialogRecovery
    ? (pgpVault?.devices.filter(device => device.status === 'active') ?? [])
    : pgpLocalActiveMethods;

  async function cacheOwnPublicKeys() {
    const keys = new Map<string, StoredPublicKey>();
    const identities = new Map<string, ReturnType<typeof getKeyIdentities>>();
    for (const record of pgpPublicKeys.filter(key => key.is_own)) {
      keys.set(record.fingerprint, record);
      try {
        identities.set(record.fingerprint, getKeyIdentities(await parsePublicKey(record.armored)));
      } catch {
        // A malformed public copy must not prevent the private vault from loading.
      }
    }
    pgpOwnPublicKeys = keys;
    pgpStoredKeyIdentities = identities;
  }

  async function loadVaultStatus() {
    const vault = await readVault().catch(() => null);
    pgpVault = vault;
    pgpVaultRecords = vault?.keys ?? [];
    const localPending = vault ? localPendingDevices(vault)[0] : undefined;
    pgpPairingSas = localPending ? await pairingSas(localPending) : '';
    pgpHasCredentials = (await listCredentials().catch(() => [])).length > 0;
    pgpPublicKeys = await listPublicKeys().catch(() => []);
    await cacheOwnPublicKeys();
  }

  function refreshUnlockState() {
    pgpUnlockRevision += 1;
  }

  async function nextManifest(changes: Partial<Pick<VaultManifest, 'keys' | 'devices'>>): Promise<VaultManifest> {
    if (!pgpVault || !pgpManifest) throw new Error('unlock the vault first');
    return {
      ...pgpManifest,
      ...changes,
      epoch: pgpManifest.epoch + 1,
      previous_commitment: await manifestCommitment(pgpVault.manifest),
    };
  }

  async function checkpointUpdate(manifest: VaultManifest, encryptedManifest: { iv: string; ciphertext: string }) {
    if (!pgpVault) return;
    const localDevice = localActiveDevice(pgpVault);
    if (localDevice) await rememberVaultCheckpoint(manifest, encryptedManifest, localDevice.cred_id);
  }

  async function onPgpImport() {
    pgpImporting = true;
    pgpImportError = '';
    pgpImportedKey = null;
    pgpImportedFingerprint = '';
    try {
      const key = await importPrivateKey(pgpArmoredKey, pgpPassphrase);
      pgpImportedKey = key;
      pgpImportedFingerprint = getFingerprint(key);
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      if (msg.toLowerCase().includes('argon2')) {
        pgpImportError = tl('pgp_importArgon2Error');
      } else {
        pgpImportError = `${tl('pgp_importError')}: ${msg}`;
      }
    }
    pgpImporting = false;
  }

  async function unlockStoredVault(device: DeviceRecord): Promise<CryptoKey> {
    if (!pgpVault || device.status !== 'active') throw new Error('no approved vault device');
    const { prfOutput } = await unlock(device.prf_salt, device.cred_id);
    const { raw, key } = await unwrapVaultKey(device, prfOutput);
    try {
      pgpManifest = await validateVault(pgpVault, key, device.cred_id);
      const keys = await decryptPrivateKeys(key, pgpVault.keys);
      openVault(key, keys);
      for (const [fp, privateKey] of keys) {
        await uploadPublicKey(privateKey, fp);
      }
      pgpPublicKeys = await listPublicKeys().catch(() => pgpPublicKeys);
      await cacheOwnPublicKeys();
      rememberLocalDevice(device.cred_id);
      refreshUnlockState();
      return key;
    } finally {
      eraseBytes(raw);
    }
  }

  // First enrollment initializes ciphertext storage and the first approved device.
  async function onPgpEnroll() {
    if (!pgpImportedKey) return;
    pgpEnrolling = true;
    pgpEnrollError = '';
    try {
      const result = await enroll(pgpDeviceLabel || 'My device');
      if (!result.hasPrf) {
        pgpEnrollError = tl('pgp_enrollNoPrf');
        pgpEnrolling = false;
        return;
      }

      const label = pgpDeviceLabel || 'My device';
      const device = await createDeviceBundle(result.credentialId, result.prfSalt, result.prfOutput);
      const { raw, key: vaultKey } = await generateVaultKey();
      try {
        const record = await encryptPrivateKey(vaultKey, pgpImportedFingerprint, pgpImportedKey);
        const wrap = await wrapVaultKey(raw, device);
        const manifest: VaultManifest = {
          format: 2,
          vault_id: createVaultId(),
          epoch: 1,
          previous_commitment: null,
          keys: [{ fingerprint: record.fingerprint, commitment: await keyCommitment(record) }],
          devices: [await manifestDevice(device, label, wrap)],
        };
        const encryptedManifest = await encryptManifest(vaultKey, manifest);
        await initializeVault(encryptedManifest, record, device, wrap);
        rememberLocalDevice(device.cred_id);
        await rememberVaultCheckpoint(manifest, encryptedManifest, device.cred_id);
        await uploadPublicKey(pgpImportedKey, pgpImportedFingerprint);
        openVault(vaultKey, new Map([[pgpImportedFingerprint, pgpImportedKey]]));
        pgpManifest = manifest;
        pgpVault = {
          revision: 1,
          manifest: encryptedManifest,
          keys: [record],
          devices: [{ ...device, device_label: label, status: 'active', wrap }],
        };
        pgpVaultRecords = [record];
        pgpPublicKeys = await listPublicKeys().catch(() => []);
        await cacheOwnPublicKeys();
        refreshUnlockState();
        pgpJustEnrolled = pgpImportedFingerprint;
        clearImportForm();
        pgpPrivateKeyDialog?.close();
      } finally {
        eraseBytes(raw);
      }
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = msg === 'prf_not_supported' ? tl('pgp_enrollNoPrf') : `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  // Add an additional key to synchronized ciphertext storage.
  async function onPgpAddKey() {
    if (!pgpImportedKey || !pgpVault) return;
    pgpEnrolling = true;
    pgpEnrollError = '';
    try {
      const vaultKey = getWrappingKey();
      if (!vaultKey || !pgpManifest) throw new Error('unlock the vault first');
      const record = await encryptPrivateKey(vaultKey, pgpImportedFingerprint, pgpImportedKey);
      const keys = [
        ...pgpManifest.keys.filter(existing => existing.fingerprint !== record.fingerprint),
        { fingerprint: record.fingerprint, commitment: await keyCommitment(record) },
      ];
      const next = await nextManifest({ keys });
      const encryptedManifest = await encryptManifest(vaultKey, next);
      const revision = await updateVault({ expected_revision: pgpVault.revision, manifest: encryptedManifest, upsert_keys: [record] });
      await checkpointUpdate(next, encryptedManifest);
      pgpManifest = next;
      pgpVault = { ...pgpVault, revision, manifest: encryptedManifest };

      await uploadPublicKey(pgpImportedKey, pgpImportedFingerprint);
      addKey(pgpImportedFingerprint, pgpImportedKey);
      pgpPublicKeys = await listPublicKeys().catch(() => pgpPublicKeys);
      await cacheOwnPublicKeys();

      pgpVaultRecords = [
        ...pgpVaultRecords.filter(existing => existing.fingerprint !== record.fingerprint),
        record,
      ];
      pgpVault = { ...pgpVault, keys: pgpVaultRecords };
      pgpJustEnrolled = pgpImportedFingerprint;
      clearImportForm();
      pgpPrivateKeyDialog?.close();
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  // Register this browser as a pending device. An unlocked existing device approves it.
  async function onPgpRegisterDevice() {
    pgpEnrolling = true;
    pgpEnrollError = '';
    try {
      const result = await enroll(pgpDeviceLabel || 'My device');
      const device = await createDeviceBundle(result.credentialId, result.prfSalt, result.prfOutput);
      await storePendingDevice(device);
      rememberLocalDevice(device.cred_id);
      pgpPairingSas = await pairingSas(device);
      if (pgpVault && pgpManifest && isUnlocked()) {
        pgpVault = {
          ...pgpVault,
          devices: [
            ...pgpVault.devices.filter(existing => existing.cred_id !== device.cred_id),
            { ...device, device_label: pgpDeviceLabel || 'My device', status: 'pending', wrap: null },
          ],
        };
      } else {
        await loadVaultStatus();
      }
      pgpAccessDialog?.close();
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  async function onPgpApproveDevice(device: DeviceRecord) {
    if (!pgpVault || !getWrappingKey() || !pgpManifest) {
      pgpDeviceActionError = tl('pgp_unlockApproveFirst');
      return;
    }
    const sas = await pairingSas(device);
    if (!confirm(tl('pgp_confirmApprove', { sas }))) return;
    let raw: Uint8Array | null = null;
    try {
      const localDevice = localActiveDevice(pgpVault);
      if (!localDevice) throw new Error('no local approved device');
      const { prfOutput } = await unlock(localDevice.prf_salt, localDevice.cred_id);
      const unwrapped = await unwrapVaultKey(localDevice, prfOutput);
      raw = unwrapped.raw;
      pgpManifest = await validateVault(pgpVault, unwrapped.key, localDevice.cred_id);
      const wrap = await wrapVaultKey(raw, device);
      const next = await nextManifest({
        devices: [...pgpManifest.devices, await manifestDevice(device, device.device_label ?? 'My device', wrap)],
      });
      const encryptedManifest = await encryptManifest(unwrapped.key, next);
      const revision = await updateVault({
        expected_revision: pgpVault.revision,
        manifest: encryptedManifest,
        approve_device: { cred_id: device.cred_id, wrap },
      });
      await rememberVaultCheckpoint(next, encryptedManifest, localDevice.cred_id);
      pgpManifest = next;
      pgpVault = {
        ...pgpVault,
        revision,
        manifest: encryptedManifest,
        devices: pgpVault.devices.map(existing => existing.cred_id === device.cred_id
          ? { ...existing, status: 'active', wrap }
          : existing),
      };
    } catch (e: unknown) {
      pgpDeviceActionError = (e instanceof Error) ? e.message : String(e);
    } finally {
      if (raw) eraseBytes(raw);
    }
  }

  async function onPgpUnlockVault(device: DeviceRecord) {
    pgpUnlocking = true;
    pgpUnlockError = '';
    try {
      await unlockStoredVault(device);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error('[pgp] unlock vault failed', e);
      pgpUnlockError = `${tl('pgp_unlockError')} ${msg}`;
    }
    pgpUnlocking = false;
  }

  function onPgpLockVault() {
    lock();
    pgpManifest = null;
    refreshUnlockState();
  }

  async function onPgpResetVault() {
    if (!confirm(tl('pgp_confirmStartOver'))) return;
    pgpResetting = true;
    pgpDeviceActionError = '';
    pgpEnrollError = '';
    try {
      await resetVault();
      lock();
      clearLocalVaultState();
      pgpVault = null;
      pgpManifest = null;
      pgpVaultRecords = [];
      pgpHasCredentials = false;
      pgpPairingSas = '';
      pgpJustEnrolled = '';
      refreshUnlockState();
    } catch {
      pgpDeviceActionError = tl('pgp_resetError');
      pgpEnrollError = tl('pgp_resetError');
    }
    pgpResetting = false;
  }

  async function onPgpRemoveKey(fingerprint: string) {
    if (!confirm(tl('pgp_confirmRemoveKey'))) return;
    if (!pgpVault || !getWrappingKey() || !pgpManifest) return;
    let raw: Uint8Array | null = null;
    try {
      const generated = await generateVaultKey();
      raw = generated.raw;
      const remainingKeys = getKeys().filter(key => getFingerprint(key) !== fingerprint);
      const records = await Promise.all(remainingKeys.map(key => encryptPrivateKey(generated.key, getFingerprint(key), key)));
      const wraps = await Promise.all(pgpManifest.devices.map(device => wrapVaultKey(raw!, device)));
      const devices = await Promise.all(pgpManifest.devices.map((device, index) => updatedManifestDevice(device, wraps[index])));
      const next = await nextManifest({
        keys: await Promise.all(records.map(async record => ({ fingerprint: record.fingerprint, commitment: await keyCommitment(record) }))),
        devices,
      });
      const encryptedManifest = await encryptManifest(generated.key, next);
      const revision = await updateVault({
        expected_revision: pgpVault.revision,
        manifest: encryptedManifest,
        upsert_keys: records,
        delete_keys: [fingerprint],
        replace_wraps: wraps,
      });
      await checkpointUpdate(next, encryptedManifest);
      pgpManifest = next;
      const wrapsById = new Map(wraps.map(wrap => [wrap.cred_id, wrap]));
      pgpVault = {
        ...pgpVault,
        revision,
        manifest: encryptedManifest,
        keys: records,
        devices: pgpVault.devices.map(device => wrapsById.has(device.cred_id)
          ? { ...device, wrap: wrapsById.get(device.cred_id)! }
          : device),
      };
      removeKey(fingerprint);
      pgpVaultRecords = records;
      openVault(generated.key, new Map(remainingKeys.map(key => [getFingerprint(key), key])));
      refreshUnlockState();
    } catch (e: unknown) {
      pgpDeviceActionError = (e instanceof Error) ? e.message : String(e);
    } finally {
      if (raw) eraseBytes(raw);
    }
  }

  function unlockedPrivateKey(fingerprint: string): openpgp.PrivateKey | undefined {
    return getKeys().find(key => getFingerprint(key) === fingerprint);
  }

  function displayedKeyIdentities(
    fingerprint: string,
    _unlockRevision: number,
    storedIdentities: Map<string, ReturnType<typeof getKeyIdentities>>,
  ): ReturnType<typeof getKeyIdentities> {
    const key = unlockedPrivateKey(fingerprint);
    return key ? getKeyIdentities(key) : (storedIdentities.get(fingerprint) ?? []);
  }

  async function updatePrivateKey(key: openpgp.PrivateKey) {
    if (!pgpVault || !pgpManifest) throw new Error('unlock the vault first');
    const vaultKey = getWrappingKey();
    if (!vaultKey) throw new Error('unlock the vault first');
    const fingerprint = getFingerprint(key);
    const record = await encryptPrivateKey(vaultKey, fingerprint, key);
    const keys = [
      ...pgpManifest.keys.filter(existing => existing.fingerprint !== fingerprint),
      { fingerprint, commitment: await keyCommitment(record) },
    ];
    const next = await nextManifest({ keys });
    const encryptedManifest = await encryptManifest(vaultKey, next);
    const revision = await updateVault({
      expected_revision: pgpVault.revision,
      manifest: encryptedManifest,
      upsert_keys: [record],
    });
    await checkpointUpdate(next, encryptedManifest);
    pgpManifest = next;
    pgpVaultRecords = [
      ...pgpVaultRecords.filter(existing => existing.fingerprint !== fingerprint),
      record,
    ];
    pgpVault = { ...pgpVault, revision, manifest: encryptedManifest, keys: pgpVaultRecords };
    addKey(fingerprint, key);
    await uploadPublicKey(key, fingerprint);
    pgpPublicKeys = await listPublicKeys().catch(() => pgpPublicKeys);
    await cacheOwnPublicKeys();
    refreshUnlockState();
  }

  async function onPgpRemoveDevice(device: DeviceRecord) {
    if (!pgpVault || !getWrappingKey() || !pgpManifest) {
      pgpDeviceActionError = tl('pgp_unlockRemoveFirst');
      return;
    }
    if (!confirm(tl('pgp_confirmRemoveDevice', { device: device.device_label ?? tl('pgp_defaultAccessMethod') }))) return;
    let raw: Uint8Array | null = null;
    try {
      const remainingDevices = pgpManifest.devices.filter(entry => entry.cred_id !== device.cred_id);
      if (device.status === 'active' && remainingDevices.length === 0) throw new Error('cannot remove the last active device');
      const generated = await generateVaultKey();
      raw = generated.raw;
      const records = await Promise.all(getKeys().map(key => encryptPrivateKey(generated.key, getFingerprint(key), key)));
      const wraps = await Promise.all(remainingDevices.map(entry => wrapVaultKey(raw!, entry)));
      const nextDevices = await Promise.all(remainingDevices.map((entry, index) => updatedManifestDevice(entry, wraps[index])));
      const next = await nextManifest({
        keys: await Promise.all(records.map(async record => ({ fingerprint: record.fingerprint, commitment: await keyCommitment(record) }))),
        devices: nextDevices,
      });
      const encryptedManifest = await encryptManifest(generated.key, next);
      const revision = await updateVault({
        expected_revision: pgpVault.revision,
        manifest: encryptedManifest,
        upsert_keys: records,
        remove_device: device.cred_id,
        replace_wraps: wraps,
      });
      await checkpointUpdate(next, encryptedManifest);
      pgpManifest = next;
      const wrapsById = new Map(wraps.map(wrap => [wrap.cred_id, wrap]));
      pgpVault = {
        ...pgpVault,
        revision,
        manifest: encryptedManifest,
        keys: records,
        devices: pgpVault.devices
          .filter(existing => existing.cred_id !== device.cred_id)
          .map(existing => wrapsById.has(existing.cred_id) ? { ...existing, wrap: wrapsById.get(existing.cred_id)! } : existing),
      };
      pgpVaultRecords = records;
      openVault(generated.key, new Map(getKeys().map(key => [getFingerprint(key), key])));
      refreshUnlockState();
    } catch (e: unknown) {
      pgpDeviceActionError = (e instanceof Error) ? e.message : String(e);
    } finally {
      if (raw) eraseBytes(raw);
    }
  }

  async function onPgpRotateVaultKey() {
    if (!pgpVault || !getWrappingKey() || !pgpManifest || !confirm(tl('pgp_confirmRotate'))) return;
    let raw: Uint8Array | null = null;
    try {
      const { raw: nextRaw, key: nextKey } = await generateVaultKey();
      raw = nextRaw;
      const records = await Promise.all(getKeys().map(key => encryptPrivateKey(nextKey, getFingerprint(key), key)));
      const wraps = await Promise.all(pgpManifest.devices.map(device => wrapVaultKey(raw!, device)));
      const devices = await Promise.all(pgpManifest.devices.map((device, index) => updatedManifestDevice(device, wraps[index])));
      const next = await nextManifest({
        keys: await Promise.all(records.map(async record => ({ fingerprint: record.fingerprint, commitment: await keyCommitment(record) }))),
        devices,
      });
      const encryptedManifest = await encryptManifest(nextKey, next);
      const revision = await updateVault({
        expected_revision: pgpVault.revision,
        manifest: encryptedManifest,
        upsert_keys: records,
        replace_wraps: wraps,
      });
      await checkpointUpdate(next, encryptedManifest);
      pgpManifest = next;
      const wrapsById = new Map(wraps.map(wrap => [wrap.cred_id, wrap]));
      pgpVault = {
        ...pgpVault,
        revision,
        manifest: encryptedManifest,
        keys: records,
        devices: pgpVault.devices.map(device => wrapsById.has(device.cred_id)
          ? { ...device, wrap: wrapsById.get(device.cred_id)! }
          : device),
      };
      pgpVaultRecords = records;
      openVault(nextKey, new Map(getKeys().map(key => [getFingerprint(key), key])));
      refreshUnlockState();
    } catch (e: unknown) {
      pgpDeviceActionError = (e instanceof Error) ? e.message : String(e);
    } finally {
      if (raw) eraseBytes(raw);
    }
  }

  function clearImportForm() {
    pgpImportedKey = null;
    pgpImportedFingerprint = '';
    pgpArmoredKey = '';
    pgpPassphrase = '';
    pgpDeviceLabel = '';
    pgpImportError = '';
  }

  function openPrivateKeyDialog() {
    clearImportForm();
    pgpEnrollError = '';
    pgpPrivateKeyDialog?.showModal();
  }

  function closePrivateKeyDialog() {
    clearImportForm();
    pgpEnrollError = '';
    pgpPrivateKeyDialog?.close();
  }

  function openAccessDialog() {
    pgpDeviceLabel = '';
    pgpEnrollError = '';
    pgpAccessDialog?.showModal();
  }

  function openUnlockDialog(recovery: boolean) {
    pgpUnlockDialogRecovery = recovery;
    pgpUnlockError = '';
    pgpUnlockDialog?.showModal();
  }

  function openPublicKeyDialog() {
    pgpPublicArmoredKey = '';
    pgpPublicImportError = '';
    pgpPublicKeyDialog?.showModal();
  }

  function openIdentityDialog(fingerprint: string) {
    pgpManagedFingerprint = fingerprint;
    pgpIdentityName = '';
    pgpIdentityEmail = '';
    pgpIdentityComment = '';
    pgpKeyActionError = '';
    pgpIdentityDialog?.showModal();
  }

  function openOwnPublicKeyDialog(fingerprint: string) {
    pgpManagedFingerprint = fingerprint;
    pgpManagedPublicArmored = '';
    pgpKeyActionError = '';
    pgpOwnPublicKeyDialog?.showModal();
  }

  async function onPgpAddIdentity() {
    const key = unlockedPrivateKey(pgpManagedFingerprint);
    if (!key) return;
    pgpKeyActionSaving = true;
    pgpKeyActionError = '';
    try {
      const updated = await addKeyIdentity(key, {
        name: pgpIdentityName.trim(),
        email: pgpIdentityEmail.trim(),
        ...(pgpIdentityComment.trim() ? { comment: pgpIdentityComment.trim() } : {}),
      });
      await updatePrivateKey(updated);
      pgpIdentityDialog?.close();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pgpKeyActionError = `${tl('pgp_identityAddError')}: ${msg}`;
    }
    pgpKeyActionSaving = false;
  }

  async function onPgpOwnPublicImport() {
    const privateKey = unlockedPrivateKey(pgpManagedFingerprint);
    if (!privateKey) return;
    pgpKeyActionSaving = true;
    pgpKeyActionError = '';
    try {
      const publicKey = await parsePublicKey(pgpManagedPublicArmored);
      if (getFingerprint(publicKey) !== pgpManagedFingerprint) {
        throw new Error(tl('pgp_publicKeyMismatch'));
      }
      await updatePrivateKey(await privateKey.update(publicKey));
      pgpOwnPublicKeyDialog?.close();
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      pgpKeyActionError = `${tl('pgp_ownPublicKeyImportError')}: ${msg}`;
    }
    pgpKeyActionSaving = false;
  }

  function onPgpExportPublicKey(fingerprint: string) {
    const key = unlockedPrivateKey(fingerprint);
    const armored = key ? extractPublicKey(key) : pgpOwnPublicKeys.get(fingerprint)?.armored;
    if (!armored) return;
    const url = URL.createObjectURL(new Blob([armored], { type: 'application/pgp-keys' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = `${fingerprint}.asc`;
    link.click();
    URL.revokeObjectURL(url);
  }

  async function selectUnlockMethod(device: DeviceRecord) {
    await onPgpUnlockVault(device);
    if (isUnlocked()) pgpUnlockDialog?.close();
  }

  async function uploadPublicKey(key: openpgp.PrivateKey, fingerprint: string) {
    const pubKeyArmored = extractPublicKey(key);
    const email = getPrimaryEmail(key);
    await apiFetch('/api/pgp/key', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ fingerprint, email, armored: pubKeyArmored, is_own: true, source: 'imported' }),
    }).catch(() => {});
  }

  async function onPgpPublicImport() {
    pgpPublicImporting = true;
    pgpPublicImportError = '';
    pgpPublicImportSuccess = '';
    try {
      const key = await parsePublicKey(pgpPublicArmoredKey);
      const saved = await savePublicKey(key, 'imported');
      pgpPublicKeys = [...pgpPublicKeys.filter(existing => existing.fingerprint !== saved.fingerprint), saved];
      pgpPublicArmoredKey = '';
      pgpPublicImportSuccess = tl('pgp_publicKeyImported');
      pgpPublicKeyDialog?.close();
    } catch {
      pgpPublicImportError = tl('pgp_publicKeyImportError');
    }
    pgpPublicImporting = false;
  }

  async function onPgpRemovePublicKey(fingerprint: string) {
    if (!confirm(tl('pgp_confirmRemovePublicKey'))) return;
    try {
      await deletePublicKey(fingerprint);
      pgpPublicKeys = pgpPublicKeys.filter(key => key.fingerprint !== fingerprint);
    } catch {
      pgpPublicImportError = tl('pgp_publicKeyRemoveError');
    }
  }

  onMount(async () => {
    await loadVaultStatus();
  });
</script>

  <div class="encryption-panel">
    {#if pgpVault}
      <div class="pgp-heading">
        <h3>{tl('pgp_vaultHeading')}</h3>
        <span class="pgp-key-status {pgpVaultUnlocked ? 'unlocked' : 'locked'}">
          {pgpVaultUnlocked ? tl('pgp_vaultUnlocked') : tl('pgp_vaultLocked')}
        </span>
      </div>
      {#if pgpUnlockError}<p class="pgp-error">{pgpUnlockError}</p>{/if}
      {#if pgpDeviceActionError}<p class="pgp-error">{pgpDeviceActionError}</p>{/if}

      <div class="pgp-global-actions">
        {#if pgpVaultUnlocked}
          <button class="secondary" onclick={onPgpLockVault}>{tl('pgp_lockVault')}</button>
        {:else if pgpLocalActiveMethods.length === 1}
          <button onclick={() => onPgpUnlockVault(pgpLocalActiveMethods[0])} disabled={pgpUnlocking} aria-busy={pgpUnlocking}>
            {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockVaultButton')}
          </button>
        {:else if pgpLocalActiveMethods.length > 1}
          <button onclick={() => openUnlockDialog(false)}>{tl('pgp_unlockVaultButton')}</button>
        {/if}
        <button class="secondary danger" onclick={onPgpResetVault} disabled={pgpResetting} aria-busy={pgpResetting}>
          {tl('pgp_startOverButton')}
        </button>
        {#if pgpVaultUnlocked}
          <button class="secondary" onclick={onPgpRotateVaultKey}>{tl('pgp_rotateVaultKey')}</button>
        {/if}
      </div>

      <div class="pgp-section-heading">
        <h4>{tl('pgp_privateKeysHeading')}</h4>
        <span
          class="pgp-tooltip-anchor"
          title={pgpVaultUnlocked ? tl('pgp_importPrivateKeyButton') : tl('pgp_importUnlockFirst')}
        >
          <button
            class="pgp-icon-btn"
            onclick={openPrivateKeyDialog}
            disabled={!pgpVaultUnlocked}
            title={pgpVaultUnlocked ? tl('pgp_importPrivateKeyButton') : tl('pgp_importUnlockFirst')}
            aria-label={pgpVaultUnlocked ? tl('pgp_importPrivateKeyButton') : tl('pgp_importUnlockFirst')}
          >
            <UiIcon name="import" />
          </button>
        </span>
      </div>
      <p class="pgp-section-hint">{tl('pgp_privateKeysHint')}</p>
      <ul class="pgp-key-list">
        {#each pgpVaultRecords as record (record.fingerprint)}
          <li class="pgp-key-item pgp-private-key-item">
            <div class="pgp-key-info pgp-private-key-info">
              <div class="pgp-private-key-header">
                <span class="pgp-fingerprint" title={record.fingerprint}>{record.fingerprint}</span>
                <div class="pgp-key-actions">
                  {#if pgpVaultUnlocked}
                    <button
                      class="pgp-icon-btn"
                      onclick={() => openIdentityDialog(record.fingerprint)}
                      title={tl('pgp_addIdentity')}
                      aria-label={tl('pgp_addIdentity')}
                    >
                      <UiIcon name="identity-plus" />
                    </button>
                    <button
                      class="pgp-icon-btn"
                      onclick={() => openOwnPublicKeyDialog(record.fingerprint)}
                      title={tl('pgp_importOwnPublicKey')}
                      aria-label={tl('pgp_importOwnPublicKey')}
                    >
                      <UiIcon name="import" />
                    </button>
                  {/if}
                  {#if pgpVaultUnlocked || pgpOwnPublicKeys.has(record.fingerprint)}
                    <button
                      class="pgp-icon-btn"
                      onclick={() => onPgpExportPublicKey(record.fingerprint)}
                      title={tl('pgp_exportPublicKey')}
                      aria-label={tl('pgp_exportPublicKey')}
                    >
                      <UiIcon name="export" />
                    </button>
                  {/if}
                  {#if pgpVaultUnlocked}
                    <button
                      class="pgp-icon-btn danger"
                      onclick={() => onPgpRemoveKey(record.fingerprint)}
                      title={tl('pgp_removeKey')}
                      aria-label={tl('pgp_removeKey')}
                    >
                      <UiIcon name="trash" />
                    </button>
                  {/if}
                </div>
              </div>
              {#if record.fingerprint === pgpJustEnrolled}
                <span class="pgp-success">{tl('pgp_enrolled')}</span>
              {/if}
              {#if displayedKeyIdentities(record.fingerprint, pgpUnlockRevision, pgpStoredKeyIdentities).length > 0}
                <span class="pgp-identity-heading">{tl('pgp_keyIdentities')}</span>
                {#each displayedKeyIdentities(record.fingerprint, pgpUnlockRevision, pgpStoredKeyIdentities) as identity (identity.userID)}
                  <span class="pgp-key-identity">{identity.userID}</span>
                {/each}
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      <div class="pgp-section-heading">
        <h4>{tl('pgp_devicesHeading')}</h4>
        {#if pgpVaultUnlocked}
          <button
            class="pgp-icon-btn"
            onclick={openAccessDialog}
            title={tl('pgp_addAccessMethod')}
            aria-label={tl('pgp_addAccessMethod')}
          >
            <UiIcon name="key-plus" />
          </button>
        {/if}
      </div>
      <p class="pgp-section-hint">{tl('pgp_accessMethodsHint')}</p>
      <ul class="pgp-key-list">
        {#each pgpVault.devices.filter(device => device.status === 'active') as device (device.cred_id)}
          <li class="pgp-key-item">
            <div class="pgp-key-info">
              <span>{device.device_label ?? tl('pgp_defaultAccessMethod')}</span>
              {#if pgpLocalActiveMethods.some(method => method.cred_id === device.cred_id)}
                <span class="pgp-key-status local">{tl('pgp_thisBrowser')}</span>
              {/if}
            </div>
            <div class="pgp-key-actions">
              {#if pgpVaultUnlocked}
                <button
                  class="pgp-icon-btn danger"
                  onclick={() => onPgpRemoveDevice(device)}
                  title={tl('pgp_removeDevice')}
                  aria-label={tl('pgp_removeDevice')}
                >
                  <UiIcon name="trash" />
                </button>
              {/if}
            </div>
          </li>
        {/each}
      </ul>

      {#if pgpVault.devices.some(device => device.status === 'pending')}
        <div class="pgp-section-heading">
          <h4>{tl('pgp_pendingDevices')}</h4>
        </div>
        <ul class="pgp-key-list">
          {#each pgpVault.devices.filter(device => device.status === 'pending') as device (device.cred_id)}
            <li class="pgp-key-item">
              <div class="pgp-key-info">
                <span>{device.device_label ?? tl('pgp_defaultAccessMethod')}</span>
                {#if pgpLocalPendingMethods.some(method => method.cred_id === device.cred_id)}
                  <span class="pgp-key-status local">{tl('pgp_thisBrowser')}</span>
                {/if}
              </div>
              <div class="pgp-key-actions">
                {#if pgpVaultUnlocked}
                  <button class="secondary small" onclick={() => onPgpApproveDevice(device)}>{tl('pgp_approveDevice')}</button>
                  <button
                    class="pgp-icon-btn danger"
                    onclick={() => onPgpRemoveDevice(device)}
                    title={tl('pgp_removeDevice')}
                    aria-label={tl('pgp_removeDevice')}
                  >
                    <UiIcon name="trash" />
                  </button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}

      {#if pgpLocalPendingMethods.length > 0 && pgpPairingSas}
        <p class="pgp-pairing">{tl('pgp_pairingWaiting', { sas: pgpPairingSas })}</p>
      {/if}
      {#if !pgpVaultUnlocked && pgpLocalActiveMethods.length === 0 && pgpLocalPendingMethods.length === 0}
        <div class="pgp-global-actions">
          <button class="secondary" onclick={openAccessDialog}>{tl('pgp_setupBrowser')}</button>
          <button class="secondary" onclick={() => openUnlockDialog(true)}>{tl('pgp_recoverAccessMethod')}</button>
        </div>
    {/if}

    {:else}
      <div class="pgp-heading">
        <h3>{tl('pgp_vaultHeading')}</h3>
        <button
          class="pgp-icon-btn"
          onclick={openPrivateKeyDialog}
          title={tl('pgp_importPrivateKeyButton')}
          aria-label={tl('pgp_importPrivateKeyButton')}
        >
          <UiIcon name="import" />
        </button>
      </div>
      <p>{tl('pgp_setupHint')}</p>
      {#if pgpHasCredentials}
        <p>{tl('pgp_partialEnrollment')}</p>
        <button class="secondary danger" onclick={onPgpResetVault} disabled={pgpResetting} aria-busy={pgpResetting}>
          {tl('pgp_startOverButton')}
        </button>
      {/if}
    {/if}

    <section class="pgp-public-keys">
      <div class="pgp-section-heading">
        <h3>{tl('pgp_publicKeysHeading')}</h3>
        <button
          class="pgp-icon-btn"
          onclick={openPublicKeyDialog}
          title={tl('pgp_importPublicKeyButton')}
          aria-label={tl('pgp_importPublicKeyButton')}
        >
          <UiIcon name="import" />
        </button>
      </div>
      <p>{tl('pgp_publicKeysHint')}</p>
      {#if pgpPublicImportError}<p class="pgp-error">{pgpPublicImportError}</p>{/if}
      {#if pgpPublicImportSuccess}<p class="pgp-success">{pgpPublicImportSuccess}</p>{/if}

      {#if pgpTrustedSenderKeys.length > 0}
        <h4>{tl('pgp_trustedKeys')}</h4>
        <ul class="pgp-key-list">
          {#each pgpTrustedSenderKeys as key (key.fingerprint)}
            <li class="pgp-key-item">
              <div class="pgp-key-info pgp-public-key-info">
                <span>{key.email || tl('pgp_noKeyEmail')}</span>
                <span class="pgp-fingerprint">{key.fingerprint.slice(0, 16)}…</span>
                {#if key.source === 'harvested_autocrypt'}
                  <span class="pgp-key-source">{tl('pgp_sourceAutocrypt')}</span>
                {:else if key.source === 'harvested_attachment'}
                  <span class="pgp-key-source">{tl('pgp_sourceSignedAttachment')}</span>
                {/if}
              </div>
              <div class="pgp-key-actions">
                <button
                  class="pgp-icon-btn danger"
                  onclick={() => onPgpRemovePublicKey(key.fingerprint)}
                  title={tl('pgp_removeKey')}
                  aria-label={tl('pgp_removeKey')}
                >
                  <UiIcon name="trash" />
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpPrivateKeyDialog}
  onclick={(e) => { if (e.target === e.currentTarget) closePrivateKeyDialog(); }}
  oncancel={(e) => { e.preventDefault(); closePrivateKeyDialog(); }}
>
  <article>
    <header>
      <h3>{tl('pgp_importHeading')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={closePrivateKeyDialog}></button>
    </header>
    {#if pgpEnrollError}<p class="pgp-error">{pgpEnrollError}</p>{/if}
    {#if pgpImportError}<p class="pgp-error">{pgpImportError}</p>{/if}
    <form onsubmit={(e) => { e.preventDefault(); pgpImportedKey ? (pgpVault ? onPgpAddKey() : onPgpEnroll()) : onPgpImport(); }}>
      {#if !pgpImportedKey}
        <label>
          <span>{tl('pgp_importArmoredKey')}</span>
          <textarea rows="6" bind:value={pgpArmoredKey} placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"></textarea>
        </label>
        <label>
          <span>{tl('pgp_importPassphrase')}</span>
          <input type="password" bind:value={pgpPassphrase} autocomplete="current-password" />
        </label>
      {:else}
        <p class="pgp-fingerprint"><strong>{tl('pgp_fingerprint')}:</strong> {pgpImportedFingerprint}</p>
        {#if !pgpVault}
          <p class="pgp-section-hint">{tl('pgp_firstAccessMethodHint')}</p>
          <label>
            <span>{tl('pgp_deviceLabel')}</span>
            <input type="text" bind:value={pgpDeviceLabel} placeholder={tl('pgp_deviceLabelPlaceholder')} />
          </label>
        {/if}
      {/if}
      <footer>
        {#if !pgpImportedKey}
          <button type="submit" disabled={pgpImporting || !pgpArmoredKey} aria-busy={pgpImporting}>
            {pgpImporting ? tl('pgp_importing') : tl('pgp_importButton')}
          </button>
        {:else if pgpVault}
          <button type="submit" disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
            {pgpEnrolling ? tl('pgp_importing') : tl('pgp_addToVaultButton')}
          </button>
        {:else}
          <button type="submit" disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
            {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_createVaultButton')}
          </button>
        {/if}
        <button type="button" class="secondary" onclick={closePrivateKeyDialog}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpIdentityDialog}
  onclick={(e) => { if (e.target === e.currentTarget) pgpIdentityDialog?.close(); }}
>
  <article>
    <header>
      <h3>{tl('pgp_addIdentity')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => pgpIdentityDialog?.close()}></button>
    </header>
    {#if pgpKeyActionError}<p class="pgp-error">{pgpKeyActionError}</p>{/if}
    <form onsubmit={(e) => { e.preventDefault(); onPgpAddIdentity(); }}>
      <label>
        <span>{tl('pgp_identityName')}</span>
        <input type="text" bind:value={pgpIdentityName} />
      </label>
      <label>
        <span>{tl('pgp_identityEmail')}</span>
        <input type="email" bind:value={pgpIdentityEmail} required />
      </label>
      <label>
        <span>{tl('pgp_identityComment')}</span>
        <input type="text" bind:value={pgpIdentityComment} />
      </label>
      <footer>
        <button type="submit" disabled={pgpKeyActionSaving} aria-busy={pgpKeyActionSaving}>
          {pgpKeyActionSaving ? tl('mail_saving') : tl('mail_add')}
        </button>
        <button type="button" class="secondary" onclick={() => pgpIdentityDialog?.close()}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpOwnPublicKeyDialog}
  onclick={(e) => { if (e.target === e.currentTarget) pgpOwnPublicKeyDialog?.close(); }}
>
  <article>
    <header>
      <h3>{tl('pgp_importOwnPublicKey')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => pgpOwnPublicKeyDialog?.close()}></button>
    </header>
    <p class="pgp-section-hint">{tl('pgp_importOwnPublicKeyHint')}</p>
    {#if pgpKeyActionError}<p class="pgp-error">{pgpKeyActionError}</p>{/if}
    <form onsubmit={(e) => { e.preventDefault(); onPgpOwnPublicImport(); }}>
      <label>
        <span>{tl('pgp_importArmoredPublicKey')}</span>
        <textarea rows="6" bind:value={pgpManagedPublicArmored} placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----"></textarea>
      </label>
      <footer>
        <button type="submit" disabled={pgpKeyActionSaving || !pgpManagedPublicArmored} aria-busy={pgpKeyActionSaving}>
          {pgpKeyActionSaving ? tl('pgp_importing') : tl('pgp_importPublicKeyButton')}
        </button>
        <button type="button" class="secondary" onclick={() => pgpOwnPublicKeyDialog?.close()}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpAccessDialog}
  onclick={(e) => { if (e.target === e.currentTarget) pgpAccessDialog?.close(); }}
>
  <article>
    <header>
      <h3>{pgpVaultUnlocked ? tl('pgp_addAccessMethod') : tl('pgp_setupBrowser')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => pgpAccessDialog?.close()}></button>
    </header>
    <p class="pgp-section-hint">{tl('pgp_addAccessMethodHint')}</p>
    {#if pgpEnrollError}<p class="pgp-error">{pgpEnrollError}</p>{/if}
    <form onsubmit={(e) => { e.preventDefault(); onPgpRegisterDevice(); }}>
      <label>
        <span>{tl('pgp_deviceLabel')}</span>
        <input type="text" bind:value={pgpDeviceLabel} placeholder={tl('pgp_deviceLabelPlaceholder')} />
      </label>
      <footer>
        <button type="submit" disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
          {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_addAccessMethod')}
        </button>
        <button type="button" class="secondary" onclick={() => pgpAccessDialog?.close()}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpUnlockDialog}
  onclick={(e) => { if (e.target === e.currentTarget) pgpUnlockDialog?.close(); }}
>
  <article>
    <header>
      <h3>{pgpUnlockDialogRecovery ? tl('pgp_recoverAccessMethod') : tl('pgp_unlockHeading')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => pgpUnlockDialog?.close()}></button>
    </header>
    <p class="pgp-section-hint">
      {pgpUnlockDialogRecovery ? tl('pgp_recoverAccessHint') : tl('pgp_chooseAccessMethod')}
    </p>
    {#if pgpUnlockError}<p class="pgp-error">{pgpUnlockError}</p>{/if}
    <ul class="pgp-key-list">
      {#each pgpUnlockCandidates as device (device.cred_id)}
        <li class="pgp-key-item">
          <span>{device.device_label ?? tl('pgp_defaultAccessMethod')}</span>
          <button class="secondary small" onclick={() => selectUnlockMethod(device)} disabled={pgpUnlocking}>
            {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockVaultButton')}
          </button>
        </li>
      {/each}
    </ul>
  </article>
</dialog>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={pgpPublicKeyDialog}
  onclick={(e) => { if (e.target === e.currentTarget) pgpPublicKeyDialog?.close(); }}
>
  <article>
    <header>
      <h3>{tl('pgp_importPublicKeyButton')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => pgpPublicKeyDialog?.close()}></button>
    </header>
    {#if pgpPublicImportError}<p class="pgp-error">{pgpPublicImportError}</p>{/if}
    <form onsubmit={(e) => { e.preventDefault(); onPgpPublicImport(); }}>
      <label>
        <span>{tl('pgp_importArmoredPublicKey')}</span>
        <textarea rows="6" bind:value={pgpPublicArmoredKey} placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----"></textarea>
      </label>
      <footer>
        <button type="submit" disabled={pgpPublicImporting || !pgpPublicArmoredKey} aria-busy={pgpPublicImporting}>
          {pgpPublicImporting ? tl('pgp_importing') : tl('pgp_importPublicKeyButton')}
        </button>
        <button type="button" class="secondary" onclick={() => pgpPublicKeyDialog?.close()}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
</dialog>


<style>
  .pgp-error {
    color: var(--pico-del-color);
    font-size: 0.9rem;
  }

  .pgp-success {
    color: var(--pico-ins-color);
    font-size: 0.9rem;
  }

  .pgp-fingerprint {
    font-family: monospace;
    font-size: 0.85rem;
    word-break: break-all;
  }

  .pgp-key-list {
    list-style: none;
    padding: 0;
    margin: 0 0 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .pgp-heading,
  .pgp-section-heading {
    display: flex;
    align-items: center;
    gap: 0.45rem;
  }

  .pgp-heading {
    margin-bottom: 1rem;
  }

  .pgp-heading h3,
  .pgp-section-heading h3,
  .pgp-section-heading h4 {
    margin: 0;
  }

  .pgp-section-heading {
    margin-top: 1rem;
    margin-bottom: 0.65rem;
  }

  .pgp-tooltip-anchor {
    display: inline-flex;
  }

  .pgp-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.15rem 0.25rem;
    margin: 0;
    line-height: 1;
    color: var(--pico-muted-color);
    flex-shrink: 0;
    transition: color 0.15s;
  }

  .pgp-icon-btn:hover:not(:disabled) {
    color: var(--pico-color);
    background: none;
  }

  .pgp-icon-btn:disabled {
    color: var(--pico-muted-color);
    opacity: 0.45;
    cursor: not-allowed;
  }

  .pgp-icon-btn.danger:hover:not(:disabled) {
    color: var(--pico-del-color);
  }

  .pgp-section-hint {
    color: var(--pico-muted-color);
    font-size: 0.9rem;
    margin-top: 0;
    margin-bottom: 0.75rem;
  }

  .pgp-pairing {
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.75rem;
    font-size: 0.9rem;
  }

  .pgp-public-keys {
    border-top: 1px solid var(--pico-muted-border-color);
    margin-top: 1.5rem;
    padding-top: 1.25rem;
  }

  .pgp-public-key-info {
    align-items: flex-start;
    flex-direction: column;
    gap: 0.15rem;
  }

  .pgp-private-key-info {
    align-items: flex-start;
    flex-direction: column;
    gap: 0.2rem;
    text-align: left;
  }

  .pgp-private-key-item {
    align-items: stretch;
  }

  .pgp-private-key-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
    width: 100%;
  }

  .pgp-private-key-header .pgp-fingerprint {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    word-break: normal;
  }

  .pgp-private-key-header .pgp-key-actions {
    flex-wrap: nowrap;
  }

  .pgp-identity-heading {
    color: var(--pico-muted-color);
    font-size: 0.8rem;
  }

  .pgp-key-identity {
    font-size: 0.9rem;
  }

  .pgp-key-source {
    color: var(--pico-muted-color);
    font-size: 0.85rem;
  }

  .pgp-key-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    text-align: left;
  }

  .pgp-key-info {
    display: flex;
    min-width: 0;
    flex: 1;
  }

  .pgp-key-status {
    font-size: 0.75rem;
    padding: 0.1rem 0.4rem;
    border-radius: 999px;
    white-space: nowrap;
  }

  .pgp-key-status.unlocked {
    background: color-mix(in srgb, var(--pico-ins-color) 15%, transparent);
    color: var(--pico-ins-color);
  }

  .pgp-key-status.locked {
    background: color-mix(in srgb, var(--pico-muted-color) 15%, transparent);
    color: var(--pico-muted-color);
  }

  .pgp-key-status.local {
    background: color-mix(in srgb, var(--pico-primary) 12%, transparent);
    color: var(--pico-primary);
  }

  .pgp-key-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.4rem;
    flex-shrink: 0;
  }

  .pgp-key-actions button.small {
    padding: 0.2rem 0.5rem;
    font-size: 0.8rem;
    margin-bottom: 0;
  }

  .pgp-global-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
  }

  .pgp-global-actions button {
    margin-bottom: 0;
    width: auto;
  }

  /* ── CalDAV account cards ────────────────────────────────────────────────── */

  .caldav-account-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-bottom: 1rem;
  }

  .caldav-account {
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.875rem 1rem;
  }

  .caldav-account-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.65rem;
  }

  .caldav-account-header h3 {
    margin: 0 0 0.15rem;
    font-size: 1rem;
    font-weight: 600;
  }

  .caldav-account-header p {
    margin: 0;
    font-size: 0.85rem;
    color: var(--pico-muted-color);
  }

  .caldav-account-actions {
    display: flex;
    gap: 0.35rem;
    flex-shrink: 0;
    align-items: flex-start;
  }

  /* Match global fieldset legend style for sub-section labels */
  .caldav-account h4 {
    font-size: 0.8125rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
    color: var(--pico-muted-color);
    margin: 0.5rem 0 0.35rem;
  }

  /* ── Encryption panel ────────────────────────────────────────────────────── */

  .encryption-panel {
    max-width: 32rem;
  }

  /* ── Danger secondary button ─────────────────────────────────────────────── */

  button.secondary.danger {
    --pico-color: var(--pico-del-color);
    --pico-border-color: color-mix(in srgb, var(--pico-del-color) 50%, transparent);
  }

  button.secondary.danger:hover,
  button.secondary.danger:focus {
    --pico-background-color: color-mix(in srgb, var(--pico-del-color) 10%, transparent);
    --pico-border-color: var(--pico-del-color);
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
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

<div data-layout="stack" data-gap="lg">
  {#if pgpVault}
    <section data-layout="stack" data-gap="md">
      <div data-layout="row" data-gap="sm">
        <h3>{tl('pgp_vaultHeading')}</h3>
        {#if pgpVaultUnlocked}
          <span data-badge data-tone="success" data-icon="lock-open">{tl('pgp_vaultUnlocked')}</span>
        {:else}
          <span data-badge data-tone="neutral" data-icon="lock">{tl('pgp_vaultLocked')}</span>
        {/if}
      </div>
      {#if pgpUnlockError}<div data-callout data-tone="danger" role="alert">{pgpUnlockError}</div>{/if}
      {#if pgpDeviceActionError}<div data-callout data-tone="danger" role="alert">{pgpDeviceActionError}</div>{/if}

      <div data-layout="row" data-gap="sm">
        {#if pgpVaultUnlocked}
          <button type="button" data-variant="secondary" data-icon="lock" onclick={onPgpLockVault}>{tl('pgp_lockVault')}</button>
        {:else if pgpLocalActiveMethods.length === 1}
          <button type="button" data-icon="lock-open" onclick={() => onPgpUnlockVault(pgpLocalActiveMethods[0])} disabled={pgpUnlocking} aria-busy={pgpUnlocking}>
            {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockVaultButton')}
          </button>
        {:else if pgpLocalActiveMethods.length > 1}
          <button type="button" data-icon="lock-open" onclick={() => openUnlockDialog(false)}>{tl('pgp_unlockVaultButton')}</button>
        {/if}
        {#if pgpVaultUnlocked}
          <button type="button" data-variant="secondary" onclick={onPgpRotateVaultKey}>{tl('pgp_rotateVaultKey')}</button>
        {/if}
        <button type="button" data-variant="outline" data-tone="danger" onclick={onPgpResetVault} disabled={pgpResetting} aria-busy={pgpResetting}>
          {tl('pgp_startOverButton')}
        </button>
      </div>
    </section>

    <section data-layout="stack" data-gap="sm">
      <div data-layout="row" data-gap="sm" data-wrap="none">
        <h4 data-grow>{tl('pgp_privateKeysHeading')}</h4>
        <button
          type="button"
          data-variant="ghost"
          data-size="sm"
          data-icon="import"
          onclick={openPrivateKeyDialog}
          disabled={!pgpVaultUnlocked}
          aria-label={pgpVaultUnlocked ? tl('pgp_importPrivateKeyButton') : tl('pgp_importUnlockFirst')}
          data-tooltip
        ></button>
      </div>
      <p data-muted>{tl('pgp_privateKeysHint')}</p>
      {#if pgpVaultRecords.length > 0}
        <ul data-list>
          {#each pgpVaultRecords as record (record.fingerprint)}
            <li data-layout="stack" data-gap="xs">
              <div data-layout="row" data-gap="sm" data-wrap="none">
                <code data-grow data-truncate title={record.fingerprint}>{record.fingerprint}</code>
                <div data-layout="row" data-gap="xs" data-wrap="none">
                  {#if pgpVaultUnlocked}
                    <button
                      type="button"
                      data-variant="ghost"
                      data-size="sm"
                      data-icon="user-plus"
                      onclick={() => openIdentityDialog(record.fingerprint)}
                      aria-label={tl('pgp_addIdentity')}
                      data-tooltip
                    ></button>
                    <button
                      type="button"
                      data-variant="ghost"
                      data-size="sm"
                      data-icon="import"
                      onclick={() => openOwnPublicKeyDialog(record.fingerprint)}
                      aria-label={tl('pgp_importOwnPublicKey')}
                      data-tooltip
                    ></button>
                  {/if}
                  {#if pgpVaultUnlocked || pgpOwnPublicKeys.has(record.fingerprint)}
                    <button
                      type="button"
                      data-variant="ghost"
                      data-size="sm"
                      data-icon="download"
                      onclick={() => onPgpExportPublicKey(record.fingerprint)}
                      aria-label={tl('pgp_exportPublicKey')}
                      data-tooltip
                    ></button>
                  {/if}
                  {#if pgpVaultUnlocked}
                    <button
                      type="button"
                      data-variant="ghost"
                      data-size="sm"
                      data-tone="danger"
                      data-icon="trash-2"
                      onclick={() => onPgpRemoveKey(record.fingerprint)}
                      aria-label={tl('pgp_removeKey')}
                      data-tooltip
                    ></button>
                  {/if}
                </div>
              </div>
              {#if record.fingerprint === pgpJustEnrolled}
                <small data-tone="success">{tl('pgp_enrolled')}</small>
              {/if}
              {#if displayedKeyIdentities(record.fingerprint, pgpUnlockRevision, pgpStoredKeyIdentities).length > 0}
                <small data-caption>{tl('pgp_keyIdentities')}</small>
                {#each displayedKeyIdentities(record.fingerprint, pgpUnlockRevision, pgpStoredKeyIdentities) as identity (identity.userID)}
                  <span>{identity.userID}</span>
                {/each}
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section data-layout="stack" data-gap="sm">
      <div data-layout="row" data-gap="sm" data-wrap="none">
        <h4 data-grow>{tl('pgp_devicesHeading')}</h4>
        {#if pgpVaultUnlocked}
          <button
            type="button"
            data-variant="ghost"
            data-size="sm"
            data-icon="key-round"
            onclick={openAccessDialog}
            aria-label={tl('pgp_addAccessMethod')}
            data-tooltip
          ></button>
        {/if}
      </div>
      <p data-muted>{tl('pgp_accessMethodsHint')}</p>
      {#if pgpVault.devices.some(device => device.status === 'active')}
        <ul data-list>
          {#each pgpVault.devices.filter(device => device.status === 'active') as device (device.cred_id)}
            <li data-layout="row" data-justify="between">
              <span data-grow>
                {device.device_label ?? tl('pgp_defaultAccessMethod')}
                {#if pgpLocalActiveMethods.some(method => method.cred_id === device.cred_id)}
                  <span data-badge data-tone="accent">{tl('pgp_thisBrowser')}</span>
                {/if}
              </span>
              {#if pgpVaultUnlocked}
                <button
                  type="button"
                  data-variant="ghost"
                  data-size="sm"
                  data-tone="danger"
                  data-icon="trash-2"
                  onclick={() => onPgpRemoveDevice(device)}
                  aria-label={tl('pgp_removeDevice')}
                  data-tooltip
                ></button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if pgpVault.devices.some(device => device.status === 'pending')}
        <h4>{tl('pgp_pendingDevices')}</h4>
        <ul data-list>
          {#each pgpVault.devices.filter(device => device.status === 'pending') as device (device.cred_id)}
            <li data-layout="row" data-justify="between">
              <span data-grow>
                {device.device_label ?? tl('pgp_defaultAccessMethod')}
                {#if pgpLocalPendingMethods.some(method => method.cred_id === device.cred_id)}
                  <span data-badge data-tone="accent">{tl('pgp_thisBrowser')}</span>
                {/if}
              </span>
              {#if pgpVaultUnlocked}
                <div data-layout="row" data-gap="xs" data-wrap="none">
                  <button type="button" data-variant="secondary" data-size="sm" onclick={() => onPgpApproveDevice(device)}>{tl('pgp_approveDevice')}</button>
                  <button
                    type="button"
                    data-variant="ghost"
                    data-size="sm"
                    data-tone="danger"
                    data-icon="trash-2"
                    onclick={() => onPgpRemoveDevice(device)}
                    aria-label={tl('pgp_removeDevice')}
                    data-tooltip
                  ></button>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if pgpLocalPendingMethods.length > 0 && pgpPairingSas}
        <div data-callout role="status">{tl('pgp_pairingWaiting', { sas: pgpPairingSas })}</div>
      {/if}
      {#if !pgpVaultUnlocked && pgpLocalActiveMethods.length === 0 && pgpLocalPendingMethods.length === 0}
        <div data-layout="row" data-gap="sm">
          <button type="button" data-variant="secondary" onclick={openAccessDialog}>{tl('pgp_setupBrowser')}</button>
          <button type="button" data-variant="secondary" onclick={() => openUnlockDialog(true)}>{tl('pgp_recoverAccessMethod')}</button>
        </div>
      {/if}
    </section>
  {:else}
    <section data-layout="stack" data-gap="md">
      <div data-layout="row" data-gap="sm" data-wrap="none">
        <h3 data-grow>{tl('pgp_vaultHeading')}</h3>
        <button
          type="button"
          data-variant="ghost"
          data-size="sm"
          data-icon="import"
          onclick={openPrivateKeyDialog}
          aria-label={tl('pgp_importPrivateKeyButton')}
          data-tooltip
        ></button>
      </div>
      <p>{tl('pgp_setupHint')}</p>
      {#if pgpHasCredentials}
        <p>{tl('pgp_partialEnrollment')}</p>
        <button type="button" data-variant="outline" data-tone="danger" data-self="start" onclick={onPgpResetVault} disabled={pgpResetting} aria-busy={pgpResetting}>
          {tl('pgp_startOverButton')}
        </button>
      {/if}
    </section>
  {/if}

  <hr />

  <section data-layout="stack" data-gap="sm">
    <div data-layout="row" data-gap="sm" data-wrap="none">
      <h3 data-grow>{tl('pgp_publicKeysHeading')}</h3>
      <button
        type="button"
        data-variant="ghost"
        data-size="sm"
        data-icon="import"
        onclick={openPublicKeyDialog}
        aria-label={tl('pgp_importPublicKeyButton')}
        data-tooltip
      ></button>
    </div>
    <p data-muted>{tl('pgp_publicKeysHint')}</p>
    {#if pgpPublicImportError}<div data-callout data-tone="danger" role="alert">{pgpPublicImportError}</div>{/if}
    {#if pgpPublicImportSuccess}<div data-callout data-tone="success" role="status">{pgpPublicImportSuccess}</div>{/if}

    {#if pgpTrustedSenderKeys.length > 0}
      <h4>{tl('pgp_trustedKeys')}</h4>
      <ul data-list>
        {#each pgpTrustedSenderKeys as key (key.fingerprint)}
          <li data-layout="row" data-justify="between" data-wrap="none">
            <div data-grow data-layout="stack" data-gap="none">
              <span data-truncate>{key.email || tl('pgp_noKeyEmail')}</span>
              <small><code>{key.fingerprint.slice(0, 16)}…</code></small>
              {#if key.source === 'harvested_autocrypt'}
                <small data-muted>{tl('pgp_sourceAutocrypt')}</small>
              {:else if key.source === 'harvested_attachment'}
                <small data-muted>{tl('pgp_sourceSignedAttachment')}</small>
              {/if}
            </div>
            <button
              type="button"
              data-variant="ghost"
              data-size="sm"
              data-tone="danger"
              data-icon="trash-2"
              onclick={() => onPgpRemovePublicKey(key.fingerprint)}
              aria-label={tl('pgp_removeKey')}
              data-tooltip
            ></button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

<dialog
  id="pgp-private-key-dialog"
  closedby="any"
  aria-labelledby="pgp-private-key-title"
  bind:this={pgpPrivateKeyDialog}
  oncancel={(e) => { e.preventDefault(); closePrivateKeyDialog(); }}
>
  <header>
    <h2 id="pgp-private-key-title">{tl('pgp_importHeading')}</h2>
    <button type="button" commandfor="pgp-private-key-dialog" command="close" aria-label={tl('weekly_aria_close')} onclick={closePrivateKeyDialog}></button>
  </header>
  {#if pgpEnrollError}<div data-callout data-tone="danger" role="alert">{pgpEnrollError}</div>{/if}
  {#if pgpImportError}<div data-callout data-tone="danger" role="alert">{pgpImportError}</div>{/if}
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); pgpImportedKey ? (pgpVault ? onPgpAddKey() : onPgpEnroll()) : onPgpImport(); }}>
    {#if !pgpImportedKey}
      <label>
        {tl('pgp_importArmoredKey')}
        <textarea rows="6" bind:value={pgpArmoredKey} placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"></textarea>
      </label>
      <label>
        {tl('pgp_importPassphrase')}
        <input type="password" bind:value={pgpPassphrase} autocomplete="current-password" />
      </label>
    {:else}
      <p><strong>{tl('pgp_fingerprint')}:</strong> <code>{pgpImportedFingerprint}</code></p>
      {#if !pgpVault}
        <p data-muted>{tl('pgp_firstAccessMethodHint')}</p>
        <label>
          {tl('pgp_deviceLabel')}
          <input type="text" bind:value={pgpDeviceLabel} placeholder={tl('pgp_deviceLabelPlaceholder')} />
        </label>
      {/if}
    {/if}
    <footer>
      <button type="button" data-variant="secondary" commandfor="pgp-private-key-dialog" command="close" onclick={closePrivateKeyDialog}>{tl('event_cancel')}</button>
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
    </footer>
  </form>
</dialog>

<dialog id="pgp-identity-dialog" closedby="any" aria-labelledby="pgp-identity-title" bind:this={pgpIdentityDialog}>
  <header>
    <h2 id="pgp-identity-title">{tl('pgp_addIdentity')}</h2>
    <button type="button" commandfor="pgp-identity-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  {#if pgpKeyActionError}<div data-callout data-tone="danger" role="alert">{pgpKeyActionError}</div>{/if}
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onPgpAddIdentity(); }}>
    <label>
      {tl('pgp_identityName')}
      <input type="text" bind:value={pgpIdentityName} />
    </label>
    <label>
      {tl('pgp_identityEmail')}
      <input type="email" bind:value={pgpIdentityEmail} required />
    </label>
    <label>
      {tl('pgp_identityComment')}
      <input type="text" bind:value={pgpIdentityComment} />
    </label>
    <footer>
      <button type="button" data-variant="secondary" commandfor="pgp-identity-dialog" command="close">{tl('event_cancel')}</button>
      <button type="submit" disabled={pgpKeyActionSaving} aria-busy={pgpKeyActionSaving}>
        {pgpKeyActionSaving ? tl('mail_saving') : tl('mail_add')}
      </button>
    </footer>
  </form>
</dialog>

<dialog id="pgp-own-public-key-dialog" closedby="any" aria-labelledby="pgp-own-public-key-title" bind:this={pgpOwnPublicKeyDialog}>
  <header>
    <h2 id="pgp-own-public-key-title">{tl('pgp_importOwnPublicKey')}</h2>
    <button type="button" commandfor="pgp-own-public-key-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  <p data-muted>{tl('pgp_importOwnPublicKeyHint')}</p>
  {#if pgpKeyActionError}<div data-callout data-tone="danger" role="alert">{pgpKeyActionError}</div>{/if}
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onPgpOwnPublicImport(); }}>
    <label>
      {tl('pgp_importArmoredPublicKey')}
      <textarea rows="6" bind:value={pgpManagedPublicArmored} placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----"></textarea>
    </label>
    <footer>
      <button type="button" data-variant="secondary" commandfor="pgp-own-public-key-dialog" command="close">{tl('event_cancel')}</button>
      <button type="submit" disabled={pgpKeyActionSaving || !pgpManagedPublicArmored} aria-busy={pgpKeyActionSaving}>
        {pgpKeyActionSaving ? tl('pgp_importing') : tl('pgp_importPublicKeyButton')}
      </button>
    </footer>
  </form>
</dialog>

<dialog id="pgp-access-dialog" closedby="any" aria-labelledby="pgp-access-title" bind:this={pgpAccessDialog}>
  <header>
    <h2 id="pgp-access-title">{pgpVaultUnlocked ? tl('pgp_addAccessMethod') : tl('pgp_setupBrowser')}</h2>
    <button type="button" commandfor="pgp-access-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  <p data-muted>{tl('pgp_addAccessMethodHint')}</p>
  {#if pgpEnrollError}<div data-callout data-tone="danger" role="alert">{pgpEnrollError}</div>{/if}
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onPgpRegisterDevice(); }}>
    <label>
      {tl('pgp_deviceLabel')}
      <input type="text" bind:value={pgpDeviceLabel} placeholder={tl('pgp_deviceLabelPlaceholder')} />
    </label>
    <footer>
      <button type="button" data-variant="secondary" commandfor="pgp-access-dialog" command="close">{tl('event_cancel')}</button>
      <button type="submit" disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
        {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_addAccessMethod')}
      </button>
    </footer>
  </form>
</dialog>

<dialog id="pgp-unlock-dialog" closedby="any" aria-labelledby="pgp-unlock-title" bind:this={pgpUnlockDialog}>
  <header>
    <h2 id="pgp-unlock-title">{pgpUnlockDialogRecovery ? tl('pgp_recoverAccessMethod') : tl('pgp_unlockHeading')}</h2>
    <button type="button" commandfor="pgp-unlock-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  <p data-muted>
    {pgpUnlockDialogRecovery ? tl('pgp_recoverAccessHint') : tl('pgp_chooseAccessMethod')}
  </p>
  {#if pgpUnlockError}<div data-callout data-tone="danger" role="alert">{pgpUnlockError}</div>{/if}
  {#if pgpUnlockCandidates.length > 0}
    <ul data-list>
      {#each pgpUnlockCandidates as device (device.cred_id)}
        <li data-layout="row" data-justify="between">
          <span data-grow>{device.device_label ?? tl('pgp_defaultAccessMethod')}</span>
          <button type="button" data-variant="secondary" data-size="sm" onclick={() => selectUnlockMethod(device)} disabled={pgpUnlocking} aria-busy={pgpUnlocking}>
            {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockVaultButton')}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</dialog>

<dialog id="pgp-public-key-dialog" closedby="any" aria-labelledby="pgp-public-key-title" bind:this={pgpPublicKeyDialog}>
  <header>
    <h2 id="pgp-public-key-title">{tl('pgp_importPublicKeyButton')}</h2>
    <button type="button" commandfor="pgp-public-key-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  {#if pgpPublicImportError}<div data-callout data-tone="danger" role="alert">{pgpPublicImportError}</div>{/if}
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onPgpPublicImport(); }}>
    <label>
      {tl('pgp_importArmoredPublicKey')}
      <textarea rows="6" bind:value={pgpPublicArmoredKey} placeholder="-----BEGIN PGP PUBLIC KEY BLOCK-----"></textarea>
    </label>
    <footer>
      <button type="button" data-variant="secondary" commandfor="pgp-public-key-dialog" command="close">{tl('event_cancel')}</button>
      <button type="submit" disabled={pgpPublicImporting || !pgpPublicArmoredKey} aria-busy={pgpPublicImporting}>
        {pgpPublicImporting ? tl('pgp_importing') : tl('pgp_importPublicKeyButton')}
      </button>
    </footer>
  </form>
</dialog>

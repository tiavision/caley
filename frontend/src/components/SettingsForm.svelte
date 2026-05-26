<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, setLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';
  import EventColorDot from './EventColorDot.svelte';
  import CreateCalendarDialog from './CreateCalendarDialog.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import type { CalendarJson } from '../types/calendar.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let caldavBaseUrl = '';
  let caldavUsername = '';
  let caldavPassword = '';
  let hasPassword = false;
  let loading = true;
  let saving = false;
  let deleting = false;
  let errorMsg = '';

  import { readAllVaults, writeVault, deleteVault, type VaultRecord } from '../lib/pgp/vault.ts';
  import { enroll, unlock, resumeEnroll, listCredentials, deleteCredential, type CredentialInfo } from '../lib/pgp/webauthn.ts';
  import { deriveKek, aesEncrypt, aesDecrypt, randomBytes, b64uEncode, b64uDecode } from '../lib/pgp/kek.ts';
  import { importPrivateKey, reencryptPrivateKey, getFingerprint, getPrimaryEmail, extractPublicKey } from '../lib/pgp/keyring.ts';
  import { setKey, lockKey, lock, isKeyUnlocked, isUnlocked } from '../lib/pgp/session.ts';
  import * as openpgp from 'openpgp';

  // ── Tabs ───────────────────────────────────────────────────────────────────
  let tab: 'settings' | 'calendars' | 'encryption' = 'settings';

  // ── Calendars tab ─────────────────────────────────────────────────────────
  let calendars: CalendarJson[] = [];
  let calendarsLoading = false;
  let createCalDialog: CreateCalendarDialog;
  let importDialog: ImportDialog;

  async function fetchCalendars() {
    calendarsLoading = true;
    try {
      const res = await fetch('/api/calendars');
      if (res.ok) calendars = await res.json();
    } catch {}
    calendarsLoading = false;
  }

  async function toggleCalendar(href: string, active: boolean) {
    calendars = calendars.map(c => c.href === href ? { ...c, is_active: active } : c);
    await fetch('/api/calendar/toggle', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: `href=${encodeURIComponent(href)}&active=${active}`,
    });
  }

  // ── Encryption tab state ──────────────────────────────────────────────────
  let pgpVaultRecords: VaultRecord[] = [];
  let pgpServerCreds: CredentialInfo[] = [];
  let pgpPartialEnrollment = false;

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
  let pgpShowAddForm = false;

  let pgpUnlocking = false;
  let pgpUnlockError = '';
  // Reactive unlock status derived from session (re-evaluated on relevant state changes)
  $: pgpUnlockedMap = pgpVaultRecords.reduce((m, v) => {
    m[v.fingerprint] = isKeyUnlocked(v.fingerprint);
    return m;
  }, {} as Record<string, boolean>);
  $: pgpAllUnlocked = pgpVaultRecords.length > 0 && pgpVaultRecords.every(v => pgpUnlockedMap[v.fingerprint]);
  $: pgpAnyUnlocked = pgpVaultRecords.some(v => pgpUnlockedMap[v.fingerprint]);

  async function loadVaultStatus() {
    pgpVaultRecords = await readAllVaults().catch(() => []);
    pgpServerCreds = await listCredentials().catch(() => []);
    pgpPartialEnrollment = pgpServerCreds.length > 0 && pgpVaultRecords.length === 0;
  }

  // Force re-evaluation of reactive unlock map by reassigning the array.
  function refreshUnlockState() {
    pgpVaultRecords = [...pgpVaultRecords];
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

  async function storeKeyToVault(
    importedKey: openpgp.PrivateKey,
    fingerprint: string,
    credentialId: string,
    prfSalt: string,
    prfOutput: ArrayBuffer,
    deviceLabel: string,
  ) {
    const hkdfSalt = randomBytes(32);
    const iv = randomBytes(12);
    const randomPassphrase = b64uEncode(randomBytes(32));
    const reencArmed = await reencryptPrivateKey(importedKey, randomPassphrase);
    const kek = await deriveKek(prfOutput, hkdfSalt);
    const payload = new TextEncoder().encode(JSON.stringify({ armored: reencArmed, passphrase: randomPassphrase }));
    const ciphertext = await aesEncrypt(kek, iv, payload);

    const record: VaultRecord = {
      credentialId,
      prfSalt,
      hkdfSalt: b64uEncode(hkdfSalt),
      iv: b64uEncode(iv),
      ciphertext: b64uEncode(ciphertext),
      fingerprint,
      deviceLabel,
    };
    await writeVault(record);
    return record;
  }

  // First enrollment: register WebAuthn credential + store first key.
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

      // Use a fresh prfSalt (stored in vault; used for future unlock/add calls).
      const prfSalt = b64uEncode(randomBytes(32));
      const record = await storeKeyToVault(
        pgpImportedKey,
        pgpImportedFingerprint,
        result.credentialId,
        prfSalt,
        result.prfOutput,
        pgpDeviceLabel || 'My device',
      );

      await uploadPublicKey(pgpImportedKey, pgpImportedFingerprint);
      setKey(pgpImportedFingerprint, pgpImportedKey);

      pgpVaultRecords = [...pgpVaultRecords, record];
      pgpJustEnrolled = pgpImportedFingerprint;
      clearImportForm();
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = msg === 'prf_not_supported' ? tl('pgp_enrollNoPrf') : `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  // Add an additional key using the existing device credential (authenticate-only).
  async function onPgpAddKey() {
    if (!pgpImportedKey || pgpVaultRecords.length === 0) return;
    pgpEnrolling = true;
    pgpEnrollError = '';
    try {
      // Authenticate with the existing prfSalt so all records stay unlockable
      // with a single WebAuthn touch.
      const existingPrfSalt = pgpVaultRecords[0].prfSalt;
      const { prfOutput } = await unlock(existingPrfSalt);

      const record = await storeKeyToVault(
        pgpImportedKey,
        pgpImportedFingerprint,
        pgpVaultRecords[0].credentialId,
        existingPrfSalt,
        prfOutput,
        pgpVaultRecords[0].deviceLabel,
      );

      await uploadPublicKey(pgpImportedKey, pgpImportedFingerprint);
      setKey(pgpImportedFingerprint, pgpImportedKey);

      pgpVaultRecords = [...pgpVaultRecords, record];
      pgpJustEnrolled = pgpImportedFingerprint;
      pgpShowAddForm = false;
      clearImportForm();
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  // Resume enrollment: credential on server but no local vault.
  async function onPgpResume() {
    if (!pgpImportedKey) return;
    pgpEnrolling = true;
    pgpEnrollError = '';
    try {
      const prfSalt = b64uEncode(randomBytes(32));
      const result = await resumeEnroll(prfSalt);

      const record = await storeKeyToVault(
        pgpImportedKey,
        pgpImportedFingerprint,
        result.credentialId,
        prfSalt,
        result.prfOutput,
        pgpServerCreds[0]?.device_label ?? 'My device',
      );

      await uploadPublicKey(pgpImportedKey, pgpImportedFingerprint);
      setKey(pgpImportedFingerprint, pgpImportedKey);

      pgpVaultRecords = [record];
      pgpPartialEnrollment = false;
      pgpJustEnrolled = pgpImportedFingerprint;
      clearImportForm();
    } catch (e: unknown) {
      const msg = (e instanceof Error) ? e.message : String(e);
      pgpEnrollError = `${tl('pgp_enrollError')}: ${msg}`;
    }
    pgpEnrolling = false;
  }

  async function onPgpStartOver() {
    for (const cred of pgpServerCreds) {
      await deleteCredential(cred.cred_id).catch(() => {});
    }
    pgpServerCreds = [];
    pgpPartialEnrollment = false;
    pgpEnrollError = '';
    clearImportForm();
  }

  async function onPgpUnlockAll() {
    pgpUnlocking = true;
    pgpUnlockError = '';
    try {
      const { prfOutput } = await unlock(pgpVaultRecords[0].prfSalt);
      for (const vault of pgpVaultRecords) {
        try {
          const kek = await deriveKek(prfOutput, b64uDecode(vault.hkdfSalt));
          const plainBuf = await aesDecrypt(kek, b64uDecode(vault.iv), b64uDecode(vault.ciphertext));
          const { armored, passphrase } = JSON.parse(new TextDecoder().decode(plainBuf)) as { armored: string; passphrase: string };
          const key = await importPrivateKey(armored, passphrase);
          setKey(vault.fingerprint, key);
        } catch {
          // Record encrypted under a different PRF output; skip.
        }
      }
      refreshUnlockState();
    } catch {
      pgpUnlockError = tl('pgp_unlockError');
    }
    pgpUnlocking = false;
  }

  function onPgpLockKey(fingerprint: string) {
    lockKey(fingerprint);
    refreshUnlockState();
  }

  function onPgpLockAll() {
    lock();
    refreshUnlockState();
  }

  async function onPgpRemoveKey(fingerprint: string) {
    if (!confirm(tl('pgp_confirmRemoveKey'))) return;
    await deleteVault(fingerprint).catch(() => {});
    lockKey(fingerprint);
    pgpVaultRecords = pgpVaultRecords.filter(v => v.fingerprint !== fingerprint);
  }

  function clearImportForm() {
    pgpImportedKey = null;
    pgpImportedFingerprint = '';
    pgpArmoredKey = '';
    pgpPassphrase = '';
    pgpDeviceLabel = '';
    pgpImportError = '';
  }

  async function uploadPublicKey(key: openpgp.PrivateKey, fingerprint: string) {
    const pubKeyArmored = extractPublicKey(key);
    const email = getPrimaryEmail(key);
    await fetch('/api/pgp/key', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ fingerprint, email, armored: pubKeyArmored, is_own: true, source: 'imported' }),
    }).catch(() => {});
  }

  function switchTab(t: 'settings' | 'calendars' | 'encryption') {
    tab = t;
    if (t === 'calendars' && calendars.length === 0) fetchCalendars();
    if (t === 'encryption') loadVaultStatus();
  }

  const localeOptions = [
    { value: 'en', label: 'English' },
    { value: 'de', label: 'Deutsch' },
  ];
  let selectedLocale = 'en';

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    selectedLocale = locale.slice(0, 2);

    // Unauthenticated users are redirected to the login page by the backend;
    // guard here too in case this page is reached directly.
    const userRes = await getUser();
    if (!userRes.ok) {
      window.location.href = '/';
      return;
    }

    // Load existing settings to prefill the form.
    try {
      const res = await fetch('/api/settings/form');
      if (res.ok) {
        const data = await res.json();
        caldavBaseUrl = data.caldav_base_url || '';
        caldavUsername = data.caldav_username || '';
        hasPassword = !!data.has_password;
      }
    } catch {}
    loading = false;
  });

  function onLocaleChange() {
    setLocale(selectedLocale);
    locale = selectedLocale;
  }

  async function onSubmit() {
    saving = true;
    errorMsg = '';
    const params = new URLSearchParams();
    params.append('caldav_base_url', caldavBaseUrl);
    params.append('caldav_username', caldavUsername);
    params.append('caldav_password', caldavPassword);

    try {
      const res = await fetch('/api/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      const data = await res.json();
      if (res.ok && data.status === 'ok') {
        window.location.href = '/calendar/';
      } else {
        errorMsg = data.error || tl('event_error');
      }
    } catch {
      errorMsg = tl('event_error');
    }
    saving = false;
  }

  async function onDeleteAccount() {
    if (!confirm(tl('settings_deleteAccountConfirm'))) return;
    deleting = true;
    try {
      const res = await fetch('/api/account', { method: 'DELETE' });
      if (res.ok) {
        const data = await res.json();
        window.location.href = data.redirect || '/calendar/';
      }
    } catch {}
    deleting = false;
  }
</script>

<div class="settings-form">
  <div class="settings-header">
    <h2>{tl('settings_heading')}</h2>
    <span class="locale-group">
      🌐
      <select
        class="locale-picker"
        aria-label={tl('settings_language')}
        bind:value={selectedLocale}
        onchange={onLocaleChange}
      >
        {#each localeOptions as opt}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </span>
  </div>

  {#if !loading}
    <div class="tabs" role="tablist">
      <button
        role="tab"
        class:active={tab === 'settings'}
        aria-selected={tab === 'settings'}
        onclick={() => switchTab('settings')}
      >{tl('nav_settings')}</button>
      <button
        role="tab"
        class:active={tab === 'calendars'}
        aria-selected={tab === 'calendars'}
        onclick={() => switchTab('calendars')}
      >{tl('cal_calendars')}</button>
      <button
        role="tab"
        class:active={tab === 'encryption'}
        aria-selected={tab === 'encryption'}
        onclick={() => switchTab('encryption')}
      >{tl('pgp_tabEncryption')}</button>
    </div>
  {/if}

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else if tab === 'settings'}
    <p>{tl('settings_subtitle')}</p>
    <form onsubmit={(e) => { e.preventDefault(); onSubmit(); }}>
      <label>
        <span>{tl('settings_baseUrl')}</span>
        <input
          type="url"
          placeholder={tl('settings_baseUrlPlaceholder')}
          bind:value={caldavBaseUrl}
          required
        />
      </label>

      <label>
        <span>{tl('settings_username')}</span>
        <input
          type="text"
          placeholder={tl('settings_usernamePlaceholder')}
          bind:value={caldavUsername}
          required
        />
      </label>

      <label>
        <span>{tl('settings_password')}</span>
        <input
          type="password"
          placeholder={!hasPassword ? tl('settings_passwordPlaceholder') : tl('settings_passwordHint')}
          bind:value={caldavPassword}
        />
      </label>

      {#if errorMsg}
        <p class="error-msg">{errorMsg}</p>
      {/if}

      <button type="submit" disabled={saving} aria-busy={saving}>
        {tl('settings_submit')}
      </button>
    </form>

    <hr />
    <button class="delete-account-btn" onclick={onDeleteAccount} disabled={deleting} aria-busy={deleting}>
      {tl('settings_deleteAccount')}
    </button>
  {:else if tab === 'calendars'}
    <!-- Calendars tab -->
    {#if calendarsLoading}
      <p aria-busy="true">{tl('nav_loading')}</p>
    {:else}
      <ul class="calendar-list">
        {#each calendars as cal}
          <li>
            <label>
              <input
                type="checkbox"
                checked={cal.is_active}
                onchange={(e) => toggleCalendar(cal.href, (e.target as HTMLInputElement).checked)}
              />
              <EventColorDot color={cal.color} />
              {cal.name}
            </label>
          </li>
        {/each}
      </ul>
      <div class="cal-actions">
        <button class="cal-action-btn" onclick={() => createCalDialog.open()}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          {tl('cal_createCalendar')}
        </button>
        <button class="cal-action-btn" onclick={() => importDialog.open()}>
          {tl('cal_import')}
        </button>
      </div>
    {/if}
  {:else if tab === 'encryption'}
    {#if pgpPartialEnrollment}
      <!-- ── Partial enrollment: credential on server, no local vault ── -->
      <p class="pgp-error">{tl('pgp_partialEnrollment')}</p>
      {#if pgpEnrollError}<p class="pgp-error">{pgpEnrollError}</p>{/if}
      <h3>{tl('pgp_importHeading')}</h3>
      {#if pgpImportError}<p class="pgp-error">{pgpImportError}</p>{/if}
      {#if !pgpImportedKey}
        <label>
          <span>{tl('pgp_importArmoredKey')}</span>
          <textarea rows="6" bind:value={pgpArmoredKey} placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"></textarea>
        </label>
        <label>
          <span>{tl('pgp_importPassphrase')}</span>
          <input type="password" bind:value={pgpPassphrase} autocomplete="current-password" />
        </label>
        <button onclick={onPgpImport} disabled={pgpImporting || !pgpArmoredKey} aria-busy={pgpImporting}>
          {pgpImporting ? tl('pgp_importing') : tl('pgp_importButton')}
        </button>
      {:else}
        <p class="pgp-fingerprint"><strong>{tl('pgp_fingerprint')}:</strong> {pgpImportedFingerprint}</p>
        <button onclick={onPgpResume} disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
          {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_resumeEnrollButton')}
        </button>
      {/if}
      <button class="secondary" onclick={onPgpStartOver}>{tl('pgp_startOverButton')}</button>

    {:else if pgpVaultRecords.length > 0}
      <!-- ── Enrolled keys list ── -->
      <h3>{tl('pgp_enrolledKeys')}</h3>
      {#if pgpUnlockError}<p class="pgp-error">{pgpUnlockError}</p>{/if}
      <ul class="pgp-key-list">
        {#each pgpVaultRecords as record (record.fingerprint)}
          <li class="pgp-key-item">
            <div class="pgp-key-info">
              <span class="pgp-fingerprint">{record.fingerprint.slice(0, 16)}…</span>
              <span class="pgp-key-status {pgpUnlockedMap[record.fingerprint] ? 'unlocked' : 'locked'}">
                {pgpUnlockedMap[record.fingerprint] ? tl('pgp_keyUnlocked') : tl('pgp_keyLocked')}
              </span>
              {#if record.fingerprint === pgpJustEnrolled}
                <span class="pgp-success">{tl('pgp_enrolled')}</span>
              {/if}
            </div>
            <div class="pgp-key-actions">
              {#if pgpUnlockedMap[record.fingerprint]}
                <button class="secondary small" onclick={() => onPgpLockKey(record.fingerprint)}>
                  {tl('pgp_lockKey')}
                </button>
              {/if}
              <button class="secondary small danger" onclick={() => onPgpRemoveKey(record.fingerprint)}>
                {tl('pgp_removeKey')}
              </button>
            </div>
          </li>
        {/each}
      </ul>

      <div class="pgp-global-actions">
        {#if !pgpAllUnlocked}
          <button onclick={onPgpUnlockAll} disabled={pgpUnlocking} aria-busy={pgpUnlocking}>
            {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockAllButton')}
          </button>
        {/if}
        {#if pgpAnyUnlocked}
          <button class="secondary" onclick={onPgpLockAll}>{tl('pgp_lockKey')}</button>
        {/if}
        <button class="secondary" onclick={() => { pgpShowAddForm = !pgpShowAddForm; clearImportForm(); }}>
          {tl('pgp_addKeyButton')}
        </button>
      </div>

      {#if pgpShowAddForm}
        <!-- ── Add key form ── -->
        <h3>{tl('pgp_addKeyHeading')}</h3>
        {#if pgpEnrollError}<p class="pgp-error">{pgpEnrollError}</p>{/if}
        {#if pgpImportError}<p class="pgp-error">{pgpImportError}</p>{/if}
        {#if !pgpImportedKey}
          <label>
            <span>{tl('pgp_importArmoredKey')}</span>
            <textarea rows="6" bind:value={pgpArmoredKey} placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"></textarea>
          </label>
          <label>
            <span>{tl('pgp_importPassphrase')}</span>
            <input type="password" bind:value={pgpPassphrase} autocomplete="current-password" />
          </label>
          <button onclick={onPgpImport} disabled={pgpImporting || !pgpArmoredKey} aria-busy={pgpImporting}>
            {pgpImporting ? tl('pgp_importing') : tl('pgp_importButton')}
          </button>
        {:else}
          <p class="pgp-fingerprint"><strong>{tl('pgp_fingerprint')}:</strong> {pgpImportedFingerprint}</p>
          <button onclick={onPgpAddKey} disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
            {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_enrollButton')}
          </button>
        {/if}
      {/if}

    {:else}
      <!-- ── First-time import form ── -->
      <h3>{tl('pgp_importHeading')}</h3>
      {#if pgpImportError}<p class="pgp-error">{pgpImportError}</p>{/if}
      {#if !pgpImportedKey}
        <label>
          <span>{tl('pgp_importArmoredKey')}</span>
          <textarea rows="6" bind:value={pgpArmoredKey} placeholder="-----BEGIN PGP PRIVATE KEY BLOCK-----"></textarea>
        </label>
        <label>
          <span>{tl('pgp_importPassphrase')}</span>
          <input type="password" bind:value={pgpPassphrase} autocomplete="current-password" />
        </label>
        <button onclick={onPgpImport} disabled={pgpImporting || !pgpArmoredKey} aria-busy={pgpImporting}>
          {pgpImporting ? tl('pgp_importing') : tl('pgp_importButton')}
        </button>
      {:else}
        <p class="pgp-fingerprint"><strong>{tl('pgp_fingerprint')}:</strong> {pgpImportedFingerprint}</p>
        <h3>{tl('pgp_enrollHeading')}</h3>
        {#if pgpEnrollError}<p class="pgp-error">{pgpEnrollError}</p>{/if}
        <label>
          <span>{tl('pgp_deviceLabel')}</span>
          <input type="text" bind:value={pgpDeviceLabel} placeholder={tl('pgp_deviceLabelPlaceholder')} />
        </label>
        <button onclick={onPgpEnroll} disabled={pgpEnrolling} aria-busy={pgpEnrolling}>
          {pgpEnrolling ? tl('pgp_enrolling') : tl('pgp_enrollButton')}
        </button>
      {/if}
    {/if}
  {/if}
</div>

<CreateCalendarDialog
  bind:this={createCalDialog}
  {locale}
  oncreated={(cal) => { calendars = [...calendars, cal]; }}
/>
<ImportDialog
  bind:this={importDialog}
  {calendars}
  {locale}
  onimported={fetchCalendars}
/>

<style>
  .settings-form {
    max-width: 32rem;
    margin: 2rem auto;
  }

  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;

    h2 {
      margin: 0;
      flex: 1;
    }
  }

  .locale-group {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
  }

  .delete-account-btn {
    background: none;
    border: 1px solid var(--pico-del-color);
    color: var(--pico-del-color);
    margin-top: 0.5rem;
  }

  .delete-account-btn:hover {
    background-color: var(--pico-del-color);
    color: white;
  }

  .tabs {
    display: flex;
    border-bottom: 2px solid var(--pico-muted-border-color);
    margin-bottom: 1.5rem;
  }
  .tabs button {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    margin-bottom: -2px;
    padding: 0.4rem 1rem;
    font-size: 0.9rem;
    font-weight: 500;
    color: var(--pico-muted-color);
    cursor: pointer;
    border-radius: 0;
    transition: color 0.15s, border-color 0.15s;
  }
  .tabs button:hover {
    color: var(--pico-primary);
  }
  .tabs button.active {
    color: var(--pico-primary);
    border-bottom-color: var(--pico-primary);
    font-weight: 600;
  }

  .calendar-list {
    list-style: none;
    padding: 0;
    margin: 0 0 0.75rem;
  }
  .calendar-list li { list-style: none; }
  .calendar-list label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
  }
  .calendar-list input[type="checkbox"] {
    cursor: pointer;
    margin: 0;
  }

  .cal-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .cal-action-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.3rem 0.6rem;
    font-size: 0.8rem;
    background: none;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    color: var(--pico-muted-color);
    cursor: pointer;
    line-height: 1.4;
    margin-bottom: 0;
    transition: color 0.15s, border-color 0.15s;
  }
  .cal-action-btn:hover {
    color: var(--pico-primary);
    border-color: var(--pico-primary);
  }

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

  .pgp-key-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
  }

  .pgp-key-info {
    display: flex;
    align-items: center;
    gap: 0.5rem;
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

  .pgp-key-actions {
    display: flex;
    gap: 0.4rem;
    flex-shrink: 0;
  }

  .pgp-key-actions button.small {
    padding: 0.2rem 0.5rem;
    font-size: 0.8rem;
    margin-bottom: 0;
  }

  .pgp-key-actions button.danger {
    border-color: var(--pico-del-color);
    color: var(--pico-del-color);
  }

  .pgp-key-actions button.danger:hover {
    background: var(--pico-del-color);
    color: white;
  }

  .pgp-global-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
  }

  .pgp-global-actions button {
    margin-bottom: 0;
  }
</style>

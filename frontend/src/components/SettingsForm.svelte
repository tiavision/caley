<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, setLocale, makeTl } from '../lib/i18n.ts';
  import { clearReturnUrl } from '../lib/return-url.ts';
  import { apiFetch } from '../lib/api.ts';
  import { getUser } from '../lib/user.ts';
  import EventColorDot from './EventColorDot.svelte';
  import CreateCalendarDialog from './CreateCalendarDialog.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import NavToggle from './NavToggle.svelte';
  import type { CalendarJson, CalDavAccountJson } from '../types/calendar.ts';
  import EncryptionSettings from './settings/EncryptionSettings.svelte';

  let locale = 'en';
  $: tl = makeTl(locale);

  let caldavBaseUrl = '';
  let caldavUsername = '';
  let caldavPassword = '';
  let caldavDisplayName = '';
  let editingAccountId = '';
  let hasPassword = false;
  let loading = true;
  let saving = false;
  let deleting = false;
  let errorMsg = '';


  // ── Tabs ───────────────────────────────────────────────────────────────────
  let tab: 'settings' | 'encryption' = 'settings';

  function switchTab(t: 'settings' | 'encryption') {
    tab = t;
  }

  // ── Calendars tab ─────────────────────────────────────────────────────────
  let calendars: CalendarJson[] = [];
  let caldavAccounts: CalDavAccountJson[] = [];
  let calendarsLoading = false;
  let createCalDialog: CreateCalendarDialog;
  let importDialog: ImportDialog;
  let accountDialog: HTMLDialogElement;

  async function fetchCalendars() {
    calendarsLoading = true;
    try {
      const res = await apiFetch('/api/calendars');
      if (res.ok) calendars = await res.json();
      const accountRes = await apiFetch('/api/caldav/accounts');
      if (accountRes.ok) caldavAccounts = (await accountRes.json()).accounts || [];
    } catch {}
    calendarsLoading = false;
  }

  async function toggleCalendar(accountId: string, href: string, active: boolean) {
    calendars = calendars.map(c => c.account_id === accountId && c.href === href ? { ...c, is_active: active } : c);
    caldavAccounts = caldavAccounts.map(acc => acc.account_id === accountId ? {
      ...acc,
      calendars: acc.calendars.map(c => c.href === href ? { ...c, is_active: active } : c),
    } : acc);
    await apiFetch('/api/calendar/toggle', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: `account_id=${encodeURIComponent(accountId)}&href=${encodeURIComponent(href)}&active=${active}`,
    });
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

    await fetchCalendars();
    loading = false;
  });

  function onLocaleChange() {
    setLocale(selectedLocale);
    locale = selectedLocale;
  }

  function openAddAccountDialog() {
    editingAccountId = '';
    caldavDisplayName = '';
    caldavBaseUrl = '';
    caldavUsername = '';
    caldavPassword = '';
    hasPassword = false;
    errorMsg = '';
    accountDialog?.showModal();
  }

  function openEditAccountDialog(account: CalDavAccountJson) {
    editingAccountId = account.account_id;
    caldavDisplayName = account.display_name || '';
    caldavBaseUrl = account.caldav_base_url || '';
    caldavUsername = account.caldav_username || '';
    caldavPassword = '';
    hasPassword = !!account.has_password;
    errorMsg = '';
    accountDialog?.showModal();
  }

  async function onSubmit() {
    saving = true;
    errorMsg = '';
    const params = new URLSearchParams();
    if (editingAccountId) params.append('account_id', editingAccountId);
    params.append('display_name', caldavDisplayName);
    params.append('caldav_base_url', caldavBaseUrl);
    params.append('caldav_username', caldavUsername);
    params.append('caldav_password', caldavPassword);

    try {
      const res = await apiFetch('/api/caldav/account', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      const data = await res.json();
      if (res.ok && data.status === 'ok') {
        accountDialog?.close();
        await fetchCalendars();
      } else {
        errorMsg = data.error || tl('event_error');
      }
    } catch {
      errorMsg = tl('event_error');
    }
    saving = false;
  }

  async function onDeleteCalDavAccount(account: CalDavAccountJson) {
    if (!confirm(`Remove account "${account.display_name}"?`)) return;
    try {
      const res = await apiFetch(`/api/caldav/account?account_id=${encodeURIComponent(account.account_id)}`, { method: 'DELETE' });
      if (res.ok) await fetchCalendars();
    } catch {}
  }

  async function onDeleteAccount() {
    if (!confirm(tl('settings_deleteAccountConfirm'))) return;
    deleting = true;
    try {
      const res = await apiFetch('/api/account', { method: 'DELETE' });
      if (res.ok) {
        const data = await res.json();
        clearReturnUrl();
        window.location.href = data.redirect || '/calendar/';
      }
    } catch {}
    deleting = false;
  }
</script>

<div class="settings" data-layout="center" data-gap="lg" data-pad="sm" style="--tm-measure: 40rem">
  <div data-layout="row" data-gap="sm">
    <NavToggle />
    <h2 data-grow style="--tm-grow-basis: 12rem">{tl('settings_heading')}</h2>
    <label data-layout="row" data-gap="sm">
      <span data-icon="globe" data-muted></span>
      <select
        data-size="sm"
        aria-label={tl('settings_language')}
        bind:value={selectedLocale}
        onchange={onLocaleChange}
      >
        {#each localeOptions as opt}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </label>
  </div>

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else}
    <div role="tablist" aria-label={tl('settings_heading')}>
      <button
        type="button"
        role="tab"
        id="settings-tab-general"
        aria-controls="settings-panel-general"
        aria-selected={tab === 'settings'}
        onclick={() => switchTab('settings')}
      >{tl('nav_settings')}</button>
      <button
        type="button"
        role="tab"
        id="settings-tab-encryption"
        aria-controls="settings-panel-encryption"
        aria-selected={tab === 'encryption'}
        onclick={() => switchTab('encryption')}
      >{tl('pgp_tabEncryption')}</button>
    </div>

    <div
      role="tabpanel"
      id="settings-panel-general"
      aria-labelledby="settings-tab-general"
      hidden={tab !== 'settings'}
      data-layout="stack"
      data-gap="lg"
    >
      <div data-layout="row" data-gap="sm">
        <button type="button" data-variant="outline" data-size="sm" data-icon="user-plus" onclick={openAddAccountDialog}>
          {tl('mail_addAccount')}
        </button>
        <button type="button" data-variant="outline" data-size="sm" data-icon="calendar-plus" onclick={() => createCalDialog.open()}>
          {tl('cal_createCalendar')}
        </button>
        <button type="button" data-variant="outline" data-size="sm" data-icon="upload" onclick={() => importDialog.open()}>
          {tl('cal_import')}
        </button>
      </div>

      {#if calendarsLoading}
        <p aria-busy="true">{tl('nav_loading')}</p>
      {:else if caldavAccounts.length === 0}
        <p data-muted>{tl('settings_subtitle')}</p>
      {:else}
        {#each caldavAccounts as account (account.account_id)}
          <article data-layout="stack" data-gap="sm">
            <header data-layout="row" data-gap="sm" data-justify="between">
              <div data-layout="stack" data-gap="none">
                <h3>{account.display_name}</h3>
                <small data-muted>{account.caldav_username}</small>
              </div>
              <div data-layout="row" data-gap="xs">
                <button type="button" data-variant="secondary" data-size="sm" onclick={() => openEditAccountDialog(account)}>{tl('contacts_edit')}</button>
                <button type="button" data-variant="outline" data-tone="danger" data-size="sm" onclick={() => onDeleteCalDavAccount(account)}>{tl('contacts_delete')}</button>
              </div>
            </header>
            <h4 data-caption>{tl('cal_calendars')}</h4>
            <div data-layout="stack" data-gap="xs">
              {#each account.calendars as cal (cal.account_id + cal.href)}
                <label>
                  <input
                    type="checkbox"
                    checked={cal.is_active}
                    onchange={(e) => toggleCalendar(cal.account_id, cal.href, (e.target as HTMLInputElement).checked)}
                  />
                  <EventColorDot color={cal.color} />
                  {cal.name}
                </label>
              {/each}
            </div>
            {#if account.addressbooks.length > 0}
              <h4 data-caption>{tl('contacts_addressbooks')}</h4>
              <ul data-layout="stack" data-gap="xs">
                {#each account.addressbooks as ab (ab.account_id + ab.href)}
                  <li>{ab.name}</li>
                {/each}
              </ul>
            {/if}
          </article>
        {/each}
      {/if}

      <hr />
      <button type="button" data-variant="outline" data-tone="danger" data-self="start" data-icon="trash-2" onclick={onDeleteAccount} disabled={deleting} aria-busy={deleting}>
        {tl('settings_deleteAccount')}
      </button>
    </div>

    <div
      role="tabpanel"
      id="settings-panel-encryption"
      aria-labelledby="settings-tab-encryption"
      hidden={tab !== 'encryption'}
    >
      {#if tab === 'encryption'}
        <EncryptionSettings {locale} />
      {/if}
    </div>
  {/if}
</div>

<dialog id="caldav-account-dialog" closedby="any" aria-labelledby="caldav-account-title" bind:this={accountDialog}>
  <header>
    <h2 id="caldav-account-title">{editingAccountId ? tl('mail_editAccountHeading') : tl('settings_addAccountHeading')}</h2>
    <button type="button" commandfor="caldav-account-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>
  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onSubmit(); }}>
    <label>
      {tl('settings_accountName')}
      <input type="text" bind:value={caldavDisplayName} />
    </label>
    <label>
      {tl('settings_baseUrl')}
      <input type="url" placeholder={tl('settings_baseUrlPlaceholder')} bind:value={caldavBaseUrl} required />
    </label>
    <label>
      {tl('settings_username')}
      <input type="text" autocomplete="username" placeholder={tl('settings_usernamePlaceholder')} bind:value={caldavUsername} required />
    </label>
    <label>
      {tl('settings_password')}
      <input type="password" autocomplete="current-password" placeholder={!hasPassword ? tl('settings_passwordPlaceholder') : tl('settings_passwordHint')} bind:value={caldavPassword} />
    </label>
    {#if errorMsg}
      <div data-callout data-tone="danger" role="alert">{errorMsg}</div>
    {/if}
    <footer>
      <button type="button" data-variant="secondary" commandfor="caldav-account-dialog" command="close">{tl('event_cancel')}</button>
      <button type="submit" disabled={saving} aria-busy={saving}>{tl('settings_submit')}</button>
    </footer>
  </form>
</dialog>

<CreateCalendarDialog
  bind:this={createCalDialog}
  {locale}
  accounts={caldavAccounts}
  oncreated={(cal) => { calendars = [...calendars, cal]; }}
/>
<ImportDialog
  bind:this={importDialog}
  {calendars}
  accounts={caldavAccounts}
  {locale}
  onimported={fetchCalendars}
/>

<style>
  /* temper's data-layout="center" keeps at least md side padding; match the sm page padding of the other
     pages so the nav toggle sits in the same place on phones. */
  .settings {
    max-inline-size: calc(var(--tm-measure) + 2 * var(--tm-space-sm));
    padding-inline: var(--tm-space-sm);
  }
</style>

<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, setLocale, makeTl } from '../lib/i18n.ts';
  import { clearReturnUrl } from '../lib/return-url.ts';
  import { apiFetch } from '../lib/api.ts';
  import { getUser } from '../lib/user.ts';
  import EventColorDot from './EventColorDot.svelte';
  import CreateCalendarDialog from './CreateCalendarDialog.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import UiIcon from './UiIcon.svelte';
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
        class:active={tab === 'encryption'}
        aria-selected={tab === 'encryption'}
        onclick={() => switchTab('encryption')}
      >{tl('pgp_tabEncryption')}</button>
    </div>
  {/if}

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else if tab === 'settings'}
    <div class="settings-panel">
      <div class="cal-actions">
        <button class="cal-action-btn" onclick={openAddAccountDialog}>
          {tl('mail_addAccount')}
        </button>
        <button class="cal-action-btn" onclick={() => createCalDialog.open()}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          {tl('cal_createCalendar')}
        </button>
        <button class="cal-action-btn" onclick={() => importDialog.open()}>
          {tl('cal_import')}
        </button>
      </div>
      {#if calendarsLoading}
        <p aria-busy="true">{tl('nav_loading')}</p>
      {:else if caldavAccounts.length === 0}
        <p class="settings-subtitle">{tl('settings_subtitle')}</p>
      {:else}
        <div class="caldav-account-list">
          {#each caldavAccounts as account (account.account_id)}
            <section class="caldav-account">
              <header class="caldav-account-header">
                <div>
                  <h3>{account.display_name}</h3>
                  <p>{account.caldav_username}</p>
                </div>
                <div class="caldav-account-actions">
                  <button class="secondary small" onclick={() => openEditAccountDialog(account)}>{tl('contacts_edit')}</button>
                  <button class="contrast outline small" onclick={() => onDeleteCalDavAccount(account)}>{tl('contacts_delete')}</button>
                </div>
              </header>
              <h4>{tl('cal_calendars')}</h4>
              <ul class="calendar-list">
                {#each account.calendars as cal (cal.account_id + cal.href)}
                  <li>
                    <label>
                      <input
                        type="checkbox"
                        checked={cal.is_active}
                        onchange={(e) => toggleCalendar(cal.account_id, cal.href, (e.target as HTMLInputElement).checked)}
                      />
                      <EventColorDot color={cal.color} />
                      {cal.name}
                    </label>
                  </li>
                {/each}
              </ul>
              {#if account.addressbooks.length > 0}
                <h4>{tl('contacts_addressbooks')}</h4>
                <ul class="calendar-list">
                  {#each account.addressbooks as ab (ab.account_id + ab.href)}
                    <li>{ab.name}</li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/each}
        </div>
      {/if}

      <hr />
      <button class="delete-account-btn" onclick={onDeleteAccount} disabled={deleting} aria-busy={deleting}>
        {tl('settings_deleteAccount')}
      </button>
    </div>
  {:else if tab === 'encryption'}
  <EncryptionSettings {locale} />
  {/if}
</div>

<dialog bind:this={accountDialog} onclick={(e) => { if (e.target === e.currentTarget) accountDialog?.close(); }}>
  <article>
    <header>
      <h3>{editingAccountId ? tl('mail_editAccountHeading') : tl('mail_addAccountHeading')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => accountDialog?.close()}></button>
    </header>
    <form class="settings-connection-form" onsubmit={(e) => { e.preventDefault(); onSubmit(); }}>
      <label>
        <span>{tl('settings_accountName')}</span>
        <input type="text" bind:value={caldavDisplayName} />
      </label>
      <label>
        <span>{tl('settings_baseUrl')}</span>
        <input type="url" placeholder={tl('settings_baseUrlPlaceholder')} bind:value={caldavBaseUrl} required />
      </label>
      <label>
        <span>{tl('settings_username')}</span>
        <input type="text" placeholder={tl('settings_usernamePlaceholder')} bind:value={caldavUsername} required />
      </label>
      <label>
        <span>{tl('settings_password')}</span>
        <input type="password" placeholder={!hasPassword ? tl('settings_passwordPlaceholder') : tl('settings_passwordHint')} bind:value={caldavPassword} />
      </label>
      {#if errorMsg}
        <p class="error-msg">{errorMsg}</p>
      {/if}
      <footer>
        <button type="submit" disabled={saving} aria-busy={saving}>{tl('settings_submit')}</button>
        <button type="button" class="secondary" onclick={() => accountDialog?.close()}>{tl('event_cancel')}</button>
      </footer>
    </form>
  </article>
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
  .settings-form {
    box-sizing: border-box;
    width: 100%;
    padding: 1rem;
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
    width: auto;
  }

  .delete-account-btn:hover {
    background-color: var(--pico-del-color);
    color: white;
  }

  .tabs {
    align-items: center;
    align-self: flex-start;
    background: var(--pico-muted-border-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: 0.5rem;
    display: flex;
    flex-wrap: wrap;
    gap: 0.15rem;
    margin-bottom: 1.5rem;
    margin-top: 1rem;
    max-width: 100%;
    padding: 0.15rem;
  }

  .tabs button {
    background: none;
    border: 1px solid transparent;
    border-radius: 0.35rem;
    box-shadow: none;
    color: var(--pico-muted-color);
    cursor: pointer;
    font-size: 0.9rem;
    font-weight: 500;
    line-height: 1.2;
    margin: 0;
    min-height: 2rem;
    padding: 0.4rem 0.75rem;
    transition: color 0.15s, background-color 0.15s, border-color 0.15s;
    white-space: nowrap;
    width: auto;
  }

  .tabs button:hover {
    background: var(--pico-card-background-color);
    color: var(--pico-primary);
  }

  .tabs button.active {
    background: var(--pico-card-background-color);
    border-color: var(--pico-card-border-color);
    color: var(--pico-primary);
    font-weight: 600;
  }

  .settings-panel {
    max-width: 32rem;
  }

  .settings-subtitle {
    color: var(--pico-muted-color);
    margin-bottom: 1rem;
  }

  .settings-connection-form {
    max-width: 30rem;
  }

  .settings-connection-form input {
    max-width: 30rem;
  }

  .settings-connection-form button[type="submit"] {
    width: auto;
  }

  .settings-panel hr {
    max-width: 30rem;
    margin: 1.25rem 0 0.75rem;
  }

  @media (max-width: 640px) {
    .settings-form {
      padding: 0.75rem;
    }

    .settings-header {
      align-items: flex-start;
    }

    .tabs {
      width: 100%;
    }

    .tabs button {
      flex: 1 1 auto;
      min-width: 8rem;
    }
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

</style>

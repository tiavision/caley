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
  // true when the user is not authenticated yet (login mode)
  let isLoginMode = false;

  // ── Tabs (only shown when authenticated) ──────────────────────────────────
  let tab: 'settings' | 'calendars' = 'settings';

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

  function switchTab(t: 'settings' | 'calendars') {
    tab = t;
    if (t === 'calendars' && calendars.length === 0) fetchCalendars();
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

    // Detect auth state: 401 (or network error) means we're not logged in → login mode
    const userRes = await getUser();
    if (!userRes.ok) {
      isLoginMode = true;
      caldavBaseUrl = localStorage.getItem('caldav_base_url') || '';
      loading = false;
      return;
    }

    // Authenticated: load existing settings to prefill the form
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

    const endpoint = isLoginMode ? '/auth/login' : '/api/settings';
    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      const data = await res.json();
      if (res.ok && data.status === 'ok') {
        if (isLoginMode) {
          localStorage.setItem('caldav_base_url', caldavBaseUrl);
        }
        window.location.href = '/';
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
        window.location.href = data.redirect || '/';
      }
    } catch {}
    deleting = false;
  }
</script>

<div class="settings-form">
  <div class="settings-header">
    {#if !isLoginMode}
      <a href="/" class="home-btn" aria-label={tl('nav_calendar')}>
        <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
        </svg>
      </a>
    {/if}
    <h2>{isLoginMode ? tl('login_heading') : tl('settings_heading')}</h2>
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

  {#if !isLoginMode && !loading}
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
    </div>
  {/if}

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else if isLoginMode || tab === 'settings'}
    <p>{isLoginMode ? tl('login_subtitle') : tl('settings_subtitle')}</p>
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
          placeholder={isLoginMode || !hasPassword ? tl('settings_passwordPlaceholder') : tl('settings_passwordHint')}
          bind:value={caldavPassword}
          required={isLoginMode}
        />
      </label>

      {#if errorMsg}
        <p class="error-msg">{errorMsg}</p>
      {/if}

      <button type="submit" disabled={saving} aria-busy={saving}>
        {isLoginMode ? tl('login_submit') : tl('settings_submit')}
      </button>
    </form>

    {#if !isLoginMode}
      <hr />
      <button class="delete-account-btn" onclick={onDeleteAccount} disabled={deleting} aria-busy={deleting}>
        {tl('settings_deleteAccount')}
      </button>
    {/if}
  {:else}
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
</style>

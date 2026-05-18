<script lang="ts">
  import { onMount } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import { initI18n, getLocale, setLocale } from '../lib/i18n.ts';

  let locale = 'en';
  $: tl = (() => { locale; return (key: string, params?: Record<string, string | number>) => { const fn = (m as Record<string, Function>)[key]; return typeof fn === 'function' ? fn(params) : key; }; })();

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

  const localeOptions = [
    { value: 'en', label: 'English' },
    { value: 'de', label: 'Deutsch' },
  ];
  let selectedLocale = 'en';

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    selectedLocale = locale.slice(0, 2);

    window.addEventListener('localeChange', (e) => {
      locale = (e as CustomEvent).detail?.locale?.slice(0, 2) || getLocale();
      selectedLocale = locale;
    });

    // Detect auth state: 401 means we're not logged in → login mode
    const userRes = await fetch('/api/user').catch(() => null);
    if (!userRes || userRes.status === 401) {
      isLoginMode = true;
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
    <h2>{isLoginMode ? tl('login_heading') : tl('settings_heading')}</h2>
    <select
      class="locale-picker"
      aria-label={tl('settings_language')}
      bind:value={selectedLocale}
      on:change={onLocaleChange}
    >
      {#each localeOptions as opt}
        <option value={opt.value}>{opt.label}</option>
      {/each}
    </select>
  </div>

  <p>{isLoginMode ? tl('login_subtitle') : tl('settings_subtitle')}</p>

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else}
    <form on:submit|preventDefault={onSubmit}>
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
      <button class="delete-account-btn" on:click={onDeleteAccount} disabled={deleting} aria-busy={deleting}>
        {tl('settings_deleteAccount')}
      </button>
    {/if}
  {/if}
</div>

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
  }

  .locale-picker {
    font-size: 0.875rem;
    padding: 0.125rem 1.5rem 0.125rem 0.5rem;
    margin: 0;
    background-color: var(--pico-form-element-background-color);
    color: var(--pico-form-element-color);
    border: var(--pico-border-width) solid var(--pico-form-element-border-color);
    border-radius: var(--pico-border-radius);
    font-family: inherit;
    cursor: pointer;
  }

  .error-msg {
    color: var(--pico-del-color);
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
</style>

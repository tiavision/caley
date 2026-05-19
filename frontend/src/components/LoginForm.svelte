<script lang="ts">
  import { onMount } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import { initI18n, getLocale, setLocale } from '../lib/i18n.ts';

  let locale = 'en';
  $: tl = (() => { locale; return (key: string, params?: Record<string, string | number>) => { const fn = (m as Record<string, Function>)[key]; return typeof fn === 'function' ? fn(params) : key; }; })();

  let caldavBaseUrl = '';
  let caldavUsername = '';
  let caldavPassword = '';
  let saving = false;
  let errorMsg = '';

  const localeOptions = [
    { value: 'en', label: 'English' },
    { value: 'de', label: 'Deutsch' },
  ];
  let selectedLocale = 'en';

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    selectedLocale = locale.slice(0, 2);
    caldavBaseUrl = localStorage.getItem('caldav_base_url') || '';

    window.addEventListener('localeChange', (e) => {
      locale = (e as CustomEvent).detail?.locale?.slice(0, 2) || getLocale();
      selectedLocale = locale;
    });
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
    try {
      const res = await fetch('/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      const data = await res.json();
      if (res.ok && data.status === 'ok') {
        localStorage.setItem('caldav_base_url', caldavBaseUrl);
        window.location.href = '/';
      } else {
        errorMsg = data.error || tl('login_error');
      }
    } catch {
      errorMsg = tl('login_error');
    }
    saving = false;
  }
</script>

<div class="login-form">
  <div class="login-header">
    <h2>{tl('login_heading')}</h2>
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

  <p>{tl('login_subtitle')}</p>

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
        placeholder={tl('settings_passwordPlaceholder')}
        bind:value={caldavPassword}
        required
      />
    </label>

    {#if errorMsg}
      <p class="error-msg">{errorMsg}</p>
    {/if}

    <button type="submit" disabled={saving} aria-busy={saving}>
      {tl('login_submit')}
    </button>
  </form>
</div>

<style>
  .login-form {
    max-width: 32rem;
    margin: 2rem auto;
  }

  .login-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }

</style>

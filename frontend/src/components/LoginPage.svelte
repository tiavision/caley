<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, setLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let caldavBaseUrl = '';
  let caldavUsername = '';
  let caldavPassword = '';
  let loading = true;
  let saving = false;
  let errorMsg = '';
  let oidcEnabled = false;

  const localeOptions = [
    { value: 'en', label: 'English' },
    { value: 'de', label: 'Deutsch' },
  ];
  let selectedLocale = 'en';

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    selectedLocale = locale.slice(0, 2);

    // Already signed in → straight to the app.
    const user = await getUser();
    if (user.ok) {
      window.location.href = '/calendar/';
      return;
    }

    caldavBaseUrl = localStorage.getItem('caldav_base_url') || '';

    try {
      const res = await fetch('/api/auth/config');
      if (res.ok) {
        const cfg = await res.json();
        oidcEnabled = !!cfg.oidc_enabled;
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
      const res = await fetch('/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      const data = await res.json();
      if (res.ok && data.status === 'ok') {
        localStorage.setItem('caldav_base_url', caldavBaseUrl);
        window.location.href = '/calendar/';
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

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else}
    <p>{tl('login_subtitle')}</p>
    <form onsubmit={(e) => { e.preventDefault(); onSubmit(); }}>
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
        <input type="password" placeholder={tl('settings_passwordPlaceholder')} bind:value={caldavPassword} required />
      </label>

      {#if errorMsg}
        <p class="error-msg">{errorMsg}</p>
      {/if}

      <button type="submit" disabled={saving} aria-busy={saving}>{tl('login_submit')}</button>
    </form>

    {#if oidcEnabled}
      <div class="oidc-divider"><span>{tl('login_or')}</span></div>
      <a class="oidc-btn" href="/auth/oidc" role="button">{tl('login_oidc_button')}</a>
    {/if}
  {/if}
</div>

<style>
  .login-form {
    max-width: 28rem;
    margin: 3rem auto;
  }

  .login-header {
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

  .error-msg {
    color: var(--pico-del-color);
  }

  .oidc-divider {
    display: flex;
    align-items: center;
    text-align: center;
    color: var(--pico-muted-color);
    margin: 1.25rem 0;
    gap: 0.75rem;
  }
  .oidc-divider::before,
  .oidc-divider::after {
    content: '';
    flex: 1;
    border-bottom: 1px solid var(--pico-muted-border-color);
  }

  .oidc-btn {
    display: block;
    width: 100%;
    text-align: center;
    --pico-background-color: transparent;
    --pico-color: var(--pico-primary);
    --pico-border-color: var(--pico-primary);
  }
</style>

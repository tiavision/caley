<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, setLocale, makeTl } from '../lib/i18n.ts';
  import { markAuthReturnPending, redirectToReturnUrl } from '../lib/return-url.ts';
  import { apiFetch } from '../lib/api.ts';
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
      redirectToReturnUrl();
      return;
    }

    caldavBaseUrl = localStorage.getItem('caldav_base_url') || '';

    try {
      const res = await apiFetch('/api/auth/config');
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
        redirectToReturnUrl();
      } else {
        errorMsg = data.error || tl('login_error');
      }
    } catch {
      errorMsg = tl('login_error');
    }
    saving = false;
  }
</script>

<div data-layout="stack" data-gap="lg">
  <div data-layout="row" data-gap="sm" data-wrap="none">
    <h2 data-grow>{tl('login_heading')}</h2>
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
  </div>

  {#if loading}
    <p aria-busy="true">{tl('nav_loading')}</p>
  {:else}
    <p data-muted>{tl('login_subtitle')}</p>
    <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); onSubmit(); }}>
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
        <input type="password" autocomplete="current-password" placeholder={tl('settings_passwordPlaceholder')} bind:value={caldavPassword} required />
      </label>

      {#if errorMsg}
        <p data-callout data-tone="danger" role="alert">{errorMsg}</p>
      {/if}

      <button type="submit" disabled={saving} aria-busy={saving}>{tl('login_submit')}</button>
    </form>

    {#if oidcEnabled}
      <p data-divider>{tl('login_or')}</p>
      <a href="/auth/oidc" role="button" data-variant="outline" onclick={markAuthReturnPending}>{tl('login_oidc_button')}</a>
    {/if}
  {/if}
</div>

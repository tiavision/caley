<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser, type UserInfo } from '../lib/user.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let authed = false;
  let user: UserInfo | null = null;
  let collapsed = false;
  let mobileOpen = false;

  const path = window.location.pathname;

  function isActive(prefix: string): boolean {
    return path.startsWith(prefix);
  }

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    collapsed = document.body.classList.contains('nav-collapsed');
    const u = await getUser();
    if (u.ok) {
      user = u;
      authed = true;
      document.body.classList.add('has-nav');
    }
  });

  function toggleCollapse() {
    collapsed = !collapsed;
    document.body.classList.toggle('nav-collapsed', collapsed);
    try {
      localStorage.setItem('nav-collapsed', collapsed ? '1' : '0');
    } catch {}
  }
</script>

{#if authed}
  <button class="nav-hamburger" onclick={() => (mobileOpen = true)} aria-label="Menu">
    <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="18" x2="21" y2="18"/>
    </svg>
  </button>

  {#if mobileOpen}
    <button type="button" class="nav-backdrop" aria-label="Close menu" onclick={() => (mobileOpen = false)}></button>
  {/if}

  <nav class="nav-rail" class:collapsed class:open={mobileOpen} aria-label="Primary">
    <div class="nav-brand-row">
      <a href="/" class="nav-brand">{tl('app_brand')}</a>
      <button class="nav-collapse-btn" onclick={toggleCollapse} aria-label="Toggle navigation labels">
        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <polyline points="11 17 6 12 11 7"/><polyline points="18 17 13 12 18 7"/>
        </svg>
      </button>
    </div>

    <div class="nav-items">
      <a href="/mail/" class="nav-item" class:active={isActive('/mail/')} title={tl('nav_mail')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/><polyline points="22,6 12,13 2,6"/>
          </svg>
        </span>
        <span class="label">{tl('nav_mail')}</span>
      </a>
      <a href="/calendar/" class="nav-item" class:active={isActive('/calendar/')} title={tl('nav_calendar')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
          </svg>
        </span>
        <span class="label">{tl('nav_calendar')}</span>
      </a>
      <a href="/contacts/" class="nav-item" class:active={isActive('/contacts/')} title={tl('nav_contacts')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>
          </svg>
        </span>
        <span class="label">{tl('nav_contacts')}</span>
      </a>
      <a href="/search/" class="nav-item" class:active={isActive('/search/')} title={tl('nav_search')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
        </span>
        <span class="label">{tl('nav_search')}</span>
      </a>
    </div>

    <div class="nav-bottom">
      {#if user?.display_name}
        <div class="nav-user" title={user.display_name}>{user.display_name}</div>
      {/if}
      <a href="/settings/" class="nav-item" class:active={isActive('/settings/')} title={user?.has_settings ? tl('nav_settings') : tl('nav_setup')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
          </svg>
        </span>
        <span class="label">{user?.has_settings ? tl('nav_settings') : tl('nav_setup')}</span>
      </a>
      <a href="/logout" class="nav-item" title={tl('nav_logout')}>
        <span class="nav-icon">
          <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" y1="12" x2="9" y2="12"/>
          </svg>
        </span>
        <span class="label">{tl('nav_logout')}</span>
      </a>
    </div>
  </nav>
{/if}

<style>
  .nav-rail {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: var(--nav-width, 15rem);
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 0.25rem;
    padding: 0.75rem 0.5rem;
    border-right: 1px solid var(--pico-muted-border-color, #ddd);
    background: var(--pico-background-color, #fff);
    overflow-x: hidden;
    overflow-y: auto;
    z-index: 100;
    transition: width 0.15s ease, transform 0.2s ease;
  }

  .nav-brand-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0.5rem 0.5rem;
  }
  .nav-brand {
    flex: 1;
    font-size: 1.25rem;
    font-weight: 700;
    /* Reset Pico's link --pico-color so the brand matches the username text. */
    --pico-color: inherit;
    color: var(--pico-color, #333);
    opacity: 0.8;
    text-decoration: none;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .nav-collapse-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.25rem;
    border: none;
    background: none;
    color: var(--pico-muted-color, #666);
    cursor: pointer;
    flex-shrink: 0;
  }
  .nav-collapse-btn svg { transition: transform 0.2s ease; }
  .nav-collapse-btn:hover { color: var(--pico-color, #333); }
  .nav-rail.collapsed .nav-collapse-btn svg { transform: rotate(180deg); }

  .nav-items {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    flex: 1;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.6rem;
    border-radius: var(--pico-border-radius, 0.25rem);
    color: var(--pico-color, #333);
    text-decoration: none;
    white-space: nowrap;
  }
  .nav-item:hover { background: var(--pico-muted-border-color, #e0e0e0); }
  .nav-item.active {
    background: var(--pico-primary-background, #e6f2fb);
    color: var(--pico-h3-color);
  }
  .nav-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .nav-item .label {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .nav-bottom {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--pico-muted-border-color, #ddd);
  }
  .nav-user {
    padding: 0.25rem 0.6rem;
    font-weight: 600;
    opacity: 0.8;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Collapsed (icon-only) — desktop */
  .nav-rail.collapsed .label,
  .nav-rail.collapsed .nav-user,
  .nav-rail.collapsed .nav-brand {
    display: none;
  }
  .nav-rail.collapsed .nav-item { justify-content: center; padding: 0.5rem; }
  .nav-rail.collapsed .nav-brand-row { justify-content: center; padding: 0.25rem; }

  /* Mobile drawer + hamburger */
  .nav-hamburger {
    display: none;
    position: fixed;
    top: 0.5rem;
    left: 0.5rem;
    z-index: 101;
    align-items: center;
    justify-content: center;
    padding: 0.4rem;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: var(--pico-border-radius, 0.25rem);
    background: var(--pico-background-color, #fff);
    color: var(--pico-color, #333);
    cursor: pointer;
  }
  .nav-backdrop {
    position: fixed;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.4);
    z-index: 99;
    cursor: pointer;
  }

  @media (min-width: 768px) {
    .nav-hamburger,
    .nav-backdrop { display: none; }
  }

  @media (max-width: 767px) {
    .nav-hamburger { display: flex; }
    .nav-collapse-btn { display: none; }
    .nav-rail {
      width: 15rem;
      transform: translateX(-100%);
      box-shadow: 0 0 1.5rem rgba(0, 0, 0, 0.25);
    }
    .nav-rail.open { transform: translateX(0); }
    /* Always show labels in the mobile drawer, even if collapsed on desktop */
    .nav-rail.collapsed .label,
    .nav-rail.collapsed .nav-user,
    .nav-rail.collapsed .nav-brand { display: revert; }
    .nav-rail.collapsed .nav-item { justify-content: flex-start; padding: 0.5rem 0.6rem; }
    .nav-rail.collapsed .nav-brand-row { justify-content: flex-start; padding: 0.25rem 0.5rem 0.5rem; }
  }
</style>

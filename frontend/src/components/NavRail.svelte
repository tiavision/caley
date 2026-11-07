<script lang="ts">
  // Navigation rail for pages on temper (app.css): a direct child of <body data-shell>, a drawer
  // (popover) below 48rem. Pages still on Pico use NavSidebar.svelte.
  import { onMount, onDestroy } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser, type UserInfo } from '../lib/user.ts';
  import {
    type NewMailNotification,
    mailNotificationsEnabled,
    mailNotificationsStorageKey,
    mailNotificationsSupported,
    showNewMailNotification,
  } from '../lib/mail-notifications.ts';

  const COLLAPSED_KEY = 'nav-collapsed';

  let locale = $state('en');
  const tl = $derived(makeTl(locale));

  let user: UserInfo | null = $state(null);
  let collapsed = $state(readCollapsed());
  let notificationSupported = $state(false);
  let notificationPermission: NotificationPermission = $state('default');
  let notificationEnabled = $state(false);
  let notificationSource: EventSource | null = null;
  let notificationReconnect: ReturnType<typeof setTimeout> | null = null;
  const path = window.location.pathname;

  function readCollapsed(): boolean {
    try { return localStorage.getItem(COLLAPSED_KEY) === '1'; } catch { return false; }
  }

  function current(prefix: string): 'page' | null {
    return path.startsWith(prefix) ? 'page' : null;
  }

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    const u = await getUser();
    if (u.ok) {
      user = u;
      initializeNotifications();
    }
  });

  onDestroy(() => {
    notificationSource?.close();
    if (notificationReconnect) clearTimeout(notificationReconnect);
  });

  function onCollapseChange() {
    try { localStorage.setItem(COLLAPSED_KEY, collapsed ? '1' : '0'); } catch {}
  }

  function initializeNotifications() {
    notificationSupported = mailNotificationsSupported();
    if (!notificationSupported) return;
    notificationPermission = Notification.permission;
    notificationEnabled = mailNotificationsEnabled();
    if (notificationEnabled && !path.startsWith('/mail/')) connectNotificationEvents();
  }

  async function toggleNotifications() {
    if (!notificationSupported || notificationPermission === 'denied') return;
    if (notificationEnabled) {
      notificationEnabled = false;
      localStorage.removeItem(mailNotificationsStorageKey);
      notificationSource?.close();
      notificationSource = null;
      return;
    }

    notificationPermission = Notification.permission === 'granted'
      ? 'granted'
      : await Notification.requestPermission();
    if (notificationPermission !== 'granted') return;
    notificationEnabled = true;
    localStorage.setItem(mailNotificationsStorageKey, '1');
    if (!path.startsWith('/mail/')) connectNotificationEvents();
  }

  function connectNotificationEvents() {
    if (!notificationEnabled || notificationSource) return;
    notificationSource = new EventSource('/api/events/stream');
    notificationSource.addEventListener('new_email', (event) => {
      showNewMailNotification(JSON.parse(event.data) as NewMailNotification, tl);
    });
    notificationSource.onerror = () => {
      notificationSource?.close();
      notificationSource = null;
      if (notificationEnabled) {
        notificationReconnect = setTimeout(connectNotificationEvents, 5000);
      }
    };
  }
</script>

{#if user}
  <nav data-rail popover id="nav" aria-label={tl('nav_label')}>
    <header>
      <a href="/" class="brand" data-rail-label>
        <img src="/icons/favicon-48x48.png" alt="" />
        <span>{tl('app_brand')}</span>
      </a>
      <input
        type="checkbox"
        data-rail-collapse
        aria-label={tl('nav_collapse')}
        bind:checked={collapsed}
        onchange={onCollapseChange}
      />
    </header>
    <a href="/mail/" data-icon="mail" aria-current={current('/mail/')} data-tooltip={tl('nav_mail')}><span>{tl('nav_mail')}</span></a>
    <a href="/calendar/" data-icon="calendar" aria-current={current('/calendar/')} data-tooltip={tl('nav_calendar')}><span>{tl('nav_calendar')}</span></a>
    <a href="/contacts/" data-icon="users" aria-current={current('/contacts/')} data-tooltip={tl('nav_contacts')}><span>{tl('nav_contacts')}</span></a>
    <a href="/search/" data-icon="search" aria-current={current('/search/')} data-tooltip={tl('nav_search')}><span>{tl('nav_search')}</span></a>
    <footer>
      {#if user.display_name}
        <span data-caption data-rail-label data-truncate>{user.display_name}</span>
      {/if}
      {#if notificationSupported}
        <button
          type="button"
          data-icon="bell"
          onclick={toggleNotifications}
          disabled={notificationPermission === 'denied'}
          aria-pressed={notificationEnabled}
          data-tooltip={notificationPermission === 'denied' ? tl('mail_notificationsBlocked') : tl('mail_notifications')}
        ><span>{tl('mail_notifications')}</span></button>
      {/if}
      <a href="/settings/" data-icon="settings" aria-current={current('/settings/')} data-tooltip={tl('nav_settings')}><span>{tl('nav_settings')}</span></a>
      <a href="/logout" data-icon="log-out" data-tooltip={tl('nav_logout')}><span>{tl('nav_logout')}</span></a>
    </footer>
  </nav>
{/if}

<style>
  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--tm-space-sm);
    font-size: var(--tm-text-lg);
    font-weight: 700;
    color: var(--tm-text);
    text-decoration: none;
  }
  .brand img {
    inline-size: 1.5rem;
    block-size: 1.5rem;
  }
</style>

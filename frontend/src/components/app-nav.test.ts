import { expect, test } from 'vitest';
import { render, mockRequests, mockStream, memoryStorage } from '@tiaforge-ui/sinter/testing';
import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../test-messages.ts';
import { app } from '../test-components.ts';
import type { NotificationStatus } from '../actions/notify.ts';
import '../test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

// A page of the app layout (src/layouts/app-layout.html), which has the rail.
const page = (here: string) => `
<app-layout data-sn-with="{ here: '${here}' }">
  <main><nav-toggle></nav-toggle></main>
</app-layout>`;

// <email-notifier>'s script listens to server events: here it only keeps the live instances.
const live = new Set<object>();
const scripts = { 'email-notifier': class { connected() { live.add(this); } disconnected() { live.delete(this); } } };

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };

function notify(initial: NotificationStatus) {
  let current = initial;
  return {
    status: () => current,
    toggle: async (s: NotificationStatus) => (current = { ...s, permission: 'granted', enabled: !s.enabled }),
  };
}

async function show(here: string, options: { status?: NotificationStatus; storage?: Storage; userResponse?: unknown } = {}) {
  const requests = mockRequests({ 'GET /api/user': options.userResponse ?? user });
  const actions = { notify: notify(options.status ?? { supported: true, permission: 'default', enabled: false }) };
  const r = await render(page(here), {
    ...app, scripts, requests, actions, messages, storage: options.storage,
    streams: { '/api/events/stream': mockStream() },
  });
  await r.settle();
  return r;
}

test('shows the rail for the signed-in user, with the current page marked', async () => {
  const { root } = await show('/contacts/');
  const rail = root.querySelector('nav[data-tm-rail]')!;
  expect(rail.id).toBe('nav');
  expect(rail.getAttribute('aria-label')).toBe('Main navigation');
  expect([...rail.querySelectorAll('[aria-current="page"]')].map((a) => a.getAttribute('href'))).toEqual(['/contacts/']);
  expect(rail.querySelector('a[href="/mail/"]')!.textContent).toBe('Mail');
  expect(rail.querySelector('a[href="/mail/"]')!.getAttribute('data-tm-tooltip')).toBe('Mail');
  expect(rail.querySelector('footer [data-tm-caption]')!.textContent).toBe('Demo User');
  expect(rail.querySelector('a[href="/logout"]')!.textContent).toBe('Logout');
  expect(root.querySelector('main button[popovertarget="nav"]')!.getAttribute('aria-label')).toBe('Open menu');
});

test('the rail does not wait for the user: its links are in the markup, only the name fills in later', async () => {
  // Signed-out visitors never get here (the server redirects them to /), so the rail is part of the page's first paint.
  const { root } = await show('/calendar/', { userResponse: { status: 401 } });
  const rail = root.querySelector('nav[data-tm-rail]')!;
  expect(rail.querySelector('a[href="/calendar/"]')!.getAttribute('aria-current')).toBe('page');
  expect(rail.querySelector('footer [data-tm-caption]')).toBeNull();
});

test('remembers the collapsed rail across page loads', async () => {
  const storage = memoryStorage();
  const first = await show('/calendar/', { storage });
  const box = first.root.querySelector<HTMLInputElement>('[data-tm-rail-collapse]')!;
  expect(box.checked).toBe(false);
  box.checked = true;
  box.dispatchEvent(new Event('change', { bubbles: true }));
  await first.flush();

  const second = await show('/mail/', { storage });
  expect(second.root.querySelector<HTMLInputElement>('[data-tm-rail-collapse]')!.checked).toBe(true);
});

test('the bell turns notifications on; the notifier runs except on the mail page', async () => {
  const { root, settle } = await show('/search/');
  const bell = root.querySelector<HTMLButtonElement>('button[data-tm-icon="bell"]')!;
  expect(bell.getAttribute('aria-pressed')).toBe('false');
  expect(live.size).toBe(0);

  bell.click();
  await settle();
  expect(bell.getAttribute('aria-pressed')).toBe('true');
  expect(live.size).toBe(1);

  const mail = await show('/mail/', { status: { supported: true, permission: 'granted', enabled: true } });
  expect(mail.root.querySelector('button[data-tm-icon="bell"]')!.getAttribute('aria-pressed')).toBe('true');
  expect(live.size).toBe(0);
});

test('blocked or unsupported notifications', async () => {
  const blocked = await show('/calendar/', { status: { supported: true, permission: 'denied', enabled: false } });
  const bell = blocked.root.querySelector<HTMLButtonElement>('button[data-tm-icon="bell"]')!;
  expect(bell.disabled).toBe(true);
  expect(bell.getAttribute('data-tm-tooltip')).toBe('Mail notifications blocked');

  const none = await show('/calendar/', { status: { supported: false, permission: 'default', enabled: false } });
  expect(none.root.querySelector('button[data-tm-icon="bell"]')).toBeNull();
});

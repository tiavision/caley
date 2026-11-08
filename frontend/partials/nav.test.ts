import { expect, test } from 'vitest';
import { render, mockRequests, memoryStorage } from '@tiaforge-ui/sinter/testing';
import * as m from '../src/paraglide/messages/_index.js';
import nav from './nav.html?raw';
import navToggle from './nav-toggle.html?raw';
import type { NotificationStatus } from '../src/actions/notify.ts';

const page = (here: string) => `
<body data-shell="fixed" data-sn-state="{ here: '${here}' }">
  <sn-include src="./nav.html"></sn-include>
  <main><sn-include src="./nav-toggle.html"></sn-include></main>
</body>`;

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
    requests, actions, helpers: { m }, storage: options.storage, files: { './nav.html': nav, './nav-toggle.html': navToggle },
  });
  await r.settle();
  return r;
}

test('shows the rail for the signed-in user, with the current page marked', async () => {
  const { root } = await show('/contacts/');
  const rail = root.querySelector('nav[data-rail]')!;
  expect(rail.id).toBe('nav');
  expect(rail.getAttribute('aria-label')).toBe('Main navigation');
  expect([...rail.querySelectorAll('[aria-current="page"]')].map((a) => a.getAttribute('href'))).toEqual(['/contacts/']);
  expect(rail.querySelector('a[href="/mail/"]')!.textContent).toBe('Mail');
  expect(rail.querySelector('a[href="/mail/"]')!.getAttribute('data-tooltip')).toBe('Mail');
  expect(rail.querySelector('footer [data-caption]')!.textContent).toBe('Demo User');
  expect(rail.querySelector('a[href="/logout"]')!.textContent).toBe('Logout');
  expect(root.querySelector('main button[popovertarget="nav"]')!.getAttribute('aria-label')).toBe('Open menu');
});

test('shows no rail when nobody is signed in', async () => {
  const { root } = await show('/calendar/', { userResponse: { status: 401 } });
  expect(root.querySelector('nav')).toBeNull();
});

test('remembers the collapsed rail across page loads', async () => {
  const storage = memoryStorage();
  const first = await show('/calendar/', { storage });
  const box = first.root.querySelector<HTMLInputElement>('[data-rail-collapse]')!;
  expect(box.checked).toBe(false);
  box.checked = true;
  box.dispatchEvent(new Event('change', { bubbles: true }));
  await first.flush();

  const second = await show('/mail/', { storage });
  expect(second.root.querySelector<HTMLInputElement>('[data-rail-collapse]')!.checked).toBe(true);
});

test('the bell turns notifications on; the notifier runs except on the mail page', async () => {
  const { root, settle } = await show('/search/');
  const bell = root.querySelector<HTMLButtonElement>('button[data-icon="bell"]')!;
  expect(bell.getAttribute('aria-pressed')).toBe('false');
  expect(root.querySelector('mail-notifier')).toBeNull();

  bell.click();
  await settle();
  expect(bell.getAttribute('aria-pressed')).toBe('true');
  expect(root.querySelector('mail-notifier')).not.toBeNull();

  const mail = await show('/mail/', { status: { supported: true, permission: 'granted', enabled: true } });
  expect(mail.root.querySelector('button[data-icon="bell"]')!.getAttribute('aria-pressed')).toBe('true');
  expect(mail.root.querySelector('mail-notifier')).toBeNull();
});

test('blocked or unsupported notifications', async () => {
  const blocked = await show('/calendar/', { status: { supported: true, permission: 'denied', enabled: false } });
  const bell = blocked.root.querySelector<HTMLButtonElement>('button[data-icon="bell"]')!;
  expect(bell.disabled).toBe(true);
  expect(bell.getAttribute('data-tooltip')).toBe('Mail notifications blocked');

  const none = await show('/calendar/', { status: { supported: false, permission: 'default', enabled: false } });
  expect(none.root.querySelector('button[data-icon="bell"]')).toBeNull();
});

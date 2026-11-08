import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, mockRequests, mockStream, memoryStorage, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import { calendarValue, importEventDate, importDraft, importSummary } from '../src/lib/import.ts';
import { caldavDraft, caldavBody, withCalendarActive, keyImportError, enrollErrorText } from '../src/lib/settings.ts';
import { newCalendarDraft, newCalendarBody } from '../src/lib/events.ts';
import * as m from '../src/paraglide/messages/_index.js';
import * as ics from '../src/actions/ics.ts';
import '../src/islands/common.ts';                 // <close-after>
import nav from '../partials/nav.html?raw';
import navToggle from '../partials/nav-toggle.html?raw';
import createCalendar from '../partials/create-calendar-dialog.html?raw';
import importDialog from '../partials/import-dialog.html?raw';
import pgpSettings from '../partials/pgp-settings.html?raw';
import source from './index.html?raw';
import type { VaultView } from '../src/actions/pgp-vault.ts';

// The test page lives at the root of sinter's virtual project: its partials sit next to it.
const page = source.replaceAll('../partials/', './');
const files = {
  './nav.html': nav, './nav-toggle.html': navToggle, './create-calendar-dialog.html': createCalendar,
  './import-dialog.html': importDialog, './pgp-settings.html': pgpSettings,
};

// The helpers the page and its partials use (not the whole helpers module: other pages' helpers load more).
const helpers = { m, calendarValue, importEventDate, importDraft, importSummary, newCalendarDraft, newCalendarBody, caldavDraft, caldavBody, withCalendarActive, keyImportError, enrollErrorText };

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };
const cal = (href: string, name: string, extra: object = {}) => ({ account_id: 'acc', href, name, color: '#ff0000', is_active: true, ...extra });
const account = (extra: object = {}) => ({
  account_id: 'acc', display_name: 'Home', caldav_base_url: 'https://dav.example/', caldav_username: 'ada', has_password: true, sort_order: 0,
  calendars: [cal('/cal/work/', 'Work'), cal('/cal/private/', 'Private', { color: null, is_active: false })],
  addressbooks: [{ account_id: 'acc', href: '/ab/', name: 'Contacts' }],
  ...extra,
});

// jsdom has no modal dialogs: open/close set `open` and fire `toggle` as browsers do.
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}
// jsdom has no command invokers either: a click on a [commandfor] button runs its command.
document.addEventListener('click', (e) => {
  const button = (e.target as Element).closest?.('button[commandfor]');
  if (!button) return;
  const dialog = document.getElementById(button.getAttribute('commandfor')!) as HTMLDialogElement;
  queueMicrotask(() => (button.getAttribute('command') === 'show-modal' ? dialog.showModal() : dialog.close()));
});

let confirmAnswer = true;
// localStorage of each render: `sn:event-default-calendar` is the default calendar shared with the event form.
let storage = memoryStorage();
beforeEach(() => { confirmAnswer = true; storage = memoryStorage(); configure({ confirm: () => confirmAnswer }); });
afterEach(() => configure({ confirm: null as never }));

const lockedVault: VaultView = {
  exists: true, unlocked: false, hasCredentials: true, pairingSas: '',
  keys: [{ fingerprint: 'AAAA1111', identities: ['Ada <ada@example.com>'], exportable: true }],
  active: [{ cred_id: 'c1', label: 'Laptop', status: 'active', local: true, sas: '' }],
  pending: [{ cred_id: 'c2', label: 'Phone', status: 'pending', local: false, sas: '123 456' }],
};

/** A stand-in for src/actions/pgp-vault.ts: records calls, answers with `next` views. */
function fakeVault(initial: VaultView | null, failUnlock = false) {
  const calls: Array<[string, unknown[]]> = [];
  let current: VaultView = initial ?? { exists: false, unlocked: false, hasCredentials: false, keys: [], active: [], pending: [], pairingSas: '' };
  const step = (name: string, change: (v: VaultView, ...args: never[]) => VaultView) => async (...args: unknown[]) => {
    calls.push([name, args.slice(0, -1)]);
    current = change(current, ...(args as never[]));
    return current;
  };
  return {
    calls,
    ns: {
      status: step('status', (v) => v),
      unlock: failUnlock ? async () => { throw new Error('NotAllowedError'); } : step('unlock', (v) => ({ ...v, unlocked: true })),
      lock: step('lock', (v) => ({ ...v, unlocked: false })),
      rotate: step('rotate', (v) => v),
      reset: step('reset', () => ({ exists: false, unlocked: false, hasCredentials: false, keys: [], active: [], pending: [], pairingSas: '' })),
      readKey: async (armored: string, passphrase: string) => {
        calls.push(['readKey', [armored, passphrase]]);
        if (passphrase !== 'right') throw new Error('Incorrect key passphrase');
        return { fingerprint: 'BBBB2222' };
      },
      forgetKey: () => { calls.push(['forgetKey', []]); },
      store: step('store', (v) => ({
        ...v, exists: true, unlocked: true,
        keys: [...v.keys, { fingerprint: 'BBBB2222', identities: [], exportable: true }],
        active: v.active.length ? v.active : [{ cred_id: 'c9', label: 'New', status: 'active', local: true, sas: '' }],
      })),
      addIdentity: step('addIdentity', (v) => v),
      updatePublic: step('updatePublic', (v) => v),
      exportPublic: async (...args: unknown[]) => { calls.push(['exportPublic', args.slice(0, -1)]); },
      register: step('register', (v) => v),
      approve: step('approve', (v) => ({ ...v, active: [...v.active, { ...v.pending[0]!, status: 'active' }], pending: [] })),
      removeKey: step('removeKey', (v) => v),
      removeDevice: step('removeDevice', (v) => v),
    },
  };
}

async function show(routes: Record<string, RouteResponse> = {}, vault: VaultView | null = lockedVault, failUnlock = false) {
  const requests = mockRequests({
    'GET /api/user': user,
    'GET /api/caldav/accounts': { accounts: [account()] },
    'GET /api/pgp/keys': [
      { fingerprint: 'AAAA1111', email: 'ada@example.com', armored: '', is_own: true, source: 'imported', trust: 'trusted' },
      { fingerprint: 'CCCC3333DDDD4444EEEE', email: 'bob@example.com', armored: '', is_own: false, source: 'harvested_autocrypt', trust: 'trusted' },
    ],
    ...routes,
  });
  const stream = mockStream();
  const pgp = fakeVault(vault, failUnlock);
  const actions = {
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    ics,
    pgpVault: pgp.ns,
    pgpKeys: { importPublic: async (armored: string) => ({ fingerprint: 'FFFF', email: armored, armored, is_own: false, source: 'imported', trust: 'trusted' }) },
  };
  const r = await render(page, { requests, actions, helpers, files, storage, streams: { '/api/events/stream': stream } });
  await r.settle();
  return { ...r, requests, stream, pgp };
}

function type(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  el.value = value;
  el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

/** Text fields of a multipart body (the mock keeps it as text where jsdom cannot parse it). */
function multipart(body: unknown): Record<string, string> {
  if (body instanceof FormData) return Object.fromEntries([...body].map(([k, v]) => [k, String(v)]));
  return Object.fromEntries([...String(body).matchAll(/name="([^"]+)"\r\n\r\n([^\r]*)\r\n/g)].map((x) => [x[1]!, x[2]!]));
}

const button = (root: HTMLElement, text: string) =>
  [...root.querySelectorAll('button')].find((b) => b.textContent === text || b.getAttribute('aria-label') === text)!;

test('lists the CalDAV accounts with their calendars and address books', async () => {
  const { root } = await show();
  const article = root.querySelector('main article')!;
  expect(article.querySelector('h3')!.textContent).toBe('Home');
  expect(article.querySelector('small')!.textContent).toBe('ada');
  const labels = [...article.querySelectorAll('label')];
  expect(labels.map((l) => l.textContent!.trim())).toEqual(['Work', 'Private']);
  expect(labels.map((l) => l.querySelector('input')!.hasAttribute('checked'))).toEqual([true, false]);
  const dots = labels.map((l) => l.querySelector<HTMLElement>('.event-color-dot')!);
  expect(dots[0]!.hidden).toBe(false);
  expect(dots[0]!.style.getPropertyValue('--tm-tone')).toBe('#ff0000');
  expect(dots[1]!.hidden).toBe(true);
  expect(article.querySelector('li')!.textContent).toBe('Contacts');
  expect(root.querySelector('[role=tab][aria-selected=true]')!.textContent).toBe('Settings');
});

test('no accounts: the hint', async () => {
  const { root } = await show({ 'GET /api/caldav/accounts': { accounts: [] } });
  expect(root.querySelector('main article')).toBeNull();
  expect(root.querySelector('#settings-panel-general p[data-muted]')!.textContent).toBe(m.settings_subtitle());
});

test('toggles a calendar', async () => {
  const { root, requests, settle } = await show({ 'POST /api/calendar/toggle': null });
  const box = root.querySelector<HTMLInputElement>('main article label input')!;
  box.checked = false;
  box.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  const call = requests.calls.at(-1)!;
  expect(call.path).toBe('/api/calendar/toggle');
  expect(String(call.body)).toBe('account_id=acc&href=%2Fcal%2Fwork%2F&active=false');
  expect(root.querySelector<HTMLInputElement>('main article label input')!.checked).toBe(false);
});

test('a failed toggle puts the calendar back', async () => {
  const { root, settle } = await show({ 'POST /api/calendar/toggle': { status: 500 } });
  const box = root.querySelector<HTMLInputElement>('main article label input')!;
  box.checked = false;
  box.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  expect(root.querySelector<HTMLInputElement>('main article label input')!.checked).toBe(true);
});

test('adds a CalDAV account; the server message shows on failure', async () => {
  let fail = true;
  const { root, requests, settle } = await show({
    'POST /api/caldav/account': () => (fail ? { status: 400, body: { error: 'Connection test failed: 401' } } : { status: 'ok', account_id: 'acc2' }),
  });
  button(root, m.mail_addAccount()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#caldav-account-dialog')!;
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('h2')!.textContent).toBe(m.settings_addAccountHeading());
  const [name, url, username, password] = dialog.querySelectorAll('input');
  type(name!, 'Work');
  type(url!, 'https://dav.work/');
  type(username!, 'bob');
  type(password!, 'pw');
  expect(password!.placeholder).toBe(m.settings_passwordPlaceholder());
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.filter((c) => c.method === 'POST').at(-1)!.body)).toBe('display_name=Work&caldav_base_url=https%3A%2F%2Fdav.work%2F&caldav_username=bob&caldav_password=pw');
  expect(dialog.querySelector('[role=alert]')!.textContent).toBe('Connection test failed: 401');
  expect(dialog.open).toBe(true);

  fail = false;
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(dialog.open).toBe(false);
  expect(requests.calls.filter((c) => c.path === '/api/caldav/accounts')).toHaveLength(2);
});

test('edits an account: prefilled, keeps the password when left empty', async () => {
  const { root, requests, settle } = await show({ 'POST /api/caldav/account': { status: 'ok', account_id: 'acc' } });
  button(root, m.contacts_edit()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#caldav-account-dialog')!;
  expect(dialog.querySelector('h2')!.textContent).toBe(m.mail_editAccountHeading());
  const inputs = [...dialog.querySelectorAll('input')];
  expect(inputs.map((i) => i.value)).toEqual(['Home', 'https://dav.example/', 'ada', '']);
  expect(inputs[3]!.placeholder).toBe(m.settings_passwordHint());
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.find((c) => c.method === 'POST')!.body)).toBe('account_id=acc&display_name=Home&caldav_base_url=https%3A%2F%2Fdav.example%2F&caldav_username=ada&caldav_password=');
});

test('removes an account after confirmation', async () => {
  const { root, requests, settle } = await show({ 'DELETE /api/caldav/account': { status: 'ok' } });
  confirmAnswer = false;
  button(root, m.contacts_delete()).click();
  await settle();
  expect(requests.calls.some((c) => c.method === 'DELETE')).toBe(false);
  confirmAnswer = true;
  button(root, m.contacts_delete()).click();
  await settle();
  expect(requests.calls.find((c) => c.method === 'DELETE')!.url).toBe('/api/caldav/account?account_id=acc');
  expect(root.querySelector('main article')).toBeNull();
});

test('a failed account removal brings the account back', async () => {
  const { root, settle } = await show({ 'DELETE /api/caldav/account': { status: 500 } });
  confirmAnswer = true;
  button(root, m.contacts_delete()).click();
  await settle();
  expect(root.querySelector('main article h3')!.textContent).toBe('Home');
});

test('creates a calendar, with a color when chosen', async () => {
  const { root, requests, settle } = await show({ 'POST /api/calendar': cal('/cal/new/', 'Trips') });
  button(root, m.cal_createCalendar()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#create-calendar-dialog')!;
  expect(dialog.open).toBe(true);
  type(dialog.querySelector<HTMLInputElement>('input[type=text]')!, 'Trips');
  expect(dialog.querySelector('input[type=color]')).toBeNull();
  const useColor = dialog.querySelector<HTMLInputElement>('input[type=checkbox]')!;
  useColor.checked = true;
  useColor.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  type(dialog.querySelector<HTMLInputElement>('input[type=color]')!, '#00ff00');
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.find((c) => c.path === '/api/calendar')!.body)).toBe('name=Trips&account_id=acc&color=%2300ff00');
  expect(dialog.open).toBe(false);

  // opens empty again
  button(root, m.cal_createCalendar()).click();
  await settle();
  expect(dialog.querySelector<HTMLInputElement>('input[type=text]')!.value).toBe('');
  expect(dialog.querySelector('input[type=color]')).toBeNull();
});

test('imports an ICS file into the remembered calendar, after a preview', async () => {
  storage.setItem('sn:event-default-calendar', JSON.stringify({ defaultCal: 'acc\0/cal/private/' }));
  const { root, requests, settle } = await show({
    'GET /api/caldav/accounts': { accounts: [account({ calendars: [cal('/cal/work/', 'Work'), cal('/cal/private/', 'Private')] })] },
    'POST /api/import/preview': [{ uid: 'e1', summary: 'Standup', dtstart: '2026-11-09T09:00', dtend: '2026-11-09T09:15', is_all_day: false, location: 'Room 1' }],
    'POST /api/import': { imported: 1, errors: 0 },
  });
  button(root, m.cal_import()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector<HTMLSelectElement>('select')!.value).toBe('acc\0/cal/private/');   // the event form's default
  type(dialog.querySelector('select')!, 'acc\0/cal/work/');

  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  expect(input.required).toBe(true);
  Object.defineProperty(input, 'files', { value: [new File(['BEGIN:VCALENDAR'], 'standup.ics')], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  expect(dialog.querySelector('small[data-truncate]')!.textContent).toBe('standup.ics');

  button(root, m.cal_importPreview()).click();
  await settle();
  const row = dialog.querySelector('ul[data-list] li')!;
  expect([...row.children].filter((c) => c.localName !== 'template').map((c) => c.textContent)).toEqual(['Standup', '2026-11-09 09:00 - 2026-11-09 09:15', 'Room 1']);

  dialog.querySelector('form')!.requestSubmit();
  await settle();
  const body = multipart(requests.calls.find((c) => c.path === '/api/import')!.body);
  expect([body.account_id, body.calendar_href]).toEqual(['acc', '/cal/work/']);
  expect(dialog.querySelector('[role=status]')!.textContent).toBe(m.cal_importSuccess({ n: 1 }));
  expect(JSON.parse(storage.getItem('sn:event-default-calendar')!)).toEqual({ defaultCal: 'acc\0/cal/work/' });
  await new Promise((resolve) => setTimeout(resolve, 1600));
  expect(dialog.open).toBe(false);
});

test('imports into a new calendar', async () => {
  const { root, requests, settle } = await show({
    'POST /api/calendar': cal('/cal/new/', 'Trips'),
    'POST /api/import': { imported: 2, errors: 1 },
  });
  button(root, m.cal_import()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  Object.defineProperty(input, 'files', { value: [new File(['x'], 'trips.ics')], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  type(dialog.querySelector('select')!, '__new__');
  await settle();
  type(dialog.querySelector<HTMLInputElement>('input[type=text]')!, 'Trips');
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(String(requests.calls.find((c) => c.path === '/api/calendar')!.body)).toBe('account_id=acc&name=Trips');
  expect(multipart(requests.calls.find((c) => c.path === '/api/import')!.body).calendar_href).toBe('/cal/new/');
  expect(JSON.parse(storage.getItem('sn:event-default-calendar') ?? '{}').defaultCal ?? '').toBe('');   // not remembered after errors
  expect(dialog.querySelector('[role=status]')!.textContent).toBe(`${m.cal_importSuccess({ n: 2 })} · ${m.cal_importPartial({ n: 1 })}`);
  expect(dialog.open).toBe(true);
});

test('reopening the import dialog shows the new draft: empty form, remembered calendar, no old messages', async () => {
  storage.setItem('sn:event-default-calendar', JSON.stringify({ defaultCal: 'acc\0/cal/private/' }));
  const { root, settle } = await show({
    'GET /api/caldav/accounts': { accounts: [account({ calendars: [cal('/cal/work/', 'Work'), cal('/cal/private/', 'Private')] })] },
    'POST /api/import/preview': { status: 400, body: 'No events found in ICS file' },
  });
  const dialog = root.querySelector<HTMLDialogElement>('#import-ics')!;
  const select = () => dialog.querySelector<HTMLSelectElement>('select')!;
  button(root, m.cal_import()).click();
  await settle();
  expect(select().value).toBe('acc\0/cal/private/');
  const input = dialog.querySelector<HTMLInputElement>('input[type=file]')!;
  Object.defineProperty(input, 'files', { value: [new File(['x'], 'empty.ics')], configurable: true });
  input.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  button(root, m.cal_importPreview()).click();
  await settle();
  expect(dialog.querySelector('[role=alert]')!.textContent).toBe('No events found in ICS file');
  type(select(), '__new__');
  await settle();
  type(dialog.querySelector<HTMLInputElement>('input[type=text]')!, 'Half typed');

  let resets = 0;
  dialog.querySelector('form')!.addEventListener('reset', () => resets++);
  button(root, m.event_cancel()).click();                     // closing resets the form: that empties the file input
  await settle();
  expect(dialog.open).toBe(false);
  expect(resets).toBe(1);
  delete (input as { files?: unknown }).files;               // what the reset did to the real input

  button(root, m.cal_import()).click();
  await settle();
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('input[type=file]')).toBe(input);   // the same form, updated in place
  expect(select().value).toBe('acc\0/cal/private/');           // the new draft's calendar, though reset() had moved the select
  expect(input.required).toBe(true);
  expect(dialog.querySelector('small[data-truncate]')).toBeNull();
  expect(dialog.querySelector('[role=alert]')).toBeNull();
  expect(dialog.querySelector('input[type=text]')).toBeNull();
  expect(dialog.querySelector('ul[data-list]')).toBeNull();
});

test('deleting the whole account hands the redirect to the island', async () => {
  const { root, settle } = await show({ 'DELETE /api/account': { redirect: '/' } });
  const seen: unknown[] = [];
  root.addEventListener('account-deleted', (e) => seen.push((e as CustomEvent).detail));
  button(root, m.settings_deleteAccount()).click();
  await settle();
  expect(seen).toEqual([{ redirect: '/' }]);
});

test('server events refetch the accounts', async () => {
  const { requests, stream, settle } = await show();
  stream.emit('calendars_changed', 1);
  await settle();
  expect(requests.calls.filter((c) => c.path === '/api/caldav/accounts')).toHaveLength(2);
});

async function encryptionTab(vault: VaultView | null = lockedVault, routes: Record<string, RouteResponse> = {}, failUnlock = false) {
  const r = await show(routes, vault, failUnlock);
  expect(r.pgp.calls).toEqual([]);                       // nothing loads before the tab opens
  button(r.root, m.pgp_tabEncryption()).click();
  await r.settle();
  return r;
}

test('encryption tab: a locked vault, unlocked with this browser\'s passkey, then locked again', async () => {
  const { root, settle, pgp } = await encryptionTab();
  const panel = root.querySelector<HTMLElement>('#settings-panel-encryption')!;
  expect(panel.hidden).toBe(false);
  expect(panel.querySelector('[data-badge]')!.textContent).toBe(m.pgp_vaultLocked());
  expect(panel.querySelector('code')!.textContent).toBe('AAAA1111');
  expect([...panel.querySelectorAll('li span[data-grow]')].map((s) => s.textContent!.replace(/\s+/g, ''))).toEqual(['Laptop' + m.pgp_thisBrowser().replace(/\s+/g, ''), 'Phone']);
  expect(button(root, m.pgp_approveDevice())).toBeUndefined();         // only while unlocked
  expect(button(root, m.pgp_importUnlockFirst()).disabled).toBe(true);

  button(root, m.pgp_unlockVaultButton()).click();
  await settle();
  expect(pgp.calls.at(-1)).toEqual(['unlock', ['c1']]);
  expect(panel.querySelector('[data-badge]')!.textContent).toBe(m.pgp_vaultUnlocked());
  expect(button(root, m.pgp_importPrivateKeyButton()).disabled).toBe(false);

  button(root, m.pgp_lockVault()).click();
  await settle();
  expect(panel.querySelector('[data-badge]')!.textContent).toBe(m.pgp_vaultLocked());
});

test('encryption tab: approving a pending device asks with its pairing code', async () => {
  const { root, settle, pgp } = await encryptionTab({ ...lockedVault, unlocked: true });
  const questions: string[] = [];
  configure({ confirm: (q: string) => { questions.push(q); return true; } });
  button(root, m.pgp_approveDevice()).click();
  await settle();
  expect(questions[0]).toContain('123 456');
  expect(pgp.calls.at(-1)).toEqual(['approve', ['c2']]);
  expect(button(root, m.pgp_approveDevice())).toBeUndefined();
});

test('encryption tab: an unlock failure shows the error', async () => {
  const { root, settle } = await encryptionTab(lockedVault, {}, true);
  button(root, m.pgp_unlockVaultButton()).click();
  await settle();
  expect(root.querySelector('#settings-panel-encryption [role=alert]')!.textContent).toBe(`${m.pgp_unlockError()} NotAllowedError`);
});

test('encryption tab, no vault: import a private key and create the vault with a passkey', async () => {
  const { root, settle, pgp } = await encryptionTab(null);
  const panel = root.querySelector<HTMLElement>('#settings-panel-encryption')!;
  expect(panel.querySelector('p')!.textContent).toBe(m.pgp_setupHint());
  button(root, m.pgp_importPrivateKeyButton()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#pgp-private-key-dialog')!;
  expect(dialog.open).toBe(true);
  const submit = dialog.querySelector<HTMLButtonElement>('button[type=submit]')!;
  expect(submit.disabled).toBe(true);
  type(dialog.querySelector('textarea')!, '-----BEGIN PGP PRIVATE KEY BLOCK-----');
  type(dialog.querySelector('input[type=password]')!, 'wrong');
  await settle();
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(dialog.querySelector('[role=alert]')!.textContent).toBe(`${m.pgp_importError()}: Incorrect key passphrase`);

  type(dialog.querySelector('input[type=password]')!, 'right');
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(dialog.querySelector('code')!.textContent).toBe('BBBB2222');
  expect(dialog.querySelector('[role=alert]')).toBeNull();
  type(dialog.querySelector('input[type=text]')!, 'Laptop');
  expect(dialog.querySelector('button[type=submit]')!.textContent).toBe(m.pgp_createVaultButton());
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(pgp.calls.find((c) => c[0] === 'store')).toEqual(['store', ['Laptop']]);
  expect(dialog.open).toBe(false);
  expect(pgp.calls.at(-1)).toEqual(['forgetKey', []]);
  expect(panel.querySelector('[data-badge]')!.textContent).toBe(m.pgp_vaultUnlocked());
  expect(panel.querySelector('small[data-tone=success]')!.textContent).toBe(m.pgp_enrolled());
});

test('encryption tab: trusted sender keys, import and remove', async () => {
  const { root, settle, requests } = await encryptionTab(lockedVault, { 'DELETE /api/pgp/key': null });
  const trusted = () => [...root.querySelectorAll('#settings-panel-encryption li span[data-truncate]')].map((s) => s.textContent);
  expect(trusted()).toEqual(['bob@example.com']);           // own keys are not listed here
  expect(root.querySelector('#settings-panel-encryption small code')!.textContent).toBe('CCCC3333DDDD4444…');

  button(root, m.pgp_importPublicKeyButton()).click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#pgp-public-key-dialog')!;
  type(dialog.querySelector('textarea')!, 'carol@example.com');
  await settle();
  dialog.querySelector('form')!.requestSubmit();
  await settle();
  expect(dialog.open).toBe(false);
  expect(trusted()).toEqual(['bob@example.com', 'carol@example.com']);
  expect(root.querySelector('#settings-panel-encryption [role=status]')!.textContent).toBe(m.pgp_publicKeyImported());

  const remove = root.querySelectorAll<HTMLButtonElement>('#settings-panel-encryption li button[data-tone=danger]');
  remove[remove.length - 2]!.click();
  await settle();
  expect(requests.calls.at(-1)!.body).toEqual({ fingerprint: 'CCCC3333DDDD4444EEEE' });
  expect(trusted()).toEqual(['carol@example.com']);
});

test('encryption tab: a failed key removal brings the key back with the error', async () => {
  const { root, settle } = await encryptionTab(lockedVault, { 'DELETE /api/pgp/key': { status: 500 } });
  const trusted = () => [...root.querySelectorAll('#settings-panel-encryption li span[data-truncate]')].map((s) => s.textContent);
  const remove = root.querySelectorAll<HTMLButtonElement>('#settings-panel-encryption li button[data-tone=danger]');
  remove[remove.length - 1]!.click();
  await settle();
  expect(trusted()).toEqual(['bob@example.com']);
  expect(root.querySelector('#settings-panel-encryption')!.textContent).toContain(m.pgp_publicKeyRemoveError());
});

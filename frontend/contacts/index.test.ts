import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, mockRequests, mockStream, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import '../src/islands/common.ts';                 // data-shortcut keys
import * as m from '../src/paraglide/messages/_index.js';
import nav from '../partials/nav.html?raw';
import navToggle from '../partials/nav-toggle.html?raw';
import shareMenu from '../partials/share-menu.html?raw';
import source from './index.html?raw';

// The test page lives at the root of sinter's virtual project: its partials sit next to it.
const page = source.replaceAll('../partials/', './');

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: 'https://maps.example/?q={q}' };
const book = (href: string, name: string) => ({ account_id: 'acc', href, name });
const contact = (uid: string, fn_name: string, extra: object = {}) => ({
  account_id: 'acc', uid, addressbook_href: '/ab/personal/', fn_name, emails: [], phones: [], addresses: [], org: '', has_photo: false, ...extra,
});
const ada = contact('1', 'Ada Lovelace', {
  emails: [{ value: 'ada@example.com', type_label: 'work' }], phones: [{ value: '+44 1', type_label: 'mobile' }], org: 'Engines Ltd',
  addresses: [{ street: '1 Main St', city: 'London', region: '', postal: 'N1', country: 'UK', type_label: 'home' }],
});
const alan = contact('2', 'Alan Turing', { has_photo: true });
const bob = contact('3', 'Bob Babbage', { addressbook_href: '/ab/work/' });

// jsdom has no modal dialogs: open/close set `open` and fire `toggle` as browsers do.
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}

let confirmAnswer = true;
// As src/sinter.config.ts: geo: links for addresses.
beforeEach(() => { confirmAnswer = true; configure({ confirm: () => confirmAnswer, urlSchemes: ['geo:'] }); });
afterEach(() => configure({ confirm: null as never, urlSchemes: [] }));

async function show(routes: Record<string, RouteResponse> = {}, books = [book('/ab/personal/', 'Personal')]) {
  const calls: Array<[string, unknown[]]> = [];
  const requests = mockRequests({
    'GET /api/user': user,
    'GET /api/contacts': { addressbooks: books, contacts: [ada, alan, bob] },
    ...routes,
  });
  const stream = mockStream();
  const actions = {
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    share: { native: () => false, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async (...args: unknown[]) => { calls.push(['maps.open', args.slice(0, 2)]); } },
  };
  const r = await render(page, {
    requests, actions, helpers, streams: { '/api/events/stream': stream },
    files: { './nav.html': nav, './nav-toggle.html': navToggle, './share-menu.html': shareMenu },
  });
  await r.settle();
  return { ...r, requests, stream, calls };
}

const names = (root: HTMLElement) => [...root.querySelectorAll('main article strong')].map((s) => s.textContent);
const letters = (root: HTMLElement) => [...root.querySelectorAll('main section h2')].map((h) => h.textContent);

function type(input: HTMLInputElement | HTMLSelectElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event(input instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

async function opened(dialog: HTMLDialogElement, flush: () => Promise<void>) {
  if (!dialog.open) dialog.showModal();   // what commandfor/command="show-modal" does in a browser
  await flush();
  return dialog;
}

async function openDetail(r: Awaited<ReturnType<typeof show>>, name: string) {
  const button = [...r.root.querySelectorAll<HTMLButtonElement>('main article button')].find((b) => b.textContent === name)!;
  button.click();
  return opened(r.root.querySelector<HTMLDialogElement>('#contact-detail')!, r.flush);
}

test('lists the contacts by letter, with avatar, first email and phone, org', async () => {
  const { root } = await show();
  expect(letters(root)).toEqual(['A', 'B']);
  expect(names(root)).toEqual(['Ada Lovelace', 'Alan Turing', 'Bob Babbage']);
  const [first, second] = root.querySelectorAll('main article');
  expect(first!.querySelector('[data-avatar]')!.textContent!.trim()).toBe('A');
  expect(first!.querySelector('a[href="mailto:ada@example.com"]')).not.toBeNull();
  expect(first!.querySelector('a[href="tel:+44 1"]')).not.toBeNull();
  expect(first!.querySelector('small[data-muted]')!.textContent).toBe('Engines Ltd');
  expect(second!.querySelector('[data-avatar] img')!.getAttribute('src')).toBe('/api/contact/photo?account_id=acc&uid=2&addressbook_href=%2Fab%2Fpersonal%2F');
  expect(root.querySelector('button[popovertarget="ab-menu"]')).toBeNull();
});

test('filters by name, email, org or phone', async () => {
  const { root, flush } = await show();
  const search = root.querySelector<HTMLInputElement>('main input[type=search]')!;
  type(search, 'engines');
  await flush();
  expect(names(root)).toEqual(['Ada Lovelace']);
  type(search, 'zzz');
  await flush();
  expect(names(root)).toEqual([]);
  expect(root.querySelector('main p[data-muted]')!.textContent).toBe('No contacts.');
});

test('with several address books: the menu filters them', async () => {
  const { root, flush } = await show({}, [book('/ab/personal/', 'Personal'), book('/ab/work/', 'Work')]);
  const button = root.querySelector('button[popovertarget="ab-menu"]')!;
  expect(button.textContent).toBe('Address books');
  expect([...root.querySelectorAll('main article')].at(-1)!.querySelector('small[data-muted]')!.textContent).toBe('Work');
  const work = root.querySelectorAll<HTMLInputElement>('#ab-menu input')[1]!;
  expect(work.checked).toBe(true);
  work.checked = false;
  work.dispatchEvent(new Event('change', { bubbles: true }));
  await flush();
  expect(names(root)).toEqual(['Ada Lovelace', 'Alan Turing']);
  expect(button.textContent).toBe('Address books (1/2)');
});

test('the detail dialog shows the contact; an address opens the map', async () => {
  const r = await show();
  const dialog = await openDetail(r, 'Ada Lovelace');
  expect(dialog.querySelector('h2')!.textContent).toBe('Ada Lovelace');
  const values = [...dialog.querySelectorAll('dd')].map((d) => d.textContent!.replace(/\s+/g, ' ').trim());
  expect(values).toEqual(['Engines Ltd', 'Work ada@example.com', 'Mobile +44 1', 'Home 1 Main St London N1 UK']);
  expect([...dialog.querySelectorAll('dd a[rel] br')].map((br) => br.hidden)).toEqual([false, false, true]);
  const map = dialog.querySelector<HTMLAnchorElement>('dd a[rel]')!;
  expect(map.getAttribute('href')).toBe('geo:0,0?q=1%20Main%20St%2C%20London%2C%20N1%2C%20UK');
  map.click();
  await r.settle();
  expect(r.calls).toEqual([['maps.open', ['1 Main St, London, N1, UK', 'https://maps.example/?q={q}']]]);
  expect(dialog.querySelector('button[popovertarget="share-menu"]')).not.toBeNull();
});

test('creates a contact: empty rows are left out, then the new contact is shown', async () => {
  const r = await show({ 'POST /api/contact': (call) => ({ ...contact('9', 'contacts-Zed'), ...(call.body as object), uid: '9', has_photo: false }) });
  r.root.querySelector<HTMLButtonElement>('button[data-fab]')!.click();
  const form = await opened(r.root.querySelector<HTMLDialogElement>('#contact-form')!, r.flush);
  expect(form.querySelector('h2')!.textContent).toBe('New contact');
  type(form.querySelector<HTMLInputElement>('input[required]')!, ' contacts-Zed ');
  type(form.querySelector<HTMLInputElement>('input[type=email]')!, 'zed@example.com');
  [...form.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Add address')!.click();
  await r.flush();
  type(form.querySelectorAll<HTMLInputElement>('fieldset:nth-of-type(3) input')[1]!, 'Paris');
  form.querySelector('form')!.requestSubmit();
  await r.settle();
  const post = r.requests.calls.find((c) => c.method === 'POST')!;
  expect(post.body).toEqual({
    account_id: 'acc', addressbook_href: '/ab/personal/', fn_name: 'contacts-Zed', org: '',
    emails: [{ value: 'zed@example.com', type_label: 'other' }], phones: [],
    addresses: [{ street: '', city: 'Paris', region: '', postal: '', country: '', type_label: 'home' }],
  });
  expect(names(r.root)).toContain('contacts-Zed');
  expect(form.open).toBe(false);
  expect(r.root.querySelector('#contact-detail h2')!.textContent).toBe('contacts-Zed');
});

test('edits from the detail dialog: rows can be added and removed, an error is shown', async () => {
  let fail = true;
  const r = await show({ 'POST /api/contact': (call) => (fail ? { status: 502 } : { ...ada, ...(call.body as object) }) });
  const detail = await openDetail(r, 'Ada Lovelace');
  detail.querySelector<HTMLButtonElement>('button[data-icon=pencil]')!.click();
  await r.flush();
  const form = await opened(r.root.querySelector<HTMLDialogElement>('#contact-form')!, r.flush);
  expect(form.querySelector('h2')!.textContent).toBe('Edit');
  expect(form.querySelector<HTMLInputElement>('input[required]')!.value).toBe('Ada Lovelace');
  expect(form.querySelector<HTMLSelectElement>('fieldset select')!.value).toBe('work');
  form.querySelector<HTMLButtonElement>('fieldset button[data-icon=x]')!.click();   // drop the email
  await r.flush();
  expect(form.querySelectorAll('input[type=email]')).toHaveLength(0);
  form.querySelector('form')!.requestSubmit();
  await r.settle();
  expect(form.querySelector('[role=alert]')!.textContent).toBe(m.contacts_error());
  fail = false;
  form.querySelector('form')!.requestSubmit();
  await r.settle();
  expect((r.requests.calls.at(-1)!.body as { uid: string; emails: unknown[] })).toMatchObject({ uid: '1', emails: [] });
  expect(form.open).toBe(false);
});

test('deletes after confirmation', async () => {
  const r = await show({ 'DELETE /api/contact': { status: 204 } });
  confirmAnswer = false;
  let dialog = await openDetail(r, 'Alan Turing');
  const del = () => dialog.querySelector<HTMLButtonElement>('footer button[data-tone=danger]')!;
  del().click();
  await r.settle();
  expect(r.requests.calls.some((c) => c.method === 'DELETE')).toBe(false);
  confirmAnswer = true;
  del().click();
  await r.settle();
  expect(r.requests.calls.find((c) => c.method === 'DELETE')!.body).toEqual({ account_id: 'acc', uid: '2', addressbook_href: '/ab/personal/' });
  expect(names(r.root)).toEqual(['Ada Lovelace', 'Bob Babbage']);
  expect(dialog.open).toBe(false);
});

test('server events update, remove and reload contacts; the open dialog follows', async () => {
  const r = await show();
  const dialog = await openDetail(r, 'Ada Lovelace');
  r.stream.emit('contact_updated', { account_id: 'acc', uid: '1', addressbook_href: '/ab/personal/', contact: { ...ada, fn_name: 'Ada King' } });
  r.stream.emit('contact_removed', { account_id: 'acc', uid: '3', addressbook_href: '/ab/work/' });
  await r.flush();
  expect(names(r.root)).toEqual(['Ada King', 'Alan Turing']);
  expect(dialog.querySelector('h2')!.textContent).toBe('Ada King');

  const before = r.requests.calls.filter((c) => c.path === '/api/contacts').length;
  r.stream.emit('contacts_refetch', 1);
  r.stream.emit('contact_updated', { account_id: 'acc', uid: '7', addressbook_href: '/ab/personal/' });
  await r.settle();
  expect(r.requests.calls.filter((c) => c.path === '/api/contacts').length).toBeGreaterThan(before);
  expect(names(r.root)).toEqual(['Ada Lovelace', 'Alan Turing', 'Bob Babbage']);
});

test('the detail dialog closes when another client removes its contact', async () => {
  const r = await show();
  const dialog = await openDetail(r, 'Ada Lovelace');
  r.stream.emit('contact_removed', { account_id: 'acc', uid: '2', addressbook_href: '/ab/personal/' });   // another contact
  await r.flush();
  expect(dialog.open).toBe(true);
  r.stream.emit('contact_removed', { account_id: 'acc', uid: '1', addressbook_href: '/ab/personal/' });
  await r.flush();
  expect(dialog.open).toBe(false);
  expect(names(r.root)).toEqual(['Bob Babbage']);
});

test('keyboard: c opens a new contact, e edits the open one', async () => {
  const r = await show();
  const form = r.root.querySelector<HTMLDialogElement>('#contact-form')!;
  const fab = r.root.querySelector<HTMLButtonElement>('button[data-fab]')!;
  let clicked = 0;
  fab.addEventListener('click', () => clicked++);
  document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', bubbles: true }));
  expect(clicked).toBe(1);                  // the shortcut clicks the button (src/islands/common.ts)
  await opened(form, r.flush);
  expect(form.querySelector('h2')!.textContent).toBe('New contact');
  form.close();
  document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', ctrlKey: true, bubbles: true }));
  expect(clicked).toBe(1);                  // other modifiers: not the shortcut

  const detail = await openDetail(r, 'Ada Lovelace');
  detail.querySelector('header')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'e', bubbles: true }));
  await opened(form, r.flush);
  expect(detail.open).toBe(false);
  expect(form.querySelector<HTMLInputElement>('input[required]')!.value).toBe('Ada Lovelace');
});

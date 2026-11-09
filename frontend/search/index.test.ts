import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { messages } from '../src/test-messages.ts';
import { render, mockRequests, mockStream, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import nav from '../partials/nav.html?raw';
import navToggle from '../partials/nav-toggle.html?raw';
import shareMenu from '../partials/share-menu.html?raw';
import eventDetail from '../partials/event-detail.html?raw';
import eventForm from '../partials/event-form.html?raw';
import source from './index.html?raw';

// The test page lives at the root of sinter's virtual project: its partials sit next to it.
const page = source.replaceAll('../partials/', './');

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };
const cal = (href: string, name: string, is_active = true) => ({ account_id: 'acc', href: `/cal/${href}/`, name, color: '#3a3', is_active });
const calendars = [cal('work', 'Work'), cal('home', 'Home'), cal('old', 'Old', false)];
const event = (uid: string, summary: string, href: string, dtstart: string, extra: object = {}) => ({
  account_id: 'acc', uid, time_range: '', summary, color: '#3a3', dtstart, dtend: '', is_all_day: true, tz: '', location: '',
  description: '', is_recurring: false, recurrence: null, calendar_href: `/cal/${href}/`, master_dtstart: '', alarms: [], attendees: [], ...extra,
});
const standup = event('s1', 'Standup', 'work', '2026-11-09', {
  is_recurring: true, recurrence: { freq: 'WEEKLY', interval: 1, by_day: ['MO'] }, location: 'Room 1', description: 'Daily',
  attendees: [{ cn: 'Ada', email: 'ada@example.com', partstat: 'ACCEPTED' }],
});
const dinner = event('d1', 'Dinner', 'home', '2026-11-12');

// jsdom has no modal dialogs: open/close set `open` and fire `toggle` as browsers do.
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}
if (!HTMLElement.prototype.hidePopover) HTMLElement.prototype.hidePopover = function () {};

beforeEach(() => configure({ confirm: () => true, urlSchemes: ['geo:'] }));
afterEach(() => configure({ confirm: null as never, urlSchemes: [] }));

async function show(routes: Record<string, RouteResponse> = {}, url?: string) {
  const requests = mockRequests({
    'GET /api/user': user,
    'GET /api/calendars': calendars,
    'GET /api/caldav/accounts': { accounts: [{ account_id: 'acc', display_name: 'Demo' }] },
    'GET /api/search': (call) => {
      const cals = new URL(call.url, 'http://x').searchParams.getAll('cal');
      return [standup, dinner].filter((e) => !cals.length || cals.includes(`acc\0${e.calendar_href}`));
    },
    'GET /api/contacts': { addressbooks: [], contacts: [] },
    ...routes,
  });
  const actions = {
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    share: { native: () => false, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async () => {} },
  };
  const r = await render(page, {
    messages, requests, actions, helpers, streams: { '/api/events/stream': mockStream() }, url,
    files: { './nav.html': nav, './nav-toggle.html': navToggle, './share-menu.html': shareMenu, './event-detail.html': eventDetail, './event-form.html': eventForm },
  });
  await r.settle();
  return { ...r, requests };
}

function type(input: HTMLInputElement | HTMLSelectElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event(input instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
}

async function search(root: HTMLElement, settle: () => Promise<void>, q: string) {
  type(root.querySelector<HTMLInputElement>('main input[type=search]')!, q);
  root.querySelector<HTMLFormElement>('main form')!.requestSubmit();
  await settle();
}

const titles = (root: HTMLElement) => [...root.querySelectorAll('.search-result strong')].map((s) => s.textContent);

test('searches the active calendars in one request, newest first', async () => {
  const { root, settle, requests, url } = await show();
  expect(root.querySelector('button[popovertarget=search-calendars]')!.textContent).toBe('Calendars (2/3)');
  await search(root, settle, ' din ');
  const urls = requests.calls.filter((c) => c.path === '/api/search').map((c) => c.url);
  expect(urls).toEqual(['/api/search?q=din&active=1']); // the server knows the active calendars
  expect(new URL(url()).search).toBe('?q=din');
  expect(titles(root)).toEqual(['Dinner', 'Standup']);
  expect(root.querySelector('.search-result small[data-caption]')!.textContent).toBe('Home');
});

test('the calendar menu and dates narrow the search, at once when one is shown', async () => {
  const { root, settle, flush, requests, url } = await show();
  const boxes = root.querySelectorAll<HTMLInputElement>('#search-calendars input[type=checkbox]');
  expect([...boxes].map((b) => b.checked)).toEqual([true, true, false]);
  boxes[1]!.click();
  await flush();
  const from = root.querySelectorAll<HTMLInputElement>('main input[type=date]')[0]!;
  from.value = '2026-11-10';
  from.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  expect(requests.calls.filter((c) => c.path === '/api/search')).toHaveLength(0); // no words yet: no search
  await search(root, settle, 'dinner');
  expect(requests.calls.at(-1)!.url).toBe('/api/search?q=dinner&start=2026-11-10&cal=acc%00%2Fcal%2Fwork%2F');
  expect(titles(root)).toEqual(['Standup']);
  boxes[1]!.click(); // back to the active calendars: the search runs again
  await settle();
  expect(requests.calls.at(-1)!.url).toBe('/api/search?q=dinner&start=2026-11-10&active=1');
  expect(Object.fromEntries(new URL(url()).searchParams)).toEqual({ q: 'dinner', start: '2026-11-10' });
});

test('a reload shows the search in the URL again', async () => {
  const { root, requests } = await show({}, '/search/?q=din&end=2026-11-30&cals=all');
  expect(requests.calls.filter((c) => c.path === '/api/search').map((c) => c.url)).toEqual(['/api/search?q=din&end=2026-11-30']);
  expect(root.querySelector<HTMLInputElement>('main input[type=search]')!.value).toBe('din');
  expect(root.querySelectorAll<HTMLInputElement>('main input[type=date]')[1]!.value).toBe('2026-11-30');
  expect([...root.querySelectorAll<HTMLInputElement>('#search-calendars input[type=checkbox]')].map((b) => b.checked)).toEqual([false, false, false]);
  expect(titles(root)).toEqual(['Dinner', 'Standup']);
});

test('a new search keeps the last results, dimmed, with a bar, until its own arrive', async () => {
  const { root, settle, flush } = await show();
  await search(root, settle, 'din');
  type(root.querySelector<HTMLInputElement>('main input[type=search]')!, 'standup');
  root.querySelector<HTMLFormElement>('main form')!.requestSubmit();
  await flush();
  expect(titles(root)).toEqual(['Dinner', 'Standup']);
  expect(root.querySelector('main ul[data-list]')!.getAttribute('aria-busy')).toBe('true');
  expect(root.querySelector('.search-progress progress')).not.toBeNull();
  await settle();
  expect(root.querySelector('main ul[data-list]')!.getAttribute('aria-busy')).toBe(null);
  expect(root.querySelector('.search-progress progress')).toBeNull();
});

test('with no calendar chosen, the search sends no cal and searches all', async () => {
  const { root, settle, flush, requests } = await show();
  const boxes = root.querySelectorAll<HTMLInputElement>('#search-calendars input[type=checkbox]');
  boxes[0]!.click();
  boxes[1]!.click();
  await flush();
  expect([...boxes].map((b) => b.checked)).toEqual([false, false, false]);
  await search(root, settle, 'din');
  expect(requests.calls.filter((c) => c.path === '/api/search').map((c) => c.url)).toEqual(['/api/search?q=din']);
  expect(titles(root)).toEqual(['Dinner', 'Standup']);
});

test('a result opens the detail; edit this instance fills the form; saving posts the form and searches again', async () => {
  const { root, settle, flush, requests } = await show({ 'POST /api/event': '' });
  await search(root, settle, 'standup');
  root.querySelectorAll<HTMLButtonElement>('.search-result')[1]!.click();
  await flush();
  const detail = root.querySelector<HTMLDialogElement>('#event-detail')!;
  expect(detail.open).toBe(true);
  expect(detail.querySelector('h2')!.textContent).toBe('Standup');
  expect([...detail.querySelectorAll('dt')].map((d) => d.textContent)).toEqual(['Time', 'Location', 'Repeats', 'Description', 'Attendees']);
  expect(detail.querySelector('[data-badge]')!.getAttribute('data-tone')).toBe('success');

  detail.querySelectorAll<HTMLButtonElement>('#event-edit-choice button')[1]!.click();
  await flush();
  const form = root.querySelector<HTMLDialogElement>('#event-form')!;
  expect(detail.open).toBe(false);
  expect(form.open).toBe(true);
  expect(form.querySelector<HTMLInputElement>('input[type=text]')!.value).toBe('Standup');
  expect(form.querySelector('small[data-muted]')!.textContent).toBe('Repeat settings apply to the whole series.');

  type(form.querySelector<HTMLInputElement>('input[type=text]')!, 'Standup (moved)');
  const before = requests.calls.length;
  form.querySelector('form')!.requestSubmit();
  await settle();
  const post = requests.calls.find((c) => c.method === 'POST')!;
  const body = new URLSearchParams(String(post.body));
  expect(body.get('summary')).toBe('Standup (moved)');
  expect(body.get('scope')).toBe('this_instance');
  expect(body.get('calendar_href')).toBe('/cal/work/');
  expect(form.open).toBe(false);
  expect(requests.calls.slice(before).filter((c) => c.path === '/api/search')).toHaveLength(1);
});

test('a refused save shows the server message in the form', async () => {
  const { root, settle, flush } = await show({ 'POST /api/event': { status: 400, body: 'Missing account_id' } });
  await search(root, settle, 'dinner');
  root.querySelector<HTMLButtonElement>('.search-result')!.click();
  await flush();
  root.querySelector<HTMLButtonElement>('#event-detail .event-edit')!.click();
  await flush();
  const form = root.querySelector<HTMLDialogElement>('#event-form')!;
  form.querySelector('form')!.requestSubmit();
  await settle();
  expect(form.open).toBe(true);
  expect(form.querySelector('[role=alert]')!.textContent).toBe('Missing account_id');
});

test('no results says so', async () => {
  const { root, settle } = await show({ 'GET /api/search': [] });
  await search(root, settle, 'nothing');
  expect(root.querySelector('main p[data-muted]')!.textContent).toBe('No events found.');
});

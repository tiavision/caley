import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, mockRequests, mockStream, memoryStorage, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import { threadRows } from '../src/lib/thread.ts';
import nav from '../partials/nav.html?raw';
import navToggle from '../partials/nav-toggle.html?raw';
import shareMenu from '../partials/share-menu.html?raw';
import eventDetail from '../partials/event-detail.html?raw';
import eventForm from '../partials/event-form.html?raw';
import threadDialog from '../partials/thread-dialog.html?raw';
import source from './index.html?raw';

// The test page lives at the root of sinter's virtual project: its partials sit next to it.
// render() keeps only what is inside <html>, so the page's <html> state (`view`, persisted) moves to <body> here;
// that `view` is remembered across page loads is checked in the browser.
const page = source.replaceAll('../partials/', './')
  .replace(/<html[^>]*>/, '<html lang="en">')
  .replace("here: '/calendar/',", "here: '/calendar/', view: 'week',");
const NOW = new Date(2026, 10, 11, 12, 0); // Wednesday, Nov 11 2026

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };
const calendars = [{ account_id: 'acc', href: '/cal/work/', name: 'Work', color: '#3a3', is_active: true }];
const event = (uid: string, summary: string, dtstart: string, extra: object = {}) => ({
  account_id: 'acc', uid, time_range: '', summary, color: '#3a3', dtstart, dtend: '', is_all_day: true, tz: '', location: '',
  description: '', is_recurring: false, recurrence: null, calendar_href: '/cal/work/', master_dtstart: '', alarms: [], attendees: [], ...extra,
});
const days = (start: string, events: Record<string, ReturnType<typeof event>[]>, n = 7) => Array.from({ length: n }, (_, i) => {
  const d = new Date(start + 'T00:00:00');
  d.setDate(d.getDate() + i);
  const iso = helpers.localDateIso(d);
  return { date_iso: iso, day_name: '', is_today: false, events: events[iso] ?? [] };
});
const lunch = event('l1', 'Lunch', '2026-11-11');
const grid = (month: string) => ({
  month, month_name: month === '2026-11' ? 'November' : 'December', month_number: Number(month.slice(5)), year: 2026,
  prev_month: '2026-10', next_month: '2026-12',
  days: Array.from({ length: 35 }, (_, i) => {
    const d = new Date(2026, 9, 26 + i);
    return { day_number: d.getDate(), date_str: helpers.localDateIso(d), is_current_month: d.getMonth() === 10, event_colors: [] };
  }),
});

// jsdom has no modal dialogs: open/close set `open` and fire `toggle` as browsers do.
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}
if (!HTMLElement.prototype.hidePopover) HTMLElement.prototype.hidePopover = function () {};
// The islands (src/islands/calendar.ts) measure layout and run timers; jsdom gets inert stand-ins.
for (const name of ['month-fit', 'event-reminders']) {
  if (!customElements.get(name)) customElements.define(name, class extends HTMLElement { set data(_: unknown) {} set days(_: unknown) {} });
}

beforeEach(() => configure({ confirm: () => true, urlSchemes: ['geo:'] }));
afterEach(() => configure({ confirm: null as never, urlSchemes: [] }));

async function show(routes: Record<string, RouteResponse> = {}, storage = memoryStorage()) {
  const requests = mockRequests({
    'GET /api/user': user,
    'GET /api/calendars': calendars,
    'GET /api/caldav/accounts': { accounts: [{ account_id: 'acc', display_name: 'Demo' }] },
    'GET /api/events': (call) => {
      const q = new URL(call.url, 'http://x').searchParams;
      const n = Math.round((Date.parse(q.get('end')!) - Date.parse(q.get('start')!)) / 86400000) + 1;
      return days(q.get('start')!, { '2026-11-11': [lunch] }, n);
    },
    'GET /api/calendar': (call) => grid(new URL(call.url, 'http://x').searchParams.get('month')!),
    'GET /api/calendar/event-emails': [],
    'GET /api/contacts': { addressbooks: [], contacts: [] },
    ...routes,
  });
  const stream = mockStream();
  const actions = {
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    share: { native: () => false, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async () => {} },
  };
  const r = await render(page, {
    requests, actions, helpers, now: NOW, storage, locale: 'en', streams: { '/api/events/stream': stream },
    files: { './nav.html': nav, './nav-toggle.html': navToggle, './share-menu.html': shareMenu, './event-detail.html': eventDetail, './event-form.html': eventForm, './thread-dialog.html': threadDialog },
  });
  await r.settle();
  return { ...r, requests, stream, storage };
}

const key = (target: Element, k: string) => target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
const range = (root: HTMLElement) => root.querySelector('.cal-range')!.textContent;

test('shows the current week; n, p and t move it', async () => {
  const { root, settle, requests } = await show();
  expect(range(root)).toBe('Nov 9 – Nov 15');
  expect(requests.calls.find((c) => c.path === '/api/events')!.url).toMatch(/^\/api\/events\?start=2026-11-09&end=2026-11-15&tz=/);
  expect([...root.querySelectorAll('.week .event-trigger span')].map((s) => s.textContent)).toEqual(['Lunch']);
  expect(root.querySelector('.day[data-date="2026-11-11"] .day-heading')!.getAttribute('aria-current')).toBe('date');
  expect(root.querySelector('.day[data-date="2026-11-09"]')!.classList.contains('past')).toBe(true);

  key(root, 'n');
  await settle();
  expect(range(root)).toBe('Nov 16 – Nov 22');
  key(root, 'P');
  await settle();
  expect(range(root)).toBe('Oct 19 – Oct 25');
  key(root, 't');
  await settle();
  expect(range(root)).toBe('Nov 9 – Nov 15');
});

test('the month view shows the month grid', async () => {
  const first = await show();
  [...first.root.querySelectorAll<HTMLButtonElement>('button[aria-pressed]')].find((b) => b.textContent === 'Month')!.click();
  await first.settle();
  expect(range(first.root)).toBe('November 2026');
  expect(first.root.querySelectorAll('.month-cell')).toHaveLength(35);
  expect(first.root.querySelector('.month-cell[data-date="2026-11-11"] .chip-text')!.textContent).toBe('Lunch');
});

test('a new event goes to the day of its + button; saving reloads the week', async () => {
  const { root, settle, flush, requests } = await show({ 'POST /api/event': '' });
  root.querySelector<HTMLButtonElement>('.day[data-date="2026-11-12"] .day-add')!.click();
  await flush();
  const form = root.querySelector<HTMLDialogElement>('#event-form')!;
  expect(form.open).toBe(true);
  expect(form.querySelector<HTMLInputElement>('input[type=date]')!.value).toBe('2026-11-12');
  const summary = form.querySelector<HTMLInputElement>('input[type=text]')!;
  summary.value = 'Dentist';
  summary.dispatchEvent(new Event('input', { bubbles: true }));
  const before = requests.calls.length;
  form.querySelector('form')!.requestSubmit();
  await settle();
  const body = new URLSearchParams(String(requests.calls.find((c) => c.method === 'POST')!.body));
  expect([body.get('summary'), body.get('date'), body.get('is_all_day'), body.get('calendar_href')]).toEqual(['Dentist', '2026-11-12', 'true', '/cal/work/']);
  expect(form.open).toBe(false);
  expect(requests.calls.slice(before).some((c) => c.path === '/api/events')).toBe(true);
});

test('a removed event closes its detail; server changes reload the week', async () => {
  const { root, settle, flush, stream, requests } = await show();
  root.querySelector<HTMLButtonElement>('.week .event-trigger')!.click();
  await flush();
  const detail = root.querySelector<HTMLDialogElement>('#event-detail')!;
  expect(detail.open).toBe(true);
  const before = requests.calls.length;
  stream.emit('event_removed', { account_id: 'acc', uid: 'l1', calendar_href: '/cal/work/' });
  await settle();
  expect(detail.open).toBe(false);
  expect(requests.calls.slice(before).some((c) => c.path === '/api/events')).toBe(true);
});

test('an event from a mail opens its conversation', async () => {
  const email = { account_id: 'm', mailbox: 'INBOX', uid: 7, from: 'Ada <ada@example.com>', subject: 'Invite', sent_at: 1 };
  const msg = (uid: number, message_id: string, parent: string | null, from: string) =>
    ({ uid, account_id: 'm', mailbox: 'INBOX', message_id, parent_message_id: parent, from, subject: 'Invite', sent_at: 1, is_read: true });
  const { root, settle, flush } = await show({
    'GET /api/calendar/event-emails': [email],
    'GET /api/email/thread': { messages: [msg(7, 'a', null, 'Ada <ada@example.com>'), msg(8, 'b', 'a', 'Bob <bob@example.com>')] },
    'GET /api/email/thread-events': [{ account_id: 'm', mailbox: 'INBOX', uid: 7, events: [lunch] }],
  });
  root.querySelector<HTMLButtonElement>('.week .event-trigger')!.click();
  await settle();
  root.querySelector<HTMLButtonElement>('#event-detail button[data-icon="list-tree"]')!.click();
  await settle();
  await flush();
  const dialog = root.querySelector<HTMLDialogElement>('#thread-dialog')!;
  expect(dialog.open).toBe(true);
  const rows = [...dialog.querySelectorAll<HTMLElement>('.thread-row')];
  expect(rows.map((r) => [r.style.getPropertyValue('--thread-depth'), r.textContent!.replace(/\s+/g, ' ').trim()])).toEqual([
    ['0', 'Ada Invite Jan 1, 1970'.replace('Jan 1, 1970', rows[0]!.querySelector('small')!.textContent!)],
    ['1', expect.stringContaining('Lunch')],
    ['1', expect.stringContaining('Bob')],
  ]);
  expect(rows[1]!.querySelector('button')!.getAttribute('aria-current')).toBe('true');
  expect(rows[0]!.querySelector('a')!.getAttribute('href')).toBe('/mail/?account_id=m&mailbox=INBOX&uid=7');
});

test('threadRows: replies indent under their parent, imported events under their mail', () => {
  const n = (uid: number, id: string, parent: string | null) => ({ uid, account_id: 'm', mailbox: 'I', message_id: id, parent_message_id: parent, from: null, subject: null, sent_at: null, is_read: true });
  const rows = threadRows([n(1, 'a', null), n(2, 'b', 'a'), n(3, 'c', 'b')], [], { account_id: 'm', mailbox: 'I', uid: 2, current: true });
  expect(rows.map((r) => [r.node!.uid, r.depth, r.current])).toEqual([[1, 0, false], [2, 1, true], [3, 2, false]]);
});

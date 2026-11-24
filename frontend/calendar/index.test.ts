import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../src/test-messages.ts';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, mockRequests, mockStream, memoryStorage, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import '../src/islands/common.ts';                 // data-shortcut keys
import { threadRows } from '../src/lib/thread.ts';
import page from './index.html?raw';
import { app } from '../src/test-components.ts';
import '../src/test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

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
    share: { native: () => false, prepare: async () => null, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async () => {} },
  };
  const r = await render(page, { ...app,
    messages, requests, actions, helpers, now: NOW, storage, locale: 'en', streams: { '/api/events/stream': stream },
  });
  await r.settle();
  return { ...r, requests, stream, storage };
}

const key = (target: Element, k: string) => target.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey: k !== k.toLowerCase(), bubbles: true }));
// The range as shown on a wide toolbar (month view: the long month name; phones show the short one, calendar.scss),
// thin spaces (Intl's, around the dash) as plain ones.
const range = (root: HTMLElement) => (root.querySelector('.cal-range .cal-long') ?? root.querySelector('.cal-range'))!.textContent.replace(/\s/g, ' ');

test('shows the current week; n, p and t move it', async () => {
  const { root, settle, requests } = await show();
  expect(range(root)).toBe('Nov 9 – 15');
  expect(requests.calls.find((c) => c.path === '/api/events')!.url).toMatch(/^\/api\/events\?start=2026-11-09&end=2026-11-15&tz=/);
  expect([...root.querySelectorAll('.week .event-trigger span')].map((s) => s.textContent)).toEqual(['Lunch']);
  expect(root.querySelector('.day[data-date="2026-11-11"] .day-heading')!.getAttribute('aria-current')).toBe('date');
  expect(root.querySelector('.day[data-date="2026-11-09"]')!.classList.contains('past')).toBe(true);

  key(root, 'n');
  await settle();
  expect(range(root)).toBe('Nov 16 – 22');
  key(root, 'P');
  await settle();
  expect(range(root)).toBe('Oct 19 – 25');
  key(root, 't');
  await settle();
  expect(range(root)).toBe('Nov 9 – 15');
});

test('Shift+N/P move four weeks with the focus in the week grid; the arrows stay the grid\'s', async () => {
  const { root, settle } = await show({
    'GET /api/events': (call) => days(new URL(call.url, 'http://x').searchParams.get('start')!, {
      '2026-11-11': [lunch], '2026-11-12': [event('v060-d1', 'v060-Dinner', '2026-11-12')],
      '2026-12-09': [event('v060-l2', 'v060-Later', '2026-12-09')],
    }),
  });
  const press = (target: Element, k: string, shiftKey = false) =>
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, bubbles: true, cancelable: true }));
  const chip = (summary: string) => [...root.querySelectorAll<HTMLButtonElement>('.week .event-trigger')].find((b) => b.textContent!.includes(summary))!;

  chip('Lunch').focus();
  expect(press(chip('Lunch'), 'ArrowRight')).toBe(false);          // taken by the grid: the next chip
  await settle();
  expect(document.activeElement).toBe(chip('v060-Dinner'));
  expect(range(root)).toBe('Nov 9 – 15');

  press(chip('v060-Dinner'), 'N', true);                            // not the grid's: the page moves four weeks
  await settle();
  expect(range(root)).toBe('Dec 7 – 13');
  press(chip('v060-Later'), 'P', true);
  await settle();
  expect(range(root)).toBe('Nov 9 – 15');
});

test('the month view shows the month grid, and is remembered across reloads', async () => {
  const first = await show();
  first.root.querySelector<HTMLInputElement>('input[name=cal-view][value=month]')!.click();
  await first.settle();
  expect(range(first.root)).toBe('November 2026');
  expect(first.root.querySelectorAll('.month-cell')).toHaveLength(42);
  expect(first.root.querySelector('.month-cell[data-date="2026-11-11"] .chip-text')!.textContent).toBe('Lunch');
  await first.window('pagehide');

  const second = await show({}, first.storage);
  expect(range(second.root)).toBe('November 2026');
  expect(second.root.querySelectorAll('.month-cell')).toHaveLength(42);
  expect(second.root.querySelector<HTMLInputElement>('input[name=cal-view]:checked')!.value).toBe('month');
});

test('a first visit starts in the week view', async () => {
  const { root } = await show();
  expect(range(root)).toBe('Nov 9 – 15');
  expect(root.querySelectorAll('.month-cell')).toHaveLength(0);
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

test('leaving the page is refused while the open event form has been edited, with a note; closed, it goes', async () => {
  const { root, settle, flush } = await show();
  const leave = async () => {
    const click = new MouseEvent('click', { bubbles: true, cancelable: true });
    root.querySelector('nav[data-rail] a[href$="/mail/"]')!.dispatchEvent(click);
    await settle();
    return click.defaultPrevented;
  };
  root.querySelector<HTMLButtonElement>('.day[data-date="2026-11-12"] .day-add')!.click();
  await flush();
  const form = root.querySelector<HTMLDialogElement>('#event-form')!;
  expect(await leave()).toBe(false);                                   // nothing typed yet
  const summary = form.querySelector<HTMLInputElement>('input[type=text]')!;
  summary.value = 'Dentist';
  summary.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
  expect(await leave()).toBe(true);
  expect(form.querySelector('[role=alert]')!.textContent).toBe(m.draft_leave());
  form.close();
  await settle();
  expect(await leave()).toBe(false);
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

test('a view saved by the old page (its whole <html> state, under calendar-view) is left alone: the week view', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:calendar-view', JSON.stringify({ view: 'month' }));
  const { root } = await show({}, storage);
  expect(root.querySelector<HTMLInputElement>('input[name=cal-view]:checked')!.value).toBe('week');
  expect(root.querySelectorAll('.month-cell')).toHaveLength(0);
});

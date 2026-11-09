import '../test-messages.ts';
// The small helpers the pages' handlers apply (`x = fn(…)`): calendar, search, contacts, create-calendar dialog.
import { describe, expect, test } from 'vitest';
import { farStep, hasDay, inWeek, monthCells, monthRange, swipeStep, weekDates } from './calendar.ts';
import { calendarsLabel, newCalendarBody, newCalendarDraft, sameEvent, submittedSearch, searchQuery } from './events.ts';
import { addressbooksLabel, sameContact } from './contacts.ts';
import { elementIn } from './dom.ts';

// dispatched, so the event has a target (shortcuts ignore keys typed into fields)
const press = (init: KeyboardEventInit) => { const e = new KeyboardEvent('keydown', init); document.body.dispatchEvent(e); return e; };
const keydown = (key: string, shiftKey = key !== key.toLowerCase()) => press({ key, shiftKey });
const touch = (x: number, y: number) => ({ changedTouches: [{ clientX: x, clientY: y }] }) as unknown as TouchEvent;

describe('calendar', () => {
  // The backend's grid (list_calendar in backend/src/calendar/mod.rs): from the Monday on or before the 1st, whole
  // weeks through the week of the last day. The page builds it itself, so both must agree.
  test('monthCells: whole weeks around the month, as /api/calendar builds them', () => {
    expect(monthRange('2026-11')).toEqual({ start: '2026-10-26', end: '2026-12-06' }); // starts on a Sunday: 6 rows
    expect(monthRange('2026-06')).toEqual({ start: '2026-06-01', end: '2026-07-05' }); // starts on a Monday
    expect(monthCells('2027-02')).toHaveLength(28); // Monday the 1st, 28 days: exactly 4 rows
    expect(monthRange('2027-02')).toEqual({ start: '2027-02-01', end: '2027-02-28' });
    expect(monthRange('2024-02')).toEqual({ start: '2024-01-29', end: '2024-03-03' }); // leap year
    expect(monthRange('2026-12')).toEqual({ start: '2026-11-30', end: '2027-01-03' }); // across the year
    const nov = monthCells('2026-11');
    expect(nov.filter((c) => c.is_current_month).map((c) => c.day_number)).toEqual(Array.from({ length: 30 }, (_, i) => i + 1));
    expect(nov[0]).toEqual({ date_str: '2026-10-26', day_number: 26, is_current_month: false });
    for (let y = 1990; y <= 2040; y++) {
      for (let mo = 1; mo <= 12; mo++) {
        const ym = `${y}-${String(mo).padStart(2, '0')}`;
        const cells = monthCells(ym);
        const first = new Date(y, mo - 1, 1), days = new Date(y, mo, 0).getDate();
        const offset = (first.getDay() + 6) % 7;
        expect(cells.length).toBe(Math.ceil((offset + days) / 7) * 7);
        expect(new Date(`${cells[0]!.date_str}T12:00`).getDay()).toBe(1); // a Monday
        expect(cells.filter((c) => c.is_current_month)).toHaveLength(days);
        expect(cells[offset]!.date_str).toBe(`${ym}-01`);
      }
    }
  });

  test('weekDates and hasDay', () => {
    expect(weekDates('2026-10-26')).toEqual(['2026-10-26', '2026-10-27', '2026-10-28', '2026-10-29', '2026-10-30', '2026-10-31', '2026-11-01']);
    const days = [{ date_iso: '2026-11-09', day_name: '', is_today: false, events: [] }];
    expect([hasDay(days, '2026-11-09'), hasDay(days, '2026-11-10'), hasDay(null, '2026-11-09')]).toEqual([true, false, false]);
  });

  test('farStep: Shift+N/P move four weeks or three months; plain keys and others do not', () => {
    expect([farStep(keydown('N'), 'week'), farStep(keydown('P'), 'week')]).toEqual([28, -28]);
    expect([farStep(keydown('N'), 'month'), farStep(keydown('P'), 'month')]).toEqual([3, -3]);
    expect(farStep(keydown('n'), 'week')).toBe(0);
    expect(farStep(keydown('N', false), 'week')).toBe(0);          // Caps Lock: plain n (the button's shortcut)
    expect(farStep(press({ key: 'N', shiftKey: true, ctrlKey: true }), 'week')).toBe(0);
  });
  test('inWeek: Monday to Sunday', () => {
    expect(['2026-11-08', '2026-11-09', '2026-11-15', '2026-11-16'].map((d) => inWeek(d, '2026-11-09'))).toEqual([false, true, true, false]);
  });
  test('swipeStep: a week (in days) or a month per swipe, nothing for short or vertical moves', () => {
    expect(swipeStep(200, 100, touch(100, 110), 'week')).toBe(7);
    expect(swipeStep(100, 100, touch(200, 110), 'week')).toBe(-7);
    expect(swipeStep(200, 100, touch(100, 110), 'month')).toBe(1);
    expect(swipeStep(100, 100, touch(130, 100), 'week')).toBe(0);
    expect(swipeStep(100, 100, touch(170, 300), 'month')).toBe(0);
  });
});

describe('events and search', () => {
  const ev = { account_id: 'a', calendar_href: '/c/', uid: 'u' };
  test('sameEvent compares account, calendar and uid', () => {
    expect(sameEvent({ ...ev, dtstart: 'x' } as never, ev)).toBe(true);
    expect(sameEvent({ ...ev, uid: 'v' }, ev)).toBe(false);
    expect(sameEvent(null, ev)).toBe(false);
  });
  test('submittedSearch: a new search replaces the old one; the same one or an empty one keeps it', () => {
    const old = searchQuery('lunch', '', '', ['a']);
    expect(submittedSearch(old, searchQuery(' lunch ', '', '', ['a']))).toBe(old);
    expect(submittedSearch(old, searchQuery('  ', '', '', []))).toBe(old);
    expect(submittedSearch(null, searchQuery('', '', '', []))).toBe(null);
    expect(submittedSearch(old, searchQuery('dinner', '', '', ['a']))).toEqual({ q: 'dinner', start: '', end: '', cals: ['a'] });
  });
  test('calendarsLabel shows the count only while some calendars are left out', () => {
    expect(calendarsLabel(3, 3)).toBe('Calendars');
    expect(calendarsLabel(0, 0)).toBe('Calendars');
    expect(calendarsLabel(1, 3)).toBe('Calendars (1/3)');
  });
  test('the create-calendar draft starts in the first account; its body sends a color only when chosen', () => {
    expect(newCalendarDraft([{ account_id: 'x' }, { account_id: 'y' }])).toEqual({ name: '', account_id: 'x', color: '#4a90d9', useColor: false });
    expect(newCalendarDraft([]).account_id).toBe('');
    expect(newCalendarBody({ name: ' Work ', account_id: 'x', color: '#ff0000', useColor: false })).toEqual({ name: 'Work', account_id: 'x', color: null });
    expect(newCalendarBody({ name: 'Work', account_id: 'x', color: '#ff0000', useColor: true }).color).toBe('#ff0000');
  });
});

describe('contacts', () => {
  test('addressbooksLabel and sameContact', () => {
    expect(addressbooksLabel(2, 2)).toBe('Address books');
    expect(addressbooksLabel(1, 2)).toBe('Address books (1/2)');
    const id = { account_id: 'a', uid: '1', addressbook_href: '/ab/' };
    expect(sameContact(id, { ...id })).toBe(true);
    expect(sameContact(id, { ...id, uid: '2' })).toBe(false);
    expect(sameContact(null, id)).toBe(false);
  });
});

test('elementIn finds a descendant as an HTMLElement', () => {
  document.body.innerHTML = '<div class="pane"><button data-pos="0:1"></button></div>';
  expect(elementIn(document.body, "[data-pos='0:1']")?.tagName).toBe('BUTTON');
  expect(elementIn(document.body, '.missing')).toBeNull();
});

// Pure helpers for the calendar page (calendar/index.html): dates as YYYY-MM-DD strings in the viewer's zone,
// months as YYYY-MM. Exported to templates through src/sinter.helpers.ts.
import { m } from '@tiaforge-ui/sinter/messages';
import type { DayEventsJson, EventJson, MonthGridJson } from '../types/calendar.ts';
import { localDateIso } from './format.ts';
import { eventKey } from './events.ts';
import { shortcutKey } from './contacts.ts';

function parse(iso: string): Date {
  const [y, mo, d] = iso.split('-').map(Number);
  return new Date(y!, (mo ?? 1) - 1, d ?? 1);
}

/** The viewer's time zone for the `tz` parameter of /api/events and /api/calendar ('' when unknown). */
export function userTz(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch { return ''; }
}

export function addDays(iso: string, n: number): string {
  const d = parse(iso);
  d.setDate(d.getDate() + n);
  return localDateIso(d);
}

/** The Monday of the week of `iso`. */
export function mondayOf(iso: string): string {
  const d = parse(iso);
  const day = d.getDay();
  d.setDate(d.getDate() - day + (day === 0 ? -6 : 1));
  return localDateIso(d);
}

export function monthOf(iso: string): string {
  return iso.slice(0, 7);
}

export function addMonths(ym: string, n: number): string {
  const [y, mo] = ym.split('-').map(Number);
  return localDateIso(new Date(y!, mo! - 1 + n, 1)).slice(0, 7);
}

/** "Nov 9 – 15", with the year where it is not the current one. */
export function weekRangeText(weekStart: string, locale: string, today: string): string {
  const start = parse(weekStart), end = parse(addDays(weekStart, 6));
  const cy = parse(today).getFullYear();
  // formatRange leaves out what both ends share ("Nov 9 – 15", "9.–15. Nov."), which keeps the toolbar on one line.
  const options: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
  if (start.getFullYear() !== cy || end.getFullYear() !== cy) options.year = 'numeric';
  return new Intl.DateTimeFormat(locale, options).formatRange(start, end);
}

/** A day's date formatted with `options` (weekday, "Nov 9", long date …). */
export function dayLabel(iso: string, locale: string, options: Intl.DateTimeFormatOptions): string {
  return iso ? parse(iso).toLocaleDateString(locale, options) : '';
}

/** Past days of the current week start collapsed (narrow screens); a click on the day heading flips that. */
export function dayCollapsed(iso: string, today: string, flipped: readonly string[]): boolean {
  const defaultCollapsed = iso < today && iso >= mondayOf(today) && iso <= addDays(mondayOf(today), 6);
  return defaultCollapsed !== flipped.includes(iso);
}

/** The events of one day of a /api/events response. */
export function eventsOn(days: readonly DayEventsJson[], iso: string): EventJson[] {
  return days.find((d) => d.date_iso === iso)?.events ?? [];
}

/** The week grid's keyboard focus: `pos` is "column:row" over the days that have events. */
export function weekNeighbor(days: readonly DayEventsJson[], pos: string, key: string): string | null {
  const cols = days.filter((d) => d.events.length > 0).map((d) => d.events.length);
  let [c, r] = pos.split(':').map(Number) as [number, number];
  if (key === 'ArrowLeft' && c > 0) { c--; r = Math.min(r, cols[c]! - 1); }
  else if (key === 'ArrowRight' && c < cols.length - 1) { c++; r = Math.min(r, cols[c]! - 1); }
  else if (key === 'ArrowUp' && r > 0) r--;
  else if (key === 'ArrowDown' && r < (cols[c] ?? 0) - 1) r++;
  else return null;
  return `${c}:${r}`;
}

/** The chip an arrow key in the week grid moves the focus to ("column:row", see weekNeighbor()), or null. */
export function weekMove(e: KeyboardEvent, days: readonly DayEventsJson[]): string | null {
  return weekNeighbor(days, posOf(e), e.key);
}

/** The "column:row" of every event in the week grid (columns count only days with events), by eventKey. */
export function weekPositions(days: readonly DayEventsJson[]): Record<string, string> {
  const out: Record<string, string> = {};
  days.filter((d) => d.events.length > 0).forEach((d, c) => d.events.forEach((ev, r) => { out[eventKey(ev)] = `${c}:${r}`; }));
  return out;
}

/** The date of the week grid's column `c` (counting days with events). */
export function weekColumnDate(days: readonly DayEventsJson[], pos: string): string {
  return days.filter((d) => d.events.length > 0)[Number(pos.split(':')[0])]?.date_iso ?? '';
}

/**
 * Shift+N / Shift+P on the calendar page: how far they move, in days (week view: four weeks) or months (month
 * view: three), else 0 (from a dialog or with Ctrl/Alt/Meta). Plain n/p are the buttons' own shortcuts (data-shortcut).
 */
export function farStep(e: KeyboardEvent, view: string): number {
  const key = e.shiftKey ? shortcutKey(e) : null;
  const step = key === 'N' ? 1 : key === 'P' ? -1 : 0;
  return step * (view === 'month' ? 3 : 28);
}

/** Whether the day `iso` (YYYY-MM-DD) is in the week starting on Monday `weekStart`. */
export function inWeek(iso: string, weekStart: string): boolean {
  return iso >= weekStart && iso <= addDays(weekStart, 6);
}

// ── Month view and picker ───────────────────────────────────────────────────

/** Monday-first weekday names, short (picker) or from the messages (month grid header). */
export function weekdayNames(locale: string): string[] {
  return Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(locale, { weekday: 'short' }));
}

export function monthGridHeader(): string[] {
  return [m.event_dayMo(), m.event_dayTu(), m.event_dayWe(), m.event_dayTh(), m.event_dayFr(), m.event_daySa(), m.event_daySu()];
}

export function monthNames(locale: string): string[] {
  return Array.from({ length: 12 }, (_, i) => new Date(2024, i, 1).toLocaleDateString(locale, { month: 'long' }));
}

/** The picker's years: fifty back and fifty ahead of `today`'s. */
export function pickerYears(today: string): number[] {
  const y = parse(today).getFullYear();
  return Array.from({ length: 101 }, (_, i) => y - 50 + i);
}

export function monthTitle(grid: MonthGridJson | null): string {
  return grid ? `${grid.month_name} ${grid.year}` : '';
}

/** "November 2026" for a month (YYYY-MM), in `locale`. */
export function monthLabel(ym: string, locale: string, month: 'long' | 'short' = 'long'): string {
  const [y, mo] = ym.split('-').map(Number);
  return new Date(y!, mo! - 1, 1).toLocaleDateString(locale, { month, year: 'numeric' });
}

/** A cell of the month grid, as the browser builds it (no data needed). */
export interface MonthCell { date_str: string; day_number: number; is_current_month: boolean }

/**
 * The month grid's cells for `ym` (YYYY-MM): whole weeks from the Monday on or before the 1st through the week
 * of the last day, as /api/calendar builds them (backend list_calendar), so the grid and its events request need
 * no response first.
 */
export function monthCells(ym: string): MonthCell[] {
  const [y, mo] = ym.split('-').map(Number);
  const first = new Date(y!, mo! - 1, 1);
  const offset = (first.getDay() + 6) % 7; // days since Monday
  const daysInMonth = new Date(y!, mo!, 0).getDate();
  const total = Math.ceil((offset + daysInMonth) / 7) * 7;
  return Array.from({ length: total }, (_, i) => {
    const d = new Date(y!, mo! - 1, 1 - offset + i);
    return { date_str: localDateIso(d), day_number: d.getDate(), is_current_month: d.getMonth() === mo! - 1 };
  });
}

/** The first and last date of the month grid of `ym` (the range of its /api/events request). */
export function monthRange(ym: string): { start: string; end: string } {
  const cells = monthCells(ym);
  return { start: cells[0]!.date_str, end: cells[cells.length - 1]!.date_str };
}

/** The seven dates of the week starting on Monday `weekStart`. */
export function weekDates(weekStart: string): string[] {
  return Array.from({ length: 7 }, (_, i) => addDays(weekStart, i));
}

/** Whether a /api/events response has the day `iso` (it has every day of its range, empty ones too). */
export function hasDay(days: readonly DayEventsJson[] | null | undefined, iso: string): boolean {
  return !!days?.some((d) => d.date_iso === iso);
}

const NO_DAYS: readonly DayEventsJson[] = Object.freeze([]);

/**
 * The /api/events response if it is the month grid's `ym` (it has the grid's first and last day), else none. While
 * the next month loads the resource still holds the last one, which shares the grid's first or last row with it:
 * those cells would show the old response's events, then lose them while <month-fit> fits the new month.
 */
export function monthDaysOf(days: readonly DayEventsJson[] | null | undefined, ym: string): readonly DayEventsJson[] {
  const { start, end } = monthRange(ym);
  return days && hasDay(days, start) && hasDay(days, end) ? days : NO_DAYS;
}

/** A month of the picker's grid with its month or year changed by the selects. */
export function pickerMonth(grid: MonthGridJson | null, field: 'month' | 'year', value: string): string {
  const year = field === 'year' ? Number(value) : grid?.year ?? new Date().getFullYear();
  const month = field === 'month' ? Number(value) : grid?.month_number ?? 1;
  return `${year}-${String(month).padStart(2, '0')}`;
}

/** The events of the cells the month grid shows: those that fit (`fit`, measured by <month-fit>), the rest as "+N". */
export function cellEvents(days: readonly DayEventsJson[], iso: string, fit: Readonly<Record<string, number>>): { shown: EventJson[]; hidden: number; all: EventJson[] } {
  const all = eventsOn(days, iso);
  const shown = iso in fit ? all.slice(0, fit[iso]) : all;
  return { shown, hidden: all.length - shown.length, all };
}

/** The week grid position ("column:row") of the event chip a key event came from, or ''. */
export function posOf(e: Event): string {
  return (e.target as HTMLElement | null)?.dataset?.pos ?? '';
}

/** How far a swipe moves the calendar: a week (in days) or a month back or forward, or 0 for no swipe. */
export function swipeStep(x: number, y: number, e: TouchEvent, view: string): number {
  return swipeDir(x, y, e) * (view === 'week' ? 7 : 1);
}

/** A horizontal swipe from (x, y) to the touch's end: -1 (to the previous), 1 (to the next) or 0. */
export function swipeDir(x: number, y: number, e: TouchEvent): number {
  const t = e.changedTouches[0];
  if (!t) return 0;
  const dx = t.clientX - x, dy = t.clientY - y;
  if (Math.abs(dx) < 50 || Math.abs(dx) < Math.abs(dy)) return 0;
  return dx > 0 ? -1 : 1;
}

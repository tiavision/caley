// Pure helpers for the calendar page (calendar/index.html): dates as YYYY-MM-DD strings in the viewer's zone,
// months as YYYY-MM. Exported to templates through src/sinter.helpers.ts.
import { m } from '../paraglide/messages.js';
import type { DayEventsJson, EventJson, MonthGridJson } from '../types/calendar.ts';
import { localDateIso } from './format.ts';
import { eventKey } from './events.ts';

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
  const so: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
  const eo: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
  if (start.getFullYear() !== cy) so.year = 'numeric';
  if (end.getFullYear() !== cy || start.getFullYear() !== end.getFullYear()) eo.year = 'numeric';
  return start.toLocaleDateString(locale, so) + ' – ' + end.toLocaleDateString(locale, eo);
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

/** Keyboard shortcuts of the calendar page: how far `n`/`p` (`N`/`P`) move, in days (week) or months. */
export function navStep(key: string | null, view: string): number {
  const week: Record<string, number> = { n: 7, N: 28, p: -7, P: -28 };
  const month: Record<string, number> = { n: 1, N: 3, p: -1, P: -3 };
  return (key ? (view === 'month' ? month : week)[key] : 0) ?? 0;
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

/** A horizontal swipe from (x, y) to the touch's end: -1 (to the previous), 1 (to the next) or 0. */
export function swipeDir(x: number, y: number, e: TouchEvent): number {
  const t = e.changedTouches[0];
  if (!t) return 0;
  const dx = t.clientX - x, dy = t.clientY - y;
  if (Math.abs(dx) < 50 || Math.abs(dx) < Math.abs(dy)) return 0;
  return dx > 0 ? -1 : 1;
}

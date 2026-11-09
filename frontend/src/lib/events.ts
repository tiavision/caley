// Pure helpers for the event partials (partials/event-detail.html, partials/event-form.html) and the
// calendar and search pages. Exported to templates through src/sinter.helpers.ts; nothing here touches
// the DOM, storage or the network.
import { m } from '@tiaforge-ui/sinter/messages';
import type {
  AlarmFields, AttendeeFields, CalDavAccountJson, CalendarJson, ContactJson, EventJson, PartStat, RecurrenceInfo,
} from '../types/calendar.ts';
import { dateFromEventValue, localDateIso } from './format.ts';

// ── Calendars ───────────────────────────────────────────────────────────────

/** The "Create calendar" dialog (partials/create-calendar-dialog.html). */
export interface CalendarDraft { name: string; account_id: string; color: string; useColor: boolean }

/** An empty draft in the first account. */
export function newCalendarDraft(accounts: readonly Pick<CalDavAccountJson, 'account_id'>[]): CalendarDraft {
  return { name: '', account_id: accounts[0]?.account_id ?? '', color: '#4a90d9', useColor: false };
}

/** POST /api/calendar (form encoded): the color only when chosen. */
export function newCalendarBody(d: CalendarDraft): { name: string; account_id: string; color: string | null } {
  return { name: d.name.trim(), account_id: d.account_id, color: d.useColor && d.color ? d.color : null };
}

// ── Keys ────────────────────────────────────────────────────────────────────

/** A calendar's identity, as the search `cal` parameter and the form's calendar select carry it. */
export function calendarKey(c: Pick<CalendarJson, 'account_id' | 'href'>): string {
  return `${c.account_id}\0${c.href}`;
}

/** One occurrence of an event (recurring events repeat uid and differ by dtstart). */
export function eventKey(ev: Pick<EventJson, 'account_id' | 'calendar_href' | 'uid' | 'dtstart'>): string {
  return `${ev.account_id}\0${ev.calendar_href}\0${ev.uid}\0${ev.dtstart}`;
}

type EventId = Pick<EventJson, 'account_id' | 'calendar_href' | 'uid'>;

/** Whether `ev` is the event `id` names (every occurrence of a series is). */
export function sameEvent(ev: EventId | null, id: EventId): boolean {
  return !!ev && ev.account_id === id.account_id && ev.calendar_href === id.calendar_href && ev.uid === id.uid;
}

/** The calendar's name for an event, or ''. */
export function calendarName(calendars: readonly CalendarJson[], ev: Pick<EventJson, 'account_id' | 'calendar_href'>): string {
  return calendars.find((c) => c.account_id === ev.account_id && c.href === ev.calendar_href)?.name ?? '';
}

// ── Event detail ────────────────────────────────────────────────────────────

export type Tone = 'success' | 'danger' | 'warning' | 'neutral';

export function partstatTone(partstat: PartStat): Tone {
  if (partstat === 'ACCEPTED') return 'success';
  if (partstat === 'DECLINED') return 'danger';
  if (partstat === 'TENTATIVE') return 'warning';
  return 'neutral';
}

export function partstatLabel(partstat: PartStat): string {
  // messages by name, so the page's language file holds what it uses
  switch (partstat) {
    case 'ACCEPTED': return m.event_attendeeStatusAccepted();
    case 'DECLINED': return m.event_attendeeStatusDeclined();
    case 'TENTATIVE': return m.event_attendeeStatusTentative();
    default: return m.event_attendeeStatusNeedsAction();
  }
}

export function attendeeLabel(a: Pick<AttendeeFields, 'cn' | 'email'>): string {
  return a.cn ? `${a.cn} <${a.email}>` : a.email;
}

/** Long date of an event's start in the viewer's zone (timed values) or as written (all-day). */
export function eventDateLong(dtstart: string, locale: string): string {
  if (dtstart.includes('T') && dtstart.includes('-')) {
    const d = dateFromEventValue(dtstart);
    if (d) return d.toLocaleDateString(locale, { dateStyle: 'long' });
  }
  const datePart = dtstart.split('T')[0]!.replace(/(\d{4})(\d{2})(\d{2})/, '$1-$2-$3');
  const [y, mo, d] = datePart.split('-').map(Number);
  if (!y || !mo || !d) return dtstart;
  return new Date(y, mo - 1, d).toLocaleDateString(locale, { dateStyle: 'long' });
}

// ── Event form ──────────────────────────────────────────────────────────────

/** The recurrence builder's fields (partials/event-form.html). */
export interface RecurrenceDraft {
  freq: string;
  interval: number | null;
  by_day: string[];
  monthly_mode: 'by_day' | 'by_setpos';
  monthday: number | null;
  setpos: string;
  setpos_day: string;
  end_type: 'never' | 'count' | 'until';
  count: number | null;
  until: string;
}

/** What partials/event-form.html edits; eventFormBody() turns it into the POST /api/event form. */
export interface EventDraft {
  mode: 'create' | 'edit';
  scope: 'series' | 'this_instance';
  uid: string;
  summary: string;
  date: string;
  is_all_day: boolean;
  start_time: string;
  end_time: string;
  location: string;
  description: string;
  tz: string;
  original_tz: string;
  original_dtstart: string;
  original_calendar_href: string;
  original_account_id: string;
  /** calendarKey() of the target calendar */
  cal: string;
  alarms: AlarmFields[];
  attendees: AttendeeFields[];
  rec: RecurrenceDraft;
}

export const WEEKDAYS: ReadonlyArray<{ code: string; label: () => string }> = [
  { code: 'MO', label: m.event_dayMo }, { code: 'TU', label: m.event_dayTu }, { code: 'WE', label: m.event_dayWe },
  { code: 'TH', label: m.event_dayTh }, { code: 'FR', label: m.event_dayFr }, { code: 'SA', label: m.event_daySa },
  { code: 'SU', label: m.event_daySu },
];

/** Weekday codes with their short names, Monday first. */
export function weekdays(): Array<{ code: string; label: string }> {
  return WEEKDAYS.map((d) => ({ code: d.code, label: d.label() }));
}

function userTz(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; } catch { return 'UTC'; }
}

function emptyRecurrence(): RecurrenceDraft {
  return {
    freq: '', interval: 1, by_day: [], monthly_mode: 'by_day', monthday: 1, setpos: '1', setpos_day: 'MO',
    end_type: 'never', count: 1, until: '',
  };
}

function recurrenceDraft(rec: RecurrenceInfo | null): RecurrenceDraft {
  const r = emptyRecurrence();
  if (!rec?.freq) return r;
  r.freq = rec.freq;
  r.interval = rec.interval || 1;
  if (rec.freq === 'WEEKLY') r.by_day = rec.by_day ? [...rec.by_day] : [];
  if (rec.freq === 'MONTHLY') {
    if (rec.by_set_pos?.length) { r.monthly_mode = 'by_setpos'; r.setpos = String(rec.by_set_pos[0]); r.setpos_day = rec.by_day?.[0] || 'MO'; }
    else { r.monthly_mode = 'by_day'; r.monthday = rec.by_month_day?.[0] || 1; }
  }
  if (rec.count) { r.end_type = 'count'; r.count = rec.count; }
  else if (rec.until) { r.end_type = 'until'; r.until = rec.until.split(' ')[0]!; }
  return r;
}

/** The server's alarm time (`20240101T100000Z` or ISO) as a datetime-local value in the viewer's zone. */
function alarmInputValue(dt: string): string {
  if (!dt) return '';
  const iso = dt.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
  const d = new Date(iso);
  if (isNaN(d.getTime())) return '';
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
}

/**
 * A new event on `date` (YYYY-MM-DD) in the remembered calendar `defaultCal` (a calendarKey) when it is
 * still active, else the first active calendar, else the first one.
 */
export function newEventDraft(calendars: readonly CalendarJson[], date: string, defaultCal: string): EventDraft {
  const stored = defaultCal ? calendars.find((c) => calendarKey(c) === defaultCal && c.is_active) : undefined;
  const selected = stored ?? calendars.find((c) => c.is_active) ?? calendars[0];
  return {
    mode: 'create', scope: 'series', uid: '', summary: '', date, is_all_day: true, start_time: '', end_time: '',
    location: '', description: '', tz: userTz(), original_tz: '', original_dtstart: '', original_calendar_href: '',
    original_account_id: '', cal: selected ? calendarKey(selected) : '', alarms: [], attendees: [], rec: emptyRecurrence(),
  };
}

/** Today in the viewer's zone as YYYY-MM-DD (pass `$now`). */
export function todayIso(now: Date): string {
  return localDateIso(now);
}

/**
 * Edits `ev`, the whole series or this one occurrence. The event is always saved in the browser's zone
 * (re-anchoring it); the master's original zone is kept only for RECURRENCE-ID/EXDATE targeting.
 */
export function editEventDraft(ev: EventJson, scope: 'series' | 'this_instance'): EventDraft {
  const dtstart = scope === 'series' && ev.is_recurring && ev.master_dtstart ? ev.master_dtstart : ev.dtstart;
  let date = dtstart.split('T')[0] || '', start = '', end = '';
  if (!ev.is_all_day) {
    const hm = (d: Date) => String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
    const s = dateFromEventValue(dtstart);
    const e = ev.dtend ? dateFromEventValue(ev.dtend) : null;
    if (s) { date = localDateIso(s); start = hm(s); }
    if (e) end = hm(e);
  }
  return {
    mode: 'edit', scope, uid: ev.uid, summary: ev.summary, date, is_all_day: ev.is_all_day, start_time: start, end_time: end,
    location: ev.location, description: ev.description, tz: userTz(), original_tz: ev.tz || '',
    original_dtstart: ev.dtstart, original_calendar_href: ev.calendar_href, original_account_id: ev.account_id,
    cal: calendarKey({ account_id: ev.account_id, href: ev.calendar_href }),
    alarms: ev.alarms.map((a) => ({ ...a, datetime: a.direction === 'absolute' ? alarmInputValue(a.datetime) : a.datetime })),
    attendees: ev.attendees.map((a) => ({ ...a })),
    rec: scope === 'series' && ev.is_recurring ? recurrenceDraft(ev.recurrence) : emptyRecurrence(),
  };
}

export function newAlarm(): AlarmFields {
  return { direction: 'before-start', value: '15', unit: '1', datetime: '' };
}

export function newAttendee(): AttendeeFields {
  return { cn: '', email: '', partstat: 'NEEDS-ACTION' };
}

/** HH:MM thirty minutes after `start` (HH:MM), wrapping at midnight. */
export function halfHourAfter(start: string): string {
  const [h, min] = start.split(':').map(Number);
  const total = (h ?? 0) * 60 + (min ?? 0) + 30;
  return String(Math.floor(total / 60) % 24).padStart(2, '0') + ':' + String(total % 60).padStart(2, '0');
}

/** The RecurrenceInfo JSON the server takes in `recurrence`, or '' for none. */
export function recurrenceJson(r: RecurrenceDraft): string {
  if (!r.freq) return '';
  const obj: RecurrenceInfo = {
    freq: r.freq, interval: r.interval || 1, by_day: [], by_month_day: [], by_month: [], by_set_pos: [], until: null, count: null,
  };
  if (r.freq === 'WEEKLY') obj.by_day = [...r.by_day];
  if (r.freq === 'MONTHLY') {
    if (r.monthly_mode === 'by_setpos') { obj.by_set_pos = [Number(r.setpos)]; obj.by_day = [r.setpos_day]; }
    else obj.by_month_day = [r.monthday ?? 1];
  }
  if (r.end_type === 'count') obj.count = r.count;
  else if (r.end_type === 'until') obj.until = r.until || null;
  return JSON.stringify(obj);
}

/** The POST /api/event form (sent with data-sn-encoding="form": arrays become repeated keys). */
export function eventFormBody(d: EventDraft): Record<string, string | string[]> {
  const [account_id = '', calendar_href = ''] = d.cal.split('\0');
  return {
    uid: d.uid, summary: d.summary, date: d.date, is_all_day: d.is_all_day ? 'true' : '',
    start_time: d.start_time, end_time: d.end_time, location: d.location, description: d.description,
    tz: d.tz, original_tz: d.original_tz, scope: d.scope, original_dtstart: d.original_dtstart,
    original_calendar_href: d.original_calendar_href, original_account_id: d.original_account_id,
    account_id, calendar_href, recurrence: recurrenceJson(d.rec),
    alarm_direction: d.alarms.map((a) => a.direction),
    alarm_value: d.alarms.map((a) => String(a.value ?? '')),
    alarm_unit: d.alarms.map((a) => a.unit),
    alarm_datetime: d.alarms.map((a) => a.datetime),
    attendee_cn: d.attendees.map((a) => a.cn),
    attendee_email: d.attendees.map((a) => a.email),
    attendee_partstat: d.attendees.map((a) => a.partstat),
  };
}

/** The repeat interval's unit ("weeks") for a frequency. */
export function intervalUnit(freq: string): string {
  const units: Record<string, () => string> = {
    DAILY: m.event_repeatUnitDaily, WEEKLY: m.event_repeatUnitWeekly, MONTHLY: m.event_repeatUnitMonthly, YEARLY: m.event_repeatUnitYearly,
  };
  return units[freq]?.() ?? '';
}

/** Calendars grouped by account for the form's <optgroup>s, in the order calendars come. */
export function calendarGroups(calendars: readonly CalendarJson[], accounts: readonly CalDavAccountJson[]): Array<{ account_id: string; label: string; calendars: CalendarJson[] }> {
  const groups: Array<{ account_id: string; label: string; calendars: CalendarJson[] }> = [];
  for (const cal of calendars) {
    let g = groups.find((x) => x.account_id === cal.account_id);
    if (!g) {
      g = { account_id: cal.account_id, label: accounts.find((a) => a.account_id === cal.account_id)?.display_name || cal.account_id, calendars: [] };
      groups.push(g);
    }
    g.calendars.push(cal);
  }
  return groups;
}

// ── Summary suggestions: "10 Lunch" offers 10:00 and 22:00, "tom Lunch" offers tomorrow ──────────

export interface SummarySuggestion {
  label: string;
  /** the typed prefix the suggestion replaces */
  prefixLen: number;
  /** a time suggestion: hours and minutes */
  h?: number;
  m?: number;
  /** a day suggestion: YYYY-MM-DD */
  date?: string;
}

function timeSuggestions(input: string): SummarySuggestion[] {
  const match = input.match(/^(\d{1,2})(?::(\d{0,2}))?(?:[ \t]*(am|pm))?(?=[ \t]|$)/i);
  if (!match) return [];
  const prefixLen = match[0].length;
  const h = parseInt(match[1]!, 10);
  const minStr = match[2] ?? '';
  const min = minStr ? parseInt(minStr.padEnd(2, '0'), 10) : 0;
  const ampm = (match[3] ?? '').toLowerCase();
  if (isNaN(h) || h > 23 || isNaN(min) || min > 59) return [];
  const make = (hour: number, minute: number): SummarySuggestion => ({
    h: hour, m: minute, prefixLen,
    label: new Date(2024, 0, 1, hour, minute).toLocaleTimeString(navigator.language || 'en', { hour: '2-digit', minute: '2-digit' }),
  });
  if (ampm === 'am') return h <= 12 ? [make(h === 12 ? 0 : h, min)] : [];
  if (ampm === 'pm') return h <= 12 ? [make(h === 12 ? 12 : h + 12, min)] : [];
  if (h > 12 || h === 0 || h === 12) return [make(h, min)];
  return [make(h, min), make(h + 12, min)];
}

function daySuggestions(input: string, locale: string, today: Date): SummarySuggestion[] {
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  // Jan 7 2024 is a Sunday (dayIndex 0) … Jan 13 a Saturday (6); -1 today, -2 tomorrow.
  const names: Array<{ name: string; dayIndex: number }> = [
    { name: rtf.format(0, 'day').toLowerCase(), dayIndex: -1 },
    { name: rtf.format(1, 'day').toLowerCase(), dayIndex: -2 },
  ];
  for (let i = 0; i < 7; i++) {
    const date = new Date(2024, 0, 7 + i);
    const long = date.toLocaleDateString(locale, { weekday: 'long' }).toLowerCase();
    const short = date.toLocaleDateString(locale, { weekday: 'short' }).toLowerCase().replace(/\.$/, '');
    names.push({ name: long, dayIndex: i });
    if (short !== long) names.push({ name: short, dayIndex: i });
  }
  const lower = input.toLowerCase();
  const space = lower.search(/[ \t]/);
  const wordLen = space < 0 ? lower.length : space;
  const typed = lower.slice(0, wordLen);
  if (!typed || (space >= 0 && lower.slice(space + 1).trimStart().length > 0)) return [];

  const seen = new Set<number>();
  const out: SummarySuggestion[] = [];
  for (const { name, dayIndex } of names) {
    if (!name.startsWith(typed)) continue;
    if (dayIndex >= 0) {
      if (seen.has(dayIndex)) continue;
      seen.add(dayIndex);
    }
    let ahead = 0;
    if (dayIndex === -2) ahead = 1;
    else if (dayIndex >= 0) ahead = (dayIndex - today.getDay() + 7) % 7 || 7;
    const target = new Date(today.getFullYear(), today.getMonth(), today.getDate() + ahead);
    out.push({
      date: localDateIso(target), prefixLen: wordLen,
      label: target.toLocaleDateString(navigator.language || 'en', { weekday: 'long', month: 'short', day: 'numeric' }),
    });
  }
  return out;
}

/** Time suggestions for a leading time, else day suggestions for a leading day word. */
export function summarySuggestions(summary: string, locale: string, now: Date): SummarySuggestion[] {
  const times = timeSuggestions(summary);
  return times.length ? times : daySuggestions(summary, locale, now);
}

/** The draft with the suggestion applied: the prefix leaves the summary and sets the time or the day. */
export function applySummarySuggestion(d: EventDraft, s: SummarySuggestion): EventDraft {
  const next = { ...d, summary: d.summary.slice(s.prefixLen).trimStart() };
  if (s.date) next.date = s.date;
  if (s.h != null) {
    const pad = (n: number) => String(n).padStart(2, '0');
    next.is_all_day = false;
    next.start_time = pad(s.h) + ':' + pad(s.m ?? 0);
    next.end_time = halfHourAfter(next.start_time);
  }
  return next;
}

// ── Attendee suggestions from the contacts ──────────────────────────────────

export function attendeeSuggestions(contacts: readonly ContactJson[], query: string): Array<{ cn: string; email: string }> {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const out: Array<{ cn: string; email: string }> = [];
  for (const c of contacts) {
    const emailHit = c.emails.find((e) => e.value.toLowerCase().includes(q));
    if (!emailHit && !c.fn_name.toLowerCase().includes(q)) continue;
    const email = emailHit?.value ?? c.emails[0]?.value ?? '';
    if (email) out.push({ cn: c.fn_name, email });
    if (out.length === 8) break;
  }
  return out;
}

// ── Search page ─────────────────────────────────────────────────────────────

/** A submitted search: the results resource loads for it. `cals` are calendarKey()s; none means all. */
/** calendarKey()s of the calendars searched: the ones picked (`picked`, see urlCals()), by default the active ones. */
export function chosenCalendars(calendars: readonly CalendarJson[], picked: readonly string[] | null): string[] {
  return picked ?? calendars.filter((c) => c.is_active).map(calendarKey);
}

// ── The search in the URL ───────────────────────────────────────────────────
// The search page keeps its search in the URL (?q=…&start=…&end=…&cals=…), so a reload, the back button or a
// shared link shows it again, and the early file can request it before the page script runs. `cals` is left out
// for the default choice (the active calendars, `active=1` to the server, which knows them); `all` when every
// box is unchecked (search all, as before); else the chosen calendarKey()s, one per line.

type UrlParams = Record<string, string | undefined>;

/** The calendar choice in the URL: null for the default (active calendars), [] for all, else the keys. */
export function urlCals(url: UrlParams): string[] | null {
  if (!url.cals) return null;
  return url.cals === 'all' ? [] : url.cals.split('\n');
}

/** The `cals` URL parameter for a choice: null when it is the default one. */
export function calsParam(picked: readonly string[] | null, calendars: readonly CalendarJson[]): string | null {
  if (picked === null) return null;
  const active = calendars.filter((c) => c.is_active).map(calendarKey);
  if (picked.length === active.length && picked.every((k) => active.includes(k))) return null;
  return picked.length ? picked.join('\n') : 'all';
}

/** The request of the search in the URL; `q` is '' without one (no request). */
export function urlSearch(url: UrlParams): { q: string; start: string | null; end: string | null; cals: string[]; active: '1' | null } {
  const picked = urlCals(url);
  return { q: (url.q ?? '').trim(), start: url.start || null, end: url.end || null, cals: picked ?? [], active: picked === null ? '1' : null };
}

/** The calendar menu's button: "Calendars", with "(chosen/all)" while some are left out. */
export function calendarsLabel(chosen: number, all: number): string {
  return !all || chosen === all ? m.search_calendars() : `${m.search_calendars()} (${chosen}/${all})`;
}

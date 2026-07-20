import { m } from '../paraglide/messages.js';
import type { AlarmFields, EventJson, RecurrenceInfo, PartStat } from '../types/calendar.ts';

export const PARTSTAT_KEYS: Record<PartStat, string> = {
  'NEEDS-ACTION': 'event_attendeeStatusNeedsAction',
  'ACCEPTED': 'event_attendeeStatusAccepted',
  'DECLINED': 'event_attendeeStatusDeclined',
  'TENTATIVE': 'event_attendeeStatusTentative',
};

export function partstatKey(ps: PartStat): string {
  return PARTSTAT_KEYS[ps] ?? 'event_attendeeStatusNeedsAction';
}

export interface EventTimeText {
  localText: string;
  sourceText: string;
}

function browserTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
  } catch {
    return 'UTC';
  }
}

function supportsTimeZone(tz: string): boolean {
  if (!tz) return false;
  try {
    new Intl.DateTimeFormat('en', { timeZone: tz }).format(new Date());
    return true;
  } catch {
    return false;
  }
}

function hasExplicitOffset(value: string): boolean {
  return /(?:Z|[+-]\d{2}:\d{2})$/.test(value);
}

function dateFromEventValue(value: string): Date | null {
  if (!value) return null;
  const d = new Date(hasExplicitOffset(value) ? value : value + 'Z');
  return isNaN(d.getTime()) ? null : d;
}

function formatDatePart(date: Date, loc: string, timeZone?: string): string {
  return new Intl.DateTimeFormat(loc, { dateStyle: 'long', ...(timeZone ? { timeZone } : {}) }).format(date);
}

function formatTimePart(date: Date, loc: string, timeZone?: string): string {
  return new Intl.DateTimeFormat(loc, {
    hour: '2-digit',
    minute: '2-digit',
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}

function formatDateTimeRange(start: Date, end: Date | null, loc: string, timeZone?: string): string {
  let text = formatDatePart(start, loc, timeZone) + ', ' + formatTimePart(start, loc, timeZone);
  if (end) {
    const startDate = formatDatePart(start, loc, timeZone);
    const endDate = formatDatePart(end, loc, timeZone);
    text += startDate === endDate
      ? ' – ' + formatTimePart(end, loc, timeZone)
      : ' – ' + endDate + ', ' + formatTimePart(end, loc, timeZone);
  }
  return text;
}

export function formatEventTimes(ev: EventJson): EventTimeText {
  const loc = navigator.language || 'en';
  if (ev.is_all_day) {
    if (ev.dtstart) {
      const [y, monthNum, d] = ev.dtstart.split('T')[0].split('-').map(Number);
      return {
        localText: m.event_allDay() + ' · ' + new Date(y, monthNum - 1, d).toLocaleDateString(loc, { dateStyle: 'long' }),
        sourceText: '',
      };
    }
    return { localText: m.event_allDay(), sourceText: '' };
  }
  if (!ev.dtstart) return { localText: '', sourceText: '' };

  const eventTz = ev.tz || '';
  // The backend always resolves `tz` and its `dtstart`/`dtend` offsets
  // together (see caldav::parsing::resolve_tzid): whenever `tz` isn't
  // something Intl can format, the timestamp itself still carries a
  // correct explicit UTC offset (or `Z` for a genuine UTC/floating-time
  // event, in which case `tz` is reported as `'UTC'`). So the value can
  // always be parsed at face value — `tz` only affects how the "source
  // timezone" line below is rendered.
  const supportedEventTz = supportsTimeZone(eventTz);
  const s = dateFromEventValue(ev.dtstart);
  if (!s) return { localText: '', sourceText: '' };

  const e = ev.dtend ? dateFromEventValue(ev.dtend) : null;
  const localText = formatDateTimeRange(s, e, loc);

  let sourceText = '';
  if (eventTz && eventTz !== 'UTC' && eventTz !== browserTimeZone()) {
    if (supportedEventTz) {
      sourceText = formatDateTimeRange(s, e, loc, eventTz) + ' ' + eventTz;
    } else {
      sourceText = formatDateTimeRange(s, e, loc) + ' ' + eventTz;
    }
  }

  return { localText, sourceText };
}

export function formatEventTime(ev: EventJson): string {
  return formatEventTimes(ev).localText;
}

export function formatRecurrence(r: RecurrenceInfo | null, locale: string): string {
  if (!r?.freq) return '';
  const loc = locale;
  const singular: Record<string, () => string> = { DAILY: m.rec_everyDay, WEEKLY: m.rec_everyWeek, MONTHLY: m.rec_everyMonth, YEARLY: m.rec_everyYear };
  const plural: Record<string, (p: Record<string, string | number>) => string> = { DAILY: m.rec_everyNDays, WEEKLY: m.rec_everyNWeeks, MONTHLY: m.rec_everyNMonths, YEARLY: m.rec_everyNYears };
  const dayMap: Record<string, number> = { MO: 0, TU: 1, WE: 2, TH: 3, FR: 4, SA: 5, SU: 6 };
  const interval = r.interval && r.interval > 1 ? r.interval : 1;
  let text = interval === 1 ? singular[r.freq]() : plural[r.freq]({ n: interval });
  if (r.by_day?.length && r.freq === 'WEEKLY') {
    const dtf = new Intl.DateTimeFormat(loc, { weekday: 'long' });
    const ref = new Date(2024, 0, 1);
    text += m.rec_on() + r.by_day.map(c => { ref.setDate(1 + (dayMap[c] ?? 0)); return dtf.format(ref); }).join(', ');
  }
  if (r.by_month_day?.length && r.freq === 'MONTHLY') text += m.rec_onDay() + r.by_month_day.join(', ');
  if (r.count) text += ', ' + r.count + m.rec_times();
  else if (r.until) {
    const parts = r.until.split(' ')[0].split('-').map(Number);
    const d = parts.length === 3 ? new Date(parts[0], parts[1] - 1, parts[2]) : new Date(r.until + 'T00:00:00');
    if (!isNaN(d.getTime())) text += m.rec_until() + d.toLocaleDateString(loc, { dateStyle: 'long' });
  }
  return text;
}

export function formatAlarm(alarm: AlarmFields): string {
  const loc = navigator.language || 'en';
  if (alarm.direction === 'absolute') {
    if (!alarm.datetime) return '';
    const iso = alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
    const d = new Date(iso);
    return isNaN(d.getTime()) ? alarm.datetime : d.toLocaleString(loc, { dateStyle: 'medium', timeStyle: 'short' });
  }
  const abs = parseInt(alarm.value || '15') * parseInt(alarm.unit || '1');
  let vs = '';
  if (abs % 1440 === 0) { const n = abs / 1440; vs = n + ' ' + (n === 1 ? m.event_reminderDay() : m.event_reminderDays()); }
  else if (abs % 60 === 0) { const n = abs / 60; vs = n + ' ' + (n === 1 ? m.event_reminderHour() : m.event_reminderHours()); }
  else vs = abs + ' ' + m.event_reminderMin();
  const amap: Record<string, string> = { 'before-start': 'event_reminderBeforeStart', 'after-start': 'event_reminderAfterStart', 'before-end': 'event_reminderBeforeEnd', 'after-end': 'event_reminderAfterEnd' };
  return vs + ' ' + (m as Record<string, Function>)[amap[alarm.direction] || 'event_reminderBeforeStart']();
}

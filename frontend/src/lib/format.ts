import { m } from '@tiaforge-ui/sinter/messages';
import type { AlarmFields, EventJson, RecurrenceInfo, PartStat } from '../types/calendar.ts';
import { browserTimeZone as zoneOfBrowser, dateFormat } from './date-format.ts';

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
  return zoneOfBrowser() || 'UTC';
}

function supportsTimeZone(tz: string): boolean {
  if (!tz) return false;
  try {
    dateFormat('en', { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

export function dateFromEventValue(value: string): Date | null {
  if (!value) return null;
  // Anchored events carry an explicit offset (or `Z`); floating events arrive
  // as a naive `YYYY-MM-DDTHH:MM:SS` string, which `new Date` parses as local
  // time — exactly the RFC 5545 floating-time contract.
  const d = new Date(value);
  return isNaN(d.getTime()) ? null : d;
}

/// Browser-local Date of an event's start (all-day → local midnight).
export function eventStartDate(ev: Pick<EventJson, 'dtstart' | 'is_all_day'>): Date | null {
  if (!ev.dtstart) return null;
  if (ev.is_all_day) {
    const [y, mo, d] = ev.dtstart.split('T')[0].split('-').map(Number);
    return new Date(y, mo - 1, d);
  }
  return dateFromEventValue(ev.dtstart);
}

export function localDateIso(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/// Short browser-local start-time label for calendar grid chips.
export function eventTimeLabel(ev: EventJson): string {
  if (ev.is_all_day) return m.event_allDay();
  const d = eventStartDate(ev);
  if (!d) return '';
  return dateFormat(navigator.language || 'en', { hour: '2-digit', minute: '2-digit' }).format(d);
}

function formatDatePart(date: Date, loc: string, timeZone?: string): string {
  return dateFormat(loc, { dateStyle: 'long', ...(timeZone ? { timeZone } : {}) }).format(date);
}

function formatTimePart(date: Date, loc: string, timeZone?: string): string {
  return dateFormat(loc, { hour: '2-digit', minute: '2-digit', ...(timeZone ? { timeZone } : {}) }).format(date);
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
        localText: m.event_allDay() + ' · ' + dateFormat(loc, { dateStyle: 'long' }).format(new Date(y, monthNum - 1, d)),
        sourceText: '',
      };
    }
    return { localText: m.event_allDay(), sourceText: '' };
  }
  if (!ev.dtstart) return { localText: '', sourceText: '' };

  const eventTz = ev.tz || '';
  // The backend resolves `tz` and the `dtstart`/`dtend` offsets together
  // (see caldav::parsing::resolve_tzid): anchored events always carry a
  // correct explicit UTC offset (or `Z`, with `tz` reported as `'UTC'`),
  // while floating events arrive as naive strings with `tz === ''` and are
  // parsed as viewer-local wall clock. So the value can always be parsed at
  // face value — `tz` only affects how the "source timezone" line below is
  // rendered.
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
    const dtf = dateFormat(loc, { weekday: 'long' });
    const ref = new Date(2024, 0, 1);
    text += m.rec_on() + r.by_day.map(c => { ref.setDate(1 + (dayMap[c] ?? 0)); return dtf.format(ref); }).join(', ');
  }
  if (r.by_month_day?.length && r.freq === 'MONTHLY') text += m.rec_onDay() + r.by_month_day.join(', ');
  if (r.count) text += ', ' + r.count + m.rec_times();
  else if (r.until) {
    const parts = r.until.split(' ')[0].split('-').map(Number);
    const d = parts.length === 3 ? new Date(parts[0], parts[1] - 1, parts[2]) : new Date(r.until + 'T00:00:00');
    if (!isNaN(d.getTime())) text += m.rec_until() + dateFormat(loc, { dateStyle: 'long' }).format(d);
  }
  return text;
}

export function formatAlarm(alarm: AlarmFields): string {
  const loc = navigator.language || 'en';
  if (alarm.direction === 'absolute') {
    if (!alarm.datetime) return '';
    const iso = alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
    const d = new Date(iso);
    return isNaN(d.getTime()) ? alarm.datetime : dateFormat(loc, { dateStyle: 'medium', timeStyle: 'short' }).format(d);
  }
  const abs = parseInt(alarm.value || '15') * parseInt(alarm.unit || '1');
  let vs = '';
  if (abs % 1440 === 0) { const n = abs / 1440; vs = n + ' ' + (n === 1 ? m.event_reminderDay() : m.event_reminderDays()); }
  else if (abs % 60 === 0) { const n = abs / 60; vs = n + ' ' + (n === 1 ? m.event_reminderHour() : m.event_reminderHours()); }
  else vs = abs + ' ' + m.event_reminderMin();
  const when: Record<string, () => string> = { 'after-start': () => m.event_reminderAfterStart(), 'before-end': () => m.event_reminderBeforeEnd(), 'after-end': () => m.event_reminderAfterEnd() };
  return vs + ' ' + (when[alarm.direction] ?? (() => m.event_reminderBeforeStart()))();
}

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

export function formatEventTime(ev: EventJson): string {
  const loc = navigator.language || 'en';
  if (ev.is_all_day) {
    if (ev.dtstart) {
      const [y, monthNum, d] = ev.dtstart.split('T')[0].split('-').map(Number);
      return m.event_allDay() + ' · ' + new Date(y, monthNum - 1, d).toLocaleDateString(loc, { dateStyle: 'long' });
    }
    return m.event_allDay();
  }
  if (!ev.dtstart) return '';
  const s = new Date(ev.dtstart.includes('Z') || ev.dtstart.includes('+') ? ev.dtstart : ev.dtstart + 'Z');
  let text = s.toLocaleDateString(loc, { dateStyle: 'long' }) + ', ' +
    s.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
  if (ev.dtend) {
    const e = new Date(ev.dtend.includes('Z') || ev.dtend.includes('+') ? ev.dtend : ev.dtend + 'Z');
    if (s.toDateString() === e.toDateString()) {
      text += ' – ' + e.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
    } else {
      text += ' – ' + e.toLocaleDateString(loc, { dateStyle: 'long' }) + ', ' +
        e.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
    }
  }
  if (ev.tz && ev.tz !== 'UTC') text += ' (' + ev.tz + ')';
  return text;
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

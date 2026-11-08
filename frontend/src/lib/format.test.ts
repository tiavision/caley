import '../test-messages.ts';
import { describe, expect, it } from 'vitest';
import { formatEventTimes, eventStartDate, localDateIso, eventTimeLabel } from './format.ts';
import type { EventJson } from '../types/calendar.ts';

function event(overrides: Partial<EventJson>): EventJson {
  return {
    account_id: 'acct',
    uid: 'uid',
    time_range: '14:00-15:00',
    summary: 'Appointment',
    color: '#123456',
    dtstart: '2026-06-10T14:00:00+02:00',
    dtend: '2026-06-10T15:00:00+02:00',
    is_all_day: false,
    tz: 'Europe/Berlin',
    location: '',
    description: '',
    is_recurring: false,
    recurrence: null,
    calendar_href: '/cal/',
    master_dtstart: '',
    alarms: [],
    attendees: [],
    ...overrides,
  };
}

function timeText(date: Date, timeZone?: string): string {
  return new Intl.DateTimeFormat(navigator.language || 'en', {
    hour: '2-digit',
    minute: '2-digit',
    ...(timeZone ? { timeZone } : {}),
  }).format(date);
}

describe('formatEventTimes', () => {
  it('formats explicit instants in the browser local timezone', () => {
    const start = new Date('2026-06-10T12:00:00Z');
    const formatted = formatEventTimes(event({
      dtstart: '2026-06-10T12:00:00Z',
      dtend: '2026-06-10T13:00:00Z',
      tz: 'UTC',
    }));

    expect(formatted.localText).toContain(timeText(start));
    expect(formatted.sourceText).toBe('');
  });

  it('shows a valid different source timezone separately', () => {
    const start = new Date('2026-06-10T18:00:00Z');
    const formatted = formatEventTimes(event({
      dtstart: '2026-06-10T14:00:00-04:00',
      dtend: '2026-06-10T15:00:00-04:00',
      tz: 'America/New_York',
    }));

    expect(formatted.localText).toContain(timeText(start));
    expect(formatted.sourceText).toContain(timeText(start, 'America/New_York'));
    expect(formatted.sourceText).toContain('America/New_York');
  });

  it('does not duplicate the source time when the event timezone is local', () => {
    const localTz = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
    const formatted = formatEventTimes(event({ tz: localTz }));

    expect(formatted.localText).not.toBe('');
    expect(formatted.sourceText).toBe('');
  });

  it('parses the explicit offset even when the tz label is not Intl-resolvable', () => {
    // The backend resolves TZID via chrono-tz / a Windows-name table / the
    // event's own VTIMEZONE block before formatting, so anchored events
    // always carry a correct explicit UTC offset even when `tz` itself is a
    // raw, non-IANA label (e.g. a custom VTIMEZONE with no IANA equivalent).
    // (Floating events instead arrive naive with `tz === ''` — see below.)
    const start = new Date('2026-06-10T14:00:00Z');
    const formatted = formatEventTimes(event({
      dtstart: '2026-06-10T16:00:00+02:00',
      dtend: '2026-06-10T17:00:00+02:00',
      tz: 'Custom Zone',
    }));

    expect(formatted.localText).toContain(timeText(start));
    expect(formatted.sourceText).toContain(timeText(start));
    expect(formatted.sourceText).toContain('Custom Zone');
  });

  it('leaves all-day events without a source timezone line', () => {
    const formatted = formatEventTimes(event({
      dtstart: '2026-06-10',
      dtend: '2026-06-11',
      is_all_day: true,
      tz: 'America/New_York',
    }));

    expect(formatted.localText).toContain('All day');
    expect(formatted.sourceText).toBe('');
  });

  it('renders floating events at face value with no source line', () => {
    // Naive string + empty tz = floating time: shown as the same wall clock
    // in every viewer timezone.
    const wall = new Date(2026, 5, 10, 9, 0, 0);
    const formatted = formatEventTimes(event({
      dtstart: '2026-06-10T09:00:00',
      dtend: '2026-06-10T10:00:00',
      tz: '',
    }));

    expect(formatted.localText).toContain(timeText(wall));
    expect(formatted.sourceText).toBe('');
  });
});

describe('eventStartDate / localDateIso', () => {
  it('buckets an offset event into the browser-local date', () => {
    const ev = event({ dtstart: '2026-06-10T23:30:00-05:00' });
    const d = eventStartDate(ev)!;
    expect(localDateIso(d)).toBe(localDateIso(new Date('2026-06-11T04:30:00Z')));
  });

  it('keeps all-day events on their literal date', () => {
    const ev = event({ dtstart: '2026-06-10', is_all_day: true });
    expect(localDateIso(eventStartDate(ev)!)).toBe('2026-06-10');
  });

  it('keeps floating events on their wall-clock date', () => {
    const ev = event({ dtstart: '2026-06-10T23:30:00', tz: '' });
    expect(localDateIso(eventStartDate(ev)!)).toBe('2026-06-10');
  });
});

describe('eventTimeLabel', () => {
  it('shows the browser-local start time for anchored events', () => {
    const ev = event({ dtstart: '2026-06-10T14:00:00+02:00' });
    expect(eventTimeLabel(ev)).toBe(timeText(new Date('2026-06-10T12:00:00Z')));
  });

  it('shows the wall-clock time for floating events', () => {
    const ev = event({ dtstart: '2026-06-10T09:00:00', tz: '' });
    expect(eventTimeLabel(ev)).toBe(timeText(new Date(2026, 5, 10, 9, 0, 0)));
  });

  it('labels all-day events', () => {
    const ev = event({ dtstart: '2026-06-10', is_all_day: true });
    expect(eventTimeLabel(ev)).not.toBe('');
  });
});

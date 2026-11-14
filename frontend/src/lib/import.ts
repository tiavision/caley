// ICS import (src/components/import-dialog.html): the dialog's draft and pure formatting. The work is in src/actions/ics.ts.
import { m } from '@tiaforge-ui/sinter/messages';
import type { CalDavAccountJson, CalendarJson } from '../types/calendar.ts';

/** The calendar select's value for "new calendar". */
export const NEW_CALENDAR = '__new__';

/** Where an ICS attachment came from, so the server can link the imported events to the message. */
export interface ImportContext {
  account_id: string;
  mailbox: string;
  uid: number;
}

/** POST /api/import/preview: one row per event in the file. */
export interface ImportPreviewEvent {
  uid: string;
  summary: string;
  dtstart: string;
  dtend: string;
  is_all_day: boolean;
  location: string;
}

/** POST /api/import (`account_id`, `calendar_href`: the calendar the events went to, a new one for
 *  `new_calendar_name`), plus `calendar`: calendarValue() of it. */
export interface ImportResult {
  account_id: string;
  calendar_href: string;
  imported: number;
  errors: number;
  calendar: string;
}

/** The import dialog's state: `target` is calendarValue() of the chosen calendar or NEW_CALENDAR. */
export interface ImportDraft {
  file: File | null;
  ctx: ImportContext | null;
  target: string;
  accountId: string;
  newName: string;
  preview: ImportPreviewEvent[];
  error: string;
  success: string;
  done: boolean;
}

/** The calendar select's option value, and the remembered default calendar (`defaultCal`, shared with the event form). */
export function calendarValue(cal: Pick<CalendarJson, 'account_id' | 'href'>): string {
  return `${cal.account_id}\0${cal.href}`;
}

/** A fresh draft: the remembered default calendar (`defaultCal`) while it is active, else the first active one, else a new one. */
export function importDraft(
  accounts: CalDavAccountJson[],
  stored: string | null,
  file: File | null = null,
  ctx: ImportContext | null = null,
): ImportDraft {
  const calendars = accounts.flatMap((a) => a.calendars);
  const storedCal = stored ? calendars.find((c) => calendarValue(c) === stored && c.is_active) : undefined;
  const cal = storedCal ?? calendars.find((c) => c.is_active) ?? calendars[0];
  return {
    file,
    ctx,
    target: cal ? calendarValue(cal) : NEW_CALENDAR,
    accountId: cal?.account_id ?? accounts[0]?.account_id ?? '',
    newName: '',
    preview: [],
    error: '',
    success: '',
    done: false,
  };
}

/** "2026-11-08 09:00 - 2026-11-08 10:00", or the date alone for all-day events. */
export function importEventDate(event: Pick<ImportPreviewEvent, 'dtstart' | 'dtend' | 'is_all_day'>): string {
  if (!event.dtstart) return '';
  if (event.is_all_day) return event.dtstart;
  const start = event.dtstart.replace('T', ' ').slice(0, 16);
  const end = event.dtend ? event.dtend.replace('T', ' ').slice(0, 16) : '';
  return end ? `${start} - ${end}` : start;
}

/** The import's success line: how many events, and how many failed. */
export function importSummary(r: { imported: number; errors: number }): string {
  return m.cal_importSuccess({ n: r.imported }) + (r.errors > 0 ? ` · ${m.cal_importPartial({ n: r.errors })}` : '');
}

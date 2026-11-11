// `ics` actions: the ICS import dialog (src/components/import-dialog.html) reads the chosen file and posts it.
// The remembered calendar is page state (`defaultCal`, persisted), not handled here.
import type { CallContext } from '@tiaforge-ui/sinter';
import type { CalDavAccountJson, CalendarJson } from '../types/calendar.ts';
import {
  NEW_CALENDAR,
  calendarValue,
  importDraft,
  type ImportContext,
  type ImportDraft,
  type ImportPreviewEvent,
  type ImportResult,
} from '../lib/import.ts';

/**
 * A fresh draft for an action that builds one (mail: the attachment's File). Markup without a file uses the
 * helper importDraft(caldavAccounts, defaultCal, null, null) directly. Pass the page's `defaultCal`.
 */
export function draft(accounts: CalDavAccountJson[], file: File | null, ctx: ImportContext | null, defaultCal = ''): ImportDraft {
  return importDraft(accounts, defaultCal, file, ctx);
}

/** The events in the file, as the server reads them. */
export async function preview(file: File, ctx: CallContext): Promise<ImportPreviewEvent[]> {
  const body = new FormData();
  body.append('file', file);
  return (await ctx.request('/api/import/preview', { method: 'POST', body })) as ImportPreviewEvent[];
}

/** Imports the file into the chosen calendar (creating it first for "new calendar"); `calendar` is where it went. */
export async function run(d: ImportDraft, ctx: CallContext): Promise<ImportResult> {
  if (!d.file) throw new Error('no file');
  let [accountId, href] = d.target.split('\0') as [string, string];
  if (d.target === NEW_CALENDAR) {
    const cal = (await ctx.request('/api/calendar', {
      method: 'POST',
      body: new URLSearchParams({ account_id: d.accountId, name: d.newName.trim() }),
    })) as CalendarJson;
    accountId = cal.account_id;
    href = cal.href;
  }
  const body = new FormData();
  body.append('file', d.file);
  body.append('account_id', accountId);
  body.append('calendar_href', href);
  if (d.ctx) {
    body.append('email_account_id', d.ctx.account_id);
    body.append('email_mailbox', d.ctx.mailbox);
    body.append('email_uid', String(d.ctx.uid));
  }
  const result = (await ctx.request('/api/import', { method: 'POST', body })) as Omit<ImportResult, 'calendar'>;
  return { ...result, calendar: calendarValue({ account_id: accountId, href }) };
}

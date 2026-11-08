// `ics` actions: the ICS import dialog (partials/import-dialog.html) reads the chosen file and posts it.
import type { CallContext } from '@tiaforge-ui/sinter';
import type { CalDavAccountJson, CalendarJson } from '../types/calendar.ts';
import {
  NEW_CALENDAR,
  importDraft,
  type ImportContext,
  type ImportDraft,
  type ImportPreviewEvent,
  type ImportResult,
} from '../lib/import.ts';

// Shared with the event form (EventFormDialog): the calendar new events and imports go to.
const DEFAULT_CALENDAR_KEY = 'defaultCalendar';

function storedDefault(): string | null {
  try {
    return localStorage.getItem(DEFAULT_CALENDAR_KEY);
  } catch {
    return null;
  }
}

/** A fresh draft for the dialog, preselecting the remembered calendar; `file`/`ctx` for an attachment from mail. */
export function draft(accounts: CalDavAccountJson[], file: File | null, ctx: ImportContext | null): ImportDraft {
  return importDraft(accounts, storedDefault(), file, ctx);
}

/** The events in the file, as the server reads them. */
export async function preview(file: File, ctx: CallContext): Promise<ImportPreviewEvent[]> {
  const body = new FormData();
  body.append('file', file);
  return (await ctx.request('/api/import/preview', { method: 'POST', body })) as ImportPreviewEvent[];
}

/** Imports the file into the chosen calendar (creating it first for "new calendar"); remembers it on full success. */
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
  const result = (await ctx.request('/api/import', { method: 'POST', body })) as ImportResult;
  if (result.errors === 0) {
    try {
      localStorage.setItem(DEFAULT_CALENDAR_KEY, `${accountId}\0${href}`);
    } catch {
      // Without storage the next import preselects the first active calendar.
    }
  }
  return result;
}

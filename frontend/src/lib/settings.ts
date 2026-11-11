// Settings page helpers (settings/index.html, src/components/pgp-settings.html): pure, exported to templates through
// src/sinter.helpers.ts.
import { m } from '@tiaforge-ui/sinter/messages';
import type { CalDavAccountJson } from '../types/calendar.ts';

/** The CalDAV account dialog (POST /api/caldav/account); account_id '' adds one. */
export interface AccountDraft {
  account_id: string;
  display_name: string;
  caldav_base_url: string;
  caldav_username: string;
  caldav_password: string;
  has_password: boolean;
}

/** The dialog for `account`, or for a new account; the password stays empty (kept when left so). */
export function caldavDraft(account: CalDavAccountJson | null): AccountDraft {
  return {
    account_id: account?.account_id ?? '', display_name: account?.display_name || '', caldav_base_url: account?.caldav_base_url || '',
    caldav_username: account?.caldav_username || '', caldav_password: '', has_password: !!account?.has_password,
  };
}

/** POST /api/caldav/account (form encoded). */
export function caldavBody(d: AccountDraft) {
  return { account_id: d.account_id || null, display_name: d.display_name, caldav_base_url: d.caldav_base_url, caldav_username: d.caldav_username, caldav_password: d.caldav_password };
}

/** The account's calendars with `href` switched on or off (the toggle's optimistic change). */
export function withCalendarActive(account: CalDavAccountJson, href: string, active: boolean): Pick<CalDavAccountJson, 'calendars'> {
  return { calendars: account.calendars.map((c) => (c.href === href ? { ...c, is_active: active } : c)) };
}

type ActionError = { detail?: string | null; message: string };

/** The private key import's error: a hint for Argon2-protected keys, else the server's or the library's message. */
export function keyImportError(error: ActionError): string {
  return (error.detail ?? '').toLowerCase().includes('argon2') ? m.pgp_importArgon2Error() : `${m.pgp_importError()}: ${error.detail ?? error.message}`;
}

/** The passkey enrollment's error: a hint when the authenticator has no PRF, else the message. */
export function enrollErrorText(error: ActionError): string {
  return error.detail === 'prf_not_supported' ? m.pgp_enrollNoPrf() : `${m.pgp_enrollError()}: ${error.detail ?? error.message}`;
}

// Types that `type="…"` on <sn-resource> and <sn-event> may name (`sinter check` runs in `vite build`).
// Re-export the app's existing types rather than redefining them.
export type { User } from './lib/user.ts';
export type { NotificationStatus } from './actions/notify.ts';
export type { ContactJson, AddressbookJson } from './types/calendar.ts';
export type { ContactDraft } from './lib/contacts.ts';
export type { ShareItem } from './lib/share.ts';
// Re-export each type by name (sinter reads the exported names; a star re-export of types is not seen).

/** GET /api/auth/config (public): what the login page offers. */
export interface AuthConfig {
  oidc_enabled: boolean;
}

/** contact_updated on /api/events/stream: `contact` is missing when the server could not send it. */
export interface ContactUpdatedEvent {
  account_id: string;
  uid: string;
  addressbook_href: string;
  contact?: import('./types/calendar.ts').ContactJson | null;
}

/** contact_removed on /api/events/stream. */
export interface ContactRemovedEvent {
  account_id: string;
  uid: string;
  addressbook_href: string;
}

// Event partials, search and calendar pages.
export type { EventJson, CalendarJson, CalDavAccountJson, SourceEmailJson, MessageEventGroupJson } from './types/calendar.ts';
export type { EventDraft, SearchQuery } from './lib/events.ts';
export type { ThreadRef, ThreadResponse } from './lib/thread.ts';

// Settings page, import dialog and encryption tab (CalDavAccountJson is exported with the event types above).
export type { ImportDraft } from './lib/import.ts';
export type { VaultView } from './actions/pgp-vault.ts';
export type { StoredPublicKey } from './lib/pgp/public-keys.ts';

/** The CalDAV account dialog on the settings page (POST /api/caldav/account); account_id '' adds one. */
export interface AccountDraft {
  account_id: string;
  display_name: string;
  caldav_base_url: string;
  caldav_username: string;
  caldav_password: string;
  has_password: boolean;
}

// Mail pages (mail/, mail/view/, partials/mail-*.html).
export type {
  Account, Identity, Mailbox, VirtualFolder, Message, Attachment, MessageRef, MailView, UnreadCounts,
  MailboxEvent, AccountEvent, EmailUpdatedEvent, EmailRemovedEvent, SearchFilter, ComposeDraft, ComposeKeys,
  ContactAddresses, ImageOrigins, AccountForm, Autodiscovery, IdentityForm, FolderDraft,
} from './lib/mail.ts';
export type { ReaderBody } from './actions/mail.ts';
export type { ThreadNode as MailThreadNode } from './lib/mail.ts';

// Calendar page and thread dialog.
export type { DayEventsJson, MonthGridJson } from './types/calendar.ts';
export type { ThreadNode } from './lib/MailTypes.ts';
export type { MonthFit } from './islands/calendar.ts';

/** event_removed on /api/events/stream. */
export interface EventRemovedEvent {
  account_id: string;
  uid: string;
  calendar_href: string;
}

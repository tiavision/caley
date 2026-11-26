// Types that `type="…"` on <sn-resource> and <sn-event> may name (`sinter check` runs in `vite build`).
// Re-export the app's existing types rather than redefining them.
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

// Event components, search and calendar pages.
export type { EventJson, CalendarJson, CalDavAccountJson, SourceEmailJson, MessageEventGroupJson } from './types/calendar.ts';
export type { EventDraft } from './lib/events.ts';
export type { ThreadRef, ThreadResponse } from './lib/thread.ts';

// Settings page, import dialog and encryption tab (CalDavAccountJson is exported with the event types above).
export type { ImportDraft, ImportResult } from './lib/import.ts';
export type { ImportPreviewRow } from './actions/ics.ts';
export type { VaultView } from './lib/pgp/vault-steps.ts';
export type { StoredPublicKey, PublicKeyBody } from './lib/pgp/public-keys.ts';
export type { KeyFacts, KeyCandidate, PendingKey } from './lib/pgp-keys.ts';

export type { AccountDraft } from './lib/settings.ts';

// Mail pages (mail/, mail/view/, src/components/mail-*.html).
export type {
  Account, Identity, Mailbox, VirtualFolder, Message, Attachment, MessageRef, MailView, UnreadCounts,
  MailboxEvent, AccountEvent, EmailUpdatedEvent, EmailRemovedEvent, SearchFilter, ComposeDraft, ComposeKeys,
  ContactAddresses, ImageOrigins, AccountForm, Autodiscovery, IdentityForm, FolderDraft, AccountUi,
  Reader, Selection, MailSearch, Drag,
} from './lib/mail.ts';
export type { ReaderBody } from './actions/mail.ts';
// sinter-mail's tree (email.parse) and its options.
export type { Body, Element, ParseOptions } from '@tiavision/sinter-mail';
export type { ThreadNode as MailThreadNode } from './lib/mail.ts';

// Calendar page and thread dialog.
export type { DayEventsJson, MonthGridJson } from './types/calendar.ts';
export type { ThreadNode } from './lib/MailTypes.ts';
export type { MonthFit } from './components/month-fit.ts';

/** event_removed on /api/events/stream. */
export interface EventRemovedEvent {
  account_id: string;
  uid: string;
  calendar_href: string;
}

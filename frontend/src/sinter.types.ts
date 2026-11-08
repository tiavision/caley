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

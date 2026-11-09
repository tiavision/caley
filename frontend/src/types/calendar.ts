export type PartStat = 'NEEDS-ACTION' | 'ACCEPTED' | 'DECLINED' | 'TENTATIVE';

export interface AlarmFields {
  direction: string;
  value: string;
  unit: string;
  datetime: string;
}

export interface AttendeeFields {
  cn: string;
  email: string;
  partstat: PartStat;
}

export interface RecurrenceInfo {
  freq: string;
  interval?: number;
  by_day?: string[];
  by_month_day?: number[];
  by_month?: number[];
  by_set_pos?: number[];
  until?: string | null;
  count?: number | null;
}

export interface EventJson {
  account_id: string;
  uid: string;
  time_range: string;
  summary: string;
  color: string;
  dtstart: string;
  dtend: string;
  is_all_day: boolean;
  /** IANA name or raw TZID label; '' means floating time (naive dtstart). */
  tz: string;
  location: string;
  description: string;
  is_recurring: boolean;
  recurrence: RecurrenceInfo | null;
  calendar_href: string;
  master_dtstart: string;
  alarms: AlarmFields[];
  attendees: AttendeeFields[];
}

export interface SourceEmailJson {
  account_id: string;
  mailbox: string;
  uid: number;
  from: string | null;
  subject: string | null;
  sent_at: number | null;
}

export interface MessageEventGroupJson {
  account_id: string;
  mailbox: string;
  uid: number;
  events: EventJson[];
}

export interface DayEventsJson {
  date_iso: string;
  day_name: string;
  is_today: boolean;
  events: EventJson[];
}

export interface CalendarJson {
  account_id: string;
  href: string;
  name: string;
  color: string | null;
  is_active: boolean;
}

export interface MonthDayJson {
  day_number: number;
  date_str: string;
  is_current_month: boolean;
  event_colors: string[];
}

export interface TypedEntry {
  value: string;
  type_label: string;
}

export interface AddressEntry {
  street: string;
  city: string;
  region: string;
  postal: string;
  country: string;
  type_label: string;
}

export interface ContactJson {
  account_id: string;
  uid: string;
  addressbook_href: string;
  fn_name: string;
  emails: TypedEntry[];
  phones: TypedEntry[];
  addresses: AddressEntry[];
  org: string;
  has_photo: boolean;
  /** the photo's version, for its URL (null without a photo) */
  photo_rev?: string | null;
}

export interface AddressbookJson {
  account_id: string;
  href: string;
  name: string;
}

export interface CalDavAccountJson {
  account_id: string;
  display_name: string;
  caldav_base_url: string;
  caldav_username: string;
  has_password: boolean;
  sort_order: number;
  calendars: CalendarJson[];
  addressbooks: AddressbookJson[];
}

export interface ContactsResponse {
  addressbooks: AddressbookJson[];
  contacts: ContactJson[];
}

export interface MonthGridJson {
  month: string;
  month_name: string;
  month_number: number;
  year: number;
  days: MonthDayJson[];
  prev_month: string;
  next_month: string;
}

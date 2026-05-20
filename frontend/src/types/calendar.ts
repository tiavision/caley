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
  uid: string;
  time_range: string;
  summary: string;
  color: string;
  dtstart: string;
  dtend: string;
  is_all_day: boolean;
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

export interface DayEventsJson {
  date_iso: string;
  day_name: string;
  is_today: boolean;
  events: EventJson[];
}

export interface CalendarJson {
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

export interface SearchResultItem {
  uid: string;
  calendar_href: string;
  summary: string;
  description: string;
  location: string;
  dtstart: string;
  calendar_name: string;
  calendar_color: string | null;
}

export interface ContactJson {
  uid: string;
  addressbook_href: string;
  fn_name: string;
  emails: string[];
  phones: string[];
  org: string;
}

export interface AddressbookJson {
  href: string;
  name: string;
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

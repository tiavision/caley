// Contacts page helpers (sinter helpers, so all pure): filtering, grouping, display and the form draft.
import { m } from '../paraglide/messages.js';
import type { AddressbookJson, AddressEntry, ContactJson, TypedEntry } from '../types/calendar.ts';

/** Identity of an address book, as the contacts filter stores it. */
export function addressbookKey(ab: AddressbookJson): string {
  return `${ab.account_id}\n${ab.href}`;
}

/** Identity of a contact's address book (see addressbookKey). */
export function contactAddressbookKey(c: ContactJson): string {
  return `${c.account_id}\n${c.addressbook_href}`;
}

type ContactId = Pick<ContactJson, 'account_id' | 'uid' | 'addressbook_href'>;

/** Whether two values name the same contact. */
export function sameContact(a: ContactId | null | undefined, b: ContactId | null | undefined): boolean {
  return !!a && !!b && a.account_id === b.account_id && a.uid === b.uid && a.addressbook_href === b.addressbook_href;
}

/** The address book menu's button: "Address books", with "(shown/all)" while some are hidden. */
export function addressbooksLabel(shown: number, all: number): string {
  return shown === all ? m.contacts_addressbooks() : `${m.contacts_addressbooks()} (${shown}/${all})`;
}

/** Contacts in the shown address books whose name, org, an email or a phone contain `q`. */
export function filterContacts(contacts: readonly ContactJson[], addressbooks: readonly AddressbookJson[], hidden: readonly string[], q: string): ContactJson[] {
  let list = [...contacts];
  if (addressbooks.length > 1 && hidden.length) list = list.filter((c) => !hidden.includes(contactAddressbookKey(c)));
  const needle = q.trim().toLowerCase();
  if (needle) {
    list = list.filter((c) =>
      c.fn_name.toLowerCase().includes(needle) ||
      c.emails.some((e) => e.value.toLowerCase().includes(needle)) ||
      c.org.toLowerCase().includes(needle) ||
      c.phones.some((p) => p.value.toLowerCase().includes(needle)));
  }
  return list;
}

/** Contacts grouped by the first letter of their name, letters in order. */
export function groupByLetter(contacts: readonly ContactJson[]): { letter: string; items: ContactJson[] }[] {
  const groups = new Map<string, ContactJson[]>();
  for (const c of contacts) {
    const letter = (c.fn_name[0] ?? '#').toUpperCase();
    if (!groups.has(letter)) groups.set(letter, []);
    groups.get(letter)!.push(c);
  }
  return [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([letter, items]) => ({ letter, items }));
}

/** Row key of a contact in a list. */
export function contactKey(c: ContactJson): string {
  return `${c.account_id}\n${c.addressbook_href}\n${c.uid}`;
}

/** Name of the contact's address book. */
export function addressbookName(addressbooks: readonly AddressbookJson[], c: ContactJson): string {
  return addressbooks.find((ab) => ab.account_id === c.account_id && ab.href === c.addressbook_href)?.name ?? '';
}

/** Avatar letter. */
export function contactInitial(c: ContactJson): string {
  return (c.fn_name[0] ?? '?').toUpperCase();
}

/** URL of the contact's photo (when `has_photo`). */
export function contactPhotoUrl(c: ContactJson): string {
  const params = new URLSearchParams({ account_id: c.account_id, uid: c.uid, addressbook_href: c.addressbook_href });
  return `/api/contact/photo?${params}`;
}

/** One-line query for a map search. */
export function addressQuery(a: AddressEntry): string {
  return [a.street, a.city, a.region, a.postal, a.country].filter(Boolean).join(', ');
}

/** An address as display lines: street, "city, region postal", country (empty lines left out). */
export function addressLines(a: AddressEntry): string[] {
  const place = a.city || a.region || a.postal
    ? `${a.city}${a.region ? (a.city ? ', ' : '') + a.region : ''}${a.postal ? ' ' + a.postal : ''}`
    : '';
  return [a.street, place, a.country].filter(Boolean);
}

/** The contact form's state: a copy of the contact, with an empty email and phone row to type into. */
export interface ContactDraft {
  uid: string | null;
  fn_name: string;
  emails: TypedEntry[];
  phones: TypedEntry[];
  addresses: AddressEntry[];
  org: string;
  /** addressbookKey of the target address book */
  addressbook: string;
}

export const emptyEntry = (): TypedEntry => ({ value: '', type_label: 'other' });
export const emptyAddress = (): AddressEntry => ({ street: '', city: '', region: '', postal: '', country: '', type_label: 'home' });

/** Draft for a new contact in the first address book. */
export function newDraft(addressbooks: readonly AddressbookJson[]): ContactDraft {
  const ab = addressbooks[0];
  return {
    uid: null, fn_name: '', emails: [emptyEntry()], phones: [emptyEntry()], addresses: [], org: '',
    addressbook: ab ? addressbookKey(ab) : '\n',
  };
}

/** Draft for editing a contact. */
export function editDraft(c: ContactJson): ContactDraft {
  return {
    uid: c.uid,
    fn_name: c.fn_name,
    emails: c.emails.length ? c.emails.map((e) => ({ ...e })) : [emptyEntry()],
    phones: c.phones.length ? c.phones.map((p) => ({ ...p })) : [emptyEntry()],
    addresses: c.addresses.map((a) => ({ ...a })),
    org: c.org,
    addressbook: contactAddressbookKey(c),
  };
}

/** POST /api/contact body: the draft without empty rows, trimmed. */
export function draftBody(d: ContactDraft) {
  const [account_id = '', addressbook_href = ''] = d.addressbook.split('\n');
  return {
    uid: d.uid ?? undefined,
    account_id,
    addressbook_href,
    fn_name: d.fn_name.trim(),
    emails: d.emails.filter((e) => e.value.trim()),
    phones: d.phones.filter((p) => p.value.trim()),
    addresses: d.addresses.filter((a) => a.street.trim() || a.city.trim() || a.country.trim()),
    org: d.org.trim(),
  };
}

/**
 * The key of a page shortcut, or null when the press belongs elsewhere: with a modifier, typed into a field,
 * or (unless `inDialog`) inside a dialog.
 */
export function shortcutKey(e: KeyboardEvent, inDialog = false): string | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  const t = e.target as HTMLElement | null;
  if (!t || t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || t instanceof HTMLSelectElement || t.isContentEditable) return null;
  if (!inDialog && t.closest('dialog')) return null;
  return e.key;
}

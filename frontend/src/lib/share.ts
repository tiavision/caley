import type { EventJson, ContactJson } from '../types/calendar.ts';

export async function fetchEventICS(event: EventJson): Promise<string> {
  const params = new URLSearchParams({
    account_id: event.account_id,
    calendar_href: event.calendar_href,
    uid: event.uid,
  });
  const res = await fetch(`/api/event/ics?${params}`);
  if (!res.ok) throw new Error('fetch failed');
  return res.text();
}

export async function fetchContactVCF(contact: ContactJson): Promise<string> {
  const params = new URLSearchParams({
    account_id: contact.account_id,
    addressbook_href: contact.addressbook_href,
    uid: contact.uid,
  });
  const res = await fetch(`/api/contact/vcf?${params}`);
  if (!res.ok) throw new Error('fetch failed');
  return res.text();
}

export function downloadFile(filename: string, content: string, mimeType: string): void {
  const blob = new Blob([content], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

export function composeEmailWithAttachment(subject: string, filename: string, mimeType: string, content: string): void {
  sessionStorage.setItem('caley_compose', JSON.stringify({ subject, filename, mimeType, content }));
  window.location.href = '/mail/';
}

export function safeFilename(name: string, ext: string): string {
  return (name.replace(/[^\w\- ]/g, '_').trim() || 'file') + ext;
}

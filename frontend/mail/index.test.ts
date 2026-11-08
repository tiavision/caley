import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, mockRequests, mockStream, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import nav from '../partials/nav.html?raw';
import navToggle from '../partials/nav-toggle.html?raw';
import reader from '../partials/mail-reader.html?raw';
import readerData from '../partials/mail-reader-data.html?raw';
import folderIcon from '../partials/mail-folder-icon.html?raw';
import origins from '../partials/mail-origins.html?raw';
import compose from '../partials/mail-compose.html?raw';
import account from '../partials/mail-account.html?raw';
import folder from '../partials/mail-folder.html?raw';
import importDialog from '../partials/import-dialog.html?raw';
import source from './index.html?raw';
import type { Message } from '../src/lib/mail.ts';

// The test page lives at the root of sinter's virtual project: its partials sit next to it.
const page = source.replaceAll('../partials/', './');
const files = {
  './nav.html': nav, './nav-toggle.html': navToggle, './mail-reader.html': reader, './mail-reader-data.html': readerData,
  './mail-folder-icon.html': folderIcon, './mail-origins.html': origins, './mail-compose.html': compose,
  './mail-account.html': account, './mail-folder.html': folder, './import-dialog.html': importDialog,
};

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };
const msg = (uid: number, subject: string, extra: Partial<Message> = {}): Message => ({
  uid, account_id: 'work', mailbox: 'INBOX', rowid: uid, message_id: `<${uid}@x>`, thread_id: null, from: `Sender ${uid} <s${uid}@x>`,
  to: ['me@x'], cc: [], subject, sent_at: 1_700_000_000 + uid, flags: ['\\Seen'], has_attachments: false, body_cached: true,
  body_text: `body ${uid}`, body_html: null, autocrypt_headers: [], is_read: true, is_flagged: false, is_encrypted: false,
  is_signed: false, attachments: [], ...extra,
});
const inbox = [msg(3, 'Third', { flags: [], is_read: false }), msg(2, 'Second'), msg(1, 'First')];

// jsdom has no modal dialogs or popovers: open/close set `open` and fire `toggle` as browsers do.
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}
if (!HTMLElement.prototype.hidePopover) HTMLElement.prototype.hidePopover = function () {};

beforeEach(() => { localStorage.clear(); configure({ confirm: () => true, urlSchemes: ['blob:'] }); });
afterEach(() => configure({ confirm: null as never, urlSchemes: [] }));

async function show(routes: Record<string, RouteResponse> = {}) {
  const opened: unknown[] = [];
  const listed = routes['GET /api/email/messages'];
  const current: Message[] = listed && typeof listed === 'object' && 'messages' in listed ? (listed as { messages: Message[] }).messages : inbox;
  const requests = mockRequests({
    'GET /api/user': user,
    'GET /api/email/accounts': { accounts: [{ account_id: 'work', display_name: 'Work', host: 'imap', port: 993, username: 'me@x', use_tls: true, smtp_host: null, smtp_port: null }] },
    'GET /api/email/mailboxes': { mailboxes: [
      { account_id: 'work', name: 'INBOX', delimiter: '/', subscribed: true },
      { account_id: 'work', name: 'Archive', delimiter: '/', subscribed: true },
      { account_id: 'work', name: 'Trash', delimiter: '/', subscribed: true },
    ] },
    'GET /api/email/folders': { folders: [] },
    'GET /api/email/unread-counts': { mailboxes: { work: { INBOX: 1 } }, folders: {} },
    'GET /api/email/identities': { identities: [{ identity_id: 'i1', account_id: 'work', name: 'Me', email: 'me@x', is_default: true, sort_order: 0 }] },
    'GET /api/email/image-origins': { origins: [] },
    'GET /api/email/image-blacklist': { origins: [] },
    'GET /api/email/messages': { messages: inbox, total: 3 },
    'GET /api/email/thread': { messages: [] },
    'GET /api/email/thread-events': [],
    'POST /api/email/sync': null,
    'GET /api/contacts': { addressbooks: [], contacts: [] },
    'GET /api/caldav/accounts': { accounts: [] },
    ...routes,
  });
  const stream = mockStream();
  const actions = {
    ics: { draft: () => null, preview: async () => [], run: async () => ({ imported: 0, errors: 0 }) },
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    mail: {
      open: async (ref: { uid: number }) => { opened.push(ref); return { ...(current.find((m) => m.uid === ref.uid) ?? inbox[0]), is_read: true }; },
      body: async (ref: { account_id: string; mailbox: string; uid: number }) => ({
        key: `${ref.account_id} ${ref.mailbox} ${ref.uid}`, html: '', text: `body ${ref.uid}`, mode: 'text', encrypted: false, signed: false,
        locked: false, badges: [], subject: '', error: '', files: [], importable: [], origins: [],
      }),
      bulk: async () => [],
      composeKeys: async () => ({ keys: [], vault: [], unlocked: [] }),
      send: async () => null,
      forwardFiles: async () => ({ files: [], html: '', preview: '' }),
      popout: () => {}, unlock: async () => true, importKey: async () => '', icsDraft: async (_m: unknown, filename: string) => ({ file: new File(['x'], filename), filename }),
      promoteOrigin: async () => null, saveFolder: async () => null,
    },
  };
  const r = await render(page, { requests, actions, helpers, files, streams: { '/api/events/stream': stream } });
  await r.settle();
  return { ...r, requests, stream, opened };
}

const rows = (root: HTMLElement) => [...root.querySelectorAll<HTMLButtonElement>('.message-row')];
const subjects = (root: HTMLElement) => rows(root).map((r) => r.querySelector('.subject')!.textContent);

test('shows the first inbox with its unread count, then opens a message in the reader', async () => {
  const { root, settle, opened } = await show();
  expect(subjects(root)).toEqual(['Third', 'Second', 'First']);
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Inbox');
  expect(root.querySelector('.sidebar-item[aria-current] [data-badge]')!.textContent).toBe('1');
  expect(rows(root)[0].classList.contains('unread')).toBe(true);

  rows(root)[0].click();
  await settle();
  expect(opened).toEqual([{ account_id: 'work', mailbox: 'INBOX', uid: 3 }]);
  expect(root.querySelector('.reader .detail-subject')!.textContent).toBe('Third');
  expect(root.querySelector('.reader .body-text')!.textContent).toBe('body 3');
  expect(rows(root)[0].classList.contains('unread')).toBe(false);
  expect(rows(root)[0].getAttribute('aria-current')).toBe('true');
});

test('searches the folder with filters, and pages by cursor', async () => {
  const { root, requests, settle, advance } = await show({
    'GET /api/email/search': { messages: [msg(2, 'Second')] },
    'GET /api/email/messages': (call) => {
      const q = new URL(call.url, 'http://x').searchParams;
      return q.get('direction') === 'forward' && q.get('cursor_rowid') ? { messages: [], total: 3 } : { messages: inbox, total: 3 };
    },
  });
  const search = root.querySelector<HTMLInputElement>('.message-list-header > input[type=search]')!;
  search.value = 'sec';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  await advance(300);
  await settle();
  expect(subjects(root)).toEqual(['Second']);
  expect(requests.calls.at(-1)!.url).toBe('/api/email/search?q=sec&account_id=work&mailbox=INBOX&limit=50&direction=forward');

  root.querySelector<HTMLButtonElement>('button[aria-label="Unread only"]')!.click();
  await settle();
  expect(requests.calls.at(-1)!.url).toContain('&unread_only=true');
});

test('marks rows with Ctrl and Shift, and runs a bulk action', async () => {
  const { root, settle } = await show();
  const r = rows(root);
  r[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  r[2].dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }));
  await settle();
  expect(rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true')).toHaveLength(3);
  expect(root.querySelector('.message-detail h2')!.textContent).toBe('3 messages selected');

  root.querySelector<HTMLButtonElement>('.bulk-menu button[data-icon="mail"]')!.click();
  await settle();
  expect(root.querySelectorAll('.message-row[aria-pressed="true"]')).toHaveLength(0);
});

test('server events: flags patch the list, a removed open message closes the reader', async () => {
  const { root, stream, settle } = await show();
  rows(root)[1].click();
  await settle();
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Seen', '\\Flagged'] });
  await settle();
  expect(rows(root)[1].querySelector('.flag-star')).not.toBeNull();
  expect(root.querySelector('.reader .star-btn')!.classList.contains('flagged')).toBe(true);

  stream.emit('email_removed', { account_id: 'work', mailbox: 'INBOX', uid: 2 });
  await settle();
  expect(subjects(root)).toEqual(['Third', 'First']);
  expect(root.querySelector('.reader')).toBeNull();
});

test('reply opens the compose dialog with the quoted message, and sends it', async () => {
  const { root, settle } = await show();
  rows(root)[2].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.reader [data-shortcut="reply"]')!.click();
  root.querySelector<HTMLDialogElement>('#mail-compose')!.showModal();
  await settle();
  expect(root.querySelector<HTMLInputElement>('#compose-subject')!.value).toBe('Re: First');
  expect(root.querySelector<HTMLInputElement>('#compose-to')!.value).toBe('Sender 1 <s1@x>');
  expect(root.querySelector<HTMLTextAreaElement>('.compose-body')!.value).toContain('> body 1');
  root.querySelector<HTMLFormElement>('.compose-form')!.requestSubmit();
  await settle();
  expect(root.querySelector<HTMLDialogElement>('#mail-compose')!.open).toBe(false);
});

test('a calendar attachment opens the import dialog with its draft', async () => {
  const invite = msg(1, 'First', { has_attachments: true, attachments: [{ filename: 'invite.ics', content_type: 'text/calendar', size: 10, content_id: null }] });
  const { root, settle } = await show({ 'GET /api/email/messages': { messages: [invite], total: 1 } });
  rows(root)[0].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.attachments button')!.click();
  await settle();
  expect(root.querySelector('.mail-attachment-chip')!.getAttribute('href')).toBe('/api/email/attachment?account_id=work&mailbox=INBOX&uid=1&filename=invite.ics');
  root.querySelector<HTMLButtonElement>('.attachments button[data-icon="calendar-plus"]')!.click();
  await settle();
  expect(root.querySelector<HTMLDialogElement>('#import-ics')!.open).toBe(true);
});

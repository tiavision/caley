import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../src/test-messages.ts';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, memoryStorage, mockRequests, mockStream, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import '../src/islands/common.ts';                 // data-shortcut keys
import page from './index.html?raw';
import { app } from '../src/test-components.ts';
import { retryPolicies } from '../src/lib/retry-policies.ts';
import type { Message } from '../src/lib/mail.ts';
import '../src/test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '' };
const msg = (uid: number, subject: string, extra: Partial<Message> = {}): Message => ({
  uid, account_id: 'work', mailbox: 'INBOX', rowid: uid, message_id: `<${uid}@x>`, thread_id: null, from: `Sender ${uid} <s${uid}@x>`,
  to: ['me@x'], cc: [], subject, sent_at: 1_700_000_000 + uid, flags: ['\\Seen'], has_attachments: false, body_cached: true,
  body_text: `body ${uid}`, body_html: null, autocrypt_headers: [], is_read: true, is_flagged: false, is_encrypted: false,
  is_signed: false, attachments: [], ...extra,
});
const inbox = [msg(3, 'Third', { flags: [], is_read: false }), msg(2, 'Second'), msg(1, 'First')];


// the app's retry policies: the flag toggles name one (data-sn-retry="flags")
beforeEach(() => { localStorage.clear(); configure({ confirm: () => true, urlSchemes: ['blob:'], retry: retryPolicies }); });
afterEach(() => configure({ confirm: null as never, urlSchemes: [], retry: null }));

async function show(routes: Record<string, RouteResponse> = {}, pending: { subject: string; files: File[] } | null = null, storage = memoryStorage()) {
  const opened: unknown[] = [];
  let pendingCalls = 0;
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
    ics: { preview: async () => [], run: async () => ({ imported: 0, errors: 0, calendar: '' }) },
    share: { native: () => false, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async () => {} },
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    mail: {
      open: async (ref: { uid: number }) => { opened.push(ref); return current.find((m) => m.uid === ref.uid) ?? inbox[0]; },
      body: async (ref: { account_id: string; mailbox: string; uid: number }) => ({
        key: `${ref.account_id} ${ref.mailbox} ${ref.uid}`, html: '', text: `body ${ref.uid}`, mode: 'text', encrypted: false, signed: false,
        locked: false, badges: [], subject: '', error: '', files: [], importable: {}, origins: [],
      }),
      composeKeys: async () => ({ keys: [], vault: [], unlocked: [] }),
      send: async () => null,
      forwardFiles: async () => ({ files: [], html: '', preview: '' }),
      popout: () => {}, unlock: async () => true, icsFile: async (_m: unknown, filename: string) => new File(['x'], filename),
      pendingCompose: () => { pendingCalls++; const p = pending; pending = null; return p; },
    },
  };
  const r = await render(page, { ...app, messages, requests, actions, helpers, storage, streams: { '/api/events/stream': stream } });
  await r.settle();
  return { ...r, requests, stream, storage, opened, pendingCalls: () => pendingCalls };
}

const rows = (root: HTMLElement) => [...root.querySelectorAll<HTMLButtonElement>('.message-row')];
const subjects = (root: HTMLElement) => rows(root).map((r) => r.querySelector('.subject')!.textContent);

test('shows the first inbox with its unread count, then opens a message in the reader and marks it read once', async () => {
  const { root, settle, opened, requests, stream } = await show({ 'PATCH /api/email/message/flags': null });
  const marks = () => requests.calls.filter((c) => c.method === 'PATCH').map((c) => c.body);
  expect(subjects(root)).toEqual(['Third', 'Second', 'First']);
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Inbox');
  expect(root.querySelector('.sidebar-item[aria-current] [data-badge]')!.textContent).toBe('1');
  expect(rows(root)[0].classList.contains('unread')).toBe(true);

  rows(root)[0].click();
  await settle();
  expect(opened).toEqual([{ account_id: 'work', mailbox: 'INBOX', uid: 3 }]);
  expect(root.querySelector('.reader .detail-subject')!.textContent).toBe('Third');
  expect(root.querySelector('.reader .body-text')!.textContent).toBe('body 3');
  // the list shows it read once the server says so (the reader patched only its own message)
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 3, flags: ['\\Seen'] });
  await settle();
  expect(rows(root)[0].classList.contains('unread')).toBe(false);
  expect(rows(root)[0].getAttribute('aria-current')).toBe('true');
  expect(marks()).toEqual([{ account_id: 'work', mailbox: 'INBOX', uid: 3, flags: ['\\Seen'], set: true }]);

  expect(root.querySelector('#mail-actions-menu button[data-icon="mail"]')!.textContent).toBe(m.mail_markUnread());   // the reader shows it read
  rows(root)[1].click();                                          // a read message is not marked
  await settle();
  expect(marks()).toHaveLength(1);
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

const ref = (uid: number) => ({ account_id: 'work', mailbox: 'INBOX', uid });

test('marks rows with Ctrl and Shift, and runs a bulk action in one request; the server events update the list', async () => {
  const { root, settle, requests, stream } = await show({ 'PATCH /api/email/messages/flags': { failed: [] } });
  const r = rows(root);
  r[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  r[2].dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }));
  await settle();
  expect(rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true')).toHaveLength(3);
  expect(root.querySelector('.message-detail h2')!.textContent).toBe('3 messages selected');

  root.querySelector<HTMLButtonElement>('.bulk-menu button[data-icon="mail"]')!.click();
  await settle();
  expect(requests.calls.at(-1)).toMatchObject({ method: 'PATCH', path: '/api/email/messages/flags' });
  expect(requests.calls.at(-1)!.body).toEqual({ messages: [ref(3), ref(2), ref(1)], flags: ['\\Seen'], set: false });
  expect(root.querySelectorAll('.message-row[aria-pressed="true"]')).toHaveLength(0);
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 2, flags: [] });
  await settle();
  expect(rows(root)[1].classList.contains('unread')).toBe(true);
});

test('a bulk archive moves each message to its account\'s archive; the failed ones stay marked, with the error', async () => {
  const { root, settle, requests } = await show({ 'POST /api/email/messages/move': { failed: [ref(1)] } });
  const r = rows(root);
  r[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  r[2].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  await settle();
  root.querySelector<HTMLButtonElement>('.bulk-menu button[data-icon="archive"]')!.click();
  await settle();
  expect(requests.calls.at(-1)).toMatchObject({ method: 'POST', path: '/api/email/messages/move' });
  expect(requests.calls.at(-1)!.body).toEqual({ messages: [{ ...ref(3), dest_mailbox: 'Archive' }, { ...ref(1), dest_mailbox: 'Archive' }] });
  expect(rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true').map((x) => x.querySelector('.subject')!.textContent)).toEqual(['First']);
  expect(root.querySelector('.message-detail [role=alert]')!.textContent).toBe(m.mail_bulkActionError());
});

test('a bulk delete that fails keeps every message marked, with the error', async () => {
  const { root, settle, requests } = await show({ 'POST /api/email/messages/delete': { status: 500 } });
  const r = rows(root);
  r[0].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  r[1].dispatchEvent(new MouseEvent('click', { bubbles: true, ctrlKey: true }));
  await settle();
  root.querySelector<HTMLButtonElement>('.bulk-menu button[data-icon="trash-2"]')!.click();
  await settle();
  expect(requests.calls.at(-1)!.body).toEqual({ messages: [ref(3), ref(2)] });
  expect(rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true')).toHaveLength(2);
  expect(root.querySelector('.message-detail [role=alert]')!.textContent).toBe(m.mail_bulkActionError());
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

test('server events: a mailbox refetch reloads the list when it is the one shown (the first inbox when none is chosen)', async () => {
  const { requests, stream, settle } = await show();
  const lists = () => requests.calls.filter((c) => c.method === 'GET' && c.path === '/api/email/messages').length;
  const before = lists();
  stream.emit('emails_refetch', { account_id: 'work', mailbox: 'Archive' });
  await settle();
  expect(lists()).toBe(before);
  stream.emit('emails_refetch', { account_id: 'work', mailbox: 'INBOX' });
  await settle();
  expect(lists()).toBe(before + 1);
});

test('the reader\'s star and read toggles stay on success; the list follows the server\'s events', async () => {
  const { root, settle, requests, stream } = await show({ 'PATCH /api/email/message/flags': null });
  const patched = () => requests.calls.filter((c) => c.method === 'PATCH').at(-1)!.body;
  rows(root)[1].click();
  await settle();
  const star = () => root.querySelector<HTMLButtonElement>('.reader .star-btn')!;
  star().click();
  await settle();
  expect(patched()).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Flagged'], set: true });
  expect(star().classList.contains('flagged')).toBe(true);
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Seen', '\\Flagged'] });
  await settle();
  expect(rows(root)[1].querySelector('.flag-star')).not.toBeNull();

  root.querySelector<HTMLButtonElement>('#mail-actions-menu button[data-icon="mail"]')!.click();
  await settle();
  expect(patched()).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Seen'], set: false });
  expect(root.querySelector('#mail-actions-menu button[data-icon="mail"]')!.textContent).toBe(m.mail_markRead());
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Flagged'] });
  await settle();
  expect(rows(root)[1].classList.contains('unread')).toBe(true);
});

test('a failed star toggle is rolled back', async () => {
  // a 4xx fails at once (a 5xx is sent again for a while: the flags retry policy)
  const { root, settle } = await show({ 'PATCH /api/email/message/flags': { status: 400 } });
  rows(root)[1].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.reader .star-btn')!.click();
  await settle();
  expect(root.querySelector('.reader .star-btn')!.classList.contains('flagged')).toBe(false);
  expect(rows(root)[1].querySelector('.flag-star')).toBeNull();
});

test('keyboard: j/k open the neighbours, Ctrl/Shift+Space mark, Escape clears, Shift+U marks unread', async () => {
  const { root, settle, requests } = await show({ 'PATCH /api/email/message/flags': null });
  const press = (target: Element, key: string, init: KeyboardEventInit = {}) =>
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }));
  const subject = () => root.querySelector('.reader .detail-subject')?.textContent;
  const marked = () => rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true').map((x) => x.querySelector('.subject')!.textContent);

  press(rows(root)[0], 'j');
  await settle();
  expect(subject()).toBe('Third');
  expect(rows(root)[0].classList.contains('cursor')).toBe(true);
  press(root.querySelector('.reader')!, 'j');                    // in the reader too
  await settle();
  expect(subject()).toBe('Second');
  press(rows(root)[1], 'k');
  await settle();
  expect(subject()).toBe('Third');
  const handle = root.querySelector('.detail-resize-handle')!;     // over a resize handle: its arrows, the page's j/k
  expect(press(handle, 'ArrowRight')).toBe(false);
  press(handle, 'j');
  await settle();
  expect(subject()).toBe('Second');
  press(handle, 'k');
  await settle();
  expect(subject()).toBe('Third');

  press(rows(root)[0], 'U', { shiftKey: true });                 // the read toggle in the actions menu
  await settle();
  expect(requests.calls.filter((c) => c.method === 'PATCH').at(-1)!.body).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: 3, flags: ['\\Seen'], set: false });

  rows(root)[1].dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
  press(rows(root)[1], ' ', { code: 'Space', ctrlKey: true });
  rows(root)[2].dispatchEvent(new FocusEvent('focusin', { bubbles: true }));
  press(rows(root)[2], ' ', { code: 'Space', shiftKey: true });
  await settle();
  expect(marked()).toEqual(['Third', 'Second', 'First']);         // the open one (Third) counts as marked
  press(rows(root)[2], 'j');                                      // no j/k while rows are marked
  await settle();
  expect(marked()).toEqual(['Third', 'Second', 'First']);
  press(rows(root)[2], 'Escape');                                 // "Clear selection"
  await settle();
  expect(marked()).toEqual([]);
});

test('j past the last row loads the next page when there is one, and nothing at the end', async () => {
  // keyset paging: a full page (50) means there may be more, a shorter one is the end
  const full = Array.from({ length: 50 }, (_, i) => msg(100 - i, `M${100 - i}`));
  const pages = (first: Message[]) => (call: { url: string }) =>
    new URL(call.url, 'http://x').searchParams.get('cursor_rowid') ? { messages: [msg(1, 'Next page')] } : { messages: first };
  const press = (target: Element, key: string) =>
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  const listed = (r: { requests: { calls: { method: string; path: string; url: string }[] } }) =>
    r.requests.calls.filter((c) => c.method === 'GET' && c.path === '/api/email/messages').map((c) => c.url);
  const jAtLast = async (r: Awaited<ReturnType<typeof show>>) => {
    rows(r.root).at(-1)!.click();
    await r.settle();
    press(rows(r.root).at(-1)!, 'j');
    await r.settle();
  };

  const short = await show({ 'GET /api/email/messages': pages(inbox), 'PATCH /api/email/message/flags': null });  // three rows: the end
  await jAtLast(short);
  expect(listed(short)).toHaveLength(1);

  const more = await show({ 'GET /api/email/messages': pages(full), 'PATCH /api/email/message/flags': null });   // a full page: load the next one
  await jAtLast(more);
  expect(listed(more)).toHaveLength(2);
  expect(listed(more)[1]).toContain('direction=forward');
  expect(listed(more)[1]).toContain('cursor_rowid=');
  expect(subjects(more.root)).toEqual(['Next page']);
});

const folderRow = { folder_id: 'f1', display_name: 'followup-Folder', sort_order: 0, query: '', mailboxes: [] };
const folderItems = (root: HTMLElement) => [...root.querySelectorAll('.sidebar-item')].map((b) => b.textContent!.trim()).filter((t) => t.startsWith('followup-'));

test('the folder dialog creates a folder and saves an edited one, each with one POST', async () => {
  const folder = { folder_id: 'f1', display_name: 'followup-Folder', sort_order: 0, only_flagged: false, only_attachments: false,
    rules: [{ account_id: null, mailbox_pattern: null }] };
  const { root, settle, requests } = await show({ 'GET /api/email/folders': { folders: [folder] }, 'POST /api/email/folder': { folder_id: 'f2' } });
  const save = async (name: string) => {
    const input = root.querySelector<HTMLInputElement>('#mail-folder input:not([type])')!;
    input.value = name;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    root.querySelector<HTMLFormElement>('#mail-folder form')!.requestSubmit();
    await settle();
    return requests.calls.filter((c) => c.method === 'POST' && c.path === '/api/email/folder').at(-1)!.body;
  };
  root.querySelector<HTMLButtonElement>('.sidebar-add button:last-child')!.click();
  await settle();
  expect(await save('followup-New')).toEqual({ folder_id: null, display_name: 'followup-New', rules: [{ account_id: null, mailbox_pattern: null }], only_flagged: false, only_attachments: false });
  expect(root.querySelector<HTMLDialogElement>('#mail-folder')!.open).toBe(false);

  root.querySelector<HTMLButtonElement>('button[data-icon="pencil"]')!.click();
  await settle();
  expect(await save('followup-Renamed')).toMatchObject({ folder_id: 'f1', display_name: 'followup-Renamed' });
  expect(requests.calls.some((c) => c.method === 'PUT')).toBe(false);
});

test('allowing an ignored image origin is one POST to the allow list (the server takes it off the ignore list)', async () => {
  const { root, settle, requests } = await show({
    'GET /api/email/image-blacklist': { origins: ['https://ads.example'] },
    'POST /api/email/image-origins': null,
  });
  root.querySelector<HTMLButtonElement>('.origins-trigger')!.click();
  await settle();
  const allow = [...root.querySelectorAll<HTMLButtonElement>('#mail-origins button')].find((b) => b.textContent === m.mail_originsAllow())!;
  const loads = () => requests.calls.filter((c) => c.method === 'GET' && c.path.startsWith('/api/email/image-')).length;
  const before = loads();
  allow.click();
  await settle();
  const writes = requests.calls.filter((c) => c.method !== 'GET' && c.path.startsWith('/api/email/image-'));
  expect(writes.map((c) => [c.method, c.path, c.body])).toEqual([['POST', '/api/email/image-origins', { origin: 'https://ads.example' }]]);
  expect(loads()).toBe(before + 2);   // both lists reloaded
});

test('removing the open virtual folder drops it and goes back to the inbox', async () => {
  const { root, settle } = await show({ 'GET /api/email/folders': { folders: [folderRow] }, 'DELETE /api/email/folder': null, 'GET /api/email/search': { messages: [] } });
  [...root.querySelectorAll<HTMLButtonElement>('.sidebar-item')].find((b) => b.textContent!.includes('followup-Folder'))!.click();
  await settle();
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('followup-Folder');
  root.querySelector<HTMLButtonElement>(`button[aria-label="${m.mail_removeAttachment()}"][data-icon="x"]`)!.click();
  await settle();
  expect(folderItems(root)).toEqual([]);
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Inbox');
});

test('a failed folder removal brings it back', async () => {
  const { root, settle } = await show({ 'GET /api/email/folders': { folders: [folderRow] }, 'DELETE /api/email/folder': { status: 500 } });
  root.querySelector<HTMLButtonElement>(`button[aria-label="${m.mail_removeAttachment()}"][data-icon="x"]`)!.click();
  await settle();
  expect(folderItems(root)).toEqual(['followup-Folder']);
});

test('reply opens the compose dialog with the quoted message, and sends it', async () => {
  const { root, settle } = await show();
  rows(root)[2].click();
  await settle();
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));   // the reply button's shortcut
  root.querySelector<HTMLDialogElement>('#mail-compose')!.showModal();
  await settle();
  expect(root.querySelector<HTMLInputElement>('#compose-subject')!.value).toBe('Re: First');
  expect(root.querySelector<HTMLInputElement>('#compose-to')!.value).toBe('Sender 1 <s1@x>');
  expect(root.querySelector<HTMLTextAreaElement>('.compose-body')!.value).toContain('> body 1');
  root.querySelector<HTMLFormElement>('.compose-form')!.requestSubmit();
  await settle();
  expect(root.querySelector<HTMLDialogElement>('#mail-compose')!.open).toBe(false);
});

test('a draft shared from another page opens compose once, from the default identity, with its attachment', async () => {
  const card = new File(['BEGIN:VCARD'], 'followup-ada.vcf', { type: 'text/vcard' });
  const { root, settle, requests, pendingCalls } = await show({ 'GET /api/email/identities': { identities: [
    { identity_id: 'i1', account_id: 'work', name: 'Me', email: 'me@x', is_default: false, sort_order: 0 },
    { identity_id: 'i2', account_id: 'work', name: 'Also me', email: 'also@x', is_default: true, sort_order: 1 },
  ] } }, { subject: 'followup-Ada', files: [card] });
  const dialog = root.querySelector<HTMLDialogElement>('#mail-compose')!;
  expect(dialog.open).toBe(true);
  expect(root.querySelector<HTMLInputElement>('#compose-subject')!.value).toBe('followup-Ada');
  expect(root.querySelector('.compose-attachments')!.textContent).toContain('followup-ada.vcf');
  expect(root.querySelector<HTMLSelectElement>('#compose-from')!.value).toBe('i2');
  expect(requests.calls.some((c) => c.url === '/api/contacts')).toBe(true);
  expect(pendingCalls()).toBe(1);

  // It is taken once: closing compose does not bring it back.
  dialog.close();
  await settle();
  expect(pendingCalls()).toBe(1);
  expect(dialog.open).toBe(false);
});

test('without a shared draft, compose stays closed', async () => {
  const { root, pendingCalls } = await show();
  expect(pendingCalls()).toBe(1);
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

test('the thread button opens the conversation; a message in it opens in the reader', async () => {
  const { root, settle } = await show({
    'GET /api/email/thread': { messages: [
      { uid: 1, account_id: 'work', mailbox: 'INBOX', message_id: '<1@x>', parent_message_id: null, from: 'Sender 1 <s1@x>', subject: 'First', sent_at: 1, is_read: true },
      { uid: 2, account_id: 'work', mailbox: 'INBOX', message_id: '<2@x>', parent_message_id: '<1@x>', from: 'Sender 2 <s2@x>', subject: 'Second', sent_at: 2, is_read: true },
    ] },
  });
  rows(root)[2].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.reader button[data-icon="list-tree"]')!.click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#thread-dialog')!;
  expect(dialog.open).toBe(true);
  const links = [...dialog.querySelectorAll<HTMLAnchorElement>('a')];
  expect(links.map((a) => a.getAttribute('aria-current'))).toEqual(['true', null]);
  links[1].click();
  await settle();
  expect(dialog.open).toBe(false);
  expect(root.querySelector('.reader .detail-subject')!.textContent).toBe('Second');
});

// The message list's URL reads only the remembered view (so sinter's early file can request it): the view the
// lists settle on is written back, a remembered one is requested at once, a gone one is replaced.
const listUrls = (requests: { calls: { path: string; url: string }[] }) => requests.calls.filter((c) => c.path === '/api/email/messages').map((c) => c.url);
const remembered = (storage: Storage) => JSON.parse(storage.getItem('sn:mail') ?? '{}').view;

test('the inbox the page settles on is remembered for the next visit', async () => {
  const { storage, requests } = await show();
  expect(remembered(storage)).toEqual({ account_id: 'work', mailbox: 'INBOX', folder_id: null });
  expect(listUrls(requests)).toEqual(['/api/email/messages?account_id=work&mailbox=INBOX&limit=50&direction=forward']);
});

test('a remembered mailbox is the first list requested', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:mail', JSON.stringify({ view: { account_id: 'work', mailbox: 'Archive', folder_id: null }, collapsed: [] }));
  const { root, requests } = await show({}, null, storage);
  expect(listUrls(requests)[0]).toBe('/api/email/messages?account_id=work&mailbox=Archive&limit=50&direction=forward');
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Archive');
});

test('a remembered folder that is gone gives way to the inbox', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:mail', JSON.stringify({ view: { account_id: null, mailbox: null, folder_id: 'gone' }, collapsed: [] }));
  const { root, requests } = await show({ 'GET /api/email/search': { messages: [] } }, null, storage);
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Inbox');
  expect(listUrls(requests).at(-1)).toBe('/api/email/messages?account_id=work&mailbox=INBOX&limit=50&direction=forward');
  expect(remembered(storage)).toEqual({ account_id: 'work', mailbox: 'INBOX', folder_id: null });
});

test('the column widths set with the dividers are remembered for the next visit', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:mail-widths', JSON.stringify({ sidebarW: 260, listW: 400 }));
  const { root, flush } = await show({}, null, storage);
  const layout = root.querySelector<HTMLElement>('.mail-layout')!;
  expect(layout.style.getPropertyValue('--sidebar-w')).toBe('260px');
  expect(layout.style.getPropertyValue('--list-w')).toBe('400px');
  const split = root.querySelector('.detail-resize-handle')!;
  split.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
  await flush();
  const saved = JSON.parse(storage.getItem('sn:mail-widths')!);
  expect(saved.sidebarW).toBe(260);
  expect(saved.listW).toBeGreaterThan(400);
});

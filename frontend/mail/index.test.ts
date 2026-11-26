import { m } from '@tiaforge-ui/sinter/messages';
import { messages } from '../src/test-messages.ts';
import { afterEach, beforeEach, expect, test } from 'vitest';
import { configure } from '@tiaforge-ui/sinter';
import { render, memoryStorage, mockRequests, mockStream, type RouteResponse } from '@tiaforge-ui/sinter/testing';
import * as helpers from '../src/sinter.helpers.ts';
import '../src/islands/common.ts';                 // data-shortcut keys
import page from './index.html?raw';
import { app, mailWidth } from '../src/test-components.ts';
import { retryPolicies } from '../src/lib/retry-policies.ts';
import type { Message } from '../src/lib/mail.ts';
import '../src/test-dom.ts';                 // jsdom: dialogs, popovers, commandfor

const user = { display_name: 'Demo User', has_settings: true, has_caldav: true, oidc_enabled: false, map_url: '', oidc_signed_in: false };
const msg = (uid: number, subject: string, extra: Partial<Message> = {}): Message => ({
  uid, account_id: 'work', mailbox: 'INBOX', rowid: uid, message_id: `<${uid}@x>`, thread_id: null, from: `Sender ${uid} <s${uid}@x>`,
  to: ['me@x'], cc: [], subject, sent_at: 1_700_000_000 + uid, flags: ['\\Seen'], has_attachments: false, body_cached: true,
  body_text: `body ${uid}`, body_html: null, autocrypt_headers: [], is_read: true, is_flagged: false, is_encrypted: false,
  is_signed: false, attachments: [], ...extra,
});
const inbox = [msg(3, 'Third', { flags: [], is_read: false }), msg(2, 'Second'), msg(1, 'First')];


// the HTML body the reader gets (mail.body), '' for a text body; `decrypted`: it is a decrypted body
let htmlBody = '';
let decrypted = false;
// the body's parts, instead of the one HTML or text part above (test of the reader's blocks)
let partsOverride: object[] | null = null;
// while set, mail.body waits for it (a body that takes a while)
let bodyGate: Promise<void> | null = null;
// what the account dialog's autodiscovery finds
let discovery: object = { found: false };
const openedParts: string[] = [];
// what mail.composeKeys answers, and how often it was asked
let composeKeysResult: object = { keys: [], pending: [], vault: [], unlocked: [] };
let composeKeysCalls = 0;
// fields of the body mail.body answers, over the defaults
let bodyExtra: object = {};
// the drafts mail.send was given
const sent: { encrypt: boolean }[] = [];

// the app's retry policies: the flag toggles name one (data-sn-retry="flags")
beforeEach(() => { composeKeysResult = { keys: [], pending: [], vault: [], unlocked: [] }; composeKeysCalls = 0; sent.length = 0; bodyExtra = {}; discovery = { found: false }; bodyGate = null; htmlBody = ''; decrypted = false; partsOverride = null; openedParts.length = 0; localStorage.clear(); configure({ confirm: () => true, urlSchemes: ['blob:'], retry: retryPolicies }); });
afterEach(() => configure({ confirm: null as never, urlSchemes: [], retry: null }));

async function show(routes: Record<string, RouteResponse> = {}, pending: { subject: string; files: File[] } | null = null, storage = memoryStorage(),
                    url = 'http://localhost/mail/', scripts: Record<string, unknown> = {}) {
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
    share: { native: () => false, prepare: async () => null, send: async () => {}, download: async () => {}, email: async () => {} },
    maps: { open: async () => {} },
    notify: { status: () => ({ supported: false, permission: 'default', enabled: false }), toggle: async (s: unknown) => s },
    mail: {
      open: async (ref: { uid: number }) => { opened.push(ref); return current.find((msg) => msg.uid === ref.uid) ?? inbox[0]; },
      body: async (ref: { account_id: string; mailbox: string; uid: number }) => {
        await bodyGate;
        const part = { id: '1', html: '', text: '', legacyDisplay: false, encrypted: decrypted, opened: false, mark: '', reason: '', step: '', depth: 0 };
        const textParts = [{ ...part, kind: 'text', text: `body ${ref.uid}` }];
        return {
          key: `${ref.account_id} ${ref.mailbox} ${ref.uid}`, message: current.find((msg) => msg.uid === ref.uid) ?? inbox[0], parts: partsOverride ?? (htmlBody ? [{ ...part, kind: 'html', html: htmlBody }] : textParts), textParts: partsOverride ?? textParts,
          html: htmlBody, text: `body ${ref.uid}`, quotable: !decrypted, mode: htmlBody ? 'html' : 'text', encrypted: decrypted, decrypted, signed: false,
          locked: false, badges: [], subject: '', reply: decrypted ? [`Sender ${ref.uid} <s${ref.uid}@x>`] : null, replyCc: decrypted ? [] : null, error: '', files: [], importable: {}, cids: null, forwardCids: null, ...bodyExtra,
        };
      },
      openPart: (_ref: unknown, step: string) => { openedParts.push(step); },
      verify: () => {},
      wipe: () => {},
      composeKeys: async () => { composeKeysCalls++; return composeKeysResult; },
      autodiscover: async () => discovery,
      send: async (draft: { encrypt: boolean }) => { sent.push(draft); return null; },
      forwardFiles: async () => ({ files: [], html: '' }),
      popout: () => {}, unlock: async () => true, icsFile: async (_m: unknown, filename: string) => new File(['x'], filename),
      pendingCompose: () => { pendingCalls++; const p = pending; pending = null; return p; },
    },
  };
  const r = await render(page, { ...app, scripts: { ...app.scripts, ...scripts }, messages, requests, actions, helpers, storage, url, streams: { '/api/events/stream': stream } });
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
  expect(root.querySelector('.reader .body-text span')).toBeNull();
  // the list shows it read once the server says so (the reader patched only its own message)
  stream.emit('email_updated', { account_id: 'work', mailbox: 'INBOX', uid: 3, flags: ['\\Seen'] });
  await settle();
  expect(rows(root)[0].classList.contains('unread')).toBe(false);
  expect(rows(root)[0].getAttribute('aria-current')).toBe('true');
  expect(marks()).toEqual([{ account_id: 'work', mailbox: 'INBOX', uid: 3, flags: ['\\Seen'], set: true }]);

  expect(root.querySelector('[id^="message-actions-"] button[data-icon="mail"] > span')!.textContent).toBe(m.mail_markUnread());   // the reader shows it read
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
  const search = root.querySelector<HTMLInputElement>('.message-list-header [data-field] > input[type=search]')!;
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

test('a search page offers the next one only when the server says there are more (has_more), not because it is full', async () => {
  const page = Array.from({ length: 50 }, (_, i) => msg(100 - i, `Hit ${i}`));
  let more = false;
  const { root, settle, advance } = await show({ 'GET /api/email/search': () => ({ messages: page, has_more: more }) });
  const next = () => root.querySelector<HTMLButtonElement>(`.pagination-bar button[aria-label="${m.mail_nextPageAria()}"]`)!;
  const search = root.querySelector<HTMLInputElement>('.message-list-header [data-field] > input[type=search]')!;
  search.value = 'hit';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  await advance(300);
  await settle();
  expect(subjects(root)).toHaveLength(50);
  expect(next().disabled).toBe(true);    // exactly 50 results: no empty next page

  more = true;
  search.value = 'hits';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  await advance(300);
  await settle();
  expect(next().disabled).toBe(false);
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

  root.querySelector<HTMLButtonElement>('[id^="message-actions-"] button[data-icon="mail"]')!.click();
  await settle();
  expect(patched()).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: 2, flags: ['\\Seen'], set: false });
  expect(root.querySelector('[id^="message-actions-"] button[data-icon="mail"] > span')!.textContent).toBe(m.mail_markRead());
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

test('selection mode (the Select button): taps mark rows without opening them, the bar acts on them and ends the mode', async () => {
  const { root, settle, requests, opened } = await show({ 'PATCH /api/email/messages/flags': { failed: [] } });
  const marked = () => rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true').map((x) => x.querySelector('.subject')!.textContent);
  const detailOpen = () => root.querySelector('.message-detail')!.classList.contains('is-open');
  expect(root.querySelector('.selection-bar')).toBeNull();
  root.querySelector<HTMLButtonElement>('.message-list-header button[data-icon="list-checks"]')!.click();
  await settle();
  expect(root.querySelector('.selection-row [role=status]')!.textContent).toBe(m.mail_selectPrompt());
  expect(root.querySelector('.pagination-bar')).toBeNull();
  expect(root.querySelector<HTMLButtonElement>('.selection-bar button[data-icon="mail-open"]')!.disabled).toBe(true);

  rows(root)[0].click();
  rows(root)[2].click();
  await settle();
  expect(marked()).toEqual(['Third', 'First']);
  expect(opened).toEqual([]);
  expect(detailOpen()).toBe(false);                                // the list stays in view while marking
  expect(root.querySelectorAll('.message-row [data-icon="square-check"]')).toHaveLength(2);
  expect(root.querySelector('.selection-row [role=status]')!.textContent).toBe('2 messages selected');
  rows(root)[2].click();                                           // a second tap unmarks
  await settle();
  expect(marked()).toEqual(['Third']);
  root.querySelector<HTMLButtonElement>('.selection-row button[data-icon="check-check"]')!.click();
  await settle();
  expect(marked()).toEqual(['Third', 'Second', 'First']);

  root.querySelector<HTMLButtonElement>('.selection-bar button[data-icon="mail-open"]')!.click();
  await settle();
  expect(requests.calls.at(-1)).toMatchObject({ method: 'PATCH', path: '/api/email/messages/flags' });
  expect(requests.calls.at(-1)!.body).toEqual({ messages: [ref(3), ref(2), ref(1)], flags: ['\\Seen'], set: true });
  expect(marked()).toEqual([]);
  expect(root.querySelector('.selection-bar')).toBeNull();
  expect(root.querySelector('.pagination-bar')).not.toBeNull();
  rows(root)[1].click();                                           // out of the mode a tap opens again
  await settle();
  expect(opened).toEqual([ref(2)]);
});

test('selection mode: More shows the whole bulk menu, its back arrow returns to the marks; a failed action keeps the mode', async () => {
  const { root, settle } = await show({ 'POST /api/email/messages/delete': { failed: [ref(2)] } });
  const marked = () => rows(root).filter((x) => x.getAttribute('aria-pressed') === 'true').map((x) => x.querySelector('.subject')!.textContent);
  const detailOpen = () => root.querySelector('.message-detail')!.classList.contains('is-open');
  root.querySelector<HTMLButtonElement>('.message-list-header button[data-icon="list-checks"]')!.click();
  await settle();
  rows(root)[0].click();
  rows(root)[1].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.selection-bar button[data-icon="ellipsis"]')!.click();
  await settle();
  expect(detailOpen()).toBe(true);
  expect(root.querySelector('.message-detail h2')!.textContent).toBe('2 messages selected');
  root.querySelector<HTMLButtonElement>('.message-detail .detail-header button[data-icon="arrow-left"]')!.click();
  await settle();
  expect(detailOpen()).toBe(false);
  expect(marked()).toEqual(['Third', 'Second']);

  root.querySelector<HTMLButtonElement>('.selection-bar button[data-icon="trash-2"]')!.click();
  await settle();
  expect(marked()).toEqual(['Second']);
  expect(root.querySelector('.selection-bar [role=alert]')!.textContent).toBe(m.mail_bulkActionError());
  root.querySelector<HTMLButtonElement>('.selection-row button[data-icon="x"]')!.click();   // Cancel
  await settle();
  expect(marked()).toEqual([]);
  expect(root.querySelector('.selection-row')).toBeNull();
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
  const body = root.ownerDocument.body;                          // nothing focused, or the rail: the keys are the page's
  press(body, 'j');
  await settle();
  expect(subject()).toBe('Second');
  press(root.querySelector('nav a')!, 'k');
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
  // offset paging: the total says whether there is a next page (three messages: none; 51: one more)
  const full = Array.from({ length: 50 }, (_, i) => msg(100 - i, `M${100 - i}`));
  const pages = (first: Message[]) => (call: { url: string }) => {
    const total = first.length === 50 ? 51 : first.length;
    return Number(new URL(call.url, 'http://x').searchParams.get('offset')) > 0 ? { messages: [msg(1, 'Next page')], total } : { messages: first, total };
  };
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
  expect(listed(more)[1]).toContain('offset=50');
  expect(subjects(more.root)).toEqual(['Next page']);
});

const folderRow = { folder_id: 'f1', display_name: 'followup-Folder', sort_order: 0, query: '', mailboxes: [] };
const folderItems = (root: HTMLElement) => [...root.querySelectorAll('.sidebar-item')].map((b) => b.textContent!.trim()).filter((t) => t.startsWith('followup-'));

test('the folder dialog creates a folder and saves an edited one, each with one POST', async () => {
  const folder = { folder_id: 'f1', display_name: 'followup-Folder', sort_order: 0, only_flagged: false, only_attachments: false,
    rules: [{ account_id: null, mailbox_pattern: null }] };
  const { root, settle, requests } = await show({ 'GET /api/email/folders': { folders: [folder] }, 'POST /api/email/folder': { folder_id: 'f2' } });
  const save = async (name: string) => {
    const input = root.querySelector<HTMLInputElement>('#email-folder input:not([type])')!;
    input.value = name;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    root.querySelector<HTMLFormElement>('#email-folder form')!.requestSubmit();
    await settle();
    return requests.calls.filter((c) => c.method === 'POST' && c.path === '/api/email/folder').at(-1)!.body;
  };
  root.querySelector<HTMLButtonElement>('.sidebar-add button:last-child')!.click();
  await settle();
  expect(await save('followup-New')).toEqual({ folder_id: null, display_name: 'followup-New', rules: [{ account_id: null, mailbox_pattern: null }], only_flagged: false, only_attachments: false });
  expect(root.querySelector<HTMLDialogElement>('#email-folder')!.open).toBe(false);

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
  const allow = [...root.querySelectorAll<HTMLButtonElement>('#email-origins button')].find((b) => b.textContent === m.mail_originsAllow())!;
  const loads = () => requests.calls.filter((c) => c.method === 'GET' && c.path.startsWith('/api/email/image-')).length;
  const before = loads();
  allow.click();
  await settle();
  const writes = requests.calls.filter((c) => c.method !== 'GET' && c.path.startsWith('/api/email/image-'));
  expect(writes.map((c) => [c.method, c.path, c.body])).toEqual([['POST', '/api/email/image-origins', { origin: 'https://ads.example' }]]);
  expect(loads()).toBe(before + 2);   // both lists reloaded
});

test('a narrow reader (a phone) gets the mail\'s mobile layout: its @media rules for a narrow screen', async () => {
  htmlBody = '<style>.col { width: 300px } @media (max-width: 600px) { .col { width: 100% !important } }</style><table class="col"><tr><td>x</td></tr></table>';
  const wide = await show({ 'PATCH /api/email/message/flags': null });
  rows(wide.root)[0].click();
  await wide.settle();
  expect((wide.root.querySelector('.reader .mail-html .mail table') as HTMLElement).style.width).toBe('300px');
  const phone = await show({ 'PATCH /api/email/message/flags': null }, null, memoryStorage(), 'http://localhost/mail/', { 'mail-width': mailWidth('narrow') });
  rows(phone.root)[0].click();
  await phone.settle();
  expect((phone.root.querySelector('.reader .mail-html .mail table') as HTMLElement).style.width).toBe('100%');
});

test('an HTML body is part of the page (sinter-mail): its images through the proxy, again once their origin is allowed', async () => {
  htmlBody = '<style>p { color: #c00 }</style><p>Hello <a href="https://shop.example/x">shop</a></p>'
    + '<img src="https://img.example/a.png" alt="logo"><img src="cid:part1@x"><script>alert(1)</script>';
  const { root, settle, requests } = await show({ 'POST /api/email/image-origins': null, 'PATCH /api/email/message/flags': null });
  rows(root)[0].click();
  await settle();
  const body = root.querySelector('.reader .mail-html .mail')!;
  expect(root.querySelector('.reader iframe')).toBeNull();
  expect(body.querySelector('p')!.textContent).toBe('Hello shop');
  expect((body.querySelector('p') as HTMLElement).style.color).toBe('rgb(204, 0, 0)');
  expect(body.querySelector('script')).toBeNull();
  const link = body.querySelector('a')!;
  expect([link.getAttribute('href'), link.getAttribute('target'), link.getAttribute('rel')]).toEqual(['https://shop.example/x', '_blank', 'noopener noreferrer']);
  const srcs = () => [...body.querySelectorAll('img')].map((i) => i.getAttribute('src'));
  expect(srcs()).toEqual([
    '/api/email/proxy-image?v=0&url=https%3A%2F%2Fimg.example%2Fa.png',
    '/api/email/proxy-image?account_id=work&mailbox=INBOX&uid=3&cid=part1%40x',
  ]);
  const notice = root.querySelector('.reader .images-notice')!;
  notice.querySelector<HTMLButtonElement>('[data-join] > button')!.click();
  await settle();
  expect(requests.calls.filter((c) => c.method === 'POST' && c.path === '/api/email/image-origins').map((c) => c.body)).toEqual([{ origin: 'https://img.example' }]);
  expect(srcs()[0]).toBe('/api/email/proxy-image?v=1&url=https%3A%2F%2Fimg.example%2Fa.png');
});

test('a decrypted body loads no remote image until the reader asks, for this message only (sinter-mail\'s encrypted, consent)', async () => {
  htmlBody = '<p>Secret</p><img src="https://img.example/a.png" alt="logo"><img src="cid:part1@x">';
  decrypted = true;
  const { root, settle } = await show({ 'PATCH /api/email/message/flags': null });
  rows(root)[0].click();
  await settle();
  const srcs = () => [...root.querySelectorAll('.reader .mail-html .mail img')].map((i) => i.getAttribute('src'));
  expect(srcs()).toEqual([null, '/api/email/proxy-image?account_id=work&mailbox=INBOX&uid=3&cid=part1%40x']);   // the inline one stays
  const notice = root.querySelector('.reader .images-notice')!;
  expect(notice.textContent).toContain(m.mail_imagesEncrypted());
  notice.querySelector<HTMLButtonElement>('button')!.click();
  await settle();
  expect(srcs()[0]).toBe('/api/email/proxy-image?v=0&url=https%3A%2F%2Fimg.example%2Fa.png');
  rows(root)[1].click();                                           // another message: no consent carried over
  await settle();
  rows(root)[0].click();
  await settle();
  expect(srcs()[0]).toBeNull();
});

test('the body\'s parts in order: a signed part marked, an encrypted part from inside the message opened only on request, then shown apart', async () => {
  const base = { html: '', text: '', legacyDisplay: false, encrypted: false, opened: false, mark: '', reason: '', step: '', depth: 0 };
  partsOverride = [
    { ...base, id: '1.1', kind: 'text', text: 'My own words', mark: 'signed_part' },
    { ...base, id: '1.2', kind: 'encrypted', reason: 'not-root', step: '1.2#ab' },
  ];
  const { root, settle } = await show({ 'PATCH /api/email/message/flags': null });
  rows(root)[0].click();
  await settle();
  const parts = () => [...root.querySelectorAll('.reader .mail-part')];
  expect(parts().map((p) => p.querySelector('[data-badge]')?.textContent ?? '')).toEqual([m.pgp_badgeSignedPart(), '']);
  expect(parts()[0].querySelector('.body-text')!.textContent).toBe('My own words');
  expect(parts()[0].textContent).not.toContain(m.pgp_unlockToRead());   // a text part is only its text
  expect(parts()[1].textContent).toContain(m.pgp_encryptedPart());

  partsOverride = [partsOverride[0], { ...base, id: '1.2#ab/1', kind: 'text', text: 'Someone else\'s secret', encrypted: true, opened: true }];
  parts()[1].querySelector<HTMLButtonElement>('button')!.click();
  await settle();
  expect(openedParts).toEqual(['1.2#ab']);
  const box = parts()[1];
  expect(box.classList.contains('mail-part-opened')).toBe(true);
  expect(box.textContent).toContain(m.pgp_openedPart());
  expect(box.querySelector('.body-text')!.textContent).toBe('Someone else\'s secret');
});

test('opening another message keeps the reader on the last one (busy) until the next one\'s body is in: it never empties', async () => {
  const { root, settle, flush } = await show({ 'PATCH /api/email/message/flags': null });
  rows(root)[0].click();
  await settle();
  const subject = () => root.querySelector('.reader .detail-subject')?.textContent ?? null;
  const first = subject();
  expect(first).not.toBeNull();
  let release = () => {};
  bodyGate = new Promise((resolve) => { release = resolve; });
  rows(root)[1].click();
  await flush();
  expect(subject()).toBe(first);                                  // still the last one, not "Loading…"
  expect(root.querySelector('.reader')!.getAttribute('aria-busy')).toBe('true');
  release();
  await settle();
  expect(subject()).not.toBe(first);
  expect(root.querySelector('.reader')!.hasAttribute('aria-busy')).toBe(false);
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

// The virtual folders in their order: alloy's <alloy-sortable> around them; a drop moves the row at once (the moved
// folder's sort_order between its neighbours', the resource ordered by it) and sends the whole order.
const threeFolders = ['A', 'B', 'C'].map((n, i) => ({ ...folderRow, folder_id: `f${n}`, display_name: `followup-${n}`, sort_order: i }));
const handle = (root: HTMLElement, name: string) => root.querySelector<HTMLElement>(`button.al-handle[aria-label="${m.sort_handle({ name })}"]`)!;
/** a keyboard press on a button: its click, with no pointer */
const press = (el: HTMLElement) => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, detail: 0 }));
const key = (el: HTMLElement, k: string) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));
const renumbered = (ids: string[]) => ({ folders: ids.map((id, i) => ({ ...threeFolders.find((f) => f.folder_id === id)!, sort_order: i })) });

test('a virtual folder moved with the keyboard moves at once, and the new order is saved', async () => {
  const { root, settle, requests } = await show({
    'GET /api/email/folders': { folders: threeFolders },
    'PUT /api/email/folders/order': renumbered(['fC', 'fA', 'fB']),
  });
  expect(folderItems(root)).toEqual(['followup-A', 'followup-B', 'followup-C']);
  const h = handle(root, 'followup-C');
  expect(h.getAttribute('aria-description')).toBe(m.sort_instructions());
  h.focus();
  press(h);
  await settle();
  expect(root.querySelector('.alloy-sortable [role="status"]')!.textContent).toBe(m.sort_lifted({ name: 'followup-C', position: 3, total: 3 }));
  key(h, 'Home');
  press(h);
  await settle();
  expect(folderItems(root)).toEqual(['followup-C', 'followup-A', 'followup-B']);
  const put = requests.calls.find((c) => c.method === 'PUT' && c.path === '/api/email/folders/order')!;
  expect(put.body).toEqual({ folder_ids: ['fC', 'fA', 'fB'] });
  expect(root.querySelector('.mail-sidebar [role="alert"]')).toBeNull();
});

test('a folder order the server refuses goes back, with a note', async () => {
  const { root, settle } = await show({
    'GET /api/email/folders': { folders: threeFolders },
    'PUT /api/email/folders/order': { status: 500, body: 'database error' },
  });
  const h = handle(root, 'followup-A');
  press(h);
  await settle();
  key(h, 'ArrowDown');
  press(h);
  await settle();
  expect(folderItems(root)).toEqual(['followup-A', 'followup-B', 'followup-C']);
  expect(root.querySelector('.mail-sidebar [role="alert"]')!.textContent).toBe(m.mail_folderOrderFailed());
});

test('folders_changed (another tab reordered them) reloads the folders in their new order', async () => {
  let folders = threeFolders;
  const { root, settle, stream } = await show({ 'GET /api/email/folders': () => ({ folders }) });
  folders = renumbered(['fB', 'fC', 'fA']).folders;
  stream.emit('folders_changed', 1);
  await settle();
  expect(folderItems(root)).toEqual(['followup-B', 'followup-C', 'followup-A']);
});

test('reply opens the compose dialog with the quoted message, and sends it', async () => {
  const { root, settle } = await show();
  rows(root)[2].click();
  await settle();
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));   // the reply button's shortcut
  root.querySelector<HTMLDialogElement>('#email-compose')!.showModal();
  await settle();
  expect(root.querySelector<HTMLInputElement>('#compose-subject')!.value).toBe('Re: First');
  expect(root.querySelector<HTMLInputElement>('#compose-to')!.value).toBe('Sender 1 <s1@x>');
  expect(root.querySelector<HTMLTextAreaElement>('.compose-body')!.value).toContain('> body 1');
  root.querySelector<HTMLFormElement>('.compose-form')!.requestSubmit();
  await settle();
  expect(root.querySelector<HTMLDialogElement>('#email-compose')!.open).toBe(false);
});

test('a reply to decrypted mail starts encrypted; without a key for a recipient it is not sent in clear: the lock is the user\'s choice, with a warning', async () => {
  decrypted = true;
  const { root, settle } = await show();
  rows(root)[2].click();
  await settle();
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-compose')!;
  const lock = () => dialog.querySelector<HTMLButtonElement>(`button[aria-label="${m.pgp_composeEncrypt()}"]`)!;
  expect(lock().getAttribute('aria-pressed')).toBe('true');
  expect(lock().disabled).toBe(false);                                 // on without keys: the user can turn it off
  expect(dialog.textContent).toContain(m.pgp_composeEncryptBlocked({ emails: 's1@x, me@x' }));
  expect(dialog.textContent).not.toContain(m.pgp_composeDecryptedInClear());
  dialog.querySelector<HTMLFormElement>('.compose-form')!.requestSubmit();
  await settle();
  expect(sent.map((d) => d.encrypt)).toEqual([true]);                 // as it was: mail.send refuses what it cannot encrypt

  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));
  await settle();
  lock().click();
  await settle();
  expect(lock().getAttribute('aria-pressed')).toBe('false');
  expect(dialog.textContent).toContain(m.pgp_composeDecryptedInClear());
  expect(dialog.textContent).not.toContain(m.pgp_composeEncryptBlocked({ emails: 's1@x, me@x' }));
  dialog.querySelector<HTMLFormElement>('.compose-form')!.requestSubmit();
  await settle();
  expect(sent.map((d) => d.encrypt)).toEqual([true, false]);
});

test('a message verified for another sender than the server\'s From shows the checked sender, with the warning', async () => {
  bodyExtra = { badges: ['verified', 'from_differs'], sender: 'Alice <alice@x>' };
  const { root, settle } = await show();
  rows(root)[2].click();
  await settle();
  const meta = root.querySelector('.detail-meta')!;
  expect(meta.querySelector('.meta-line [data-truncate]')!.textContent).toBe('Alice <alice@x>');
  expect(root.querySelector('.detail-body')!.textContent).toContain(m.pgp_badgeFromDiffers());
  meta.querySelector<HTMLButtonElement>('.meta-primary')!.click();
  await settle();
  expect(meta.querySelector('.meta-details')!.textContent).toContain(m.mail_metaFromHeader());
  expect(meta.querySelector('.meta-details')!.textContent).toContain('Sender 1 <s1@x>');
});

test('importing an attached public key shows its fingerprint and user IDs first; a key that does not name the sender is stored for the address the user picks', async () => {
  const withKey = [msg(1, 'Key', { attachments: [{ filename: 'key.asc', content_type: 'application/pgp-keys', size: 900, content_id: null }], has_attachments: true })];
  let trusted = false;
  bodyExtra = { importable: { 'key.asc': {
    body: { fingerprint: 'ABCD1234ABCD1234ABCD1234ABCD1234ABCD1234', email: '', armored: 'K', is_own: false, source: 'message_attachment' },
    facts: { fingerprint: 'ABCD1234ABCD1234ABCD1234ABCD1234ABCD1234', userIds: ['Someone <other@x>', 'Someone <third@x>'], emails: ['other@x', 'third@x'] },
    sender: 's1@x', claimsAddress: false,
  } } };
  const { root, settle, requests } = await show({
    'GET /api/email/messages': { messages: withKey, total: 1 },
    'POST /api/pgp/key': (call) => { trusted = true; bodyExtra = {}; return { ...(call.body as object), trust: 'trusted' }; },
  });
  rows(root)[0].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.attachments button[aria-expanded]')!.click();
  await settle();
  root.querySelector<HTMLButtonElement>(`.attachments button[aria-label="${m.pgp_importPublicKeyButton()}"]`)!.click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-key-import')!;
  expect(dialog.open).toBe(true);
  expect(dialog.querySelector('.key-fingerprint')!.textContent).toBe('ABCD 1234 ABCD 1234 ABCD 1234 ABCD 1234 ABCD 1234');
  expect(dialog.querySelector('.key-facts')!.textContent).toContain('Someone <third@x>');
  expect(dialog.textContent).toContain(m.pgp_keyNotSender({ sender: 's1@x', emails: 'other@x, third@x' }));
  const store = () => [...dialog.querySelectorAll('button')].find((b) => b.textContent === m.pgp_trustImportButton())!;
  expect(store().disabled).toBe(true);                                 // no address chosen yet
  expect(requests.calls.some((c) => c.method === 'POST' && c.path === '/api/pgp/key')).toBe(false);
  const select = dialog.querySelector('select')!;
  select.value = 'third@x';
  select.dispatchEvent(new Event('change', { bubbles: true }));
  await settle();
  store().click();
  await settle();
  expect(trusted).toBe(true);
  expect(requests.calls.find((c) => c.method === 'POST' && c.path === '/api/pgp/key')!.body).toMatchObject({ email: 'third@x', source: 'message_attachment' });
  expect(dialog.open).toBe(false);
  expect(root.querySelector('.detail-body [role=status]')!.textContent).toContain(m.pgp_publicKeyImported());
  expect(root.querySelector(`.attachments button[aria-label="${m.pgp_importPublicKeyButton()}"]`)).toBeNull();   // trusted now
});

test('an attached key that names the sender is offered for the sender\'s address', async () => {
  const withKey = [msg(1, 'Key', { attachments: [{ filename: 'key.asc', content_type: 'application/pgp-keys', size: 900, content_id: null }], has_attachments: true })];
  bodyExtra = { importable: { 'key.asc': {
    body: { fingerprint: 'AB12', email: 's1@x', armored: 'K', is_own: false, source: 'message_attachment' },
    facts: { fingerprint: 'AB12', userIds: ['Sender 1 <s1@x>'], emails: ['s1@x'] }, sender: 's1@x', claimsAddress: true,
  } } };
  const { root, settle } = await show({ 'GET /api/email/messages': { messages: withKey, total: 1 } });
  rows(root)[0].click();
  await settle();
  root.querySelector<HTMLButtonElement>('.attachments button[aria-expanded]')!.click();
  await settle();
  root.querySelector<HTMLButtonElement>(`.attachments button[aria-label="${m.pgp_importPublicKeyButton()}"]`)!.click();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-key-import')!;
  expect(dialog.textContent).toContain(m.pgp_importForSender({ email: 's1@x' }));
  expect(dialog.querySelector('select')).toBeNull();
});

test('compose shows a recipient\'s key found in mail with its full fingerprint and user IDs, and accepts it only on request', async () => {
  composeKeysResult = {
    keys: [], vault: [], unlocked: [],
    pending: [{ fingerprint: 'ABCD1234ABCD1234ABCD1234ABCD1234ABCD1234', email: 's1@x', userIds: ['Sender 1 <s1@x>'], change: false }],
  };
  const { root, settle, requests } = await show({ 'POST /api/pgp/key/accept': (call) => ({ ...(call.body as object), trust: 'trusted' }) });
  rows(root)[2].click();
  await settle();
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-compose')!;
  const box = dialog.querySelector('.compose-keys')!;
  expect(box.textContent).toContain(m.pgp_composeKeyWaiting({ email: 's1@x' }));
  expect(box.querySelector('.key-fingerprint')!.textContent).toBe('ABCD 1234 ABCD 1234 ABCD 1234 ABCD 1234 ABCD 1234');
  expect(box.textContent).toContain('Sender 1 <s1@x>');
  expect(requests.calls.some((c) => c.path === '/api/pgp/key/accept')).toBe(false);
  const asked = composeKeysCalls;
  composeKeysResult = { keys: [{ fingerprint: 'ABCD1234ABCD1234ABCD1234ABCD1234ABCD1234', emails: ['s1@x'], is_own: false }], pending: [], vault: [], unlocked: [] };
  [...box.querySelectorAll('button')].find((b) => b.textContent === m.pgp_acceptKey())!.click();
  await settle();
  expect(requests.calls.find((c) => c.path === '/api/pgp/key/accept')!.body).toEqual({ fingerprint: 'ABCD1234ABCD1234ABCD1234ABCD1234ABCD1234', replace: false });
  expect(composeKeysCalls).toBeGreaterThan(asked);
  expect(dialog.querySelector('.compose-keys')).toBeNull();
});

test('leaving the page is refused while the open draft has been edited, with a note; untouched or discarded, it goes', async () => {
  const { root, settle, advance } = await show();
  rows(root)[2].click();
  await settle();
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-compose')!;
  const leave = async () => {
    const click = new MouseEvent('click', { bubbles: true, cancelable: true });
    root.querySelector('nav[data-rail] a[href$="/calendar/"]')!.dispatchEvent(click);
    await settle();
    return click.defaultPrevented;
  };
  expect(dialog.open).toBe(true);
  expect(await leave()).toBe(false);                                   // a reply nobody typed in yet

  const subject = root.querySelector<HTMLInputElement>('#compose-subject')!;
  subject.value = 'Re: First, edited';
  subject.dispatchEvent(new Event('input', { bubbles: true }));
  await settle();
  expect(await leave()).toBe(true);
  expect(dialog.querySelector('[role=alert]')!.textContent).toBe(m.mail_leaveDraft());

  root.querySelector<HTMLButtonElement>(`#email-compose footer button[commandfor="email-compose"]`)!.click();   // Discard
  await settle();
  expect(dialog.open).toBe(false);
  expect(await leave()).toBe(false);

  // a field with its own (debounced) input handler counts too
  rows(root)[2].dispatchEvent(new KeyboardEvent('keydown', { key: 'r', bubbles: true }));
  await settle();
  expect(await leave()).toBe(false);
  const to = root.querySelector<HTMLInputElement>('#compose-to')!;
  to.value += ', b';                                            // a token too short to ask for suggestions
  to.dispatchEvent(new Event('input', { bubbles: true }));
  await advance(200);
  await settle();
  expect(await leave()).toBe(true);
});

test('a draft shared from another page opens compose once, from the default identity, with its attachment', async () => {
  const card = new File(['BEGIN:VCARD'], 'followup-ada.vcf', { type: 'text/vcard' });
  const { root, settle, requests, pendingCalls } = await show({ 'GET /api/email/identities': { identities: [
    { identity_id: 'i1', account_id: 'work', name: 'Me', email: 'me@x', is_default: false, sort_order: 0 },
    { identity_id: 'i2', account_id: 'work', name: 'Also me', email: 'also@x', is_default: true, sort_order: 1 },
  ] } }, { subject: 'followup-Ada', files: [card] });
  const dialog = root.querySelector<HTMLDialogElement>('#email-compose')!;
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
  expect(root.querySelector<HTMLDialogElement>('#email-compose')!.open).toBe(false);
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

// Without a view in the URL, the message list shows the remembered one (its URL reads only the URL and the
// remembered view, so sinter's early file can request it): the view the lists settle on is remembered and put
// into the URL, a remembered one is requested at once, a gone one is replaced.
const listUrls = (requests: { calls: { path: string; url: string }[] }) => requests.calls.filter((c) => c.path === '/api/email/messages').map((c) => c.url);
const remembered = (storage: Storage) => JSON.parse(storage.getItem('sn:mail') ?? '{}').view;

test('the inbox the page settles on is remembered for the next visit', async () => {
  const { storage, requests } = await show();
  expect(remembered(storage)).toEqual({ account_id: 'work', mailbox: 'INBOX', folder_id: null });
  expect(listUrls(requests)).toEqual(['/api/email/messages?account_id=work&mailbox=INBOX&offset=0&limit=50']);
});

test('a remembered mailbox is the first list requested', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:mail', JSON.stringify({ view: { account_id: 'work', mailbox: 'Archive', folder_id: null }, collapsed: [] }));
  const { root, requests } = await show({}, null, storage);
  expect(listUrls(requests)[0]).toBe('/api/email/messages?account_id=work&mailbox=Archive&offset=0&limit=50');
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Archive');
});

test('a remembered folder that is gone gives way to the inbox', async () => {
  const storage = memoryStorage();
  storage.setItem('sn:mail', JSON.stringify({ view: { account_id: null, mailbox: null, folder_id: 'gone' }, collapsed: [] }));
  const { root, requests } = await show({ 'GET /api/email/search': { messages: [] } }, null, storage);
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Inbox');
  expect(listUrls(requests).at(-1)).toBe('/api/email/messages?account_id=work&mailbox=INBOX&offset=0&limit=50');
  expect(remembered(storage)).toEqual({ account_id: 'work', mailbox: 'INBOX', folder_id: null });
});

// The URL holds what is shown: the view, the message open, the page and the search (src/lib/mail.ts).
const query = (url: string) => Object.fromEntries(new URL(url).searchParams);

test('a link names the view, the message open and the page', async () => {
  const { root, requests, opened } = await show({}, null, memoryStorage(), 'http://localhost/mail/?account_id=work&mailbox=Archive&uid=2&page=2');
  expect(listUrls(requests)[0]).toBe('/api/email/messages?account_id=work&mailbox=Archive&offset=50&limit=50');
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Archive');
  expect(opened).toContainEqual({ account_id: 'work', mailbox: 'Archive', uid: 2 });
  expect(root.querySelector('.message-detail')!.classList.contains('is-open')).toBe(true);
});

test('without a view in the URL, the one shown is put into it, in place', async () => {
  const { url, history } = await show();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'INBOX' });
  expect(history()).toHaveLength(1);
});

test('a message and a mailbox are history entries: Back returns to the message, then closes it', async () => {
  const { root, settle, url, back } = await show();
  rows(root)[1].click();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: '2' });
  expect(root.querySelector('.message-detail')!.classList.contains('is-open')).toBe(true);
  [...root.querySelectorAll<HTMLButtonElement>('.sidebar-item')].find((b) => b.textContent!.includes('Archive'))!.click();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'Archive' });
  expect(root.querySelector('.message-list-header h2')!.textContent).toBe('Archive');

  back();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'INBOX', uid: '2' });
  expect(root.querySelector('.reader .detail-subject')!.textContent).toBe('Second');
  back();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'INBOX' });
  expect(root.querySelector('.message-detail')!.classList.contains('is-open')).toBe(false);
});

test('the search and its filters are in the URL; another mailbox drops them', async () => {
  const { root, settle, advance, url, requests } = await show({ 'GET /api/email/search': { messages: [msg(2, 'Second')] } });
  const search = root.querySelector<HTMLInputElement>('.message-list-header [data-field] > input[type=search]')!;
  search.value = 'sec';
  search.dispatchEvent(new Event('input', { bubbles: true }));
  await advance(300);
  await settle();
  root.querySelector<HTMLButtonElement>(`button[aria-label="${m.mail_filterUnread()}"]`)!.click();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'INBOX', q: 'sec', unread: '1' });
  expect(requests.calls.at(-1)!.url).toContain('/api/email/search?q=sec&account_id=work&mailbox=INBOX&unread_only=true');

  [...root.querySelectorAll<HTMLButtonElement>('.sidebar-item')].find((b) => b.textContent!.includes('Archive'))!.click();
  await settle();
  expect(query(url())).toEqual({ account_id: 'work', mailbox: 'Archive' });
  expect(search.value).toBe('');
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

test('adding an account the address\'s domain has a JMAP server for: JMAP, signed in with the user\'s OIDC sign-in, no password', async () => {
  discovery = { found: false, jmap_url: 'https://example.com/.well-known/jmap' };
  const { root, settle, requests } = await show({
    'GET /api/user': { ...user, oidc_enabled: true, oidc_signed_in: true },
    'POST /api/email/account': { account_id: 'new' },
  });
  const field = (sel: string, value: string) => {
    const el = root.querySelector<HTMLInputElement | HTMLSelectElement>(sel)!;
    el.value = value;
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  };
  [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === m.mail_addAccount())!.click();
  await settle();
  field('#email-account input[type="email"]', 'me@example.com');
  field('#email-account input[required]:not([type])', 'Me');
  root.querySelector<HTMLFormElement>('#email-account form')!.requestSubmit();
  await settle();

  const dialog = root.querySelector<HTMLDialogElement>('#email-account')!;
  expect(dialog.querySelector<HTMLSelectElement>('select')!.value).toBe('jmap');
  expect(dialog.querySelector('input[type="password"]')).toBeNull();
  expect(dialog.textContent).toContain(m.mail_oidcSignInHint());
  expect(dialog.textContent).not.toContain(m.mail_imapHost());
  dialog.querySelector<HTMLFormElement>('form')!.requestSubmit();
  await settle();
  const saved = requests.calls.find((c) => c.method === 'POST' && c.path === '/api/email/account')!.body;
  expect(saved).toMatchObject({ protocol: 'jmap', jmap_url: 'https://example.com/.well-known/jmap', auth: 'oidc', password: '', username: 'me@example.com', sender_name: 'Me' });
  expect(dialog.open).toBe(false);
});

test('without an OIDC sign-in a JMAP account asks for a password or an API token', async () => {
  discovery = { found: false, jmap_url: 'https://example.com/.well-known/jmap' };
  const { root, settle } = await show();
  [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === m.mail_addAccount())!.click();
  await settle();
  const email = root.querySelector<HTMLInputElement>('#email-account input[type="email"]')!;
  email.value = 'me@example.com';
  email.dispatchEvent(new Event('input', { bubbles: true }));
  const name = root.querySelector<HTMLInputElement>('#email-account input[required]:not([type])')!;
  name.value = 'Me';
  name.dispatchEvent(new Event('input', { bubbles: true }));
  root.querySelector<HTMLFormElement>('#email-account form')!.requestSubmit();
  await settle();
  const dialog = root.querySelector<HTMLDialogElement>('#email-account')!;
  const auth = dialog.querySelectorAll<HTMLSelectElement>('select')[1]!;
  expect([...auth.options].map((o) => o.value)).toEqual(['password', 'bearer', 'oauth']);
  expect(dialog.querySelector('input[type="password"]')).not.toBeNull();
});

test('an account that signs in at the server: its OAuth client, then saving goes to the server\'s sign-in', async () => {
  discovery = { found: false, jmap_url: 'https://example.com/.well-known/jmap' };
  const { root, settle, requests, navigations } = await show({
    'POST /api/email/account': { account_id: 'new', authorize: '/auth/jmap?start=abc' },
  });
  const field = (el: HTMLInputElement | HTMLSelectElement, value: string) => {
    el.value = value;
    el.dispatchEvent(new Event(el instanceof HTMLSelectElement ? 'change' : 'input', { bubbles: true }));
  };
  [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === m.mail_addAccount())!.click();
  await settle();
  field(root.querySelector<HTMLInputElement>('#email-account input[type="email"]')!, 'me@example.com');
  field(root.querySelector<HTMLInputElement>('#email-account input[required]:not([type])')!, 'Me');
  root.querySelector<HTMLFormElement>('#email-account form')!.requestSubmit();
  await settle();

  const dialog = root.querySelector<HTMLDialogElement>('#email-account')!;
  field(dialog.querySelectorAll<HTMLSelectElement>('select')[1]!, 'oauth');
  await settle();
  expect(dialog.querySelector('input[type="password"][autocomplete="current-password"]')).toBeNull();
  expect(dialog.textContent).toContain('/auth/jmap');
  const [clientId, secret] = [...dialog.querySelectorAll<HTMLInputElement>('input[autocomplete="off"]')];
  field(clientId!, 'caley-client');
  field(secret!, 's3cret');
  dialog.querySelector<HTMLFormElement>('form')!.requestSubmit();
  await settle();
  const saved = requests.calls.find((c) => c.method === 'POST' && c.path === '/api/email/account')!.body;
  expect(saved).toMatchObject({ protocol: 'jmap', auth: 'oauth', oauth_client_id: 'caley-client', oauth_client_secret: 's3cret', password: '' });
  expect(navigations.at(-1)).toContain('/auth/jmap?start=abc');
});

test('a failed sign-in at the mail server comes back as an error above the mail, until closed', async () => {
  // a visit before: the page's remembered state must not hide the error
  const storage = memoryStorage();
  (await show({}, null, storage)).root.remove();
  const { root, settle } = await show({}, null, storage, 'http://localhost/mail/?account_error=the%20server%20said%20no');
  const alert = root.querySelector('.account-error')!;
  expect(alert.textContent).toContain(m.mail_accountError({ error: 'the server said no' }));
  alert.querySelector<HTMLButtonElement>('button')!.click();
  await settle();
  expect(root.querySelector('.account-error')).toBeNull();
});

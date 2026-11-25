import '../test-messages.ts';
// The mail page's state helpers (src/lib/mail.ts "Page state", keys) and the settings helpers they sit next to.
import { describe, expect, test } from 'vitest';
import {
  accountUi, addableSender, afterBulk, bodyOf, canDrop, cleared, currentView, folderView, inMailbox, mailboxView, noView, listStatus, markedRead, noReader, noSearch,
  noSelection, openedAt, opensRow, pageKey, readerOn, rowClick, scrollTopFor, searchActive, senderAdded, sharedDraft,
  siblingRow, unmarked, urlRef, urlView, shownView, viewHref, urlLags, noSearchBox, mailSearch, newAccount, editAccount, discovered,
  accountBody, needsPassword, signsInAtServer, replyDraft, forwardDraft, missingKeys, type Account, type Mailbox, type Message, type VirtualFolder,
} from './mail.ts';
import { caldavBody, caldavDraft, enrollErrorText, keyImportError, withCalendarActive } from './settings.ts';
import { loadErrorOr } from './errors.ts';
import { m } from '@tiaforge-ui/sinter/messages';
import { importSummary } from './import.ts';

const msg = (uid: number, extra: Partial<Message> = {}) => ({ uid, account_id: 'a', mailbox: 'INBOX', flags: [], ...extra }) as unknown as Message;
const list = [msg(1), msg(2), msg(3)];
const key = (uid: number) => `a INBOX ${uid}`;
const click = (init: Partial<MouseEvent> = {}) => ({ ctrlKey: false, metaKey: false, shiftKey: false, ...init });

describe('selection', () => {
  test('a plain click moves the cursor and unmarks; Ctrl marks one, Shift a range', async () => {
    const { messageKey } = await import('./mail.ts');
    const keys = list.map(messageKey);
    let sel = rowClick(click({ ctrlKey: true }), noSelection(), keys[0]!, list);
    expect([sel.keys, sel.anchor]).toEqual([[keys[0]], keys[0]]);
    sel = rowClick(click({ shiftKey: true }), { ...sel, error: 'x', move: true }, keys[2]!, list);
    expect([sel.keys, sel.error, sel.move]).toEqual([keys, '', false]);
    sel = rowClick(click({ metaKey: true }), sel, keys[1]!, list);
    expect(sel.keys).toEqual([keys[0], keys[2]]);
    sel = rowClick(click(), sel, keys[1]!, list);
    expect(sel).toMatchObject({ keys: [], anchor: null, cursor: keys[1] });
    expect([opensRow(click() as MouseEvent, sel), opensRow(click({ shiftKey: true }) as MouseEvent, sel)]).toEqual([true, false]);
  });

  test('selection mode: a plain tap marks and unmarks, opens nothing; an action that fails for none ends it', async () => {
    const { messageKey, selecting, allMarked, bulkBack, afterBulk, cleared } = await import('./mail.ts');
    const keys = list.map(messageKey);
    let sel = selecting(noSelection());
    sel = rowClick(click(), sel, keys[1]!, list);
    sel = rowClick(click(), sel, keys[2]!, list);
    expect([sel.selecting, sel.keys]).toEqual([true, [keys[1], keys[2]]]);
    expect(opensRow(click() as MouseEvent, sel)).toBe(false);
    sel = rowClick(click(), sel, keys[1]!, list);
    expect(sel.keys).toEqual([keys[2]]);
    expect(allMarked(sel, list).keys).toEqual(keys);
    expect(bulkBack({ ...sel, more: true })).toMatchObject({ selecting: true, more: false, keys: [keys[2]] });
    expect(bulkBack({ ...sel, selecting: false })).toMatchObject({ keys: [] });
    expect(afterBulk({ ...sel, more: true }, [list[2]!])).toMatchObject({ selecting: true, more: true, keys: [keys[2]] });
    expect(afterBulk({ ...sel, more: true }, [])).toMatchObject({ selecting: false, more: false, keys: [] });
    expect([unmarked(sel).selecting, cleared(sel).selecting]).toEqual([false, false]);
  });
  test('with nothing marked, the open message counts as the first one marked', () => {
    const open = list[0]!;
    expect(rowClick(click({ ctrlKey: true }), noSelection(), key(3), list, open)).toMatchObject({ keys: [key(1), key(3)], anchor: key(3) });
    expect(rowClick(click({ shiftKey: true }), noSelection(), key(3), list, open)).toMatchObject({ keys: [key(1), key(2), key(3)], anchor: key(1) });
    expect(rowClick(click({ ctrlKey: true }), noSelection(), key(1), list, open).keys).toEqual([key(1)]);
    // Marks already there are kept as they are; an open message not in the list is not added.
    expect(rowClick(click({ ctrlKey: true }), { ...noSelection(), keys: [key(2)] }, key(3), list, open).keys).toEqual([key(2), key(3)]);
    expect(rowClick(click({ ctrlKey: true }), noSelection(), key(3), list, msg(9)).keys).toEqual([key(3)]);
  });
  test('unmarked keeps the anchor and the menu; cleared drops them; both end selection mode; openedAt puts the cursor on the open one', () => {
    const sel = { cursor: 'c', keys: ['k'], anchor: 'k', error: 'e', move: true, busy: true, selecting: true, more: true };
    expect(unmarked(sel)).toEqual({ ...sel, keys: [], selecting: false, more: false });
    expect(cleared(sel)).toEqual({ cursor: 'c', keys: [], anchor: null, error: '', move: false, busy: true, selecting: false, more: false });
    expect(cleared(sel, null).cursor).toBeNull();
    expect(openedAt(sel, 'x')).toMatchObject({ cursor: 'x', keys: [] });
  });
  test('after a bulk action the failed ones stay marked, with the error', () => {
    expect(afterBulk({ ...noSelection(), busy: true, move: true }, [])).toEqual(noSelection());
    const ref = (uid: number) => ({ account_id: 'a', mailbox: 'INBOX', uid });
    expect(afterBulk(noSelection(), [ref(1), ref(2)])).toMatchObject({ keys: [key(1), key(2)], anchor: key(1), busy: false });
    expect(afterBulk(noSelection(), [ref(1)]).error).not.toBe('');
  });
});

describe('search', () => {
  test('active with a query or a filter', () => {
    expect(searchActive(noSearch())).toBe(false);
    expect(searchActive({ ...noSearch(), q: ' x ' })).toBe(true);
    expect(searchActive({ ...noSearch(), filter: { ...noSearch().filter, unread: true } })).toBe(true);
  });
  test('a typed sender is added once, and the box empties either way', () => {
    const s = { ...noSearch(), addrInput: ' ada@x ', addrQ: 'ada' };
    expect(addableSender(s)).toBe('ada@x');
    const added = senderAdded(s);
    expect([added.filter.addrs, added.addrInput, added.addrQ]).toEqual([['ada@x'], '', '']);
    expect(addableSender({ ...added, addrInput: 'ada@x' })).toBe('');
    expect(senderAdded({ ...added, addrInput: 'ada@x' }).filter.addrs).toEqual(['ada@x']);
  });
  test('listStatus says why the list is empty', () => {
    const view = { account_id: null, mailbox: null, folder_id: null };
    expect(listStatus(true, 'q', 1, view)).toContain('q');
    expect(listStatus(false, '', 0, view)).not.toBe(listStatus(false, '', 1, view));
    expect(listStatus(false, '', 1, view)).not.toBe(listStatus(false, '', 1, { ...view, account_id: 'a', mailbox: 'INBOX' }));
  });
});

describe('view', () => {
  const accounts = [{ account_id: 'a' }, { account_id: 'b' }] as Account[];
  const mbs = (account_id: string, ...names: string[]) => names.map((name) => ({ account_id, name })) as Mailbox[];
  const folders = [{ folder_id: 'f' }] as VirtualFolder[];
  const none = { accounts: false, folders: false };
  test('the remembered view is shown before the lists that could disprove it have arrived', () => {
    expect(currentView(mailboxView('b', 'Sent'), [], [], [], none)).toEqual(mailboxView('b', 'Sent'));
    expect(currentView(mailboxView('b', 'Sent'), accounts, mbs('a', 'INBOX'), [], none)).toEqual(mailboxView('b', 'Sent'));
    expect(currentView(folderView('x'), accounts, mbs('a', 'INBOX'), [], none)).toEqual(folderView('x'));
    expect(currentView(noView(), accounts, [], [], none)).toEqual(mailboxView('a', 'INBOX'));
    expect(currentView(noView(), accounts, mbs('a', 'Inbox'), [], none)).toEqual(mailboxView('a', 'Inbox'));
  });
  test('once they have, a view that is gone falls back to the first inbox', () => {
    const loaded = { accounts: true, folders: true };
    const all = [...mbs('a', 'Drafts', 'INBOX'), ...mbs('b', 'INBOX')];
    expect(currentView(mailboxView('b', 'Sent'), accounts, all, folders, loaded)).toEqual(mailboxView('a', 'INBOX'));
    expect(currentView(mailboxView('c', 'INBOX'), accounts, all, folders, loaded)).toEqual(mailboxView('a', 'INBOX'));
    expect(currentView(folderView('x'), accounts, all, folders, loaded)).toEqual(mailboxView('a', 'INBOX'));
    expect(currentView(folderView('f'), accounts, all, folders, loaded)).toEqual(folderView('f'));
    expect(currentView(noView(), accounts, all, folders, loaded)).toEqual(mailboxView('a', 'INBOX'));
    expect(currentView(noView(), [], [], [], loaded)).toEqual(noView());
  });
});

describe('reader and list', () => {
  test('readerOn shows a message by its ref; inMailbox compares the view', () => {
    expect(readerOn(msg(2))).toEqual({ ref: { account_id: 'a', mailbox: 'INBOX', uid: 2 }, shown: true });
    expect(readerOn(null)).toEqual(noReader());
    expect(inMailbox(msg(2), { account_id: 'a', mailbox: 'INBOX', folder_id: null })).toBe(true);
    expect(inMailbox(msg(2), { account_id: 'a', mailbox: 'Trash', folder_id: null })).toBe(false);
    expect(inMailbox(null, { account_id: 'a', mailbox: 'INBOX', folder_id: null })).toBe(false);
  });
  test('bodyOf is the body of that message only', () => {
    expect(bodyOf({ key: key(2) }, msg(2))).toEqual({ key: key(2) });
    expect(bodyOf({ key: key(1) }, msg(2))).toBeNull();
    expect(bodyOf(null, msg(2))).toBeNull();
  });
  test('canDrop to another mailbox of the same account; markedRead adds \\Seen', () => {
    const mb = (account_id: string, name: string) => ({ account_id, name }) as never;
    expect([canDrop(msg(1), mb('a', 'Archive')), canDrop(msg(1), mb('a', 'INBOX')), canDrop(msg(1), mb('b', 'X')), canDrop(null, mb('a', 'X'))])
      .toEqual([true, false, false, false]);
    expect(markedRead({ flags: ['\\Flagged'] })).toEqual({ flags: ['\\Flagged', '\\Seen'], is_read: true, is_flagged: true });
  });
  test('sharedDraft: the shared subject and files, from the default identity', () => {
    const file = new File(['x'], 'a.vcf');
    const d = sharedDraft({ subject: 'Hi', files: [file] }, [{ identity_id: 'i1', is_default: false }, { identity_id: 'i2', is_default: true }] as never);
    expect([d.mode, d.subject, d.files, d.identity_id]).toEqual(['new', 'Hi', [file], 'i2']);
  });
});

describe('keys', () => {
  test('pageKey: not from a dialog or with Ctrl', () => {
    document.body.innerHTML = '<div class="page"><button></button><dialog><button></button></dialog></div>';
    const [page, inDialog] = [...document.querySelectorAll('button')];
    const at = (el: Element, init: KeyboardEventInit = {}) => { const e = new KeyboardEvent('keydown', { key: 'j', ...init }); el.dispatchEvent(e); return e; };
    expect(pageKey(at(page!))).toBe(true);
    expect(pageKey(at(page!, { ctrlKey: true }))).toBe(false);
    document.querySelector('dialog')!.setAttribute('open', '');
    expect(pageKey(at(inDialog!))).toBe(false);
    expect(pageKey(at(page!))).toBe(false);                        // a dialog is open
  });
  test('scrollTopFor: Home, End and 90% pages', () => {
    const el = { scrollTop: 100, clientHeight: 200, scrollHeight: 1000 } as HTMLElement;
    expect(['Home', 'End', 'PageDown', 'PageUp'].map((k) => scrollTopFor(k, el))).toEqual([0, 1000, 280, -80]);
    expect(scrollTopFor('End', null)).toBe(0);
  });
  test('siblingRow: the neighbouring message rows only', () => {
    document.body.innerHTML = '<p></p><button class="message-row" id="a"></button><button class="message-row" id="b"></button>';
    const [a, b] = [document.getElementById('a')!, document.getElementById('b')!];
    expect([siblingRow(a, 1)?.id, siblingRow(b, -1)?.id, siblingRow(b, 1), siblingRow(a, -1)]).toEqual(['b', 'a', null, null]);
  });
});

describe('dialogs', () => {
  test('accountUi: basic, details after autodiscovery (advanced unless found)', () => {
    expect(accountUi('basic')).toEqual({ step: 'basic', discovery: null, advanced: false, error: '', busy: false });
    expect(accountUi('details').advanced).toBe(true);
    expect(accountUi('details', { found: true } as never).advanced).toBe(false);
    expect(accountUi('details', { found: false, jmap_url: 'https://x/.well-known/jmap' }).advanced).toBe(false);
  });
  test('autodiscovery takes a JMAP server over IMAP, with the OIDC sign-in when there is one', () => {
    const form = { ...newAccount(), username: 'me@example.com' };
    const imapAndJmap = { found: true, jmap_url: 'https://example.com/.well-known/jmap', provider_name: 'Ex', imap_host: 'imap.example.com',
      imap_port: 993, imap_use_tls: true, imap_username: 'me@example.com', smtp_host: 'smtp.example.com', smtp_port: 587 };
    expect(discovered(form, imapAndJmap, true)).toMatchObject({ protocol: 'jmap', jmap_url: imapAndJmap.jmap_url, auth: 'oidc', host: 'imap.example.com', display_name: 'Ex' });
    expect(discovered(form, imapAndJmap, false)).toMatchObject({ protocol: 'jmap', auth: 'password' });
    expect(discovered(form, { found: true, imap_host: 'imap.example.com' }, true)).toMatchObject({ protocol: 'imap', auth: 'password', host: 'imap.example.com' });
    expect(discovered(form, null)).toMatchObject({ protocol: 'imap', display_name: 'example.com' });
  });
  test('an account body: IMAP with its SMTP server, JMAP with its URL and sign-in and no password for OIDC', () => {
    const imap = { ...newAccount(), display_name: 'Work', username: 'me@x', password: 'pw', host: 'imap.x', smtp_host: 'smtp.x' };
    expect(accountBody(imap)).toEqual({
      account_id: null, display_name: 'Work', sender_name: null, protocol: 'imap', jmap_url: null, auth: 'password', host: 'imap.x', port: 993,
      username: 'me@x', password: 'pw', use_tls: true, smtp_host: 'smtp.x', smtp_port: 587,
    });
    const jmap = { ...imap, protocol: 'jmap' as const, jmap_url: 'mail.x', auth: 'oidc' as const };
    expect(needsPassword(jmap)).toBe(false);
    expect(accountBody(jmap)).toMatchObject({ protocol: 'jmap', jmap_url: 'mail.x', auth: 'oidc', password: '', host: '', port: 443, smtp_host: null, smtp_port: null });
    expect(needsPassword({ ...jmap, auth: 'bearer' })).toBe(true);
    expect(accountBody({ ...jmap, auth: 'bearer' }).password).toBe('pw');
  });
  test('an account that signs in at the server sends its OAuth client, and no password', () => {
    const f = { ...newAccount(), protocol: 'jmap' as const, jmap_url: 'mail.x', auth: 'oauth' as const, username: 'me@x', password: 'stale',
      oauth_client_id: 'caley', oauth_client_secret: '', authorize: false };
    expect(signsInAtServer(f)).toBe(true);
    expect(needsPassword(f)).toBe(false);
    expect(accountBody(f)).toMatchObject({ auth: 'oauth', password: '', oauth_client_id: 'caley', oauth_client_secret: null, authorize: false });
    expect(accountBody({ ...f, auth: 'password' })).not.toHaveProperty('oauth_client_id');
  });
  test('editing an account keeps its protocol, URL and sign-in', () => {
    const acc = { account_id: 'a', display_name: 'Stalwart', protocol: 'jmap', jmap_url: 'https://mail.x/.well-known/jmap', auth: 'oidc', host: 'mail.x',
      port: 443, username: 'me@x', use_tls: true, smtp_host: null, smtp_port: null, oauth_client_id: null } as Account;
    expect(editAccount(acc, [])).toMatchObject({ protocol: 'jmap', jmap_url: 'https://mail.x/.well-known/jmap', auth: 'oidc', password: '' });
  });
  test('the CalDAV account draft and body, the calendar toggle, the import summary and the PGP errors', () => {
    const account = { account_id: 'x', display_name: 'Work', caldav_base_url: 'https://dav', caldav_username: 'me', has_password: true, sort_order: 0, calendars: [
      { href: '/a/', is_active: true }, { href: '/b/', is_active: false }], addressbooks: [] } as never;
    expect(caldavDraft(account)).toEqual({ account_id: 'x', display_name: 'Work', caldav_base_url: 'https://dav', caldav_username: 'me', caldav_password: '', has_password: true });
    expect(caldavDraft(null).account_id).toBe('');
    expect(caldavBody(caldavDraft(null)).account_id).toBeNull();
    expect(withCalendarActive(account, '/b/', true).calendars.map((c) => c.is_active)).toEqual([true, true]);
    expect(importSummary({ imported: 2, errors: 0 })).not.toContain('·');
    expect(importSummary({ imported: 2, errors: 1 })).toContain('·');
    expect(keyImportError({ detail: 'Argon2 not supported', message: '' })).not.toContain('Argon2 not supported');
    expect(keyImportError({ detail: null, message: 'bad key' })).toContain('bad key');
    expect(enrollErrorText({ detail: 'other', message: '' })).toContain('other');
    expect(enrollErrorText({ detail: 'prf_not_supported', message: '' })).not.toContain('prf_not_supported');
    // a stale page (openpgp gone from the server after a deploy) says to reload instead
    expect(keyImportError({ detail: null, message: 'out of date', stale: true })).toBe(m.pgp_loadError());
    expect(enrollErrorText({ detail: null, message: 'out of date', stale: true })).toBe(m.pgp_loadError());
    expect(loadErrorOr({ stale: true }, 'Unlock failed')).toBe(m.pgp_loadError());
    expect(loadErrorOr({ stale: false }, 'Unlock failed')).toBe('Unlock failed');
    expect(loadErrorOr({}, 'Unlock failed')).toBe('Unlock failed');
  });
});

describe('the mail page URL', () => {
  test('urlRef and urlView read the parameters; shownView falls back to the remembered view', () => {
    expect(urlRef('a', 'INBOX', '3')).toEqual({ account_id: 'a', mailbox: 'INBOX', uid: 3 });
    expect(urlRef('a', 'INBOX', 'x')).toBeNull();
    expect(urlRef(undefined, 'INBOX', '3')).toBeNull();
    expect(urlView('f1', 'a', 'INBOX')).toEqual(folderView('f1'));   // a folder wins: account_id/mailbox are the message's
    expect(urlView(undefined, 'a', 'INBOX')).toEqual(mailboxView('a', 'INBOX'));
    expect(urlView(undefined, 'a', undefined)).toBeNull();
    expect(shownView(null, mailboxView('a', 'Archive'))).toEqual(mailboxView('a', 'Archive'));
    expect(shownView(folderView('f1'), mailboxView('a', 'Archive'))).toEqual(folderView('f1'));
  });

  test('viewHref names only the view; urlLags says when the URL must follow', () => {
    expect(viewHref(mailboxView('a b', 'INBOX/Sub'))).toBe('?account_id=a+b&mailbox=INBOX%2FSub');
    expect(viewHref(folderView('f1'))).toBe('?folder=f1');
    expect(viewHref(noView())).toBe('?');
    expect(urlLags(mailboxView('a', 'INBOX'), null)).toBe(true);
    expect(urlLags(mailboxView('a', 'INBOX'), mailboxView('a', 'INBOX'))).toBe(false);
    expect(urlLags(mailboxView('a', 'INBOX'), folderView('gone'))).toBe(true);
    expect(urlLags(noView(), null)).toBe(false);
  });

  test('mailSearch puts the URL query and filters together with the sender box', () => {
    const s = mailSearch({ addrInput: 'ad', addrQ: 'ad' }, 'invoice', ['ada@x', 'bob@y'], '2026-01-01', undefined, true);
    expect(s).toEqual({ input: 'invoice', q: 'invoice', filter: { addrs: ['ada@x', 'bob@y'], from: '2026-01-01', to: '', unread: true }, addrInput: 'ad', addrQ: 'ad' });
    expect(searchActive(mailSearch(noSearchBox(), undefined, [], undefined, undefined, false))).toBe(false);
  });
});

describe('replies and forwards of encrypted mail', () => {
  const ids = [{ identity_id: 'i1', account_id: 'a', name: 'Me', email: 'me@x', is_default: true, sort_order: 0 }] as never;
  const plain = { subject: '', reply: null, text: 'hello', quotable: true, decrypted: false };
  const secret = { subject: 'Secret plan', reply: ['Alice <alice@x>'], text: 'the plan', quotable: true, decrypted: true };
  const from = msg(1, { from: 'Alice <alice@x>', to: ['me@x'], cc: [], subject: '...', message_id: '<1@x>', attachments: [] });
  test('a reply or forward of decrypted content starts encrypted, and says it quotes decrypted text', () => {
    expect(replyDraft('reply', from, secret, ids)).toMatchObject({ encrypt: true, decrypted: true });
    expect(replyDraft('replyAll', from, secret, ids)).toMatchObject({ encrypt: true, decrypted: true });
    expect(forwardDraft(from, secret, ids, null, [])).toMatchObject({ encrypt: true, decrypted: true });
    expect(replyDraft('reply', from, plain, ids)).toMatchObject({ encrypt: false, decrypted: false });
    expect(forwardDraft(from, null, ids, null, [])).toMatchObject({ encrypt: false, decrypted: false });
  });
  test('missingKeys: the recipients and the identity an encrypted draft has no trusted key for', () => {
    const keys = { keys: [{ fingerprint: 'A', emails: ['alice@x'], is_own: false }, { fingerprint: 'M', emails: ['me@x'], is_own: true }], pending: [], vault: [], unlocked: [] };
    const draft = { ...replyDraft('reply', from, secret, ids), cc: 'Bob <bob@x>' };
    expect(missingKeys(draft, ids, keys)).toEqual(['bob@x']);
    expect(missingKeys(draft, ids, { ...keys, keys: keys.keys.slice(0, 1) })).toEqual(['bob@x', 'me@x']);
    expect(missingKeys({ ...draft, cc: '' }, ids, keys)).toEqual([]);
  });
});

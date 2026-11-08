// The mail page's state helpers (src/lib/mail.ts "Page state", keys) and the settings helpers they sit next to.
import { describe, expect, test } from 'vitest';
import {
  accountUi, addableSender, afterBulk, bodyOf, canDrop, cleared, inMailbox, listStatus, markedRead, noReader, noSearch,
  noSelection, openedAt, opensRow, pageKey, readerOn, rowClick, scrollTopFor, searchActive, senderAdded, sharedDraft,
  siblingRow, unmarked, type Message,
} from './mail.ts';
import { caldavBody, caldavDraft, enrollErrorText, keyImportError, withCalendarActive } from './settings.ts';
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
    expect([opensRow(click() as MouseEvent), opensRow(click({ shiftKey: true }) as MouseEvent)]).toEqual([true, false]);
  });
  test('unmarked keeps the anchor and the menu; cleared drops them; openedAt puts the cursor on the open one', () => {
    const sel = { cursor: 'c', keys: ['k'], anchor: 'k', error: 'e', move: true, busy: true };
    expect(unmarked(sel)).toEqual({ ...sel, keys: [] });
    expect(cleared(sel)).toEqual({ cursor: 'c', keys: [], anchor: null, error: '', move: false, busy: true });
    expect(cleared(sel, null).cursor).toBeNull();
    expect(openedAt(sel, 'x')).toMatchObject({ cursor: 'x', keys: [] });
  });
  test('after a bulk action the failed ones stay marked, with the error', () => {
    expect(afterBulk({ ...noSelection(), busy: true, move: true }, [])).toEqual(noSelection());
    expect(afterBulk(noSelection(), ['k1', 'k2'])).toMatchObject({ keys: ['k1', 'k2'], anchor: 'k1', busy: false });
    expect(afterBulk(noSelection(), ['k1']).error).not.toBe('');
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
  test('pageKey: not with marks, from a dialog or with Ctrl', () => {
    document.body.innerHTML = '<div class="page"><button></button><dialog><button></button></dialog></div>';
    const [page, inDialog] = [...document.querySelectorAll('button')];
    const at = (el: Element, init: KeyboardEventInit = {}) => { const e = new KeyboardEvent('keydown', { key: 'j', ...init }); el.dispatchEvent(e); return e; };
    expect(pageKey(at(page!), 0)).toBe(true);
    expect(pageKey(at(page!), 2)).toBe(false);
    expect(pageKey(at(page!, { ctrlKey: true }), 0)).toBe(false);
    document.querySelector('dialog')!.setAttribute('open', '');
    expect(pageKey(at(inDialog!), 0)).toBe(false);
    expect(pageKey(at(page!), 0)).toBe(false);                        // a dialog is open
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
  });
});

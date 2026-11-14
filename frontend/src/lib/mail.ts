// Mail page helpers (mail/index.html, mail/view/index.html, src/components/mail-*.html): pure functions the templates
// call by name through src/sinter.helpers.ts. Types of the mail API and the page's state live here too.
import { m } from '@tiaforge-ui/sinter/messages';
import {
  canPreviewAttachment,
  folderKind,
  folderSortKey,
  linkifySegments,
  isPgpSignatureAttachment,
  isPgpTransportAttachment,
} from './mailUtils.ts';
import type { Account, Attachment, Identity, Mailbox, Message, ThreadNode, VirtualFolder } from './MailTypes.ts';
import { parseEnvelopeAddresses } from './addresses.ts';

export type { Account, Attachment, Identity, Mailbox, Message, ThreadNode, VirtualFolder };
export {
  canPreviewAttachment,
  folderKind,
  htmlAddsValue,
  isIcsAttachment,
  formatDate,
  humanSize,
  senderName,
} from './mailUtils.ts';

// ── Types ─────────────────────────────────────────────────────────────────────

/** Where a message lives: what the message, thread and raw endpoints take. */
export interface MessageRef { account_id: string; mailbox: string; uid: number }

/** The list the page shows: one mailbox of an account, or a virtual folder. */
export interface MailView { account_id: string | null; mailbox: string | null; folder_id: string | null }

/** GET /api/email/unread-counts. */
export interface UnreadCounts { mailboxes: Record<string, Record<string, number>>; folders: Record<string, number> }

export interface MailboxesResponse { mailboxes: Mailbox[] }
export interface MessagesResponse { messages: Message[]; total?: number }

/** sync_started / sync_completed / emails_refetch / mailboxes_changed payloads. */
export interface MailboxEvent { account_id: string; mailbox: string }
export interface AccountEvent { account_id: string }
/** email_updated on /api/events/stream. */
export interface EmailUpdatedEvent { account_id: string; mailbox: string; uid: number; flags: string[] }
/** email_removed on /api/events/stream. */
export interface EmailRemovedEvent { account_id: string; mailbox: string; uid: number }

export interface SidebarMailbox { mailbox: Mailbox; key: string; unread: number }
export interface SidebarSection { account: Account; collapsed: boolean; unread: number; mailboxes: SidebarMailbox[] }

// ── Views and the sidebar ─────────────────────────────────────────────────────

/** No view chosen (currentView() shows the first inbox). */
export function noView(): MailView {
  return { account_id: null, mailbox: null, folder_id: null };
}

export function mailboxView(account_id: string, mailbox: string): MailView {
  return { account_id, mailbox, folder_id: null };
}

export function folderView(folder_id: string): MailView {
  return { account_id: null, mailbox: null, folder_id };
}

/** Which lists the server has answered on this page load (`!!$meta.x.loadedAt`; cached data does not count). */
export interface ViewLists { accounts: boolean; folders: boolean }

/**
 * The view to show: the remembered one while it still exists, else the first account's inbox
 * (or its first mailbox; `INBOX` before its mailboxes are known), as MailApp did on load. The remembered view is trusted until the lists
 * that could disprove it have arrived (its account's mailboxes, the folders), so its messages load
 * alongside them instead of after them.
 */
export function currentView(view: MailView, accounts: Account[], mailboxes: Mailbox[], folders: VirtualFolder[],
                            loaded: ViewLists = { accounts: true, folders: true }): MailView {
  if (view.folder_id) {
    if (!loaded.folders || folders.some((f) => f.folder_id === view.folder_id)) return view;
  } else if (view.account_id && view.mailbox) {
    const known = accounts.some((a) => a.account_id === view.account_id);
    if (!known && !loaded.accounts) return view;
    const own = known ? mailboxes.filter((mb) => mb.account_id === view.account_id) : [];
    // an account's mailboxes arrive in one response: none yet means not loaded yet
    if (known && (!own.length || own.some((mb) => mb.name === view.mailbox))) return view;
  }
  const first = accounts[0];
  if (!first) return noView();
  const own = mailboxes.filter((mb) => mb.account_id === first.account_id);
  // every IMAP account has an INBOX: assume it until the account's mailboxes have arrived
  if (!own.length) return mailboxView(first.account_id, 'INBOX');
  const inbox = own.find((mb) => mb.name.toUpperCase() === 'INBOX') ?? own[0]!;
  return mailboxView(first.account_id, inbox.name);
}

export function sameView(a: MailView, b: MailView): boolean {
  return a.account_id === b.account_id && a.mailbox === b.mailbox && a.folder_id === b.folder_id;
}

/** The localized name of a well-known folder, else the mailbox name. */
export function folderName(name: string): string {
  switch (folderKind(name)) {
    case 'inbox': return m.mail_folder_inbox();
    case 'sent': return m.mail_folder_sent();
    case 'drafts': return m.mail_folder_drafts();
    case 'trash': return m.mail_folder_trash();
    case 'junk': return m.mail_folder_junk();
    case 'archive': return m.mail_folder_archive();
    default: return name;
  }
}

export function viewTitle(view: MailView, folders: VirtualFolder[]): string {
  if (view.folder_id) return folders.find((f) => f.folder_id === view.folder_id)?.display_name || m.mail_aria_folders();
  if (view.mailbox) return folderName(view.mailbox);
  return m.nav_mail();
}

export function badgeCount(n: number): string {
  return n > 999 ? '999+' : String(n);
}

export function mailboxKey(account_id: string, mailbox: string): string {
  return `${account_id}|${mailbox}`;
}

/** Subscribed mailboxes per account, sorted inbox first, with unread counts; collapsed accounts keep only unread ones. */
export function sidebarSections(accounts: Account[], mailboxes: Mailbox[], unread: UnreadCounts | null, collapsed: string[]): SidebarSection[] {
  return accounts.map((account) => {
    const items = mailboxes
      .filter((mb) => mb.account_id === account.account_id && mb.subscribed)
      .toSorted((a, b) => {
        const [ap, an] = folderSortKey(a.name);
        const [bp, bn] = folderSortKey(b.name);
        return ap !== bp ? ap - bp : an.localeCompare(bn);
      })
      .map((mailbox) => ({
        mailbox,
        key: mailboxKey(account.account_id, mailbox.name),
        unread: unread?.mailboxes?.[account.account_id]?.[mailbox.name] ?? 0,
      }));
    const isCollapsed = collapsed.includes(account.account_id);
    return {
      account,
      collapsed: isCollapsed,
      unread: items.reduce((sum, item) => sum + item.unread, 0),
      mailboxes: isCollapsed ? items.filter((item) => item.unread > 0) : items,
    };
  });
}

/** The page title with the unread count of every mailbox. */
export function mailTitle(unread: UnreadCounts | null): string {
  let total = 0;
  for (const boxes of Object.values(unread?.mailboxes ?? {})) for (const n of Object.values(boxes)) total += n;
  return total > 0 ? `(${badgeCount(total)}) Caley - Mail` : 'Caley - Mail';
}

export function toggled(list: string[], value: string): string[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

// ── Message list ──────────────────────────────────────────────────────────────

export function refOf(msg: MessageRef): MessageRef {
  return { account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid };
}

export function isRef(msg: MessageRef | null, ref: MessageRef | null): boolean {
  return !!msg && !!ref && msg.account_id === ref.account_id && msg.mailbox === ref.mailbox && msg.uid === ref.uid;
}

// ── The mail page's URL ────────────────────────────────────────────────────────
// The list shown, the message open, the page and the search live in the URL, so a reload, Back and Forward and a
// shared link show the same: `?account_id=&mailbox=` (a mailbox) or `?folder=` (a virtual folder), `uid` (the
// message open: in `account_id`'s `mailbox`, which in a virtual folder is the message's own), `page`, and the
// search `q`, `sender` (addresses, comma-separated), `after`, `before`, `unread`. Pages read the parameters by name
// (`$url.uid`), never `$url` whole: a page that reads it whole cannot follow its own links in the page.

/** The message `?account_id=&mailbox=&uid=` names (a notification, the popout, the open message), or null. */
export function urlRef(account_id: string | undefined, mailbox: string | undefined, uid: string | undefined): MessageRef | null {
  const n = Number(uid);
  return account_id && mailbox && Number.isInteger(n) && n > 0 ? { account_id, mailbox, uid: n } : null;
}

/** The view the URL names (`folder`, else `account_id` + `mailbox`), or null. */
export function urlView(folder: string | undefined, account_id: string | undefined, mailbox: string | undefined): MailView | null {
  if (folder) return folderView(folder);
  return account_id && mailbox ? mailboxView(account_id, mailbox) : null;
}

/** The view shown: the URL's, else the one remembered from the last visit (`last`). */
export function shownView(url: MailView | null, last: MailView): MailView {
  return url ?? last;
}

/** This page's URL showing `view`: its own parameters only, so no message, page or search. */
export function viewHref(view: MailView): string {
  const p = new URLSearchParams();
  if (view.folder_id) p.set('folder', view.folder_id);
  else if (view.account_id && view.mailbox) {
    p.set('account_id', view.account_id);
    p.set('mailbox', view.mailbox);
  }
  return '?' + p.toString();
}

/** Whether the URL must change to show `view` (a real view the URL does not name: none, or another one). */
export function urlLags(view: MailView, url: MailView | null): boolean {
  return !!(view.folder_id || (view.account_id && view.mailbox)) && !(url && sameView(view, url));
}

/** The `sender` parameter for a list of addresses: null for none. */
export function senderParam(addrs: readonly string[]): string | null {
  return addrs.length ? addrs.join(',') : null;
}

/** The sender filter box (what is typed, and what the suggestions are asked for); the rest of the search is in the URL. */
export interface SearchBox { addrInput: string; addrQ: string }

export function noSearchBox(): SearchBox {
  return { addrInput: '', addrQ: '' };
}

/** The search: the URL's query and filters, with the sender box. */
export function mailSearch(box: SearchBox, q: string | undefined, sender: string | undefined, after: string | undefined,
                          before: string | undefined, unread: string | undefined): MailSearch {
  return {
    input: q ?? '', q: q ?? '',
    filter: { addrs: sender ? sender.split(',').filter(Boolean) : [], from: after ?? '', to: before ?? '', unread: !!unread },
    addrInput: box.addrInput, addrQ: box.addrQ,
  };
}

export interface SearchFilter { addrs: string[]; from: string; to: string; unread: boolean }

export function filterActive(f: SearchFilter): boolean {
  return f.addrs.length > 0 || f.from !== '' || f.to !== '' || f.unread;
}

/** Marks the messages between the anchor and `key` (in list order), or just `key` without an anchor in the list. */
export function markRange(list: Message[], marked: string[], anchor: string | null, key: string): string[] {
  const keys = list.map(messageKey);
  const from = anchor ? keys.indexOf(anchor) : -1;
  const to = keys.indexOf(key);
  if (from < 0 || to < 0) return marked.includes(key) ? marked : [...marked, key];
  const range = keys.slice(Math.min(from, to), Math.max(from, to) + 1);
  return [...marked, ...range.filter((k) => !marked.includes(k))];
}

/** The message `step` rows from `key` (or the first/last one without a key), or null at the end of the page. */
export function neighbor(list: Message[], key: string | null, step: number): Message | null {
  const index = key ? list.findIndex((msg) => messageKey(msg) === key) : -1;
  if (index < 0) return (step > 0 ? list[0] : list.at(-1)) ?? null;
  return list[index + step] ?? null;
}

// ── Page state ────────────────────────────────────────────────────────────────

/** The message in the reader, and whether its overlay is shown (below 1024px). */
export interface Reader { ref: MessageRef | null; shown: boolean }

/** The reader on `msg` (its overlay shown), or empty for null. */
export function readerOn(msg: MessageRef | null): Reader {
  return { ref: msg && refOf(msg), shown: !!msg };
}

export function noReader(): Reader {
  return { ref: null, shown: false };
}

/** Whether `ref` is a message of the view's mailbox. */
export function inMailbox(ref: MessageRef | null, view: MailView): boolean {
  return !!ref && ref.account_id === view.account_id && ref.mailbox === view.mailbox;
}

/**
 * The list's selection: `cursor` the key of the row with the focus ring; `keys` the rows marked for a bulk action
 * (`anchor` the last one marked with Ctrl or a plain click); the bulk menu's `error`, `move` (its folder list is
 * open) and `busy` (an action runs).
 */
export interface Selection { cursor: string | null; keys: string[]; anchor: string | null; error: string; move: boolean; busy: boolean }

export function noSelection(): Selection {
  return { cursor: null, keys: [], anchor: null, error: '', move: false, busy: false };
}

/** No rows marked any more (the list changed under them). */
export function unmarked(sel: Selection): Selection {
  return { ...sel, keys: [] };
}

/** The marks and the bulk menu cleared (Escape, "Clear selection"); with `cursor`, the focus ring moves too. */
export function cleared(sel: Selection, cursor: string | null = sel.cursor): Selection {
  return { ...sel, cursor, keys: [], anchor: null, error: '', move: false };
}

/** The cursor on `key` with no rows marked: the message there is open in the reader. */
export function openedAt(sel: Selection, key: string): Selection {
  return { ...sel, cursor: key, keys: [] };
}

type Modifiers = Pick<MouseEvent, 'ctrlKey' | 'metaKey' | 'shiftKey'>;

/**
 * The selection after a click on the row `key` (or Ctrl/Cmd+Space, Shift+Space on it): Ctrl (Cmd) marks or
 * unmarks it, Shift marks the rows from the anchor; a plain click unmarks all and puts the cursor there (the page
 * opens the message, see opensRow()). With no rows marked, the message `open` in the reader (when it is in the
 * list) counts as the first one marked: Ctrl adds the row to it, Shift marks the rows from it.
 */
export function rowClick(e: Modifiers, sel: Selection, key: string, list: Message[], open: MessageRef | null = null): Selection {
  if (!e.ctrlKey && !e.metaKey && !e.shiftKey) return { ...sel, keys: [], anchor: null, error: '', cursor: key };
  const opened = !sel.keys.length && open && list.some((msg) => isRef(msg, open)) ? messageKey(open) : null;
  const keys = opened ? [opened] : sel.keys;
  const anchor = opened ?? sel.anchor;
  if (e.shiftKey && !e.ctrlKey && !e.metaKey) return { ...sel, keys: markRange(list, keys, anchor, key), anchor, error: '', move: false };
  return { ...sel, keys: opened === key ? keys : toggled(keys, key), anchor: key, error: '', move: false };
}

/** Whether a click on a row opens its message (one without Ctrl, Cmd or Shift). */
export function opensRow(e: MouseEvent): boolean {
  return !e.ctrlKey && !e.metaKey && !e.shiftKey;
}

/** After a bulk action: the messages it failed for stay marked, with the error. */
export function afterBulk(sel: Selection, failed: string[]): Selection {
  return { ...sel, busy: false, keys: failed, anchor: failed[0] ?? null, error: failed.length ? m.mail_bulkActionError() : '', move: false };
}

/** The search box and filters: `input` while typing, `q` the search (debounced), `addrInput`/`addrQ` the sender box. */
export interface MailSearch { input: string; q: string; filter: SearchFilter; addrInput: string; addrQ: string }

export function noSearch(): MailSearch {
  return { input: '', q: '', filter: { addrs: [], from: '', to: '', unread: false }, addrInput: '', addrQ: '' };
}

/** Whether the list shows search results (a query or a filter) instead of the view's messages. */
export function searchActive(s: MailSearch): boolean {
  return s.q.trim() !== '' || filterActive(s.filter);
}

/** The sender typed into the filter box when it is a new one to filter by, else ''. */
export function addableSender(s: MailSearch): string {
  const addr = s.addrInput.trim();
  return addr && !s.filter.addrs.includes(addr) ? addr : '';
}

/** The search with the typed sender added to the filter (when new), and the sender box emptied. */
export function senderAdded(s: MailSearch): MailSearch {
  const addr = addableSender(s);
  return { ...s, filter: { ...s.filter, addrs: addr ? [...s.filter.addrs, addr] : s.filter.addrs }, addrInput: '', addrQ: '' };
}

/** The status line of an empty list. */
export function listStatus(searching: boolean, q: string, accounts: number, view: MailView): string {
  if (searching) return q.trim() ? m.mail_noResults({ q }) : m.mail_noResultsFiltered();
  if (!accounts) return m.mail_noAccounts();
  return !view.account_id && !view.folder_id ? m.mail_selectMailbox() : m.mail_noMessages();
}

/** The message dragged to a mailbox of the sidebar, and the mailbox under it (its sidebar key). */
export interface Drag { msg: Message | null; over: string | null }

export function noDrag(): Drag {
  return { msg: null, over: null };
}

/** Whether the dragged message can be dropped on `mb`: another mailbox of its account. */
export function canDrop(msg: Message | null, mb: Mailbox): boolean {
  return !!msg && msg.account_id === mb.account_id && msg.mailbox !== mb.name;
}

/** The flags of `msg` once read (for the lists, as the server marks it read when it opens). */
export function markedRead(msg: { flags: string[] }): Partial<Message> {
  return withFlag(msg, '\\Seen', true);
}

/** The archive mailbox to move `msg` to: the account's only subscribed archive folder. */
export function archiveMailbox(msg: MessageRef | null, mailboxes: Mailbox[]): string | null {
  if (!msg) return null;
  const candidates = mailboxes.filter((mb) =>
    mb.account_id === msg.account_id && mb.subscribed && mb.name !== msg.mailbox && folderKind(mb.name) === 'archive');
  return candidates.length === 1 ? candidates[0].name : null;
}

/** Subscribed mailboxes of the account other than `exclude`, for "Move to folder". */
export function moveTargets(mailboxes: Mailbox[], account_id: string | null, exclude: string[]): Mailbox[] {
  return account_id ? mailboxes.filter((mb) => mb.account_id === account_id && mb.subscribed && !exclude.includes(mb.name)) : [];
}

export interface Bulk { messages: Message[]; account_id: string | null; targets: Mailbox[]; canArchive: boolean; inTrash: boolean }

/** What the bulk menu offers for the marked messages of the list. */
export function bulkOf(list: Message[], marked: string[], mailboxes: Mailbox[]): Bulk {
  const messages = list.filter((msg) => marked.includes(messageKey(msg)));
  const account_id = messages.length > 0 && messages.every((msg) => msg.account_id === messages[0].account_id)
    ? messages[0].account_id : null;
  return {
    messages,
    account_id,
    targets: moveTargets(mailboxes, account_id, [...new Set(messages.map((msg) => msg.mailbox))]),
    canArchive: messages.length > 0 && messages.every((msg) => archiveMailbox(msg, mailboxes) !== null),
    inTrash: messages.some((msg) => folderKind(msg.mailbox) === 'trash'),
  };
}

export function withFlag(msg: { flags: string[] }, flag: string, set: boolean): Partial<Message> {
  const flags = set ? [...new Set([...msg.flags, flag])] : msg.flags.filter((f) => f !== flag);
  return { flags, is_read: flags.includes('\\Seen'), is_flagged: flags.includes('\\Flagged') };
}

export function flagBody(msg: MessageRef, flag: string, set: boolean) {
  return { account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid, flags: [flag], set };
}

export function moveBody(msg: MessageRef, dest_mailbox: string) {
  return { account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid, dest_mailbox };
}

export function inTrash(msg: MessageRef | null): boolean {
  return !!msg && folderKind(msg.mailbox) === 'trash';
}

export function emptyTrashDays(choice: string, custom: number | null): number {
  return choice === 'custom' ? Number(custom) : Number(choice);
}

export function validTrashDays(days: number): boolean {
  return Number.isInteger(days) && days > 0 && days <= 3650;
}

// ── Keyboard ──────────────────────────────────────────────────────────────────
// Shortcuts with a visible control are `data-shortcut` on it (src/islands/common.ts): r, a, f, e, #, Shift+I/U in
// the reader, Escape on the bulk menu's "Clear selection". The keys below have no control of their own.

/**
 * The target of a keydown that is for the page (no field, no open dialog, no modifier but Shift), else null.
 * Handlers ask again after their own preventDefault(), so a key they took still counts.
 */
function pageKeyTarget(e: KeyboardEvent): Element | null {
  const target = e.target instanceof Element ? e.target : null;
  if (!target || e.altKey || e.ctrlKey || e.metaKey) return null;
  if (target.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"], dialog[open]')) return null;
  return target.ownerDocument.querySelector('dialog[open]') ? null : target;
}

/**
 * Whether a key filter on the mail page may take this press: no rows marked, and not from a dialog (the dialogs
 * sit inside the page; key filters already leave fields alone).
 */
export function pageKey(e: KeyboardEvent, marked: number): boolean {
  return marked === 0 && !!pageKeyTarget(e);
}

/** The reader's part that PageUp/PageDown/Home/End scroll: `.message-detail`, else its `.detail-body`, whichever can. */
export function readerScroller(detail: Element): HTMLElement | null {
  return [detail, detail.querySelector('.detail-body')]
    .find((el): el is HTMLElement => el instanceof HTMLElement && el.scrollHeight > el.clientHeight) ?? null;
}

/** Where PageUp/PageDown/Home/End scroll `el` to (its new scrollTop): a page is 90% of its height. */
export function scrollTopFor(key: string, el: HTMLElement | null): number {
  if (!el) return 0;
  if (key === 'Home') return 0;
  if (key === 'End') return el.scrollHeight;
  return el.scrollTop + el.clientHeight * 0.9 * (key === 'PageDown' ? 1 : -1);
}

/** The list row after (1) or before (-1) `row`, or null at the end of the page. */
export function siblingRow(row: Element, step: number): HTMLElement | null {
  const el = step > 0 ? row.nextElementSibling : row.previousElementSibling;
  return el instanceof HTMLElement && el.matches('.message-row') ? el : null;
}

/** The resize handles' widths: [min, max]. */
export function clampWidth(which: 'sidebar' | 'list', width: number): number {
  return which === 'sidebar' ? Math.max(140, Math.min(400, width)) : Math.max(180, Math.min(600, width));
}

/** Arrow keys on a resize handle: the new width, or the same one for other keys. */
export function keyWidth(which: 'sidebar' | 'list', e: KeyboardEvent, width: number): number {
  if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return width;
  return clampWidth(which, width + (e.shiftKey ? 25 : 10) * (e.key === 'ArrowRight' ? 1 : -1));
}

// ── Reader ────────────────────────────────────────────────────────────────────

/** The decrypted/parsed body (mail.body()) when it is the one of `msg`, else null (still loading another one). */
export function bodyOf<B extends { key: string }>(body: B | null | undefined, msg: Message): B | null {
  return body && body.key === messageKey(msg) ? body : null;
}

export function isSentMailbox(mailbox: string | null): boolean {
  return !!mailbox && /sent/i.test(mailbox);
}

export function messageDateTime(ts: number | null): string {
  return ts ? new Date(ts * 1000).toLocaleString() : '';
}

/** Attachments the server lists that the reader shows (signatures and, when encrypted, PGP transport parts are hidden). */
export function shownAttachments(msg: Message | null, encrypted: boolean): (Attachment & { url?: string })[] {
  return msg?.attachments.filter((att) => !isPgpSignatureAttachment(att) && (!encrypted || !isPgpTransportAttachment(att))) ?? [];
}

export function attachmentUrl(msg: MessageRef, filename: string): string {
  return `/api/email/attachment?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}&filename=${encodeURIComponent(filename)}`;
}

export function attachmentCount(n: number): string {
  return n === 1 ? m.mail_attachment1() : m.mail_attachmentsN({ n });
}

export function pgpBadgeLabel(badge: string): string {
  switch (badge) {
    case 'encrypted': return m.pgp_badgeEncrypted();
    case 'signed': return m.pgp_badgeSigned();
    case 'verified': return m.pgp_badgeVerified();
    case 'unverified': return m.pgp_badgeUnverified();
    case 'missing_key': return m.pgp_badgeMissingKey();
    case 'unsupported_alg': return m.pgp_badgeUnsupportedAlg();
    case 'verification_failed': return m.pgp_badgeVerificationFailed();
    default: return badge;
  }
}

export function isIconBadge(badge: string): boolean {
  return badge === 'encrypted' || badge === 'signed' || badge === 'verified';
}

export interface ImageOrigins { origins: string[] }

/** The remote image origins of a body (ReaderBody.origins), split by the user's allow and ignore lists. */
export function bodyOrigins(all: string[], allowed: string[], ignored: string[]) {
  return {
    all,
    blocked: all.filter((o) => !allowed.includes(o) && !ignored.includes(o)),
    allowed: all.filter((o) => allowed.includes(o)),
    ignored: all.filter((o) => ignored.includes(o)),
  };
}

export function matching(origins: string[], q: string): string[] {
  return q ? origins.filter((o) => o.includes(q)) : origins;
}

/** Is `node` the open message (same Message-ID, else the same place)? */
export function isThreadCurrent(node: ThreadNode, msg: Message | null): boolean {
  if (!msg) return false;
  if (node.message_id && msg.message_id) return node.message_id === msg.message_id;
  return isRef(node, msg);
}

// ── Compose ───────────────────────────────────────────────────────────────────

export type ComposeMode = 'new' | 'reply' | 'replyAll' | 'forward';

/** The compose dialog's state; `files` are the attachments, `forward*` the HTML part of a forward. */
export interface ComposeDraft {
  mode: ComposeMode;
  identity_id: string;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  ccOpen: boolean;
  in_reply_to: string | null;
  references: string | null;
  forwardHtml: string | null;
  forwardText: string | null;
  forwardRef: MessageRef | null;
  /** the forwarded HTML as the dialog previews it (mail.forwardFiles) */
  forwardPreview: string;
  files: File[];
  encrypt: boolean;
  sign: boolean;
  signOptOut: boolean;
}

function quoteHeaderLines(msg: Message, includeSubject: boolean): string[] {
  const lines: string[] = [];
  const date = messageDateTime(msg.sent_at);
  if (date) lines.push(`Date: ${date}`);
  if (msg.from) lines.push(`From: ${msg.from}`);
  if (msg.to.length > 0) lines.push(`To: ${msg.to.join(', ')}`);
  if (msg.cc.length > 0) lines.push(`Cc: ${msg.cc.join(', ')}`);
  if (includeSubject) lines.push(`Subject: ${msg.subject || m.mail_noSubject()}`);
  return lines;
}

const cleanQuoted = (text: string) => text.replace(/\r\n?/g, '\n').trim();

function replyQuote(msg: Message, text: string): string {
  const quoted = [...quoteHeaderLines(msg, false), '', cleanQuoted(text)]
    .join('\n').split('\n').map((line) => (line ? `> ${line}` : '>')).join('\n');
  return `\n\n${quoted}`;
}

function forwardQuote(msg: Message, text: string): string {
  return ['', '', '--- Forwarded message ---', ...quoteHeaderLines(msg, true), '', cleanQuoted(text)]
    .join('\n').replace(/\n+$/, '');
}

function forwardHtml(msg: Message, html: string | null): string | null {
  if (!html) return null;
  const bodyMatch = html.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
  const content = bodyMatch ? bodyMatch[1] : html
    .replace(/^[\s\S]*?<html[^>]*>/i, '').replace(/<\/html>\s*$/i, '').replace(/^[\s\S]*?<\/head>/i, '').trim();
  const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const lines: string[] = [];
  if (msg.from) lines.push(`From: ${esc(msg.from)}`);
  const date = messageDateTime(msg.sent_at);
  if (date) lines.push(`Date: ${esc(date)}`);
  if (msg.to.length > 0) lines.push(`To: ${esc(msg.to.join(', '))}`);
  if (msg.cc.length > 0) lines.push(`Cc: ${esc(msg.cc.join(', '))}`);
  lines.push(`Subject: ${esc(msg.subject || '')}`);
  return `<br><div style="border-left:2px solid #ccc;padding-left:8px;margin:8px 0"><div>---------- Forwarded message ---------</div>${lines.map((l) => `<div>${l}</div>`).join('')}<br>${content}</div>`;
}

const replyRecipients = (msg: Message) => (msg.reply_to?.length ? msg.reply_to : msg.from ? [msg.from] : []);

function replyIdentity(msg: Message, identities: Identity[]): string {
  const own = identities.filter((i) => i.account_id === msg.account_id);
  const to = (msg.to ?? []).map((a) => a.toLowerCase());
  const matched = own.find((i) => to.some((a) => a.includes(i.email.toLowerCase())));
  return (matched ?? own.find((i) => i.is_default) ?? identities[0])?.identity_id ?? '';
}

/**
 * A new compose draft (MailComposeDialog.open): `msg` is the message replied to or forwarded, `text`/`html` its
 * shown body; `files` the attachments to start with.
 */
/** The draft another page shared ("send by email": its subject and attachments), from the default identity. */
export function sharedDraft(shared: { subject: string; files: File[] }, identities: Identity[]): ComposeDraft {
  return composeDraft('new', null, identities, '', null, shared.files, shared.subject);
}

export function composeDraft(mode: ComposeMode, msg: Message | null, identities: Identity[], text = '', html: string | null = null, files: File[] = [], subject = '', preview = ''): ComposeDraft {
  const draft: ComposeDraft = {
    mode: 'new', identity_id: (identities.find((i) => i.is_default) ?? identities[0])?.identity_id ?? '',
    to: '', cc: '', bcc: '', subject, body: '', ccOpen: false, in_reply_to: null, references: null,
    forwardHtml: null, forwardText: null, forwardRef: null, forwardPreview: preview, files, encrypt: false, sign: false, signOptOut: false,
  };
  if (!msg || mode === 'new') return draft;
  draft.mode = mode;
  draft.identity_id = replyIdentity(msg, identities);
  if (mode === 'forward') {
    draft.subject = /^fwd:/i.test(msg.subject ?? '') ? (msg.subject ?? '') : `Fwd: ${msg.subject || ''}`;
    draft.forwardHtml = forwardHtml(msg, html);
    if (draft.forwardHtml) draft.forwardText = forwardQuote(msg, text);
    else draft.body = forwardQuote(msg, text);
    draft.forwardRef = refOf(msg);
    return draft;
  }
  draft.to = replyRecipients(msg).join(', ');
  draft.subject = msg.subject?.startsWith('Re:') ? msg.subject : `Re: ${msg.subject || ''}`;
  draft.body = replyQuote(msg, text);
  draft.in_reply_to = msg.message_id;
  draft.references = msg.message_id;
  if (mode === 'replyAll') {
    const mine = identities.find((i) => i.identity_id === draft.identity_id)?.email.toLowerCase() ?? '';
    const excluded = new Set([msg.from ?? '', ...replyRecipients(msg)].map((a) => a.toLowerCase()));
    draft.cc = [...(msg.to ?? []), ...(msg.cc ?? [])]
      .filter((addr) => !addr.toLowerCase().includes(mine) && !excluded.has(addr.toLowerCase()))
      .join(', ');
    draft.ccOpen = draft.cc.length > 0;
  }
  return draft;
}

export function composeTitle(mode: ComposeMode): string {
  return mode === 'forward' ? m.mail_forward() : mode === 'replyAll' ? m.mail_replyAll() : mode === 'reply' ? m.mail_reply() : m.mail_newMessage();
}

/** The address being typed: what follows the last comma. */
export function recipientToken(value: string): string {
  return value.split(',').at(-1)!.trim();
}

/** `value` with the address being typed replaced by `addr`. */
export function withRecipient(value: string, addr: string): string {
  const comma = value.lastIndexOf(',');
  return `${comma >= 0 ? value.slice(0, comma + 1) + ' ' : ''}${addr}, `;
}

export interface AddressSuggestion { name: string; email: string }

/** Every contact e-mail address once, with the contact's name (GET /api/contacts). */
export function contactAddresses(contacts: ContactAddresses[]): AddressSuggestion[] {
  const seen = new Set<string>();
  const out: AddressSuggestion[] = [];
  for (const c of contacts) for (const e of c.emails ?? []) {
    if (e.value && !seen.has(e.value.toLowerCase())) {
      seen.add(e.value.toLowerCase());
      out.push({ name: c.fn_name || '', email: e.value });
    }
  }
  return out;
}

/** Up to 7 recipient suggestions for `token`: matching contacts first, then addresses seen in mail. */
export function recipientHits(token: string, contacts: AddressSuggestion[], seen: string[]): string[] {
  if (token.length < 2) return [];
  const q = token.toLowerCase();
  const hits = contacts.filter((c) => c.email.toLowerCase().includes(q) || c.name.toLowerCase().includes(q));
  const emails = new Set(hits.map((c) => c.email.toLowerCase()));
  return [
    ...hits.slice(0, 5).map((c) => (c.name ? `${c.name} <${c.email}>` : c.email)),
    ...seen.filter((a) => !emails.has(a.toLowerCase())),
  ].slice(0, 7);
}

/** The sender filter's suggestion rows: contacts matching the input, then addresses from mail not yet listed. */
export function senderHits(input: string, contacts: AddressSuggestion[], seen: string[], chosen: string[]) {
  const q = input.toLowerCase();
  return {
    contacts: contacts.filter((c) => !q || c.email.toLowerCase().includes(q) || c.name.toLowerCase().includes(q)).slice(0, 12),
    seen: input.trim() ? seen.filter((a) => !contacts.some((c) => c.email.toLowerCase() === a) && !chosen.includes(a)) : [],
    canAdd: !!input && !contacts.some((c) => c.email.toLowerCase() === q) && !seen.includes(q),
  };
}

export function senderLabel(addr: string, contacts: AddressSuggestion[]): string {
  return contacts.find((c) => c.email === addr)?.name || addr;
}

/** Public keys the compose dialog may encrypt to (compose.keys action): emails per trusted key. */
export interface ComposeKey { fingerprint: string; emails: string[]; is_own: boolean }
export interface ComposeKeys { keys: ComposeKey[]; vault: string[]; unlocked: string[] }

const hasEmail = (k: ComposeKey, email: string) => k.emails.includes(email.trim().toLowerCase());

/** Can the draft be signed: an unlocked key for the identity, or an own key stored in the vault. */
export function canSign(draft: ComposeDraft, identities: Identity[], keys: ComposeKeys | null): boolean {
  const identity = identities.find((i) => i.identity_id === draft.identity_id);
  if (!identity || !keys) return false;
  return keys.unlocked.includes(identity.email.toLowerCase()) ||
    keys.keys.some((k) => k.is_own && keys.vault.includes(k.fingerprint.toUpperCase()));
}

/** Can the draft be encrypted: a trusted key for every recipient and an own key for the identity. */
export function canEncrypt(draft: ComposeDraft, identities: Identity[], keys: ComposeKeys | null): boolean {
  const identity = identities.find((i) => i.identity_id === draft.identity_id);
  if (!identity || !keys) return false;
  const recipients = [...parseEnvelopeAddresses(draft.to), ...parseEnvelopeAddresses(draft.cc), ...parseEnvelopeAddresses(draft.bcc)];
  return recipients.length > 0 &&
    recipients.every((r) => keys.keys.some((k) => hasEmail(k, r))) &&
    keys.keys.some((k) => k.is_own && hasEmail(k, identity.email));
}

/** Signing is on by default when possible, unless the user turned it off. */
export function signs(draft: ComposeDraft, identities: Identity[], keys: ComposeKeys | null): boolean {
  return canSign(draft, identities, keys) && (draft.sign || !draft.signOptOut);
}

// ── Accounts, identities, virtual folders ────────────────────────────────────

export interface AccountForm {
  account_id: string | null; display_name: string; sender_name: string; host: string; port: number | null;
  username: string; password: string; use_tls: boolean; smtp_host: string; smtp_port: number | null;
}

/** GET /api/email/autodiscover: `{ found: false }` alone, or `found: true` with the servers. */
export interface Autodiscovery {
  found: boolean; provider_name?: string | null; imap_host?: string; imap_port?: number; imap_use_tls?: boolean;
  imap_username?: string; smtp_host?: string; smtp_port?: number;
}

export function newAccount(): AccountForm {
  return { account_id: null, display_name: '', sender_name: '', host: '', port: 993, username: '', password: '', use_tls: true, smtp_host: '', smtp_port: 587 };
}

export function editAccount(acc: Account, identities: Identity[]): AccountForm {
  return {
    account_id: acc.account_id, display_name: acc.display_name,
    sender_name: identities.find((i) => i.account_id === acc.account_id && i.is_default)?.name ?? '',
    host: acc.host, port: acc.port, username: acc.username, password: '', use_tls: acc.use_tls,
    smtp_host: acc.smtp_host || '', smtp_port: acc.smtp_port || 587,
  };
}

/** The account dialog around its form: the step, what autodiscovery found, the advanced settings, error, busy. */
export interface AccountUi {
  step: 'basic' | 'details'; discovery: Autodiscovery | null; advanced: boolean; error: string; busy: boolean;
}

/**
 * The account dialog at a step: 'basic' (name and address, before autodiscovery) or 'details' (the servers),
 * with the advanced settings open unless autodiscovery found them.
 */
export function accountUi(step: 'basic' | 'details', discovery: Autodiscovery | null = null): AccountUi {
  return { step, discovery, advanced: step === 'details' && !discovery?.found, error: '', busy: false };
}

/** The form after autodiscovery (`found` fills the servers); the display name defaults to the provider or domain. */
export function discovered(form: AccountForm, d: Autodiscovery | null): AccountForm {
  const next = { ...form };
  if (d?.found) {
    next.host = d.imap_host ?? next.host;
    next.port = d.imap_port ?? next.port;
    next.use_tls = d.imap_use_tls ?? next.use_tls;
    next.username = d.imap_username ?? next.username;
    next.smtp_host = d.smtp_host ?? next.smtp_host;
    next.smtp_port = d.smtp_port ?? next.smtp_port;
  }
  if (!next.display_name) next.display_name = (d?.found && d.provider_name) || form.username.split('@')[1] || '';
  return next;
}

export function accountBody(f: AccountForm) {
  return {
    account_id: f.account_id, display_name: f.display_name, sender_name: f.sender_name || null, host: f.host, port: f.port,
    username: f.username, password: f.password, use_tls: f.use_tls, smtp_host: f.smtp_host || null, smtp_port: f.smtp_port || null,
  };
}

export interface IdentityForm { identity_id: string | null; account_id: string; name: string; email: string }

export function identityBody(f: IdentityForm | Identity, is_default: boolean) {
  return { identity_id: f.identity_id, account_id: f.account_id, name: f.name, email: f.email, is_default };
}

export interface FolderDraft { folder_id: string | null; name: string; boxes: string[]; only_flagged: boolean; only_attachments: boolean }

export function folderDraft(folder: VirtualFolder | null): FolderDraft {
  if (!folder) return { folder_id: null, name: '', boxes: [], only_flagged: false, only_attachments: false };
  const all = folder.rules.length === 1 && folder.rules[0].account_id === null && folder.rules[0].mailbox_pattern === null;
  return {
    folder_id: folder.folder_id, name: folder.display_name, only_flagged: folder.only_flagged ?? false,
    only_attachments: folder.only_attachments ?? false,
    boxes: all ? [] : folder.rules.map((r) => `${r.account_id ?? ''}::${r.mailbox_pattern ?? ''}`),
  };
}

export function folderBody(d: FolderDraft) {
  const rules = d.boxes.length === 0
    ? [{ account_id: null, mailbox_pattern: null }]
    : d.boxes.map((key) => {
        const sep = key.indexOf('::');
        return { account_id: key.slice(0, sep) || null, mailbox_pattern: key.slice(sep + 2) || null };
      });
  return { display_name: d.name || m.mail_folderNamePlaceholder(), rules, only_flagged: d.only_flagged, only_attachments: d.only_attachments };
}

export function subscribed(mailboxes: Mailbox[], account_id: string): Mailbox[] {
  return mailboxes.filter((mb) => mb.account_id === account_id && mb.subscribed);
}

/** Each account's archive mailbox for the marked messages (bulk archive). */
export function archiveTargets(messages: Message[], mailboxes: Mailbox[]): Record<string, string | null> {
  return Object.fromEntries(messages.map((msg) => [msg.account_id, archiveMailbox(msg, mailboxes)]));
}


/** What the mail page reads of GET /api/contacts: names and e-mail addresses. */
export interface ContactAddresses { fn_name?: string; emails?: { value: string }[] }

/** The text body as plain and link pieces (`href` null for plain text). */
export function textSegments(text: string): { text: string; href: string | null }[] {
  return linkifySegments(text).map((seg) => ({ text: seg.text, href: 'href' in seg ? seg.href : null }));
}

/** A message's identity in lists and marks: account, mailbox and UID. */
export function messageKey(msg: MessageRef): string {
  return `${msg.account_id} ${msg.mailbox} ${msg.uid}`;
}

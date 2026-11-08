// Mail page helpers (mail/index.html, mail/view/index.html, partials/mail-*.html): pure functions the templates
// call by name through src/sinter.helpers.ts. Types of the mail API and the page's state live here too.
import * as m from '../paraglide/messages/_index.js';
import {
  canPreviewAttachment,
  folderKind,
  folderSortKey,
  linkifySegments,
  isPgpSignatureAttachment,
  isPgpTransportAttachment,
} from '../components/mail/mailUtils.ts';
import type { Account, Attachment, Identity, Mailbox, Message, ThreadNode, VirtualFolder } from '../components/mail/MailTypes.ts';
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
} from '../components/mail/mailUtils.ts';

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

const noView: MailView = { account_id: null, mailbox: null, folder_id: null };

export function mailboxView(account_id: string, mailbox: string): MailView {
  return { account_id, mailbox, folder_id: null };
}

export function folderView(folder_id: string): MailView {
  return { account_id: null, mailbox: null, folder_id };
}

/**
 * The view to show: the remembered one while it still exists, else the first account's inbox
 * (or its first mailbox), as MailApp did on load.
 */
export function currentView(view: MailView, accounts: Account[], mailboxes: Mailbox[], folders: VirtualFolder[]): MailView {
  if (view.folder_id && folders.some((f) => f.folder_id === view.folder_id)) return view;
  if (view.account_id && view.mailbox &&
      mailboxes.some((mb) => mb.account_id === view.account_id && mb.name === view.mailbox)) return view;
  const first = accounts[0];
  if (!first) return noView;
  const own = mailboxes.filter((mb) => mb.account_id === first.account_id);
  const inbox = own.find((mb) => mb.name.toUpperCase() === 'INBOX') ?? own[0];
  return inbox ? mailboxView(first.account_id, inbox.name) : noView;
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

/** The ?account_id=&mailbox=&uid= of a link (notification, popout), or null. */
export function urlRef(url: Record<string, string | undefined>): MessageRef | null {
  const uid = Number(url.uid);
  return url.account_id && url.mailbox && Number.isInteger(uid) && uid > 0
    ? { account_id: url.account_id, mailbox: url.mailbox, uid }
    : null;
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

/**
 * The page shortcut a keydown on the mail page asks for, or '' (MailApp's handleMailKeydown):
 * 'clear' (Escape with marked rows), 'next'/'prev' (j/k), 'back' (Escape in the reader), 'scroll' (Page/Home/End
 * in the reader), 'reply', 'replyAll', 'forward', 'archive', 'delete', 'read', 'unread'.
 */
export function mailShortcut(e: KeyboardEvent, marked: number, standalone: boolean): string {
  const target = e.target instanceof Element ? e.target : null;
  if (e.defaultPrevented || !target || target.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"], dialog[open]')) return '';
  if (target.ownerDocument.querySelector('dialog[open]')) return '';
  const plain = !e.altKey && !e.ctrlKey && !e.metaKey;
  if (!standalone && marked > 0 && plain && !e.shiftKey && e.key === 'Escape') return 'clear';
  if (marked > 0) return '';
  if (!standalone && plain && !e.shiftKey && (e.key === 'j' || e.key === 'k')) return e.key === 'j' ? 'next' : 'prev';
  const inReader = !!target.closest('.message-detail');
  if (inReader && plain) {
    if (!standalone && !e.shiftKey && e.key === 'Escape') return 'back';
    if (!e.shiftKey && ['PageUp', 'PageDown', 'Home', 'End'].includes(e.key)) return 'scroll';
  }
  if (!plain) return '';
  if (!e.shiftKey && e.key === 'r') return 'reply';
  if (!e.shiftKey && e.key === 'a') return 'replyAll';
  if (!e.shiftKey && e.key === 'f') return 'forward';
  if (!e.shiftKey && e.key === 'e') return 'archive';
  if (e.key === '#') return 'delete';
  if (e.shiftKey && e.key.toLowerCase() === 'i') return 'read';
  if (e.shiftKey && e.key.toLowerCase() === 'u') return 'unread';
  return '';
}

/** A list row's own keys (on a focused .message-row, else ''): 'down'/'up' (arrows), 'open' (Enter), 'toggle' (Ctrl/Cmd+Space), 'range' (Shift+Space). */
export function rowKey(e: KeyboardEvent): string {
  if (!(e.target instanceof Element) || !e.target.matches('.message-row')) return '';
  const space = e.code === 'Space' || e.key === ' ';
  if (e.altKey) return '';
  if (space && (e.ctrlKey || e.metaKey) && !e.shiftKey) return 'toggle';
  if (space && e.shiftKey && !e.ctrlKey && !e.metaKey) return 'range';
  if (e.ctrlKey || e.metaKey || e.shiftKey) return '';
  if (e.key === 'ArrowDown') return 'down';
  if (e.key === 'ArrowUp') return 'up';
  if (e.key === 'Enter') return 'open';
  return '';
}

/** How far PageUp/PageDown/Home/End scroll a reader of `height` px: [top, by] (one of them is null). */
export function readerScroll(key: string, height: number, scrollHeight: number): { top?: number; by?: number } {
  if (key === 'Home') return { top: 0 };
  if (key === 'End') return { top: scrollHeight };
  return { by: height * 0.9 * (key === 'PageDown' ? 1 : -1) };
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

export interface Autodiscovery {
  found: boolean; provider_name?: string; imap_host: string; imap_port: number; imap_use_tls: boolean;
  imap_username: string; smtp_host: string; smtp_port: number;
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

/** The form after autodiscovery (`found` fills the servers); the display name defaults to the provider or domain. */
export function discovered(form: AccountForm, d: Autodiscovery | null): AccountForm {
  const next = { ...form };
  if (d?.found) {
    Object.assign(next, { host: d.imap_host, port: d.imap_port, use_tls: d.imap_use_tls, username: d.imap_username, smtp_host: d.smtp_host, smtp_port: d.smtp_port });
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

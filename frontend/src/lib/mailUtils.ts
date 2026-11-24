import type { Attachment, BodySegment, FolderKind, Message, ThreadNode, ThreadTreeNode } from './MailTypes.ts';

// ── Attachment type checks ────────────────────────────────────────────────────

export function canPreviewContentType(value: string | null | undefined): boolean {
  const contentType = value?.split(';', 1)[0].trim().toLowerCase() ?? '';
  return [
    'application/pdf',
    'text/plain',
    'image/png',
    'image/jpeg',
    'image/gif',
    'image/webp',
    'image/avif',
  ].includes(contentType) ||
    (contentType.startsWith('audio/') && contentType.length > 'audio/'.length) ||
    (contentType.startsWith('video/') && contentType.length > 'video/'.length);
}

export function canPreviewAttachment(attachment: Attachment): boolean {
  return canPreviewContentType(attachment.content_type);
}

export function isPgpTransportAttachment(attachment: Attachment): boolean {
  const filename = attachment.filename.toLowerCase();
  const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
  return filename === 'encrypted.asc' ||
    (/^(encrypted|message|msg)\.(asc|pgp|gpg)$/.test(filename) &&
      (contentType === 'application/octet-stream' || contentType === 'application/pgp-encrypted'));
}

export function isPgpSignatureAttachment(attachment: Attachment): boolean {
  const filename = attachment.filename.toLowerCase();
  const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
  return contentType === 'application/pgp-signature' ||
    /^signature\.(asc|sig|pgp|gpg)$/.test(filename);
}

export function isPgpPublicKeyAttachment(attachment: Attachment): boolean {
  const filename = attachment.filename.toLowerCase();
  const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
  return contentType === 'application/pgp-keys' ||
    (!isPgpTransportAttachment(attachment) && !isPgpSignatureAttachment(attachment) &&
      /\.(asc|pgp|gpg|pub)$/.test(filename));
}

export function isIcsAttachment(attachment: Attachment): boolean {
  const filename = attachment.filename.toLowerCase();
  const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
  return contentType === 'text/calendar' ||
    contentType === 'application/ics' ||
    contentType === 'application/icalendar' ||
    /\.(ics|ical)$/.test(filename);
}

// ── Email address helpers ─────────────────────────────────────────────────────

export function bareMailbox(value: string | null): string {
  const match = /<([^<>]+)>/.exec(value ?? '');
  return (match?.[1] ?? value ?? '').trim().toLowerCase();
}

export function keyHasEmail(key: { users: { userID?: { email?: string } }[] }, email: string): boolean {
  return key.users.some(user => user.userID?.email?.trim().toLowerCase() === email.toLowerCase());
}

// ── Message key helpers ───────────────────────────────────────────────────────

export function messageKey(msg: Message): string {
  return `${msg.account_id}\0${msg.mailbox}\0${msg.uid}`;
}

export function identityKey(accountId: string, mailbox: string, uid: number): string {
  return `${accountId}\0${mailbox}\0${uid}`;
}

export function sameMessage(a: Message | null, b: Message | null): boolean {
  return !!a && !!b && messageKey(a) === messageKey(b);
}

// ── Thread tree ───────────────────────────────────────────────────────────────

export function buildThreadTree(nodes: ThreadNode[]): ThreadTreeNode[] {
  const ids = new Set(nodes.map(node => node.message_id).filter((id): id is string => !!id));
  const childrenByParent = new Map<string, ThreadNode[]>();
  for (const node of nodes) {
    if (!node.parent_message_id) continue;
    const children = childrenByParent.get(node.parent_message_id) ?? [];
    children.push(node);
    childrenByParent.set(node.parent_message_id, children);
  }

  const included = new Set<ThreadNode>();
  const branch = (node: ThreadNode, ancestry: Set<ThreadNode>): ThreadTreeNode => {
    included.add(node);
    const nextAncestry = new Set(ancestry).add(node);
    const children = node.message_id
      ? (childrenByParent.get(node.message_id) ?? [])
          .filter(child => !nextAncestry.has(child))
          .map(child => branch(child, nextAncestry))
      : [];
    return { message: node, children };
  };
  const roots = nodes.filter(node => !node.parent_message_id || !ids.has(node.parent_message_id));
  const tree = roots.map(node => branch(node, new Set()));
  for (const node of nodes) {
    if (!included.has(node)) tree.push(branch(node, new Set()));
  }
  return tree;
}

// ── HTML helpers ──────────────────────────────────────────────────────────────

// Tags that convey something plain text cannot (links, images, tables, lists,
// headings, formatting). Structural wrappers (html/head/body/div/p/span/br/font)
// are excluded so "basically plain text" HTML is treated as plain text.
const MEANINGFUL_HTML =
  /<(a\s[^>]*href|img|table|thead|tbody|tr|td|th|ul|ol|li|h[1-6]|blockquote|hr|b|strong|i|em|u|s|strike|code|pre|mark)\b/i;

export function htmlAddsValue(html: string | null): boolean {
  return !!html && MEANINGFUL_HTML.test(html);
}

export function linkifySegments(text: string): BodySegment[] {
  const re = /(https?:\/\/[^\s<>()]+[^\s<>().,;:!?'"])/g;
  const out: BodySegment[] = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push({ text: text.slice(last, m.index) });
    out.push({ href: m[0], text: m[0] });
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

// ── Folder utilities ──────────────────────────────────────────────────────────

export function folderKind(name: string): FolderKind {
  const leaf = (name.split(/[\/.]/).pop() || name).toLowerCase().trim();
  if (leaf === 'inbox') return 'inbox';
  if (leaf.includes('sent')) return 'sent';
  if (leaf.includes('draft')) return 'drafts';
  if (leaf.includes('trash') || leaf.includes('deleted') || leaf.includes('bin')) return 'trash';
  if (leaf.includes('junk') || leaf.includes('spam')) return 'junk';
  if (leaf.includes('archive')) return 'archive';
  return 'folder';
}

export const FOLDER_PRIORITY: Record<FolderKind, number> = {
  inbox: 0, drafts: 1, sent: 2, junk: 3, archive: 4, trash: 5, folder: 6,
};

export function folderSortKey(name: string): [number, string] {
  return [FOLDER_PRIORITY[folderKind(name)], name.toLowerCase()];
}

// ── Formatting helpers ────────────────────────────────────────────────────────

export function formatDate(ts: number | null): string {
  if (!ts) return '';
  const d = new Date(ts * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }
  const opts: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
  if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
  return d.toLocaleDateString([], opts);
}

export function formatMessageDateTime(ts: number | null): string {
  return ts ? new Date(ts * 1000).toLocaleString() : '';
}

export function humanSize(bytes: number): string {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}

/** Display name of a From header (`"Doe, Jane" <jane@x>` → `Doe, Jane`), or the header itself without one. */
export function senderName(from: string | null): string {
  if (!from) return '';
  const match = from.match(/^(.+?)\s*<([^>]+)>/);
  const name = match ? match[1].trim() : from.trim();
  // RFC 5322 quoted-string: drop the surrounding quotes and undo backslash escapes.
  const quoted = name.match(/^"(.*)"$/s);
  if (!quoted) return name;
  return quoted[1].replace(/\\(.)/g, '$1').trim() || match?.[2] || name;
}

// ── DOM utilities ─────────────────────────────────────────────────────────────

export function intrinsicWidth(element: HTMLElement): number {
  const clone = element.cloneNode(true) as HTMLElement;
  clone.style.position = 'fixed';
  clone.style.left = '-10000px';
  clone.style.top = '0';
  clone.style.width = 'max-content';
  clone.style.maxWidth = 'none';
  clone.style.flex = 'none';
  clone.style.whiteSpace = 'nowrap';
  clone.style.visibility = 'hidden';
  clone.style.pointerEvents = 'none';
  document.body.appendChild(clone);
  const width = clone.getBoundingClientRect().width;
  clone.remove();
  return width;
}

export function computePopoverStyle(anchorEl: HTMLElement, popEl: HTMLElement): string {
  const rect = anchorEl.getBoundingClientRect();
  const width = popEl.offsetWidth;
  const top = rect.bottom + 4;
  const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
  return `position: fixed; top: ${top}px; left: ${left}px; right: auto;`;
}

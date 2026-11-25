// `mail` actions (mail/index.html, mail/view/index.html, src/components/mail-*.html): the mail page's imperative work —
// MIME parsing, OpenPGP, the vault, building and sending raw MIME, opening a window. Plain requests (flags, moves,
// deletions, bulk changes, folders, image origins, storing a key) are in the markup.
// Each returns data; the markup puts it where it belongs (data-sn-then, <sn-resource call>).
import type { CallContext } from '@tiaforge-ui/sinter';
import { apiFetch } from '../lib/api.ts';
import {
  attachmentUrl, canEncrypt, messageKey, missingKeys, type Autodiscovery, type ComposeDraft, type ComposeKeys, type Identity, type Message, type MessageRef,
} from '../lib/mail.ts';
import { cidReferencesFromHtml, htmlToPlainText, withInlineImages } from '../lib/mail-html.ts';
import { htmlAddsValue, isPgpPublicKeyAttachment, bareMailbox, keyHasEmail } from '../lib/mailUtils.ts';
import { messageBadges, protectedReply, protectedSubject, quotable, readerParts, type ReaderPart } from '../lib/mail-reader.ts';
import * as mail from '@tiavision/sinter-mail';
import type { ImageType, Message as MimeMessage, MimeOptions, Ref } from '@tiavision/sinter-mail';
// openpgp is a lazy package (sinter({ lazy }) in vite.config.ts): these modules import it as usual, sinter loads it
// on the first call, and a failed load is a SinterError (stale when a deploy removed it).
import { getKeys, isUnlocked, onLock, openVault } from '../lib/pgp/session.ts';
import { autocryptKeys } from '../lib/pgp/autocrypt.ts';
import {
  keyFacts, listPublicKeys, parsePublicKey, publicKeyBody, savePublicKey, trustedKeys, type PublicKeyBody, type StoredPublicKey,
} from '../lib/pgp/public-keys.ts';
import { keyChanges, type PendingKey } from '../lib/pgp-keys.ts';
import { knownKeys, settle, type Results } from '../lib/pgp/steps.ts';
import { decryptPrivateKeys, eraseBytes, localActiveDevice, readVault, rememberLocalDevice, unwrapVaultKey, validateVault } from '../lib/pgp/vault.ts';
import { unlock as webauthnUnlock } from '../lib/pgp/webauthn.ts';
import { buildProtectedMime, parseEnvelopeAddresses } from '../lib/pgp/compose.ts';
import type * as openpgp from 'openpgp';
import { SinterError } from '@tiaforge-ui/sinter';
import { m } from '@tiaforge-ui/sinter/messages';
import { pagePath } from '../lib/locale-path.ts';

// Locking the vault drops what its keys decrypted: the plaintexts mail.mime keeps in its cache.
onLock(() => void mail.forget());

// ── Accounts ──────────────────────────────────────────────────────────────────

/** The IMAP and SMTP servers of an address, for the account form's details step (src/components/email-account.html). */
export function autodiscover(email: string, ctx: CallContext): Promise<Autodiscovery> {
  return ctx.request(`/api/email/autodiscover?email=${encodeURIComponent(email)}`) as Promise<Autodiscovery>;
}

// ── Reading ───────────────────────────────────────────────────────────────────

// The URLs this module requests itself (the bytes of a message, an attachment or an inline image): attachmentUrl() and
// these.
const messageUrl = (path: 'message' | 'message/raw' | 'proxy-image', ref: MessageRef) =>
  `/api/email/${path}?account_id=${encodeURIComponent(ref.account_id)}&mailbox=${encodeURIComponent(ref.mailbox)}&uid=${ref.uid}`;

// The last message fetched, shared by open() and body(), which load together when a message is opened.
let lastFetch: { key: string; message: Promise<Message> } | null = null;

function fetchMessage(ref: MessageRef, ctx: CallContext, fresh: boolean): Promise<Message> {
  const key = messageKey(ref);
  if (fresh || lastFetch?.key !== key) {
    lastFetch = { key, message: ctx.request(messageUrl('message', ref)) as Promise<Message> };
    lastFetch.message.catch(() => { if (lastFetch?.key === key) lastFetch = null; });
  }
  return lastFetch.message;
}

/** The full message, fetched again (body() shares the fetch). The reader marks it read (src/components/email-reader-data.html). */
export function open(ref: MessageRef, ctx: CallContext): Promise<Message> {
  return fetchMessage(ref, ctx, true);
}

export interface DecryptedFile { filename: string; content_type: string; size: number; content_id: null; url: string }

/** What the reader shows for a message body (plain, signed or encrypted). */
export interface ReaderBody {
  key: string;
  /** the message as it was when this body was read: the reader keeps showing it until the next one's body is in */
  message: Message;
  /** the blocks to show in reading order, with a multipart/alternative's HTML (mode 'html') or its text ('text') */
  parts: ReaderPart[];
  textParts: ReaderPart[];
  /** the first HTML body and the text, for forwards and replies (a reply quotes `text` only when `quotable`) */
  html: string;
  text: string;
  /** nothing was decrypted, or a good signature binds the sender shown: a reply may quote it */
  quotable: boolean;
  mode: 'html' | 'text';
  encrypted: boolean;
  /** anything in it was decrypted (sinter-mail's summary.encrypted is not 'none'): a reply or forward starts encrypted */
  decrypted: boolean;
  signed: boolean;
  /** encrypted and no key is unlocked yet: the reader offers to unlock the vault */
  locked: boolean;
  badges: string[];
  /** the protected subject of a decrypted message */
  subject: string;
  /** whom a reply goes to when protected headers say so (else null: the message's From and Reply-To) */
  reply: string[] | null;
  error: string;
  /** decrypted attachments, as blob: URLs */
  files: DecryptedFile[];
  /** the public keys not stored yet, by the name of the attachment that holds them: the bodies of POST /pgp/key */
  importable: Record<string, PublicKeyBody>;
  /** inline images of a message read here (part id → content id → blob: URL); null: the server serves them */
  cids: Record<string, Record<string, string>> | null;
  /** a decrypted message's inline images as data: URLs by content id, written into a forward; null for other messages */
  forwardCids: Record<string, string> | null;
  /** read from the cut message, a signature unchecked: its signed bytes include a cut attachment, so checking it needs
   * the whole message (`wholeSize` bytes), which the reader fetches when asked (mail.verify) */
  pending: boolean;
  wholeSize: number;
}

// Bytes of the open message's decrypted attachments (for key import and calendar import), by file name.
let decryptedData = new Map<string, Uint8Array>();
let blobUrls: string[] = [];

function forget() {
  for (const url of blobUrls) URL.revokeObjectURL(url);
  blobUrls = [];
  decryptedData = new Map();
}

function blobUrl(bytes: Uint8Array, type: string): string {
  const url = URL.createObjectURL(new Blob([bytes as BlobPart], { type }));
  blobUrls.push(url);
  return url;
}

async function bytes(url: string): Promise<Uint8Array> {
  const res = await apiFetch(url);
  if (!res.ok) throw new Error(`${res.status}`);
  return new Uint8Array(await res.arrayBuffer());
}

/** Stores the sender's Autocrypt keys; in the background (body() does not wait), errors ignored. */
async function harvestAutocrypt(headers: readonly string[], msg: Message) {
  if (headers.length === 0) return;
  const raw = new TextEncoder().encode(`${headers.map((v) => `Autocrypt: ${v}`).join('\r\n')}\r\n\r\n`);
  for (const key of await autocryptKeys(raw, msg.from)) {
    await savePublicKey(key, 'harvested_autocrypt', bareMailbox(msg.from)).catch(() => {});
  }
}

async function importableKeys(msg: Message, files: { filename: string; content_type: string }[]): Promise<Record<string, PublicKeyBody>> {
  const candidates = files.filter((f) => isPgpPublicKeyAttachment({ ...f, size: 0, content_id: null }));
  if (candidates.length === 0) return {};
  const stored = new Set((await listPublicKeys()).map((r) => r.fingerprint.toUpperCase()));
  const out: Record<string, PublicKeyBody> = {};
  for (const c of candidates) {
    try {
      const data = decryptedData.get(c.filename) ?? await bytes(attachmentUrl(msg, c.filename));
      const key = await parsePublicKey(new TextDecoder().decode(data));
      if (!stored.has(key.getFingerprint().toUpperCase())) out[c.filename] = publicKeyBody(key, 'message_attachment');
    } catch { /* only parsed public keys get an import button */ }
  }
  return out;
}

// ── Messages read here: sinter-mail's mail.mime, and the OpenPGP steps it asks for ──────────────────────────────

// The open message's source (its bytes never changed: mail.mime caches by identity), the encrypted parts the reader
// chose to open in it (by step id; `openIds`: by part id, one whose ciphertext was cut and has no step until the
// whole message is read), and whether it needs the whole message (`whole`: a signature over cut attachments to
// verify, a cut encrypted part to open). Dropped when another message is read.
let current: { key: string; src: Source; open: string[]; openIds?: string[]; whole?: boolean } | null = null;

type Hole = NonNullable<MimeOptions['holes']>[number];

/**
 * A message's source as the server sends it (`/api/email/message/raw`): cut (`omit`: the bodies of attachments over
 * 64 KiB left out, where they were in `Mail-Holes`; sinter-mail's "Large attachments: cut messages"), so its text
 * renders without downloading the attachments first, or whole. Every message is fetched cut first: an armored
 * ciphertext is never cut (so it comes whole in that one request), a base64 one is, and is then fetched whole to
 * decrypt. `etag` is the whole message's, for the Range requests of cut bodies.
 */
interface Source { url: string; raw: Uint8Array; holes: Hole[]; etag: string }

const OMIT = 64 * 1024;

async function source(msg: Message, whole: boolean): Promise<Source> {
  const url = messageUrl('message/raw', msg);
  const res = await apiFetch(whole ? url : `${url}&omit=${OMIT}`);
  if (!res.ok) throw new Error(`${res.status}`);
  const holes = whole ? [] : JSON.parse(res.headers.get('Mail-Holes') ?? '[]') as Hole[];
  return { url, raw: new Uint8Array(await res.arrayBuffer()), holes, etag: res.headers.get('ETag') ?? '' };
}

/** The decoded bytes of a Ref; one that touches a cut body (`missing`) with that range of the whole message fetched. */
async function partBytes(src: Source, ref: Ref, r: Results): Promise<Uint8Array> {
  if (!ref.missing) return mail.bytes(src.raw, ref, r);
  const res = await apiFetch(src.url, { headers: { Range: `bytes=${ref.missing.start}-${ref.missing.end - 1}`, 'If-Range': src.etag } });
  if (res.status !== 206) throw new Error(`the part: HTTP ${res.status}`);
  return mail.bytes(src.raw, ref, { ...r, fetched: new Uint8Array(await res.arrayBuffer()) });
}

/** Every message a message holds: itself and the ones forwarded inline in it. */
function messagesOf(msg: MimeMessage): MimeMessage[] {
  return [msg, ...msg.parts.flatMap((p) => (p.type === 'message' ? messagesOf(p.message) : []))];
}

/** The decoded bytes and their image type, by the magic number (never the declared type, never SVG). */
async function image(src: Source, ref: Ref, r: Results): Promise<{ bytes: Uint8Array; type: ImageType | null }> {
  const data = await partBytes(src, ref, r);
  return { bytes: data, type: await mail.imageType(data) };
}

function dataUrl(data: Uint8Array, type: string): string {
  let binary = '';
  for (let i = 0; i < data.length; i += 0x8000) binary += String.fromCharCode(...data.subarray(i, i + 0x8000));
  return `data:${type};base64,${btoa(binary)}`;
}

/**
 * The reader's body of a message read from its source; null when the source is cut and that is not enough to show
 * it: a cut the reader does not accept (`cut: 'invalid'`), or a ciphertext that was cut (a decrypt step
 * `incomplete`): read the whole message. A signature over a cut body (a verify step `incomplete`) is left
 * unchecked: the body is shown `pending`, the message marked signed, until the reader asks to verify it.
 */
async function fromRaw(msg: Message, src: Source, out: ReaderBody): Promise<ReaderBody | null> {
  const raw = src.raw;
  const r: Results = { decrypted: {}, verdicts: {}, open: [...(current?.open ?? [])], holes: src.holes };
  let mime: MimeMessage;
  try {
    // encrypted parts the reader asked to open while their ciphertext was cut: their steps, now that they have one
    if (current?.openIds?.length) {
      const ids = new Set(current.openIds);
      const steps = messagesOf(await mail.mime(raw, r)).flatMap((x) => x.parts)
        .flatMap((p) => (p.type === 'encrypted' && ids.has(p.id) && p.step ? [p.step] : []));
      r.open = [...new Set([...r.open, ...steps])];
      current.open = r.open;
    }
    mime = await settle(raw, r, getKeys(), async () => knownKeys(await listPublicKeys().catch(() => [])));
  } catch (e) {
    // holes the reader refuses (malformed, out of order: a TypeError, as any invalid option): read the whole message
    if (src.holes.length && e instanceof TypeError) return null;
    throw e;
  }
  if (src.holes.length && (mime.cut !== 'valid' || mime.steps.some((s) => s.incomplete && s.kind === 'decrypt'))) return null;
  out.pending = mime.steps.some((s) => s.incomplete);
  out.wholeSize = raw.length + src.holes.reduce((n, h) => n + h.end - h.start, 0);
  const text = await mail.mime(raw, { ...r, prefer: 'text' });
  const decrypts = mime.steps.filter((s) => s.kind === 'decrypt');
  out.encrypted ||= decrypts.length > 0 || mime.parts.some((p) => p.type === 'encrypted');
  out.decrypted = mime.summary.encrypted !== 'none';
  out.signed ||= mime.steps.some((s) => s.kind !== 'decrypt');
  out.badges = messageBadges(mime);
  for (const s of decrypts) {
    const d = r.decrypted[s.id];
    if (d?.status === 'locked') out.locked = true;
    else if (d && d.status !== 'ok') {
      out.badges.push(d.status === 'no-key' ? 'missing_key' : d.status === 'unsupported' ? 'unsupported_alg' : 'decrypt_failed');
      out.error = m.pgp_decryptFailed();
    } else if (s.state === 'failed') out.error = m.pgp_decryptFailed();
  }
  // locked is not failed: nothing was tried yet, and nothing is known of the sender
  out.badges = [...new Set(out.badges)].filter((x) => !out.locked || (x !== 'decrypt_failed' && x !== 'from_unknown'));
  if (out.pending) out.badges.push('signed');
  out.subject = protectedSubject(mime);
  out.reply = protectedReply(mime);
  out.quotable = quotable(mime);
  out.parts = readerParts(mime);
  out.textParts = readerParts(text).map((p) => (p.kind === 'html' ? { ...p, kind: 'text', text: htmlToPlainText(p.html), html: '' } : p));
  out.html = out.parts.find((p) => p.kind === 'html' && p.depth === 0 && !p.opened)?.html ?? '';
  out.text = out.textParts.filter((p) => p.kind === 'text' && p.depth === 0 && !p.opened).map((p) => p.text).join('\n\n');
  out.mode = out.parts.some((p) => p.kind === 'html' && htmlAddsValue(p.html)) ? 'html' : 'text';

  // inline images: a decrypted one as a data: URL of its bytes' own image type (the CSP's img-src allows 'self' and
  // data:), per HTML part, and for a forward; any other the server serves by content id (bodyImages), so the body
  // renders without waiting for it and a cut one is never fetched here
  const all = messagesOf(mime);
  const attachment = (id: string) => all.flatMap((x) => x.attachments).find((a) => a.id === id);
  out.cids = {};
  for (const x of all) {
    for (const p of x.parts) {
      if (p.type !== 'html') continue;
      const inline = p.inline.map((i) => ({ contentId: i.contentId, a: attachment(i.attachment) }))
        .filter((i): i is { contentId: string; a: NonNullable<typeof i.a> } => !!i.a?.protection.encrypted);
      if (!inline.length) continue;
      const cid: Record<string, string> = {};
      for (const { contentId, a } of inline) {
        const { bytes: data, type } = await image(src, a.ref, r);
        if (!type) continue;
        cid[contentId] = dataUrl(data, `image/${type}`);
        if (x === mime) (out.forwardCids ??= {})[contentId] = cid[contentId];
      }
      out.cids[p.id] = cid;
    }
  }

  // decrypted attachments (the server has only the ciphertext): offered as files, an image shown by its bytes' type
  for (const a of mime.attachments) {
    if (!a.protection.encrypted || a.kind === 'related') continue;
    const { bytes: data, type } = await image(src, a.ref, r);
    decryptedData.set(a.filename, data);
    const shownType = type ? `image/${type}` : 'application/octet-stream';
    out.files.push({ filename: a.filename, content_type: shownType, size: a.size, content_id: null, url: blobUrl(data, shownType) });
  }
  const declared = mime.attachments.filter((a) => a.protection.encrypted && a.kind !== 'related')
    .map((a) => ({ filename: a.filename, content_type: a.kind === 'pgp-keys' ? 'application/pgp-keys' : a.contentType }));
  out.importable = await importableKeys(msg, out.encrypted ? declared : msg.attachments);
  void harvestAutocrypt(mime.outer.autocrypt, msg).catch(() => {});
  return out;
}

function blank(msg: Message): ReaderBody {
  return {
    key: messageKey(msg), message: msg, parts: [], textParts: [], html: '', text: '', quotable: true, mode: 'text',
    encrypted: msg.is_encrypted, decrypted: false, signed: msg.is_signed, locked: false, badges: [], subject: '', reply: null, error: '',
    files: [], importable: {}, cids: null, forwardCids: null, pending: false, wholeSize: 0,
  };
}

/**
 * The reader's body of `msg`, read here from its source by sinter-mail's mail.mime, its OpenPGP steps run with the
 * unlocked and trusted keys: from the cut message (its large attachments left on the server), whole when the cut is
 * not enough to show it (a cut ciphertext among them). A signature over a cut attachment is checked once the reader
 * asks (`pending`, mail.verify()): that downloads the attachments.
 */
export async function body(ref: MessageRef, ctx: CallContext): Promise<ReaderBody> {
  const msg = await fetchMessage(ref, ctx, false);
  ctx.signal.throwIfAborted();
  forget();
  const key = messageKey(msg);
  if (current?.key !== key) current = null;
  try {
    const want = current?.whole && current.src.holes.length > 0;
    let src = (!want && current?.src) || await source(msg, !!want);
    ctx.signal.throwIfAborted();
    current = { key, src, open: current?.open ?? [], openIds: current?.openIds, whole: current?.whole };
    let out = await fromRaw(msg, src, blank(msg));
    if (!out) {
      // the cut is not enough (one the reader does not accept, or a step that needs a cut body): the whole message
      ctx.signal.throwIfAborted();
      forget();
      src = await source(msg, true);
      ctx.signal.throwIfAborted();
      current = { ...current, key, src };
      out = await fromRaw(msg, src, blank(msg));   // never null: a whole message has no holes
    }
    ctx.signal.throwIfAborted();
    return out ?? blank(msg);
  } catch (e) {
    if (ctx.signal.aborted) throw ctx.signal.reason;
    // openpgp that could not be loaded (offline, or removed by a deploy) is not a decryption failure
    if (e instanceof SinterError) return { ...blank(msg), error: m.pgp_loadError() };
    if (e instanceof Error && /^\d+$/.test(e.message)) return { ...blank(msg), error: m.mail_bodyError() };
    throw e;
  }
}

/** Asks to verify the signature of the open message (a `pending` body): the reader then reloads its body, read whole. */
export function verify(ref: MessageRef): void {
  if (current?.key === messageKey(ref)) current.whole = true;
}

/**
 * Drops what the open message decrypted (the page was hidden or left): its attachments' blob: URLs and bytes, and the
 * plaintexts sinter-mail's mail.mime keeps in its cache. The reader reads the body again when it is shown.
 */
export function wipe(): void {
  forget();
  void mail.forget();
}

/**
 * Opens an encrypted part inside the open message (one the reader chose: not the message's own root), by its step,
 * or, in a cut message, by its part id in the whole message (a cut message's steps take no result); the reader then
 * reloads its body.
 */
export function openPart(ref: MessageRef, step: string, part = ''): void {
  if (current?.key !== messageKey(ref)) return;
  if (step && !step.endsWith('#cut')) {
    if (!current.open.includes(step)) current.open = [...current.open, step];
  } else if (part) {
    // a cut message's step (`<part>#cut`, which no step of the whole message matches) or none (its ciphertext was
    // cut, reason 'incomplete'): the whole message is read, the part opened in it by its id
    current.whole = true;
    current.openIds = [...(current.openIds ?? []), part];
  }
}

/** Unlocks the synced OpenPGP vault with this device's passkey; the reader then reloads its body. */
export async function unlock(): Promise<boolean> {
  try {
    const vault = await readVault();
    if (!vault) throw new Error('no vault');
    const device = localActiveDevice(vault);
    if (!device) throw new Error('no local approved device');
    const { prfOutput } = await webauthnUnlock(device.prf_salt, device.cred_id);
    const { raw, key } = await unwrapVaultKey(device, prfOutput);
    try {
      await validateVault(vault, key, device.cred_id);
      openVault(key, await decryptPrivateKeys(key, vault.keys));
      rememberLocalDevice(device.cred_id);
      return true;
    } finally {
      eraseBytes(raw);
    }
  } catch (e) {
    if (e instanceof SinterError) throw e;   // openpgp could not be loaded: the markup says to reload
    console.error('[pgp] unlock vault failed', e);
    throw new Error(`${m.pgp_unlockError()} ${e instanceof Error ? e.message : String(e)}`);
  }
}

export function unlocked(): boolean {
  return isUnlocked();
}

/** A calendar attachment as a File, for the import dialog (src/components/import-dialog.html). */
export async function icsFile(msg: MessageRef, filename: string, contentType: string): Promise<File> {
  try {
    const data = decryptedData.get(filename) ?? await bytes(attachmentUrl(msg, filename));
    return new File([data as BlobPart], filename, { type: contentType || 'text/calendar' });
  } catch {
    throw new Error(m.cal_importError());
  }
}

/** Opens the message in its own window (/mail/view/). */
export function popout(msg: MessageRef | null): void {
  if (!msg) return;
  const params = new URLSearchParams({ account_id: msg.account_id, mailbox: msg.mailbox, uid: String(msg.uid) });
  window.open(pagePath(`/mail/view/?${params}`), '_blank', 'popup=yes,width=980,height=760,noopener,noreferrer');
}

// ── Compose ───────────────────────────────────────────────────────────────────

/**
 * The keys compose may encrypt to: each trusted key with the address it was stored for (an own key with the
 * addresses its user IDs name, matched to the identity), the keys found in mail that wait to be accepted (`pending`,
 * with their user IDs, for compose to offer accepting them), the vault's key fingerprints and the unlocked keys' emails.
 */
export async function composeKeys(): Promise<ComposeKeys> {
  const [records, vault] = await Promise.all([listPublicKeys().catch(() => []), readVault().catch(() => null)]);
  const keys = [];
  const pending: PendingKey[] = [];
  for (const record of records) {
    try {
      const key = await parsePublicKey(record.armored);
      if (record.trust === 'trusted') {
        const emails = record.is_own ? keyFacts(key).emails : [record.email.trim().toLowerCase()].filter(Boolean);
        keys.push({ fingerprint: record.fingerprint, emails, is_own: record.is_own });
      } else if (!record.is_own && record.email) {
        pending.push({ fingerprint: record.fingerprint, email: record.email.trim().toLowerCase(), userIds: keyFacts(key).userIds, change: keyChanges(record, records) });
      }
    } catch (e) {
      if (e instanceof SinterError) throw e;
      /* skip unreadable keys */
    }
  }
  const unlockedEmails = getKeys().flatMap((k) => k.users.map((u) => u.userID?.email?.trim().toLowerCase() ?? '')).filter(Boolean);
  return {
    keys,
    pending,
    vault: vault && localActiveDevice(vault) ? vault.keys.map((r) => r.fingerprint.toUpperCase()) : [],
    unlocked: unlockedEmails,
  };
}

/**
 * The keys a message is encrypted to: for each recipient the trusted keys stored for exactly that address (never a
 * key that merely names it in a user ID), and the sender's own trusted keys that `ownKey` matches to the identity.
 */
export function encryptionRecords(records: StoredPublicKey[], recipients: string[], ownKey: (r: StoredPublicKey) => boolean): StoredPublicKey[] {
  const wanted = new Set(recipients.map((e) => e.trim().toLowerCase()));
  return trustedKeys(records).filter((r) => (r.is_own ? ownKey(r) : wanted.has(r.email.trim().toLowerCase())));
}

/**
 * The attachments a forward carries (all but the inline images the HTML body references), and the HTML it forwards:
 * a decrypted message's inline images (`cids`, ReaderBody.cids) written in as data: URLs, since the server has no
 * parts to send for them.
 */
export async function forwardFiles(msg: Message, html: string, cids: Record<string, string> | null): Promise<{ files: File[]; html: string }> {
  const forwardHtml = html && cids ? withInlineImages(html, cids) : html;
  const inline = new Set(forwardHtml ? cidReferencesFromHtml(forwardHtml) : []);
  const files: File[] = [];
  for (const att of msg.attachments) {
    if (att.content_id && inline.has(att.content_id)) continue;
    try {
      files.push(new File([await bytes(attachmentUrl(msg, att.filename)) as BlobPart], att.filename, { type: att.content_type || 'application/octet-stream' }));
    } catch { /* skip the attachment */ }
  }
  return { files, html: forwardHtml };
}

async function unlockForSigning() {
  try { await unlock(); } catch { /* the caller reports the missing key */ }
}

/** Builds the (signed, encrypted) MIME message of the draft and sends it with POST /api/email/send-raw. */
export async function send(draft: ComposeDraft, identities: Identity[], sign: boolean, ctx: CallContext): Promise<null> {
  const identity = identities.find((i) => i.identity_id === draft.identity_id);
  if (!identity) throw new Error(m.pgp_composeIdentityMissing());
  const to = parseEnvelopeAddresses(draft.to);
  const cc = parseEnvelopeAddresses(draft.cc);
  const bcc = parseEnvelopeAddresses(draft.bcc);
  const signingKeyFor = () => getKeys().find((k) => keyHasEmail(k, identity.email)) ?? null;
  let signingKey = sign ? signingKeyFor() : null;
  if (sign && !signingKey) {
    await unlockForSigning();
    signingKey = signingKeyFor();
  }
  if (sign && !signingKey) throw new Error(isUnlocked() ? m.pgp_composeNoSigningKey() : m.pgp_composeUnlockSigningKey());

  let encryptionKeys: openpgp.PublicKey[] = [];
  if (draft.encrypt) {
    if (to.length + cc.length + bcc.length === 0) throw new Error(m.pgp_composeNoRecipients());
    const keys = await composeKeys();
    if (!canEncrypt(draft, identities, keys)) throw new Error(m.pgp_composeMissingKeys({ emails: missingKeys(draft, identities, keys).join(', ') }));
    const records = await listPublicKeys();
    const parsed = new Map<string, openpgp.PublicKey>();
    for (const r of trustedKeys(records)) {
      const key = await parsePublicKey(r.armored).catch(() => null);
      if (key) parsed.set(r.fingerprint, key);
    }
    const ownMatches = (r: StoredPublicKey) => { const key = parsed.get(r.fingerprint); return !!key && keyHasEmail(key, identity.email); };
    encryptionKeys = encryptionRecords(records, [...to, ...cc, ...bcc], ownMatches).flatMap((r) => parsed.get(r.fingerprint) ?? []);
  }

  const inlineImages: { cid: string; contentType: string; bytes: Uint8Array }[] = [];
  if (draft.forwardHtml && draft.forwardRef) {
    for (const cid of cidReferencesFromHtml(draft.forwardHtml)) {
      try {
        const ref = draft.forwardRef;
        const res = await apiFetch(`${messageUrl('proxy-image', ref)}&cid=${encodeURIComponent(cid)}`);
        if (res.ok) inlineImages.push({ cid, contentType: res.headers.get('content-type') ?? 'image/jpeg', bytes: new Uint8Array(await res.arrayBuffer()) });
      } catch { /* skip the image */ }
    }
  }

  const intro = draft.body.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\r?\n/g, '<br>');
  const raw = await buildProtectedMime({
    identity, to, cc,
    subject: draft.subject,
    body: draft.forwardText !== null ? draft.body + draft.forwardText : draft.body,
    bodyHtml: draft.forwardHtml ? (draft.body.trim() ? `<div>${intro}</div>${draft.forwardHtml}` : draft.forwardHtml) : undefined,
    inlineImages,
    inReplyTo: draft.in_reply_to,
    references: draft.references,
    attachments: draft.files,
    encrypt: draft.encrypt,
    sign,
    encryptionKeys,
    signingKey,
  });
  const form = new FormData();
  form.append('identity_id', draft.identity_id);
  form.append('envelope_to', JSON.stringify(to));
  form.append('envelope_cc', JSON.stringify(cc));
  form.append('envelope_bcc', JSON.stringify(bcc));
  form.append('raw_mime', new Blob([raw as BlobPart], { type: 'message/rfc822' }), 'message.eml');
  await ctx.request('/api/email/send-raw', { method: 'POST', body: form });
  return null;
}

/** A message another page asked to compose (share as e-mail: subject and one attachment), once. */
export function pendingCompose(): { subject: string; files: File[] } | null {
  const pending = sessionStorage.getItem('caley_compose');
  if (!pending) return null;
  sessionStorage.removeItem('caley_compose');
  try {
    const { subject, filename, mimeType, content } = JSON.parse(pending);
    return { subject, files: [new File([content], filename, { type: mimeType })] };
  } catch {
    return null;
  }
}

// `mail` actions (mail/index.html, mail/view/index.html, src/components/mail-*.html): the mail page's imperative work —
// MIME parsing, OpenPGP, the vault, building and sending raw MIME, opening a window. Plain requests (flags, moves,
// deletions, bulk changes, folders, image origins, storing a key) are in the markup.
// Each returns data; the markup puts it where it belongs (data-sn-then, <sn-resource call>).
import type { CallContext } from '@tiaforge-ui/sinter';
import { apiFetch } from '../lib/api.ts';
import {
  attachmentUrl, canEncrypt, messageKey, type ComposeDraft, type ComposeKeys, type Identity, type Message, type MessageRef,
} from '../lib/mail.ts';
import { cidReferencesFromHtml, htmlToPlainText, withInlineImages } from '../lib/mail-html.ts';
import { attachmentDataUrl, canPreviewContentType, htmlAddsValue, isPgpPublicKeyAttachment, bareMailbox, keyHasEmail } from '../lib/mailUtils.ts';
// openpgp is a lazy package (sinter({ lazy }) in vite.config.ts): these modules import it as usual, sinter loads it
// on the first call, and a failed load is a SinterError (stale when a deploy removed it).
import { detectPgpMessage, parseMimeMessage } from '../lib/pgp/mime.ts';
import { getKeys, isUnlocked, openVault } from '../lib/pgp/session.ts';
import { autocryptKeys } from '../lib/pgp/autocrypt.ts';
import { listPublicKeys, parsePublicKey, publicKeyBody, readVerificationKeys, savePublicKey, type PublicKeyBody } from '../lib/pgp/public-keys.ts';
import { decryptMessage, verifySignedMessage } from '../lib/pgp/keyring.ts';
import { decryptPrivateKeys, eraseBytes, localActiveDevice, readVault, rememberLocalDevice, unwrapVaultKey, validateVault } from '../lib/pgp/vault.ts';
import { unlock as webauthnUnlock } from '../lib/pgp/webauthn.ts';
import { buildProtectedMime, parseEnvelopeAddresses } from '../lib/pgp/compose.ts';
import type * as openpgp from 'openpgp';
import { SinterError } from '@tiaforge-ui/sinter';
import { m } from '@tiaforge-ui/sinter/messages';
import { pagePath } from '../lib/locale-path.ts';

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
  /** the HTML body as the message has it: the reader renders it with email.parse and <mail-body> (sinter-mail) */
  html: string;
  text: string;
  mode: 'html' | 'text';
  encrypted: boolean;
  signed: boolean;
  /** encrypted and no key is unlocked yet: the reader offers to unlock the vault */
  locked: boolean;
  badges: string[];
  /** the protected subject of a decrypted message */
  subject: string;
  error: string;
  /** decrypted attachments, as blob: URLs */
  files: DecryptedFile[];
  /** the public keys not stored yet, by the name of the attachment that holds them: the bodies of POST /pgp/key */
  importable: Record<string, PublicKeyBody>;
  /** a decrypted message's inline images (`cid:` parts) as data: URLs, by content id; null for other messages */
  cids: Record<string, string> | null;
}

// Bytes of the open message's decrypted attachments (for key import and calendar import), by file name.
let decryptedData = new Map<string, Uint8Array>();
let blobUrls: string[] = [];

function forget() {
  for (const url of blobUrls) URL.revokeObjectURL(url);
  blobUrls = [];
  decryptedData = new Map();
}

async function bytes(url: string): Promise<Uint8Array> {
  const res = await apiFetch(url);
  if (!res.ok) throw new Error(`${res.status}`);
  return new Uint8Array(await res.arrayBuffer());
}

/** Stores the sender's Autocrypt keys; in the background (body() does not wait), errors ignored. */
async function harvestAutocrypt(headers: string[], msg: Message) {
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

const verificationKeys = () => listPublicKeys().then(readVerificationKeys).catch(() => []);

async function decrypt(msg: Message, raw: Uint8Array, body: ReaderBody): Promise<ReaderBody> {
  const result = await decryptMessage(getKeys(), raw, await verificationKeys());
  if (result.badges.some((b) => b === 'decrypt_failed' || b === 'missing_key' || b === 'unsupported_alg')) {
    return { ...body, badges: result.badges, error: m.pgp_decryptFailed() };
  }
  const files = result.attachments.map((a) => {
    const url = URL.createObjectURL(new Blob([a.data as BlobPart], { type: canPreviewContentType(a.contentType) ? a.contentType : 'application/octet-stream' }));
    blobUrls.push(url);
    decryptedData.set(a.filename, a.data);
    return { filename: a.filename, content_type: a.contentType, size: a.data.byteLength, content_id: null, url };
  });
  const next: ReaderBody = { ...body, badges: result.badges, subject: result.subject ?? '', files };
  next.cids = {};
  for (const a of result.attachments) {
    if (a.contentId && /^image\/(png|jpeg|gif|webp|avif)$/i.test(a.contentType)) next.cids[a.contentId] = attachmentDataUrl(a);
  }
  if (result.html) {
    next.html = result.html;
    next.text = result.text ?? htmlToPlainText(result.html);
    next.mode = htmlAddsValue(result.html) ? 'html' : 'text';
  } else if (result.text) {
    next.html = '';
    next.text = result.text;
    next.mode = 'text';
  }
  next.importable = await importableKeys(msg, files);
  return next;
}

/**
 * The reader's body of `msg` (MailApp.openMessage): the cached body, or the raw MIME parsed here; signatures
 * verified, Autocrypt keys harvested, and an encrypted message decrypted when a key is unlocked.
 */
export async function body(ref: MessageRef, ctx: CallContext): Promise<ReaderBody> {
  const msg = await fetchMessage(ref, ctx, false);
  ctx.signal.throwIfAborted();
  forget();
  let out: ReaderBody = {
    key: messageKey(msg), html: '', text: '', mode: 'text', encrypted: msg.is_encrypted, signed: msg.is_signed,
    locked: false, badges: [], subject: '', error: '', files: [], importable: {}, cids: null,
  };
  if (msg.body_cached && !msg.is_encrypted) {
    out.html = msg.body_html ?? '';
    out.text = msg.body_text ?? '';
    out.mode = htmlAddsValue(msg.body_html) ? 'html' : 'text';
  }
  void harvestAutocrypt(msg.autocrypt_headers, msg).catch(() => {});
  let raw: Uint8Array | null = null;
  if (!msg.body_cached || msg.is_encrypted || msg.is_signed) {
    try {
      raw = await bytes(messageUrl('message/raw', msg));
      ctx.signal.throwIfAborted();
      const detected = detectPgpMessage(raw);
      out.encrypted ||= detected.isEncrypted;
      out.signed ||= detected.isSigned;
      if (!out.encrypted && !msg.body_cached) {
        const parsed = parseMimeMessage(raw);
        out.html = parsed.html ?? '';
        out.text = parsed.text ?? '';
        out.mode = htmlAddsValue(parsed.html) ? 'html' : 'text';
      }
      if (!msg.body_cached) {
        void harvestAutocrypt(new TextDecoder().decode(raw).split(/\r?\n\r?\n/, 1)[0]
          .replace(/\r?\n[ \t]+/g, ' ').split(/\r?\n/)
          .filter((line) => line.toLowerCase().startsWith('autocrypt:'))
          .map((line) => line.slice(line.indexOf(':') + 1).trim()), msg).catch(() => {});
      }
      if (detected.isSigned && !out.encrypted) {
        out.badges = await verifySignedMessage(raw, await verificationKeys());
      }
    } catch {
      if (ctx.signal.aborted) throw ctx.signal.reason;
      // Raw MIME unavailable: the message still renders without it.
    }
  }
  out.importable = await importableKeys(msg, msg.attachments);
  if (out.encrypted && raw) {
    if (getKeys().length === 0) out.locked = true;
    else {
      // openpgp that could not be loaded (offline, or removed by a deploy) is not a decryption failure.
      try { out = await decrypt(msg, raw, out); } catch (e) { out.error = e instanceof SinterError ? m.pgp_loadError() : m.pgp_decryptFailed(); }
    }
  }
  ctx.signal.throwIfAborted();
  return out;
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
export function popout(msg: MessageRef): void {
  const params = new URLSearchParams({ account_id: msg.account_id, mailbox: msg.mailbox, uid: String(msg.uid) });
  window.open(pagePath(`/mail/view/?${params}`), '_blank', 'popup=yes,width=980,height=760,noopener,noreferrer');
}

// ── Compose ───────────────────────────────────────────────────────────────────

/** The trusted public keys (emails per key), the vault's key fingerprints and the unlocked keys' emails. */
export async function composeKeys(): Promise<ComposeKeys> {
  const [records, vault] = await Promise.all([listPublicKeys().catch(() => []), readVault().catch(() => null)]);
  const keys = [];
  for (const record of records.filter((r) => r.trust === 'trusted')) {
    try {
      const key = await parsePublicKey(record.armored);
      const emails = key.users.map((u) => u.userID?.email?.trim().toLowerCase()).filter((e): e is string => !!e);
      keys.push({ fingerprint: record.fingerprint, emails, is_own: record.is_own });
    } catch { /* skip unreadable keys */ }
  }
  const unlockedEmails = getKeys().flatMap((k) => k.users.map((u) => u.userID?.email?.trim().toLowerCase() ?? '')).filter(Boolean);
  return {
    keys,
    vault: vault && localActiveDevice(vault) ? vault.keys.map((r) => r.fingerprint.toUpperCase()) : [],
    unlocked: unlockedEmails,
  };
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
    if (!canEncrypt(draft, identities, keys)) {
      const missing = [...to, ...cc, ...bcc, identity.email].filter((e) => !keys.keys.some((k) => k.emails.includes(e.toLowerCase())));
      throw new Error(m.pgp_composeMissingKeys({ emails: [...new Set(missing)].join(', ') }));
    }
    const wanted = new Set([...to, ...cc, ...bcc].map((e) => e.toLowerCase()));
    for (const record of (await listPublicKeys()).filter((r) => r.trust === 'trusted')) {
      const key = await parsePublicKey(record.armored).catch(() => null);
      if (!key) continue;
      if ([...wanted].some((e) => keyHasEmail(key, e)) || (record.is_own && keyHasEmail(key, identity.email))) encryptionKeys.push(key);
    }
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

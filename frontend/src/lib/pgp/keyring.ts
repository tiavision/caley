// openpgp.js wrappers for key import, decryption, and signature verification.
// CSP constraint: no 'unsafe-eval', no wasm — Argon2 S2K is explicitly rejected.

import * as openpgp from 'openpgp';

export type DecryptBadge =
  | 'encrypted'       // decrypted successfully
  | 'signed'          // signature present
  | 'verified'        // signature cryptographically valid
  | 'unverified'      // signed but key not available
  | 'missing_key'     // encrypted but our private key not present
  | 'unsupported_alg' // Argon2 or other unsupported S2K
  | 'verification_failed' // signature invalid
  | 'decrypt_failed'; // decryption error

export interface DecryptResult {
  text: string | null;
  html: string | null;
  badges: DecryptBadge[];
  attachments: DecryptedAttachment[];
}

export interface DecryptedAttachment {
  filename: string;
  contentType: string;
  contentId: string | null;
  data: Uint8Array;
}

export interface PgpMessageDetection {
  isEncrypted: boolean;
  isSigned: boolean;
}

function hasArgon2(err: unknown): boolean {
  return err instanceof Error && err.message.toLowerCase().includes('argon2');
}

/** Import and decrypt a private key from armored text + passphrase. */
export async function importPrivateKey(armored: string, passphrase: string): Promise<openpgp.PrivateKey> {
  const key = await openpgp.readPrivateKey({ armoredKey: armored });
  if (key.isDecrypted()) return key;
  const decrypted = await openpgp.decryptKey({ privateKey: key, passphrase });
  return decrypted;
}

/** Return the primary fingerprint (uppercase hex) of a private or public key. */
export function getFingerprint(key: openpgp.Key): string {
  return key.getFingerprint().toUpperCase();
}

/** Return the primary email from the key's User ID. */
export function getPrimaryEmail(key: openpgp.Key): string {
  const uid = key.users[0]?.userID?.email ?? '';
  return uid;
}

export interface KeyIdentity {
  name: string;
  email: string;
  comment: string;
  userID: string;
}

/** Return all textual User IDs certified on a key. */
export function getKeyIdentities(key: openpgp.Key): KeyIdentity[] {
  return key.users.flatMap(user => user.userID ? [{
    name: user.userID.name,
    email: user.userID.email,
    comment: user.userID.comment,
    userID: user.userID.userID,
  }] : []);
}

/** Self-certify another User ID while retaining existing key certifications. */
export async function addKeyIdentity(
  privateKey: openpgp.PrivateKey,
  identity: openpgp.UserID,
): Promise<openpgp.PrivateKey> {
  const reformatted = await openpgp.reformatKey({ privateKey, userIDs: [identity], format: 'object' });
  return privateKey.update(reformatted.publicKey);
}

/** Return armored public key from a private key. */
export function extractPublicKey(privateKey: openpgp.PrivateKey): string {
  return privateKey.toPublic().armor();
}

/**
 * Decrypt a PGP/MIME or inline PGP message from raw MIME bytes.
 * privateKeys — unlocked in-memory private keys (tried in order)
 * rawMime     — the raw .eml bytes fetched from /api/email/raw
 * publicKeys  — trusted public keys available for signer verification
 * Returns decrypted text/html bodies plus signature status badges.
 */
export async function decryptMessage(
  privateKeys: openpgp.PrivateKey[],
  rawMime: Uint8Array,
  publicKeys: openpgp.PublicKey[] = [],
): Promise<DecryptResult> {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawMime);
  const badges: DecryptBadge[] = [];

  // Extract the PGP block from the MIME structure.
  const pgpBlock = extractPgpBlock(mimeStr);
  if (!pgpBlock) {
    return { text: mimeStr, html: null, badges: [], attachments: [] };
  }

  let message: openpgp.Message<string | Uint8Array>;
  try {
    message = await openpgp.readMessage({ armoredMessage: pgpBlock.ciphertext });
  } catch (e) {
    if (hasArgon2(e)) {
      return { text: null, html: null, badges: ['unsupported_alg'], attachments: [] };
    }
    return { text: null, html: null, badges: ['decrypt_failed'], attachments: [] };
  }

  let decrypted: openpgp.DecryptMessageResult;
  try {
    decrypted = await openpgp.decrypt({
      message,
      decryptionKeys: privateKeys,
      ...(publicKeys.length > 0 ? { verificationKeys: publicKeys } : {}),
      config: { allowUnauthenticatedMessages: false },
    });
  } catch (e) {
    if (hasArgon2(e)) {
      return { text: null, html: null, badges: ['unsupported_alg'], attachments: [] };
    }
    return { text: null, html: null, badges: ['decrypt_failed'], attachments: [] };
  }

  badges.push('encrypted');

  // Check signatures
  if (decrypted.signatures.length > 0) {
    badges.push('signed');
    const matchingKeys = publicKeys.filter(key =>
      decrypted.signatures.some(sig =>
        key.getKeyIDs().some(keyId => keyId.equals(sig.keyID))
      )
    );

    if (matchingKeys.length === 0) {
      badges.push('unverified');
    } else {
      let anyVerified = false;
      let anyFailed = false;
      for (const sig of decrypted.signatures) {
        try {
          await sig.verified;
          anyVerified = true;
        } catch {
          anyFailed = true;
        }
      }
      if (anyVerified) badges.push('verified');
      else if (anyFailed) badges.push('verification_failed');
      else badges.push('unverified');
    }
  }

  const decryptedText = typeof decrypted.data === 'string'
    ? decrypted.data
    : new TextDecoder().decode(decrypted.data as Uint8Array);

  // PGP/MIME plaintext is itself a MIME entity. Parse it in the browser: the
  // server cannot see inner bodies or attachments without the private key.
  const inner = parseDecryptedMime(decryptedText);
  if (inner) {
    return { text: inner.text, html: inner.html, badges, attachments: inner.attachments };
  }

  // Inline PGP commonly decrypts to an ordinary body rather than a MIME entity.
  const isHtml = decryptedText.trimStart().toLowerCase().startsWith('<!doctype html') ||
    decryptedText.trimStart().toLowerCase().startsWith('<html');

  return {
    text: isHtml ? null : decryptedText,
    html: isHtml ? decryptedText : null,
    badges,
    attachments: [],
  };
}

/**
 * Parse a plain (non-encrypted) raw MIME message into html/text/attachments.
 * Used to render all messages through the same path as decrypted PGP messages.
 */
export function parseMimeMessage(rawBytes: Uint8Array): { html: string | null; text: string | null; attachments: DecryptedAttachment[] } {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawBytes);
  const parsed = parseDecryptedMime(mimeStr);
  return parsed ?? { html: null, text: mimeStr, attachments: [] };
}

export function detectPgpMessage(rawBytes: Uint8Array): PgpMessageDetection {
  const mime = new TextDecoder('utf-8', { fatal: false }).decode(rawBytes).toLowerCase();
  return {
    isEncrypted: (mime.includes('multipart/encrypted') && mime.includes('application/pgp-encrypted')) ||
      hasAsciiMarker(rawBytes, '-----BEGIN PGP MESSAGE-----'),
    isSigned: (mime.includes('multipart/signed') && mime.includes('application/pgp-signature')) ||
      hasAsciiMarker(rawBytes, '-----BEGIN PGP SIGNED MESSAGE-----'),
  };
}

/**
 * Verify a PGP/MIME or inline signed message.
 */
export async function verifySignedMessage(
  rawMime: Uint8Array,
  publicKeys: openpgp.PublicKey[],
): Promise<DecryptBadge[]> {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawMime);
  const badges: DecryptBadge[] = ['signed'];

  const signedBlock = extractSignedBlock(mimeStr);
  const detached = extractDetachedSignedMime(mimeStr);
  if (!signedBlock && !detached) return badges;
  if (publicKeys.length === 0) return [...badges, 'unverified'];

  try {
    const result = detached
      ? await openpgp.verify({
        message: await openpgp.createMessage({ binary: new TextEncoder().encode(detached.text) }),
        signature: await openpgp.readSignature({ armoredSignature: detached.signature }),
        verificationKeys: publicKeys,
      })
      : await openpgp.verify({
        message: await openpgp.createCleartextMessage({ text: signedBlock!.text }),
        verificationKeys: publicKeys,
      });
    const hasMatchingKey = result.signatures.some(sig =>
      publicKeys.some(key => key.getKeyIDs().some(keyId => keyId.equals(sig.keyID)))
    );
    if (!hasMatchingKey) return [...badges, 'unverified'];

    let anyVerified = false;
    let anyFailed = false;
    for (const sig of result.signatures) {
      try {
        await sig.verified;
        anyVerified = true;
      } catch {
        anyFailed = true;
      }
    }
    if (anyVerified) badges.push('verified');
    else if (anyFailed) badges.push('verification_failed');
    else badges.push('unverified');
  } catch {
    badges.push('verification_failed');
  }

  return badges;
}

export async function verifiesWithKey(rawMime: Uint8Array, key: openpgp.PublicKey): Promise<boolean> {
  return (await verifySignedMessage(rawMime, [key])).includes('verified');
}

// ── MIME parsing helpers ──────────────────────────────────────────────────────

interface PgpBlock {
  ciphertext: string;
}

/** Extract the PGP armored ciphertext from a MIME message string. */
function extractPgpBlock(mime: string): PgpBlock | null {
  const begin = mime.indexOf('-----BEGIN PGP MESSAGE-----');
  const end = mime.indexOf('-----END PGP MESSAGE-----');
  if (begin === -1 || end === -1) return null;
  return { ciphertext: mime.slice(begin, end + '-----END PGP MESSAGE-----'.length) };
}

function hasAsciiMarker(bytes: Uint8Array, marker: string): boolean {
  const needle = new TextEncoder().encode(marker);
  outer: for (let i = 0; i <= bytes.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (bytes[i + j] !== needle[j]) continue outer;
    }
    return true;
  }
  return false;
}

interface SignedBlock {
  text: string;
  signature: string;
}

function extractSignedBlock(mime: string): SignedBlock | null {
  const begin = mime.indexOf('-----BEGIN PGP SIGNED MESSAGE-----');
  const end = mime.indexOf('-----END PGP SIGNATURE-----');
  if (begin === -1 || end === -1) return null;
  const full = mime.slice(begin, end + '-----END PGP SIGNATURE-----'.length);
  const sigBegin = full.indexOf('-----BEGIN PGP SIGNATURE-----');
  return {
    text: full.slice(0, sigBegin).trim(),
    signature: full.slice(sigBegin),
  };
}

function extractDetachedSignedMime(mime: string): SignedBlock | null {
  const headerEnd = mime.search(/\r?\n\r?\n/);
  if (headerEnd === -1) return null;
  const headers = parseHeaders(mime.slice(0, headerEnd));
  const contentType = headers.get('content-type') ?? '';
  if (!contentType.toLowerCase().startsWith('multipart/signed')) return null;
  const boundary = readParameter(contentType, 'boundary');
  if (!boundary) return null;

  // Slice the signed part's exact on-wire octets. Re-canonicalizing line endings
  // (as splitMultipart does) would "repair" mangling introduced in transit/forwarding
  // and validate signatures that strict clients reject. The CRLF that precedes the
  // next boundary delimiter is part of the delimiter (RFC 1521), so it is excluded.
  const delimiter = `--${boundary}`;
  const open = findLineStart(mime, delimiter, headerEnd);
  if (open === -1) return null;
  const openLineEnd = mime.indexOf('\n', open);
  if (openLineEnd === -1) return null;
  const contentStart = openLineEnd + 1;

  const next = mime.indexOf(`\n${delimiter}`, contentStart);
  if (next === -1) return null;
  let contentEnd = next; // index of the '\n' belonging to the delimiter's line break
  if (mime[contentEnd - 1] === '\r') contentEnd -= 1;

  const signatureBegin = mime.indexOf('-----BEGIN PGP SIGNATURE-----', next);
  const signatureEnd = mime.indexOf('-----END PGP SIGNATURE-----', signatureBegin);
  if (signatureBegin === -1 || signatureEnd === -1) return null;
  return {
    text: mime.slice(contentStart, contentEnd),
    signature: mime.slice(signatureBegin, signatureEnd + '-----END PGP SIGNATURE-----'.length),
  };
}

/** Index of `needle` when it starts a line at or after `from`, else -1. */
function findLineStart(mime: string, needle: string, from: number): number {
  if (mime.startsWith(needle, from)) return from;
  const idx = mime.indexOf(`\n${needle}`, from);
  return idx === -1 ? -1 : idx + 1;
}

// ── Decrypted MIME parsing ───────────────────────────────────────────────────

interface ParsedDecryptedMime {
  text: string | null;
  html: string | null;
  attachments: DecryptedAttachment[];
}

interface MimeEntity {
  headers: Map<string, string>;
  body: string;
  contentType: string;
  children: MimeEntity[];
}

function parseDecryptedMime(raw: string): ParsedDecryptedMime | null {
  const entity = parseEntity(raw);
  if (!entity || entity.headers.size === 0) return null;

  const parsed: ParsedDecryptedMime = { text: null, html: null, attachments: [] };
  collectMimeContent(entity, parsed);
  return parsed;
}

function parseEntity(raw: string): MimeEntity | null {
  const match = /\r?\n\r?\n/.exec(raw);
  if (!match) return null;
  const headers = parseHeaders(raw.slice(0, match.index));
  const body = raw.slice(match.index + match[0].length);
  const contentTypeHeader = headers.get('content-type') ?? 'text/plain; charset=utf-8';
  const contentType = contentTypeHeader.split(';', 1)[0].trim().toLowerCase();
  const boundary = readParameter(contentTypeHeader, 'boundary');
  const children: MimeEntity[] = [];

  if (contentType.startsWith('multipart/') && boundary) {
    for (const part of splitMultipart(body, boundary)) {
      const child = parseEntity(part);
      if (child) children.push(child);
    }
  }

  return { headers, body, contentType, children };
}

function parseHeaders(raw: string): Map<string, string> {
  const headers = new Map<string, string>();
  const unfolded = raw.replace(/\r?\n[ \t]+/g, ' ');
  for (const line of unfolded.split(/\r?\n/)) {
    const colon = line.indexOf(':');
    if (colon <= 0) continue;
    headers.set(line.slice(0, colon).trim().toLowerCase(), line.slice(colon + 1).trim());
  }
  return headers;
}

function readParameter(value: string, parameter: string): string | null {
  const pattern = new RegExp(`(?:^|;)\\s*${parameter}(\\*)?\\s*=\\s*(?:"([^"]*)"|([^;]*))`, 'i');
  const match = pattern.exec(value);
  if (!match) return null;
  const raw = (match[2] ?? match[3] ?? '').trim();
  if (match[1]) {
    const encoded = raw.replace(/^[^']*'[^']*'/, '');
    try {
      return decodeURIComponent(encoded);
    } catch {
      return encoded;
    }
  }
  return decodeMimeWords(raw);
}

function splitMultipart(body: string, boundary: string): string[] {
  const parts: string[] = [];
  const lines = body.split(/\r?\n/);
  const marker = `--${boundary}`;
  let current: string[] | null = null;
  for (const line of lines) {
    if (line === marker || line === `${marker}--`) {
      if (current) parts.push(current.join('\r\n'));
      current = line.endsWith('--') ? null : [];
      if (line.endsWith('--')) break;
      continue;
    }
    if (current) current.push(line);
  }
  return parts;
}

function collectMimeContent(entity: MimeEntity, parsed: ParsedDecryptedMime) {
  if (entity.children.length > 0) {
    for (const child of entity.children) collectMimeContent(child, parsed);
    return;
  }

  const disposition = entity.headers.get('content-disposition') ?? '';
  const filename = readParameter(disposition, 'filename') ?? readParameter(entity.headers.get('content-type') ?? '', 'name');
  const contentId = (entity.headers.get('content-id') ?? '').replace(/^<|>$/g, '') || null;
  const dispositionType = disposition.split(';', 1)[0].trim().toLowerCase();
  const bytes = decodeEntityBody(entity);
  const isAttachment = dispositionType === 'attachment' || filename !== null ||
    (dispositionType === 'inline' && entity.contentType.startsWith('image/'));

  if (isAttachment) {
    parsed.attachments.push({
      filename: filename ?? contentId ?? 'attachment',
      contentType: entity.contentType || 'application/octet-stream',
      contentId,
      data: bytes,
    });
    return;
  }

  const text = decodeText(bytes, readParameter(entity.headers.get('content-type') ?? '', 'charset'));
  if (entity.contentType === 'text/html' && parsed.html === null) {
    parsed.html = text;
  } else if (entity.contentType === 'text/plain' && parsed.text === null) {
    parsed.text = text;
  }
}

function decodeEntityBody(entity: MimeEntity): Uint8Array {
  const encoding = (entity.headers.get('content-transfer-encoding') ?? '').toLowerCase();
  if (encoding === 'base64') {
    try {
      const decoded = atob(entity.body.replace(/\s/g, ''));
      return Uint8Array.from(decoded, c => c.charCodeAt(0));
    } catch {
      return new Uint8Array();
    }
  }
  if (encoding === 'quoted-printable') return decodeQuotedPrintable(entity.body);
  return new TextEncoder().encode(entity.body);
}

function decodeQuotedPrintable(value: string): Uint8Array {
  const bytes: number[] = [];
  const unwrapped = value.replace(/=\r?\n/g, '');
  for (let i = 0; i < unwrapped.length;) {
    const encoded = /^=([0-9a-f]{2})/i.exec(unwrapped.slice(i));
    if (encoded) {
      bytes.push(parseInt(encoded[1], 16));
      i += 3;
      continue;
    }
    const codePoint = unwrapped.codePointAt(i)!;
    const char = String.fromCodePoint(codePoint);
    bytes.push(...new TextEncoder().encode(char));
    i += char.length;
  }
  return Uint8Array.from(bytes);
}

function decodeText(bytes: Uint8Array, charset: string | null): string {
  try {
    return new TextDecoder(charset || 'utf-8').decode(bytes);
  } catch {
    return new TextDecoder().decode(bytes);
  }
}

function decodeMimeWords(value: string): string {
  return value.replace(/=\?([^?]+)\?([bq])\?([^?]*)\?=/gi, (_all, charset: string, mode: string, encoded: string) => {
    try {
      const bytes = mode.toLowerCase() === 'b'
        ? Uint8Array.from(atob(encoded), c => c.charCodeAt(0))
        : decodeQuotedPrintable(encoded.replace(/_/g, ' '));
      return decodeText(bytes, charset);
    } catch {
      return encoded;
    }
  });
}

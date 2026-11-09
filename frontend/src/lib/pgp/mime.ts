// MIME parsing without openpgp: what the reader runs on every message (src/actions/mail.ts body()), kept
// out of keyring.ts so a plain message does not load openpgp. keyring.ts decrypts and verifies with it.

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

/**
 * Parse a plain (non-encrypted) raw MIME message into html/text/attachments.
 * Used to render all messages through the same path as decrypted PGP messages.
 */
export function parseMimeMessage(rawBytes: Uint8Array): { html: string | null; text: string | null; attachments: DecryptedAttachment[]; subject: string | null } {
  const mimeStr = new TextDecoder('utf-8', { fatal: false }).decode(rawBytes);
  const parsed = parseDecryptedMime(mimeStr);
  return parsed ?? { html: null, text: mimeStr, attachments: [], subject: null };
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

// ── MIME parsing helpers ──────────────────────────────────────────────────────

interface PgpBlock {
  ciphertext: string;
}

/** Extract the PGP armored ciphertext from a MIME message string. */
export function extractPgpBlock(mime: string): PgpBlock | null {
  const begin = mime.indexOf('-----BEGIN PGP MESSAGE-----');
  const end = mime.indexOf('-----END PGP MESSAGE-----');
  if (begin === -1 || end === -1) return null;
  return { ciphertext: mime.slice(begin, end + '-----END PGP MESSAGE-----'.length) };
}

export function hasAsciiMarker(bytes: Uint8Array, marker: string): boolean {
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

export function extractSignedBlock(mime: string): SignedBlock | null {
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

export function extractDetachedSignedMime(mime: string): SignedBlock | null {
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
  subject: string | null;
}

interface MimeEntity {
  headers: Map<string, string>;
  body: string;
  contentType: string;
  children: MimeEntity[];
}

export function parseDecryptedMime(raw: string): ParsedDecryptedMime | null {
  const entity = parseEntity(raw);
  if (!entity || entity.headers.size === 0) return null;

  const parsed: ParsedDecryptedMime = {
    text: null,
    html: null,
    attachments: [],
    subject: extractProtectedSubject(entity),
  };
  collectMimeContent(entity, parsed);
  return parsed;
}

/**
 * Recover a "protected" Subject carried as a header on the top-level MIME entity
 * inside the cryptographic payload (RFC 9788 / draft-ietf-lamps-header-protection).
 * The `protected-headers="v1"` Content-Type marker is accepted but not required —
 * older clients omit it, and callers always prefer this value when present.
 */
function extractProtectedSubject(entity: MimeEntity): string | null {
  const direct = entity.headers.get('subject');
  if (direct && direct.trim()) return decodeMimeWords(direct).trim();

  // Legacy "Memory Hole" wrapper: a text/rfc822-headers part holding the headers.
  for (const child of entity.children) {
    if (child.contentType === 'text/rfc822-headers') {
      const subject = parseHeaders(child.body).get('subject');
      if (subject && subject.trim()) return decodeMimeWords(subject).trim();
    }
  }
  return null;
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

export function parseHeaders(raw: string): Map<string, string> {
  const headers = new Map<string, string>();
  const unfolded = raw.replace(/\r?\n[ \t]+/g, ' ');
  for (const line of unfolded.split(/\r?\n/)) {
    const colon = line.indexOf(':');
    if (colon <= 0) continue;
    headers.set(line.slice(0, colon).trim().toLowerCase(), line.slice(colon + 1).trim());
  }
  return headers;
}

export function readParameter(value: string, parameter: string): string | null {
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

export function splitMultipart(body: string, boundary: string): string[] {
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

export function decodeText(bytes: Uint8Array, charset: string | null): string {
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

import * as openpgp from 'openpgp';

export interface ComposeIdentity {
  name: string;
  email: string;
}

export interface ProtectedComposeMessage {
  identity: ComposeIdentity;
  to: string[];
  cc: string[];
  subject: string;
  body: string;
  bodyHtml?: string;
  inlineImages?: Array<{ cid: string; contentType: string; bytes: Uint8Array }>;
  inReplyTo: string | null;
  references: string | null;
  attachments: File[];
  encrypt: boolean;
  sign: boolean;
  encryptionKeys: openpgp.PublicKey[];
  signingKey: openpgp.PrivateKey | null;
}

const encoder = new TextEncoder();

function safeHeader(value: string): string {
  return value.replace(/[\r\n]+/g, ' ').trim();
}

function asciiHeader(value: string): string {
  return safeHeader(value).replace(/[^\x20-\x7e]/g, '');
}

function hasNonAscii(value: string): boolean {
  return /[^\x00-\x7f]/.test(value);
}

function encodedWord(value: string): string {
  const safe = safeHeader(value);
  if (!safe) return '';
  if (!hasNonAscii(safe)) return safe;
  return `=?UTF-8?B?${base64Lines(encoder.encode(safe)).replace(/\r\n/g, '')}?=`;
}

function domainToAscii(domain: string): string {
  if (!hasNonAscii(domain)) return domain;
  try {
    return new URL(`http://${domain}`).hostname;
  } catch {
    return domain.replace(/[^\x21-\x7e]/g, '');
  }
}

function asciiAddrSpec(value: string): string | null {
  const trimmed = safeHeader(value);
  const match = /<([^<>]+)>/.exec(trimmed);
  const address = (match?.[1] ?? trimmed).trim();
  const at = address.lastIndexOf('@');
  if (at <= 0 || at === address.length - 1) return null;
  const local = address.slice(0, at);
  const domain = domainToAscii(address.slice(at + 1));
  if (!local || !domain || hasNonAscii(local) || hasNonAscii(domain)) return null;
  return `${local}@${domain}`;
}

function boundary(label: string): string {
  return `----caley-${label}-${crypto.randomUUID().replace(/-/g, '')}`;
}

// Map an OpenPGP hash-algorithm id (RFC 4880 §9.4) to its RFC 3156 micalg token.
const MICALG_BY_HASH: Record<number, string> = {
  1: 'pgp-md5', 2: 'pgp-sha1', 3: 'pgp-ripemd160',
  8: 'pgp-sha256', 9: 'pgp-sha384', 10: 'pgp-sha512', 11: 'pgp-sha224',
};

function micalgForSignature(signature: openpgp.Signature): string {
  const hashAlgorithm = signature.packets[0]?.hashAlgorithm;
  return (hashAlgorithm !== undefined && MICALG_BY_HASH[hashAlgorithm]) || 'pgp-sha256';
}

function base64Lines(bytes: Uint8Array): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return (btoa(binary).match(/.{1,76}/g) ?? []).join('\r\n');
}

function mailbox(identity: ComposeIdentity): string {
  const email = asciiAddrSpec(identity.email) ?? asciiHeader(identity.email);
  const name = encodedWord(identity.name);
  return name ? `${name} <${email}>` : email;
}

function addressListHeader(addresses: string[]): string {
  const normalized = addresses
    .map(address => asciiAddrSpec(address))
    .filter((address): address is string => Boolean(address));
  return normalized.length > 0 ? normalized.join(', ') : 'undisclosed-recipients:;';
}

function quotedPrintable(value: string): string {
  const bytes = encoder.encode(value.replace(/\r?\n/g, '\r\n'));
  const lines: string[] = [];
  let line = '';

  const push = (chunk: string) => {
    if (line.length + chunk.length > 75) {
      lines.push(`${line}=`);
      line = '';
    }
    line += chunk;
  };

  for (let i = 0; i < bytes.length; i += 1) {
    const byte = bytes[i];
    if (byte === 13 && bytes[i + 1] === 10) {
      lines.push(line);
      line = '';
      i += 1;
      continue;
    }

    const next = bytes[i + 1];
    const isLineEnd = next === 13 || next === undefined;
    if ((byte >= 33 && byte <= 60) || (byte >= 62 && byte <= 126) || ((byte === 9 || byte === 32) && !isLineEnd)) {
      push(String.fromCharCode(byte));
    } else {
      push(`=${byte.toString(16).toUpperCase().padStart(2, '0')}`);
    }
  }
  lines.push(line);
  return lines.join('\r\n');
}

function parameterValue(name: string, value: string): string {
  const safe = safeHeader(value).replace(/"/g, '');
  if (!hasNonAscii(safe)) return `${name}="${safe}"`;
  return `${name}*=UTF-8''${encodeURIComponent(safe).replace(/[!'()*]/g, char => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)}`;
}

export function parseEnvelopeAddresses(value: string): string[] {
  return value
    .split(',')
    .map(address => {
      const match = /<([^<>]+)>/.exec(address);
      return (match?.[1] ?? address).trim();
    })
    .filter(address => address.length > 0);
}

function messageHeaderLines(message: ProtectedComposeMessage, date: string, messageId: string): string[] {
  const headers = [
    `Date: ${date}`,
    `Message-ID: ${messageId}`,
    `From: ${mailbox(message.identity)}`,
    `To: ${addressListHeader(message.to)}`,
    ...(message.cc.length ? [`Cc: ${addressListHeader(message.cc)}`] : []),
    `Subject: ${encodedWord(message.subject)}`,
    'MIME-Version: 1.0',
  ];
  if (message.inReplyTo) headers.push(`In-Reply-To: ${asciiHeader(message.inReplyTo)}`);
  if (message.references) headers.push(`References: ${asciiHeader(message.references)}`);
  const autocrypt = autocryptHeader(message);
  if (autocrypt) headers.push(autocrypt);
  return headers;
}

function messageHeaders(headers: string[]): string {
  return `${headers.join('\r\n')}\r\n`;
}

function protectedHeaders(headers: string[]): string {
  return headers
    .filter(header => {
      const name = header.split(':', 1)[0].toLowerCase();
      return name !== 'mime-version' && name !== 'autocrypt';
    })
    .join('\r\n');
}

// Advertise the sender's key via an Autocrypt header instead of attaching it to
// the signed body. K-9 / Thunderbird for Android fails signature verification
// when a public key is attached to a signed message (thundernest/k-9#6511); the
// header sits outside the signature, so it avoids that bug while still letting
// receivers (including caley's own autocryptKeys harvester) import the key.
function autocryptHeader(message: ProtectedComposeMessage): string | null {
  if (!message.signingKey) return null;
  const addr = asciiAddrSpec(message.identity.email);
  if (!addr) return null;
  // RFC 5322 folding: continuation lines start with whitespace; Autocrypt
  // parsers strip the folding before base64-decoding the keydata.
  const keydata = base64Lines(message.signingKey.toPublic().write()).replace(/\r\n/g, '\r\n ');
  return `Autocrypt: addr=${addr};\r\n keydata=${keydata}`;
}

async function innerEntity(message: ProtectedComposeMessage, signedProtectedHeaders: string | null = null): Promise<string> {
  const attachments = [...message.attachments];

  const textPart =
    `Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n` +
    `${quotedPrintable(message.body)}\r\n`;

  let bodyPart: string;
  if (message.bodyHtml) {
    const htmlBody = message.bodyHtml.replace(/\r?\n/g, '\r\n');
    let htmlPart: string;
    if (message.inlineImages && message.inlineImages.length > 0) {
      const relBoundary = boundary('related');
      htmlPart =
        `Content-Type: multipart/related; boundary="${relBoundary}"\r\n\r\n` +
      `--${relBoundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n${quotedPrintable(htmlBody)}\r\n`;
      for (const img of message.inlineImages) {
        const ct = asciiHeader(img.contentType || 'image/jpeg');
        const cid = asciiHeader(img.cid);
        htmlPart +=
          `--${relBoundary}\r\nContent-Type: ${ct}\r\nContent-Transfer-Encoding: base64\r\n` +
          `Content-ID: <${cid}>\r\nContent-Disposition: inline\r\n\r\n${base64Lines(img.bytes)}\r\n`;
      }
      htmlPart += `--${relBoundary}--\r\n`;
    } else {
      htmlPart = `Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n${quotedPrintable(htmlBody)}\r\n`;
    }
    const altBoundary = boundary('alternative');
    bodyPart =
      `Content-Type: multipart/alternative; boundary="${altBoundary}"\r\n\r\n` +
      `--${altBoundary}\r\n${textPart}` +
      `--${altBoundary}\r\n${htmlPart}` +
      `--${altBoundary}--\r\n`;
  } else {
    bodyPart = textPart;
  }

  if (attachments.length === 0) {
    return bodyPart;
  }

  const mixedBoundary = boundary('mixed');
  let entity = signedProtectedHeaders
    ? `Content-Type: multipart/mixed; boundary="${mixedBoundary}";\r\n protected-headers="v1"\r\n${signedProtectedHeaders}\r\n\r\n`
    : `Content-Type: multipart/mixed; boundary="${mixedBoundary}"\r\n\r\n`;
  entity += `--${mixedBoundary}\r\n${bodyPart}`;
  for (const attachment of attachments) {
    const filename = parameterValue('name', attachment.name);
    const dispositionFilename = parameterValue('filename', attachment.name);
    const contentType = safeHeader(attachment.type || 'application/octet-stream');
    const content = new Uint8Array(await attachment.arrayBuffer());
    entity += `--${mixedBoundary}\r\nContent-Type: ${contentType}; ${filename}\r\n`;
    entity += `Content-Disposition: attachment; ${dispositionFilename}\r\nContent-Transfer-Encoding: base64\r\n\r\n`;
    entity += `${base64Lines(content)}\r\n`;
  }
  entity += `--${mixedBoundary}--\r\n`;
  return entity;
}

export async function buildProtectedMime(message: ProtectedComposeMessage): Promise<Uint8Array> {
  const headerLines = messageHeaderLines(
    message,
    new Date().toUTCString(),
    `<${crypto.randomUUID()}@caley.local>`,
  );
  const headers = messageHeaders(headerLines);
  const hasAttachments = message.attachments.length > 0;
  const inner = await innerEntity(
    message,
    message.sign && !message.encrypt && hasAttachments ? protectedHeaders(headerLines) : null,
  );

  if (message.encrypt) {
    const pgpMessage = await openpgp.createMessage({ binary: encoder.encode(inner) });
    const ciphertext = await openpgp.encrypt({
      message: pgpMessage,
      encryptionKeys: message.encryptionKeys,
      ...(message.sign && message.signingKey ? { signingKeys: message.signingKey } : {}),
      format: 'armored',
    });
    const encryptedBoundary = boundary('encrypted');
    const body =
      `Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"; boundary="${encryptedBoundary}"\r\n\r\n` +
      `--${encryptedBoundary}\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n` +
      `--${encryptedBoundary}\r\nContent-Type: application/octet-stream; name="encrypted.asc"\r\n\r\n${ciphertext}\r\n` +
      `--${encryptedBoundary}--\r\n`;
    return encoder.encode(headers + body);
  }

  if (!message.sign || !message.signingKey) {
    // Plain message: return headers + inner entity directly.
    return encoder.encode(headers + inner);
  }
  // RFC 3156 allows signed material to follow the OpenPGP convention of ending
  // with CRLF when the signed and transmitted bytes are identical. For nested
  // multipart/mixed attachment bodies, keep that terminator and add a separate
  // delimiter CRLF before the outer signature boundary, matching Thunderbird's
  // secure attachment-bearing PGP/MIME shape. Simpler non-attachment bodies keep
  // excluding the delimiter CRLF for broad compatibility.
  const signedContent = hasAttachments ? inner : inner.replace(/\r\n$/, '');
  const signedMessage = await openpgp.createMessage({ binary: encoder.encode(signedContent) });
  const signatureObject = await openpgp.sign({
    message: signedMessage,
    signingKeys: message.signingKey,
    detached: true,
    format: 'object',
  });
  // micalg MUST name the hash actually used (RFC 3156 §5). Ed25519 keys hash with
  // SHA-512, not SHA-256 — a hardcoded value makes strict verifiers reject the
  // signature. The armor uses bare LF; MIME requires CRLF throughout the entity.
  const micalg = micalgForSignature(signatureObject);
  const signature = signatureObject.armor().replace(/\r?\n/g, '\r\n');
  const signedBoundary = boundary('signed');
  const signaturePart = hasAttachments
    ? `Content-Type: application/pgp-signature; name="OpenPGP_signature.asc"\r\nContent-Description: OpenPGP digital signature\r\nContent-Disposition: attachment; filename="OpenPGP_signature"\r\n\r\n${signature}\r\n`
    : `Content-Type: application/pgp-signature; name="signature.asc"\r\nContent-Disposition: attachment; filename="signature.asc"\r\n\r\n${signature}\r\n`;
  const body =
    `Content-Type: multipart/signed; protocol="application/pgp-signature"; micalg=${micalg}; boundary="${signedBoundary}"\r\n\r\n` +
    `--${signedBoundary}\r\n${signedContent}\r\n` +
    `--${signedBoundary}\r\n${signaturePart}` +
    `--${signedBoundary}--\r\n`;
  return encoder.encode(headers + body);
}

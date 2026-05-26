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
  inReplyTo: string | null;
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

function boundary(label: string): string {
  return `----caley-${label}-${crypto.randomUUID().replace(/-/g, '')}`;
}

function base64Lines(bytes: Uint8Array): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return (btoa(binary).match(/.{1,76}/g) ?? []).join('\r\n');
}

function mailbox(identity: ComposeIdentity): string {
  const email = safeHeader(identity.email);
  const name = safeHeader(identity.name).replace(/(["\\])/g, '\\$1');
  return name ? `"${name}" <${email}>` : email;
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

function messageHeaders(message: ProtectedComposeMessage): string {
  const headers = [
    `Date: ${new Date().toUTCString()}`,
    `Message-ID: <${crypto.randomUUID()}@caley.local>`,
    `From: ${mailbox(message.identity)}`,
    `To: ${message.to.map(safeHeader).join(', ')}`,
    ...(message.cc.length ? [`Cc: ${message.cc.map(safeHeader).join(', ')}`] : []),
    `Subject: ${safeHeader(message.subject)}`,
    'MIME-Version: 1.0',
  ];
  if (message.inReplyTo) headers.push(`In-Reply-To: ${safeHeader(message.inReplyTo)}`);
  return `${headers.join('\r\n')}\r\n`;
}

async function innerEntity(message: ProtectedComposeMessage): Promise<string> {
  const attachments = [...message.attachments];
  if (message.sign && message.signingKey) {
    attachments.push(new File(
      [message.signingKey.toPublic().armor()],
      'openpgp-key.asc',
      { type: 'application/pgp-keys' },
    ));
  }
  if (attachments.length === 0) {
    return `Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n${message.body.replace(/\r?\n/g, '\r\n')}\r\n`;
  }

  const partBoundary = boundary('mixed');
  let entity = `Content-Type: multipart/mixed; boundary="${partBoundary}"\r\n\r\n`;
  entity += `--${partBoundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n${message.body.replace(/\r?\n/g, '\r\n')}\r\n`;
  for (const attachment of attachments) {
    const filename = safeHeader(attachment.name).replace(/"/g, '');
    const contentType = safeHeader(attachment.type || 'application/octet-stream');
    const content = new Uint8Array(await attachment.arrayBuffer());
    entity += `--${partBoundary}\r\nContent-Type: ${contentType}; name="${filename}"\r\n`;
    entity += `Content-Disposition: attachment; filename="${filename}"\r\nContent-Transfer-Encoding: base64\r\n\r\n`;
    entity += `${base64Lines(content)}\r\n`;
  }
  entity += `--${partBoundary}--\r\n`;
  return entity;
}

export async function buildProtectedMime(message: ProtectedComposeMessage): Promise<Uint8Array> {
  const headers = messageHeaders(message);
  const inner = await innerEntity(message);

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

  if (!message.sign || !message.signingKey) throw new Error('protected MIME needs sign or encrypt');
  const signedMessage = await openpgp.createMessage({ binary: encoder.encode(inner) });
  const signature = await openpgp.sign({
    message: signedMessage,
    signingKeys: message.signingKey,
    detached: true,
    format: 'armored',
  });
  const signedBoundary = boundary('signed');
  const body =
    `Content-Type: multipart/signed; protocol="application/pgp-signature"; micalg=pgp-sha256; boundary="${signedBoundary}"\r\n\r\n` +
    `--${signedBoundary}\r\n${inner}` +
    `--${signedBoundary}\r\nContent-Type: application/pgp-signature; name="signature.asc"\r\nContent-Disposition: attachment; filename="signature.asc"\r\n\r\n${signature}\r\n` +
    `--${signedBoundary}--\r\n`;
  return encoder.encode(headers + body);
}

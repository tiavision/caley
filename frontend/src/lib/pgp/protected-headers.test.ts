// @vitest-environment node
// Integration cases use real openpgp crypto, which needs node (see sign-verify.test.ts).
import { describe, expect, it } from 'vitest';
import * as openpgp from 'openpgp';
import { decryptMessage, parseMimeMessage } from './keyring';

function mime(lines: string[]): Uint8Array {
  return new TextEncoder().encode(lines.join('\r\n'));
}

describe('protected header subject recovery (parsing)', () => {
  it('reads a Subject header off the top-level entity (protected-headers marker)', () => {
    const raw = mime([
      'Content-Type: multipart/mixed; boundary="b1"; protected-headers="v1"',
      'Subject: The real subject',
      'From: alice@example.com',
      '',
      '--b1',
      'Content-Type: text/plain; charset=utf-8',
      '',
      'Body text',
      '--b1--',
      '',
    ]);
    expect(parseMimeMessage(raw).subject).toBe('The real subject');
  });

  it('reads the Subject even without the protected-headers marker', () => {
    const raw = mime([
      'Content-Type: text/plain; charset=utf-8',
      'Subject: Legacy protected subject',
      '',
      'Body text',
    ]);
    expect(parseMimeMessage(raw).subject).toBe('Legacy protected subject');
  });

  it('decodes an RFC 2047 encoded-word subject', () => {
    const raw = mime([
      'Content-Type: text/plain; charset=utf-8',
      'Subject: =?UTF-8?Q?Gr=C3=BC=C3=9Fe?=',
      '',
      'Body text',
    ]);
    expect(parseMimeMessage(raw).subject).toBe('Grüße');
  });

  it('falls back to a legacy text/rfc822-headers part', () => {
    const raw = mime([
      'Content-Type: multipart/mixed; boundary="b2"',
      '',
      '--b2',
      'Content-Type: text/rfc822-headers; protected-headers="v1"',
      '',
      'Subject: Memory Hole subject',
      'From: alice@example.com',
      '',
      '--b2',
      'Content-Type: text/plain; charset=utf-8',
      '',
      'Body text',
      '--b2--',
      '',
    ]);
    expect(parseMimeMessage(raw).subject).toBe('Memory Hole subject');
  });

  it('returns null when no inner subject is present', () => {
    const raw = mime([
      'Content-Type: text/plain; charset=utf-8',
      '',
      'Body text',
    ]);
    expect(parseMimeMessage(raw).subject).toBeNull();
  });
});

describe('protected header subject recovery (decryption)', () => {
  it('recovers the protected subject from an encrypted payload', async () => {
    const { privateKey, publicKey } = await openpgp.generateKey({
      userIDs: [{ name: 'Tester', email: 'tester@example.com' }],
      format: 'object',
    });

    const innerMime = [
      'Content-Type: multipart/mixed; boundary="inner"; protected-headers="v1"',
      'Subject: The real secret subject',
      'From: sender@example.com',
      '',
      '--inner',
      'Content-Type: text/plain; charset=utf-8',
      '',
      'Encrypted body',
      '--inner--',
      '',
    ].join('\r\n');

    const ciphertext = await openpgp.encrypt({
      message: await openpgp.createMessage({ text: innerMime }),
      encryptionKeys: publicKey,
    });

    const raw = new TextEncoder().encode([
      'Subject: ...',
      'Content-Type: multipart/encrypted; protocol="application/pgp-encrypted"',
      '',
      String(ciphertext),
    ].join('\r\n'));

    const result = await decryptMessage([privateKey], raw, []);
    expect(result.badges).toContain('encrypted');
    expect(result.subject).toBe('The real secret subject');
    expect(result.text).toBe('Encrypted body');
  });
});

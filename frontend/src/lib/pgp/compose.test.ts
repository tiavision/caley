import { describe, expect, it, vi } from 'vitest';
import { buildProtectedMime } from './compose';

vi.mock('openpgp', () => ({}));

const baseMessage = {
  identity: { name: 'Müller Mail', email: 'sender@example.com' },
  to: ['recipient@example.com'],
  cc: [],
  inReplyTo: null,
  references: null,
  encrypt: false,
  sign: false,
  encryptionKeys: [],
  signingKey: null,
};

async function raw(message: Parameters<typeof buildProtectedMime>[0]): Promise<string> {
  return new TextDecoder().decode(await buildProtectedMime(message));
}

describe('buildProtectedMime', () => {
  it('encodes non-ASCII headers and text body as ASCII MIME', async () => {
    const mime = await raw({
      ...baseMessage,
      subject: 'Fwd: Grüße vom Büro',
      body: 'Hallo Müller,\nSchöne Grüße',
      attachments: [],
    });

    expect(mime).toMatch(/^From: =\?UTF-8\?B\?/m);
    expect(mime).toContain('Subject: =?UTF-8?B?');
    expect(mime).toContain('Content-Transfer-Encoding: quoted-printable');
    expect(mime).toContain('Gr=C3=BC=C3=9Fe');
    expect(/[\u0080-\uffff]/.test(mime)).toBe(false);
  });

  it('punycodes internationalized recipient domains in address headers', async () => {
    const mime = await raw({
      ...baseMessage,
      to: ['person@bücher.example'],
      subject: 'Hello',
      body: 'Body',
      attachments: [],
    });

    expect(mime).toContain('To: person@xn--bcher-kva.example');
    expect(/[\u0080-\uffff]/.test(mime)).toBe(false);
  });

  it('encodes non-ASCII attachment filenames as RFC 2231 parameters', async () => {
    const mime = await raw({
      ...baseMessage,
      subject: 'Attachment',
      body: 'See attached',
      attachments: [new File(['content'], 'büro.txt', { type: 'text/plain' })],
    });

    expect(mime).toContain("name*=UTF-8''b%C3%BCro.txt");
    expect(mime).toContain("filename*=UTF-8''b%C3%BCro.txt");
    expect(/[\u0080-\uffff]/.test(mime)).toBe(false);
  });
});

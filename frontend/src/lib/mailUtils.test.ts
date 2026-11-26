import { describe, expect, it } from 'vitest';
import { canPreviewContentType, senderName } from './mailUtils.ts';

describe('senderName', () => {
  it('drops the quotes of a quoted display name', () => {
    expect(senderName('"CI/CD" <ci@ops.example>')).toBe('CI/CD');
    expect(senderName('"Doe, Jane" <jane@example.com>')).toBe('Doe, Jane');
  });

  it('undoes backslash escapes inside the quotes', () => {
    expect(senderName('"Jane \\"JD\\" Doe" <jane@example.com>')).toBe('Jane "JD" Doe');
  });

  it('keeps unquoted names and bare addresses', () => {
    expect(senderName('Jane Doe <jane@example.com>')).toBe('Jane Doe');
    expect(senderName('jane@example.com')).toBe('jane@example.com');
    expect(senderName(null)).toBe('');
  });

  it('falls back to the address for an empty quoted name', () => {
    expect(senderName('"" <jane@example.com>')).toBe('jane@example.com');
  });
});

describe('canPreviewContentType', () => {
  it('previews the types the server shows, by essence', () => {
    for (const type of ['application/pdf', 'APPLICATION/PDF; version=1.7', ' text/plain; charset=utf-8', 'image/png',
      'image/avif', 'audio/mpeg', 'Video/MP4', 'video/quicktime']) {
      expect(canPreviewContentType(type), type).toBe(true);
    }
  });

  it('downloads anything else', () => {
    for (const type of ['audio/mpeg,text/html', 'video/mp4 , text/html', 'text/plain,text/html', 'audio/x\ttext/html',
      'text/html', 'image/svg+xml', 'application/javascript', 'text/javascript', 'text/css', 'audio/x-unknown',
      'audio/', 'video/', 'application/octet-stream', '', null, undefined]) {
      expect(canPreviewContentType(type), String(type)).toBe(false);
    }
  });
});

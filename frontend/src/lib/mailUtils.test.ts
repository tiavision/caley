import { describe, expect, it } from 'vitest';
import { bareMailbox, canPreviewContentType, senderName } from './mailUtils.ts';
import { formatAddress, isPlainAddress, parseEnvelopeAddresses, splitAddressList } from './addresses.ts';

describe('bareMailbox, parseEnvelopeAddresses', () => {
  it('take the last angle-address: a name comes first and may look like one', () => {
    expect(bareMailbox('Ann <Ann@X.example>')).toBe('ann@x.example');
    expect(bareMailbox('<ceo@bank.example> <attacker@evil.example>')).toBe('attacker@evil.example');
    expect(bareMailbox('"<ceo@bank.example>" <attacker@evil.example>')).toBe('attacker@evil.example');
    expect(bareMailbox('ann@x.example')).toBe('ann@x.example');
    expect(parseEnvelopeAddresses('<ceo@bank.example> <attacker@evil.example>, Bob <bob@x>')).toEqual(['attacker@evil.example', 'bob@x']);
  });
});

describe('address lists (RFC 5322 commas)', () => {
  it('keeps a quoted name with a comma as one recipient', () => {
    expect(parseEnvelopeAddresses('"Doe, Jane" <jane@x.example>')).toEqual(['jane@x.example']);
    expect(parseEnvelopeAddresses('"Doe, Jane" <jane@x.example>, Bob <bob@x>, c@x')).toEqual(['jane@x.example', 'bob@x', 'c@x']);
  });
  it('reads escaped quotes inside a quoted name, and a comma or angle-address after them', () => {
    expect(parseEnvelopeAddresses(String.raw`"Jane \"JD, Esq.\" <ceo@bank>" <jane@x>, b@y`)).toEqual(['jane@x', 'b@y']);
    expect(splitAddressList(String.raw`"a\\" <a@x>, b@y`)).toEqual([String.raw`"a\\" <a@x>`, 'b@y']);
  });
  it('leaves out empty entries (a trailing comma, doubled commas)', () => {
    expect(parseEnvelopeAddresses('a@x, ')).toEqual(['a@x']);
    expect(parseEnvelopeAddresses('a@x,, b@y,')).toEqual(['a@x', 'b@y']);
    expect(parseEnvelopeAddresses('')).toEqual([]);
  });
  it('does not split at a comma inside an angle-address', () => {
    expect(splitAddressList('Ann <a,b@x>, c@y')).toEqual(['Ann <a,b@x>', 'c@y']);
  });
  it('formatAddress quotes a name as the backend does; isPlainAddress accepts only local@domain', () => {
    expect(formatAddress('Jane Doe', 'j@x')).toBe('Jane Doe <j@x>');
    expect(formatAddress('Doe, Jane', 'j@x')).toBe('"Doe, Jane" <j@x>');
    expect(formatAddress('evil@evil.example, Jane <x>', 'j@x')).toBe('"evil@evil.example, Jane x" <j@x>');
    expect(formatAddress(String.raw`Jane "JD" \ Doe`, 'j@x')).toBe(String.raw`"Jane \"JD\" \\ Doe" <j@x>`);
    expect(formatAddress('', 'j@x')).toBe('j@x');
    expect(isPlainAddress('jane@corp.example')).toBe(true);
    for (const bad of ['jane', 'a@b@c', 'a@b, c@d', 'Jane <j@x>', '"a"@b', 'a b@c', 'a@b;c']) expect(isPlainAddress(bad), bad).toBe(false);
  });
});

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

  it('shows a name that holds an @ with the real address', () => {
    expect(senderName('"ceo@bank.com" <evil@x.example>')).toBe('ceo@bank.com <evil@x.example>');
    expect(senderName('ceo@bank.com <evil@x.example>')).toBe('ceo@bank.com <evil@x.example>');
    expect(senderName('"Support (help@bank.com)" <evil@x.example>')).toBe('Support (help@bank.com) <evil@x.example>');
    expect(senderName('"" <jane@example.com>')).toBe('jane@example.com');
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

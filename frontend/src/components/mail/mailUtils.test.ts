import { describe, expect, it } from 'vitest';
import { senderName } from './mailUtils.ts';

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

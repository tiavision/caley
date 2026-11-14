import { expect, test } from 'vitest';
import { pagePath, withoutLocale } from './locale-path.ts';

test('the language prefix of a path', () => {
  expect(withoutLocale('/de/mail/')).toBe('/mail/');
  expect(withoutLocale('/pt-BR/mail/')).toBe('/mail/');
  expect(withoutLocale('/de/')).toBe('/');
  expect(withoutLocale('/mail/')).toBe('/mail/');
});

test('a URL that already has a language keeps it', () => {
  expect(pagePath('/en/mail/?uid=3')).toBe('/en/mail/?uid=3');
});

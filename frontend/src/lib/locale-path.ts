// Page URLs in this page's language. Each page is built once per language under its prefix (`localePrefix`:
// `/de/mail/`); the build writes links in the markup in the language, but not URLs code puts together. An
// unprefixed URL still works (the server sends it on to the visitor's language), at the cost of a redirect.

const PREFIX = /^\/[a-z]{2}(?:-[A-Za-z]+)?(?=\/)/;

/** This page's language prefix (`/de`), or '' outside a language (tests, the Vite dev server's unprefixed pages). */
export function localePrefix(): string {
  return typeof location === 'undefined' ? '' : (location.pathname.match(PREFIX)?.[0] ?? '');
}

/** `path` (`/mail/?uid=3`) in this page's language; one that already has a language is left alone. */
export function pagePath(path: string): string {
  return PREFIX.test(path) ? path : localePrefix() + path;
}

/** `pathname` without its language prefix. */
export function withoutLocale(pathname: string): string {
  return pathname.replace(PREFIX, '');
}

/**
 * This page's URL in language `locale` (`/de/settings/`), for test renders only: they compile without a language,
 * so the built-in `localeUrl()` the build writes into each page is not there (tests pass this in `helpers`).
 */
export function localeUrl(locale: string): string {
  return `/${locale}${withoutLocale(location.pathname)}`;
}

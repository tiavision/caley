// Date formatters, built once. `toLocaleDateString(locale, options)` and `new Intl.DateTimeFormat(…)` construct a
// formatter on every call, which costs more than the formatting itself: a list row or a day cell that formats a
// date did so on every render. Pure and light: the helpers (and through them the pages' early scripts) import this.

const formatters = new Map<string, Intl.DateTimeFormat>();

/**
 * The formatter for `locale` and `options`, kept for the page's life. `locale` undefined is the browser's own
 * language; the key then names `navigator.language`, so a browser whose language changes while the page is open gets
 * a new formatter. A formatter without a `timeZone` option keeps the zone the browser had when it was made (see
 * `browserTimeZone`).
 */
export function dateFormat(locale: string | undefined, options: Intl.DateTimeFormatOptions = {}): Intl.DateTimeFormat {
  const key = `${locale ?? `\0${typeof navigator === 'undefined' ? '' : navigator.language}`}\0${JSON.stringify(options)}`;
  let format = formatters.get(key);
  if (!format) formatters.set(key, format = new Intl.DateTimeFormat(locale, options));
  return format;
}

let zone: string | undefined;

/**
 * The browser's IANA time zone ('' when it cannot say), read once: asking builds a formatter each time. It stays
 * what it was when the page first asked: a device that changes zone while a page is open shows the new zone after
 * the next page load (a navigation inside the app keeps the document, and the zone with it).
 */
export function browserTimeZone(): string {
  if (zone === undefined) {
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch { zone = ''; }
  }
  return zone;
}

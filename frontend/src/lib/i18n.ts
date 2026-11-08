import { setLocale as paraglideSetLocale, getLocale } from '../paraglide/runtime.js';
import { m } from '../paraglide/messages.js';

export { getLocale };

export function makeTl(locale: string): (key: string, params?: Record<string, string | number>) => string {
  void locale;
  return (key, params) => {
    const fn = (m as Record<string, Function>)[key];
    return typeof fn === 'function' ? fn(params) : key;
  };
}

// sinter's first-paint script has picked the locale before the page showed (the `locale` option in
// vite.config.ts: localStorage `locale`, else the browser's languages, else English) and set <html lang>.
export function initI18n() {
  paraglideSetLocale(document.documentElement.lang.slice(0, 2) || 'en');
}

export function setLocale(locale: string) {
  const lang = locale.slice(0, 2);
  paraglideSetLocale(lang);
  localStorage.setItem('locale', lang);
  document.documentElement.lang = lang;
  document.body.dispatchEvent(new CustomEvent('localeChange', { detail: { locale: lang } }));
}

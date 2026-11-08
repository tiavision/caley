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

// The language is picked in one place, src/lib/language.ts (run before the first paint by src/sinter.early.ts).
export { initLanguage as initI18n } from './language.ts';

export function setLocale(locale: string) {
  const lang = locale.slice(0, 2);
  paraglideSetLocale(lang);
  localStorage.setItem('locale', lang);
  document.documentElement.lang = lang;
  document.body.dispatchEvent(new CustomEvent('localeChange', { detail: { locale: lang } }));
}

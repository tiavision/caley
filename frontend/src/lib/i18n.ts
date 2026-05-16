import { setLocale as paraglideSetLocale, getLocale } from '../paraglide/runtime.js';

export { getLocale };

export function initI18n() {
  const stored = localStorage.getItem('locale');
  const browserLang = navigator.language || 'en';
  const locale = (stored || browserLang).slice(0, 2);
  paraglideSetLocale(locale);
  document.documentElement.lang = locale;
}

export function setLocale(locale: string) {
  const lang = locale.slice(0, 2);
  paraglideSetLocale(lang);
  localStorage.setItem('locale', lang);
  document.documentElement.lang = lang;
  document.body.dispatchEvent(new CustomEvent('localeChange', { detail: { locale: lang } }));
}

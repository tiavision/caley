// The interface language, picked once per page before anything renders (src/sinter.early.ts runs it in the
// page's early chunk): the stored choice (localStorage `locale`, set in Settings), else the browser's, else English.
// Kept apart from i18n.ts so the early chunk loads paraglide's runtime only, not every message.
import { baseLocale, isLocale, setLocale } from '../paraglide/runtime.js';

export function pickLanguage(): string {
  let stored: string | null = null;
  try { stored = localStorage.getItem('locale'); } catch { /* storage blocked */ }
  const lang = (stored || navigator.language || baseLocale).slice(0, 2);
  return isLocale(lang) ? lang : baseLocale;
}

/** Sets paraglide's locale without the reload paraglide would do on a first visit, and <html lang>. */
export function initLanguage(): void {
  const lang = pickLanguage();
  setLocale(lang, { reload: false });
  document.documentElement.lang = lang;
}

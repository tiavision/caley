import en from './en.json';
import de from './de.json';

type Translations = Record<string, string>;
type LocaleMap = Record<string, Translations>;

const locales: LocaleMap = { en, de };

let currentLocale: string = 'en';
let currentLang: string = 'en';
let translations: Translations = en;

export function t(key: string, params?: Record<string, string | number>): string {
  let val = translations[key];
  if (val === undefined) {
    val = (locales.en as Translations)[key];
    if (val === undefined) return key;
  }
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      val = val.replace(`{${k}}`, String(v));
    }
  }
  return val;
}

export function getLocale(): string {
  return currentLocale;
}

export function getLang(): string {
  return currentLang;
}

function loadLocale(locale: string): Translations {
  const lang = locale.slice(0, 2);
  return locales[lang] || locales[lang] === undefined ? (locales as LocaleMap)[lang] || en : en;
}

function applyTranslations(root?: HTMLElement) {
  const ctx = root || document;

  const i18nEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-i18n]')
    : [];
  for (const el of i18nEls) {
    const key = el.getAttribute('data-i18n');
    if (key) {
      el.textContent = t(key);
    }
  }

  const attrEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-i18n-attr]')
    : [];
  for (const el of attrEls) {
    const spec = el.getAttribute('data-i18n-attr');
    if (spec) {
      const colon = spec.indexOf(':');
      if (colon !== -1) {
        const attr = spec.slice(0, colon);
        const key = spec.slice(colon + 1);
        el.setAttribute(attr, t(key));
      }
    }
  }

  const dayShortEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-i18n-day-short]')
    : [];
  for (const el of dayShortEls) {
    const dayNum = parseInt(el.getAttribute('data-i18n-day-short') || '0', 10);
    if (dayNum >= 1 && dayNum <= 7) {
      const date = new Date(2024, 0, dayNum);
      el.textContent = date.toLocaleDateString(currentLocale, { weekday: 'short' });
    }
  }

  const monthEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-i18n-month]')
    : [];
  for (const el of monthEls) {
    const m = parseInt(el.getAttribute('data-i18n-month') || '0', 10);
    const y = parseInt(el.getAttribute('data-i18n-year') || '0', 10);
    if (m >= 1 && m <= 12 && y > 0) {
      const date = new Date(y, m - 1, 1);
      el.textContent = date.toLocaleDateString(currentLocale, { month: 'long', year: 'numeric' });
    }
  }

  const rawDateEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-raw-date]')
    : [];
  for (const el of rawDateEls) {
    const iso = el.getAttribute('data-raw-date');
    if (iso) {
      const parts = iso.split('-');
      const d = new Date(+parts[0], +parts[1] - 1, +parts[2]);
      if (!isNaN(d.getTime())) {
        el.textContent = d.toLocaleDateString(currentLocale, { weekday: 'long' });
      }
    }
  }

  const rawDateAttrEls = (ctx as HTMLElement).querySelectorAll
    ? (ctx as HTMLElement).querySelectorAll('[data-raw-date-aria]')
    : [];
  for (const el of rawDateAttrEls) {
    const iso = el.getAttribute('data-raw-date-aria');
    if (iso) {
      const parts = iso.split('-');
      const d = new Date(+parts[0], +parts[1] - 1, +parts[2]);
      if (!isNaN(d.getTime())) {
        el.setAttribute('aria-label', d.toLocaleDateString(currentLocale, { weekday: 'long' }));
      }
    }
  }
}

function onDomReady(fn: () => void) {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', fn);
  } else {
    fn();
  }
}

function setLangAttr() {
  document.documentElement.lang = currentLang;
}

export async function initI18n() {
  const stored = localStorage.getItem('locale');
  const browserLang = navigator.language || 'en';
  currentLocale = stored || browserLang;
  currentLang = currentLocale.slice(0, 2);
  translations = loadLocale(currentLocale);

  onDomReady(() => {
    setLangAttr();
    applyTranslations();
  });
}

export function setLocale(locale: string) {
  currentLocale = locale;
  currentLang = locale.slice(0, 2);
  translations = loadLocale(locale);
  localStorage.setItem('locale', locale);
  setLangAttr();
  applyTranslations();
  document.body.dispatchEvent(new CustomEvent('localeChange', { detail: { locale } }));
}

export function createLocalePicker(): HTMLSelectElement {
  const select = document.createElement('select');
  select.className = 'locale-picker';
  select.setAttribute('aria-label', t('settings.language'));

  const opts = [
    { value: 'en', label: 'English' },
    { value: 'de', label: 'Deutsch' },
  ];

  for (const o of opts) {
    const opt = document.createElement('option');
    opt.value = o.value;
    opt.textContent = o.label;
    if (currentLang === o.value) opt.selected = true;
    select.appendChild(opt);
  }

  select.addEventListener('change', () => {
    setLocale(select.value);
  });

  return select;
}

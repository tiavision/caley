const RETURN_URL_KEY = 'caley.returnUrl';
const AUTH_RETURN_PENDING_KEY = 'caley.authReturnPending';

const RESTORABLE_PATH_PREFIXES = [
  '/calendar/',
  '/mail/',
  '/contacts/',
  '/search/',
  '/settings/',
];

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function removeStored(s: Storage, key: string) {
  try {
    s.removeItem(key);
  } catch {}
}

function isRestorablePath(pathname: string): boolean {
  if (pathname === '/' || pathname.startsWith('/auth/') || pathname === '/logout') return false;
  if (pathname.startsWith('/assets/')) return false;
  if (pathname.split('/').pop()?.includes('.')) return false;
  return RESTORABLE_PATH_PREFIXES.some(prefix => pathname.startsWith(prefix));
}

export function getReturnUrl(): string | null {
  const s = storage();
  if (!s) return null;

  const stored = s.getItem(RETURN_URL_KEY);
  if (!stored) return null;

  try {
    const url = new URL(stored, window.location.origin);
    if (url.origin !== window.location.origin || !isRestorablePath(url.pathname)) {
      removeStored(s, RETURN_URL_KEY);
      return null;
    }
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    removeStored(s, RETURN_URL_KEY);
    return null;
  }
}

export function rememberCurrentAppUrl() {
  const s = storage();
  if (!s || !isRestorablePath(window.location.pathname)) return;
  try {
    s.setItem(RETURN_URL_KEY, `${window.location.pathname}${window.location.search}${window.location.hash}`);
  } catch {}
}

export function clearReturnUrl() {
  const s = storage();
  if (!s) return;
  removeStored(s, RETURN_URL_KEY);
  removeStored(s, AUTH_RETURN_PENDING_KEY);
}

export function markAuthReturnPending() {
  const s = storage();
  if (!s) return;
  try {
    s.setItem(AUTH_RETURN_PENDING_KEY, '1');
  } catch {}
}

export function consumeAuthReturnPending(): boolean {
  const s = storage();
  if (!s) return false;

  const pending = s.getItem(AUTH_RETURN_PENDING_KEY) === '1';
  removeStored(s, AUTH_RETURN_PENDING_KEY);
  return pending;
}

export function redirectToReturnUrl(fallback = '/calendar/') {
  const target = getReturnUrl() ?? fallback;
  window.location.href = target;
}

export function redirectToPendingReturnUrl(fallback = '/calendar/') {
  if (!consumeAuthReturnPending()) return false;

  const target = getReturnUrl() ?? fallback;
  if (target === `${window.location.pathname}${window.location.search}${window.location.hash}`) return false;

  window.location.href = target;
  return true;
}

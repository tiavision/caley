// openpgp (about 135 kB gzip) is loaded on first use, not with the page: the mail and settings pages render
// without it, and a plain message never needs it. preloadPgp() fetches it once the page is idle, so the first
// encrypted or signed message (or an Autocrypt header) does not wait for the download.
import { m } from '@tiaforge-ui/sinter/messages';

export type Pgp = typeof import('./modules.ts');

/**
 * A module loaded on first use, once. A failed load (offline, or a tab open across a deploy whose old chunk is
 * gone) rejects with a message that says to reload, and the next call tries again.
 */
export function lazyModule<T>(load: () => Promise<T>): () => Promise<T> {
  let loading: Promise<T> | null = null;
  return () => loading ??= load().catch((e: unknown) => {
    loading = null;
    console.error('[pgp] loading openpgp failed', e);
    throw new Error(m.pgp_loadError());
  });
}

/** The openpgp modules (./modules.ts). */
export const loadPgp: () => Promise<Pgp> = lazyModule(() => import('./modules.ts'));

/**
 * Starts loading openpgp once the page has loaded and the browser is idle (at most 5 s later), so it does not
 * compete with the page's own files and first requests; errors are left to the real use.
 */
export function preloadPgp(): void {
  const start = () => { loadPgp().catch(() => {}); };
  const idle = () => {
    if ('requestIdleCallback' in window) requestIdleCallback(start, { timeout: 5000 });
    else setTimeout(start, 1000);
  };
  if (document.readyState === 'complete') idle();
  else window.addEventListener('load', idle, { once: true });
}

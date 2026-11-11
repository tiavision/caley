// openpgp (about 135 kB gzip) is loaded on first use, not with the page: the mail and settings pages render
// without it, and a plain message never needs it. On the mail pages <pgp-preload> (src/components/pgp-preload.ts,
// inside <mail-reader>) fetches it once the page is idle, so the first encrypted or signed message (or an Autocrypt
// header) does not wait for the download.
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


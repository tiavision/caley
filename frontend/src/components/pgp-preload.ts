// The script of <pgp-preload>: starts loading openpgp (about 135 kB gzip) when the browser is next idle (at most 5 s
// later). Loading happens once per page; a failure is left to the real use, which tries again.
import { loadPgp } from '../lib/pgp/load.ts';
import { PgpPreloadComponent } from './pgp-preload.html';

export class PgpPreload extends PgpPreloadComponent {
  connected() {
    const start = () => void loadPgp().catch(() => {});
    if ('requestIdleCallback' in window) requestIdleCallback(start, { timeout: 5000 });
    else setTimeout(start, 1000);
  }
}

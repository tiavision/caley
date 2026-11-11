// The script of <pgp-preload>: starts loading openpgp (about 135 kB gzip) as soon as the page has taken over. By then
// the page's data is in or on its way (measured on a slow connection with a 4x slower CPU: the list and the message
// show as fast as without it, and openpgp is ready earlier than when waiting for idle time). It loads once per page;
// a failure is left to the real use, which tries again.
import { loadPgp } from '../lib/pgp/load.ts';
import { PgpPreloadComponent } from './pgp-preload.html';

export class PgpPreload extends PgpPreloadComponent {
  connected() {
    void loadPgp().catch(() => {});
  }
}

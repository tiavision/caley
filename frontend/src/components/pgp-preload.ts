// The script of <pgp-preload>: starts loading openpgp (about 135 kB gzip) after the page's load event, when the browser
// is idle (at most 5 s later), so it does not compete with the page's own files and first requests. Errors are left
// to the real use, which tries again.
import { loadPgp } from '../lib/pgp/load.ts';
import { PgpPreloadComponent } from './pgp-preload.html';

export class PgpPreload extends PgpPreloadComponent {
  #idle = 0;
  #onLoad = () => this.#whenIdle();

  connected() {
    if (document.readyState === 'complete') this.#whenIdle();
    else window.addEventListener('load', this.#onLoad, { once: true });
  }

  disconnected() {
    window.removeEventListener('load', this.#onLoad);
    if ('cancelIdleCallback' in window) cancelIdleCallback(this.#idle);
    else clearTimeout(this.#idle);
  }

  #whenIdle() {
    const start = () => { loadPgp().catch(() => {}); };
    this.#idle = 'requestIdleCallback' in window ? requestIdleCallback(start, { timeout: 5000 }) : window.setTimeout(start, 1000);
  }
}

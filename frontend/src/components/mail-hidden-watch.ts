// The script of <mail-hidden-watch>: the page's visibility, as an event.
import { MailHiddenWatchComponent } from './mail-hidden-watch.html';

export class MailHiddenWatch extends MailHiddenWatchComponent {
  #fire = () => { if (document.hidden) this.emit('page-hidden'); };
  #leave = () => this.emit('page-hidden');

  connected() {
    document.addEventListener('visibilitychange', this.#fire);
    window.addEventListener('pagehide', this.#leave);
  }

  disconnected() {
    document.removeEventListener('visibilitychange', this.#fire);
    window.removeEventListener('pagehide', this.#leave);
  }
}

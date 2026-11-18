// The script of <email-hidden-watch>: the page's visibility, as an event.
import { EmailHiddenWatchComponent } from './email-hidden-watch.html';

export class EmailHiddenWatch extends EmailHiddenWatchComponent {
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

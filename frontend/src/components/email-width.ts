// The script of <email-width>: whether the reader is narrow (a phone), as an event when it changes.
import { EmailWidthComponent } from './email-width.html';

/** below this width the mail's @media rules are evaluated for a phone: mails write their mobile layout for max-width 480–640px */
const NARROW = 640;

export class EmailWidth extends EmailWidthComponent {
  #narrow: boolean | null = null;
  #observer = new ResizeObserver(() => this.#measure());

  connected() {
    this.#observer.observe(this.root);
    this.#measure();
  }

  disconnected() {
    this.#observer.disconnect();
  }

  #measure() {
    const width = this.root.clientWidth;
    // not laid out (a hidden reader): nothing to say
    if (width === 0) return;
    const narrow = width < NARROW;
    if (narrow === this.#narrow) return;
    this.#narrow = narrow;
    this.emit('narrow', narrow);
  }
}

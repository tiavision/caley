// The script of <close-after>: a timer that closes the surrounding dialog.
import { CloseAfterComponent } from './close-after.html';

export class CloseAfter extends CloseAfterComponent {
  #timer: ReturnType<typeof setTimeout> | undefined;

  connected() {
    this.#timer = setTimeout(() => this.root.closest('dialog')?.close(), this.props.ms);
  }

  disconnected() {
    clearTimeout(this.#timer);
  }
}

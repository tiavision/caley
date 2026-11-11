// The script of <mail-split>: the pointer drag of a resize handle.
import { MailSplitComponent } from './mail-split.html';

export class MailSplit extends MailSplitComponent {
  connected() {
    this.root.addEventListener('pointerdown', this.#down);
  }

  disconnected() {
    this.root.removeEventListener('pointerdown', this.#down);
  }

  #down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const handle = this.root;
    const startX = e.clientX;
    const start = this.props.width;
    handle.setPointerCapture(e.pointerId);
    this.state.dragging = true;
    const move = (ev: PointerEvent) => this.emit('split-resize', start + ev.clientX - startX);
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      this.state.dragging = false;
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  };
}

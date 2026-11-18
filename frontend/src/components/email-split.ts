// The script of <email-split>: the pointer drag of a resize handle.
import { EmailSplitComponent } from './email-split.html';

export class EmailSplit extends EmailSplitComponent {
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

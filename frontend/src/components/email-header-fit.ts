// The script of <email-header-fit>: measures whether the subject and the controls fit on one line.
// Measuring forces a layout (the intrinsic widths are read from clones), so it happens once per frame at most and
// never while the reader is still being rendered: a change inside the row (the subject, a control that comes or
// goes; a message's render makes many) asks for one measurement in the next frame, before it is painted, and a
// resize measures in the ResizeObserver's callback, when the layout is at hand anyway.
import { EmailHeaderFitComponent } from './email-header-fit.html';

export class EmailHeaderFit extends EmailHeaderFitComponent {
  #observer = new ResizeObserver(() => { if (this.root.clientWidth !== this.#width) this.#measure(); });
  #mutations = new MutationObserver(() => this.#schedule());
  #frame = 0;
  /** The row's width at the last measurement: a resize that leaves it (the row growing taller) changes nothing. */
  #width = -1;

  connected() {
    this.#observer.observe(this.root);
    this.#mutations.observe(this.root, { subtree: true, childList: true, characterData: true });
    this.#schedule();
  }

  disconnected() {
    this.#observer.disconnect();
    this.#mutations.disconnect();
    cancelAnimationFrame(this.#frame);
    this.#frame = 0;
  }

  #schedule() {
    this.#frame ||= requestAnimationFrame(() => this.#measure());
  }

  #measure() {
    cancelAnimationFrame(this.#frame);
    this.#frame = 0;
    const row = this.root;
    const line = row.querySelector<HTMLElement>('.detail-subject-line');
    const controls = row.querySelector<HTMLElement>('.detail-controls');
    if (!line || !controls) return;
    const style = getComputedStyle(row);
    const gap = parseFloat(style.columnGap || style.gap) || 0;
    this.#width = row.clientWidth;
    // Both clones in, then both read, then both out: one layout for the two, not one each.
    const clones = [line, controls].map(intrinsicClone);
    document.body.append(...clones);
    const stacked = clones[0]!.getBoundingClientRect().width + clones[1]!.getBoundingClientRect().width + gap > this.#width;
    for (const clone of clones) clone.remove();
    if (stacked !== this.state.stacked) this.state.stacked = stacked;
  }
}

/** A copy of `element` that takes the width its content wants, out of sight. */
function intrinsicClone(element: HTMLElement): HTMLElement {
  const clone = element.cloneNode(true) as HTMLElement;
  // display: a clone that were `display: contents` would have no box, and its children would be laid out in <body>.
  Object.assign(clone.style, { display: 'flex', position: 'fixed', left: '-10000px', top: '0', width: 'max-content', maxWidth: 'none', flex: 'none', whiteSpace: 'nowrap', visibility: 'hidden', pointerEvents: 'none' });
  return clone;
}

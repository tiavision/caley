// The script of <mail-header-fit>: measures whether the subject and the controls fit on one line.
import { MailHeaderFitComponent } from './mail-header-fit.html';

export class MailHeaderFit extends MailHeaderFitComponent {
  #observer = new ResizeObserver(() => this.#measure());
  #mutations = new MutationObserver(() => this.#measure());

  connected() {
    this.#observer.observe(this.root);
    this.#mutations.observe(this.root, { subtree: true, childList: true, characterData: true });
    requestAnimationFrame(() => this.#measure());
  }

  disconnected() {
    this.#observer.disconnect();
    this.#mutations.disconnect();
  }

  #measure() {
    const row = this.root;
    const line = row.querySelector<HTMLElement>('.detail-subject-line');
    const controls = row.querySelector<HTMLElement>('.detail-controls');
    if (!line || !controls) return;
    const style = getComputedStyle(row);
    const gap = parseFloat(style.columnGap || style.gap) || 0;
    const stacked = intrinsicWidth(line) + intrinsicWidth(controls) + gap > row.clientWidth;
    if (stacked !== this.state.stacked) this.state.stacked = stacked;
  }
}

function intrinsicWidth(element: HTMLElement): number {
  const clone = element.cloneNode(true) as HTMLElement;
  Object.assign(clone.style, { position: 'fixed', left: '-10000px', top: '0', width: 'max-content', maxWidth: 'none', flex: 'none', whiteSpace: 'nowrap', visibility: 'hidden', pointerEvents: 'none' });
  document.body.appendChild(clone);
  const width = clone.getBoundingClientRect().width;
  clone.remove();
  return width;
}

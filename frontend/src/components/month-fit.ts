// The script of <month-fit>: measures the month grid's cells (layout work markup cannot do).
import type { DayEventsJson } from '../types/calendar.ts';
import { eventKey } from '../lib/events.ts';
import { MonthFitComponent } from './month-fit.html';

/** What <month-fit> reports in its `fit` event. */
export interface MonthFit {
  /** per date shown in the grid: how many event chips fit; dates where all fit are missing */
  counts: Record<string, number>;
  /** below 60rem: a tap on a cell opens the day's list instead of an event */
  narrow: boolean;
}

const MORE_H = 19; // px: the "+N" row's height until one has been measured
const MAX_PASSES = 8; // corrections after the first measurement

/**
 * After each change of the events and each resize it reports how many chips fit each cell. It first reports no limits, so every chip renders, estimates the counts from the chips'
 * heights, then checks the result: a cell whose content still overflows (the "+N" row taller than estimated,
 * rounding, a chip that wrapped) loses one chip more, until none overflows. A reload with the same events
 * (a live update) keeps the counts, so the grid does not flash.
 */
export default class extends MonthFitComponent {
  #ro: ResizeObserver | null = null;
  #mql: MediaQueryList | null = null;
  #frame = 0;
  #size = '';
  #events = '';
  #counts: Record<string, number> = {};
  #moreH = MORE_H;

  propsChanged() {
    const events = (this.props.data ?? []).map((d) => `${d.date_iso}:${d.events.map((e) => eventKey(e) + e.summary).join('\u0001')}`).join('\n');
    if (events === this.#events) return;
    this.#events = events;
    this.#schedule();
  }

  connected() {
    this.#events = (this.props.data ?? []).map((d) => `${d.date_iso}:${d.events.map((e) => eventKey(e) + e.summary).join('\u0001')}`).join('\n');
    this.#mql = window.matchMedia('(max-width: 959px)');
    this.#mql.addEventListener('change', this.#onMedia);
    this.#ro = new ResizeObserver(() => {
      const g = this.#grid();
      const size = `${g?.clientWidth}x${g?.clientHeight}`;
      if (size !== this.#size) { this.#size = size; this.#schedule(); }
    });
    const grid = this.#grid();
    if (grid) this.#ro.observe(grid);
    // chip heights change when the web font arrives
    void document.fonts?.ready.then(() => { if (this.root.isConnected) this.#schedule(); });
    this.#schedule();
  }

  disconnected() {
    this.#ro?.disconnect();
    this.#mql?.removeEventListener('change', this.#onMedia);
    cancelAnimationFrame(this.#frame);
  }

  #onMedia = () => this.#schedule();
  #grid = () => this.root.parentElement?.querySelector<HTMLElement>('.month-grid') ?? null;

  #report(counts: Record<string, number>) {
    this.#counts = counts;
    this.emit('fit', { counts, narrow: !!this.#mql?.matches });
  }

  /** After sinter has rendered the last report and the browser has laid it out (two frames). */
  #later(fn: () => void) {
    cancelAnimationFrame(this.#frame);
    this.#frame = requestAnimationFrame(() => { this.#frame = requestAnimationFrame(fn); });
  }

  #schedule() {
    this.#report({});
    this.#later(() => this.#measure());
  }

  #cells() {
    const grid = this.#grid();
    return grid ? [...grid.querySelectorAll<HTMLElement>('.month-cell')] : [];
  }

  #measure() {
    const counts: Record<string, number> = {};
    for (const cell of this.#cells()) {
      const box = cell.querySelector<HTMLElement>('.cell-events');
      const chips = box ? [...box.querySelectorAll<HTMLElement>('.event-trigger')] : [];
      if (!box || !chips.length || box.scrollHeight <= box.clientHeight) continue;
      // chips stack from the box's top: the room is up to the box's bottom, less the "+N" row
      const top = box.getBoundingClientRect().top;
      const room = box.clientHeight - this.#moreH;
      let n = 0;
      while (n < chips.length && chips[n]!.getBoundingClientRect().bottom - top <= room) n++;
      counts[cell.dataset.date!] = n;
    }
    this.#report(counts);
    this.#later(() => this.#check(MAX_PASSES));
  }

  /** One chip fewer in every cell that still overflows, until none does. */
  #check(passes: number) {
    const counts = { ...this.#counts };
    let changed = false;
    for (const cell of this.#cells()) {
      const box = cell.querySelector<HTMLElement>('.cell-events');
      const more = box?.querySelector<HTMLElement>('.cell-more');
      if (more) this.#moreH = more.offsetHeight;
      if (!box || box.scrollHeight <= box.clientHeight + 0.5) continue;
      const date = cell.dataset.date!;
      const shown = counts[date] ?? box.querySelectorAll('.event-trigger').length;
      if (shown === 0) continue; // only the "+N" left: the cell is too small for even that
      counts[date] = shown - 1;
      changed = true;
    }
    if (!changed || passes === 0) return;
    this.#report(counts);
    this.#later(() => this.#check(passes - 1));
  }
}

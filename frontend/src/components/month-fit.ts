// The script of <month-fit>: measures the month grid's cells (layout work markup cannot do).
import type { DayEventsJson } from '../types/calendar.ts';
import { eventKey } from '../lib/events.ts';
import { MonthFitComponent } from './month-fit.html';

/** What <month-fit> reports in its `fit` event. */
export interface MonthFit {
  /** per date shown in the grid: how many event chips fit; dates where all fit are missing */
  counts: Record<string, number>;
  /** per date: how many lines each title may take (cells with room to spare and a cut title); missing = 1 */
  lines: Record<string, number>;
  /** below 60rem: a tap on a cell opens the day's list instead of an event */
  narrow: boolean;
  /** false while a newly shown month is being fitted: the page keeps its chips invisible until then */
  settled: boolean;
  /** dates whose first title takes more than one line (wide screens): the chip spans the cell and its first line
   * leaves the corner to the day number, which it flows around (calendar.scss `.flow`) */
  flow: Record<string, boolean>;
}

const MORE_H = 19; // px: the "+N" row's height until one has been measured
const MAX_PASSES = 8; // corrections after the first measurement
const MAX_LINES = 3; // lines a title may take in a cell with room to spare

/**
 * After each change of the events and each resize it reports how many chips fit each cell, and then how many lines
 * the titles of a cell may take: a cell whose chips all fit, with height left over and a title cut off, gives its
 * titles more lines (up to MAX_LINES), so long titles show more where the month has room for them. It first reports no limits, so every chip renders, estimates the counts from the chips'
 * heights, then checks the result: a cell whose content still overflows (the "+N" row taller than estimated,
 * rounding, a chip that wrapped) loses one chip more, until none overflows. A reload with the same events
 * (a live update) keeps the counts, so the grid does not flash.
 * A newly shown month (other dates, or the first data) is fitted with `settled: false`, so the page can keep the
 * chips invisible until the last step instead of showing each one; a live update or a resize is fitted in view.
 */
export default class extends MonthFitComponent {
  #ro: ResizeObserver | null = null;
  #mql: MediaQueryList | null = null;
  #frame = 0;
  #size = '';
  #events = '';
  #counts: Record<string, number> = {};
  #lines: Record<string, number> = {};
  #flow: Record<string, boolean> = {};
  #moreH = MORE_H;
  #month = ''; // the dates the grid shows, to tell a new month from a change in the same one
  #hiding = false;

  #eventsKey() {
    return (this.props.data ?? []).map((d) => `${d.date_iso}:${d.events.map((e) => eventKey(e) + e.summary).join('\u0001')}`).join('\n');
  }

  /** A new month starts hidden; the same month (a live update) is fitted in view. */
  #newMonth() {
    const data = this.props.data ?? [];
    const month = data.length ? `${data[0]!.date_iso}..${data[data.length - 1]!.date_iso}` : '';
    if (month === this.#month) return;
    this.#month = month;
    this.#hiding = month !== '';
  }

  propsChanged() {
    const events = this.#eventsKey();
    if (events === this.#events) return;
    this.#events = events;
    this.#newMonth();
    this.#schedule();
  }

  connected() {
    this.#events = this.#eventsKey();
    this.#newMonth();
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

  #report(counts: Record<string, number>, lines: Record<string, number> = {}, flow: Record<string, boolean> = {}) {
    this.#counts = counts;
    this.#lines = lines;
    this.#flow = flow;
    this.emit('fit', structuredClone({ counts, lines, flow, narrow: !!this.#mql?.matches, settled: !this.#hiding }));
  }

  /**
   * The last step: on wide screens, the cells whose first title takes more than one line flow it around the day
   * number (its lines after the first get the cell's full width; they need no more room than before, so nothing
   * overflows anew); then the chips are shown (again).
   */
  #done() {
    const flow: Record<string, boolean> = {};
    if (!this.#mql?.matches) {
      for (const cell of this.#cells()) {
        const text = cell.querySelector<HTMLElement>('.chip-text');
        if (!text) continue;
        const lineH = parseFloat(getComputedStyle(text).lineHeight) || 16;
        // the lines it shows (clientHeight), not the text's whole height: a title cut to one line stays beside the number
        if (text.clientHeight > lineH * 1.5) flow[cell.dataset.date!] = true;
      }
    }
    const hiding = this.#hiding;
    this.#hiding = false;
    if (hiding || Object.keys(flow).length) this.#report(this.#counts, this.#lines, flow);
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
    this.#later(() => this.#check(MAX_PASSES, true));
  }

  /**
   * More lines for the titles of cells that show all their chips with room to spare. It assumes every cut title
   * takes all its lines (a title that needs fewer takes fewer), so the estimate errs towards fitting; #check
   * takes lines back from a cell that still overflows.
   */
  #grow() {
    const lines: Record<string, number> = {};
    for (const cell of this.#cells()) {
      const date = cell.dataset.date!;
      const box = cell.querySelector<HTMLElement>('.cell-events');
      if (!box || date in this.#counts) continue; // a cell hiding chips has no room to spare
      const cut = [...box.querySelectorAll<HTMLElement>('.chip-text')].filter((t) => t.scrollHeight > t.clientHeight + 0.5);
      if (!cut.length) continue;
      const last = box.lastElementChild?.getBoundingClientRect().bottom ?? 0;
      const spare = box.getBoundingClientRect().bottom - last;
      const lineH = parseFloat(getComputedStyle(cut[0]!).lineHeight) || 16;
      const n = Math.min(MAX_LINES, 1 + Math.floor(spare / (cut.length * lineH)));
      if (n > 1) lines[date] = n;
    }
    if (!Object.keys(lines).length) return this.#done();
    this.#report(this.#counts, lines);
    this.#later(() => this.#check(MAX_PASSES, false));
  }

  /**
   * In every cell that still overflows: a line less for its titles, or else one chip fewer, until none overflows.
   * `grow`: then give cells with room to spare more lines (once, after the chips' counts have settled).
   */
  #check(passes: number, grow: boolean) {
    const counts = { ...this.#counts };
    const lines = { ...this.#lines };
    let changed = false;
    for (const cell of this.#cells()) {
      const box = cell.querySelector<HTMLElement>('.cell-events');
      const more = box?.querySelector<HTMLElement>('.cell-more');
      if (more) this.#moreH = more.offsetHeight;
      if (!box || box.scrollHeight <= box.clientHeight + 0.5) continue;
      const date = cell.dataset.date!;
      if ((lines[date] ?? 1) > 1) {
        lines[date] = lines[date]! - 1;
        changed = true;
        continue;
      }
      const shown = counts[date] ?? box.querySelectorAll('.event-trigger').length;
      if (shown === 0) continue; // only the "+N" left: the cell is too small for even that
      counts[date] = shown - 1;
      changed = true;
    }
    if (!changed || passes === 0) {
      if (grow) this.#grow();
      else this.#done();
      return;
    }
    this.#report(counts, lines);
    this.#later(() => this.#check(passes - 1, grow));
  }
}

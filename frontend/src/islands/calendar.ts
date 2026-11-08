// Islands of the calendar page (calendar/index.html): work that measures layout or runs timers.
import { dateFromEventValue, formatEventTime } from '../lib/format.ts';
import type { DayEventsJson } from '../types/calendar.ts';

/** What <month-fit> reports in its `fit` event. */
export interface MonthFit {
  /** per date shown in the grid: how many event chips fit; dates where all fit are missing */
  counts: Record<string, number>;
  /** below 60rem: a tap on a cell opens the day's list instead of an event */
  narrow: boolean;
}

const MORE_H = 19; // px: the "+N" row

/**
 * `<month-fit hidden data-sn-prop="{data: …}" data-sn-on-fit="fit = $event.detail">`, next to the month grid
 * (`.month-grid` in the same parent): after each data change and resize it reports how many chips fit
 * each cell. It first reports no limits, so every chip renders, then measures them.
 */
customElements.define('month-fit', class extends HTMLElement {
  #ro: ResizeObserver | null = null;
  #mql: MediaQueryList | null = null;
  #frame = 0;
  #size = '';

  set data(_: unknown) { this.#schedule(); }

  connectedCallback() {
    this.#mql = window.matchMedia('(max-width: 959px)');
    this.#mql.addEventListener('change', this.#onMedia);
    this.#ro = new ResizeObserver(() => {
      const g = this.#grid();
      const size = `${g?.clientWidth}x${g?.clientHeight}`;
      if (size !== this.#size) { this.#size = size; this.#schedule(); }
    });
    const grid = this.#grid();
    if (grid) this.#ro.observe(grid);
    this.#schedule();
  }

  disconnectedCallback() {
    this.#ro?.disconnect();
    this.#mql?.removeEventListener('change', this.#onMedia);
    cancelAnimationFrame(this.#frame);
  }

  #onMedia = () => this.#schedule();
  #grid = () => this.parentElement?.querySelector<HTMLElement>('.month-grid') ?? null;

  #report(counts: Record<string, number>) {
    this.dispatchEvent(new CustomEvent<MonthFit>('fit', { detail: { counts, narrow: !!this.#mql?.matches } }));
  }

  #schedule() {
    cancelAnimationFrame(this.#frame);
    this.#report({});
    // two frames: sinter renders the unlimited chips, then the browser lays them out
    this.#frame = requestAnimationFrame(() => { this.#frame = requestAnimationFrame(() => this.#measure()); });
  }

  #measure() {
    const grid = this.#grid();
    if (!grid) return;
    const counts: Record<string, number> = {};
    for (const cell of grid.querySelectorAll<HTMLElement>('.month-cell')) {
      const box = cell.querySelector<HTMLElement>('.cell-events');
      const chips = box ? [...box.querySelectorAll<HTMLElement>('.event-trigger')] : [];
      if (!box || !chips.length) continue;
      const available = box.clientHeight - 2; // the list box's top and bottom border
      if (available <= 0) continue;
      let h = 0, n = 0;
      for (let i = 0; i < chips.length; i++) {
        const chip = chips[i]!.offsetHeight + 1; // +1px gap
        if (h + chip + (i < chips.length - 1 ? MORE_H : 0) > available) break;
        h += chip;
        n++;
      }
      if (n < chips.length) counts[cell.dataset.date!] = n;
    }
    this.#report(counts);
  }
});

/**
 * `<event-reminders hidden data-sn-prop="{days: weekDays}">`: desktop notifications for the event alarms of
 * the next 24 hours among `days`. Asks for permission once, as the calendar always did.
 */
customElements.define('event-reminders', class extends HTMLElement {
  #timers: ReturnType<typeof setTimeout>[] = [];

  set days(days: readonly DayEventsJson[]) {
    this.#clear();
    if (typeof Notification === 'undefined' || !days?.length) return;
    if (Notification.permission === 'granted') this.#schedule(days);
    else if (Notification.permission === 'default') {
      void Notification.requestPermission().then((p) => { if (p === 'granted' && this.isConnected) this.#schedule(days); });
    }
  }

  disconnectedCallback() { this.#clear(); }

  #clear() {
    for (const t of this.#timers) clearTimeout(t);
    this.#timers = [];
  }

  #schedule(days: readonly DayEventsJson[]) {
    const now = Date.now(), horizon = now + 86400000;
    const ms = (s: string) => { const d = s ? dateFromEventValue(s) : null; return d ? d.getTime() : NaN; };
    for (const day of days) {
      for (const ev of day.events) {
        for (const alarm of ev.alarms) {
          let fire: number;
          if (alarm.direction === 'absolute') {
            if (!alarm.datetime) continue;
            fire = new Date(alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z')).getTime();
          } else {
            const base = alarm.direction.endsWith('-end') ? ms(ev.dtend) : ms(ev.dtstart);
            if (isNaN(base)) continue;
            fire = base + (alarm.direction.startsWith('before') ? -1 : 1) * parseInt(alarm.value || '0') * parseInt(alarm.unit || '1') * 60000;
          }
          if (isNaN(fire) || fire <= now || fire > horizon) continue;
          this.#timers.push(setTimeout(() => {
            if (Notification.permission === 'granted') new Notification(ev.summary, { body: formatEventTime(ev) });
          }, fire - now));
        }
      }
    }
  }
});

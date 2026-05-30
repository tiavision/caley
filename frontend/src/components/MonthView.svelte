<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson, DayEventsJson, MonthGridJson } from '../types/calendar.ts';

  export let monthGrid: MonthGridJson | null;
  export let days: DayEventsJson[];
  export let todayStr: string;
  export let locale: string;
  export let onEventClick: (event: EventJson) => void;
  export let onMoreClick: (date: string, events: EventJson[]) => void;

  $: tl = makeTl(locale);

  // ── Day name headers (Mon–Sun) ─────────────────────────────────────────────
  const DOW_KEYS = ['event_dayMo','event_dayTu','event_dayWe','event_dayTh','event_dayFr','event_daySa','event_daySu'] as const;

  // ── Event map ──────────────────────────────────────────────────────────────
  let dayMap: Map<string, EventJson[]> = new Map();
  $: {
    const m = new Map<string, EventJson[]>();
    for (const d of days) m.set(d.date_iso, d.events);
    dayMap = m;
  }

  // ── Mobile detection ───────────────────────────────────────────────────────
  let isMobile = false;
  let mql: MediaQueryList | null = null;
  function onMqlChange(e: MediaQueryListEvent) { isMobile = e.matches; }

  // ── Visible counts (DOM measurement) ──────────────────────────────────────
  let visibleCounts: Map<string, number> = new Map();
  let gridEl: HTMLElement;
  let roScheduled = false;
  const MORE_BTN_H = 19; // px — estimated height of the "+N more" row

  function computeVisibility() {
    if (!gridEl) return;
    const next = new Map<string, number>();
    for (const cell of gridEl.querySelectorAll<HTMLElement>('.month-cell')) {
      const dateStr = cell.dataset.date!;
      const container = cell.querySelector<HTMLElement>('.cell-events');
      if (!container) continue;
      const chips = Array.from(container.querySelectorAll<HTMLElement>('.month-appointment'));
      if (!chips.length) continue;

      const available = container.clientHeight;
      if (available <= 0) continue;

      let h = 0, count = 0;
      for (let i = 0; i < chips.length; i++) {
        const chipH = chips[i].offsetHeight + 1; // +1px gap
        const moreChipsFollow = i < chips.length - 1;
        const reserve = moreChipsFollow ? MORE_BTN_H : 0;
        if (h + chipH + reserve > available) break;
        h += chipH;
        count++;
      }
      if (count < chips.length) next.set(dateStr, count);
    }
    visibleCounts = next;
  }

  let ro: ResizeObserver | null = null;

  function scheduleCompute() {
    if (roScheduled) return;
    roScheduled = true;
    requestAnimationFrame(() => { roScheduled = false; computeVisibility(); });
  }

  onMount(() => {
    mql = window.matchMedia('(max-width: 959px)');
    isMobile = mql.matches;
    mql.addEventListener('change', onMqlChange);

    ro = new ResizeObserver(scheduleCompute);
    if (gridEl) ro.observe(gridEl);

    // Initial measurement after layout
    requestAnimationFrame(computeVisibility);
  });

  onDestroy(() => {
    mql?.removeEventListener('change', onMqlChange);
    ro?.disconnect();
  });

  // Re-compute whenever data changes
  $: if (days || monthGrid) {
    // Schedule after Svelte has updated the DOM
    if (typeof requestAnimationFrame !== 'undefined') {
      requestAnimationFrame(() => requestAnimationFrame(computeVisibility));
    }
  }

  // ── Row count ─────────────────────────────────────────────────────────────
  $: rowCount = monthGrid ? monthGrid.days.length / 7 : 5;
</script>

<div class="month-view">
  <!-- Day-of-week header -->
  <div class="month-dow-header">
    {#each DOW_KEYS as key}
      <div class="month-dow-label">{tl(key)}</div>
    {/each}
  </div>

  <!-- Calendar grid -->
  {#if monthGrid}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="month-grid"
      bind:this={gridEl}
      style="--row-count: {rowCount}"
      role="grid"
      aria-label={monthGrid.month_name + ' ' + monthGrid.year}
    >
      {#each monthGrid.days as cell (cell.date_str)}
        {@const events = dayMap.get(cell.date_str) ?? []}
        {@const maxVis = visibleCounts.has(cell.date_str) ? visibleCounts.get(cell.date_str)! : events.length}
        {@const visibleEvents = events.slice(0, maxVis)}
        {@const hidden = events.length - visibleEvents.length}
        {@const isToday = cell.date_str === todayStr}

        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="month-cell"
          class:other-month={!cell.is_current_month}
          class:today={isToday}
          class:has-events={events.length > 0}
          data-date={cell.date_str}
          role="gridcell"
          onclick={() => { if (isMobile && events.length > 0) onMoreClick(cell.date_str, events); }}
        >
          <span class="cell-day-num" class:today-num={isToday}>{cell.day_number}</span>

          <div class="cell-events">
            {#each visibleEvents as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <article
                class="appointment month-appointment event-trigger"
                style="border-left: 3px solid {event.color}"
                tabindex="-1"
                onclick={(e) => { if (!isMobile) { e.stopPropagation(); onEventClick(event); } }}
                onkeydown={(e) => { if (!isMobile && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onEventClick(event); } }}
              >
                <div class="appt-content month-appt-content">
                  {event.summary}
                </div>
              </article>
            {/each}

            {#if hidden > 0}
              <button
                class="more-btn"
                onclick={(e) => { e.stopPropagation(); onMoreClick(cell.date_str, events); }}
              >
                +{hidden} {tl('cal_month_more')}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .month-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  /* Day-of-week header */
  .month-dow-header {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    border-bottom: var(--pico-border-width) solid var(--pico-muted-border-color);
    flex-shrink: 0;
  }
  .month-dow-label {
    text-align: center;
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--pico-muted-color);
    padding: 0.3rem 0;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  /* Grid */
  .month-grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    grid-template-rows: repeat(var(--row-count), 1fr);
    flex: 1;
    min-height: 0;
    border-left: var(--pico-border-width) solid var(--pico-muted-border-color);
    border-bottom: var(--pico-border-width) solid var(--pico-muted-border-color);
  }

  /* Cell */
  .month-cell {
    border-right: var(--pico-border-width) solid var(--pico-muted-border-color);
    border-top: var(--pico-border-width) solid var(--pico-muted-border-color);
    padding: 0.2rem 0.15rem 0.15rem;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
    transition: background 0.1s;
  }
  .month-cell.other-month {
    opacity: 0.45;
  }
  .month-cell.today {
    border-top: 2px solid var(--pico-primary-background);
  }

  @media (max-width: 959px) {
    .month-cell { padding-left: 0; padding-right: 0; }
    .month-cell.has-events { cursor: pointer; }
    .month-cell.has-events:hover { background: var(--pico-card-background-color, rgba(0,0,0,0.02)); }
  }
  @media (prefers-color-scheme: dark) {
    .month-cell.has-events:hover { background: rgba(255,255,255,0.03); }
  }

  /* Day number */
  .cell-day-num {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--pico-muted-color);
    line-height: 1.4;
    flex-shrink: 0;
    align-self: flex-end;
    min-width: 1.4em;
    text-align: center;
    border-radius: 999px;
    padding: 0 0.15em;
    margin-bottom: 0.1rem;
  }
  .cell-day-num.today-num {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    font-weight: 700;
  }

  /* Events area */
  .cell-events {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  /* Compact month event chips */
  :global(.month-appointment) {
    margin: 0 !important;
    padding: 0.05rem 0.15rem !important;
    flex-shrink: 0;
  }
  @media (max-width: 959px) {
    :global(article.appointment.month-appointment) {
      border-left-width: 2px !important;
      padding: 0.25rem 0.15rem !important;
      margin-bottom: 0.3rem !important;
    }
  }
  /* Override weekly-view line-clamp: plain cut, break-all, no ellipsis */
  :global(.month-appointment .month-appt-content) {
    display: block !important;
    -webkit-line-clamp: unset !important;
    overflow: hidden !important;
    max-height: 2.8em !important; /* 2 lines × line-height 1.4 */
    word-break: break-all;
    font-size: 0.8em;
  }

  /* "+N more" button */
  .more-btn {
    background: none;
    border: none;
    padding: 0.05rem 0.15rem;
    margin: 0;
    font-size: 0.75em;
    color: var(--pico-muted-color);
    cursor: pointer;
    text-align: left;
    border-radius: 3px;
    line-height: 1.4;
    flex-shrink: 0;
  }
  .more-btn:hover { color: var(--pico-primary-background); }
</style>

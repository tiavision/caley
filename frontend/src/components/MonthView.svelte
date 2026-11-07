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
      const chips = Array.from(container.querySelectorAll<HTMLElement>('.event-trigger'));
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
  <div class="dow" data-layout="grid" data-cols="7" data-gap="none" aria-hidden="true">
    {#each DOW_KEYS as key}
      <span data-caption>{tl(key)}</span>
    {/each}
  </div>

  <!-- Calendar grid -->
  {#if monthGrid}
    <div
      class="month-grid"
      data-layout="grid"
      data-cols="7"
      data-gap="none"
      bind:this={gridEl}
      style="--row-count: {rowCount}"
      role="group"
      aria-label={monthGrid.month_name + ' ' + monthGrid.year}
    >
      {#each monthGrid.days as cell (cell.date_str)}
        {@const events = dayMap.get(cell.date_str) ?? []}
        {@const maxVis = visibleCounts.has(cell.date_str) ? visibleCounts.get(cell.date_str)! : events.length}
        {@const visibleEvents = events.slice(0, maxVis)}
        {@const hidden = events.length - visibleEvents.length}
        {@const isToday = cell.date_str === todayStr}

        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          class="month-cell"
          class:other-month={!cell.is_current_month}
          class:has-events={events.length > 0}
          data-layout="stack"
          data-gap="xs"
          data-date={cell.date_str}
          onclick={() => { if (isMobile && events.length > 0) onMoreClick(cell.date_str, events); }}
        >
          <span class="day-num" data-self="end" aria-current={isToday ? 'date' : null}>{cell.day_number}</span>

          <div class="cell-events" data-layout="stack" data-gap="none" data-grow>
            {#each visibleEvents as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
              <button
                type="button"
                class="event-trigger"
                data-variant="item"
                data-size="sm"
                data-bar
                style={event.color ? `--tm-tone: ${event.color}` : null}
                tabindex="-1"
                onclick={(e) => { if (!isMobile) { e.stopPropagation(); onEventClick(event); } }}
              >
                <span class="chip-text">{event.summary}</span>
              </button>
            {/each}

            {#if hidden > 0}
              <button
                type="button"
                data-variant="link"
                data-size="sm"
                data-tone="neutral"
                data-self="start"
                onclick={(e) => { e.stopPropagation(); onMoreClick(cell.date_str, events); }}
              >
                +{hidden}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Month grid: hairline-ruled cells that share the pane height; temper has no ruled grid. */
  .month-view {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .dow {
    flex-shrink: 0;
    text-align: center;
    text-transform: uppercase;
    border-block-end: 1px solid var(--tm-border);
  }
  .month-grid {
    grid-template-rows: repeat(var(--row-count), minmax(0, 1fr));
    flex: 1;
    min-block-size: 0;
    border-inline-start: 1px solid var(--tm-border);
    border-block-end: 1px solid var(--tm-border);
  }
  .month-cell {
    min-block-size: 0;
    overflow: hidden;
    padding: var(--tm-space-xs) 2px 2px;
    border-inline-end: 1px solid var(--tm-border);
    border-block-start: 1px solid var(--tm-border);
  }
  .month-cell.other-month { opacity: 0.45; }

  /* Day number; today gets a filled accent circle. */
  .day-num {
    flex-shrink: 0;
    min-inline-size: 1.5em;
    border-radius: var(--tm-radius-full);
    font-size: var(--tm-text-xs);
    font-weight: 500;
    line-height: 1.5;
    text-align: center;
    color: var(--tm-text-muted);
  }
  .day-num[aria-current='date'] {
    --tm-tone: var(--tm-accent);
    background: var(--tm-tone);
    color: var(--tm-tone-on);
    font-weight: 700;
  }

  .cell-events { overflow: hidden; }
  .cell-events > :global(*) { flex-shrink: 0; }
  /* Two lines at most, cut without ellipsis (chips are narrow). */
  .chip-text {
    max-block-size: 2.8em;
    overflow: hidden;
    font-size: var(--tm-text-xs);
    word-break: break-all;
  }

  @media (width < 60rem) {
    .month-cell { padding-inline: 0; }
    .month-cell.has-events { cursor: pointer; }
    .month-cell.has-events:hover { background: var(--tm-fill); }
  }
</style>

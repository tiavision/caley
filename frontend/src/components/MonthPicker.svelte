<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import type { MonthGridJson } from '../types/calendar.ts';

  export let monthGrid: MonthGridJson | null;
  export let weekStart: Date;
  export let weekEnd: Date;
  export let todayStr: string;
  export let locale: string;

  const dispatch = createEventDispatcher<{ selectDay: string; navigateToMonth: string }>();

  $: tl = (() => { locale; return (key: string, params?: Record<string, string | number>) => { const fn = (m as Record<string, Function>)[key]; return typeof fn === 'function' ? fn(params) : key; }; })();

  $: gridYear = monthGrid?.year ?? new Date().getFullYear();
  $: gridMonthNum = monthGrid?.month_number ?? (new Date().getMonth() + 1);

  $: monthNames = (() => {
    locale;
    const loc = locale;
    return Array.from({ length: 12 }, (_, i) =>
      new Date(2024, i, 1).toLocaleDateString(loc, { month: 'long' })
    );
  })();

  $: dayOfWeekShort = (() => {
    locale;
    const loc = locale;
    // Jan 1 2024 is Monday
    return Array.from({ length: 7 }, (_, i) =>
      new Date(2024, 0, 1 + i).toLocaleDateString(loc, { weekday: 'short' })
    );
  })();

  const yearRange = Array.from({ length: 101 }, (_, i) => new Date().getFullYear() - 50 + i);

  $: selectedWeekDates = (() => {
    weekStart; weekEnd;
    const set = new Set<string>();
    if (monthGrid) {
      for (const day of monthGrid.days) {
        if (day.is_current_month) {
          const [y, mo, d] = day.date_str.split('-').map(Number);
          const dt = new Date(y, mo - 1, d);
          dt.setHours(0, 0, 0, 0);
          if (dt.getTime() >= weekStart.getTime() && dt.getTime() <= weekEnd.getTime()) {
            set.add(day.date_str);
          }
        }
      }
    }
    return set;
  })();

  function onSelectChange(field: 'month' | 'year', val: number) {
    if (field === 'month') {
      dispatch('navigateToMonth', String(gridYear) + '-' + String(val).padStart(2, '0'));
    } else {
      dispatch('navigateToMonth', String(val) + '-' + String(gridMonthNum).padStart(2, '0'));
    }
  }
</script>

<article class="card contrast">
  {#if monthGrid}
    <div style="position:relative">
      <header>
        <hgroup>
          <h2>
            <button
              class="nav-btn"
              aria-label={tl('cal_aria_prevMonth')}
              on:click={() => dispatch('navigateToMonth', monthGrid.prev_month)}
            ><span class="arrow">‹</span></button>

            <span class="month-picker">
              <select
                aria-label={tl('cal_aria_month')}
                value={gridMonthNum}
                on:change={(e) => onSelectChange('month', +(e.target as HTMLSelectElement).value)}
              >
                {#each monthNames as name, i}
                  <option value={i + 1}>{name}</option>
                {/each}
              </select>
              <select
                aria-label={tl('cal_aria_year')}
                value={gridYear}
                on:change={(e) => onSelectChange('year', +(e.target as HTMLSelectElement).value)}
              >
                {#each yearRange as y}
                  <option value={y}>{y}</option>
                {/each}
              </select>
            </span>

            <button
              class="nav-btn"
              aria-label={tl('cal_aria_nextMonth')}
              on:click={() => dispatch('navigateToMonth', monthGrid.next_month)}
            ><span class="arrow">›</span></button>
          </h2>
        </hgroup>
      </header>

      <div class="date-grid">
        {#each dayOfWeekShort as dayLabel}
          <div class="date-header">{dayLabel}</div>
        {/each}
      </div>

      <div class="date-grid" role="grid">
        {#each monthGrid.days as day (day.date_str)}
          {#if day.is_current_month}
            <div
              class="date-cell"
              class:selected-week={selectedWeekDates.has(day.date_str)}
              class:is-today={day.date_str === todayStr}
              data-date={day.date_str}
              role="gridcell"
              tabindex="0"
              on:click={() => dispatch('selectDay', day.date_str)}
              on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); dispatch('selectDay', day.date_str); } }}
            >
              <span class="day-number">{day.day_number}</span>
              {#if day.event_colors.length > 0}
                <div class="event-dots">
                  {#each day.event_colors.slice(0, 3) as color}
                    <span class="event-dot" style="background:{color}"></span>
                  {/each}
                </div>
              {/if}
            </div>
          {:else}
            <div class="date-cell empty"></div>
          {/if}
        {/each}
      </div>
    </div>
  {:else}
    <div class="calendar-loading">{tl('cal_loadingCalendar')}</div>
  {/if}
</article>

<style>
  hgroup h2 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
  }
  .month-picker { display: inline-flex; align-items: baseline; gap: 0.25rem; white-space: nowrap; flex: 1; justify-content: center; }
  .month-picker select {
    font: inherit;
    min-width: 0;
    cursor: pointer;
    padding: 0.15rem 1.25rem 0.15rem 0.4rem;
    line-height: 1.4;
    margin: 0;
  }
  .month-picker select:last-of-type { max-width: 7rem; }

  .date-grid {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 0;
  }
  .date-header {
    text-align: center;
    min-width: 0;
    font-size: 0.75rem;
    color: var(--pico-muted-color);
    padding: 0.25rem 0;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .date-cell {
    padding: 0 0.25rem;
    min-width: 0;
    text-align: center;
    border-bottom: var(--pico-border-width) solid var(--pico-muted-border-color);
    border-right: var(--pico-border-width) solid var(--pico-muted-border-color);
    aspect-ratio: 1;
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    font-size: 0.875rem;
    transition: background 0.1s;
    background: transparent;
    color: inherit;
  }
  .date-cell:nth-child(7n) { border-right: none; }
  .date-cell:nth-last-child(-n+7) { border-bottom: none; }
  .date-cell.selected-week {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    font-weight: 600;
  }
  .date-cell.empty { background: transparent; cursor: default; }
  .date-cell.is-today .day-number {
    position: relative;
    z-index: 0;
    display: block;
    width: 1.5rem;
    line-height: 1.5rem;
    text-align: center;
    color: var(--pico-primary-inverse);
    font-weight: 700;
    margin: 0 auto;
  }
  .date-cell.is-today .day-number::before {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: 50%;
    background: var(--pico-primary-background);
    z-index: -1;
  }
  .date-cell.selected-week.is-today .day-number {
    color: var(--pico-primary-background);
  }
  .date-cell.selected-week.is-today .day-number::before {
    background: var(--pico-primary-inverse);
  }
  .event-dots { position: absolute; bottom: 0.3rem; left: 0; right: 0; display: flex; justify-content: center; gap: 3px; }
  .event-dot { width: 6px; height: 6px; border-radius: 50%; display: inline-block; flex-shrink: 0; }
  .calendar-loading { padding: 2rem; text-align: center; font-style: italic; color: var(--pico-muted-color); }
</style>

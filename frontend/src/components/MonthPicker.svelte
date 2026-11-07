<script lang="ts">
  import { makeTl } from '../lib/i18n.ts';
  import type { MonthGridJson } from '../types/calendar.ts';

  export let monthGrid: MonthGridJson | null;
  export let weekStart: Date;
  export let weekEnd: Date;
  export let todayStr: string;
  export let locale: string;
  export let onselectDay: ((d: string) => void) | undefined = undefined;
  export let onnavigateToMonth: ((m: string) => void) | undefined = undefined;

  $: tl = makeTl(locale);

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
      onnavigateToMonth?.(String(gridYear) + '-' + String(val).padStart(2, '0'));
    } else {
      onnavigateToMonth?.(String(val) + '-' + String(gridMonthNum).padStart(2, '0'));
    }
  }
</script>

<div class="picker" data-layout="stack" data-gap="sm" data-pad="xs">
  {#if monthGrid}
    <div data-layout="row" data-gap="xs" data-wrap="none">
      <button
        type="button"
        data-variant="ghost"
        data-size="sm"
        data-icon="chevron-left"
        aria-label={tl('cal_aria_prevMonth')}
        onclick={() => onnavigateToMonth?.(monthGrid.prev_month)}
      ></button>
      <select
        data-size="sm"
        data-grow
        aria-label={tl('cal_aria_month')}
        value={gridMonthNum}
        onchange={(e) => onSelectChange('month', +(e.target as HTMLSelectElement).value)}
      >
        {#each monthNames as name, i}
          <option value={i + 1}>{name}</option>
        {/each}
      </select>
      <select
        data-size="sm"
        aria-label={tl('cal_aria_year')}
        value={gridYear}
        onchange={(e) => onSelectChange('year', +(e.target as HTMLSelectElement).value)}
      >
        {#each yearRange as y}
          <option value={y}>{y}</option>
        {/each}
      </select>
      <button
        type="button"
        data-variant="ghost"
        data-size="sm"
        data-icon="chevron-right"
        aria-label={tl('cal_aria_nextMonth')}
        onclick={() => onnavigateToMonth?.(monthGrid.next_month)}
      ></button>
    </div>

    <div data-layout="grid" data-cols="7" data-gap="xs">
      {#each dayOfWeekShort as dayLabel}
        <span class="dow" data-caption aria-hidden="true">{dayLabel}</span>
      {/each}
      {#each monthGrid.days as day (day.date_str)}
        {#if day.is_current_month}
          <button
            type="button"
            class="date"
            data-variant="ghost"
            data-size="sm"
            data-layout="stack"
            data-gap="none"
            data-align="center"
            aria-pressed={selectedWeekDates.has(day.date_str)}
            data-date={day.date_str}
            onclick={() => onselectDay?.(day.date_str)}
          >
            <span aria-current={day.date_str === todayStr ? 'date' : null}>{day.day_number}</span>
            <span class="dots" aria-hidden="true">
              {#each day.event_colors.slice(0, 3) as color}
                <span class="dot" style="--tm-tone: {color}"></span>
              {/each}
            </span>
          </button>
        {:else}
          <span></span>
        {/if}
      {/each}
    </div>
  {:else}
    <p data-muted aria-busy="true">{tl('cal_loadingCalendar')}</p>
  {/if}
</div>

<style>
  .picker { inline-size: min(18rem, 100vw - 2rem); }
  .dow { text-align: center; }
  .date { padding-inline: 0; }
  /* Up to three calendar-colored dots under the day number. */
  .dots { display: flex; gap: 3px; block-size: 6px; }
  .dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--tm-tone); }
</style>

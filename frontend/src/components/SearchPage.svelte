<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import EventColorDot from './EventColorDot.svelte';
  import EventDetail from './EventDetail.svelte';
  import EventFormDialog from './EventFormDialog.svelte';
  import type { EventJson, CalendarJson, CalDavAccountJson } from '../types/calendar.ts';
  import { apiFetch } from '../lib/api.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let query = '';
  let start = '';
  let end = '';
  let results: EventJson[] = [];
  let loading = false;
  let searched = false;
  let errorMsg = '';

  let calendars: CalendarJson[] = [];
  let accounts: CalDavAccountJson[] = [];
  let selectedHrefs = new Set<string>();
  let calendarsLoaded = false;
  let calDropdownOpen = false;

  let eventDetail: EventDetail;
  let formDialog: EventFormDialog;
  let detailIsOpen = false;

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    try {
      const res = await apiFetch('/api/calendars');
      if (res.ok) {
        calendars = await res.json();
        selectedHrefs = new Set(calendars.filter(c => c.is_active).map(calendarKey));
      }
      const accountRes = await apiFetch('/api/caldav/accounts');
      if (accountRes.ok) accounts = (await accountRes.json()).accounts || [];
    } catch {}
    calendarsLoaded = true;
  });

  $: calBtnLabel = (() => {
    if (!calendarsLoaded || calendars.length === 0) return tl('search_calendars');
    const n = selectedHrefs.size;
    const total = calendars.length;
    return n === total ? tl('search_calendars') : `${tl('search_calendars')} (${n}/${total})`;
  })();

  function calendarKey(cal: CalendarJson): string {
    return `${cal.account_id}\0${cal.href}`;
  }

  function toggleCalendar(key: string) {
    if (selectedHrefs.has(key)) selectedHrefs.delete(key);
    else selectedHrefs.add(key);
    selectedHrefs = selectedHrefs;
  }

  function handleWindowClick(e: MouseEvent) {
    if (calDropdownOpen && !(e.target as Element).closest('.cal-dropdown')) {
      calDropdownOpen = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const tag = (document.activeElement as HTMLElement)?.tagName?.toLowerCase();
    if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
    if (detailIsOpen && e.key === 'e') { e.preventDefault(); eventDetail.triggerEdit(); }
  }

  async function doSearch() {
    const q = query.trim();
    if (!q) return;

    loading = true;
    searched = false;
    errorMsg = '';
    results = [];

    try {
      const params = new URLSearchParams({ q });
      if (start) params.set('start', start);
      if (end) params.set('end', end);
      if (calendarsLoaded && calendars.length > 0) {
        for (const href of selectedHrefs) params.append('cal', href);
      }

      const res = await apiFetch(`/api/search?${params}`);
      if (!res.ok) {
        errorMsg = tl('search_error');
      } else {
        results = await res.json();
        searched = true;
      }
    } catch {
      errorMsg = tl('search_error');
    }

    loading = false;
  }

  function formatDate(dtstart: string): string {
    const datePart = dtstart.split('T')[0].replace(/(\d{4})(\d{2})(\d{2})/, '$1-$2-$3');
    const [y, mo, d] = datePart.split('-').map(Number);
    if (!y || !mo || !d) return dtstart;
    return new Date(y, mo - 1, d).toLocaleDateString(locale, { dateStyle: 'long' });
  }

  function calendarName(accountId: string, href: string): string {
    return calendars.find(c => c.account_id === accountId && c.href === href)?.name ?? '';
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

<div class="search-page">
<div class="page-header">
  <h2>{tl('search_heading')}</h2>
</div>

<div class="filter-row">
  <div class="cal-dropdown">
    <button
      type="button"
      class="cal-dropdown-btn"
      onclick={() => calDropdownOpen = !calDropdownOpen}
    >
      {calBtnLabel} <span class="dropdown-arrow" aria-hidden="true">▾</span>
    </button>
    {#if calDropdownOpen}
      <ul class="cal-dropdown-list">
        {#each calendars as cal (cal.account_id + cal.href)}
          <li>
            <label>
              <input
                type="checkbox"
                checked={selectedHrefs.has(calendarKey(cal))}
                onchange={() => toggleCalendar(calendarKey(cal))}
              />
              <EventColorDot color={cal.color} />
              {cal.name}
            </label>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="date-range">
    <label class="date-label">
      {tl('search_from')}
      <input type="date" bind:value={start} />
    </label>
    <label class="date-label">
      {tl('search_to')}
      <input type="date" bind:value={end} />
    </label>
  </div>
</div>

<form onsubmit={(e) => { e.preventDefault(); doSearch(); }} class="search-form">
  <div class="search-row">
    <input
      type="search"
      bind:value={query}
      placeholder={tl('search_placeholder')}
      class="search-input"
      autofocus
    />
    <button type="submit">{tl('search_submit')}</button>
  </div>
</form>

{#if loading}
  <p aria-live="polite">{tl('search_loading')}</p>
{:else if errorMsg}
  <p class="form-error" aria-live="polite">{errorMsg}</p>
{:else if searched && results.length === 0}
  <p aria-live="polite">{tl('search_noResults')}</p>
{:else if results.length > 0}
  <ul class="search-results" aria-live="polite">
    {#each results as item (`${item.account_id}\0${item.calendar_href}\0${item.uid}\0${item.dtstart}`)}
      <li>
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <button type="button" class="search-result" onclick={() => eventDetail.open(item)}>
          <EventColorDot color={item.color} />
          <div class="result-body">
            <span class="result-summary">{item.summary}</span>
            <span class="result-meta">
              {formatDate(item.dtstart)}
              {#if item.location} · {item.location}{/if}
            </span>
            {#if item.description}
              <span class="result-desc">{item.description}</span>
            {/if}
            {#if calendarName(item.account_id, item.calendar_href)}
              <span class="result-calendar">{calendarName(item.account_id, item.calendar_href)}</span>
            {/if}
          </div>
        </button>
      </li>
    {/each}
  </ul>
{/if}
</div>

<EventDetail
  bind:this={eventDetail}
  bind:isOpen={detailIsOpen}
  {locale}
  onedit={(d) => formDialog.openEditForm(d.event, d.scope)}
/>

<EventFormDialog
  bind:this={formDialog}
  {calendars}
  {accounts}
  {locale}
  focusedEventDate=""
  onsaved={() => doSearch()}
  ondeleted={() => doSearch()}
/>

<style>
  .search-page {
    box-sizing: border-box;
    width: 100%;
  }

  .page-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .page-header h2 { margin: 0; }

  .filter-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    width: 100%;
    margin-bottom: 0.75rem;
  }

  .date-label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8em;
    margin-bottom: 0;
    white-space: nowrap;
  }

  .date-range {
    display: flex;
    gap: 0.75rem;
    align-items: center;
    flex-shrink: 0;
  }

  .date-label input[type="date"] {
    margin-bottom: 0;
    font-size: 0.85em;
    opacity: 0.65;
    padding: var(--pico-form-element-spacing-vertical) var(--pico-form-element-spacing-horizontal) !important;
  }

  .cal-dropdown {
    position: relative;
  }

  .cal-dropdown-btn {
    opacity: 0.65;
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.85em;
    width: auto;
    margin-bottom: 0;
    cursor: pointer;
    background: var(--pico-form-element-background-color);
    border: var(--pico-border-width) solid var(--pico-form-element-border-color);
    box-shadow: none;
    color: var(--pico-form-element-color);
    line-height: var(--pico-line-height);
  }

  .dropdown-arrow {
    font-size: 0.7em;
    opacity: 0.6;
    line-height: 1;
  }

  .cal-dropdown-list {
    position: absolute;
    top: calc(100% + 0.25rem);
    left: 0;
    z-index: 100;
    background: var(--pico-card-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.25rem 0;
    margin: 0;
    min-width: 12rem;
    max-height: 16rem;
    overflow-y: auto;
    box-shadow: 0 0.25rem 0.75rem rgba(0, 0, 0, 0.12);
    list-style: none;
  }

  .cal-dropdown-list li {
    list-style: none;
  }

  .cal-dropdown-list label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
    cursor: pointer;
    margin-bottom: 0;
    font-size: 0.9em;
    white-space: nowrap;
  }

  .cal-dropdown-list label:hover {
    background: var(--pico-muted-background-color);
  }

  .cal-dropdown-list input[type="checkbox"] {
    margin-bottom: 0;
    cursor: pointer;
  }

  .search-form {
    width: 100%;
    margin-bottom: 1.5rem;
  }

  .search-row {
    display: flex;
    gap: 0.5rem;
    align-items: center;
  }

  .search-input {
    flex: 1;
    min-width: 0;
    margin-bottom: 0;
  }

  .search-row button {
    width: auto;
    flex-shrink: 0;
    margin-bottom: 0;
  }

  .search-results {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .search-results li {
    list-style: none;
  }

  .search-result :global(.dot) {
    margin-top: 0.5rem;
  }

  .search-result {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    padding: 0.6rem 0.75rem;
    border-radius: var(--pico-border-radius);
    text-decoration: none;
    color: inherit;
    border: 1px solid var(--pico-muted-border-color);
    background: none;
    width: 100%;
    text-align: left;
    cursor: pointer;
    font-size: inherit;
    line-height: inherit;
  }

  .search-result:hover {
    background: var(--pico-card-background-color);
    border-color: var(--pico-primary);
  }

  .result-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .result-summary {
    font-weight: 600;
  }

  .result-meta {
    font-size: 0.85em;
    opacity: 0.75;
  }

  .result-desc {
    font-size: 0.85em;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .result-calendar {
    font-size: 0.8em;
    opacity: 0.6;
  }
</style>

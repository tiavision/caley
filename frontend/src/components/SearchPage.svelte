<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import EventColorDot from './EventColorDot.svelte';
  import EventDetail from './EventDetail.svelte';
  import EventFormDialog from './EventFormDialog.svelte';
  import NavToggle from './NavToggle.svelte';
  import type { EventJson, CalendarJson, CalDavAccountJson } from '../types/calendar.ts';
  import { dateFromEventValue } from '../lib/format.ts';
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
    // Timed values are parsed as instants (or floating local wall clock) so
    // the shown date is the viewer-local one; date-only values stay literal.
    if (dtstart.includes('T') && dtstart.includes('-')) {
      const d = dateFromEventValue(dtstart);
      if (d) return d.toLocaleDateString(locale, { dateStyle: 'long' });
    }
    const datePart = dtstart.split('T')[0].replace(/(\d{4})(\d{2})(\d{2})/, '$1-$2-$3');
    const [y, mo, d] = datePart.split('-').map(Number);
    if (!y || !mo || !d) return dtstart;
    return new Date(y, mo - 1, d).toLocaleDateString(locale, { dateStyle: 'long' });
  }

  function calendarName(accountId: string, href: string): string {
    return calendars.find(c => c.account_id === accountId && c.href === href)?.name ?? '';
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div data-layout="stack" data-gap="md" data-pad="sm">
  <div data-layout="row" data-gap="sm" data-wrap="none">
    <NavToggle />
    <form data-layout="row" data-gap="sm" data-wrap="none" data-grow onsubmit={(e) => { e.preventDefault(); doSearch(); }}>
      <input
        type="search"
        data-grow
        bind:value={query}
        placeholder={tl('search_placeholder')}
        aria-label={tl('search_placeholder')}
        autofocus
      />
      <button type="submit" data-icon="search">{tl('search_submit')}</button>
    </form>
  </div>

  <div data-layout="row" data-gap="sm">
    <button type="button" popovertarget="search-calendars" data-variant="outline" data-size="sm">{calBtnLabel}</button>
    <div popover id="search-calendars" data-menu>
      {#each calendars as cal (cal.account_id + cal.href)}
        <label>
          <input
            type="checkbox"
            checked={selectedHrefs.has(calendarKey(cal))}
            onchange={() => toggleCalendar(calendarKey(cal))}
          />
          <EventColorDot color={cal.color} />
          {cal.name}
        </label>
      {/each}
    </div>

    <label data-layout="row" data-gap="xs">
      <small data-muted>{tl('search_from')}</small>
      <input type="date" data-size="sm" bind:value={start} />
    </label>
    <label data-layout="row" data-gap="xs">
      <small data-muted>{tl('search_to')}</small>
      <input type="date" data-size="sm" bind:value={end} />
    </label>
  </div>

  {#if loading}
    <p aria-busy="true" aria-live="polite">{tl('search_loading')}</p>
  {:else if errorMsg}
    <p data-tone="danger" aria-live="polite">{errorMsg}</p>
  {:else if searched && results.length === 0}
    <p data-muted aria-live="polite">{tl('search_noResults')}</p>
  {:else if results.length > 0}
    <ul data-list aria-live="polite">
      {#each results as item (`${item.account_id}\0${item.calendar_href}\0${item.uid}\0${item.dtstart}`)}
        <li data-pad="none">
          <button
            type="button"
            class="result"
            data-variant="item"
            data-bar={item.color ? '' : null}
            style:--tm-tone={item.color || null}
            data-layout="stack"
            data-gap="none"
            onclick={() => eventDetail.open(item)}
          >
            <strong>{item.summary}</strong>
            <small data-muted>
              {formatDate(item.dtstart)}
              {#if item.location} · {item.location}{/if}
            </small>
            {#if item.description}
              <small data-muted data-truncate>{item.description}</small>
            {/if}
            {#if calendarName(item.account_id, item.calendar_href)}
              <small data-caption>{calendarName(item.account_id, item.calendar_href)}</small>
            {/if}
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
  /* A little more room per result than a plain item row. */
  .result { padding-block: calc(var(--tm-space-sm) + var(--tm-space-xs)); }
</style>

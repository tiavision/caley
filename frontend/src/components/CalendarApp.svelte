<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import EventFormDialog from './EventFormDialog.svelte';
  import MonthPicker from './MonthPicker.svelte';
  import EventDetail from './EventDetail.svelte';
  import type { EventJson, DayEventsJson, CalendarJson, MonthGridJson } from '../types/calendar.ts';
  import { getMonday, fmtDate, fmtYM } from '../lib/date.ts';

  // ── i18n ───────────────────────────────────────────────────────────────────
  let locale = 'en';
  $: tl = makeTl(locale);

  // ── App state ──────────────────────────────────────────────────────────────
  let calendars: CalendarJson[] = [];
  let days: DayEventsJson[] = [];
  let monthGrid: MonthGridJson | null = null;
  let eventsLoading = false;

  let selectedWeekStart: Date = getMonday(new Date());
  let gridMonth: string = fmtYM(selectedWeekStart);
  const todayStr: string = fmtDate(new Date());

  $: weekEnd = (() => { const d = new Date(selectedWeekStart); d.setDate(d.getDate() + 6); return d; })();

  $: weekRangeText = (() => {
    locale;
    const loc = locale;
    const start = selectedWeekStart;
    const end = weekEnd;
    const today = new Date();
    const cy = today.getFullYear();
    const so: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
    const eo: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
    if (start.getFullYear() !== cy) so.year = 'numeric';
    if (end.getFullYear() !== cy || start.getFullYear() !== end.getFullYear()) eo.year = 'numeric';
    return start.toLocaleDateString(loc, so) + ' – ' + end.toLocaleDateString(loc, eo);
  })();

  // ── Data fetching ──────────────────────────────────────────────────────────
  async function fetchCalendars() {
    try {
      const res = await fetch('/api/calendars');
      if (res.ok) calendars = await res.json();
    } catch {}
  }

  async function fetchEvents() {
    eventsLoading = true;
    try {
      const start = fmtDate(selectedWeekStart);
      const endDate = new Date(selectedWeekStart); endDate.setDate(endDate.getDate() + 6);
      const end = fmtDate(endDate);
      const res = await fetch(`/api/events?start=${start}&end=${end}`);
      if (res.ok) {
        days = await res.json();
        await tick();
        initRovingTabindex();
        scheduleReminders();
        if (pendingFocusUid && pendingFocusDate) {
          const uid = pendingFocusUid;
          const dateStr = pendingFocusDate;
          pendingFocusUid = null;
          pendingFocusDate = null;
          const targetDay = days.find(d => d.date_iso === dateStr);
          if (targetDay) {
            const s = new Set(expandedPastDays);
            s.add(dateStr);
            expandedPastDays = s;
            await tick();
            const eventIdx = Math.max(0, targetDay.events.findIndex(ev => ev.uid === uid));
            const dayEl = document.querySelector<HTMLElement>(`.day[data-date="${dateStr}"]`);
            if (dayEl) {
              const eventEls = dayEl.querySelectorAll<HTMLElement>('.appointment.event-trigger');
              const targetEl = eventEls[eventIdx] ?? eventEls[0];
              if (targetEl) {
                document.querySelectorAll<HTMLElement>('.appointment.event-trigger')
                  .forEach(el => el.setAttribute('tabindex', '-1'));
                targetEl.setAttribute('tabindex', '0');
                targetEl.focus();
                focusedEventDate = dateStr;
              }
            }
          }
        }
      }
    } catch {}
    eventsLoading = false;
  }

  async function fetchMonthGrid(month?: string) {
    try {
      const m = month || gridMonth;
      const res = await fetch(`/api/calendar?month=${m}`);
      if (res.ok) monthGrid = await res.json();
    } catch {}
  }

  // ── SSE ────────────────────────────────────────────────────────────────────
  let sseSource: EventSource | null = null;
  let sseReconnectTimer: ReturnType<typeof setTimeout> | null = null;

  function setupSSE() {
    sseSource = new EventSource('/api/events/stream');
    sseSource.onopen = () => { fetchCalendars(); fetchEvents(); fetchMonthGrid(); };
    sseSource.addEventListener('events_refetch', () => { fetchEvents(); fetchMonthGrid(); });
    sseSource.addEventListener('calendars_changed', () => { fetchCalendars(); fetchMonthGrid(); });
    sseSource.addEventListener('event_updated', async (e: MessageEvent) => {
      const payload: { uid: string; calendar_href: string; event?: EventJson } = JSON.parse(e.data);
      const { uid } = payload;
      if (payload.event) {
        days = days.map(day => ({
          ...day,
          events: day.events.filter(ev => ev.uid !== uid),
        }));
        const dateIso = payload.event.dtstart.substring(0, 10);
        days = days.map(day => {
          if (day.date_iso !== dateIso) return day;
          const events = [...day.events, payload.event!].sort((a, b) => {
            const aDay = a.time_range === 'All day';
            const bDay = b.time_range === 'All day';
            if (aDay !== bDay) return bDay ? 1 : -1;
            return a.time_range < b.time_range ? -1 : 1;
          });
          return { ...day, events };
        });
        fetchMonthGrid();
      } else {
        fetchEvents();
        fetchMonthGrid();
      }
    });
    sseSource.addEventListener('event_removed', (e: MessageEvent) => {
      const [uid, calendar_href] = (e.data as string).split('|');
      days = days.map(day => ({
        ...day,
        events: day.events.filter(ev => !(ev.uid === uid && ev.calendar_href === calendar_href))
      }));
    });
    sseSource.onerror = () => {
      sseSource?.close();
      sseSource = null;
      if (!sseReconnectTimer) {
        sseReconnectTimer = setTimeout(() => { sseReconnectTimer = null; setupSSE(); }, 5000);
      }
    };
  }

  // ── Navigation ─────────────────────────────────────────────────────────────
  function navigateWeek(delta: number) {
    const d = new Date(selectedWeekStart);
    d.setDate(d.getDate() + delta);
    selectedWeekStart = d;
    fetchEvents();
    const nm = fmtYM(d);
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
  }

  function selectDay(dateStr: string) {
    const [y, mo, dd] = dateStr.split('-').map(Number);
    selectedWeekStart = getMonday(new Date(y, mo - 1, dd));
    fetchEvents();
    const nm = String(y) + '-' + String(mo).padStart(2, '0');
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
  }

  function navigateToMonth(month: string) {
    gridMonth = month;
    fetchMonthGrid(month);
  }

  function navigateToday() {
    const today = new Date();
    selectedWeekStart = getMonday(today);
    fetchEvents();
    const nm = fmtYM(today);
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
  }

  // ── Event detail ───────────────────────────────────────────────────────────
  let eventDetail: EventDetail;
  let detailIsOpen = false;

  // ── Event form ─────────────────────────────────────────────────────────────
  let formDialog: EventFormDialog;
  let focusedEventDate = '';


  // ── Past days ──────────────────────────────────────────────────────────────
  function isPastDay(dateStr: string): boolean {
    const [y, mo, d] = dateStr.split('-').map(Number);
    const dt = new Date(y, mo - 1, d);
    const today = new Date(); today.setHours(0, 0, 0, 0);
    return dt < today;
  }

  let expandedPastDays = new Set<string>();
  let pendingFocusUid: string | null = null;
  let pendingFocusDate: string | null = null;
  function togglePastDay(dateStr: string) {
    const s = new Set(expandedPastDays);
    s.has(dateStr) ? s.delete(dateStr) : s.add(dateStr);
    expandedPastDays = s;
  }

  // ── Grid keyboard navigation ───────────────────────────────────────────────
  function initRovingTabindex() {
    const els = document.querySelectorAll<HTMLElement>('.appointment.event-trigger');
    let first = true;
    els.forEach(el => { el.setAttribute('tabindex', first ? '0' : '-1'); first = false; });
  }

  function handleGridKeydown(e: KeyboardEvent) {
    if (detailIsOpen) return;
    const target = e.target as HTMLElement;
    if (!target.classList?.contains('event-trigger')) return;
    const grid = e.currentTarget as HTMLElement;
    const cols = Array.from(grid.querySelectorAll<HTMLElement>('.day'))
      .map(d => Array.from(d.querySelectorAll<HTMLElement>('.appointment.event-trigger')))
      .filter(c => c.length > 0);
    if (!cols.length) return;
    let col = -1, row = -1;
    for (let c = 0; c < cols.length; c++) {
      const r = cols[c].indexOf(target);
      if (r !== -1) { col = c; row = r; break; }
    }
    if (col === -1) return;
    let nc = col, nr = row;
    if (e.key === 'ArrowLeft' && col > 0) { nc = col - 1; nr = Math.min(row, cols[nc].length - 1); }
    else if (e.key === 'ArrowRight' && col < cols.length - 1) { nc = col + 1; nr = Math.min(row, cols[nc].length - 1); }
    else if (e.key === 'ArrowUp' && row > 0) nr = row - 1;
    else if (e.key === 'ArrowDown' && row < cols[col].length - 1) nr = row + 1;
    else return;
    e.preventDefault();
    const next = cols[nc][nr];
    grid.querySelectorAll<HTMLElement>('.appointment.event-trigger').forEach(el => el.setAttribute('tabindex', '-1'));
    next.setAttribute('tabindex', '0');
    next.focus();
    focusedEventDate = next.closest<HTMLElement>('.day')?.dataset.date ?? '';
  }

  // ── Browser notifications ──────────────────────────────────────────────────
  let reminderTimeouts: ReturnType<typeof setTimeout>[] = [];

  function scheduleReminders() {
    for (const id of reminderTimeouts) clearTimeout(id);
    reminderTimeouts = [];
    if (typeof Notification === 'undefined') return;
    const now = Date.now();
    const horizon = now + 86400000;

    function proceed() {
      for (const day of days) {
        for (const ev of day.events) {
          if (!ev.alarms.length) continue;
          const toMs = (s: string) => s ? new Date(s.includes('Z') || s.includes('+') ? s : s + 'Z').getTime() : NaN;
          const startMs = toMs(ev.dtstart), endMs = toMs(ev.dtend);
          for (const alarm of ev.alarms) {
            let fireMs: number;
            if (alarm.direction === 'absolute') {
              if (!alarm.datetime) continue;
              const iso = alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
              fireMs = new Date(iso).getTime();
            } else {
              const base = (alarm.direction === 'before-end' || alarm.direction === 'after-end') ? endMs : startMs;
              if (isNaN(base)) continue;
              const sign = (alarm.direction.startsWith('before')) ? -1 : 1;
              fireMs = base + sign * parseInt(alarm.value || '0') * parseInt(alarm.unit || '1') * 60000;
            }
            if (isNaN(fireMs) || fireMs <= now || fireMs > horizon) continue;
            reminderTimeouts.push(setTimeout(() => {
              if (Notification.permission === 'granted') new Notification(ev.summary, { body: ev.time_range });
            }, fireMs - now));
          }
        }
      }
    }

    if (Notification.permission === 'granted') proceed();
    else if (Notification.permission === 'default') Notification.requestPermission().then(p => { if (p === 'granted') proceed(); });
  }

  // ── Global keyboard shortcuts ──────────────────────────────────────────────
  function handleKeydown(e: KeyboardEvent) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const active = document.activeElement as HTMLElement;
    const tag = active?.tagName?.toLowerCase();
    if (tag === 'input' || tag === 'textarea' || tag === 'select' || active?.isContentEditable) return;
    if (detailIsOpen) {
      if (e.key === 'e') { e.preventDefault(); eventDetail.triggerEdit(); }
      return;
    }
    if (e.key === 't') { e.preventDefault(); navigateToday(); }
    else if (e.key === 'n') { e.preventDefault(); navigateWeek(7); }
    else if (e.key === 'N') { e.preventDefault(); navigateWeek(28); }
    else if (e.key === 'p') { e.preventDefault(); navigateWeek(-7); }
    else if (e.key === 'P') { e.preventDefault(); navigateWeek(-28); }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await initI18n();
    locale = getLocale();

    const dateParam = new URLSearchParams(window.location.search).get('date');
    if (dateParam) {
      const [y, mo, d] = dateParam.split('-').map(Number);
      if (y && mo && d) {
        selectedWeekStart = getMonday(new Date(y, mo - 1, d));
        gridMonth = fmtYM(selectedWeekStart);
      }
      const uidParam = new URLSearchParams(window.location.search).get('uid');
      if (uidParam) {
        pendingFocusUid = uidParam;
        pendingFocusDate = dateParam;
      }
    }

    window.addEventListener('keydown', handleKeydown);

    setupSSE();
  });

  onDestroy(() => {
    sseSource?.close();
    if (sseReconnectTimer) clearTimeout(sseReconnectTimer);
    window.removeEventListener('keydown', handleKeydown);
    for (const id of reminderTimeouts) clearTimeout(id);
  });
</script>

<!-- ── Layout ─────────────────────────────────────────────────────────────── -->
<div class="layout">

  <!-- Sidebar: month picker + calendar list -->
  <div class="sidebar">
    <div class="calendar-picker">
      <MonthPicker
        {monthGrid}
        weekStart={selectedWeekStart}
        {weekEnd}
        {todayStr}
        {locale}
        on:selectDay={(e) => selectDay(e.detail)}
        on:navigateToMonth={(e) => navigateToMonth(e.detail)}
      />
    </div>
  </div>

  <!-- Weekly view -->
  <div class="weekly">
    <div class="weekly-header">
      <button class="nav-btn" aria-label="Previous week" on:click={() => navigateWeek(-7)}><span class="arrow">‹</span></button>
      <span class="week-range">{weekRangeText}</span>
      <button class="nav-btn" aria-label="Next week" on:click={() => navigateWeek(7)}><span class="arrow">›</span></button>
      <div class="header-actions">
        <a href="/contacts" class="contacts-btn" aria-label={tl('nav_contacts')}>
          <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>
          </svg>
        </a>
        <a href="/search" class="search-btn" aria-label={tl('nav_search')}>
          <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
          </svg>
        </a>
        <button class="today-btn" on:click={navigateToday}>{tl('cal_today')}</button>
      </div>
    </div>

    <div
      class="calendar"
      class:events-loading={eventsLoading}
      role="grid"
      aria-label="Weekly calendar"
      tabindex="0"
      style="position:relative"
      on:keydown={handleGridKeydown}
    >
      {#if eventsLoading}
        <div class="week-loading">{tl('weekly_loading')}</div>
      {/if}

      {#each days as day (day.date_iso)}
        {@const past = isPastDay(day.date_iso)}
        {@const expanded = expandedPastDays.has(day.date_iso)}
        <div
          class="day"
          class:today={day.is_today}
          class:day-past={past}
          class:day-past-empty={past && day.events.length === 0}
          class:day-expanded={expanded}
          data-date={day.date_iso}
        >
          <button type="button" class="day-heading"
            title={new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { dateStyle: 'long' })}
            on:click={() => { if (past && day.events.length > 0) togglePastDay(day.date_iso); }}>
            {new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { weekday: 'long' })}
            {#if past && day.events.length > 0 && !expanded}
              <span class="day-past-count">{day.events.length}</span>
            {/if}
            {#if past && day.events.length > 0}
              <span class="day-expand-icon" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
            {/if}
            <span class="day-date">{new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { month: 'short', day: 'numeric' })}</span>
          </button>
          <div class="appointments">
            {#each day.events as event (event.uid + event.dtstart)}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <article
                class="appointment event-trigger"
                style="border-left: 3px solid {event.color}"
                tabindex="-1"
                on:click={() => eventDetail.open(event)}
                on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); eventDetail.open(event); } }}
              >
                <div class="appt-content">
                  <span class="appt-time">{event.time_range.split('-')[0]}</span> · {event.summary}
                </div>
              </article>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  </div>
</div>

<!-- ── Event detail dialog ────────────────────────────────────────────────── -->
<EventDetail
  bind:this={eventDetail}
  bind:isOpen={detailIsOpen}
  {locale}
  on:edit={(e) => formDialog.openEditForm(e.detail.event, e.detail.scope)}
/>

<!-- ── Event form dialog ──────────────────────────────────────────────────── -->
<EventFormDialog
  bind:this={formDialog}
  {calendars}
  {locale}
  {focusedEventDate}
  on:saved={() => { fetchEvents(); fetchMonthGrid(); }}
  on:deleted={() => { fetchEvents(); fetchMonthGrid(); }}
/>


<!-- FAB -->
<button class="fab" aria-label={tl('event_new')} title={tl('event_new')} on:click={() => formDialog.openNewEventForm()}>
  <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2">
    <line x1="12" y1="5" x2="12" y2="19"></line>
    <line x1="5" y1="12" x2="19" y2="12"></line>
  </svg>
</button>

<!-- ── Styles ──────────────────────────────────────────────────────────────── -->
<style>
  /* Layout */
  .layout {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .sidebar { order: 2; }
  .weekly { order: 1; }

  @media (min-width: 600px) and (max-width: 959px) {
    .layout {
      display: grid;
      grid-template-columns: 22rem 1fr;
      grid-template-areas: "sidebar weekly";
      gap: 1rem;
      align-items: start;
    }
    .sidebar { grid-area: sidebar; display: flex; flex-direction: column; gap: 1rem; position: sticky; top: 0; }
    .weekly { grid-area: weekly; }
  }

  @media (min-width: 960px) and (max-width: 1399px) {
    .layout {
      display: grid;
      grid-template-areas: "weekly weekly" "sidebar sidebar";
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
    }
    .sidebar { grid-area: sidebar; }
    .weekly { grid-area: weekly; }
  }

  @media (min-width: 1400px) {
    .layout {
      display: grid;
      grid-template-columns: 22rem 1fr;
      grid-template-areas: "sidebar weekly";
      gap: 1rem;
      align-items: start;
    }
    .sidebar { grid-area: sidebar; display: flex; flex-direction: column; gap: 1rem; position: sticky; top: 0; }
    .weekly { grid-area: weekly; }
  }

  /* Weekly header */
  .weekly-header {
    display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; margin-bottom: 0.5rem;
    position: sticky; top: 0; z-index: 5;
    background: var(--pico-background-color);
    padding-top: 0.25rem;
  }
  .week-range { font-size: 0.875rem; color: var(--pico-muted-color); }
  .header-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .search-btn,
  .contacts-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.2rem 0.3rem;
    color: var(--pico-primary);
    text-decoration: none;
    opacity: 0.8;
  }
  .search-btn:hover,
  .contacts-btn:hover {
    opacity: 1;
  }
  .today-btn {
    margin-left: 0;
    font-size: 0.75rem;
    padding: 0.2rem 0.6rem;
    border-radius: var(--pico-border-radius);
    border: var(--pico-border-width) solid var(--pico-primary-border);
    background: transparent;
    color: var(--pico-primary);
    cursor: pointer;
    line-height: 1.4;
    margin-bottom: 0;
    transition: background 0.15s, color 0.15s;
  }
  .today-btn:hover {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    border-color: var(--pico-primary-background);
  }

  /* Weekly grid */
  .calendar {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 0.5rem;
  }
  @media (max-width: 959px) { .calendar { grid-template-columns: minmax(0, 1fr); } }

  .week-loading {
    position: absolute;
    inset: 0;
    display: none;
    align-items: flex-start;
    justify-content: center;
    padding-top: 1.5rem;
    z-index: 1;
    font-style: italic;
    color: var(--pico-muted-color);
  }
  .events-loading .week-loading { display: flex; }
  .events-loading :global(.day) { opacity: 0.3; transition: opacity 200ms ease-in; }

  /* Day columns */
  :global(.day) { min-width: 0; }
  :global(.day.today) { border-top: 2px solid var(--pico-primary-background); }
  :global(.day.today .day-heading) { color: var(--pico-primary-background); font-weight: bold; }

  .day-heading {
    background: none; border: none; margin: 0; padding: 0;
    color: inherit; font: inherit; text-align: inherit; cursor: pointer;
    font-size: 1.125rem; font-weight: 700; line-height: 1.225;
    width: 100%; display: block;
  }
  .day-heading:focus:not(:focus-visible) { outline: none; }
  .calendar:focus:not(:focus-visible) { outline: none; }
  .day-date { display: none; }
  .day-expand-icon { display: none; }
  .day-past-count { display: none; }
  :global(.appointments) { margin-top: 0.4rem; }

  @media (max-width: 959px) {
    .day-heading {
      display: flex;
      align-items: center;
      gap: 0.25rem;
    }
    .day-date {
      display: block;
      margin-left: auto;
      font-size: 0.8125rem;
      font-weight: 400;
      color: var(--pico-muted-color);
      flex-shrink: 0;
    }
    .day-expand-icon {
      font-size: 0.7em;
      color: var(--pico-muted-color);
      display: inline;
    }
    :global(.day.day-past .appointments) { display: none; }
    :global(.day.day-past.day-expanded .appointments) { display: block; }
    :global(.day.day-past .day-heading) { cursor: pointer; user-select: none; }
    :global(.day.day-past .day-heading::after) { content: none; }
    :global(.day.day-past.day-expanded .day-heading::after) { content: none; }
    :global(.day.day-past-empty) { display: none; }
    .day-past-count {
      display: inline-flex; align-items: center; justify-content: center;
      background: var(--pico-primary-background); color: var(--pico-primary-inverse);
      border-radius: 999px; font-size: 0.65em; font-weight: 700;
      min-width: 1.4em; height: 1.4em; padding: 0 0.25em;
      margin-left: 0.4em; vertical-align: middle;
    }
  }
  @media (min-width: 960px) {
    :global(.day.day-past) { opacity: 0.45; }
  }

  /* Appointments */
  :global(article.appointment) {
    margin: 0 0 0.5rem 0 !important;
    padding: 0.55rem 0.65rem !important;
    min-width: 0;
    box-shadow: 0 1px 3px rgba(0,0,0,0.12), 0 0 0 1px rgba(0,0,0,0.08);
  }
  :global(.appt-content) {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: 1.4;
  }
  :global(.appt-time) {
    color: var(--pico-muted-color);
    font-size: 0.85em;
  }
  @media (prefers-color-scheme: dark) {
    :global(article.appointment) { box-shadow: 0 1px 3px rgba(0,0,0,0.4), 0 0 0 1px rgba(255,255,255,0.08); }
  }
  :global(.event-trigger) { cursor: pointer; transition: filter 0.15s ease; }
  :global(.event-trigger:hover) { filter: brightness(0.95); }
  :global(.event-trigger:focus-visible) { outline: 2px solid var(--pico-primary-background, #0074d9); outline-offset: 2px; }


  /* FAB */
  .fab {
    position: fixed; bottom: 2rem; right: 2rem; z-index: 20;
    width: 3.5rem; height: 3.5rem; border-radius: 50%; border: none;
    background: var(--pico-primary-background); color: var(--pico-primary-inverse);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.25); transition: transform 0.15s, box-shadow 0.15s;
  }
  .fab:hover { transform: scale(1.05); box-shadow: 0 0.35rem 1.25rem rgba(0,0,0,0.3); }
</style>

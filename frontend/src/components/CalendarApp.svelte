<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';
  import EventFormDialog from './EventFormDialog.svelte';
  import MonthPicker from './MonthPicker.svelte';
  import EventDetail from './EventDetail.svelte';
  import ThreadDialog from './mail/ThreadDialog.svelte';
  import type { EventJson, DayEventsJson, CalendarJson, MonthGridJson, CalDavAccountJson, SourceEmailJson } from '../types/calendar.ts';
  import type { ThreadNode } from './mail/MailTypes.ts';
  import { getMonday, fmtDate, fmtYM } from '../lib/date.ts';
  import { apiFetch } from '../lib/api.ts';
  import MonthView from './MonthView.svelte';
  import DayEventsDialog from './DayEventsDialog.svelte';

  // ── i18n ───────────────────────────────────────────────────────────────────
  let locale = 'en';
  $: tl = makeTl(locale);

  // ── App state ──────────────────────────────────────────────────────────────
  let calendars: CalendarJson[] = [];
  let accounts: CalDavAccountJson[] = [];
  let days: DayEventsJson[] = [];
  let monthViewDays: DayEventsJson[] = [];
  let monthGrid: MonthGridJson | null = null;
  let pickerGrid: MonthGridJson | null = null;
  let eventsLoading = false;
  let monthPickerOpen = false;
  let viewMode: 'week' | 'month' = 'week';

  let selectedWeekStart: Date = getMonday(new Date());
  let gridMonth: string = fmtYM(selectedWeekStart);
  const todayStr: string = fmtDate(new Date());
  let touchStartX = 0;
  let touchStartY = 0;

  $: weekEnd = (() => { const d = new Date(selectedWeekStart); d.setDate(d.getDate() + 6); return d; })();

  $: monthRangeText = monthGrid ? monthGrid.month_name + ' ' + monthGrid.year : '';

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
      const res = await apiFetch('/api/calendars');
      if (res.ok) calendars = await res.json();
      const accountRes = await apiFetch('/api/caldav/accounts');
      if (accountRes.ok) accounts = (await accountRes.json()).accounts || [];
    } catch {}
  }

  async function fetchEvents() {
    eventsLoading = true;
    try {
      const start = fmtDate(selectedWeekStart);
      const endDate = new Date(selectedWeekStart); endDate.setDate(endDate.getDate() + 6);
      const end = fmtDate(endDate);
      const res = await apiFetch(`/api/events?start=${start}&end=${end}`);
      if (res.ok) {
        days = await res.json();
        await tick();
        initRovingTabindex();
        scheduleReminders();
      }
    } catch {}
    eventsLoading = false;
  }

  async function fetchMonthViewEvents() {
    if (!monthGrid?.days.length) return;
    eventsLoading = true;
    try {
      const start = monthGrid.days[0].date_str;
      const end = monthGrid.days[monthGrid.days.length - 1].date_str;
      const res = await apiFetch(`/api/events?start=${start}&end=${end}`);
      if (res.ok) monthViewDays = await res.json();
    } catch {}
    eventsLoading = false;
  }

  async function fetchMonthGrid(month?: string) {
    try {
      const m = month || gridMonth;
      const res = await apiFetch(`/api/calendar?month=${m}`);
      if (res.ok) {
        monthGrid = await res.json();
        if (viewMode === 'month') await fetchMonthViewEvents();
      }
    } catch {}
  }

  // ── SSE ────────────────────────────────────────────────────────────────────
  let sseSource: EventSource | null = null;
  let sseReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let sseConnectedOnce = false;

  function setupSSE() {
    sseSource = new EventSource('/api/events/stream');
    sseSource.onopen = () => {
      if (sseConnectedOnce) { fetchCalendars(); fetchEvents(); fetchMonthGrid(); }
      sseConnectedOnce = true;
    };
    sseSource.addEventListener('events_refetch', () => { fetchEvents(); fetchMonthGrid(); });
    sseSource.addEventListener('calendars_changed', () => { fetchCalendars(); fetchMonthGrid(); });
    sseSource.addEventListener('event_updated', async (e: MessageEvent) => {
      const payload: { account_id: string; uid: string; calendar_href: string; event?: EventJson } = JSON.parse(e.data);
      const { uid } = payload;
      if (payload.event) {
        days = days.map(day => ({
          ...day,
          events: day.events.filter(ev => !(ev.account_id === payload.account_id && ev.uid === uid)),
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
        const payload: { account_id: string; uid: string; calendar_href: string } = JSON.parse(e.data);
      days = days.map(day => ({
        ...day,
        events: day.events.filter(ev => !(ev.account_id === payload.account_id && ev.uid === payload.uid && ev.calendar_href === payload.calendar_href))
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
    window.scrollTo(0, 0);
  }

  function selectDay(dateStr: string) {
    monthPickerOpen = false;
    focusedEventDate = dateStr;
    const [y, mo, dd] = dateStr.split('-').map(Number);
    selectedWeekStart = getMonday(new Date(y, mo - 1, dd));
    fetchEvents();
    const nm = String(y) + '-' + String(mo).padStart(2, '0');
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
    window.scrollTo(0, 0);
  }

  function selectMonthFromDay(dateStr: string) {
    monthPickerOpen = false;
    const [y, mo] = dateStr.split('-');
    navigateToMonth(y + '-' + mo);
  }

  async function fetchPickerGrid(month: string) {
    try {
      const res = await apiFetch(`/api/calendar?month=${month}`);
      if (res.ok) pickerGrid = await res.json();
    } catch {}
  }

  function navigateToMonth(month: string) {
    gridMonth = month;
    fetchMonthGrid(month);
  }

  function navigateMonth(delta: number) {
    const [y, m] = gridMonth.split('-').map(Number);
    const d = new Date(y, m - 1 + delta, 1);
    gridMonth = fmtYM(d);
    fetchMonthGrid(gridMonth);
  }

  function switchView(mode: 'week' | 'month') {
    viewMode = mode;
    localStorage.setItem('cal_view_mode', mode);
    if (mode === 'month') {
      fetchMonthViewEvents();
    } else {
      fetchEvents();
    }
  }

  function navigateToday() {
    const today = new Date();
    selectedWeekStart = getMonday(today);
    fetchEvents();
    const nm = fmtYM(today);
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
    window.scrollTo(0, 0);
  }

  function handleTouchStart(e: TouchEvent) {
    touchStartX = e.touches[0].clientX;
    touchStartY = e.touches[0].clientY;
  }

  function handleTouchEnd(e: TouchEvent) {
    const dx = e.changedTouches[0].clientX - touchStartX;
    const dy = e.changedTouches[0].clientY - touchStartY;
    if (Math.abs(dx) < 50 || Math.abs(dx) < Math.abs(dy)) return;
    if (viewMode === 'month') navigateMonth(dx > 0 ? -1 : 1);
    else navigateWeek(dx > 0 ? -7 : 7);
  }

  // ── Event detail ───────────────────────────────────────────────────────────
  let eventDetail: EventDetail;
  let detailIsOpen = false;
  let mapUrl = 'https://www.openstreetmap.org/search?query={q}';

  // ── Thread dialog (for events imported from email) ─────────────────────────
  let threadDialog: ThreadDialog;
  let calThreadMessages: ThreadNode[] = [];
  let calMessageEvents: EventJson[] = [];
  let calSourceUids: { account_id: string; mailbox: string; uid: number }[] = [];
  let calCurrentEventUid: string | undefined = undefined;

  async function openEventThread(emails: SourceEmailJson[], event: EventJson) {
    if (!emails.length) return;
    const src = emails[0];
    calSourceUids = emails.map(e => ({ account_id: e.account_id, mailbox: e.mailbox, uid: e.uid }));
    calCurrentEventUid = event.uid;
    calMessageEvents = [];
    calThreadMessages = [];
    try {
      const [threadRes, eventsRes] = await Promise.all([
        apiFetch(`/api/email/thread?account_id=${encodeURIComponent(src.account_id)}&mailbox=${encodeURIComponent(src.mailbox)}&uid=${src.uid}`),
        apiFetch(`/api/email/message-events?account_id=${encodeURIComponent(src.account_id)}&mailbox=${encodeURIComponent(src.mailbox)}&uid=${src.uid}`),
      ]);
      if (threadRes.ok) calThreadMessages = (await threadRes.json()).messages || [];
      if (eventsRes.ok) calMessageEvents = await eventsRes.json();
    } catch { /* show empty thread */ }
    threadDialog?.show();
  }

  // ── Event form ─────────────────────────────────────────────────────────────
  let formDialog: EventFormDialog;
  let focusedEventDate = '';

  // ── Day events dialog (month view) ─────────────────────────────────────────
  let dayEventsDialog: DayEventsDialog;


  // ── Past days / collapsing ──────────────────────────────────────────────────
  function isPastDay(dateStr: string): boolean {
    const [y, mo, d] = dateStr.split('-').map(Number);
    const dt = new Date(y, mo - 1, d);
    const today = new Date(); today.setHours(0, 0, 0, 0);
    return dt < today;
  }

  function isCurrentWeekDay(dateStr: string): boolean {
    const [y, mo, d] = dateStr.split('-').map(Number);
    const dt = new Date(y, mo - 1, d);
    const today = new Date(); today.setHours(0, 0, 0, 0);
    const dow = today.getDay();
    const monday = new Date(today);
    monday.setDate(today.getDate() - (dow === 0 ? 6 : dow - 1));
    const sunday = new Date(monday);
    sunday.setDate(monday.getDate() + 6);
    return dt >= monday && dt <= sunday;
  }

  function isDefaultCollapsed(dateStr: string): boolean {
    return isPastDay(dateStr) && isCurrentWeekDay(dateStr);
  }

  let toggledDays = new Set<string>();

  function isDayCollapsed(dateStr: string): boolean {
    return isDefaultCollapsed(dateStr) !== toggledDays.has(dateStr);
  }

  function toggleDay(dateStr: string) {
    const s = new Set(toggledDays);
    s.has(dateStr) ? s.delete(dateStr) : s.add(dateStr);
    toggledDays = s;
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
    if (monthPickerOpen && e.key === 'Escape') { e.preventDefault(); monthPickerOpen = false; return; }
    if (detailIsOpen) {
      if (e.key === 'e') { e.preventDefault(); eventDetail.triggerEdit(); }
      return;
    }
    if (e.key === 'c') { e.preventDefault(); formDialog.openNewEventForm(); }
    else if (e.key === 't') { e.preventDefault(); navigateToday(); }
    else if (viewMode === 'month') {
      if (e.key === 'n') { e.preventDefault(); navigateMonth(1); }
      else if (e.key === 'N') { e.preventDefault(); navigateMonth(3); }
      else if (e.key === 'p') { e.preventDefault(); navigateMonth(-1); }
      else if (e.key === 'P') { e.preventDefault(); navigateMonth(-3); }
    } else {
      if (e.key === 'n') { e.preventDefault(); navigateWeek(7); }
      else if (e.key === 'N') { e.preventDefault(); navigateWeek(28); }
      else if (e.key === 'p') { e.preventDefault(); navigateWeek(-7); }
      else if (e.key === 'P') { e.preventDefault(); navigateWeek(-28); }
    }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await initI18n();
    locale = getLocale();

    const savedView = localStorage.getItem('cal_view_mode');
    if (savedView === 'month') viewMode = 'month';

    window.addEventListener('keydown', handleKeydown);

    getUser().then(u => { if (u.map_url) mapUrl = u.map_url; });

    setupSSE();
    fetchCalendars();
    fetchEvents();
    fetchMonthGrid();
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
  <!-- Weekly view -->
  <div class="weekly" role="application" ontouchstart={handleTouchStart} ontouchend={handleTouchEnd}>
    <div class="weekly-header">
      <div class="week-nav-wrapper">
        <button class="nav-btn"
          aria-label={viewMode === 'month' ? 'Previous month' : 'Previous week'}
          onclick={() => viewMode === 'month' ? navigateMonth(-1) : navigateWeek(-7)}>
          <span class="arrow">‹</span>
        </button>
        {#if viewMode === 'week'}
          <button class="week-range-btn" onclick={() => monthPickerOpen = !monthPickerOpen} aria-expanded={monthPickerOpen} aria-haspopup="true">
            <span class="week-range">{weekRangeText}</span>
            <span class="picker-caret" aria-hidden="true"></span>
          </button>
        {:else}
          <button class="week-range-btn" onclick={() => { if (!monthPickerOpen) pickerGrid = monthGrid; monthPickerOpen = !monthPickerOpen; }} aria-expanded={monthPickerOpen} aria-haspopup="true">
            <span class="week-range">{monthRangeText}</span>
            <span class="picker-caret" aria-hidden="true"></span>
          </button>
        {/if}
        <button class="nav-btn"
          aria-label={viewMode === 'month' ? 'Next month' : 'Next week'}
          onclick={() => viewMode === 'month' ? navigateMonth(1) : navigateWeek(7)}>
          <span class="arrow">›</span>
        </button>
        {#if monthPickerOpen}
          <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
          <div class="popover-backdrop" onclick={() => monthPickerOpen = false}></div>
          <div class="month-picker-popover">
            <MonthPicker
              monthGrid={viewMode === 'month' ? pickerGrid : monthGrid}
              weekStart={selectedWeekStart}
              {weekEnd}
              {todayStr}
              {locale}
              onselectDay={viewMode === 'week' ? (d) => selectDay(d) : (d) => selectMonthFromDay(d)}
              onnavigateToMonth={viewMode === 'week' ? (m) => navigateToMonth(m) : (m) => fetchPickerGrid(m)}
            />
          </div>
        {/if}
      </div>
      <div class="header-actions">
        <div class="view-toggle" role="group">
          <button class="view-toggle-btn" class:active={viewMode === 'week'} onclick={() => switchView('week')}>{tl('cal_view_week')}</button>
          <button class="view-toggle-btn" class:active={viewMode === 'month'} onclick={() => switchView('month')}>{tl('cal_view_month')}</button>
        </div>
        <button class="today-btn" onclick={navigateToday}>{tl('cal_today')}</button>
      </div>
    </div>

    {#if viewMode === 'week'}
      <div
        class="calendar"
        class:events-loading={eventsLoading}
        role="grid"
        aria-label="Weekly calendar"
        tabindex="0"
        style="position:relative"
        onkeydown={handleGridKeydown}
      >
        {#if eventsLoading}
          <div class="week-loading">{tl('weekly_loading')}</div>
        {/if}

        {#each days as day (day.date_iso)}
          {@const past = isPastDay(day.date_iso)}
          {@const collapsed = isDefaultCollapsed(day.date_iso) !== toggledDays.has(day.date_iso)}
          <div
            class="day"
            class:today={day.is_today}
            class:day-past={past}
            class:day-past-empty={past && day.events.length === 0}
            class:day-collapsed={collapsed}
            class:day-collapsible={day.events.length > 0}
            data-date={day.date_iso}
          >
            <div class="day-header-row">
              <button type="button" class="day-heading"
                title={new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { dateStyle: 'long' })}
                onclick={() => { focusedEventDate = day.date_iso; if (day.events.length > 0) toggleDay(day.date_iso); }}>
                {new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { weekday: 'long' })}
                {#if collapsed && day.events.length > 0}
                  <span class="day-past-count">{day.events.length}</span>
                {/if}
                {#if day.events.length > 0}
                  <span class="day-expand-icon" aria-hidden="true">{collapsed ? '▸' : '▾'}</span>
                {/if}
                <span class="day-date">{new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { month: 'short', day: 'numeric' })}</span>
              </button>
              <button type="button" class="day-add-btn" aria-label={tl('event_new')}
                onclick={() => { focusedEventDate = day.date_iso; formDialog.openNewEventForm(); }}>
                <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" aria-hidden="true">
                  <line x1="8" y1="2" x2="8" y2="14"/><line x1="2" y1="8" x2="14" y2="8"/>
                </svg>
              </button>
            </div>
            <div class="appointments">
              {#each day.events as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
                <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
                <article
                  class="appointment event-trigger"
                  style="border-left: 3px solid {event.color}"
                  tabindex="-1"
                  onclick={() => eventDetail.open(event)}
                  onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); eventDetail.open(event); } }}
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
    {:else}
      <div class="month-view-wrapper" class:events-loading={eventsLoading}>
        {#if eventsLoading}
          <div class="week-loading" style="display:flex">{tl('weekly_loading')}</div>
        {/if}
        <MonthView
          {monthGrid}
          days={monthViewDays}
          {todayStr}
          {locale}
          onEventClick={(ev) => eventDetail.open(ev)}
          onMoreClick={(date, evs) => { focusedEventDate = date; dayEventsDialog.open(date, evs); }}
        />
      </div>
    {/if}
  </div>
</div>

<!-- ── Event detail dialog ────────────────────────────────────────────────── -->
<EventDetail
  bind:this={eventDetail}
  bind:isOpen={detailIsOpen}
  {locale}
  {mapUrl}
  onedit={(d) => formDialog.openEditForm(d.event, d.scope)}
  onthreads={openEventThread}
/>

<!-- ── Thread dialog (email threads for imported events) ─────────────────── -->
<ThreadDialog
  bind:this={threadDialog}
  {locale}
  threadMessages={calThreadMessages}
  messageEvents={calMessageEvents}
  sourceUids={calSourceUids}
  currentEventUid={calCurrentEventUid}
  onmessageclick={(node) => { window.location.href = `/mail/?account_id=${encodeURIComponent(node.account_id)}&mailbox=${encodeURIComponent(node.mailbox)}&uid=${node.uid}`; }}
  oneventclick={(ev) => { threadDialog.close(); eventDetail.open(ev); }}
/>

<!-- ── Event form dialog ──────────────────────────────────────────────────── -->
<EventFormDialog
  bind:this={formDialog}
  {calendars}
  {accounts}
  {locale}
  {focusedEventDate}
  onsaved={() => { fetchEvents(); fetchMonthGrid(); }}
  ondeleted={() => { fetchEvents(); fetchMonthGrid(); }}
/>

<!-- ── Day events dialog (month view) ────────────────────────────────────── -->
<DayEventsDialog
  bind:this={dayEventsDialog}
  {locale}
  onEventClick={(ev) => eventDetail.open(ev)}
  onNewEvent={(date) => { focusedEventDate = date; formDialog.openNewEventForm(); }}
/>


<!-- FAB -->
<button class="fab" aria-label={tl('event_new')} title={tl('event_new')} onclick={() => formDialog.openNewEventForm()}>
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
    flex: 1;
    min-height: 0;
  }

  /* Weekly header */
  .weekly-header {
    display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; margin-bottom: 0.5rem;
    position: sticky; top: 0; z-index: 5;
    background: var(--pico-background-color);
  }
  .week-nav-wrapper {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .week-range-btn {
    display: flex; align-items: center; gap: 0.3rem;
    background: none; border: none; cursor: pointer;
    padding: 0.15rem 0.35rem; font: inherit; color: inherit;
    border-radius: var(--pico-border-radius);
    transition: background 0.12s; margin-bottom: 0;
  }
  .week-range-btn:hover { background: rgba(0,0,0,0.06); }
  @media (prefers-color-scheme: dark) {
    .week-range-btn:hover { background: rgba(255,255,255,0.08); }
  }
  .week-range { font-size: 0.875rem; color: var(--pico-muted-color); }
  .picker-caret { font-size: 0.6em; color: var(--pico-muted-color); opacity: 0.7; }
  .picker-caret::before { content: '▾'; }
  .popover-backdrop { position: fixed; inset: 0; z-index: 50; background: transparent; }
  .month-picker-popover {
    position: absolute; top: calc(100% + 0.5rem); left: 0; z-index: 51;
    min-width: 18rem; max-width: calc(100vw - 2rem);
    border-radius: var(--pico-border-radius);
  }
  .header-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 0.4rem;
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

  /* Weekly container */
  .weekly {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  /* Weekly grid */
  .calendar {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    gap: 0.5rem;
  }
  @media (max-width: 959px) {
    .calendar { grid-template-columns: minmax(0, 1fr); }
    .weekly { min-height: 100svh; }
  }

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

  .day-header-row {
    display: flex; align-items: center; gap: 0.25rem;
  }
  .day-heading {
    background: none; border: none; margin: 0; padding: 0;
    color: inherit; font: inherit; text-align: inherit; cursor: pointer;
    font-size: 1.125rem; font-weight: 700; line-height: 1.225;
    flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  .day-heading:focus:not(:focus-visible) { outline: none; }
  .day-add-btn {
    background: none; border: none; padding: 0.2rem 0.3rem; margin: 0;
    cursor: pointer; color: var(--pico-muted-color);
    border-radius: 4px; display: flex; align-items: center; justify-content: center;
    flex-shrink: 0; opacity: 0;
    transition: opacity 120ms, color 120ms, background 120ms;
  }
  .day-header-row:hover .day-add-btn,
  .day-add-btn:focus-visible { opacity: 1; }
  .day-add-btn:hover { color: var(--pico-primary-background); background: var(--pico-primary-hover-background, rgba(0,0,0,0.07)); }
  .calendar:focus:not(:focus-visible) { outline: none; }
  .day-date { display: none; }
  .day-expand-icon { display: none; }
  .day-past-count { display: none; }
  :global(.appointments) { margin-top: 0.4rem; }

  @media (max-width: 959px) {
    .day-add-btn { opacity: 1; }
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
    :global(.day.day-collapsed .appointments) { display: none; }
    :global(.day.day-collapsible .day-heading) { user-select: none; }
    :global(.day.day-collapsible .day-heading::after) { content: none; }
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


  /* View toggle — each button carries its own border so height matches today-btn exactly */
  .view-toggle {
    display: flex;
    margin-bottom: 0;
  }
  .view-toggle-btn {
    background: none;
    border: var(--pico-border-width) solid var(--pico-muted-border-color);
    border-radius: 0;
    padding: 0.2rem 0.65rem;
    font-size: 0.75rem;
    line-height: 1.4;
    color: var(--pico-muted-color);
    cursor: pointer;
    margin: 0;
    margin-bottom: 0;
    transition: background 0.12s, color 0.12s;
  }
  .view-toggle-btn:first-child {
    border-radius: var(--pico-border-radius) 0 0 var(--pico-border-radius);
  }
  .view-toggle-btn:last-child {
    border-radius: 0 var(--pico-border-radius) var(--pico-border-radius) 0;
  }
  .view-toggle-btn + .view-toggle-btn {
    border-left: none;
  }
  .view-toggle-btn.active {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    border-color: var(--pico-primary-background);
  }
  .view-toggle-btn:not(.active):hover {
    background: rgba(0,0,0,0.05);
    color: var(--pico-color);
  }
  @media (prefers-color-scheme: dark) {
    .view-toggle-btn:not(.active):hover { background: rgba(255,255,255,0.07); }
  }

  .month-range-label {
    cursor: default;
  }

  /* Month view wrapper */
  .month-view-wrapper {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  @media (max-width: 959px) {
    :global(main.container-fluid) {
      padding-inline: 0;
    }
    .weekly-header {
      padding-inline: var(--pico-spacing, 1rem);
    }
    /* Week view restores the padding that month view intentionally omits */
    .calendar {
      padding-inline: var(--pico-spacing, 1rem);
    }
    :global(header.nav-mobile-header) {
      margin-left: var(--pico-spacing) !important;
    }
  }

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

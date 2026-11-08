<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';
  import EventFormDialog from './EventFormDialog.svelte';
  import MonthPicker from './MonthPicker.svelte';
  import EventDetail from './EventDetail.svelte';
  import ThreadDialog from './mail/ThreadDialog.svelte';
  import type { EventJson, DayEventsJson, CalendarJson, MonthGridJson, CalDavAccountJson, SourceEmailJson, MessageEventGroupJson } from '../types/calendar.ts';
  import type { ThreadNode } from './mail/MailTypes.ts';
  import { getMonday, fmtDate, fmtYM } from '../lib/date.ts';
  import { apiFetch } from '../lib/api.ts';
  import { dateFromEventValue, eventStartDate, localDateIso, eventTimeLabel, formatEventTime } from '../lib/format.ts';
  import MonthView from './MonthView.svelte';
  import DayEventsDialog from './DayEventsDialog.svelte';
  import NavToggle from './NavToggle.svelte';

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

  // Viewer timezone, sent with event/grid requests so the backend buckets
  // days and renders time ranges in the user's local timezone.
  const userTzParam = encodeURIComponent(
    (() => { try { return Intl.DateTimeFormat().resolvedOptions().timeZone || ''; } catch { return ''; } })()
  );

  let selectedWeekStart: Date = getMonday(new Date());
  let gridMonth: string = fmtYM(selectedWeekStart);
  let todayStr: string = fmtDate(new Date());
  let midnightTimer: ReturnType<typeof setTimeout> | null = null;
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
      const res = await apiFetch(`/api/events?start=${start}&end=${end}&tz=${userTzParam}`);
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
      const res = await apiFetch(`/api/events?start=${start}&end=${end}&tz=${userTzParam}`);
      if (res.ok) monthViewDays = await res.json();
    } catch {}
    eventsLoading = false;
  }

  async function fetchMonthGrid(month?: string) {
    try {
      const m = month || gridMonth;
      const res = await apiFetch(`/api/calendar?month=${m}&tz=${userTzParam}`);
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
      if (sseConnectedOnce) { refreshToday(); fetchCalendars(); fetchEvents(); fetchMonthGrid(); }
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
        const startDate = eventStartDate(payload.event);
        const dateIso = startDate ? localDateIso(startDate) : payload.event.dtstart.substring(0, 10);
        days = days.map(day => {
          if (day.date_iso !== dateIso) return day;
          const events = [...day.events, payload.event!].sort((a, b) => {
            if (a.is_all_day !== b.is_all_day) return a.is_all_day ? -1 : 1;
            const aMs = eventStartDate(a)?.getTime() ?? 0;
            const bMs = eventStartDate(b)?.getTime() ?? 0;
            return aMs - bMs;
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
  // The scrolling pane below the toolbar (main itself does not scroll on this page).
  let paneEl: HTMLElement;
  function scrollToTop() { paneEl?.scrollTo(0, 0); }

  // Month picker popover: the opener uses popovertarget; this mirrors its state.
  let pickerEl: HTMLElement;
  function closePicker() {
    if (pickerEl?.matches(':popover-open')) pickerEl.hidePopover();
  }
  function onPickerBeforeToggle(e: ToggleEvent) {
    const opening = e.newState === 'open';
    if (opening && viewMode === 'month') pickerGrid = monthGrid;
    monthPickerOpen = opening;
  }

  function navigateWeek(delta: number) {
    const d = new Date(selectedWeekStart);
    d.setDate(d.getDate() + delta);
    selectedWeekStart = d;
    fetchEvents();
    const nm = fmtYM(d);
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
    scrollToTop();
  }

  function selectDay(dateStr: string) {
    closePicker();
    focusedEventDate = dateStr;
    const [y, mo, dd] = dateStr.split('-').map(Number);
    selectedWeekStart = getMonday(new Date(y, mo - 1, dd));
    fetchEvents();
    const nm = String(y) + '-' + String(mo).padStart(2, '0');
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
    scrollToTop();
  }

  function selectMonthFromDay(dateStr: string) {
    closePicker();
    const [y, mo] = dateStr.split('-');
    navigateToMonth(y + '-' + mo);
  }

  async function fetchPickerGrid(month: string) {
    try {
      const res = await apiFetch(`/api/calendar?month=${month}&tz=${userTzParam}`);
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

  function refreshToday() {
    const newTodayStr = fmtDate(new Date());
    if (newTodayStr !== todayStr) {
      todayStr = newTodayStr;
      // Server-derived state (weekly is_today, past-day collapsing, month
      // grid) must roll over together with the client-side highlight.
      fetchEvents();
      fetchMonthGrid();
    }
    scheduleMidnightRefresh();
  }

  function scheduleMidnightRefresh() {
    if (midnightTimer) clearTimeout(midnightTimer);
    const now = new Date();
    const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 0, 0, 5);
    midnightTimer = setTimeout(refreshToday, next.getTime() - now.getTime());
  }

  function navigateToday() {
    const today = new Date();
    selectedWeekStart = getMonday(today);
    fetchEvents();
    const nm = fmtYM(today);
    if (nm !== gridMonth) { gridMonth = nm; fetchMonthGrid(nm); }
    scrollToTop();
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
  let calEventsByMessageKey: Record<string, EventJson[]> = {};
  let calCurrentEventUid: string | undefined = undefined;

  async function openEventThread(emails: SourceEmailJson[], event: EventJson) {
    if (!emails.length) return;
    const src = emails[0];
    calCurrentEventUid = event.uid;
    calEventsByMessageKey = {};
    calThreadMessages = [];
    try {
      const [threadRes, eventsRes] = await Promise.all([
        apiFetch(`/api/email/thread?account_id=${encodeURIComponent(src.account_id)}&mailbox=${encodeURIComponent(src.mailbox)}&uid=${src.uid}`),
        apiFetch(`/api/email/thread-events?account_id=${encodeURIComponent(src.account_id)}&mailbox=${encodeURIComponent(src.mailbox)}&uid=${src.uid}`),
      ]);
      if (threadRes.ok) calThreadMessages = (await threadRes.json()).messages || [];
      if (eventsRes.ok) {
        const groups = await eventsRes.json() as MessageEventGroupJson[];
        calEventsByMessageKey = Object.fromEntries(
          groups.map(group => [
            `${group.account_id}\0${group.mailbox}\0${group.uid}`,
            group.events,
          ]),
        );
      }
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
    const els = document.querySelectorAll<HTMLElement>('.event-trigger');
    let first = true;
    els.forEach(el => { el.setAttribute('tabindex', first ? '0' : '-1'); first = false; });
  }

  function handleGridKeydown(e: KeyboardEvent) {
    if (detailIsOpen) return;
    const target = e.target as HTMLElement;
    if (!target.classList?.contains('event-trigger')) return;
    const grid = e.currentTarget as HTMLElement;
    const cols = Array.from(grid.querySelectorAll<HTMLElement>('.day'))
      .map(d => Array.from(d.querySelectorAll<HTMLElement>('.event-trigger')))
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
    grid.querySelectorAll<HTMLElement>('.event-trigger').forEach(el => el.setAttribute('tabindex', '-1'));
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
          const toMs = (s: string) => { const d = s ? dateFromEventValue(s) : null; return d ? d.getTime() : NaN; };
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
              if (Notification.permission === 'granted') new Notification(ev.summary, { body: formatEventTime(ev) });
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
    if (monthPickerOpen && e.key === 'Escape') { e.preventDefault(); closePicker(); return; }
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
    // visibilitychange misses window re-focus while the tab stayed visible,
    // and pageshow covers bfcache restores; the midnight timer alone is
    // throttled across system sleep.
    document.addEventListener('visibilitychange', refreshToday);
    window.addEventListener('focus', refreshToday);
    window.addEventListener('pageshow', refreshToday);

    getUser().then(u => { if (u.map_url) mapUrl = u.map_url; });

    setupSSE();
    scheduleMidnightRefresh();
    fetchCalendars();
    fetchEvents();
    fetchMonthGrid();
  });

  onDestroy(() => {
    sseSource?.close();
    if (sseReconnectTimer) clearTimeout(sseReconnectTimer);
    if (midnightTimer) clearTimeout(midnightTimer);
    window.removeEventListener('keydown', handleKeydown);
    document.removeEventListener('visibilitychange', refreshToday);
    window.removeEventListener('focus', refreshToday);
    window.removeEventListener('pageshow', refreshToday);
    for (const id of reminderTimeouts) clearTimeout(id);
  });
</script>

<!-- ── Layout ─────────────────────────────────────────────────────────────── -->
<div class="cal" data-layout="stack" data-gap="sm" data-pad="sm" data-grow role="application" ontouchstart={handleTouchStart} ontouchend={handleTouchEnd}>
  <div data-layout="row" data-gap="sm" data-justify="between">
    <div data-layout="row" data-gap="xs" data-wrap="none">
      <NavToggle />
      {#if viewMode === 'month'}
        <button type="button" data-variant="ghost" data-icon="chevron-left" aria-label={tl('cal_aria_prevMonth')} onclick={() => navigateMonth(-1)}></button>
        <button type="button" class="range" data-variant="ghost" popovertarget="month-picker">{monthRangeText}</button>
        <button type="button" data-variant="ghost" data-icon="chevron-right" aria-label={tl('cal_aria_nextMonth')} onclick={() => navigateMonth(1)}></button>
      {:else}
        <button type="button" data-variant="ghost" data-icon="chevron-left" aria-label={tl('cal_aria_prevWeek')} onclick={() => navigateWeek(-7)}></button>
        <button type="button" class="range" data-variant="ghost" popovertarget="month-picker">{weekRangeText}</button>
        <button type="button" data-variant="ghost" data-icon="chevron-right" aria-label={tl('cal_aria_nextWeek')} onclick={() => navigateWeek(7)}></button>
      {/if}
    </div>
    <div data-layout="row" data-gap="sm" data-wrap="none">
      <div role="group" aria-label={tl('cal_aria_view')} data-layout="row" data-gap="xs" data-wrap="none">
        <button type="button" data-variant="outline" data-size="sm" aria-pressed={viewMode === 'week'} onclick={() => switchView('week')}>{tl('cal_view_week')}</button>
        <button type="button" data-variant="outline" data-size="sm" aria-pressed={viewMode === 'month'} onclick={() => switchView('month')}>{tl('cal_view_month')}</button>
      </div>
      <button type="button" data-variant="outline" data-size="sm" onclick={navigateToday}>{tl('cal_today')}</button>
    </div>
  </div>

  <div popover id="month-picker" data-menu bind:this={pickerEl} onbeforetoggle={onPickerBeforeToggle}>
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

  <div class="pane" data-grow data-scroll="y" bind:this={paneEl}>
    {#if viewMode === 'week'}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="week"
        data-layout="grid"
        data-cols="7"
        data-gap="sm"
        role="group"
        aria-label={tl('weekly_heading')}
        aria-busy={eventsLoading}
        onkeydown={handleGridKeydown}
      >
        {#each days as day (day.date_iso)}
          {@const past = isPastDay(day.date_iso)}
          {@const collapsed = isDefaultCollapsed(day.date_iso) !== toggledDays.has(day.date_iso)}
          <div
            class="day"
            class:past
            class:past-empty={past && day.events.length === 0}
            class:collapsed={collapsed && day.events.length > 0}
            data-layout="stack"
            data-gap="xs"
            data-date={day.date_iso}
          >
            <div class="day-head" data-layout="row" data-gap="xs" data-wrap="none">
              <button type="button" class="day-heading" data-variant="ghost" data-grow
                data-layout="row" data-gap="xs" data-wrap="none" data-justify="start"
                aria-current={day.is_today ? 'date' : null}
                title={new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { dateStyle: 'long' })}
                onclick={() => { focusedEventDate = day.date_iso; if (day.events.length > 0) toggleDay(day.date_iso); }}>
                <span data-truncate>{new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { weekday: 'long' })}</span>
                {#if collapsed && day.events.length > 0}
                  <span class="narrow-only" data-badge data-tone="accent">{day.events.length}</span>
                {/if}
                {#if day.events.length > 0}
                  {#if collapsed}
                    <span class="narrow-only" data-icon="chevron-right" aria-hidden="true"></span>
                  {:else}
                    <span class="narrow-only" data-icon="chevron-down" aria-hidden="true"></span>
                  {/if}
                {/if}
                <span class="narrow-only day-date" data-muted data-grow>{new Date(day.date_iso + 'T00:00:00').toLocaleDateString(locale, { month: 'short', day: 'numeric' })}</span>
              </button>
              <button type="button" class="day-add" data-variant="ghost" data-size="sm" data-icon="plus" aria-label={tl('event_new')}
                onclick={() => { focusedEventDate = day.date_iso; formDialog.openNewEventForm(); }}></button>
            </div>
            {#if day.events.length > 0}
              <div class="events" data-list>
                {#each day.events as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
                  <button
                    type="button"
                    class="event-trigger"
                    data-variant="item"
                    data-size="sm"
                    data-bar
                    style={event.color ? `--tm-tone: ${event.color}` : null}
                    tabindex="-1"
                    onclick={() => eventDetail.open(event)}
                  >
                    <small data-muted>{eventTimeLabel(event)}</small>
                    <span data-clamp="2">{event.summary}</span>
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <div class="month" data-layout="stack" aria-busy={eventsLoading}>
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
  eventsByMessageKey={calEventsByMessageKey}
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

<button type="button" data-fab data-icon="plus" aria-label={tl('event_new')} onclick={() => formDialog.openNewEventForm()}></button>

<style>
  /* The page fills main: toolbar on top, the calendar scrolls in .pane below it. */
  .cal { min-block-size: 0; }
  .month { block-size: 100%; }
  .range { white-space: nowrap; }

  /* Week grid: seven columns, one column with collapsible days on narrow screens. */
  .day { min-inline-size: 0; }
  .day-heading { font-size: var(--tm-text-md); font-weight: 600; text-align: start; }
  .day-date { text-align: end; font-weight: 400; font-size: var(--tm-text-sm); }

  /* Compact rows in the seven columns, with a little more room than the sm item size. */
  .day .event-trigger { padding-block: var(--tm-space-sm); }

  @media (width < 60rem) {
    .week { grid-template-columns: minmax(0, 1fr); }
    .day.past-empty { display: none; }
    .day.collapsed .events { display: none; }
    /* One day per row: full-size event rows instead of the compact chips of the seven columns. */
    .day .event-trigger {
      padding: var(--tm-space-sm) var(--tm-space-md);
      font-size: var(--tm-control-font);
      gap: 0.125rem;
    }
  }
  @media (width >= 60rem) {
    .narrow-only { display: none; }
    .day.past { opacity: 0.45; }
    .day-add { opacity: 0; }
    .day-head:hover .day-add, .day-add:focus-visible { opacity: 1; }
  }
</style>

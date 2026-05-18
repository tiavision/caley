<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import { initI18n, getLocale } from '../lib/i18n.ts';
  import EventFormDialog from './EventFormDialog.svelte';

  // ── Types ──────────────────────────────────────────────────────────────────
  interface AlarmFields { direction: string; value: string; unit: string; datetime: string; }
  type PartStat = 'NEEDS-ACTION' | 'ACCEPTED' | 'DECLINED' | 'TENTATIVE';
  interface AttendeeFields { cn: string; email: string; partstat: PartStat; }
  interface RecurrenceInfo {
    freq: string; interval?: number; by_day?: string[]; by_month_day?: number[];
    by_month?: number[]; by_set_pos?: number[]; until?: string | null; count?: number | null;
  }
  interface EventJson {
    uid: string; time_range: string; summary: string; color: string;
    dtstart: string; dtend: string; is_all_day: boolean; tz: string;
    location: string; description: string; is_recurring: boolean;
    recurrence: RecurrenceInfo | null; calendar_href: string;
    master_dtstart: string; alarms: AlarmFields[]; attendees: AttendeeFields[];
  }
  interface DayEventsJson { date_iso: string; day_name: string; is_today: boolean; events: EventJson[]; }
  interface CalendarJson { href: string; name: string; color: string | null; is_active: boolean; }
  interface MonthDayJson { day_number: number; date_str: string; is_current_month: boolean; event_colors: string[]; }
  interface MonthGridJson {
    month: string; month_name: string; month_number: number; year: number;
    days: MonthDayJson[]; prev_month: string; next_month: string;
  }

  // ── i18n ───────────────────────────────────────────────────────────────────
  let locale = 'en';
  $: tl = (() => { locale; return (key: string, params?: Record<string, string | number>) => { const fn = (m as Record<string, Function>)[key]; return typeof fn === 'function' ? fn(params) : key; }; })();

  // ── App state ──────────────────────────────────────────────────────────────
  let calendars: CalendarJson[] = [];
  let days: DayEventsJson[] = [];
  let monthGrid: MonthGridJson | null = null;
  let eventsLoading = false;

  function getMonday(d: Date): Date {
    const date = new Date(d);
    const day = date.getDay();
    date.setDate(date.getDate() - day + (day === 0 ? -6 : 1));
    date.setHours(0, 0, 0, 0);
    return date;
  }

  function fmtDate(d: Date): string {
    return d.getFullYear() + '-' +
      String(d.getMonth() + 1).padStart(2, '0') + '-' +
      String(d.getDate()).padStart(2, '0');
  }

  function fmtYM(d: Date): string {
    return d.getFullYear() + '-' + String(d.getMonth() + 1).padStart(2, '0');
  }

  let selectedWeekStart: Date = getMonday(new Date());
  let gridMonth: string = fmtYM(selectedWeekStart);
  const todayStr: string = fmtDate(new Date());

  $: weekEnd = (() => { const d = new Date(selectedWeekStart); d.setDate(d.getDate() + 6); return d; })();
  $: [gridYear, gridMonthNum] = gridMonth.split('-').map(Number);

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
    const [y, m, dd] = dateStr.split('-').map(Number);
    selectedWeekStart = getMonday(new Date(y, m - 1, dd));
    fetchEvents();
    const nm = String(y) + '-' + String(m).padStart(2, '0');
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

  function onMonthSelectChange(field: 'month' | 'year', val: number) {
    if (field === 'month') {
      navigateToMonth(String(gridYear) + '-' + String(val).padStart(2, '0'));
    } else {
      navigateToMonth(String(val) + '-' + String(gridMonthNum).padStart(2, '0'));
    }
  }

  // Reactive: recalculate when week changes
  $: selectedWeekDates = (() => {
    selectedWeekStart; weekEnd;
    const set = new Set<string>();
    if (monthGrid) {
      for (const day of monthGrid.days) {
        if (day.is_current_month) {
          const [y, m, d] = day.date_str.split('-').map(Number);
          const dt = new Date(y, m - 1, d);
          dt.setHours(0, 0, 0, 0);
          if (dt.getTime() >= selectedWeekStart.getTime() && dt.getTime() <= weekEnd.getTime()) {
            set.add(day.date_str);
          }
        }
      }
    }
    return set;
  })();

  // ── Calendar toggle ────────────────────────────────────────────────────────
  async function toggleCalendar(href: string, active: boolean) {
    calendars = calendars.map(c => c.href === href ? { ...c, is_active: active } : c);
    await fetch('/api/calendar/toggle', {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: `href=${encodeURIComponent(href)}&active=${active}`,
    });
    fetchEvents();
    fetchMonthGrid();
  }

  // ── Event detail ───────────────────────────────────────────────────────────
  let detailOpen = false;
  let detailEvent: EventJson | null = null;
  let detailEditPopoverOpen = false;
  let detailDialogEl: HTMLDialogElement;

  async function openDetail(event: EventJson) {
    detailEvent = event;
    detailEditPopoverOpen = false;
    detailOpen = true;
    await tick();
    detailDialogEl?.showModal();
  }

  function closeDetail() {
    detailOpen = false;
    detailEvent = null;
    detailDialogEl?.close();
  }

  function formatEventTime(ev: EventJson): string {
    const loc = navigator.language || 'en';
    if (ev.is_all_day) {
      if (ev.dtstart) {
        const [y, monthNum, d] = ev.dtstart.split('T')[0].split('-').map(Number);
        return m.event_allDay() + ' · ' + new Date(y, monthNum - 1, d).toLocaleDateString(loc, { dateStyle: 'long' });
      }
      return m.event_allDay();
    }
    if (!ev.dtstart) return '';
    const s = new Date(ev.dtstart.includes('Z') || ev.dtstart.includes('+') ? ev.dtstart : ev.dtstart + 'Z');
    let text = s.toLocaleDateString(loc, { dateStyle: 'long' }) + ', ' +
      s.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
    if (ev.dtend) {
      const e = new Date(ev.dtend.includes('Z') || ev.dtend.includes('+') ? ev.dtend : ev.dtend + 'Z');
      if (s.toDateString() === e.toDateString()) {
        text += ' – ' + e.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
      } else {
        text += ' – ' + e.toLocaleDateString(loc, { dateStyle: 'long' }) + ', ' +
          e.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' });
      }
    }
    if (ev.tz && ev.tz !== 'UTC') text += ' (' + ev.tz + ')';
    return text;
  }

  function formatRecurrence(r: RecurrenceInfo | null): string {
    if (!r?.freq) return '';
    const loc = locale;
    const singular: Record<string, () => string> = { DAILY: m.rec_everyDay, WEEKLY: m.rec_everyWeek, MONTHLY: m.rec_everyMonth, YEARLY: m.rec_everyYear };
    const plural: Record<string, (p: Record<string, string | number>) => string> = { DAILY: m.rec_everyNDays, WEEKLY: m.rec_everyNWeeks, MONTHLY: m.rec_everyNMonths, YEARLY: m.rec_everyNYears };
    const dayMap: Record<string, number> = { MO: 0, TU: 1, WE: 2, TH: 3, FR: 4, SA: 5, SU: 6 };
    const interval = r.interval && r.interval > 1 ? r.interval : 1;
    let text = interval === 1 ? singular[r.freq]() : plural[r.freq]({ n: interval });
    if (r.by_day?.length && r.freq === 'WEEKLY') {
      const dtf = new Intl.DateTimeFormat(loc, { weekday: 'long' });
      const ref = new Date(2024, 0, 1);
      text += m.rec_on() + r.by_day.map(c => { ref.setDate(1 + (dayMap[c] ?? 0)); return dtf.format(ref); }).join(', ');
    }
    if (r.by_month_day?.length && r.freq === 'MONTHLY') text += m.rec_onDay() + r.by_month_day.join(', ');
    if (r.count) text += ', ' + r.count + m.rec_times();
    else if (r.until) {
      const parts = r.until.split(' ')[0].split('-').map(Number);
      const d = parts.length === 3 ? new Date(parts[0], parts[1] - 1, parts[2]) : new Date(r.until + 'T00:00:00');
      if (!isNaN(d.getTime())) text += m.rec_until() + d.toLocaleDateString(loc, { dateStyle: 'long' });
    }
    return text;
  }

  function formatAlarm(alarm: AlarmFields): string {
    const loc = navigator.language || 'en';
    if (alarm.direction === 'absolute') {
      if (!alarm.datetime) return '';
      const iso = alarm.datetime.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
      const d = new Date(iso);
      return isNaN(d.getTime()) ? alarm.datetime : d.toLocaleString(loc, { dateStyle: 'medium', timeStyle: 'short' });
    }
    const abs = parseInt(alarm.value || '15') * parseInt(alarm.unit || '1');
    let vs = '';
    if (abs % 1440 === 0) { const n = abs / 1440; vs = n + ' ' + (n === 1 ? m.event_reminderDay() : m.event_reminderDays()); }
    else if (abs % 60 === 0) { const n = abs / 60; vs = n + ' ' + (n === 1 ? m.event_reminderHour() : m.event_reminderHours()); }
    else vs = abs + ' ' + m.event_reminderMin();
    const amap: Record<string, string> = { 'before-start': 'event_reminderBeforeStart', 'after-start': 'event_reminderAfterStart', 'before-end': 'event_reminderBeforeEnd', 'after-end': 'event_reminderAfterEnd' };
    return vs + ' ' + (m as Record<string, Function>)[amap[alarm.direction] || 'event_reminderBeforeStart']();
  }

  const PARTSTAT_KEYS: Record<PartStat, string> = {
    'NEEDS-ACTION': 'event_attendeeStatusNeedsAction',
    'ACCEPTED': 'event_attendeeStatusAccepted',
    'DECLINED': 'event_attendeeStatusDeclined',
    'TENTATIVE': 'event_attendeeStatusTentative',
  };
  function partstatKey(ps: PartStat): string { return PARTSTAT_KEYS[ps] ?? 'event_attendeeStatusNeedsAction'; }

  // ── Event form ─────────────────────────────────────────────────────────────
  let formDialog: EventFormDialog;
  let focusedEventDate = '';

  // ── Past days ──────────────────────────────────────────────────────────────
  function isPastDay(dateStr: string): boolean {
    const [y, m, d] = dateStr.split('-').map(Number);
    const dt = new Date(y, m - 1, d);
    const today = new Date(); today.setHours(0, 0, 0, 0);
    return dt < today;
  }

  let expandedPastDays = new Set<string>();
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
    if (detailOpen) return;
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
    if (detailOpen) {
      if (e.key === 'e') {
        e.preventDefault();
        if (detailEvent) {
          if (detailEvent.is_recurring) detailEditPopoverOpen = !detailEditPopoverOpen;
          else { closeDetail(); formDialog.openEditForm(detailEvent, 'series'); }
        }
      }
      return;
    }
    if (e.key === 't') { e.preventDefault(); navigateToday(); }
    else if (e.key === 'n') { e.preventDefault(); navigateWeek(7); }
    else if (e.key === 'N') { e.preventDefault(); navigateWeek(28); }
    else if (e.key === 'p') { e.preventDefault(); navigateWeek(-7); }
    else if (e.key === 'P') { e.preventDefault(); navigateWeek(-28); }
  }

  // ── Detail dialog computed values ─────────────────────────────────────────
  $: detailTimeText = detailEvent ? formatEventTime(detailEvent) : '';
  $: detailRecText = (detailEvent?.is_recurring && detailEvent?.recurrence)
    ? formatRecurrence(detailEvent.recurrence) : '';

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await initI18n();
    locale = getLocale();

    window.addEventListener('localeChange', (e) => {
      locale = (e as CustomEvent).detail?.locale?.slice(0, 2) || getLocale();
    });
    window.addEventListener('keydown', handleKeydown);

    setupSSE();
    await Promise.all([fetchCalendars(), fetchEvents(), fetchMonthGrid()]);
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

  <!-- Sidebar: month grid + calendar list -->
  <div class="sidebar">

  <!-- Month grid -->
  <div class="calendar-picker">
    <article class="card contrast">
      {#if monthGrid}
        <div style="position:relative">
          <header>
          <hgroup>
            <h2>
              <button
                class="nav-btn"
                aria-label={tl('cal_aria_prevMonth')}
                on:click={() => navigateToMonth(monthGrid!.prev_month)}
              ><span class="arrow">‹</span></button>

              <span class="month-picker">
                <select
                  aria-label={tl('cal_aria_month')}
                  value={gridMonthNum}
                  on:change={(e) => onMonthSelectChange('month', +(e.target as HTMLSelectElement).value)}
                >
                  {#each monthNames as name, i}
                    <option value={i + 1}>{name}</option>
                  {/each}
                </select>
                <select
                  aria-label={tl('cal_aria_year')}
                  value={gridYear}
                  on:change={(e) => onMonthSelectChange('year', +(e.target as HTMLSelectElement).value)}
                >
                  {#each yearRange as y}
                    <option value={y}>{y}</option>
                  {/each}
                </select>
              </span>

              <button
                class="nav-btn"
                aria-label={tl('cal_aria_nextMonth')}
                on:click={() => navigateToMonth(monthGrid!.next_month)}
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
                  on:click={() => selectDay(day.date_str)}
                  on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectDay(day.date_str); } }}
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
  </div>

  <!-- Calendar list -->
  <div class="calendar-list-section">
<ul class="calendar-list">
      {#each calendars as cal}
        <li>
          <label>
            <input
              type="checkbox"
              checked={cal.is_active}
              on:change={(e) => toggleCalendar(cal.href, (e.target as HTMLInputElement).checked)}
            />
            {#if cal.color}
              <span class="dot" style="background:{cal.color}"></span>
            {/if}
            {cal.name}
          </label>
        </li>
      {/each}
    </ul>
  </div>

  </div>

  <!-- Weekly view -->
  <div class="weekly">
    <div class="weekly-header">
      <button class="nav-btn" aria-label="Previous week" on:click={() => navigateWeek(-7)}><span class="arrow">‹</span></button>
      <span class="week-range">{weekRangeText}</span>
      <button class="nav-btn" aria-label="Next week" on:click={() => navigateWeek(7)}><span class="arrow">›</span></button>
      <button class="today-btn" on:click={navigateToday}>{tl('cal_today')}</button>
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
                  on:click={() => openDetail(event)}
                on:keydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openDetail(event); } }}
              >
                <header>{event.time_range}</header>
                <p>{event.summary}</p>
              </article>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  </div>
</div>

<!-- ── Event detail dialog ────────────────────────────────────────────────── -->
<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={detailDialogEl}
  id="event-detail"
  on:click={(e) => { if (e.target === e.currentTarget) closeDetail(); }}
  on:close={closeDetail}
  on:cancel={(e) => { e.preventDefault(); closeDetail(); }}
>
  <article>
    <header>
      <h3>{detailEvent?.summary || ''}</h3>
      <div class="header-actions">
        {#if detailEvent}
          <button
            class="edit-btn"
            title={tl('event_edit')}
            on:click={() => {
              if (detailEvent?.is_recurring) detailEditPopoverOpen = !detailEditPopoverOpen;
              else if (detailEvent) { closeDetail(); formDialog.openEditForm(detailEvent, 'series'); }
            }}
          >&#9998;</button>
        {/if}
        <button class="close-btn" aria-label={tl('weekly_aria_close')} on:click={closeDetail}></button>
      </div>
    </header>

    {#if detailEditPopoverOpen && detailEvent}
      <div class="edit-choice-popover" style="display:flex">
        <button on:click={() => { detailEditPopoverOpen = false; if (detailEvent) { closeDetail(); formDialog.openEditForm(detailEvent, 'series'); } }}>
          {tl('event_editSeries')}
        </button>
        <button on:click={() => { detailEditPopoverOpen = false; if (detailEvent) { closeDetail(); formDialog.openEditForm(detailEvent, 'this_instance'); } }}>
          {tl('event_editInstance')}
        </button>
      </div>
    {/if}

    {#if detailEvent}
      <div class="event-detail-body">
        {#if detailTimeText}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_time')}</span>
            <span>{detailTimeText}</span>
          </div>
        {/if}
        {#if detailEvent.location}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_location')}</span>
            <a href="https://www.google.com/maps/search/?api=1&query={encodeURIComponent(detailEvent.location)}" target="_blank" rel="noopener noreferrer">{detailEvent.location}</a>
          </div>
        {/if}
        {#if detailRecText}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_repeats')}</span>
            <span>{detailRecText}</span>
          </div>
        {/if}
        {#if detailEvent.description}
          <div>
            <span class="event-detail-label">{tl('event_description')}</span>
            <p style="margin:0.25rem 0 0;white-space:pre-wrap;word-break:break-word">{detailEvent.description}</p>
          </div>
        {/if}
        {#if detailEvent.alarms.length > 0}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_reminder')}</span>
            <div>
              {#each detailEvent.alarms as alarm}
                <div>{formatAlarm(alarm)}</div>
              {/each}
            </div>
          </div>
        {/if}
        {#if detailEvent.attendees.length > 0}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_attendees')}</span>
            <div class="attendee-list-detail">
              {#each detailEvent.attendees as att}
                <div class="attendee-detail-row">
                  <span class="attendee-badge attendee-badge--{att.partstat.toLowerCase()}">{tl(partstatKey(att.partstat))}</span>
                  <span>{att.cn ? `${att.cn} <${att.email}>` : att.email}</span>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </article>
</dialog>

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
    .sidebar { grid-area: sidebar; display: flex; gap: 1rem; }
    .calendar-picker { flex: 1; }
    .calendar-list-section { flex: 1; }
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

  /* Month grid */
  .date-grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 0;
  }
  .date-header {
    text-align: center;
    font-size: 0.75rem;
    color: var(--pico-muted-color);
    padding: 0.25rem 0;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .date-cell {
    padding: 0 0.25rem;
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
  .day-heading {
    background: none; border: none; margin: 0; padding: 0;
    color: inherit; font: inherit; text-align: inherit; cursor: pointer;
    font-size: 1.125rem; font-weight: 700; line-height: 1.225;
    width: 100%; display: block;
  }
  .day-heading:focus:not(:focus-visible) { outline: none; }
  .calendar:focus:not(:focus-visible) { outline: none; }
  .event-dots { position: absolute; bottom: 0.3rem; left: 0; right: 0; display: flex; justify-content: center; gap: 3px; }
  .event-dot { width: 6px; height: 6px; border-radius: 50%; display: inline-block; flex-shrink: 0; }
  .calendar-loading { padding: 2rem; text-align: center; font-style: italic; color: var(--pico-muted-color); }

  hgroup h2 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    margin: 0 0 0.5rem;
    font-size: 0.9rem;
  }
  .nav-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--pico-border-radius);
    font-size: 1rem;
    line-height: 1;
    border: var(--pico-border-width) solid var(--pico-primary-border);
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: background 0.15s;
    margin-bottom: 0;
  }
  .nav-btn:hover { background: var(--pico-primary-hover-background); color: var(--pico-primary-inverse); }
  .arrow { display: inline-block; transform: translateY(-0.08em); }
  .month-picker { display: inline-flex; align-items: baseline; gap: 0.25rem; white-space: nowrap; flex: 1; justify-content: center; }
  .month-picker select {
    font: inherit;
    cursor: pointer;
    padding: 0.15rem 1.25rem 0.15rem 0.4rem;
    line-height: 1.4;
    margin: 0;
  }
  .month-picker select:last-of-type { max-width: 7rem; }

  /* Calendar list */
  .calendar-list { padding: 0; }
  :global(.calendar-list li) { list-style: none; }
  :global(.calendar-list input[type="checkbox"]) { cursor: pointer; }
  :global(.dot) { border-radius: 50%; width: 10px; height: 10px; display: inline-block; }

  /* Weekly header */
  .weekly-header { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; margin-bottom: 0.5rem; }
  .week-range { font-size: 0.875rem; color: var(--pico-muted-color); }
  .today-btn {
    margin-left: auto;
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
    grid-template-columns: repeat(7, 1fr);
    gap: 0.5rem;
  }
  @media (max-width: 959px) { .calendar { grid-template-columns: 1fr; } }

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
  :global(.day.today) { border-top: 2px solid var(--pico-primary-background); }
  :global(.day.today .day-heading) { color: var(--pico-primary-background); font-weight: bold; }

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
    :global(.appointments) { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem; }
    :global(.day.day-past .appointments) { display: none; }
    :global(.day.day-past.day-expanded .appointments) { display: grid; }
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
    :global(article.appointment:last-child:nth-child(odd)) { grid-column: 1 / -1; }
    :global(article.appointment p) { display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  }
  @media (min-width: 960px) {
    :global(.day.day-past) { opacity: 0.45; }
    :global(article.appointment p) { display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  }

  /* Appointments */
  :global(article.appointment) {
    margin: 0 0 0.5rem 0 !important;
    font-size: medium;
    box-shadow: 0 1px 3px rgba(0,0,0,0.12), 0 0 0 1px rgba(0,0,0,0.08);
  }
  :global(article.appointment p) { margin-bottom: 0.125rem; }
  :global(article.appointment > header) {
    font-size: small;
    color: var(--pico-muted-color);
  }
  @media (prefers-color-scheme: dark) {
    :global(article.appointment) { box-shadow: 0 1px 3px rgba(0,0,0,0.4), 0 0 0 1px rgba(255,255,255,0.08); }
  }
  :global(.event-trigger) { cursor: pointer; transition: filter 0.15s ease; }
  :global(.event-trigger:hover) { filter: brightness(0.95); }
  :global(.event-trigger:focus-visible) { outline: 2px solid var(--pico-primary-background, #0074d9); outline-offset: 2px; }

  /* Dialogs */
  :global(#event-detail),
  :global(#event-form) {
    border: 1px solid var(--pico-muted-border-color, #c8c8c8);
    border-radius: var(--pico-border-radius, 0.25rem);
    padding: 0;
    max-width: 28rem;
    width: 90vw;
    background: var(--pico-background-color, #fff);
    color: var(--pico-color, #333);
    box-shadow: 0 0.5rem 2rem rgba(0,0,0,0.25);
  }
  :global(#event-detail::backdrop),
  :global(#event-form::backdrop) { background: rgba(0,0,0,0.4); }
  :global(#event-detail article),
  :global(#event-form article) { margin: 0; padding: 1.5rem; }
  :global(#event-detail article header),
  :global(#event-form article header) { display: flex; justify-content: space-between; align-items: center; gap: 0.5rem; margin-bottom: 1rem; }
  :global(#event-detail article header h3),
  :global(#event-form article header h3) { margin: 0; flex: 1; word-break: break-word; }

  .header-actions { display: flex; align-items: center; gap: 0.25rem; }
  .edit-btn {
    background: none; border: none; cursor: pointer;
    font-size: 1rem; color: var(--pico-muted-color);
    padding: 0.25rem; line-height: 1; transition: color 0.15s;
  }
  .edit-btn:hover { color: var(--pico-primary-background); }

  :global(.close-btn) {
    position: relative; background: none; border: none;
    font-size: 1.5rem; line-height: 1; padding: 0;
    width: 1.2em; height: 1.2em; cursor: pointer;
    color: transparent; display: inline-flex; align-items: center; justify-content: center;
  }
  :global(.close-btn::before), :global(.close-btn::after) {
    content: ''; position: absolute; top: 50%; left: 50%;
    width: 0.75em; height: 0.1em;
    background-color: var(--pico-muted-color, #666); transition: background-color 0.15s;
  }
  :global(.close-btn::before) { transform: translate(-50%, -50%) rotate(45deg); }
  :global(.close-btn::after) { transform: translate(-50%, -50%) rotate(-45deg); }
  :global(.close-btn:hover::before), :global(.close-btn:hover::after) { background-color: var(--pico-color, #333); }

  .event-detail-body { margin-top: 1rem; display: flex; flex-direction: column; gap: 0.75rem; }
  .event-detail-row { display: flex; flex-direction: column; gap: 0.125rem; }
  .event-detail-label {
    font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--pico-muted-color, #666); font-weight: 600;
  }

  .edit-choice-popover {
    background: var(--pico-background-color); border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius); box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.2);
    padding: 0.25rem; flex-direction: column; gap: 0.125rem; margin: 0.25rem 0;
  }
  .edit-choice-popover button {
    background: none; border: none; text-align: left; font-size: 0.875rem;
    cursor: pointer; padding: 0.5rem 0.75rem; border-radius: var(--pico-border-radius);
    color: var(--pico-color); white-space: nowrap;
  }
  .edit-choice-popover button:hover { background: var(--pico-primary-background); color: var(--pico-primary-inverse); }

  /* Detail attendees */
  .attendee-list-detail { display: flex; flex-direction: column; gap: 0.25rem; }
  .attendee-detail-row { display: flex; align-items: center; gap: 0.5rem; }
  .attendee-badge { font-size: 0.75rem; font-weight: 600; padding: 0.1rem 0.45rem; border-radius: 999px; white-space: nowrap; }
  .attendee-badge--accepted { background: #d1fae5; color: #065f46; }
  .attendee-badge--declined { background: #fee2e2; color: #991b1b; }
  .attendee-badge--tentative { background: #fef9c3; color: #854d0e; }
  .attendee-badge--needs-action { background: var(--pico-secondary-background); color: var(--pico-secondary-inverse); opacity: 0.7; }

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

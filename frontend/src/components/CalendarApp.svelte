<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import { initI18n, getLocale } from '../lib/i18n.ts';

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
  let formMode: 'create' | 'edit' = 'create';
  let formScope: 'series' | 'this_instance' = 'series';
  let formDialogEl: HTMLDialogElement;
  let summaryInputEl: HTMLInputElement;
  let formError = '';
  let formSaving = false;

  let fUid = '', fSummary = '', fDate = '', fStartTime = '', fEndTime = '';
  let focusedEventDate = '';
  let fIsAllDay = false, fLocation = '', fDescription = '', fTz = '';
  let fOriginalDtstart = '', fOriginalCalendarHref = '', fCalendarHref = '';
  let formAlarms: AlarmFields[] = [];
  let formAttendees: AttendeeFields[] = [];

  let fFreq = '', fInterval = 1, fByDay: string[] = [];
  let fMonthlyMode: 'by_day' | 'by_setpos' = 'by_day';
  let fMonthday = 1, fSetpos = 1, fSetposDay = 'MO';
  let fEndType: 'never' | 'count' | 'until' = 'never';
  let fCount = 1, fUntil = '';

  let timeSuggestions: { label: string; h: number; m: number; prefixLen: number }[] = [];
  let timeSuggestionIdx = -1;

  $: rruleJson = (() => {
    if (!fFreq) return '';
    const obj: RecurrenceInfo & { by_set_pos?: number[] } = {
      freq: fFreq, interval: fInterval || 1,
      by_day: [], by_month_day: [], by_month: [], by_set_pos: [], until: null, count: null,
    };
    if (fFreq === 'WEEKLY') obj.by_day = [...fByDay];
    if (fFreq === 'MONTHLY') {
      if (fMonthlyMode === 'by_setpos') { obj.by_set_pos = [fSetpos]; obj.by_day = [fSetposDay]; }
      else obj.by_month_day = [fMonthday];
    }
    if (fEndType === 'count') obj.count = fCount;
    else if (fEndType === 'until') obj.until = fUntil || null;
    return JSON.stringify(obj);
  })();

  $: intervalUnit = (() => {
    const unitMap: Record<string, string> = { DAILY: m.event_repeatUnitDaily(), WEEKLY: m.event_repeatUnitWeekly(), MONTHLY: m.event_repeatUnitMonthly(), YEARLY: m.event_repeatUnitYearly() };
    return unitMap[fFreq] || '';
  })();

  function resetRecurrence() {
    fFreq = ''; fInterval = 1; fByDay = []; fMonthlyMode = 'by_day';
    fMonthday = 1; fSetpos = 1; fSetposDay = 'MO'; fEndType = 'never'; fCount = 1; fUntil = '';
  }

  function populateRecurrence(rec: RecurrenceInfo | null) {
    if (!rec?.freq) { resetRecurrence(); return; }
    fFreq = rec.freq; fInterval = rec.interval || 1;
    if (rec.freq === 'WEEKLY') fByDay = rec.by_day ? [...rec.by_day] : [];
    if (rec.freq === 'MONTHLY') {
      if (rec.by_set_pos?.length) { fMonthlyMode = 'by_setpos'; fSetpos = rec.by_set_pos[0]; fSetposDay = rec.by_day?.[0] || 'MO'; }
      else { fMonthlyMode = 'by_day'; fMonthday = rec.by_month_day?.[0] || 1; }
    }
    if (rec.count) { fEndType = 'count'; fCount = rec.count; }
    else if (rec.until) { fEndType = 'until'; fUntil = rec.until.split(' ')[0]; }
    else fEndType = 'never';
  }

  function getUserTz(): string {
    try { return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'; } catch { return 'UTC'; }
  }

  function datetimeForInput(dt: string): string {
    if (!dt) return '';
    const iso = dt.replace(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z?$/, '$1-$2-$3T$4:$5:$6Z');
    const d = new Date(iso);
    if (isNaN(d.getTime())) return '';
    return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
  }

  function autoFillEndTime() {
    if (fEndTime || !fStartTime) return;
    const [h, m] = fStartTime.split(':').map(Number);
    const total = h * 60 + m + 30;
    fEndTime = String(Math.floor(total / 60) % 24).padStart(2, '0') + ':' + String(total % 60).padStart(2, '0');
  }

  function openNewEventForm() {
    formMode = 'create'; formScope = 'series';
    fUid = ''; fSummary = ''; fDate = focusedEventDate || fmtDate(new Date());
    fIsAllDay = true; fStartTime = ''; fEndTime = '';
    fLocation = ''; fDescription = ''; fTz = getUserTz();
    fOriginalDtstart = ''; fOriginalCalendarHref = '';
    const stored = localStorage.getItem('defaultCalendar');
    const active = calendars.find(c => c.is_active);
    const fallback = active?.href || calendars[0]?.href || '';
    fCalendarHref = (stored && calendars.find(c => c.href === stored && c.is_active)) ? stored : fallback;
    resetRecurrence(); formAlarms = []; formAttendees = []; formError = '';
    timeSuggestions = []; timeSuggestionIdx = -1;
    setTimeout(() => {
      formDialogEl?.showModal();
      formDialogEl.scrollTop = 0;
      summaryInputEl?.focus();
      summaryInputEl?.select();
    }, 0);
  }

  function openEditForm(ev: EventJson, scope: 'series' | 'this_instance') {
    formMode = 'edit'; formScope = scope;
    fUid = ev.uid; fSummary = ev.summary; fLocation = ev.location;
    fDescription = ev.description; fTz = ev.tz || getUserTz();
    fOriginalDtstart = ev.dtstart; fOriginalCalendarHref = ev.calendar_href;
    fCalendarHref = ev.calendar_href; fIsAllDay = ev.is_all_day;
    let dtstart = ev.dtstart;
    if (scope === 'series' && ev.is_recurring && ev.master_dtstart) dtstart = ev.master_dtstart;
    fDate = dtstart.split('T')[0] || '';
    if (!fIsAllDay) {
      const sm = dtstart.match(/T(\d{2}:\d{2})/);
      fStartTime = sm ? sm[1] : '';
      const em = ev.dtend?.match(/T(\d{2}:\d{2})/);
      fEndTime = em ? em[1] : '';
    } else { fStartTime = ''; fEndTime = ''; }
    if (scope === 'series' && ev.is_recurring) populateRecurrence(ev.recurrence);
    else resetRecurrence();
    formAlarms = [...ev.alarms]; formAttendees = [...ev.attendees]; formError = '';
    timeSuggestions = []; timeSuggestionIdx = -1;
    closeDetail();
    setTimeout(() => {
      formDialogEl?.showModal();
      formDialogEl.scrollTop = 0;
      summaryInputEl?.focus();
      summaryInputEl?.select();
    }, 0);
  }

  async function submitEventForm() {
    formSaving = true; formError = '';
    const p = new URLSearchParams();
    p.append('uid', fUid); p.append('summary', fSummary);
    p.append('date', fDate); p.append('is_all_day', fIsAllDay ? 'true' : '');
    p.append('start_time', fStartTime); p.append('end_time', fEndTime);
    p.append('location', fLocation); p.append('description', fDescription);
    p.append('tz', fTz); p.append('scope', formScope);
    p.append('original_dtstart', fOriginalDtstart);
    p.append('original_calendar_href', fOriginalCalendarHref);
    p.append('calendar_href', fCalendarHref); p.append('recurrence', rruleJson);
    for (const a of formAlarms) {
      p.append('alarm_direction', a.direction); p.append('alarm_value', a.value);
      p.append('alarm_unit', a.unit); p.append('alarm_datetime', a.datetime);
    }
    for (const a of formAttendees) {
      p.append('attendee_cn', a.cn);
      p.append('attendee_email', a.email);
      p.append('attendee_partstat', a.partstat);
    }
    try {
      const res = await fetch('/api/event', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: p.toString(),
      });
      if (res.ok) {
        if (formMode === 'create') localStorage.setItem('defaultCalendar', fCalendarHref);
        formDialogEl?.close();
        fetchEvents(); fetchMonthGrid();
      } else {
        formError = (await res.text()) || m.event_error();
      }
    } catch { formError = m.event_error(); }
    formSaving = false;
  }

  async function deleteEvent() {
    if (!fUid || !fCalendarHref) return;
    if (!confirm(m.event_deleteConfirm())) return;
    formSaving = true;
    let url = `/api/event?uid=${encodeURIComponent(fUid)}&calendar_href=${encodeURIComponent(fCalendarHref)}`;
    if (formScope === 'this_instance' && fOriginalDtstart)
      url += `&scope=this_instance&original_dtstart=${encodeURIComponent(fOriginalDtstart)}&tz=${encodeURIComponent(fTz)}`;
    try {
      const res = await fetch(url, { method: 'DELETE' });
      if (res.ok) { formDialogEl?.close(); fetchEvents(); fetchMonthGrid(); }
    } catch {}
    formSaving = false;
  }

  // ── Time prediction ────────────────────────────────────────────────────────
  function parseTimeSuggestions(input: string) {
    const match = input.match(/^(\d{1,2})(?::(\d{0,2}))?(?:[ \t]*(am|pm))?(?=[ \t]|$)/i);
    if (!match) return [];
    const prefixLen = match[0].length;
    const h = parseInt(match[1], 10);
    const minStr = match[2] ?? '';
    const m = minStr ? parseInt(minStr.padEnd(2, '0'), 10) : 0;
    const ampm = (match[3] ?? '').toLowerCase();
    if (isNaN(h) || h > 23 || isNaN(m) || m > 59) return [];
    const make = (hour: number, min: number) => ({
      h: hour, m: min, prefixLen,
      label: new Date(2024, 0, 1, hour, min).toLocaleTimeString(navigator.language || 'en', { hour: '2-digit', minute: '2-digit' }),
    });
    if (ampm === 'am') return h <= 12 ? [make(h === 12 ? 0 : h, m)] : [];
    if (ampm === 'pm') return h <= 12 ? [make(h === 12 ? 12 : h + 12, m)] : [];
    if (h > 12) return [make(h, m)];
    if (h === 0 || h === 12) return [make(h, m)];
    return [make(h, m), make(h + 12, m)];
  }

  function applyTimeSuggestion(s: { label: string; h: number; m: number; prefixLen: number }) {
    fSummary = fSummary.slice(s.prefixLen).trimStart();
    fIsAllDay = false;
    const pad = (n: number) => String(n).padStart(2, '0');
    fStartTime = pad(s.h) + ':' + pad(s.m);
    const end = s.h * 60 + s.m + 30;
    fEndTime = pad(Math.floor(end / 60) % 24) + ':' + pad(end % 60);
    timeSuggestions = [];
    timeSuggestionIdx = -1;
    tick().then(() => summaryInputEl?.focus());
  }

  function onSummaryInput() {
    timeSuggestions = parseTimeSuggestions(fSummary);
    timeSuggestionIdx = -1;
  }

  function onSummaryKeydown(e: KeyboardEvent) {
    if (!timeSuggestions.length) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      timeSuggestionIdx = Math.min(timeSuggestionIdx + 1, timeSuggestions.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      timeSuggestionIdx = Math.max(timeSuggestionIdx - 1, -1);
    } else if (e.key === 'Enter' && timeSuggestionIdx >= 0) {
      e.preventDefault();
      applyTimeSuggestion(timeSuggestions[timeSuggestionIdx]);
    } else if (e.key === 'Tab') {
      e.preventDefault();
      applyTimeSuggestion(timeSuggestions[timeSuggestionIdx >= 0 ? timeSuggestionIdx : 0]);
    } else if (e.key === 'Escape') {
      timeSuggestions = [];
      timeSuggestionIdx = -1;
    }
  }

  function onSummaryBlur() {
    setTimeout(() => { timeSuggestions = []; timeSuggestionIdx = -1; }, 150);
  }

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
          else openEditForm(detailEvent, 'series');
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
              else if (detailEvent) openEditForm(detailEvent, 'series');
            }}
          >&#9998;</button>
        {/if}
        <button class="close-btn" aria-label={tl('weekly_aria_close')} on:click={closeDetail}></button>
      </div>
    </header>

    {#if detailEditPopoverOpen && detailEvent}
      <div class="edit-choice-popover" style="display:flex">
        <button on:click={() => { detailEditPopoverOpen = false; if (detailEvent) openEditForm(detailEvent, 'series'); }}>
          {tl('event_editSeries')}
        </button>
        <button on:click={() => { detailEditPopoverOpen = false; if (detailEvent) openEditForm(detailEvent, 'this_instance'); }}>
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

<!-- ── Event form dialog ──────────────────────────────────────────────────── -->
<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={formDialogEl}
  id="event-form"
  on:click={(e) => { if (e.target === e.currentTarget) formDialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{formMode === 'edit' ? tl('event_edit') : tl('event_create')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} on:click={() => formDialogEl?.close()}></button>
    </header>

    {#if formError}
      <div class="form-error">{formError}</div>
    {/if}

    <form on:submit|preventDefault={submitEventForm}>
      <label>
        <span>{tl('event_summary')}</span>
        <div class="summary-wrap">
          <input
            type="text" bind:value={fSummary} bind:this={summaryInputEl} required
            on:input={onSummaryInput}
            on:keydown={onSummaryKeydown}
            on:blur={onSummaryBlur}
          />
          {#if timeSuggestions.length > 0}
            <ul class="time-suggestions" role="listbox">
              {#each timeSuggestions as s, i}
                <li role="option" aria-selected={i === timeSuggestionIdx}>
                  <button
                    type="button"
                    class:active={i === timeSuggestionIdx}
                    on:mousedown|preventDefault={() => applyTimeSuggestion(s)}
                  >{s.label}</button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </label>

      <div class="event-datetime-group">
        <div class="datetime-header">
          <label>
            <span>{tl('event_date')}</span>
            <input type="date" bind:value={fDate} required />
          </label>
          <label class="all-day-label">
            <input type="checkbox" bind:checked={fIsAllDay} />
            <span>{tl('event_allDay')}</span>
          </label>
        </div>
        {#if !fIsAllDay}
          <div class="time-fields">
            <label>
              <span>{tl('event_startTime')}</span>
              <input type="time" bind:value={fStartTime} on:change={autoFillEndTime} />
            </label>
            <label>
              <span>{tl('event_endTime')}</span>
              <input type="time" bind:value={fEndTime} />
            </label>
          </div>
        {/if}
      </div>

      <label>
        <span>{tl('event_location')}</span>
        <input type="text" bind:value={fLocation} />
      </label>

      <label>
        <span>{tl('event_description')}</span>
        <textarea bind:value={fDescription} rows="3"></textarea>
      </label>

      <!-- Alarms -->
      <fieldset class="alarm-section">
        <legend>{tl('event_reminder')}</legend>
        <ul class="alarm-list">
          {#each formAlarms as alarm, i}
            <li class="alarm-row">
              {#if alarm.direction !== 'absolute'}
                <div class="alarm-relative-fields">
                  <input
                    type="number" min="1" value={alarm.value} style="width:4rem;margin-bottom:0"
                    on:input={(e) => { formAlarms[i] = { ...alarm, value: (e.target as HTMLInputElement).value }; formAlarms = formAlarms; }}
                  />
                  <select
                    value={alarm.unit} style="margin-bottom:0"
                    on:change={(e) => { formAlarms[i] = { ...alarm, unit: (e.target as HTMLSelectElement).value }; formAlarms = formAlarms; }}
                  >
                    <option value="1">{tl('event_reminderMin')}</option>
                    <option value="60">{tl('event_reminderHours')}</option>
                    <option value="1440">{tl('event_reminderDays')}</option>
                  </select>
                </div>
              {:else}
                <div class="alarm-absolute-fields">
                  <input
                    type="datetime-local" value={datetimeForInput(alarm.datetime)} style="margin-bottom:0"
                    on:change={(e) => { formAlarms[i] = { ...alarm, datetime: (e.target as HTMLInputElement).value }; formAlarms = formAlarms; }}
                  />
                </div>
              {/if}
              <select
                class="alarm-direction" value={alarm.direction} style="margin-bottom:0"
                on:change={(e) => { formAlarms[i] = { ...alarm, direction: (e.target as HTMLSelectElement).value }; formAlarms = formAlarms; }}
              >
                <option value="before-start">{tl('event_reminderBeforeStart')}</option>
                <option value="before-end">{tl('event_reminderBeforeEnd')}</option>
                <option value="after-start">{tl('event_reminderAfterStart')}</option>
                <option value="after-end">{tl('event_reminderAfterEnd')}</option>
                <option value="absolute">{tl('event_reminderAtTime')}</option>
              </select>
              <button type="button" class="alarm-remove outline" aria-label="Remove" on:click={() => { formAlarms = formAlarms.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small" on:click={() => { formAlarms = [...formAlarms, { direction: 'before-start', value: '15', unit: '1', datetime: '' }]; }}>
          {tl('event_reminderAdd')}
        </button>
      </fieldset>

      <!-- Attendees -->
      <fieldset class="alarm-section">
        <legend>{tl('event_attendees')}</legend>
        <ul class="alarm-list">
          {#each formAttendees as att, i}
            <li class="alarm-row attendee-row">
              <input
                type="text"
                placeholder={tl('event_attendeeName')}
                value={att.cn}
                style="flex:1;margin-bottom:0"
                on:input={(e) => { formAttendees[i] = { ...att, cn: (e.target as HTMLInputElement).value }; formAttendees = formAttendees; }}
              />
              <input
                type="email"
                placeholder={tl('event_attendeeEmail')}
                value={att.email}
                style="flex:2;margin-bottom:0"
                on:input={(e) => { formAttendees[i] = { ...att, email: (e.target as HTMLInputElement).value }; formAttendees = formAttendees; }}
              />
              <select
                class="attendee-partstat-select"
                value={att.partstat}
                style="margin-bottom:0"
                on:change={(e) => { formAttendees[i] = { ...att, partstat: (e.target as HTMLSelectElement).value as PartStat }; formAttendees = formAttendees; }}
              >
                <option value="NEEDS-ACTION">{tl('event_attendeeStatusNeedsAction')}</option>
                <option value="ACCEPTED">{tl('event_attendeeStatusAccepted')}</option>
                <option value="DECLINED">{tl('event_attendeeStatusDeclined')}</option>
                <option value="TENTATIVE">{tl('event_attendeeStatusTentative')}</option>
              </select>
              <button type="button" class="alarm-remove outline" aria-label={tl('event_attendeeRemove')}
                on:click={() => { formAttendees = formAttendees.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small"
          on:click={() => { formAttendees = [...formAttendees, { cn: '', email: '', partstat: 'NEEDS-ACTION' as PartStat }]; }}>
          {tl('event_attendeeAdd')}
        </button>
      </fieldset>

      <!-- Recurrence -->
      {#if formScope !== 'this_instance'}
        <label>
          <span>{tl('event_repeat')}</span>
          <select bind:value={fFreq}>
            <option value="">{tl('event_repeatNone')}</option>
            <option value="DAILY">{tl('event_repeatDaily')}</option>
            <option value="WEEKLY">{tl('event_repeatWeekly')}</option>
            <option value="MONTHLY">{tl('event_repeatMonthly')}</option>
            <option value="YEARLY">{tl('event_repeatYearly')}</option>
          </select>
        </label>
      {:else}
        <p class="repeat-instance-note">{tl('event_repeatInstanceNote')}</p>
      {/if}

      {#if fFreq}
        <div class="rrule-options">
          <div class="rrule-row">
            <span class="rrule-label">{tl('event_repeatEvery')}</span>
            <div class="rrule-controls">
              <input type="number" min="1" bind:value={fInterval} class="rrule-num-input" />
              <span class="rrule-unit">{intervalUnit}</span>
            </div>
          </div>

          {#if fFreq === 'WEEKLY'}
            <div class="rrule-row">
              <span class="rrule-label">{tl('event_repeatOn')}</span>
              <div class="weekday-toggles">
                {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
                  <label class="day-toggle">
                    <input
                      type="checkbox" value={code} checked={fByDay.includes(code)}
                      on:change={(e) => {
                        if ((e.target as HTMLInputElement).checked) fByDay = [...fByDay, code];
                        else fByDay = fByDay.filter(d => d !== code);
                      }}
                    />
                    <span>{tl(lk)}</span>
                  </label>
                {/each}
              </div>
            </div>
          {/if}

          {#if fFreq === 'MONTHLY'}
            <div class="rrule-row">
              <span class="rrule-label">{tl('event_repeatOn')}</span>
              <div class="monthly-options">
                <label class="monthly-option">
                  <input type="radio" name="monthly_mode" value="by_day" bind:group={fMonthlyMode} />
                  <span>{tl('event_monthlyByDay')}</span>
                  <input type="number" min="1" max="31" bind:value={fMonthday} class="rrule-num-input" disabled={fMonthlyMode !== 'by_day'} />
                </label>
                <label class="monthly-option">
                  <input type="radio" name="monthly_mode" value="by_setpos" bind:group={fMonthlyMode} />
                  <span>{tl('event_monthlyByPos')}</span>
                  <select bind:value={fSetpos} disabled={fMonthlyMode !== 'by_setpos'}>
                    <option value={1}>{tl('event_monthlyPos1')}</option>
                    <option value={2}>{tl('event_monthlyPos2')}</option>
                    <option value={3}>{tl('event_monthlyPos3')}</option>
                    <option value={4}>{tl('event_monthlyPos4')}</option>
                    <option value={-1}>{tl('event_monthlyPosLast')}</option>
                  </select>
                  <select bind:value={fSetposDay} disabled={fMonthlyMode !== 'by_setpos'}>
                    {#each [['MO','event_dayMo'],['TU','event_dayTu'],['WE','event_dayWe'],['TH','event_dayTh'],['FR','event_dayFr'],['SA','event_daySa'],['SU','event_daySu']] as [code, lk]}
                      <option value={code}>{tl(lk)}</option>
                    {/each}
                  </select>
                </label>
              </div>
            </div>
          {/if}

          <div class="rrule-row">
            <span class="rrule-label">{tl('event_repeatEnd')}</span>
            <div class="rrule-end-options">
              <label class="rrule-end-option">
                <input type="radio" name="rrule_end_type" value="never" bind:group={fEndType} />
                <span>{tl('event_repeatEndNever')}</span>
              </label>
              <label class="rrule-end-option">
                <input type="radio" name="rrule_end_type" value="count" bind:group={fEndType} />
                <span>{tl('event_repeatEndAfter')}</span>
                <input type="number" min="1" bind:value={fCount} class="rrule-num-input" disabled={fEndType !== 'count'} />
                <span>{tl('event_repeatEndAfterN')}</span>
              </label>
              <label class="rrule-end-option">
                <input type="radio" name="rrule_end_type" value="until" bind:group={fEndType} />
                <span>{tl('event_repeatEndOn')}</span>
                <input type="date" bind:value={fUntil} disabled={fEndType !== 'until'} />
              </label>
            </div>
          </div>
        </div>
      {/if}

      <label>
        <span>{tl('event_calendar')}</span>
        <select bind:value={fCalendarHref} disabled={formScope === 'this_instance'} on:change={() => localStorage.setItem('defaultCalendar', fCalendarHref)}>
          {#each calendars as cal}
            <option value={cal.href}>{cal.name}</option>
          {/each}
        </select>
      </label>

      <div class="event-form-actions">
        <button type="submit" disabled={formSaving}>{tl('event_save')}</button>
        <button type="button" class="secondary" disabled={formSaving} on:click={() => formDialogEl?.close()}>{tl('event_cancel')}</button>
        {#if formMode === 'edit'}
          <button type="button" class="contrast outline" disabled={formSaving} on:click={deleteEvent}>{tl('event_delete')}</button>
        {/if}
      </div>
    </form>
  </article>
</dialog>

<!-- FAB -->
<button class="fab" aria-label={tl('event_new')} title={tl('event_new')} on:click={openNewEventForm}>
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

  /* Event form */
  .summary-wrap { position: relative; }
  .time-suggestions {
    position: absolute; top: 100%; left: 0; right: 0; z-index: 10;
    margin: 0.125rem 0 0; padding: 0.25rem; list-style: none;
    background: var(--pico-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    box-shadow: 0 0.25rem 0.75rem rgba(0,0,0,0.15);
  }
  .time-suggestions li { margin: 0; }
  .time-suggestions button {
    width: 100%; text-align: left; border: none; background: none; margin: 0;
    padding: 0.375rem 0.75rem; cursor: pointer; font-size: 0.875rem;
    border-radius: calc(var(--pico-border-radius, 0.25rem) - 0.125rem);
    color: var(--pico-color); transition: background 0.1s;
  }
  .time-suggestions button:hover,
  .time-suggestions button.active {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
  }

  :global(#event-form form label) { display: flex; flex-direction: column; gap: 0.125rem; margin-bottom: 0.75rem; }
  :global(#event-form form label.all-day-label) { flex-direction: row; align-items: center; gap: 0.5rem; }
  :global(#event-form input:not([type=checkbox],[type=radio])),
  :global(#event-form select),
  :global(#event-form textarea) { margin-bottom: 0; }

  .event-datetime-group { border-left: 3px solid var(--pico-primary-background); padding-left: 0.75rem; margin-bottom: 0.75rem; }
  .datetime-header { display: flex; align-items: flex-end; gap: 1rem; margin-bottom: 0.5rem; flex-wrap: wrap; }
  .datetime-header label:first-child { flex: 1; }
  .event-datetime-group label { margin-bottom: 0 !important; }
  .time-fields { display: flex; gap: 0.75rem; }
  .time-fields label { flex: 1; }

  .form-error { color: var(--pico-del-color); font-size: 0.875rem; margin-bottom: 0.75rem; }

  .event-form-actions { display: flex; gap: 0.5rem; margin-top: 1rem; }
  .event-form-actions :global(.contrast.outline) { margin-left: auto; }

  /* Alarms */
  .alarm-section { border: none; border-left: 3px solid var(--pico-primary-background); padding: 0 0 0 0.75rem; margin: 0 0 0.75rem; }
  :global(.alarm-section legend) { font-size: 0.8125rem; text-transform: uppercase; letter-spacing: 0.04em; font-weight: 600; color: var(--pico-muted-color); margin-bottom: 0.375rem; }
  .alarm-list { list-style: none; padding: 0; margin: 0 0 0.5rem; display: flex; flex-direction: column; gap: 0.5rem; }
  .alarm-row { display: flex; align-items: center; gap: 0.5rem; }
  .attendee-row { flex-wrap: nowrap; }
  .attendee-row input { min-width: 0; }
  :global(.attendee-partstat-select) { flex: 0 0 auto; width: 7.5rem; margin-bottom: 0 !important; padding-left: 0.4rem; padding-right: 0.4rem; }
  .attendee-list-detail { display: flex; flex-direction: column; gap: 0.25rem; }
  .attendee-detail-row { display: flex; align-items: center; gap: 0.5rem; }
  .attendee-badge { font-size: 0.75rem; font-weight: 600; padding: 0.1rem 0.45rem; border-radius: 999px; white-space: nowrap; }
  .attendee-badge--accepted { background: #d1fae5; color: #065f46; }
  .attendee-badge--declined { background: #fee2e2; color: #991b1b; }
  .attendee-badge--tentative { background: #fef9c3; color: #854d0e; }
  .attendee-badge--needs-action { background: var(--pico-secondary-background); color: var(--pico-secondary-inverse); opacity: 0.7; }
  .alarm-relative-fields { display: flex; align-items: center; gap: 0.25rem; }
  .alarm-absolute-fields { flex: 1; }
  :global(.alarm-direction) { flex: 1 1 0; min-width: 0; margin-bottom: 0 !important; }
  :global(.alarm-remove) { font-size: 1rem; line-height: 1; margin-bottom: 0 !important; margin-left: auto; }
  :global(button.small) { font-size: 0.875rem; padding: 0.25rem 0.75rem; }

  /* Recurrence */
  .rrule-options { border-left: 3px solid var(--pico-primary-background); padding-left: 0.75rem; margin-bottom: 0.75rem; display: flex; flex-direction: column; gap: 0.625rem; }
  .rrule-row { display: flex; align-items: flex-start; gap: 0.75rem; }
  .rrule-label { font-size: 0.8125rem; text-transform: uppercase; letter-spacing: 0.04em; color: var(--pico-muted-color); font-weight: 600; width: 3.25rem; flex-shrink: 0; padding-top: 0.45rem; }
  .rrule-controls { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  :global(.rrule-num-input) { width: 4rem !important; margin-bottom: 0 !important; }
  .rrule-unit { font-size: 0.9375rem; }

  .weekday-toggles { display: flex; gap: 0.3rem; flex-wrap: wrap; padding-top: 0.1rem; }
  .day-toggle { position: relative; margin-bottom: 0 !important; display: inline-flex !important; flex-direction: row !important; align-items: center !important; }
  .day-toggle input[type="checkbox"] { position: absolute; opacity: 0; width: 1px; height: 1px; overflow: hidden; }
  .day-toggle span { display: flex; align-items: center; justify-content: center; width: 2.125rem; height: 2.125rem; border-radius: 50%; border: 1.5px solid var(--pico-muted-border-color); font-size: 0.7rem; font-weight: 600; cursor: pointer; user-select: none; transition: background 0.15s, border-color 0.15s, color 0.15s; color: var(--pico-color); background: transparent; letter-spacing: 0; }
  .day-toggle input[type="checkbox"]:checked + span { background: var(--pico-primary-background); border-color: var(--pico-primary-background); color: var(--pico-primary-inverse); }
  .day-toggle span:hover { border-color: var(--pico-primary-background); color: var(--pico-primary-background); }
  .day-toggle input[type="checkbox"]:focus-visible + span { outline: 2px solid var(--pico-primary-background); outline-offset: 2px; }

  .monthly-options { display: flex; flex-direction: column; gap: 0.375rem; }
  .monthly-option { display: flex !important; align-items: center !important; flex-direction: row !important; gap: 0.4rem; margin-bottom: 0 !important; flex-wrap: wrap; }
  .monthly-option input[type="radio"] { flex-shrink: 0; margin: 0; }
  :global(.monthly-option select), :global(.monthly-option .rrule-num-input) { margin-bottom: 0 !important; }

  .rrule-end-options { display: flex; flex-direction: column; gap: 0.375rem; }
  .rrule-end-option { display: flex !important; align-items: center !important; flex-direction: row !important; gap: 0.4rem; margin-bottom: 0 !important; }
  .rrule-end-option input[type="radio"] { flex-shrink: 0; margin: 0; }
  :global(.rrule-end-option input[type="number"]) { width: 4rem !important; margin-bottom: 0 !important; }
  :global(.rrule-end-option input[type="date"]) { flex: 1; min-width: 8rem; margin-bottom: 0 !important; }
  :global(.rrule-end-option input:disabled) { opacity: 0.4; cursor: not-allowed; }

  .repeat-instance-note { font-size: 0.8125rem; color: var(--pico-muted-color); margin: 0 0 0.75rem; font-style: italic; }

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

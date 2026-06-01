<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { PartStat, AlarmFields, AttendeeFields, RecurrenceInfo, EventJson, CalendarJson, ContactJson, CalDavAccountJson } from '../types/calendar.ts';
  import { fmtDate } from '../lib/date.ts';
  import { apiFetch } from '../lib/api.ts';
  import RecurrenceBuilder from './RecurrenceBuilder.svelte';

  export let calendars: CalendarJson[] = [];
  export let accounts: CalDavAccountJson[] = [];
  export let locale: string = 'en';
  export let focusedEventDate: string = '';
  export let onsaved: (() => void) | undefined = undefined;
  export let ondeleted: (() => void) | undefined = undefined;

  $: tl = makeTl(locale);

  let formMode: 'create' | 'edit' = 'create';
  let formScope: 'series' | 'this_instance' = 'series';
  let formDialogEl: HTMLDialogElement;
  let summaryInputEl: HTMLInputElement;
  let formError = '';
  let formSaving = false;

  let fUid = '', fSummary = '', fDate = '', fStartTime = '', fEndTime = '';
  let fIsAllDay = false, fLocation = '', fDescription = '', fTz = '';
  let fOriginalDtstart = '', fOriginalCalendarHref = '', fOriginalAccountId = '', fCalendarHref = '', fAccountId = '';
  let formAlarms: AlarmFields[] = [];
  let formAttendees: AttendeeFields[] = [];

  let fFreq = '', fInterval = 1, fByDay: string[] = [];
  let fMonthlyMode: 'by_day' | 'by_setpos' = 'by_day';
  let fMonthday = 1, fSetpos = 1, fSetposDay = 'MO';
  let fEndType: 'never' | 'count' | 'until' = 'never';
  let fCount = 1, fUntil = '';

  let timeSuggestions: { label: string; h: number; m: number; prefixLen: number }[] = [];
  let timeSuggestionIdx = -1;
  let daySuggestions: { label: string; date: string; prefixLen: number }[] = [];
  let daySuggestionIdx = -1;

  let allContacts: ContactJson[] = [];
  let contactsLoaded = false;
  let attendeeSuggestions: { fn_name: string; email: string }[] = [];
  let attendeeSuggestionIdx = -1;
  let attendeeSuggestionRow = -1;

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

  $: calendarGroups = (() => {
    const seen = new Set<string>();
    return calendars
      .map(cal => {
        if (seen.has(cal.account_id)) return null;
        seen.add(cal.account_id);
        return {
          account_id: cal.account_id,
          label: accounts.find(a => a.account_id === cal.account_id)?.display_name || cal.account_id,
        };
      })
      .filter((group): group is { account_id: string; label: string } => group !== null);
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
    const [h, min] = fStartTime.split(':').map(Number);
    const total = h * 60 + min + 30;
    fEndTime = String(Math.floor(total / 60) % 24).padStart(2, '0') + ':' + String(total % 60).padStart(2, '0');
  }

  export function openNewEventForm() {
    loadContacts();
    formMode = 'create'; formScope = 'series';
    fUid = ''; fSummary = ''; fDate = focusedEventDate || fmtDate(new Date());
    fIsAllDay = true; fStartTime = ''; fEndTime = '';
    fLocation = ''; fDescription = ''; fTz = getUserTz();
    fOriginalDtstart = ''; fOriginalCalendarHref = ''; fOriginalAccountId = '';
    const stored = localStorage.getItem('defaultCalendar');
    const active = calendars.find(c => c.is_active);
    const fallback = active || calendars[0];
    const storedCal = stored ? calendars.find(c => `${c.account_id}\0${c.href}` === stored && c.is_active) : null;
    const selected = storedCal || fallback;
    fAccountId = selected?.account_id || '';
    fCalendarHref = selected?.href || '';
    resetRecurrence(); formAlarms = []; formAttendees = []; formError = '';
    timeSuggestions = []; timeSuggestionIdx = -1;
    setTimeout(() => {
      formDialogEl?.showModal();
      formDialogEl.scrollTop = 0;
      summaryInputEl?.focus();
      summaryInputEl?.select();
    }, 0);
  }

  export function openEditForm(ev: EventJson, scope: 'series' | 'this_instance') {
    loadContacts();
    formMode = 'edit'; formScope = scope;
    fUid = ev.uid; fSummary = ev.summary; fLocation = ev.location;
    fDescription = ev.description; fTz = ev.tz || getUserTz();
    fOriginalDtstart = ev.dtstart; fOriginalCalendarHref = ev.calendar_href; fOriginalAccountId = ev.account_id;
    fAccountId = ev.account_id; fCalendarHref = ev.calendar_href; fIsAllDay = ev.is_all_day;
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
    p.append('original_account_id', fOriginalAccountId);
    p.append('account_id', fAccountId);
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
      const res = await apiFetch('/api/event', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: p.toString(),
      });
      if (res.ok) {
        if (formMode === 'create') localStorage.setItem('defaultCalendar', `${fAccountId}\0${fCalendarHref}`);
        formDialogEl?.close();
        onsaved?.();
      } else {
        formError = (await res.text()) || tl('event_error');
      }
    } catch { formError = tl('event_error'); }
    formSaving = false;
  }

  async function deleteEvent() {
    if (!fUid || !fCalendarHref) return;
    if (!confirm(tl('event_deleteConfirm'))) return;
    formSaving = true;
    let url = `/api/event?account_id=${encodeURIComponent(fAccountId)}&uid=${encodeURIComponent(fUid)}&calendar_href=${encodeURIComponent(fCalendarHref)}`;
    if (formScope === 'this_instance' && fOriginalDtstart)
      url += `&scope=this_instance&original_dtstart=${encodeURIComponent(fOriginalDtstart)}&tz=${encodeURIComponent(fTz)}`;
    try {
      const res = await apiFetch(url, { method: 'DELETE' });
      if (res.ok) { formDialogEl?.close(); ondeleted?.(); }
      else { formError = (await res.text()) || tl('event_error'); }
    } catch { formError = tl('event_error'); }
    formSaving = false;
  }

  function parseTimeSuggestions(input: string) {
    const match = input.match(/^(\d{1,2})(?::(\d{0,2}))?(?:[ \t]*(am|pm))?(?=[ \t]|$)/i);
    if (!match) return [];
    const prefixLen = match[0].length;
    const h = parseInt(match[1], 10);
    const minStr = match[2] ?? '';
    const min = minStr ? parseInt(minStr.padEnd(2, '0'), 10) : 0;
    const ampm = (match[3] ?? '').toLowerCase();
    if (isNaN(h) || h > 23 || isNaN(min) || min > 59) return [];
    const make = (hour: number, minute: number) => ({
      h: hour, m: minute, prefixLen,
      label: new Date(2024, 0, 1, hour, minute).toLocaleTimeString(navigator.language || 'en', { hour: '2-digit', minute: '2-digit' }),
    });
    if (ampm === 'am') return h <= 12 ? [make(h === 12 ? 0 : h, min)] : [];
    if (ampm === 'pm') return h <= 12 ? [make(h === 12 ? 12 : h + 12, min)] : [];
    if (h > 12) return [make(h, min)];
    if (h === 0 || h === 12) return [make(h, min)];
    return [make(h, min), make(h + 12, min)];
  }

  function parseDaySuggestion(input: string, loc: string): { label: string; date: string; prefixLen: number }[] {
    const rtf = new Intl.RelativeTimeFormat(loc, { numeric: 'auto' });
    const todayWord = rtf.format(0, 'day').toLowerCase();
    const tomorrowWord = rtf.format(1, 'day').toLowerCase();
    // Jan 7 2024 = Sunday (dayIndex 0), Jan 8 = Monday (dayIndex 1), …, Jan 13 = Saturday (dayIndex 6)
    const dayNames: { name: string; dayIndex: number }[] = [];
    for (let i = 0; i < 7; i++) {
      const date = new Date(2024, 0, 7 + i);
      const long = date.toLocaleDateString(loc, { weekday: 'long' }).toLowerCase();
      const short = date.toLocaleDateString(loc, { weekday: 'short' }).toLowerCase().replace(/\.$/, '');
      dayNames.push({ name: long, dayIndex: i });
      if (short !== long) dayNames.push({ name: short, dayIndex: i });
    }
    const combined: { name: string; dayIndex: number }[] = [
      { name: todayWord, dayIndex: -1 },
      { name: tomorrowWord, dayIndex: -2 },
      ...dayNames,
    ];

    const lower = input.toLowerCase();
    let wordLen = lower.length;
    let hasMore = false;
    for (let i = 0; i < lower.length; i++) {
      if (lower[i] === ' ' || lower[i] === '\t') {
        wordLen = i;
        hasMore = lower.slice(i + 1).trimStart().length > 0;
        break;
      }
    }
    const typedWord = lower.slice(0, wordLen);
    if (!typedWord || hasMore) return [];

    const today = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    const makeEntry = (daysAhead: number) => {
      const target = new Date(today.getFullYear(), today.getMonth(), today.getDate() + daysAhead);
      const dateStr = `${target.getFullYear()}-${pad(target.getMonth() + 1)}-${pad(target.getDate())}`;
      const label = target.toLocaleDateString(navigator.language || 'en', { weekday: 'long', month: 'short', day: 'numeric' });
      return { label, date: dateStr, prefixLen: wordLen };
    };

    const seen = new Set<number>();
    const results: { label: string; date: string; prefixLen: number }[] = [];
    for (const { name, dayIndex } of combined) {
      if (!name.startsWith(typedWord)) continue;
      if (dayIndex >= 0 && seen.has(dayIndex)) continue;
      if (dayIndex >= 0) seen.add(dayIndex);
      let daysAhead = 0;
      if (dayIndex === -2) {
        daysAhead = 1;
      } else if (dayIndex >= 0) {
        daysAhead = (dayIndex - today.getDay() + 7) % 7;
        if (daysAhead === 0) daysAhead = 7;
      }
      results.push(makeEntry(daysAhead));
    }
    return results;
  }

  function applyDaySuggestion(s: { label: string; date: string; prefixLen: number }) {
    fSummary = fSummary.slice(s.prefixLen).trimStart();
    fDate = s.date;
    daySuggestions = [];
    daySuggestionIdx = -1;
    tick().then(() => summaryInputEl?.focus());
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
    daySuggestions = timeSuggestions.length === 0 ? parseDaySuggestion(fSummary, locale) : [];
    timeSuggestionIdx = -1;
    daySuggestionIdx = -1;
  }

  function onSummaryKeydown(e: KeyboardEvent) {
    if (!timeSuggestions.length && !daySuggestions.length) return;
    if (timeSuggestions.length) {
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
    } else {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        daySuggestionIdx = Math.min(daySuggestionIdx + 1, daySuggestions.length - 1);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        daySuggestionIdx = Math.max(daySuggestionIdx - 1, -1);
      } else if (e.key === 'Enter' && daySuggestionIdx >= 0) {
        e.preventDefault();
        applyDaySuggestion(daySuggestions[daySuggestionIdx]);
      } else if (e.key === 'Tab') {
        e.preventDefault();
        applyDaySuggestion(daySuggestions[daySuggestionIdx >= 0 ? daySuggestionIdx : 0]);
      } else if (e.key === 'Escape') {
        daySuggestions = [];
        daySuggestionIdx = -1;
      }
    }
  }

  function onSummaryBlur() {
    setTimeout(() => {
      timeSuggestions = []; timeSuggestionIdx = -1;
      daySuggestions = []; daySuggestionIdx = -1;
    }, 150);
  }

  async function loadContacts() {
    if (contactsLoaded) return;
    try {
      const res = await apiFetch('/api/contacts');
      if (res.ok) {
        const data = await res.json();
        allContacts = data.contacts ?? [];
      }
    } catch {}
    contactsLoaded = true;
  }

  function filterContacts(query: string): { fn_name: string; email: string }[] {
    const q = query.trim().toLowerCase();
    if (!q || allContacts.length === 0) return [];
    const results: { fn_name: string; email: string }[] = [];
    for (const c of allContacts) {
      const nameMatch = c.fn_name.toLowerCase().includes(q);
      const emailMatch = c.emails.some(e => e.value.toLowerCase().includes(q));
      if (nameMatch || emailMatch) {
        const bestEmail = c.emails.find(e => e.value.toLowerCase().includes(q))?.value
                       ?? c.emails[0]?.value ?? '';
        if (bestEmail) results.push({ fn_name: c.fn_name, email: bestEmail });
      }
    }
    return results.slice(0, 8);
  }

  function onAttendeeInput(i: number, value: string) {
    attendeeSuggestionRow = i;
    attendeeSuggestions = filterContacts(value);
    attendeeSuggestionIdx = -1;
  }

  function onAttendeeKeydown(e: KeyboardEvent) {
    if (!attendeeSuggestions.length) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      attendeeSuggestionIdx = Math.min(attendeeSuggestionIdx + 1, attendeeSuggestions.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      attendeeSuggestionIdx = Math.max(attendeeSuggestionIdx - 1, -1);
    } else if (e.key === 'Enter' && attendeeSuggestionIdx >= 0) {
      e.preventDefault();
      applyAttendeeSuggestion(attendeeSuggestions[attendeeSuggestionIdx]);
    } else if (e.key === 'Tab') {
      if (attendeeSuggestionIdx >= 0) {
        e.preventDefault();
        applyAttendeeSuggestion(attendeeSuggestions[attendeeSuggestionIdx]);
      } else if (attendeeSuggestions.length === 1) {
        e.preventDefault();
        applyAttendeeSuggestion(attendeeSuggestions[0]);
      }
    } else if (e.key === 'Escape') {
      attendeeSuggestions = [];
      attendeeSuggestionIdx = -1;
    }
  }

  function applyAttendeeSuggestion(s: { fn_name: string; email: string }) {
    const i = attendeeSuggestionRow;
    formAttendees[i] = { ...formAttendees[i], cn: s.fn_name, email: s.email };
    formAttendees = formAttendees;
    attendeeSuggestions = [];
    attendeeSuggestionIdx = -1;
    attendeeSuggestionRow = -1;
  }

  function onAttendeeBlur() {
    setTimeout(() => {
      attendeeSuggestions = [];
      attendeeSuggestionIdx = -1;
    }, 150);
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={formDialogEl}
  id="event-form"
  onclick={(e) => { if (e.target === e.currentTarget) formDialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{formMode === 'edit' ? tl('event_edit') : tl('event_create')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => formDialogEl?.close()}></button>
    </header>

    {#if formError}
      <div class="form-error">{formError}</div>
    {/if}

    <form onsubmit={(e) => { e.preventDefault(); submitEventForm(); }}>
      <label>
        <span>{tl('event_summary')}</span>
        <div class="summary-wrap">
          <input
            type="text" bind:value={fSummary} bind:this={summaryInputEl} required
            oninput={onSummaryInput}
            onkeydown={onSummaryKeydown}
            onblur={onSummaryBlur}
          />
          {#if timeSuggestions.length > 0}
            <ul class="time-suggestions" role="listbox">
              {#each timeSuggestions as s, i}
                <li role="option" aria-selected={i === timeSuggestionIdx}>
                  <button
                    type="button"
                    class:active={i === timeSuggestionIdx}
                    onmousedown={(e) => { e.preventDefault(); applyTimeSuggestion(s); }}
                  >{s.label}</button>
                </li>
              {/each}
            </ul>
          {/if}
          {#if daySuggestions.length > 0}
            <ul class="time-suggestions" role="listbox">
              {#each daySuggestions as s, i}
                <li role="option" aria-selected={i === daySuggestionIdx}>
                  <button
                    type="button"
                    class:active={i === daySuggestionIdx}
                    onmousedown={(e) => { e.preventDefault(); applyDaySuggestion(s); }}
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
              <input type="time" bind:value={fStartTime} onchange={autoFillEndTime} />
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
                    oninput={(e) => { formAlarms[i] = { ...alarm, value: (e.target as HTMLInputElement).value }; formAlarms = formAlarms; }}
                  />
                  <select
                    value={alarm.unit} style="margin-bottom:0"
                    onchange={(e) => { formAlarms[i] = { ...alarm, unit: (e.target as HTMLSelectElement).value }; formAlarms = formAlarms; }}
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
                    onchange={(e) => { formAlarms[i] = { ...alarm, datetime: (e.target as HTMLInputElement).value }; formAlarms = formAlarms; }}
                  />
                </div>
              {/if}
              <select
                class="alarm-direction" value={alarm.direction} style="margin-bottom:0"
                onchange={(e) => { formAlarms[i] = { ...alarm, direction: (e.target as HTMLSelectElement).value }; formAlarms = formAlarms; }}
              >
                <option value="before-start">{tl('event_reminderBeforeStart')}</option>
                <option value="before-end">{tl('event_reminderBeforeEnd')}</option>
                <option value="after-start">{tl('event_reminderAfterStart')}</option>
                <option value="after-end">{tl('event_reminderAfterEnd')}</option>
                <option value="absolute">{tl('event_reminderAtTime')}</option>
              </select>
              <button type="button" class="alarm-remove outline" aria-label="Remove" onclick={() => { formAlarms = formAlarms.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small" onclick={() => { formAlarms = [...formAlarms, { direction: 'before-start', value: '15', unit: '1', datetime: '' }]; }}>
          {tl('event_reminderAdd')}
        </button>
      </fieldset>

      <!-- Attendees -->
      <fieldset class="alarm-section">
        <legend>{tl('event_attendees')}</legend>
        <ul class="alarm-list">
          {#each formAttendees as att, i}
            <li class="alarm-row attendee-row">
              <div class="attendee-input-wrap" style="flex:1">
                <input
                  type="text"
                  placeholder={tl('event_attendeeName')}
                  value={att.cn}
                  style="margin-bottom:0;width:100%"
                  oninput={(e) => { const v = (e.target as HTMLInputElement).value; formAttendees[i] = { ...att, cn: v }; formAttendees = formAttendees; onAttendeeInput(i, v); }}
                  onkeydown={onAttendeeKeydown}
                  onblur={onAttendeeBlur}
                />
                {#if attendeeSuggestionRow === i && attendeeSuggestions.length > 0}
                  <ul class="time-suggestions" role="listbox">
                    {#each attendeeSuggestions as s, si}
                      <li role="option" aria-selected={si === attendeeSuggestionIdx}>
                        <button type="button" class:active={si === attendeeSuggestionIdx}
                          onmousedown={(e) => { e.preventDefault(); applyAttendeeSuggestion(s); }}
                        >{s.fn_name ? `${s.fn_name} <${s.email}>` : s.email}</button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
              <div class="attendee-input-wrap" style="flex:2">
                <input
                  type="email"
                  placeholder={tl('event_attendeeEmail')}
                  value={att.email}
                  style="margin-bottom:0;width:100%"
                  oninput={(e) => { const v = (e.target as HTMLInputElement).value; formAttendees[i] = { ...att, email: v }; formAttendees = formAttendees; onAttendeeInput(i, v); }}
                  onkeydown={onAttendeeKeydown}
                  onblur={onAttendeeBlur}
                />
                {#if attendeeSuggestionRow === i && attendeeSuggestions.length > 0}
                  <ul class="time-suggestions" role="listbox">
                    {#each attendeeSuggestions as s, si}
                      <li role="option" aria-selected={si === attendeeSuggestionIdx}>
                        <button type="button" class:active={si === attendeeSuggestionIdx}
                          onmousedown={(e) => { e.preventDefault(); applyAttendeeSuggestion(s); }}
                        >{s.fn_name ? `${s.fn_name} <${s.email}>` : s.email}</button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
              <select
                class="attendee-partstat-select"
                value={att.partstat}
                style="margin-bottom:0"
                onchange={(e) => { formAttendees[i] = { ...att, partstat: (e.target as HTMLSelectElement).value as PartStat }; formAttendees = formAttendees; }}
              >
                <option value="NEEDS-ACTION">{tl('event_attendeeStatusNeedsAction')}</option>
                <option value="ACCEPTED">{tl('event_attendeeStatusAccepted')}</option>
                <option value="DECLINED">{tl('event_attendeeStatusDeclined')}</option>
                <option value="TENTATIVE">{tl('event_attendeeStatusTentative')}</option>
              </select>
              <button type="button" class="alarm-remove outline" aria-label={tl('event_attendeeRemove')}
                onclick={() => { formAttendees = formAttendees.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small"
          onclick={() => { formAttendees = [...formAttendees, { cn: '', email: '', partstat: 'NEEDS-ACTION' as PartStat }]; }}>
          {tl('event_attendeeAdd')}
        </button>
      </fieldset>

      <!-- Recurrence -->
      <RecurrenceBuilder
        {tl}
        {formScope}
        bind:fFreq
        bind:fInterval
        bind:fByDay
        bind:fMonthlyMode
        bind:fMonthday
        bind:fSetpos
        bind:fSetposDay
        bind:fEndType
        bind:fCount
        bind:fUntil
      />

      <label>
        <span>{tl('event_calendar')}</span>
        <select
          value={`${fAccountId}\0${fCalendarHref}`}
          disabled={formScope === 'this_instance'}
          onchange={(e) => {
            const [accountId, href] = (e.target as HTMLSelectElement).value.split('\0');
            fAccountId = accountId;
            fCalendarHref = href;
            localStorage.setItem('defaultCalendar', `${fAccountId}\0${fCalendarHref}`);
          }}
        >
          {#each calendarGroups as group (group.account_id)}
            <optgroup label={group.label}>
              {#each calendars.filter(c => c.account_id === group.account_id) as cal (cal.account_id + cal.href)}
                <option value={`${cal.account_id}\0${cal.href}`}>{cal.name}</option>
              {/each}
            </optgroup>
          {/each}
        </select>
      </label>

      <div class="event-form-actions">
        <button type="submit" disabled={formSaving}>{tl('event_save')}</button>
        <button type="button" class="secondary" disabled={formSaving} onclick={() => formDialogEl?.close()}>{tl('event_cancel')}</button>
        {#if formMode === 'edit'}
          <button type="button" class="contrast outline" disabled={formSaving} onclick={deleteEvent}>{tl('event_delete')}</button>
        {/if}
      </div>
    </form>
  </article>
</dialog>

<style>
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
  .time-suggestions li { list-style: none; padding: 0; margin: 0; }
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
  .alarm-list { list-style: none; padding: 0; margin: 0 0 0.5rem; display: flex; flex-direction: column; gap: 0.5rem; }
  .alarm-row { display: flex; align-items: center; gap: 0.5rem; }
  .attendee-row { flex-wrap: nowrap; }
  .attendee-input-wrap { position: relative; min-width: 0; }
  .attendee-input-wrap input { width: 100%; }
  :global(.attendee-partstat-select) { flex: 0 0 auto; width: 7.5rem; margin-bottom: 0 !important; padding-left: 0.4rem; padding-right: 0.4rem; }
  .alarm-relative-fields { display: flex; align-items: center; gap: 0.25rem; }
  .alarm-absolute-fields { flex: 1; }
  :global(.alarm-direction) { flex: 1 1 0; min-width: 0; margin-bottom: 0 !important; }
  :global(.alarm-remove) { font-size: 1rem; line-height: 1; margin-bottom: 0 !important; margin-left: auto; }

</style>

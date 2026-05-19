<script lang="ts">
  import { tick } from 'svelte';
  import { createEventDispatcher } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import type { PartStat, AlarmFields, AttendeeFields, RecurrenceInfo, EventJson, CalendarJson } from '../types/calendar.ts';
  import { fmtDate } from '../lib/date.ts';

  export let calendars: CalendarJson[] = [];
  export let locale: string = 'en';
  export let focusedEventDate: string = '';

  const dispatch = createEventDispatcher<{ saved: void; deleted: void }>();

  $: tl = (() => {
    locale;
    return (key: string, params?: Record<string, string | number>) => {
      const fn = (m as Record<string, Function>)[key];
      return typeof fn === 'function' ? fn(params) : key;
    };
  })();

  let formMode: 'create' | 'edit' = 'create';
  let formScope: 'series' | 'this_instance' = 'series';
  let formDialogEl: HTMLDialogElement;
  let summaryInputEl: HTMLInputElement;
  let formError = '';
  let formSaving = false;

  let fUid = '', fSummary = '', fDate = '', fStartTime = '', fEndTime = '';
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
    const unitMap: Record<string, string> = {
      DAILY: tl('event_repeatUnitDaily'),
      WEEKLY: tl('event_repeatUnitWeekly'),
      MONTHLY: tl('event_repeatUnitMonthly'),
      YEARLY: tl('event_repeatUnitYearly'),
    };
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
    const [h, min] = fStartTime.split(':').map(Number);
    const total = h * 60 + min + 30;
    fEndTime = String(Math.floor(total / 60) % 24).padStart(2, '0') + ':' + String(total % 60).padStart(2, '0');
  }

  export function openNewEventForm() {
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

  export function openEditForm(ev: EventJson, scope: 'series' | 'this_instance') {
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
        dispatch('saved');
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
    let url = `/api/event?uid=${encodeURIComponent(fUid)}&calendar_href=${encodeURIComponent(fCalendarHref)}`;
    if (formScope === 'this_instance' && fOriginalDtstart)
      url += `&scope=this_instance&original_dtstart=${encodeURIComponent(fOriginalDtstart)}&tz=${encodeURIComponent(fTz)}`;
    try {
      const res = await fetch(url, { method: 'DELETE' });
      if (res.ok) { formDialogEl?.close(); dispatch('deleted'); }
    } catch {}
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
</script>

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

<style>
  /* Dialog base */
  dialog {
    border: 1px solid var(--pico-muted-border-color, #c8c8c8);
    border-radius: var(--pico-border-radius, 0.25rem);
    padding: 0;
    max-width: 28rem;
    width: 90vw;
    background: var(--pico-background-color, #fff);
    color: var(--pico-color, #333);
    box-shadow: 0 0.5rem 2rem rgba(0,0,0,0.25);
  }
  dialog::backdrop { background: rgba(0,0,0,0.4); }
  article { margin: 0; padding: 1.5rem; }
  article header { display: flex; justify-content: space-between; align-items: center; gap: 0.5rem; margin-bottom: 1rem; }
  article header h3 { margin: 0; flex: 1; word-break: break-word; }

  .close-btn {
    position: relative; background: none; border: none;
    font-size: 1.5rem; line-height: 1; padding: 0;
    width: 1.2em; height: 1.2em; cursor: pointer;
    color: transparent; display: inline-flex; align-items: center; justify-content: center;
  }
  .close-btn::before, .close-btn::after {
    content: ''; position: absolute; top: 50%; left: 50%;
    width: 0.75em; height: 0.1em;
    background-color: var(--pico-muted-color, #666); transition: background-color 0.15s;
  }
  .close-btn::before { transform: translate(-50%, -50%) rotate(45deg); }
  .close-btn::after { transform: translate(-50%, -50%) rotate(-45deg); }
  .close-btn:hover::before, .close-btn:hover::after { background-color: var(--pico-color, #333); }

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
</style>

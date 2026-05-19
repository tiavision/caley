<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { m } from '../paraglide/messages.js';
  import type { CalendarJson } from '../types/calendar.ts';

  export let calendars: CalendarJson[] = [];
  export let locale: string = 'en';

  const dispatch = createEventDispatcher<{ imported: { imported: number; errors: number } }>();

  $: tl = (() => {
    locale;
    return (key: string, params?: Record<string, string | number>) => {
      const fn = (m as Record<string, Function>)[key];
      return typeof fn === 'function' ? fn(params) : key;
    };
  })();

  const NEW_CAL_SENTINEL = '__new__';

  let dialogEl: HTMLDialogElement;
  let selectedHref = '';
  let newCalName = '';
  let fileInput: HTMLInputElement;
  let error = '';
  let successMsg = '';
  let importing = false;

  $: isNewCal = selectedHref === NEW_CAL_SENTINEL;

  export function open() {
    const stored = localStorage.getItem('defaultCalendar');
    const active = calendars.find(c => c.is_active);
    const fallback = active?.href || calendars[0]?.href || NEW_CAL_SENTINEL;
    selectedHref = (stored && calendars.find(c => c.href === stored && c.is_active)) ? stored : fallback;
    newCalName = '';
    error = '';
    successMsg = '';
    importing = false;
    if (fileInput) fileInput.value = '';
    dialogEl?.showModal();
  }

  async function submit() {
    error = '';
    successMsg = '';
    const file = fileInput?.files?.[0];
    if (!file) { error = tl('cal_importFile'); return; }

    importing = true;
    try {
      let targetHref = selectedHref;

      if (isNewCal) {
        const calName = newCalName.trim();
        if (!calName) { error = tl('cal_importNewCalendarName'); importing = false; return; }
        const resp = await fetch('/api/calendar', {
          method: 'POST',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: new URLSearchParams({ name: calName }).toString(),
        });
        if (!resp.ok) { error = await resp.text(); importing = false; return; }
        const cal: CalendarJson = await resp.json();
        targetHref = cal.href;
        dispatch('imported', { imported: 0, errors: 0 }); // signal calendars changed
      }

      const fd = new FormData();
      fd.append('file', file);
      fd.append('calendar_href', targetHref);
      const resp = await fetch('/api/import', { method: 'POST', body: fd });
      if (!resp.ok) { error = await resp.text(); return; }
      const result: { imported: number; errors: number } = await resp.json();
      successMsg = tl('cal_importSuccess', { n: result.imported });
      if (result.errors > 0) {
        successMsg += ' · ' + tl('cal_importPartial', { n: result.errors });
      }
      dispatch('imported', result);
      if (result.errors === 0) {
        setTimeout(() => dialogEl?.close(), 1500);
      }
    } catch (e) {
      error = tl('cal_importError');
    } finally {
      importing = false;
    }
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialogEl}
  on:click={(e) => { if (e.target === e.currentTarget) dialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{tl('cal_import')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} on:click={() => dialogEl?.close()}></button>
    </header>

    {#if error}
      <div class="form-error">{error}</div>
    {/if}
    {#if successMsg}
      <div class="form-success">{successMsg}</div>
    {/if}

    <form on:submit|preventDefault={submit}>
      <label>
        <span>{tl('cal_importFile')}</span>
        <input type="file" accept=".ics" bind:this={fileInput} required />
      </label>

      <label>
        <span>{tl('cal_importCalendar')}</span>
        <select bind:value={selectedHref}>
          {#each calendars as cal}
            <option value={cal.href}>{cal.name}</option>
          {/each}
          <option disabled>─────</option>
          <option value={NEW_CAL_SENTINEL}>{tl('cal_importNewCalendar')}</option>
        </select>
      </label>

      {#if isNewCal}
        <label>
          <span>{tl('cal_importNewCalendarName')}</span>
          <input type="text" bind:value={newCalName} required={isNewCal} />
        </label>
      {/if}

      <footer>
        <button type="submit" aria-busy={importing}>{tl('cal_importSubmit')}</button>
        <button type="button" class="secondary" on:click={() => dialogEl?.close()}>
          {tl('event_cancel')}
        </button>
      </footer>
    </form>
  </article>
</dialog>

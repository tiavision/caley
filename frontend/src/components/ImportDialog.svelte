<script lang="ts">
  import { makeTl } from '../lib/i18n.ts';
  import { apiFetch } from '../lib/api.ts';
  import type { CalendarJson, CalDavAccountJson } from '../types/calendar.ts';

  export let calendars: CalendarJson[] = [];
  export let accounts: CalDavAccountJson[] = [];
  export let locale: string = 'en';
  export let onimported: ((data: { imported: number; errors: number }) => void) | undefined = undefined;

  $: tl = makeTl(locale);

  const NEW_CAL_SENTINEL = '__new__';

  let dialogEl: HTMLDialogElement;
  let selectedHref = '';
  let selectedAccountId = '';
  let newCalName = '';
  let fileInput: HTMLInputElement;
  let selectedFile: File | null = null;
  let error = '';
  let successMsg = '';
  let importing = false;
  let previewing = false;
  let previewEvents: {
    uid: string;
    summary: string;
    dtstart: string;
    dtend: string;
    is_all_day: boolean;
    location: string;
  }[] = [];

  $: isNewCal = selectedHref === NEW_CAL_SENTINEL;

  export function open(file?: File) {
    const stored = localStorage.getItem('defaultCalendar');
    const active = calendars.find(c => c.is_active);
    const fallback = active?.href || calendars[0]?.href || NEW_CAL_SENTINEL;
    const storedCal = stored ? calendars.find(c => `${c.account_id}\0${c.href}` === stored && c.is_active) : null;
    const fallbackCal = active || calendars[0];
    selectedAccountId = storedCal?.account_id || fallbackCal?.account_id || accounts[0]?.account_id || '';
    selectedHref = storedCal?.href || fallback;
    newCalName = '';
    error = '';
    successMsg = '';
    importing = false;
    previewing = false;
    previewEvents = [];
    selectedFile = file ?? null;
    if (fileInput) fileInput.value = '';
    dialogEl?.showModal();
  }

  function onFileChange() {
    selectedFile = fileInput?.files?.[0] ?? null;
    previewEvents = [];
    error = '';
    successMsg = '';
  }

  function formatEventDate(event: { dtstart: string; dtend: string; is_all_day: boolean }) {
    if (!event.dtstart) return '';
    if (event.is_all_day) return event.dtstart;
    const start = event.dtstart.replace('T', ' ').slice(0, 16);
    const end = event.dtend ? event.dtend.replace('T', ' ').slice(0, 16) : '';
    return end ? `${start} - ${end}` : start;
  }

  async function preview() {
    error = '';
    successMsg = '';
    previewEvents = [];
    const file = selectedFile;
    if (!file) { error = tl('cal_importFile'); return; }

    previewing = true;
    try {
      const fd = new FormData();
      fd.append('file', file);
      const resp = await apiFetch('/api/import/preview', { method: 'POST', body: fd });
      if (!resp.ok) { error = await resp.text(); return; }
      previewEvents = await resp.json();
    } catch {
      error = tl('cal_importPreviewError');
    } finally {
      previewing = false;
    }
  }

  async function submit() {
    error = '';
    successMsg = '';
    const file = selectedFile;
    if (!file) { error = tl('cal_importFile'); return; }

    importing = true;
    try {
      let targetHref = selectedHref;
      let targetAccountId = selectedAccountId;

      if (isNewCal) {
        const calName = newCalName.trim();
        if (!calName) { error = tl('cal_importNewCalendarName'); importing = false; return; }
        const resp = await apiFetch('/api/calendar', {
          method: 'POST',
          headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
          body: new URLSearchParams({ account_id: targetAccountId, name: calName }).toString(),
        });
        if (!resp.ok) { error = await resp.text(); importing = false; return; }
        const cal: CalendarJson = await resp.json();
        targetHref = cal.href;
        targetAccountId = cal.account_id;
        onimported?.({ imported: 0, errors: 0 }); // signal calendars changed
      }

      const fd = new FormData();
      fd.append('file', file);
      fd.append('account_id', targetAccountId);
      fd.append('calendar_href', targetHref);
      const resp = await apiFetch('/api/import', { method: 'POST', body: fd });
      if (!resp.ok) { error = await resp.text(); return; }
      const result: { imported: number; errors: number } = await resp.json();
      successMsg = tl('cal_importSuccess', { n: result.imported });
      if (result.errors > 0) {
        successMsg += ' · ' + tl('cal_importPartial', { n: result.errors });
      }
      onimported?.(result);
      if (result.errors === 0) {
        localStorage.setItem('defaultCalendar', `${targetAccountId}\0${targetHref}`);
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
  onclick={(e) => { if (e.target === e.currentTarget) dialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{tl('cal_import')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => dialogEl?.close()}></button>
    </header>

    {#if error}
      <div class="form-error">{error}</div>
    {/if}
    {#if successMsg}
      <div class="form-success">{successMsg}</div>
    {/if}

    <form onsubmit={(e) => { e.preventDefault(); submit(); }}>
      <label>
        <span>{tl('cal_importFile')}</span>
        <input type="file" accept=".ics,.ical,text/calendar" bind:this={fileInput} required={!selectedFile} onchange={onFileChange} />
      </label>

      {#if selectedFile}
        <div class="import-file-summary">
          <span>{selectedFile.name}</span>
          <button type="button" class="outline secondary small" onclick={preview} disabled={previewing || importing} aria-busy={previewing}>
            {tl('cal_importPreview')}
          </button>
        </div>
      {/if}

      {#if previewEvents.length > 0}
        <div class="import-preview" aria-live="polite">
          <h4>{tl('cal_importPreviewHeading')}</h4>
          <ul>
            {#each previewEvents as event (event.uid)}
              <li>
                <strong>{event.summary || tl('cal_importUntitledEvent')}</strong>
                <span>{formatEventDate(event)}</span>
                {#if event.location}<span>{event.location}</span>{/if}
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      <label>
        <span>{tl('cal_importCalendar')}</span>
        <select
          value={`${selectedAccountId}\0${selectedHref}`}
          onchange={(e) => {
            const value = (e.target as HTMLSelectElement).value;
            if (value === NEW_CAL_SENTINEL) {
              selectedHref = NEW_CAL_SENTINEL;
              selectedAccountId = accounts[0]?.account_id || '';
            } else {
              const [accountId, href] = value.split('\0');
              selectedAccountId = accountId;
              selectedHref = href;
            }
          }}
        >
          {#each accounts as account (account.account_id)}
            <optgroup label={account.display_name}>
              {#each account.calendars as cal (cal.account_id + cal.href)}
                <option value={`${cal.account_id}\0${cal.href}`}>{cal.name}</option>
              {/each}
            </optgroup>
          {/each}
          <option disabled>─────</option>
          <option value={NEW_CAL_SENTINEL}>{tl('cal_importNewCalendar')}</option>
        </select>
      </label>

      {#if isNewCal && accounts.length > 1}
        <label>
          <span>{tl('mail_account')}</span>
          <select bind:value={selectedAccountId}>
            {#each accounts as account (account.account_id)}
              <option value={account.account_id}>{account.display_name}</option>
            {/each}
          </select>
        </label>
      {/if}

      {#if isNewCal}
        <label>
          <span>{tl('cal_importNewCalendarName')}</span>
          <input type="text" bind:value={newCalName} required={isNewCal} />
        </label>
      {/if}

      <footer>
        <button type="submit" aria-busy={importing}>{tl('cal_importSubmit')}</button>
        <button type="button" class="secondary" onclick={() => dialogEl?.close()}>
          {tl('event_cancel')}
        </button>
      </footer>
    </form>
  </article>
</dialog>

<style>
  .import-file-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin: -0.35rem 0 0.75rem;
    color: var(--pico-muted-color);
    font-size: 0.875rem;
  }

  .import-file-summary span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .import-file-summary button {
    flex-shrink: 0;
    margin: 0;
  }

  .import-preview {
    border-left: 3px solid var(--pico-primary-background);
    padding-left: 0.75rem;
    margin-bottom: 0.75rem;
  }

  .import-preview h4 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
  }

  .import-preview ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .import-preview li {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
  }

  .import-preview li span {
    color: var(--pico-muted-color);
    font-size: 0.85rem;
  }
</style>

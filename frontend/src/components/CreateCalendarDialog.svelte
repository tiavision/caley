<script lang="ts">
  import { makeTl } from '../lib/i18n.ts';
  import { apiFetch } from '../lib/api.ts';
  import type { CalendarJson, CalDavAccountJson } from '../types/calendar.ts';

  export let locale: string = 'en';
  export let accounts: CalDavAccountJson[] = [];
  export let oncreated: ((cal: CalendarJson) => void) | undefined = undefined;

  $: tl = makeTl(locale);

  let dialogEl: HTMLDialogElement;
  let name = '';
  let accountId = '';
  let color = '';
  let useColor = false;
  let error = '';
  let saving = false;

  export function open() {
    name = '';
    accountId = accounts[0]?.account_id || '';
    color = '#4a90d9';
    useColor = false;
    error = '';
    saving = false;
    dialogEl?.showModal();
  }

  async function submit() {
    error = '';
    const trimmed = name.trim();
    if (!trimmed) return;
    saving = true;
    try {
      const params = new URLSearchParams({ name: trimmed });
      params.set('account_id', accountId);
      if (useColor && color) params.set('color', color);
      const resp = await apiFetch('/api/calendar', {
        method: 'POST',
        headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
        body: params.toString(),
      });
      if (!resp.ok) {
        error = await resp.text();
        return;
      }
      const cal: CalendarJson = await resp.json();
      oncreated?.(cal);
      dialogEl?.close();
    } catch (e) {
      error = tl('cal_createCalendarError');
    } finally {
      saving = false;
    }
  }
</script>

<dialog id="create-calendar-dialog" closedby="any" aria-labelledby="create-calendar-title" bind:this={dialogEl}>
  <header>
    <h2 id="create-calendar-title">{tl('cal_createCalendar')}</h2>
    <button type="button" commandfor="create-calendar-dialog" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>

  {#if error}
    <div data-callout data-tone="danger" role="alert">{error}</div>
  {/if}

  <form data-layout="stack" data-gap="md" onsubmit={(e) => { e.preventDefault(); submit(); }}>
    <label>
      {tl('cal_createCalendarName')}
      <input type="text" bind:value={name} required />
    </label>

    {#if accounts.length > 1}
      <label>
        {tl('cal_createAccount')}
        <select bind:value={accountId} required>
          {#each accounts as account (account.account_id)}
            <option value={account.account_id}>{account.display_name}</option>
          {/each}
        </select>
      </label>
    {/if}

    <label>
      <input type="checkbox" bind:checked={useColor} />
      {tl('cal_createCalendarColor')}
    </label>

    {#if useColor}
      <input type="color" aria-label={tl('cal_createCalendarColor')} bind:value={color} />
    {/if}

    <footer>
      <button type="button" data-variant="secondary" commandfor="create-calendar-dialog" command="close">
        {tl('event_cancel')}
      </button>
      <button type="submit" aria-busy={saving}>{tl('cal_createCalendarSave')}</button>
    </footer>
  </form>
</dialog>

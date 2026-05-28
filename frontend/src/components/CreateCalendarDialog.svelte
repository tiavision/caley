<script lang="ts">
  import { makeTl } from '../lib/i18n.ts';
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
      const resp = await fetch('/api/calendar', {
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

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialogEl}
  onclick={(e) => { if (e.target === e.currentTarget) dialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{tl('cal_createCalendar')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={() => dialogEl?.close()}></button>
    </header>

    {#if error}
      <div class="form-error">{error}</div>
    {/if}

    <form onsubmit={(e) => { e.preventDefault(); submit(); }}>
      <label>
        <span>{tl('cal_createCalendarName')}</span>
        <input type="text" bind:value={name} required />
      </label>

      {#if accounts.length > 1}
        <label>
          <span>{tl('cal_createAccount')}</span>
          <select bind:value={accountId} required>
            {#each accounts as account (account.account_id)}
              <option value={account.account_id}>{account.display_name}</option>
            {/each}
          </select>
        </label>
      {/if}

      <label class="color-toggle">
        <input type="checkbox" bind:checked={useColor} />
        <span>{tl('cal_createCalendarColor')}</span>
      </label>

      {#if useColor}
        <label>
          <input type="color" bind:value={color} />
        </label>
      {/if}

      <footer>
        <button type="submit" aria-busy={saving}>{tl('cal_createCalendarSave')}</button>
        <button type="button" class="secondary" onclick={() => dialogEl?.close()}>
          {tl('event_cancel')}
        </button>
      </footer>
    </form>
  </article>
</dialog>

<style>
  .color-toggle {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
</style>

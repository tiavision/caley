<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { CalendarJson } from '../types/calendar.ts';

  export let locale: string = 'en';

  const dispatch = createEventDispatcher<{ created: CalendarJson }>();

  $: tl = makeTl(locale);

  let dialogEl: HTMLDialogElement;
  let name = '';
  let color = '';
  let useColor = false;
  let error = '';
  let saving = false;

  export function open() {
    name = '';
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
      dispatch('created', cal);
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
  on:click={(e) => { if (e.target === e.currentTarget) dialogEl?.close(); }}
>
  <article>
    <header>
      <h3>{tl('cal_createCalendar')}</h3>
      <button class="close-btn" aria-label={tl('weekly_aria_close')} on:click={() => dialogEl?.close()}></button>
    </header>

    {#if error}
      <div class="form-error">{error}</div>
    {/if}

    <form on:submit|preventDefault={submit}>
      <label>
        <span>{tl('cal_createCalendarName')}</span>
        <input type="text" bind:value={name} required />
      </label>

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
        <button type="button" class="secondary" on:click={() => dialogEl?.close()}>
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

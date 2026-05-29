<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson } from '../types/calendar.ts';

  export let locale: string;
  export let onEventClick: (event: EventJson) => void;

  $: tl = makeTl(locale);

  let dialogEl: HTMLDialogElement;
  let selectedDate = '';
  let events: EventJson[] = [];

  $: dateLabel = selectedDate
    ? new Date(selectedDate + 'T00:00:00').toLocaleDateString(locale, { weekday: 'long', month: 'long', day: 'numeric' })
    : '';

  export async function open(date: string, evs: EventJson[]) {
    selectedDate = date;
    events = evs;
    await tick();
    dialogEl?.showModal();
  }

  function close() {
    dialogEl?.close();
  }

  function handleEventClick(event: EventJson) {
    close();
    onEventClick(event);
  }

  function handleBackdropClick(e: MouseEvent) {
    if (e.target === dialogEl) close();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
<dialog bind:this={dialogEl} onclick={handleBackdropClick}>
  <!-- article + header: picked up by global `dialog article { padding }` and
       `dialog article header { flex; space-between; margin-bottom }` in main.scss,
       making this look identical to EventDetail -->
  <article class="day-events-article">
    <header>
      <h3>{dateLabel}</h3>
      <button class="close-btn" aria-label={tl('cal_month_close')} onclick={close}></button>
    </header>
    <div class="day-dialog-events">
      {#each events as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <article
          class="appointment event-trigger"
          style="border-left: 3px solid {event.color}"
          tabindex="0"
          onclick={() => handleEventClick(event)}
          onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); handleEventClick(event); } }}
        >
          <div class="appt-content">
            <span class="appt-time">{event.time_range.split('-')[0]}</span> · {event.summary}
          </div>
        </article>
      {/each}
    </div>
  </article>
</dialog>

<style>
  dialog {
    max-height: 80svh;
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(0,0,0,0.4);
  }

  /* Flex column so the header stays pinned and events scroll beneath it.
     The global `dialog article { padding: 1.25rem }` already provides spacing. */
  .day-events-article {
    display: flex;
    flex-direction: column;
    /* box-sizing lets max-height account for the 1.25rem padding on all sides */
    box-sizing: border-box;
    max-height: 80svh;
    overflow: hidden;
  }

  /* Scrollable events list — overflow-x:hidden stops the box-shadow on
     appointment chips (which extends 1px outside the element) from causing
     a horizontal scrollbar */
  .day-dialog-events {
    overflow-x: hidden;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
</style>

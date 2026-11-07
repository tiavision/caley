<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import { eventTimeLabel } from '../lib/format.ts';
  import type { EventJson } from '../types/calendar.ts';

  export let locale: string;
  export let onEventClick: (event: EventJson) => void;
  export let onNewEvent: (date: string) => void;

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

  function handleNewEvent() {
    close();
    onNewEvent(selectedDate);
  }
</script>

<dialog bind:this={dialogEl} id="day-events" closedby="any" data-layout="stack">
  <header data-layout="row" data-gap="xs" data-wrap="none">
    <h2 data-grow>{dateLabel}</h2>
    <button type="button" data-variant="ghost" data-icon="plus" aria-label={tl('event_new')} onclick={handleNewEvent}></button>
    <button type="button" commandfor="day-events" command="close" aria-label={tl('cal_month_close')}></button>
  </header>
  <!-- the wrapper scrolls, so the list keeps its natural height (rows of a scrolling flex list would shrink) -->
  <div data-scroll="y">
    <div data-list>
      {#each events as event (`${event.account_id}\0${event.calendar_href}\0${event.uid}\0${event.dtstart}`)}
        <button
          type="button"
          data-variant="item"
          data-size="sm"
          data-bar
          style={event.color ? `--tm-tone: ${event.color}` : null}
          onclick={() => handleEventClick(event)}
        >
          <small data-muted>{eventTimeLabel(event)}</small>
          <span data-clamp="2">{event.summary}</span>
        </button>
      {/each}
    </div>
  </div>
</dialog>

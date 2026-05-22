<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson } from '../types/calendar.ts';
  import { formatEventTime, formatRecurrence, formatAlarm, partstatKey } from '../lib/format.ts';

  export let locale: string;
  export let isOpen = false;
  export let onedit: ((data: { event: EventJson; scope: 'series' | 'this_instance' }) => void) | undefined = undefined;
  export let mapUrl: string = 'https://www.openstreetmap.org/search?query={q}';

  $: tl = makeTl(locale);

  let detailEvent: EventJson | null = null;
  let detailEditPopoverOpen = false;
  let dialogEl: HTMLDialogElement;

  $: detailTimeText = detailEvent ? formatEventTime(detailEvent) : '';
  $: detailRecText = (detailEvent?.is_recurring && detailEvent?.recurrence)
    ? formatRecurrence(detailEvent.recurrence, locale) : '';

  export async function open(event: EventJson) {
    detailEvent = event;
    detailEditPopoverOpen = false;
    isOpen = true;
    await tick();
    dialogEl?.showModal();
  }

  function openMap(event: MouseEvent, query: string) {
    event.preventDefault();
    const encoded = encodeURIComponent(query);

    let appOpened = false;
    const onBlur = () => { appOpened = true; };
    const onVisibility = () => { if (document.hidden) appOpened = true; };
    window.addEventListener('blur', onBlur, { once: true });
    document.addEventListener('visibilitychange', onVisibility, { once: true });

    window.location.href = `geo:0,0?q=${encoded}`;

    setTimeout(() => {
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onVisibility);
      if (!appOpened) {
        window.open(mapUrl.replace('{q}', encoded), '_blank');
      }
    }, 500);
  }

  export function close() {
    if (!isOpen) return;
    isOpen = false;
    detailEvent = null;
    dialogEl?.close();
  }

  export function triggerEdit() {
    if (!detailEvent) return;
    if (detailEvent.is_recurring) {
      detailEditPopoverOpen = !detailEditPopoverOpen;
    } else {
      const ev = detailEvent;
      close();
      onedit?.({ event: ev, scope: 'series' });
    }
  }

  function editWith(scope: 'series' | 'this_instance') {
    if (!detailEvent) return;
    const ev = detailEvent;
    close();
    onedit?.({ event: ev, scope });
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialogEl}
  id="event-detail"
  onclick={(e) => { if (e.target === e.currentTarget) close(); }}
  onclose={close}
  oncancel={(e) => { e.preventDefault(); close(); }}
>
  <article>
    <header>
      <h3>{detailEvent?.summary || ''}</h3>
      <div class="header-actions">
        {#if detailEvent}
          <div class="edit-wrap">
            <button
              class="edit-btn"
              title={tl('event_edit')}
              onclick={() => triggerEdit()}
            >&#9998;</button>
            {#if detailEditPopoverOpen}
              <div class="edit-choice-popover">
                <button onclick={() => editWith('series')}>{tl('event_editSeries')}</button>
                <button onclick={() => editWith('this_instance')}>{tl('event_editInstance')}</button>
              </div>
            {/if}
          </div>
        {/if}
        <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={close}></button>
      </div>
    </header>

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
            <a href="geo:0,0?q={encodeURIComponent(detailEvent.location)}" onclick={(e) => openMap(e, detailEvent!.location)} rel="noopener noreferrer">{detailEvent.location}</a>
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

<style>
  .header-actions { display: flex; align-items: center; gap: 0.25rem; }
  .edit-btn {
    background: none; border: none; cursor: pointer;
    font-size: 1rem; color: var(--pico-muted-color);
    padding: 0.25rem; line-height: 1; transition: color 0.15s;
  }
  .edit-btn:hover { color: var(--pico-primary-background); }

  .event-detail-body { margin-top: 1rem; display: flex; flex-direction: column; gap: 0.75rem; }
  .event-detail-row { display: flex; flex-direction: column; gap: 0.125rem; }
  .event-detail-label {
    font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--pico-muted-color, #666); font-weight: 600;
  }

  .edit-wrap { position: relative; }
  .edit-choice-popover {
    position: absolute;
    top: calc(100% + 0.25rem);
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    padding: 0.25rem;
    background: var(--pico-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.2);
    white-space: nowrap;
  }
  .edit-choice-popover button {
    background: none; border: none; text-align: left; font-size: 0.875rem;
    cursor: pointer; padding: 0.5rem 0.75rem; border-radius: var(--pico-border-radius);
    color: var(--pico-color); white-space: nowrap;
  }
  .edit-choice-popover button:hover { background: var(--pico-primary-background); color: var(--pico-primary-inverse); }

  .attendee-list-detail { display: flex; flex-direction: column; gap: 0.25rem; }
  .attendee-detail-row { display: flex; align-items: center; gap: 0.5rem; }
  .attendee-badge { font-size: 0.75rem; font-weight: 600; padding: 0.1rem 0.45rem; border-radius: 999px; white-space: nowrap; }
  .attendee-badge--accepted { background: #d1fae5; color: #065f46; }
  .attendee-badge--declined { background: #fee2e2; color: #991b1b; }
  .attendee-badge--tentative { background: #fef9c3; color: #854d0e; }
  .attendee-badge--needs-action { background: var(--pico-secondary-background); color: var(--pico-secondary-inverse); opacity: 0.7; }
</style>

<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson, SourceEmailJson } from '../types/calendar.ts';
  import { formatEventTimes, formatRecurrence, formatAlarm, partstatKey } from '../lib/format.ts';
  import { apiFetch } from '../lib/api.ts';
  import ShareMenu from './ShareMenu.svelte';
  import UiIcon from './UiIcon.svelte';

  export let locale: string;
  export let isOpen = false;
  export let onedit: ((data: { event: EventJson; scope: 'series' | 'this_instance' }) => void) | undefined = undefined;
  export let mapUrl: string = 'https://www.openstreetmap.org/search?query={q}';
  export let onthreads: ((emails: SourceEmailJson[], event: EventJson) => void) | undefined = undefined;

  $: tl = makeTl(locale);

  let detailEvent: EventJson | null = null;
  let detailEditPopoverOpen = false;
  let dialogEl: HTMLDialogElement;
  let sourceEmails: SourceEmailJson[] = [];

  $: detailTime = detailEvent ? formatEventTimes(detailEvent) : { localText: '', sourceText: '' };
  $: detailRecText = (detailEvent?.is_recurring && detailEvent?.recurrence)
    ? formatRecurrence(detailEvent.recurrence, locale) : '';

  export async function open(event: EventJson) {
    detailEvent = event;
    detailEditPopoverOpen = false;
    sourceEmails = [];
    isOpen = true;
    await tick();
    dialogEl?.showModal();
    void fetchSourceEmails(event);
  }

  async function fetchSourceEmails(event: EventJson) {
    try {
      const url = `/api/calendar/event-emails?cal_account_id=${encodeURIComponent(event.account_id)}&calendar_href=${encodeURIComponent(event.calendar_href)}&event_uid=${encodeURIComponent(event.uid)}`;
      const r = await apiFetch(url);
      if (r.ok) sourceEmails = await r.json();
    } catch {
      // non-fatal: thread icon simply won't appear
    }
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
    sourceEmails = [];
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
          {#if onthreads && sourceEmails.length > 0}
            <button class="edit-btn" title={tl('mail_threads')} onclick={() => onthreads!(sourceEmails, detailEvent!)}>
              <UiIcon name="tree" size={16} strokeWidth={2.2} />
            </button>
          {/if}
          <ShareMenu event={detailEvent} {locale} />
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
        {#if detailTime.localText}
          <div class="event-detail-row">
            <span class="event-detail-label">{tl('event_time')}</span>
            <span>{detailTime.localText}</span>
            {#if detailTime.sourceText}
              <span class="event-detail-secondary">{tl('event_originalTime')}: {detailTime.sourceText}</span>
            {/if}
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
  /* When the title and icons no longer fit beside each other, drop the title to a
     full-width second line while the icons stay on the top-right. wrap-reverse makes
     the wrapped (icons) line render above the title line. */
  header { flex-wrap: wrap-reverse; justify-content: flex-end; }
  header h3 { flex: 1 1 auto; min-width: 10rem; }

  .header-actions { display: flex; align-items: center; gap: 0.25rem; }
  .edit-btn {
    display: inline-flex; align-items: center; justify-content: center;
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
  .event-detail-secondary {
    color: var(--pico-muted-color, #666);
    font-size: 0.875rem;
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

<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson, SourceEmailJson } from '../types/calendar.ts';
  import { formatEventTimes, formatRecurrence, formatAlarm, partstatKey } from '../lib/format.ts';
  import { apiFetch } from '../lib/api.ts';
  import ShareMenu from './ShareMenu.svelte';

  export let locale: string;
  export let isOpen = false;
  export let onedit: ((data: { event: EventJson; scope: 'series' | 'this_instance' }) => void) | undefined = undefined;
  export let mapUrl: string = 'https://www.openstreetmap.org/search?query={q}';
  export let onthreads: ((emails: SourceEmailJson[], event: EventJson) => void) | undefined = undefined;

  $: tl = makeTl(locale);

  let detailEvent: EventJson | null = null;
  let editChoiceEl: HTMLElement;
  let editBtn: HTMLButtonElement;
  let dialogEl: HTMLDialogElement;
  let sourceEmails: SourceEmailJson[] = [];

  $: detailTime = detailEvent ? formatEventTimes(detailEvent) : { localText: '', sourceText: '' };
  $: detailRecText = (detailEvent?.is_recurring && detailEvent?.recurrence)
    ? formatRecurrence(detailEvent.recurrence, locale) : '';

  export async function open(event: EventJson) {
    detailEvent = event;
    hideEditChoice();
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
    hideEditChoice();
    detailEvent = null;
    sourceEmails = [];
    dialogEl?.close();
  }

  export function triggerEdit() {
    if (!detailEvent) return;
    if (detailEvent.is_recurring) {
      // Same as clicking the edit button, which opens the choice popover via popovertarget.
      try { editChoiceEl?.togglePopover({ source: editBtn } as unknown as boolean); }
      catch { editChoiceEl?.togglePopover(); }
    } else {
      const ev = detailEvent;
      close();
      onedit?.({ event: ev, scope: 'series' });
    }
  }

  function hideEditChoice() {
    if (editChoiceEl?.matches(':popover-open')) editChoiceEl.hidePopover();
  }

  function partstatTone(partstat: string): 'success' | 'danger' | 'warning' | 'neutral' {
    if (partstat === 'ACCEPTED') return 'success';
    if (partstat === 'DECLINED') return 'danger';
    if (partstat === 'TENTATIVE') return 'warning';
    return 'neutral';
  }

  function editWith(scope: 'series' | 'this_instance') {
    if (!detailEvent) return;
    hideEditChoice();
    const ev = detailEvent;
    close();
    onedit?.({ event: ev, scope });
  }
</script>

<dialog
  bind:this={dialogEl}
  id="event-detail"
  closedby="any"
  onclose={close}
>
  <header data-layout="row" data-gap="xs" data-align="start">
    <h2 data-grow>{detailEvent?.summary || ''}</h2>
    <div data-layout="row" data-gap="xs" data-wrap="none">
      {#if detailEvent}
        {#if onthreads && sourceEmails.length > 0}
          <button type="button" data-variant="ghost" data-icon="list-tree" aria-label={tl('mail_threads')} data-tooltip
            onclick={() => onthreads!(sourceEmails, detailEvent!)}></button>
        {/if}
        <ShareMenu event={detailEvent} {locale} />
        {#if detailEvent.is_recurring}
          <button type="button" bind:this={editBtn} data-variant="ghost" data-icon="pencil" aria-label={tl('event_edit')}
            popovertarget="event-edit-choice"></button>
        {:else}
          <button type="button" bind:this={editBtn} data-variant="ghost" data-icon="pencil" aria-label={tl('event_edit')} data-tooltip
            onclick={() => triggerEdit()}></button>
        {/if}
      {/if}
      <button type="button" commandfor="event-detail" command="close" aria-label={tl('weekly_aria_close')}></button>
    </div>
  </header>

  <div popover id="event-edit-choice" data-menu bind:this={editChoiceEl}>
    <button type="button" onclick={() => editWith('series')}>{tl('event_editSeries')}</button>
    <button type="button" onclick={() => editWith('this_instance')}>{tl('event_editInstance')}</button>
  </div>

  {#if detailEvent}
    <dl data-pairs>
      {#if detailTime.localText}
        <div>
          <dt>{tl('event_time')}</dt>
          <dd>
            {detailTime.localText}
            {#if detailTime.sourceText}
              <br /><small data-muted>{tl('event_originalTime')}: {detailTime.sourceText}</small>
            {/if}
          </dd>
        </div>
      {/if}
      {#if detailEvent.location}
        <div>
          <dt>{tl('event_location')}</dt>
          <dd><a href="geo:0,0?q={encodeURIComponent(detailEvent.location)}" onclick={(e) => openMap(e, detailEvent!.location)} rel="noopener noreferrer">{detailEvent.location}</a></dd>
        </div>
      {/if}
      {#if detailRecText}
        <div>
          <dt>{tl('event_repeats')}</dt>
          <dd>{detailRecText}</dd>
        </div>
      {/if}
      {#if detailEvent.description}
        <div>
          <dt>{tl('event_description')}</dt>
          <dd class="description">{detailEvent.description}</dd>
        </div>
      {/if}
      {#if detailEvent.alarms.length > 0}
        <div>
          <dt>{tl('event_reminder')}</dt>
          <dd data-layout="stack" data-gap="xs">
            {#each detailEvent.alarms as alarm}
              <span>{formatAlarm(alarm)}</span>
            {/each}
          </dd>
        </div>
      {/if}
      {#if detailEvent.attendees.length > 0}
        <div>
          <dt>{tl('event_attendees')}</dt>
          <dd data-layout="stack" data-gap="xs">
            {#each detailEvent.attendees as att}
              <span data-layout="row" data-gap="sm">
                <span data-badge data-tone={partstatTone(att.partstat)}>{tl(partstatKey(att.partstat))}</span>
                <span>{att.cn ? `${att.cn} <${att.email}>` : att.email}</span>
              </span>
            {/each}
          </dd>
        </div>
      {/if}
    </dl>
  {/if}
</dialog>

<style>
  .description { white-space: pre-wrap; overflow-wrap: anywhere; }
</style>

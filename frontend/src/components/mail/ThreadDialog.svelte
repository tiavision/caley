<script lang="ts">
  import { makeTl } from '../../lib/i18n.ts';
  import { buildThreadTree, senderName, formatDate, identityKey } from './mailUtils.ts';
  import type { EventJson } from '../../types/calendar.ts';
  import type { ThreadNode } from './MailTypes.ts';
  import ThreadEventItem from './ThreadEventItem.svelte';

  export let locale: string;
  export let threadMessages: ThreadNode[] = [];
  export let eventsByMessageKey: Record<string, EventJson[]> = {};
  // Which message node should be styled as "current" (the one being read)
  export let highlightedUid: { account_id: string; mailbox: string; uid: number } | undefined = undefined;
  // Which event uid should be styled as "current" (when arriving from an event)
  export let currentEventUid: string | undefined = undefined;
  export let onmessageclick: ((node: ThreadNode) => void) | undefined = undefined;
  export let oneventclick: ((ev: EventJson) => void) | undefined = undefined;

  $: tl = makeTl(locale);
  $: threadTree = buildThreadTree(threadMessages);
  $: eventsRenderKey = Object.entries(eventsByMessageKey)
    .map(([key, events]) => `${key}:${events.map(ev => `${ev.account_id}/${ev.calendar_href}/${ev.uid}`).join(',')}`)
    .join('|');

  let dialogEl: HTMLDialogElement;

  export function show() { dialogEl?.showModal(); }
  export function close() { dialogEl?.close(); }

  function isHighlightedMessage(node: ThreadNode): boolean {
    if (!highlightedUid) return false;
    return highlightedUid.account_id === node.account_id &&
      highlightedUid.mailbox === node.mailbox &&
      highlightedUid.uid === node.uid;
  }

  function nodeEvents(node: ThreadNode): EventJson[] {
    const exact = eventsByMessageKey[identityKey(node.account_id, node.mailbox, node.uid)] ?? [];
    if (exact.length > 0) return exact;

    const uidMatches = Object.entries(eventsByMessageKey)
      .filter(([key]) => key.endsWith(`\0${node.uid}`))
      .flatMap(([, events]) => events);
    if (uidMatches.length > 0) return uidMatches;

    if (threadMessages.length === 1) {
      return Object.values(eventsByMessageKey).flat();
    }
    return [];
  }
</script>

{#snippet threadBranch(nodes)}
  {#each nodes as branch}
    {@const node = branch.message}
    {@const events = nodeEvents(node)}
    <li data-layout="stack" data-gap="xs">
      <button
        type="button"
        data-variant="item"
        data-layout="row"
        data-gap="sm"
        data-wrap="none"
        aria-current={isHighlightedMessage(node) ? 'true' : null}
        on:click={() => onmessageclick?.(node)}
      >
        <span class="from" data-truncate>{senderName(node.from) || tl('mail_unknownSender')}</span>
        <span data-truncate data-grow data-muted>{node.subject || tl('mail_noSubject')}</span>
        <small data-muted>{formatDate(node.sent_at)}</small>
      </button>
      {#if events.length > 0}
        <ul class="branch" data-layout="stack" data-gap="xs">
          {#each events as ev (ev.account_id + '\0' + ev.calendar_href + '\0' + ev.uid)}
            <li>
              <ThreadEventItem
                event={ev}
                {locale}
                current={currentEventUid === ev.uid}
                onclick={() => oneventclick?.(ev)}
              />
            </li>
          {/each}
        </ul>
      {/if}
      {#if branch.children.length > 0}
        <ul class="branch" data-layout="stack" data-gap="xs">{@render threadBranch(branch.children)}</ul>
      {/if}
    </li>
  {/each}
{/snippet}

<dialog
  bind:this={dialogEl}
  id="thread-dialog"
  closedby="any"
  data-layout="stack"
  style="--tm-dialog-width: 40rem"
>
  <header>
    <h2>{tl('mail_threadsHeading')}</h2>
    <button type="button" commandfor="thread-dialog" command="close" aria-label={tl('mail_close')}></button>
  </header>
  <!-- the wrapper scrolls, so a short thread does not get a minimum height and long ones keep full-height rows -->
  <div data-scroll="y">
    <ul class="tree" data-layout="stack" data-gap="xs" aria-label={tl('mail_threadsHeading')}>
      {#key eventsRenderKey}
        {@render threadBranch(threadTree)}
      {/key}
    </ul>
  </div>
</dialog>

<style>
  /* Thread tree: nested replies are indented with a guide line. */
  .tree, .branch { list-style: none; margin: 0; padding: 0; }
  .branch {
    padding-inline-start: var(--tm-space-md);
    border-inline-start: 1px solid var(--tm-border);
  }
  .from { flex: 0 1 10rem; font-weight: 550; }
</style>

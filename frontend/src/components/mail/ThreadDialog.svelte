<script lang="ts">
  import { makeTl } from '../../lib/i18n.ts';
  import { buildThreadTree, senderName, formatDate } from './mailUtils.ts';
  import type { EventJson } from '../../types/calendar.ts';
  import type { ThreadNode } from './MailTypes.ts';
  import ThreadEventItem from './ThreadEventItem.svelte';

  export let locale: string;
  export let threadMessages: ThreadNode[] = [];
  export let messageEvents: EventJson[] = [];
  // Which messages have imported events underneath them
  export let sourceUids: { account_id: string; mailbox: string; uid: number }[] = [];
  // Which message node should be styled as "current" (the one being read)
  export let highlightedUid: { account_id: string; mailbox: string; uid: number } | undefined = undefined;
  // Which event uid should be styled as "current" (when arriving from an event)
  export let currentEventUid: string | undefined = undefined;
  export let onmessageclick: ((node: ThreadNode) => void) | undefined = undefined;
  export let oneventclick: ((ev: EventJson) => void) | undefined = undefined;

  $: tl = makeTl(locale);
  $: threadTree = buildThreadTree(threadMessages);

  let dialogEl: HTMLDialogElement;

  export function show() { dialogEl?.showModal(); }
  export function close() { dialogEl?.close(); }

  function isSourceMessage(node: ThreadNode): boolean {
    return sourceUids.some(
      s => s.account_id === node.account_id && s.mailbox === node.mailbox && s.uid === node.uid
    );
  }

  function isHighlightedMessage(node: ThreadNode): boolean {
    if (!highlightedUid) return false;
    return highlightedUid.account_id === node.account_id &&
      highlightedUid.mailbox === node.mailbox &&
      highlightedUid.uid === node.uid;
  }
</script>

{#snippet threadBranch(nodes)}
  {#each nodes as branch}
    {@const node = branch.message}
    <li>
      <button
        class="thread-message"
        class:current={isHighlightedMessage(node)}
        aria-current={isHighlightedMessage(node) ? 'true' : undefined}
        on:click={() => onmessageclick?.(node)}
      >
        <span class="thread-message-from">{senderName(node.from) || tl('mail_unknownSender')}</span>
        <span class="thread-message-subject">{node.subject || tl('mail_noSubject')}</span>
        <span class="thread-message-date">{formatDate(node.sent_at)}</span>
      </button>
      {#if isSourceMessage(node) && messageEvents.length > 0}
        <ul class="thread-events">
          {#each messageEvents as ev (ev.uid)}
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
        <ul>{@render threadBranch(branch.children)}</ul>
      {/if}
    </li>
  {/each}
{/snippet}

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialogEl}
  class="thread-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) close(); }}
  on:cancel={(e) => { e.preventDefault(); close(); }}
>
  <article>
    <header>
      <h3>{tl('mail_threadsHeading')}</h3>
      <button class="close-btn" aria-label={tl('mail_close')} on:click={close}></button>
    </header>
    <ul class="thread-tree" aria-label={tl('mail_threadsHeading')}>
      {@render threadBranch(threadTree)}
    </ul>
  </article>
</dialog>

<style>
  .thread-dialog {
    width: min(640px, calc(100vw - 2rem));
  }

  .thread-dialog article {
    margin: 0;
    padding: 0;
  }

  .thread-dialog header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 0;
    padding: 1rem 1.25rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
  }

  .thread-dialog header h3 {
    margin: 0;
  }

  .thread-tree,
  .thread-tree ul {
    list-style: none;
    list-style-type: none;
    margin: 0;
    padding: 0;
  }

  .thread-tree li::marker {
    content: '';
  }

  .thread-tree {
    padding: 0.65rem;
    max-height: min(65vh, 34rem);
    overflow-y: auto;
  }

  .thread-tree ul {
    padding-left: 1.15rem;
    border-left: 1px solid var(--pico-muted-border-color, #ddd);
  }

  .thread-tree li {
    margin: 0.2rem 0;
  }

  .thread-message {
    display: grid;
    grid-template-columns: minmax(6rem, 10rem) minmax(0, 1fr) auto;
    align-items: baseline;
    gap: 0.7rem;
    width: 100%;
    margin: 0;
    padding: 0.55rem 0.65rem;
    border: none;
    border-radius: 0.35rem;
    background: none;
    color: inherit;
    font-size: 0.84rem;
    text-align: left;
  }

  .thread-message:hover,
  .thread-message:focus-visible {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .thread-message.current {
    background: var(--pico-primary-background, #e6f2fb);
    box-shadow: inset 3px 0 0 var(--pico-primary, #0172ad);
  }

  .thread-message-from,
  .thread-message-subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .thread-message-subject {
    color: var(--pico-secondary, #444);
  }

  .thread-message-date {
    color: var(--pico-muted-color, #888);
    font-size: 0.75rem;
    white-space: nowrap;
  }
</style>

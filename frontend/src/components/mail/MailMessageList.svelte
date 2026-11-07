<script lang="ts">
  import { makeTl } from '../../lib/i18n.ts';
  import NavToggle from '../NavToggle.svelte';
  import { sameMessage, messageKey, senderName, formatDate, folderKind } from './mailUtils.ts';
  import type { Account, Message, VirtualFolder } from './MailTypes.ts';

  // ── Core display state ────────────────────────────────────────────────────
  export let tl: ReturnType<typeof makeTl>;
  export let standalone: boolean = false;
  export let isPhone: boolean = false;
  export let sidebarOpen: boolean = false;
  export let selectedFolderId: string | null = null;
  export let selectedMailbox: string | null = null;
  export let selectedAccountId: string | null = null;
  export let virtualFolders: VirtualFolder[] = [];
  export let FOLDER_NAMES: Record<string, string> = {};
  export let accounts: Account[] = [];

  // ── Message list state ────────────────────────────────────────────────────
  export let messages: Message[] = [];
  export let totalMessages: number = 0;
  export let currentPage: number = 1;
  export let messagesLoading: boolean = false;
  export let searching: boolean = false;
  export let searchQuery: string = '';
  export let showEmptyTrash: boolean = false;

  // ── Selection state ───────────────────────────────────────────────────────
  export let selectedMessage: Message | null = null;
  export let listCursor: Message | null = null;
  export let markedMessageKeys: Set<string> = new Set();
  export let selectedMessages: Message[] = [];

  // ── Filter state ──────────────────────────────────────────────────────────
  export let filterAddrActive: boolean = false;
  export let filterDateActive: boolean = false;
  export let filterUnreadOnly: boolean = false;
  export let filterAddrOpen: boolean = false;
  export let filterDateOpen: boolean = false;
  export let filterAddrs: string[] = [];
  export let filterDateFrom: string = '';
  export let filterDateTo: string = '';
  export let addrFilterInput: string = '';
  export let contactSuggestions: { name: string; email: string }[] = [];
  export let addrDbSuggestions: string[] = [];
  export let addrFilterEl: HTMLDivElement | undefined = undefined;
  export let addrPopoverEl: HTMLDivElement | undefined = undefined;
  export let addrPopoverStyle: string = '';
  export let dateFilterEl: HTMLDivElement | undefined = undefined;
  export let datePopoverEl: HTMLDivElement | undefined = undefined;
  export let datePopoverStyle: string = '';

  // ── Drag state ────────────────────────────────────────────────────────────
  export let draggedMessage: Message | null = null;
  export let dragOverMailbox: string | null = null;

  // ── Pagination ────────────────────────────────────────────────────────────
  export let hasPrev: boolean = false;
  export let hasNext: boolean = false;
  export let pageStart: number = 0;
  export let pageEnd: number = 0;
  export let totalPages: number = 0;
  export let pageInputActive: boolean = false;
  export let pageInputValue: string = '';
  export let listWidth: number = 320;

  // ── Exposed element refs ──────────────────────────────────────────────────
  export let listScrollEl: HTMLElement | undefined = undefined;
  export let pageInputEl: HTMLInputElement | null = null;

  // ── Callbacks ─────────────────────────────────────────────────────────────
  export let onSearchInput: () => void = () => {};
  export let onToggleAddrFilter: () => void = () => {};
  export let onToggleDateFilter: () => void = () => {};
  export let onToggleUnreadFilter: () => void = () => {};
  export let onAddrInput: () => void = () => {};
  export let onRunSearch: () => void = () => {};
  export let onMessageRowClick: (e: MouseEvent, msg: Message) => void = () => {};
  export let registerMessageRow: (node: HTMLButtonElement, msg: Message) => { update(m: Message): void; destroy(): void } = () => ({ update: () => {}, destroy: () => {} });
  export let onFirstPage: () => void = () => {};
  export let onPrevPage: () => void = () => {};
  export let onNextPage: () => void = () => {};
  export let onLastPage: () => void = () => {};
  export let onActivatePageInput: () => void = () => {};
  export let onConfirmPageInput: () => void = () => {};
  export let onCancelPageInput: () => void = () => {};
  export let onStartResize: (which: 'list', e: MouseEvent) => void = () => {};
  export let onResizeWithKeyboard: (which: 'list', e: KeyboardEvent) => void = () => {};
  export let onDragStart: (msg: Message, e: DragEvent) => void = () => {};
  export let onDragEnd: () => void = () => {};
  export let onEmptyTrash: () => void = () => {};
</script>

<!-- ── Message list ────────────────────────────────────────────────────── -->
<section class="message-list" data-layout="stack" data-gap="none" aria-label={tl('nav_mail')}>
  <div class="message-list-header" data-layout="stack" data-gap="sm" data-pad="sm">
    <div data-layout="row" data-gap="xs" data-wrap="none">
      {#if !standalone}<NavToggle />{/if}
      <button type="button" class="folder-toggle" data-variant="ghost" data-icon="folder" on:click={() => (sidebarOpen = true)} aria-label={tl('mail_aria_folders')}></button>
      {#if selectedFolderId}
        <h2 data-truncate data-grow>{virtualFolders.find(f => f.folder_id === selectedFolderId)?.display_name || tl('mail_aria_folders')}</h2>
      {:else if selectedMailbox}
        <h2 data-truncate data-grow>{FOLDER_NAMES[folderKind(selectedMailbox)] || selectedMailbox}</h2>
      {:else}
        <h2 data-truncate data-grow>{tl('nav_mail')}</h2>
      {/if}
      {#if selectedFolderId || selectedMailbox}
        <div data-layout="row" data-gap="xs" data-wrap="none">
          {#if showEmptyTrash}
            <button
              type="button"
              data-variant="ghost"
              data-tone="danger"
              data-icon="trash-2"
              aria-label={tl('mail_emptyTrash')}
              data-tooltip
              on:click|stopPropagation={onEmptyTrash}
            ></button>
          {/if}
          <div class="search-filter-wrap" bind:this={addrFilterEl}>
            <button
              type="button"
              data-variant="ghost"
              data-icon="user"
              aria-label={tl('mail_filterBySender')}
              data-tooltip
              on:click|stopPropagation={onToggleAddrFilter}
              aria-pressed={filterAddrActive}
            ></button>
            {#if filterAddrOpen}
              <div class="filter-popover" data-menu role="dialog" aria-label={tl('mail_filterBySender')} bind:this={addrPopoverEl} style={addrPopoverStyle}>
                <input
                  type="search"
                  placeholder={tl('mail_filterAddrPlaceholder')}
                  aria-label={tl('mail_filterAddrPlaceholder')}
                  bind:value={addrFilterInput}
                  on:input={onAddrInput}
                />
                {#if filterAddrs.length > 0}
                  <div data-layout="row" data-gap="xs" data-pad="xs">
                    {#each filterAddrs as addr}
                      {@const chipLabel = contactSuggestions.find(c => c.email === addr)?.name || addr}
                      <span class="filter-chip" data-badge data-tone="accent" title={addr}>
                        <span data-truncate>{chipLabel}</span>
                        <button type="button" class="chip-remove" data-variant="ghost" data-icon="x" aria-label={tl('mail_removeAttachment')} on:click={() => { filterAddrs = filterAddrs.filter(a => a !== addr); onRunSearch(); }}></button>
                      </span>
                    {/each}
                  </div>
                  <button type="button" data-icon="trash-2" on:click={() => { filterAddrs = []; onRunSearch(); }}>
                    {tl('mail_filterClearAddrs')}
                  </button>
                  <hr />
                {/if}
                {#each contactSuggestions.filter(c =>
                  !addrFilterInput ||
                  c.email.toLowerCase().includes(addrFilterInput.toLowerCase()) ||
                  c.name.toLowerCase().includes(addrFilterInput.toLowerCase())
                ).slice(0, 12) as s}
                  <label class="filter-suggestion-row">
                    <input
                      type="checkbox"
                      checked={filterAddrs.includes(s.email)}
                      on:change={(e) => {
                        if ((e.target as HTMLInputElement).checked) {
                          if (!filterAddrs.includes(s.email)) filterAddrs = [...filterAddrs, s.email];
                        } else {
                          filterAddrs = filterAddrs.filter(a => a !== s.email);
                        }
                        onRunSearch();
                      }}
                    />
                    <span data-layout="stack" data-gap="none" data-grow>
                      {#if s.name}<span data-truncate>{s.name}</span>{/if}
                      <small data-muted class="suggestion-email">{s.email}</small>
                    </span>
                  </label>
                {/each}
                {#each addrDbSuggestions.filter(addr =>
                  !contactSuggestions.some(c => c.email.toLowerCase() === addr) &&
                  !filterAddrs.includes(addr)
                ) as addr}
                  <label class="filter-suggestion-row">
                    <input
                      type="checkbox"
                      checked={filterAddrs.includes(addr)}
                      on:change={(e) => {
                        if ((e.target as HTMLInputElement).checked) {
                          if (!filterAddrs.includes(addr)) filterAddrs = [...filterAddrs, addr];
                        } else {
                          filterAddrs = filterAddrs.filter(a => a !== addr);
                        }
                        onRunSearch();
                      }}
                    />
                    <small class="suggestion-email" data-grow>{addr}</small>
                  </label>
                {/each}
                {#if addrFilterInput
                  && !contactSuggestions.some(c => c.email.toLowerCase() === addrFilterInput.toLowerCase())
                  && !addrDbSuggestions.includes(addrFilterInput.toLowerCase())}
                  <button type="button" data-tone="accent" data-icon="plus" on:click={() => {
                    const v = addrFilterInput.trim();
                    if (v && !filterAddrs.includes(v)) { filterAddrs = [...filterAddrs, v]; onRunSearch(); }
                    addrFilterInput = '';
                  }}>
                    <span data-truncate>{tl('mail_filterAddAddr')}: {addrFilterInput}</span>
                  </button>
                {/if}
              </div>
            {/if}
          </div>
          <div class="search-filter-wrap" bind:this={dateFilterEl}>
            <button
              type="button"
              data-variant="ghost"
              data-icon="calendar"
              aria-label={tl('mail_filterByDate')}
              data-tooltip
              on:click|stopPropagation={onToggleDateFilter}
              aria-pressed={filterDateActive}
            ></button>
            {#if filterDateOpen}
              <div class="filter-popover filter-popover-date" data-menu role="dialog" aria-label={tl('mail_filterByDate')} bind:this={datePopoverEl} style={datePopoverStyle}>
                <div data-layout="stack" data-gap="sm" data-pad="xs">
                  <label>
                    {tl('mail_filterDateFrom')}
                    <input type="date" bind:value={filterDateFrom} on:change={onRunSearch} />
                  </label>
                  <label>
                    {tl('mail_filterDateTo')}
                    <input type="date" bind:value={filterDateTo} on:change={onRunSearch} />
                  </label>
                  {#if filterDateActive}
                    <button type="button" data-variant="secondary" data-size="sm" data-self="start" on:click={() => { filterDateFrom = ''; filterDateTo = ''; onRunSearch(); }}>
                      {tl('mail_filterClearDate')}
                    </button>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
          <button
            type="button"
            data-variant="ghost"
            data-icon="circle-check"
            aria-label={tl('mail_filterUnread')}
            data-tooltip
            on:click|stopPropagation={onToggleUnreadFilter}
            aria-pressed={filterUnreadOnly}
          ></button>
        </div>
      {/if}
    </div>
    {#if selectedFolderId || selectedMailbox}
      <input
        type="search"
        placeholder={tl('mail_searchPlaceholder')}
        aria-label={tl('mail_searchPlaceholder')}
        bind:value={searchQuery}
        on:input={onSearchInput}
      />
    {/if}
    {#if searching && !messagesLoading}
      <small data-muted>{messages.length === 1 ? tl('mail_result1') : tl('mail_resultsN', { n: messages.length })}</small>
    {/if}
  </div>

  <div class="message-list-scroll" data-layout="stack" data-gap="none" data-scroll="y" data-grow bind:this={listScrollEl}>
    {#if messagesLoading}
      <p class="list-status" data-muted aria-busy="true">{tl('mail_loading')}</p>
    {:else if messages.length === 0}
      <p class="list-status" data-muted>
        {#if searching}
          {searchQuery.trim() ? tl('mail_noResults', { q: searchQuery }) : tl('mail_noResultsFiltered')}
        {:else if accounts.length === 0}
          {tl('mail_noAccounts')}
        {:else if !selectedAccountId && !selectedFolderId}
          {tl('mail_selectMailbox')}
        {:else}
          {tl('mail_noMessages')}
        {/if}
      </p>
    {:else}
      {#each messages as msg}
        <button
          type="button"
          class="message-row"
          class:unread={!msg.is_read}
          class:cursor={sameMessage(listCursor, msg)}
          data-variant="item"
          data-layout="stack"
          data-gap="none"
          data-align="stretch"
          data-bar
          data-tone={msg.is_read ? null : 'accent'}
          aria-pressed={markedMessageKeys.has(messageKey(msg))}
          aria-current={selectedMessages.length === 0 && sameMessage(selectedMessage, msg) ? 'true' : null}
          use:registerMessageRow={msg}
          tabindex={sameMessage(listCursor, msg) || (!listCursor && messages[0] === msg) ? 0 : -1}
          on:focus={() => (listCursor = msg)}
          on:click={(e) => onMessageRowClick(e, msg)}
          draggable="true"
          on:dragstart={(e) => onDragStart(msg, e)}
          on:dragend={onDragEnd}
        >
          <span data-layout="row" data-gap="sm" data-wrap="none">
            <span class="from" data-truncate data-grow>
              {#if !msg.is_read}<span class="unread-dot" aria-label="Unread"></span>{/if}{senderName(msg.from) || tl('mail_unknownSender')}
            </span>
            <small class="date" data-muted>{formatDate(msg.sent_at)}</small>
          </span>
          <span data-layout="row" data-gap="xs" data-wrap="none">
            <span class="subject" data-truncate data-grow>{msg.subject || tl('mail_noSubject')}</span>
            {#if msg.is_flagged}<span class="flag-star" data-icon="star" role="img" aria-label={tl('mail_flaggedTitle')} title={tl('mail_flaggedTitle')}></span>{/if}
            {#if msg.has_attachments && !msg.is_encrypted}<span data-icon="paperclip" data-muted role="img" aria-label={tl('mail_hasAttachments')} title={tl('mail_hasAttachments')}></span>{/if}
          </span>
        </button>
      {/each}
    {/if}
  </div>

  {#if searching ? messages.length > 0 : totalMessages > 0}
    <nav class="pagination-bar" data-layout="row" data-gap="xs" data-justify="center" data-wrap="none" data-pad="xs">
      <button type="button" data-variant="ghost" data-size="sm" data-icon="chevrons-left" on:click={onFirstPage} disabled={!hasPrev} aria-label={tl('mail_firstPageAria')} data-tooltip></button>
      <button type="button" data-variant="ghost" data-size="sm" data-icon="chevron-left" on:click={onPrevPage} disabled={!hasPrev} aria-label={tl('mail_prevPageAria')} data-tooltip></button>
      {#if !searching && pageInputActive}
        <input
          bind:this={pageInputEl}
          bind:value={pageInputValue}
          class="page-input"
          data-size="sm"
          type="text"
          inputmode="numeric"
          size="3"
          aria-label={tl('mail_jumpToPageAria')}
          on:keydown={(e) => { if (e.key === 'Enter') onConfirmPageInput(); if (e.key === 'Escape') onCancelPageInput(); }}
          on:blur={onConfirmPageInput}
        />
        <small data-muted>/ {totalPages}</small>
      {:else if !searching}
        <button type="button" data-variant="ghost" data-size="sm" on:click={onActivatePageInput} title={tl('mail_jumpToPageAria')}>
          {tl('mail_pageStatus', { start: pageStart, end: pageEnd, total: totalMessages })}
        </button>
      {:else}
        <small data-muted>{currentPage}</small>
      {/if}
      <button type="button" data-variant="ghost" data-size="sm" data-icon="chevron-right" on:click={onNextPage} disabled={!hasNext} aria-label={tl('mail_nextPageAria')} data-tooltip></button>
      {#if !searching}
        <button type="button" data-variant="ghost" data-size="sm" data-icon="chevrons-right" on:click={onLastPage} disabled={!hasNext} aria-label={tl('mail_lastPageAria')} data-tooltip></button>
      {/if}
    </nav>
  {/if}
</section>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions a11y_no_noninteractive_tabindex -->
<div
  class="resize-handle detail-resize-handle"
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={tl('mail_resizeMessages')}
  aria-valuemin="180"
  aria-valuemax="600"
  aria-valuenow={listWidth}
  on:mousedown={(e) => onStartResize('list', e)}
  on:keydown={(e) => onResizeWithKeyboard('list', e)}
></div>

<style>
  .message-list {
    min-inline-size: 0;
    overflow: hidden;
    background: var(--tm-bg);
  }

  .message-list-header {
    border-block-end: 1px solid var(--tm-border);
  }

  .message-list-header h2 {
    font-size: var(--tm-text-md);
  }

  .folder-toggle {
    display: none;
  }

  .search-filter-wrap {
    position: relative;
    flex-shrink: 0;
  }

  /* Placed with position: fixed by computePopoverStyle() in MailApp. */
  .filter-popover {
    position: fixed;
    z-index: 200;
    inline-size: min(20rem, 100vw - 1rem);
  }

  .filter-popover-date {
    inline-size: 14rem;
  }

  .filter-chip {
    max-inline-size: 100%;
    padding-inline-end: 0.125rem;
  }

  .chip-remove {
    inline-size: 1rem;
    block-size: 1rem;
    min-block-size: 0;
    color: inherit;
  }

  .filter-suggestion-row {
    align-items: start;
    block-size: auto;
    padding-block: var(--tm-space-xs);
  }

  .suggestion-email {
    overflow-wrap: anywhere;
  }

  .list-status {
    padding: var(--tm-space-xl) var(--tm-space-md);
    text-align: center;
  }

  .message-row {
    flex-shrink: 0;
    border-radius: 0;
    border-block-end: 1px solid var(--tm-border);
    padding-inline-start: calc(var(--tm-space-md) - 3px);
  }

  .message-row:not(.unread) {
    border-inline-start-color: transparent;
  }

  .message-row.cursor:not([aria-pressed='true'], [aria-current]) {
    background: var(--tm-fill);
  }

  .message-row[aria-pressed='true'] {
    box-shadow: inset 3px 0 0 var(--tm-accent);
  }

  .message-row .date {
    flex-shrink: 0;
    white-space: nowrap;
  }

  .message-row .subject {
    color: var(--tm-text-muted);
  }

  .message-row.unread .from,
  .message-row.unread .subject {
    font-weight: 600;
    color: var(--tm-text);
  }

  .unread-dot {
    display: inline-block;
    inline-size: 0.45rem;
    block-size: 0.45rem;
    margin-inline-end: var(--tm-space-xs);
    border-radius: var(--tm-radius-full);
    background: var(--tm-accent);
    vertical-align: middle;
  }

  .flag-star {
    color: var(--tm-warning);
  }

  .pagination-bar {
    border-block-start: 1px solid var(--tm-border);
  }

  .page-input {
    inline-size: 3.5rem;
    text-align: center;
  }

  .detail-resize-handle {
    inline-size: 3px;
    cursor: col-resize;
    background: var(--tm-border);
    transition: background-color var(--tm-duration) var(--tm-ease);
  }

  .detail-resize-handle:hover,
  .detail-resize-handle:focus-visible {
    background: var(--tm-accent);
    outline: none;
  }

  @media (max-width: 639px) {
    .folder-toggle {
      display: inline-flex;
    }

    .message-list {
      flex: 1;
    }
  }

  @media (max-width: 1023px) {
    .detail-resize-handle {
      display: none;
    }
  }
</style>

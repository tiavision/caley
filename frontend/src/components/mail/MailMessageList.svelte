<script lang="ts">
  import { makeTl } from '../../lib/i18n.ts';
  import UiIcon from '../UiIcon.svelte';
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
<section class="message-list">
  <div class="message-list-header">
    <div class="list-header-row">
      <button class="folder-toggle" on:click={() => (sidebarOpen = true)} aria-label={tl('mail_aria_folders')}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
      </button>
      {#if selectedFolderId}
        <h2>{virtualFolders.find(f => f.folder_id === selectedFolderId)?.display_name || tl('mail_aria_folders')}</h2>
      {:else if selectedMailbox}
        <h2>{FOLDER_NAMES[folderKind(selectedMailbox)] || selectedMailbox}</h2>
      {:else}
        <h2>{tl('nav_mail')}</h2>
      {/if}
      {#if selectedFolderId || selectedMailbox}
        <div class="header-filter-actions">
          {#if showEmptyTrash}
            <button
              class="empty-trash-btn"
              title={tl('mail_emptyTrash')}
              aria-label={tl('mail_emptyTrash')}
              on:click|stopPropagation={onEmptyTrash}
            >
              <UiIcon name="trash" size={15} strokeWidth={2.2} />
            </button>
          {/if}
          <div class="search-filter-wrap" bind:this={addrFilterEl}>
            <button
              class="filter-icon-btn"
              class:active={filterAddrActive}
              title={tl('mail_filterBySender')}
              on:click|stopPropagation={onToggleAddrFilter}
              aria-pressed={filterAddrActive}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 3.6-7 8-7s8 3 8 7"/></svg>
            </button>
            {#if filterAddrOpen}
              <div class="filter-popover" role="dialog" aria-label={tl('mail_filterBySender')} bind:this={addrPopoverEl} style={addrPopoverStyle}>
                <input
                  class="filter-popover-input"
                  type="search"
                  placeholder={tl('mail_filterAddrPlaceholder')}
                  bind:value={addrFilterInput}
                  on:input={onAddrInput}
                />
                {#if filterAddrs.length > 0}
                  <div class="filter-chips">
                    {#each filterAddrs as addr}
                      {@const chipLabel = contactSuggestions.find(c => c.email === addr)?.name || addr}
                      <span class="filter-chip" title={addr}>
                        <span class="chip-label">{chipLabel}</span>
                        <button class="chip-remove" on:click={() => { filterAddrs = filterAddrs.filter(a => a !== addr); onRunSearch(); }}>×</button>
                      </span>
                    {/each}
                  </div>
                  <button class="filter-clear-btn" on:click={() => { filterAddrs = []; onRunSearch(); }}>
                    <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6M14 11v6"/><path d="M9 6V4h6v2"/></svg>
                    {tl('mail_filterClearAddrs')}
                  </button>
                {/if}
                <div class="filter-suggestions">
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
                      <span class="suggestion-info">
                        {#if s.name}<span class="suggestion-name">{s.name}</span>{/if}
                        <span class="suggestion-email">{s.email}</span>
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
                      <span class="suggestion-info">
                        <span class="suggestion-email">{addr}</span>
                      </span>
                    </label>
                  {/each}
                  {#if addrFilterInput
                    && !contactSuggestions.some(c => c.email.toLowerCase() === addrFilterInput.toLowerCase())
                    && !addrDbSuggestions.includes(addrFilterInput.toLowerCase())}
                    <button class="filter-add-custom" on:click={() => {
                      const v = addrFilterInput.trim();
                      if (v && !filterAddrs.includes(v)) { filterAddrs = [...filterAddrs, v]; onRunSearch(); }
                      addrFilterInput = '';
                    }}>
                      + {tl('mail_filterAddAddr')}: {addrFilterInput}
                    </button>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
          <div class="search-filter-wrap" bind:this={dateFilterEl}>
            <button
              class="filter-icon-btn"
              class:active={filterDateActive}
              title={tl('mail_filterByDate')}
              on:click|stopPropagation={onToggleDateFilter}
              aria-pressed={filterDateActive}
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>
            </button>
            {#if filterDateOpen}
              <div class="filter-popover filter-popover-date" role="dialog" aria-label={tl('mail_filterByDate')} bind:this={datePopoverEl} style={datePopoverStyle}>
                <label class="filter-date-label">
                  {tl('mail_filterDateFrom')}
                  <input type="date" bind:value={filterDateFrom} on:change={onRunSearch} />
                </label>
                <label class="filter-date-label">
                  {tl('mail_filterDateTo')}
                  <input type="date" bind:value={filterDateTo} on:change={onRunSearch} />
                </label>
                {#if filterDateActive}
                  <button class="filter-clear-btn" on:click={() => { filterDateFrom = ''; filterDateTo = ''; onRunSearch(); }}>
                    {tl('mail_filterClearDate')}
                  </button>
                {/if}
              </div>
            {/if}
          </div>
          <button
            class="filter-icon-btn"
            class:active={filterUnreadOnly}
            title={tl('mail_filterUnread')}
            on:click|stopPropagation={onToggleUnreadFilter}
            aria-label={tl('mail_filterUnread')}
            aria-pressed={filterUnreadOnly}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M8 12.5l2.5 2.5L16 9"/></svg>
          </button>
        </div>
      {/if}
    </div>
    {#if selectedFolderId || selectedMailbox}
      <div class="search-row">
        <input
          class="search-input"
          type="search"
          placeholder={tl('mail_searchPlaceholder')}
          bind:value={searchQuery}
          on:input={onSearchInput}
        />
      </div>
    {/if}
    {#if searching && !messagesLoading}
      <span class="count">{messages.length === 1 ? tl('mail_result1') : tl('mail_resultsN', { n: messages.length })}</span>
    {/if}
  </div>

  <div class="message-list-scroll" bind:this={listScrollEl}>
    {#if messagesLoading}
      <div class="loading">{tl('mail_loading')}</div>
    {:else if messages.length === 0}
      <div class="empty">
        {#if searching}
          {searchQuery.trim() ? tl('mail_noResults', { q: searchQuery }) : tl('mail_noResultsFiltered')}
        {:else if accounts.length === 0}
          {tl('mail_noAccounts')}
        {:else if !selectedAccountId && !selectedFolderId}
          {tl('mail_selectMailbox')}
        {:else}
          {tl('mail_noMessages')}
        {/if}
      </div>
    {:else}
      {#each messages as msg}
        <button
          class="message-row"
          class:unread={!msg.is_read}
          class:selected={sameMessage(listCursor, msg)}
          class:marked={markedMessageKeys.has(messageKey(msg))}
          class:opened={selectedMessages.length === 0 && sameMessage(selectedMessage, msg)}
          aria-pressed={markedMessageKeys.has(messageKey(msg))}
          use:registerMessageRow={msg}
          tabindex={sameMessage(listCursor, msg) || (!listCursor && messages[0] === msg) ? 0 : -1}
          on:focus={() => (listCursor = msg)}
          on:click={(e) => onMessageRowClick(e, msg)}
          draggable="true"
          on:dragstart={(e) => onDragStart(msg, e)}
          on:dragend={onDragEnd}
        >
          <div class="message-row-top">
            <span class="from">
              {#if !msg.is_read}<span class="unread-dot" aria-label="Unread"></span>{/if}{senderName(msg.from) || tl('mail_unknownSender')}
            </span>
            <span class="date">{formatDate(msg.sent_at)}</span>
          </div>
          <div class="message-row-subject-line">
            <span class="message-row-subject">{msg.subject || tl('mail_noSubject')}</span>
            <span class="message-row-icons">
              {#if msg.is_flagged}<span class="flag-star" title={tl('mail_flaggedTitle')}>⭐</span>{/if}
              {#if msg.has_attachments && !msg.is_encrypted}<span class="attach-icon" title={tl('mail_hasAttachments')}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 10.56a4.94 4.94 0 0 1-7-7l9.19-10.56a3.33 3.33 0 0 1 4.7 4.7l-9.2 10.56a1.67 1.67 0 0 1-2.35-2.35l9.2-10.56"/></svg></span>{/if}
            </span>
          </div>
        </button>
      {/each}
    {/if}
  </div>

  {#if searching ? messages.length > 0 : totalMessages > 0}
    <nav class="pagination-bar">
      <div class="page-btn-group">
        <button class="page-btn" on:click={onFirstPage} disabled={!hasPrev} aria-label={tl('mail_firstPageAria')} title={tl('mail_firstPageAria')}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="5" y1="6" x2="5" y2="18"/><polyline points="15 18 9 12 15 6"/></svg>
        </button>
        <button class="page-btn" on:click={onPrevPage} disabled={!hasPrev} aria-label={tl('mail_prevPageAria')} title={tl('mail_prevPage')}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6"/></svg>
        </button>
      </div>
      {#if !searching && pageInputActive}
        <input
          bind:this={pageInputEl}
          bind:value={pageInputValue}
          class="page-input"
          type="text"
          inputmode="numeric"
          size="3"
          aria-label={tl('mail_jumpToPageAria')}
          on:keydown={(e) => { if (e.key === 'Enter') onConfirmPageInput(); if (e.key === 'Escape') onCancelPageInput(); }}
          on:blur={onConfirmPageInput}
        />
        <span class="page-status-of">/ {totalPages}</span>
      {:else if !searching}
        <button class="page-status page-status-btn" on:click={onActivatePageInput} title={tl('mail_jumpToPageAria')}>
          {tl('mail_pageStatus', { start: pageStart, end: pageEnd, total: totalMessages })}
        </button>
      {:else}
        <span class="page-status">{currentPage}</span>
      {/if}
      <div class="page-btn-group">
        <button class="page-btn" on:click={onNextPage} disabled={!hasNext} aria-label={tl('mail_nextPageAria')} title={tl('mail_nextPage')}>
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>
        </button>
        {#if !searching}
          <button class="page-btn" on:click={onLastPage} disabled={!hasNext} aria-label={tl('mail_lastPageAria')} title={tl('mail_lastPageAria')}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/><line x1="19" y1="6" x2="19" y2="18"/></svg>
          </button>
        {/if}
      </div>
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
    display: flex;
    flex-direction: column;
    min-width: 0;
    overflow: hidden;
    margin-bottom: 0;
  }

  .message-list-header {
    padding: 0.5rem 0.75rem 0.25rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex-shrink: 0;
  }

  .list-header-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }

  .list-header-row h2 {
    margin: 0;
    font-size: 1rem;
    line-height: 1.35;
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .header-filter-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    flex-shrink: 0;
  }

  .folder-toggle {
    display: none;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.2rem;
    color: inherit;
    margin: 0;
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .search-input {
    flex: 1;
    margin-bottom: 0;
    font-size: 0.85rem;
    padding: 0.3rem 0.5rem;
    height: auto;
  }

  .search-filter-wrap {
    position: relative;
    flex-shrink: 0;
  }

  .filter-icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    background: none;
    border: 1px solid transparent;
    border-radius: var(--pico-border-radius, 4px);
    cursor: pointer;
    color: var(--pico-muted-color, #888);
    padding: 0;
    margin: 0;
    transition: color 0.1s, border-color 0.1s;
  }

  .filter-icon-btn:hover {
    color: var(--pico-color);
    border-color: var(--pico-muted-border-color);
  }

  .filter-icon-btn.active {
    color: var(--pico-primary);
    border-color: var(--pico-primary);
    background: color-mix(in srgb, var(--pico-primary) 10%, transparent);
  }

  .empty-trash-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.3rem;
    width: 2rem;
    height: 2rem;
    margin: 0;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--pico-del-color, #c0392b) 45%, transparent);
    border-radius: var(--pico-border-radius, 4px);
    background: color-mix(in srgb, var(--pico-del-color, #c0392b) 8%, transparent);
    color: var(--pico-del-color, #c0392b);
    font-size: 0.78rem;
    line-height: 1;
    white-space: nowrap;
    cursor: pointer;
  }

  .empty-trash-btn:hover {
    background: var(--pico-del-color, #c0392b);
    border-color: var(--pico-del-color, #c0392b);
    color: #fff;
  }

  .filter-popover {
    position: fixed;
    z-index: 200;
    box-sizing: border-box;
    background: var(--pico-card-background-color, white);
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: var(--pico-border-radius, 4px);
    box-shadow: 0 4px 16px rgba(0,0,0,0.12);
    padding: 0.75rem;
    width: 320px;
    max-width: calc(100vw - 16px);
    max-height: 400px;
    overflow: hidden auto;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .filter-popover-input {
    margin-bottom: 0;
    font-size: 0.85rem;
  }

  .filter-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    background: var(--pico-primary);
    color: var(--pico-primary-inverse, #fff);
    border-radius: 1rem;
    padding: 0.3rem 0.45rem 0.3rem 0.7rem;
    font-size: 0.8rem;
    line-height: 1.2;
    max-width: 100%;
    min-width: 0;
    box-sizing: border-box;
  }

  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1 1 0;
  }

  .chip-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    width: 1.1rem;
    height: 1.1rem;
    min-height: 0;
    font-size: 0.85rem;
    color: inherit;
    opacity: 0.75;
    line-height: 1;
    margin: 0;
    flex-shrink: 0;
  }

  .chip-remove:hover { opacity: 1; }

  .filter-clear-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    background: var(--pico-muted-background-color, rgba(0,0,0,0.05));
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.75rem;
    line-height: 1.4;
    color: var(--pico-muted-color);
    padding: 0.35rem 0.75rem !important;
    margin: 0;
    width: auto;
    align-self: flex-start;
    transition: background 0.12s, border-color 0.12s;
  }

  .filter-clear-btn:hover {
    background: color-mix(in srgb, var(--pico-muted-border-color, #ccc) 30%, transparent);
    border-color: var(--pico-color, #333);
  }

  .filter-suggestions {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .filter-suggestion-row {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.22rem 0.4rem;
    margin: 0;
    cursor: pointer;
    border-radius: 4px;
    min-width: 0;
    overflow: hidden;
  }

  .filter-suggestion-row:hover {
    background: var(--pico-muted-background-color);
  }

  .filter-suggestion-row input[type="checkbox"] {
    margin: 0.2rem 0 0;
    flex-shrink: 0;
  }

  .suggestion-info {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    min-width: 0;
    flex: 1 1 0;
  }

  .suggestion-name {
    font-size: 0.85rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .suggestion-email {
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    word-break: break-all;
  }

  .filter-add-custom {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 0.82rem;
    color: var(--pico-primary);
    text-align: left;
    padding: 0.25rem;
    margin: 0;
    width: auto;
  }

  .filter-popover-date { min-width: 200px; }

  .filter-date-label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.8rem;
    margin-bottom: 0;
  }

  .filter-date-label input { margin: 0; font-size: 0.85rem; }

  .count {
    font-size: 0.78rem;
    color: var(--pico-muted-color, #888);
    padding-bottom: 0.1rem;
  }

  .message-list-scroll {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .loading, .empty {
    padding: 2rem 1rem;
    text-align: center;
    color: var(--pico-muted-color, #888);
    font-size: 0.9rem;
  }

  .message-row {
    display: block;
    width: 100%;
    padding: 0.6rem 1rem 0.6rem 1rem;
    border: none;
    border-left: 3px solid transparent;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    background: none;
    cursor: pointer;
    text-align: left;
    color: inherit;
  }

  .message-row:hover {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .message-row.selected {
    background: var(--pico-primary-background, #e6f2fb);
  }

  .message-row.marked {
    background: color-mix(in srgb, var(--pico-primary, #0172ad) 18%, transparent);
    box-shadow: inset 3px 0 0 var(--pico-primary, #0172ad);
  }

  .message-row.marked.selected {
    background: color-mix(in srgb, var(--pico-primary, #0172ad) 24%, transparent);
  }

  .message-row.opened:not(.selected) {
    box-shadow: inset 3px 0 0 var(--pico-primary, #0172ad);
  }

  .message-row.unread {
    border-left-color: var(--pico-primary, #0172ad);
  }

  .message-row.unread .from,
  .message-row.unread .message-row-subject {
    font-weight: 600;
  }

  .unread-dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--pico-primary, #0172ad);
    margin-right: 5px;
    vertical-align: middle;
    flex-shrink: 0;
  }

  .message-row-top {
    display: flex;
    justify-content: space-between;
    margin-bottom: 0.15rem;
  }

  .from {
    font-size: 0.85rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 70%;
  }

  .date {
    font-size: 0.75rem;
    color: var(--pico-muted-color, #888);
    flex-shrink: 0;
  }

  .message-row-subject-line {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  .message-row-subject {
    flex: 1;
    min-width: 0;
    font-size: 0.82rem;
    color: var(--pico-secondary, #444);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .message-row-icons {
    display: flex;
    align-items: center;
    gap: 0.1rem;
    flex-shrink: 0;
    font-size: 0.75rem;
    line-height: 1;
  }

  .flag-star { font-size: 0.75rem; }

  .attach-icon {
    display: inline-flex;
    align-items: center;
    color: var(--pico-muted-color, #888);
  }

  .pagination-bar {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.5rem 1rem;
    border-top: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
  }

  .page-btn-group {
    display: flex;
    align-items: center;
    gap: 0;
  }

  .page-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--pico-muted-color, #bbb);
    font-size: 0.85rem;
    padding: 0.15rem 0.3rem;
    line-height: 1;
    transition: color 0.15s;
  }

  .page-btn:hover:not(:disabled) {
    color: var(--pico-color, #333);
  }

  .page-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .page-status {
    font-size: 0.8rem;
    color: var(--pico-muted-color, #888);
    white-space: nowrap;
  }

  .page-status-btn {
    cursor: pointer;
    background: none;
    border: none;
    border-radius: 3px;
    padding: 0.1rem 0.2rem;
    font-size: 0.8rem;
    color: var(--pico-muted-color, #888);
    white-space: nowrap;
    line-height: 1;
  }

  .page-status-btn:hover {
    background: color-mix(in srgb, currentColor 10%, transparent);
  }

  .page-input {
    width: 2.8rem;
    text-align: center;
    font-size: 0.8rem;
    padding: 0.1rem 0.2rem;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: 3px;
    background: var(--pico-background-color);
    color: var(--pico-color);
    margin: 0;
  }

  .page-status-of {
    font-size: 0.8rem;
    color: var(--pico-muted-color, #888);
  }

  .detail-resize-handle {
    cursor: col-resize;
    background: var(--pico-muted-border-color, #ddd);
    border: none;
    border-radius: 0;
    margin: 0;
    padding: 0;
    min-width: 3px;
    width: 3px;
    transition: background 0.12s;
    position: relative;
    z-index: 1;
  }

  .detail-resize-handle:hover,
  .detail-resize-handle:focus-visible {
    background: var(--pico-primary, #0172ad);
  }

  @media (max-width: 639px) {
    .folder-toggle {
      display: flex;
      align-items: center;
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

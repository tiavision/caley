<script lang="ts">
  import { makeTl } from '../../lib/i18n.ts';
  import FolderIcon from './FolderIcon.svelte';
  import type { Account, Message, SidebarAccountSection, VirtualFolder } from './MailTypes.ts';
  import { folderKind } from './mailUtils.ts';

  export let tl: ReturnType<typeof makeTl>;
  export let isPhone: boolean = false;
  export let sidebarOpen: boolean = false;
  export let virtualFolders: VirtualFolder[] = [];
  export let selectedFolderId: string | null = null;
  export let folderUnreadCounts: Record<string, number> = {};
  export let sidebarAccountSections: SidebarAccountSection[] = [];
  export let selectedAccountId: string | null = null;
  export let selectedMailbox: string | null = null;
  export let syncingMailboxes: Set<string> = new Set();
  export let FOLDER_NAMES: Record<string, string> = {};
  export let sidebarWidth: number = 220;
  export let draggedMessage: Message | null = null;
  export let dragOverMailbox: string | null = null;

  // Callbacks
  export let onSelectVirtualFolder: (folderId: string) => void = () => {};
  export let onSelectMailbox: (accountId: string, mailbox: string) => void = () => {};
  export let onToggleCollapsed: (accountId: string) => void = () => {};
  export let onRemoveAccount: (acc: Account) => void = () => {};
  export let onMoveMessage: (msg: Message, destMailbox: string) => void = () => {};
  export let onStartResize: (which: 'sidebar', e: MouseEvent) => void = () => {};
  export let onResizeWithKeyboard: (which: 'sidebar', e: KeyboardEvent) => void = () => {};
  export let onEditFolder: (folder: VirtualFolder) => void = () => {};
  export let onDeleteFolder: (folder: VirtualFolder) => void = () => {};
  export let onOpenAddAccount: () => void = () => {};
  export let onOpenEditAccount: (acc: Account) => void = () => {};
  export let onOpenIdentities: (accountId: string) => void = () => {};
  export let onOpenCreateFolder: () => void = () => {};
</script>

{#if isPhone && sidebarOpen}
  <button type="button" class="sidebar-backdrop" aria-label={tl('mail_aria_folders')} on:click={() => (sidebarOpen = false)}></button>
{/if}
<aside class="mail-sidebar" class:open={isPhone && sidebarOpen}>
  <!-- Virtual folders -->
  {#if virtualFolders.length > 0}
    <div class="sidebar-section">
      <div class="sidebar-section-header">
        <span>{tl('mail_virtualFolders')}</span>
      </div>
      {#each virtualFolders as folder}
        {@const folderUnread = folderUnreadCounts[folder.folder_id] ?? 0}
        <div
          class="sidebar-item"
          class:active={selectedFolderId === folder.folder_id}
          role="button"
          tabindex="0"
          on:click={() => onSelectVirtualFolder(folder.folder_id)}
          on:keydown={e => e.key === 'Enter' && onSelectVirtualFolder(folder.folder_id)}
        >
          <FolderIcon kind="virtual" />
          <span class="folder-name">{folder.display_name}</span>
          {#if folderUnread > 0}
            <span class="unread-badge">{folderUnread > 999 ? '999+' : folderUnread}</span>
          {/if}
          <button class="icon-btn" on:click|stopPropagation={() => { sidebarOpen = false; onEditFolder(folder); }} title={tl('mail_editVirtualFolder')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
          <button class="icon-btn delete-btn" on:click|stopPropagation={() => { sidebarOpen = false; onDeleteFolder(folder); }} title={tl('mail_removeAttachment')}>×</button>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Accounts + mailboxes -->
  {#each sidebarAccountSections as section}
    {@const acc = section.account}
    <div class="sidebar-section">
      <div class="sidebar-section-header">
        <button
          class="icon-btn account-collapse-btn"
          class:collapsed={section.collapsed}
          on:click={() => onToggleCollapsed(acc.account_id)}
          title={section.collapsed ? tl('mail_expandAccount') : tl('mail_collapseAccount')}
          aria-label={section.collapsed ? tl('mail_expandAccount') : tl('mail_collapseAccount')}
          aria-expanded={!section.collapsed}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
        </button>
        <span class="account-name" title={acc.username}>{acc.display_name}</span>
        {#if section.unread_count > 0}
          <span class="unread-badge account-unread-badge">{section.unread_count > 999 ? '999+' : section.unread_count}</span>
        {/if}
        <button class="icon-btn" on:click={() => { sidebarOpen = false; onOpenIdentities(acc.account_id); }} title={tl('mail_identities')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 3.6-7 8-7s8 3 8 7"/></svg></button>
        <button class="icon-btn" on:click={() => { sidebarOpen = false; onOpenEditAccount(acc); }} title={tl('mail_editAccountHeading')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
        <button class="icon-btn delete-btn" on:click={() => { sidebarOpen = false; onRemoveAccount(acc); }} title={tl('mail_removeAttachment')}>×</button>
      </div>
      {#each section.mailboxes as item}
        {@const mb = item.mailbox}
        {@const syncKey = `${acc.account_id}|${mb.name}`}
        {@const isSyncing = syncingMailboxes.has(syncKey)}
        <button
          class="sidebar-item"
          class:active={selectedAccountId === acc.account_id && selectedMailbox === mb.name}
          class:drag-target={draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name}
          class:drag-over={dragOverMailbox === `${acc.account_id}|${mb.name}`}
          on:click={() => onSelectMailbox(acc.account_id, mb.name)}
          on:dragover={(e) => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { e.preventDefault(); dragOverMailbox = `${acc.account_id}|${mb.name}`; } }}
          on:dragleave={() => { if (dragOverMailbox === `${acc.account_id}|${mb.name}`) dragOverMailbox = null; }}
          on:drop={() => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { onMoveMessage(draggedMessage, mb.name); } draggedMessage = null; dragOverMailbox = null; }}
        >
          <FolderIcon kind={folderKind(mb.name)} />
          <span class="folder-name">{FOLDER_NAMES[folderKind(mb.name)] || mb.name}</span>
          {#if isSyncing}
            <span class="sync-spinner" aria-label="Syncing"></span>
          {:else if item.unread_count > 0}
            <span class="unread-badge">{item.unread_count > 999 ? '999+' : item.unread_count}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/each}

  <!-- Add account / folder buttons -->
  <div class="sidebar-actions sidebar-add">
    <button class="link-btn" on:click={() => { sidebarOpen = false; onOpenAddAccount(); }}>+ {tl('mail_addAccount')}</button>
    <button class="link-btn" on:click={() => { sidebarOpen = false; onOpenCreateFolder(); }}>+ {tl('mail_addVirtualFolder')}</button>
  </div>
</aside>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions a11y_no_noninteractive_tabindex -->
<div
  class="resize-handle"
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={tl('mail_resizeFolders')}
  aria-valuemin="140"
  aria-valuemax="400"
  aria-valuenow={sidebarWidth}
  on:mousedown={(e) => onStartResize('sidebar', e)}
  on:keydown={(e) => onResizeWithKeyboard('sidebar', e)}
></div>

<style>
  .sidebar-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.2);
    z-index: 60;
    border: none;
    padding: 0;
    cursor: pointer;
  }

  .mail-sidebar {
    background: var(--pico-card-background-color, #f4f4f4);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .sidebar-section {
    margin-bottom: 0.25rem;
  }

  .sidebar-section-header {
    display: flex;
    align-items: center;
    padding: 0.4rem 0.75rem;
    font-size: 0.75rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    color: var(--pico-muted-color, #666);
    gap: 0.25rem;
  }

  .account-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .account-collapse-btn {
    opacity: 0.65;
    transition: transform 0.12s, opacity 0.12s;
  }

  .account-collapse-btn.collapsed {
    transform: rotate(-90deg);
  }

  .account-collapse-btn:hover,
  .account-collapse-btn:focus-visible {
    opacity: 1;
  }

  .unread-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--pico-primary, #0172ad);
    color: #fff;
    font-size: 0.7rem;
    font-weight: 600;
    line-height: 1;
    flex-shrink: 0;
  }

  .account-unread-badge {
    margin-right: 0.15rem;
  }

  .sidebar-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.3rem 0.75rem 0.3rem 1.25rem;
    border: none;
    background: none;
    cursor: pointer;
    text-align: left;
    font-size: 0.85rem;
    color: inherit;
    border-radius: 0;
  }

  .sidebar-item:hover {
    background: var(--pico-muted-border-color, #e0e0e0);
  }

  .sidebar-item.active {
    background: var(--pico-primary-background, #e6f2fb);
    font-weight: 500;
  }

  .sidebar-item.drag-target {
    outline: 1px dashed var(--pico-primary, #2563eb);
  }

  .sidebar-item.drag-over {
    background: var(--pico-primary-background, #eff6ff);
    outline: 1px solid var(--pico-primary, #2563eb);
  }

  .folder-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

.sidebar-actions {
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .sidebar-add {
    margin-top: auto;
  }

  .link-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.1rem 0;
    font-size: 0.8rem;
    color: var(--pico-muted-color, #888);
    text-align: left;
  }

  .link-btn:hover {
    color: var(--pico-color);
  }

  .sync-spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--pico-muted-border-color, #ccc);
    border-top-color: var(--pico-primary, #0172ad);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .resize-handle {
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

  .resize-handle:hover,
  .resize-handle:focus-visible {
    background: var(--pico-primary, #0172ad);
  }

  @media (max-width: 639px) {
    .sidebar-backdrop {
      position: absolute;
    }

    .mail-sidebar {
      position: absolute;
      top: 0;
      left: 0;
      bottom: 0;
      width: 280px;
      max-width: 80vw;
      z-index: 61;
      transform: translateX(-100%);
      transition: transform 0.25s ease;
      box-shadow: 0 0 1.5rem rgba(0, 0, 0, 0.25);
      border-right: 1px solid var(--pico-muted-border-color, #ddd);
    }

    .mail-sidebar.open {
      transform: translateX(0);
    }

    .resize-handle {
      display: none;
    }
  }
</style>

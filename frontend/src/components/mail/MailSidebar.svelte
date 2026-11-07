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
<aside class="mail-sidebar" class:open={isPhone && sidebarOpen} data-layout="stack" data-gap="md" data-pad="sm" data-scroll="y" aria-label={tl('mail_aria_folders')}>
  <!-- Virtual folders -->
  {#if virtualFolders.length > 0}
    <div data-layout="stack" data-gap="none">
      <div class="section-header" data-layout="row" data-gap="xs" data-wrap="none">
        <span data-caption data-truncate data-grow>{tl('mail_virtualFolders')}</span>
      </div>
      {#each virtualFolders as folder}
        {@const folderUnread = folderUnreadCounts[folder.folder_id] ?? 0}
        <div data-layout="row" data-gap="none" data-wrap="none">
          <button
            type="button"
            class="sidebar-item"
            data-variant="item"
            data-size="sm"
            data-layout="row"
            data-gap="sm"
            data-wrap="none"
            data-grow
            aria-current={selectedFolderId === folder.folder_id ? 'true' : null}
            on:click={() => onSelectVirtualFolder(folder.folder_id)}
          >
            <FolderIcon kind="virtual" />
            <span data-truncate data-grow>{folder.display_name}</span>
            {#if folderUnread > 0}
              <span data-badge data-tone="accent">{folderUnread > 999 ? '999+' : folderUnread}</span>
            {/if}
          </button>
          <button type="button" data-variant="ghost" data-size="sm" data-icon="pencil" on:click={() => { sidebarOpen = false; onEditFolder(folder); }} aria-label={tl('mail_editVirtualFolder')} data-tooltip></button>
          <button type="button" data-variant="ghost" data-size="sm" data-icon="x" on:click={() => { sidebarOpen = false; onDeleteFolder(folder); }} aria-label={tl('mail_removeAttachment')} data-tooltip></button>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Accounts + mailboxes -->
  {#each sidebarAccountSections as section}
    {@const acc = section.account}
    <div data-layout="stack" data-gap="none">
      <div class="section-header" data-layout="row" data-gap="none" data-wrap="none">
        <button
          type="button"
          class="account-collapse-btn"
          class:collapsed={section.collapsed}
          data-variant="ghost"
          data-size="sm"
          data-icon="chevron-down"
          on:click={() => onToggleCollapsed(acc.account_id)}
          aria-label={section.collapsed ? tl('mail_expandAccount') : tl('mail_collapseAccount')}
          aria-expanded={!section.collapsed}
        ></button>
        <span data-caption data-truncate data-grow title={acc.username}>{acc.display_name}</span>
        {#if section.unread_count > 0}
          <span data-badge data-tone="accent">{section.unread_count > 999 ? '999+' : section.unread_count}</span>
        {/if}
        <button type="button" data-variant="ghost" data-size="sm" data-icon="user" on:click={() => { sidebarOpen = false; onOpenIdentities(acc.account_id); }} aria-label={tl('mail_identities')} data-tooltip></button>
        <button type="button" data-variant="ghost" data-size="sm" data-icon="pencil" on:click={() => { sidebarOpen = false; onOpenEditAccount(acc); }} aria-label={tl('mail_editAccountHeading')} data-tooltip></button>
        <button type="button" data-variant="ghost" data-size="sm" data-icon="x" on:click={() => { sidebarOpen = false; onRemoveAccount(acc); }} aria-label={tl('mail_removeAttachment')} data-tooltip></button>
      </div>
      {#each section.mailboxes as item}
        {@const mb = item.mailbox}
        {@const syncKey = `${acc.account_id}|${mb.name}`}
        {@const isSyncing = syncingMailboxes.has(syncKey)}
        <button
          type="button"
          class="sidebar-item"
          class:drag-target={draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name}
          class:drag-over={dragOverMailbox === `${acc.account_id}|${mb.name}`}
          data-variant="item"
          data-size="sm"
          aria-busy={isSyncing ? 'true' : null}
          data-layout="row"
          data-gap="sm"
          data-wrap="none"
          aria-current={selectedAccountId === acc.account_id && selectedMailbox === mb.name ? 'true' : null}
          on:click={() => onSelectMailbox(acc.account_id, mb.name)}
          on:dragover={(e) => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { e.preventDefault(); dragOverMailbox = `${acc.account_id}|${mb.name}`; } }}
          on:dragleave={() => { if (dragOverMailbox === `${acc.account_id}|${mb.name}`) dragOverMailbox = null; }}
          on:drop={() => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { onMoveMessage(draggedMessage, mb.name); } draggedMessage = null; dragOverMailbox = null; }}
        >
          <FolderIcon kind={folderKind(mb.name)} />
          <span data-truncate data-grow>{FOLDER_NAMES[folderKind(mb.name)] || mb.name}</span>
          {#if !isSyncing && item.unread_count > 0}
            <span data-badge data-tone="accent">{item.unread_count > 999 ? '999+' : item.unread_count}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/each}

  <!-- Add account / folder buttons -->
  <div class="sidebar-add" data-layout="stack" data-gap="none">
    <button type="button" data-variant="ghost" data-size="sm" data-icon="plus" on:click={() => { sidebarOpen = false; onOpenAddAccount(); }}>{tl('mail_addAccount')}</button>
    <button type="button" data-variant="ghost" data-size="sm" data-icon="plus" on:click={() => { sidebarOpen = false; onOpenCreateFolder(); }}>{tl('mail_addVirtualFolder')}</button>
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
    position: absolute;
    inset: 0;
    z-index: 60;
    min-block-size: 0;
    border: 0;
    border-radius: 0;
    padding: 0;
    background: color-mix(in oklch, var(--tm-text) 25%, transparent);
  }

  .mail-sidebar {
    background: var(--tm-surface);
    border-inline-end: 1px solid var(--tm-border);
  }

  .section-header {
    padding-inline-start: var(--tm-space-xs);
  }

  .account-collapse-btn {
    transition: rotate var(--tm-duration) var(--tm-ease);
  }

  .account-collapse-btn.collapsed {
    rotate: -90deg;
  }

  .sidebar-item {
    font-weight: 450;
  }

  .sidebar-item[aria-current] {
    font-weight: 550;
  }

  .sidebar-item.drag-target {
    outline: 1px dashed var(--tm-accent);
    outline-offset: -1px;
  }

  .sidebar-item.drag-over {
    background: var(--tm-tone-soft);
    outline: 1px solid var(--tm-accent);
    outline-offset: -1px;
  }

  .sidebar-add {
    margin-block-start: auto;
    align-items: start;
  }


  .resize-handle {
    inline-size: 3px;
    cursor: col-resize;
    background: var(--tm-border);
    transition: background-color var(--tm-duration) var(--tm-ease);
  }

  .resize-handle:hover,
  .resize-handle:focus-visible {
    background: var(--tm-accent);
    outline: none;
  }

  @media (max-width: 639px) {
    .mail-sidebar {
      position: absolute;
      inset-block: 0;
      inset-inline-start: 0;
      inline-size: min(280px, 80vw);
      z-index: 61;
      translate: -100% 0;
      transition: translate 0.25s var(--tm-ease);
      box-shadow: var(--tm-shadow-lg);
    }

    .mail-sidebar.open {
      translate: 0 0;
    }

    .resize-handle {
      display: none;
    }
  }
</style>

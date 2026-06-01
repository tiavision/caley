<script lang="ts">
  import { getLocale, makeTl } from '../../lib/i18n.ts';
  import { folderKind, FOLDER_PRIORITY } from './mailUtils.ts';
  import type { Account, Mailbox, VirtualFolder } from './MailTypes.ts';

  export let accounts: Account[] = [];
  export let mailboxesByAccount: Record<string, Mailbox[]> = {};
  export let onSaved: () => Promise<void> = async () => {};
  export let onDeleted: (folderId: string) => void = () => {};

  let locale = getLocale();
  $: tl = makeTl(locale);
  $: FOLDER_NAMES = {
    inbox: tl('mail_folder_inbox'),
    sent: tl('mail_folder_sent'),
    drafts: tl('mail_folder_drafts'),
    trash: tl('mail_folder_trash'),
    junk: tl('mail_folder_junk'),
    archive: tl('mail_folder_archive'),
    folder: '',
  };

  let dialog: HTMLDialogElement;
  let folderName = '';
  let folderSaving = false;
  let folderSelectedMailboxes: Set<string> = new Set();
  let folderOnlyFlagged = false;
  let folderOnlyAttachments = false;
  let editingFolder: VirtualFolder | null = null;

  export function openCreate() {
    editingFolder = null;
    folderName = '';
    folderSelectedMailboxes = new Set();
    folderOnlyFlagged = false;
    folderOnlyAttachments = false;
    dialog?.showModal();
  }

  export function openEdit(folder: VirtualFolder) {
    editingFolder = folder;
    folderName = folder.display_name;
    folderOnlyFlagged = folder.only_flagged ?? false;
    folderOnlyAttachments = folder.only_attachments ?? false;
    const isMatchAll = folder.rules.length === 1 &&
      folder.rules[0].account_id === null &&
      folder.rules[0].mailbox_pattern === null;
    folderSelectedMailboxes = isMatchAll
      ? new Set()
      : new Set(folder.rules.map(r => `${r.account_id ?? ''}::${r.mailbox_pattern ?? ''}`));
    dialog?.showModal();
  }

  async function saveVirtualFolder() {
    folderSaving = true;
    try {
      const rules = folderSelectedMailboxes.size === 0
        ? [{ account_id: null, mailbox_pattern: null }]
        : [...folderSelectedMailboxes].map(key => {
            const sep = key.indexOf('::');
            return {
              account_id: key.slice(0, sep) || null,
              mailbox_pattern: key.slice(sep + 2) || null,
            };
          });
      const body = JSON.stringify({
        display_name: folderName || tl('mail_folderNamePlaceholder'),
        rules,
        only_flagged: folderOnlyFlagged,
        only_attachments: folderOnlyAttachments,
      });
      if (editingFolder) {
        await fetch(`/api/email/folder?folder_id=${encodeURIComponent(editingFolder.folder_id)}`, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body,
        });
      } else {
        await fetch('/api/email/folder', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body,
        });
      }
      editingFolder = null;
      folderName = '';
      folderSelectedMailboxes = new Set();
      dialog?.close();
      await onSaved();
    } catch {}
    folderSaving = false;
  }

  export async function deleteFolder(folder: VirtualFolder) {
    await fetch(`/api/email/folder?folder_id=${encodeURIComponent(folder.folder_id)}`, { method: 'DELETE' });
    onDeleted(folder.folder_id);
    await onSaved();
  }
</script>

<dialog
  bind:this={dialog}
  on:click={(e) => { if (e.target === e.currentTarget) dialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); dialog?.close(); }}
>
  <article>
    <header>
      <h3>{editingFolder ? tl('mail_editVirtualFolder') : tl('mail_createVirtualFolder')}</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label={tl('mail_close')} on:click={() => dialog?.close()}></button>
      </div>
    </header>
    <form on:submit|preventDefault={saveVirtualFolder}>
      <label>
        {tl('mail_name')}
        <input bind:value={folderName} placeholder={tl('mail_folderNamePlaceholder')} />
      </label>
      <p class="folder-mailbox-label">{tl('mail_mailboxesToInclude')}</p>
      <div class="folder-mailbox-list">
        {#each accounts as acc}
          {#if (mailboxesByAccount[acc.account_id] || []).filter(mb => mb.subscribed).length > 0}
            <div class="folder-account-group">
              <h4>{acc.display_name}</h4>
              {#each (mailboxesByAccount[acc.account_id] || []).filter(mb => mb.subscribed) as mb}
                {@const key = `${acc.account_id}::${mb.name}`}
                <label>
                  <input
                    type="checkbox"
                    checked={folderSelectedMailboxes.has(key)}
                    on:change={() => {
                      if (folderSelectedMailboxes.has(key)) folderSelectedMailboxes.delete(key);
                      else folderSelectedMailboxes.add(key);
                      folderSelectedMailboxes = folderSelectedMailboxes;
                    }}
                  />
                  {FOLDER_NAMES[folderKind(mb.name)] || mb.name}
                </label>
              {/each}
            </div>
          {/if}
        {/each}
        {#if accounts.length === 0}
          <p class="muted">{tl('mail_noEmailAccounts')}</p>
        {/if}
      </div>
      <p class="folder-hint">{tl('mail_allMailboxesHint')}</p>
      <div class="folder-filters">
        <label>
          <input type="checkbox" bind:checked={folderOnlyFlagged} />
          {tl('mail_onlyFlagged')}
        </label>
        <label>
          <input type="checkbox" bind:checked={folderOnlyAttachments} />
          {tl('mail_onlyWithAttachments')}
        </label>
      </div>
      <footer>
        <button type="button" class="secondary outline" on:click={() => dialog?.close()}>{tl('mail_cancel')}</button>
        <button type="submit" class="primary" disabled={folderSaving}>{editingFolder ? tl('mail_save') : tl('mail_create')}</button>
      </footer>
    </form>
  </article>
</dialog>

<style>
  .folder-mailbox-label {
    font-size: 0.875rem;
    font-weight: 500;
    margin: 1rem 0 0.25rem;
  }

  .folder-mailbox-list {
    max-height: 14rem;
    overflow-y: auto;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: 4px;
    padding: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .folder-account-group h4 {
    font-size: 0.8rem;
    margin: 0 0 0.25rem;
    color: var(--pico-muted-color);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .folder-hint {
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    margin: 0.5rem 0;
  }

  .folder-filters {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin-top: 0.5rem;
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
</style>

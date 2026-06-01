<script lang="ts">
  import { getLocale, makeTl } from '../../lib/i18n.ts';
  import type { Account, Identity } from './MailTypes.ts';

  export let identities: Identity[] = [];
  export let accounts: Account[] = [];
  export let onAccountsChanged: () => Promise<void> = async () => {};
  export let onIdentitiesChanged: () => Promise<void> = async () => {};

  let locale = getLocale();
  $: tl = makeTl(locale);

  // ── Account form state ─────────────────────────────────────────────────────

  let accountDialog: HTMLDialogElement;
  let identityDialog: HTMLDialogElement;

  function emptyAccountForm() {
    return {
      account_id: null as string | null,
      display_name: '',
      sender_name: '',
      host: '',
      port: 993,
      username: '',
      password: '',
      use_tls: true,
      smtp_host: '',
      smtp_port: 587,
    };
  }

  let accountForm = emptyAccountForm();
  let accountSaving = false;
  let accountError = '';
  let accountStep: 'basic' | 'details' = 'basic';
  let autodiscoverResult: { provider_name?: string; imap_host: string; imap_port: number; imap_use_tls: boolean; imap_username: string; smtp_host: string; smtp_port: number } | null = null;
  let autodiscovering = false;
  let autodiscoverError = '';
  let showAdvanced = false;

  // ── Identity form state ────────────────────────────────────────────────────

  function emptyIdentityForm() {
    return {
      identity_id: null as string | null,
      account_id: '',
      name: '',
      email: '',
    };
  }

  let identityForm = emptyIdentityForm();
  let identitySaving = false;
  let identityError = '';
  let identityAccountId = '';
  let identityMode: 'idle' | 'add' | 'edit' = 'idle';

  // ── Exported open methods ──────────────────────────────────────────────────

  export function openAdd() {
    accountForm = emptyAccountForm();
    accountError = '';
    accountStep = 'basic';
    autodiscoverResult = null;
    autodiscoverError = '';
    showAdvanced = false;
    accountDialog?.showModal();
  }

  export function openEdit(acc: Account) {
    const defaultIdentity = identities.find(i => i.account_id === acc.account_id && i.is_default);
    accountForm = {
      account_id: acc.account_id,
      display_name: acc.display_name,
      sender_name: defaultIdentity?.name ?? '',
      host: acc.host,
      port: acc.port,
      username: acc.username,
      password: '',
      use_tls: acc.use_tls,
      smtp_host: acc.smtp_host || '',
      smtp_port: acc.smtp_port || 587,
    };
    accountError = '';
    accountStep = 'details';
    autodiscoverResult = null;
    showAdvanced = true;
    accountDialog?.showModal();
  }

  export function openIdentities(accountId: string) {
    identityAccountId = accountId;
    identityMode = 'idle';
    identityForm = emptyIdentityForm();
    identityForm.account_id = accountId;
    identityError = '';
    identityDialog?.showModal();
  }

  // ── Account operations ─────────────────────────────────────────────────────

  async function continueSetup() {
    autodiscovering = true;
    autodiscoverError = '';
    try {
      const r = await fetch(`/api/email/autodiscover?email=${encodeURIComponent(accountForm.username)}`);
      if (r.ok) {
        const data = await r.json();
        if (data.found) {
          autodiscoverResult = data;
          accountForm.host = data.imap_host;
          accountForm.port = data.imap_port;
          accountForm.use_tls = data.imap_use_tls;
          accountForm.username = data.imap_username;
          accountForm.smtp_host = data.smtp_host;
          accountForm.smtp_port = data.smtp_port;
          if (!accountForm.display_name) {
            accountForm.display_name = data.provider_name || accountForm.username.split('@')[1] || '';
          }
          showAdvanced = false;
        } else {
          if (!accountForm.display_name) {
            accountForm.display_name = accountForm.username.split('@')[1] || '';
          }
          showAdvanced = true;
        }
      } else {
        if (!accountForm.display_name) {
          accountForm.display_name = accountForm.username.split('@')[1] || '';
        }
        showAdvanced = true;
      }
    } catch {
      autodiscoverError = tl('mail_autodiscoverError');
      if (!accountForm.display_name) {
        accountForm.display_name = accountForm.username.split('@')[1] || '';
      }
      showAdvanced = true;
    }
    autodiscovering = false;
    accountStep = 'details';
  }

  async function saveAccount() {
    accountSaving = true;
    accountError = '';
    try {
      const r = await fetch('/api/email/account', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          account_id: accountForm.account_id,
          display_name: accountForm.display_name,
          sender_name: accountForm.sender_name || null,
          host: accountForm.host,
          port: accountForm.port,
          username: accountForm.username,
          password: accountForm.password,
          use_tls: accountForm.use_tls,
          smtp_host: accountForm.smtp_host || null,
          smtp_port: accountForm.smtp_port || null,
        }),
      });
      if (!r.ok) {
        const text = await r.text();
        accountError = text || tl('mail_saveAccountError');
      } else {
        accountDialog?.close();
        await onAccountsChanged();
      }
    } catch {
      accountError = tl('mail_saveAccountError');
    }
    accountSaving = false;
  }

  // ── Identity operations ────────────────────────────────────────────────────

  function openEditIdentity(id: Identity) {
    identityForm = {
      identity_id: id.identity_id,
      account_id: id.account_id,
      name: id.name,
      email: id.email,
    };
    identityMode = 'edit';
    identityError = '';
  }

  function cancelIdentityForm() {
    identityMode = 'idle';
    identityForm = emptyIdentityForm();
    identityForm.account_id = identityAccountId;
    identityError = '';
  }

  async function saveIdentity() {
    identitySaving = true;
    identityError = '';
    try {
      const r = await fetch('/api/email/identity', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          identity_id: identityForm.identity_id,
          account_id: identityForm.account_id,
          name: identityForm.name,
          email: identityForm.email,
          is_default: false,
        }),
      });
      if (!r.ok) {
        identityError = (await r.text()) || tl('mail_saveIdentityError');
      } else {
        await onIdentitiesChanged();
        identityMode = 'idle';
        identityForm = emptyIdentityForm();
        identityForm.account_id = identityAccountId;
      }
    } catch {
      identityError = tl('mail_saveIdentityError');
    }
    identitySaving = false;
  }

  async function setDefaultIdentity(id: Identity) {
    await fetch('/api/email/identity', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        identity_id: id.identity_id,
        account_id: id.account_id,
        name: id.name,
        email: id.email,
        is_default: true,
      }),
    });
    await onIdentitiesChanged();
  }

  async function deleteIdentity(id: Identity) {
    if (!confirm(tl('mail_removeIdentityConfirm', { name: id.name, email: id.email }))) return;
    await fetch(`/api/email/identity?identity_id=${encodeURIComponent(id.identity_id)}`, { method: 'DELETE' });
    await onIdentitiesChanged();
  }
</script>

<!-- ── Identity management dialog ────────────────────────────────────────── -->
<dialog
  bind:this={identityDialog}
  on:click={(e) => { if (e.target === e.currentTarget) identityDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); identityDialog?.close(); }}
>
  <article>
    <header>
      <h3>{tl('mail_identities')}</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label={tl('mail_close')} on:click={() => identityDialog?.close()}></button>
      </div>
    </header>
    <div class="identity-list">
      {#each identities.filter(i => i.account_id === identityAccountId) as id}
        <div class="identity-row">
          <div class="identity-info">
            <span class="identity-name">{id.name}</span>
            <span class="identity-email">&lt;{id.email}&gt;</span>
            {#if id.is_default}<span class="identity-default-badge">{tl('mail_identityDefault')}</span>{/if}
          </div>
          <div class="identity-actions">
            {#if !id.is_default}
              <button class="icon-btn" disabled={identityMode !== 'idle'} on:click={() => setDefaultIdentity(id)} title={tl('mail_identitySetDefault')}>★</button>
            {/if}
            <button class="icon-btn" disabled={identityMode !== 'idle'} on:click={() => openEditIdentity(id)} title={tl('mail_update')}>
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg>
            </button>
            {#if !id.is_default}
              <button class="icon-btn delete-btn" disabled={identityMode !== 'idle'} on:click={() => deleteIdentity(id)} title={tl('mail_removeAttachment')}>×</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
    {#if identityMode === 'idle'}
      <button class="link-btn identity-add-btn" on:click={() => { identityMode = 'add'; identityForm = emptyIdentityForm(); identityForm.account_id = identityAccountId; }}>+ {tl('mail_addIdentity')}</button>
    {:else}
      <form on:submit|preventDefault={saveIdentity} class="identity-form">
        <label>
          {tl('mail_name')}
          <input bind:value={identityForm.name} placeholder={tl('mail_namePlaceholder')} required />
        </label>
        <label>
          {tl('mail_emailAddress')}
          <input type="email" bind:value={identityForm.email} placeholder={tl('mail_aliasPlaceholder')} required />
        </label>
        {#if identityError}
          <p class="error">{identityError}</p>
        {/if}
        <footer>
          <button type="button" class="secondary outline" on:click={cancelIdentityForm}>{tl('mail_cancel')}</button>
          <button type="submit" class="primary" disabled={identitySaving}>
            {identitySaving ? tl('mail_saving') : (identityMode === 'edit' ? tl('mail_update') : tl('mail_add'))}
          </button>
        </footer>
      </form>
    {/if}
  </article>
</dialog>

<!-- ── Account form dialog ───────────────────────────────────────────────── -->
<dialog
  bind:this={accountDialog}
  on:click={(e) => { if (e.target === e.currentTarget) accountDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); accountDialog?.close(); }}
>
  <article>
    <header>
      <h3>
        {accountForm.account_id ? tl('mail_editAccountHeading') : tl('mail_addAccountHeading')}
        {#if accountForm.account_id}
          <a class="header-icon-link" href={`/api/email/export?account_id=${accountForm.account_id}`} download title={tl('mail_exportAccount')}><svg width="1.2rem" height="1.2rem" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg></a>
        {/if}
      </h3>
      <div class="header-actions">
        <button class="close-btn" aria-label={tl('mail_close')} on:click={() => accountDialog?.close()}></button>
      </div>
    </header>

    {#if accountStep === 'basic'}
      <form on:submit|preventDefault={continueSetup}>
        <label>
          {tl('mail_yourName')}
          <input bind:value={accountForm.sender_name} placeholder={tl('mail_namePlaceholder')} required />
        </label>
        <label>
          {tl('mail_emailAddress')}
          <input type="email" bind:value={accountForm.username} placeholder={tl('mail_emailPlaceholder')} required />
        </label>
        {#if autodiscovering}
          <p class="hint">{tl('mail_detectingSettings')}</p>
        {/if}
        {#if autodiscoverError}
          <p class="error">{autodiscoverError}</p>
        {/if}
        <footer>
          <button type="button" class="secondary outline" on:click={() => accountDialog?.close()}>{tl('mail_cancel')}</button>
          <button type="submit" class="primary" disabled={autodiscovering}>{tl('mail_continue')}</button>
        </footer>
      </form>
    {:else}
      <form on:submit|preventDefault={saveAccount}>
        {#if autodiscoverResult?.provider_name}
          <p class="hint">{tl('mail_settingsDetected', { provider: autodiscoverResult.provider_name })}</p>
        {:else if !accountForm.account_id}
          <p class="hint">{tl('mail_enterServerDetails')}</p>
        {/if}
        <label>
          {tl('mail_password')}
          <input type="password" bind:value={accountForm.password} placeholder={accountForm.account_id ? tl('mail_passwordPlaceholderKeep') : ''} autocomplete="current-password" />
        </label>
        <details bind:open={showAdvanced}>
          <summary>{tl('mail_advancedSettings')}</summary>
          <label>
            {tl('mail_displayName')}
            <input bind:value={accountForm.display_name} placeholder={tl('mail_displayNamePlaceholder')} />
          </label>
          <label>
            {tl('mail_usernameEmail')}
            <input bind:value={accountForm.username} placeholder={tl('mail_emailPlaceholder')} />
          </label>
          <label>
            {tl('mail_imapHost')}
            <input bind:value={accountForm.host} placeholder={tl('mail_imapHostPlaceholder')} required />
          </label>
          <label>
            {tl('mail_imapPort')}
            <input type="number" bind:value={accountForm.port} required />
          </label>
          <label class="checkbox-label">
            <input type="checkbox" bind:checked={accountForm.use_tls} /> {tl('mail_useTls')}
          </label>
          <label>
            {tl('mail_smtpHost')}
            <input bind:value={accountForm.smtp_host} placeholder={tl('mail_smtpHostPlaceholder')} />
          </label>
          <label>
            {tl('mail_smtpPort')}
            <input type="number" bind:value={accountForm.smtp_port} />
          </label>
        </details>
        {#if accountError}
          <p class="error">{accountError}</p>
        {/if}
        <footer>
          {#if !accountForm.account_id}
            <button type="button" class="secondary outline" on:click={() => { accountStep = 'basic'; accountError = ''; }}>{tl('mail_back')}</button>
          {:else}
            <button type="button" class="secondary outline" on:click={() => accountDialog?.close()}>{tl('mail_cancel')}</button>
          {/if}
          <button type="submit" class="primary" disabled={accountSaving}>
            {accountSaving ? tl('mail_testingConnection') : tl('mail_save')}
          </button>
        </footer>
      </form>
    {/if}
  </article>
</dialog>

<style>
  .identity-list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }

  .identity-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.5rem;
    border-radius: 4px;
    background: var(--pico-card-sectioning-background-color, rgba(0,0,0,0.03));
  }

  .identity-info {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    overflow: hidden;
  }

  .identity-name {
    font-weight: 500;
    white-space: nowrap;
  }

  .identity-email {
    font-size: 0.8rem;
    color: var(--pico-muted-color, #888);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .identity-default-badge {
    font-size: 0.7rem;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    background: var(--pico-primary-background, #1095c1);
    color: var(--pico-primary-inverse, #fff);
    white-space: nowrap;
  }

  .identity-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    flex-shrink: 0;
  }

  .identity-form {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  .identity-add-btn {
    margin-top: 0.5rem;
  }

  .header-icon-link {
    color: var(--pico-muted-color);
    display: inline-flex;
    margin-left: 0.4rem;
    vertical-align: middle;
  }

  .header-icon-link:hover {
    color: var(--pico-color);
  }

  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }
</style>

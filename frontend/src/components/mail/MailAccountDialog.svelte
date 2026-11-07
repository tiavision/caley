<script lang="ts">
  import { getLocale, makeTl } from '../../lib/i18n.ts';
  import { apiFetch } from '../../lib/api.ts';
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
      const r = await apiFetch(`/api/email/autodiscover?email=${encodeURIComponent(accountForm.username)}`);
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
      const r = await apiFetch('/api/email/account', {
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
      const r = await apiFetch('/api/email/identity', {
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
    await apiFetch('/api/email/identity', {
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
    await apiFetch(`/api/email/identity?identity_id=${encodeURIComponent(id.identity_id)}`, { method: 'DELETE' });
    await onIdentitiesChanged();
  }
</script>

<!-- ── Identity management dialog ────────────────────────────────────────── -->
<dialog id="mail-identities" bind:this={identityDialog} closedby="any">
  <header>
    <h2>{tl('mail_identities')}</h2>
    <button type="button" commandfor="mail-identities" command="close" aria-label={tl('mail_close')}></button>
  </header>
  {#if identities.some(i => i.account_id === identityAccountId)}
    <ul data-list>
      {#each identities.filter(i => i.account_id === identityAccountId) as id}
        <li data-layout="row" data-gap="xs" data-wrap="none" data-pad="xs">
          <span data-layout="row" data-gap="xs" data-wrap="none" data-grow>
            <strong data-truncate>{id.name}</strong>
            <small data-muted data-truncate>&lt;{id.email}&gt;</small>
            {#if id.is_default}<span data-badge data-tone="accent">{tl('mail_identityDefault')}</span>{/if}
          </span>
          {#if !id.is_default}
            <button type="button" data-variant="ghost" data-size="sm" data-icon="star" disabled={identityMode !== 'idle'} on:click={() => setDefaultIdentity(id)} aria-label={tl('mail_identitySetDefault')} data-tooltip></button>
          {/if}
          <button type="button" data-variant="ghost" data-size="sm" data-icon="pencil" disabled={identityMode !== 'idle'} on:click={() => openEditIdentity(id)} aria-label={tl('mail_update')} data-tooltip></button>
          {#if !id.is_default}
            <button type="button" data-variant="ghost" data-size="sm" data-icon="x" disabled={identityMode !== 'idle'} on:click={() => deleteIdentity(id)} aria-label={tl('mail_removeAttachment')} data-tooltip></button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if identityMode === 'idle'}
    <div data-layout="row">
      <button type="button" data-variant="link" data-icon="plus" on:click={() => { identityMode = 'add'; identityForm = emptyIdentityForm(); identityForm.account_id = identityAccountId; }}>{tl('mail_addIdentity')}</button>
    </div>
  {:else}
    <form data-layout="stack" data-gap="md" on:submit|preventDefault={saveIdentity}>
      <label>
        {tl('mail_name')}
        <input bind:value={identityForm.name} placeholder={tl('mail_namePlaceholder')} required />
      </label>
      <label>
        {tl('mail_emailAddress')}
        <input type="email" bind:value={identityForm.email} placeholder={tl('mail_aliasPlaceholder')} required />
      </label>
      {#if identityError}
        <p data-callout data-tone="danger" role="alert">{identityError}</p>
      {/if}
      <footer>
        <button type="button" data-variant="secondary" on:click={cancelIdentityForm}>{tl('mail_cancel')}</button>
        <button type="submit" disabled={identitySaving} aria-busy={identitySaving}>
          {identitySaving ? tl('mail_saving') : (identityMode === 'edit' ? tl('mail_update') : tl('mail_add'))}
        </button>
      </footer>
    </form>
  {/if}
</dialog>

<!-- ── Account form dialog ───────────────────────────────────────────────── -->
<dialog id="mail-account" bind:this={accountDialog} closedby="any">
  <header>
    <h2>{accountForm.account_id ? tl('mail_editAccountHeading') : tl('mail_addAccountHeading')}</h2>
    {#if accountForm.account_id}
      <a href={`/api/email/export?account_id=${accountForm.account_id}`} download role="button" data-variant="ghost" data-icon="download" aria-label={tl('mail_exportAccount')} data-tooltip></a>
    {/if}
    <button type="button" commandfor="mail-account" command="close" aria-label={tl('mail_close')}></button>
  </header>

  {#if accountStep === 'basic'}
    <form data-layout="stack" data-gap="md" on:submit|preventDefault={continueSetup}>
      <label>
        {tl('mail_yourName')}
        <input bind:value={accountForm.sender_name} placeholder={tl('mail_namePlaceholder')} required />
      </label>
      <label>
        {tl('mail_emailAddress')}
        <input type="email" bind:value={accountForm.username} placeholder={tl('mail_emailPlaceholder')} required />
      </label>
      {#if autodiscovering}
        <p data-muted aria-busy="true">{tl('mail_detectingSettings')}</p>
      {/if}
      {#if autodiscoverError}
        <p data-callout data-tone="danger" role="alert">{autodiscoverError}</p>
      {/if}
      <footer>
        <button type="button" data-variant="secondary" commandfor="mail-account" command="close">{tl('mail_cancel')}</button>
        <button type="submit" disabled={autodiscovering} aria-busy={autodiscovering}>{tl('mail_continue')}</button>
      </footer>
    </form>
  {:else}
    <form data-layout="stack" data-gap="md" on:submit|preventDefault={saveAccount}>
      {#if autodiscoverResult?.provider_name}
        <p data-muted>{tl('mail_settingsDetected', { provider: autodiscoverResult.provider_name })}</p>
      {:else if !accountForm.account_id}
        <p data-muted>{tl('mail_enterServerDetails')}</p>
      {/if}
      <label>
        {tl('mail_password')}
        <input type="password" bind:value={accountForm.password} placeholder={accountForm.account_id ? tl('mail_passwordPlaceholderKeep') : ''} autocomplete="current-password" />
      </label>
      <details bind:open={showAdvanced}>
        <summary>{tl('mail_advancedSettings')}</summary>
        <div data-layout="stack" data-gap="md">
          <label>
            {tl('mail_displayName')}
            <input bind:value={accountForm.display_name} placeholder={tl('mail_displayNamePlaceholder')} />
          </label>
          <label>
            {tl('mail_usernameEmail')}
            <input bind:value={accountForm.username} placeholder={tl('mail_emailPlaceholder')} />
          </label>
          <div data-layout="grid" data-gap="sm" style="--tm-min: 9rem">
            <label>
              {tl('mail_imapHost')}
              <input bind:value={accountForm.host} placeholder={tl('mail_imapHostPlaceholder')} required />
            </label>
            <label>
              {tl('mail_imapPort')}
              <input type="number" bind:value={accountForm.port} required />
            </label>
          </div>
          <label>
            <input type="checkbox" bind:checked={accountForm.use_tls} /> {tl('mail_useTls')}
          </label>
          <div data-layout="grid" data-gap="sm" style="--tm-min: 9rem">
            <label>
              {tl('mail_smtpHost')}
              <input bind:value={accountForm.smtp_host} placeholder={tl('mail_smtpHostPlaceholder')} />
            </label>
            <label>
              {tl('mail_smtpPort')}
              <input type="number" bind:value={accountForm.smtp_port} />
            </label>
          </div>
        </div>
      </details>
      {#if accountError}
        <p data-callout data-tone="danger" role="alert">{accountError}</p>
      {/if}
      <footer>
        {#if !accountForm.account_id}
          <button type="button" data-variant="secondary" on:click={() => { accountStep = 'basic'; accountError = ''; }}>{tl('mail_back')}</button>
        {:else}
          <button type="button" data-variant="secondary" commandfor="mail-account" command="close">{tl('mail_cancel')}</button>
        {/if}
        <button type="submit" disabled={accountSaving} aria-busy={accountSaving}>
          {accountSaving ? tl('mail_testingConnection') : tl('mail_save')}
        </button>
      </footer>
    </form>
  {/if}
</dialog>

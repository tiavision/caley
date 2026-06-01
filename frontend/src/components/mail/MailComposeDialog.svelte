<script lang="ts">
  import { getLocale, makeTl } from '../../lib/i18n.ts';
  import { apiFetch } from '../../lib/api.ts';
  import SelectDropdown from '../SelectDropdown.svelte';
  import UiIcon from '../UiIcon.svelte';
  import { readVault, decryptPrivateKeys, localActiveDevice, rememberLocalDevice, unwrapVaultKey, validateVault, eraseBytes } from '../../lib/pgp/vault.ts';
  import { unlock } from '../../lib/pgp/webauthn.ts';
  import { getKeys } from '../../lib/pgp/session.ts';
  import { parsePublicKey, listPublicKeys, type StoredPublicKey } from '../../lib/pgp/public-keys.ts';
  import { buildProtectedMime, parseEnvelopeAddresses } from '../../lib/pgp/compose.ts';
  import { formatMessageDateTime, keyHasEmail } from './mailUtils.ts';
  import type { Identity, Message } from './MailTypes.ts';
  import type * as openpgp from 'openpgp';

  export let identities: Identity[] = [];
  export let contactSuggestions: { name: string; email: string }[] = [];
  export let onEnsureContactSuggestions: () => void = () => {};

  let locale = getLocale();
  $: tl = makeTl(locale);

  // ── Compose state ─────────────────────────────────────────────────────────

  let dialog: HTMLDialogElement;
  let composeTo = '';
  let composeCc = '';
  let composeBcc = '';
  let composeSubject = '';
  let composeBody = '';
  let composeIdentityId = '';
  let composeInReplyTo: string | null = null;
  let composeReferences: string | null = null;
  let composeMode: 'new' | 'reply' | 'replyAll' | 'forward' = 'new';
  let composeSending = false;
  let composeError = '';
  let showComposeCcBcc = false;
  let composeAttachments: File[] = [];
  let composeEncrypt = false;
  let composeSign = false;
  let composeCanEncrypt = false;
  let composeCanSign = false;
  let composeSignOptedOut = false;
  let composeTrustedKeys: { record: StoredPublicKey; key: openpgp.PublicKey }[] = [];
  let composeStoredSigningFingerprints = new Set<string>();
  let composeKeysLoading = false;
  let composeKeysRequest = 0;
  let composeFileInput: HTMLInputElement;
  let filePickerOpen = false;
  let recipientSuggestions: string[] = [];
  let recipientSuggestionIdx = -1;
  let recipientActiveField: 'to' | 'cc' | 'bcc' | null = null;
  let recipientSuggestTimer: ReturnType<typeof setTimeout> | null = null;

  // ── PGP key helpers ────────────────────────────────────────────────────────

  async function trustedPublicKeys(): Promise<{ record: StoredPublicKey; key: openpgp.PublicKey }[]> {
    const records = (await listPublicKeys()).filter(record => record.trust === 'trusted');
    const parsed: { record: StoredPublicKey; key: openpgp.PublicKey }[] = [];
    for (const record of records) {
      try {
        parsed.push({ record, key: await parsePublicKey(record.armored) });
      } catch {}
    }
    return parsed;
  }

  function signingKeyForIdentity(identityId: string): openpgp.PrivateKey | null {
    const identity = identities.find(item => item.identity_id === identityId);
    if (!identity) return null;
    return getKeys().find(key => keyHasEmail(key, identity.email)) ?? null;
  }

  function storedSigningKeyExists(
    identityId: string,
    keys: { record: StoredPublicKey; key: openpgp.PublicKey }[],
    fingerprints: Set<string>,
  ): boolean {
    const identity = identities.find(item => item.identity_id === identityId);
    if (!identity) return false;
    return keys.some(({ record, key }) =>
      record.is_own
      && fingerprints.has(record.fingerprint.toUpperCase())
      && keyHasEmail(key, identity.email)
    );
  }

  function reconcileComposeSigning(
    identityId: string,
    optedOut: boolean,
    keys: { record: StoredPublicKey; key: openpgp.PublicKey }[],
    fingerprints: Set<string>,
  ) {
    composeCanSign = signingKeyForIdentity(identityId) !== null
      || storedSigningKeyExists(identityId, keys, fingerprints);
    if (!composeCanSign) {
      composeSign = false;
    } else if (!optedOut) {
      composeSign = true;
    }
  }

  function composeEncryptionSelection(
    identityId: string,
    toValue: string,
    ccValue: string,
    bccValue: string,
    keys: { record: StoredPublicKey; key: openpgp.PublicKey }[],
  ): { keys: openpgp.PublicKey[]; missing: string[] } {
    const identity = identities.find(item => item.identity_id === identityId);
    const recipients = [
      ...parseEnvelopeAddresses(toValue),
      ...parseEnvelopeAddresses(ccValue),
      ...parseEnvelopeAddresses(bccValue),
    ];
    if (!identity || recipients.length === 0) {
      return { keys: [], missing: identity ? recipients : [''] };
    }
    const encryptionKeys: openpgp.PublicKey[] = [];
    const missing: string[] = [];
    for (const recipient of recipients) {
      const matching = keys.filter(({ key }) => keyHasEmail(key, recipient));
      if (matching.length === 0) {
        missing.push(recipient);
      } else {
        for (const { key } of matching) encryptionKeys.push(key);
      }
    }
    const ownKeys = keys.filter(({ record, key }) => record.is_own && keyHasEmail(key, identity.email));
    if (ownKeys.length === 0) {
      missing.push(identity.email);
    } else {
      for (const { key } of ownKeys) encryptionKeys.push(key);
    }
    return { keys: encryptionKeys, missing: [...new Set(missing)] };
  }

  function reconcileComposeEncryption(
    identityId: string,
    toValue: string,
    ccValue: string,
    bccValue: string,
    keys: { record: StoredPublicKey; key: openpgp.PublicKey }[],
    loading: boolean,
  ) {
    const selection = composeEncryptionSelection(identityId, toValue, ccValue, bccValue, keys);
    composeCanEncrypt = !loading && selection.keys.length > 0 && selection.missing.length === 0;
    if (!composeCanEncrypt) composeEncrypt = false;
  }

  async function loadComposeTrustedKeys() {
    const request = ++composeKeysRequest;
    composeKeysLoading = true;
    composeTrustedKeys = [];
    composeStoredSigningFingerprints = new Set();
    try {
      const [keys, vault] = await Promise.all([
        trustedPublicKeys(),
        readVault().catch(() => null),
      ]);
      if (request === composeKeysRequest) {
        composeTrustedKeys = keys;
        composeStoredSigningFingerprints = vault && localActiveDevice(vault)
          ? new Set(vault.keys.map(record => record.fingerprint.toUpperCase()))
          : new Set();
      }
    } catch {
      if (request === composeKeysRequest) {
        composeTrustedKeys = [];
        composeStoredSigningFingerprints = new Set();
      }
    } finally {
      if (request === composeKeysRequest) composeKeysLoading = false;
    }
  }

  function toggleComposeSign() {
    if (!composeCanSign) return;
    composeSign = !composeSign;
    composeSignOptedOut = !composeSign;
  }

  function toggleComposeEncrypt() {
    if (composeCanEncrypt) composeEncrypt = !composeEncrypt;
  }

  $: reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
  $: reconcileComposeEncryption(composeIdentityId, composeTo, composeCc, composeBcc, composeTrustedKeys, composeKeysLoading);

  // ── Quote helpers ──────────────────────────────────────────────────────────

  function cleanQuotedBody(message: Message): string {
    return (message.body_text ?? '').replace(/\r\n?/g, '\n').trim();
  }

  function quoteHeaderLines(message: Message, includeSubject = false): string[] {
    const lines: string[] = [];
    const date = formatMessageDateTime(message.sent_at);
    if (date) lines.push(`Date: ${date}`);
    if (message.from) lines.push(`From: ${message.from}`);
    if (message.to.length > 0) lines.push(`To: ${message.to.join(', ')}`);
    if (message.cc.length > 0) lines.push(`Cc: ${message.cc.join(', ')}`);
    if (includeSubject) lines.push(`Subject: ${message.subject || tl('mail_noSubject')}`);
    return lines;
  }

  function replyQuote(message: Message): string {
    const header = quoteHeaderLines(message, false);
    const quotedLines = [...header, '', cleanQuotedBody(message)]
      .join('\n')
      .split('\n')
      .map(line => line ? `> ${line}` : '>')
      .join('\n');
    return `\n\n${quotedLines}`;
  }

  function forwardQuote(message: Message): string {
    const lines = [
      '',
      '',
      '--- Forwarded message ---',
      ...quoteHeaderLines(message, true),
      '',
      cleanQuotedBody(message),
    ];
    return lines.join('\n').replace(/\n+$/, '');
  }

  // ── Public open/close interface ────────────────────────────────────────────

  export function open(options: {
    mode?: 'new' | 'reply' | 'replyAll' | 'forward';
    replyTo?: Message;
    subject?: string;
    attachments?: File[];
  } = {}) {
    composeError = '';
    composeSending = false;
    composeCc = '';
    composeBcc = '';
    composeAttachments = options.attachments ?? [];
    showComposeCcBcc = false;
    composeEncrypt = false;
    composeSign = false;
    composeSignOptedOut = false;

    const mode = options.mode ?? 'new';

    if (mode === 'reply' && options.replyTo) {
      const replyTo = options.replyTo;
      composeMode = 'reply';
      composeTo = replyTo.from || '';
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = replyQuote(replyTo);
      composeInReplyTo = replyTo.message_id;
      composeReferences = replyTo.message_id;
      const accountIdentities = identities.filter(i => i.account_id === replyTo.account_id);
      const toAddresses = (replyTo.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
    } else if (mode === 'replyAll' && options.replyTo) {
      const replyTo = options.replyTo;
      composeMode = 'reply';
      composeTo = replyTo.from || '';
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = replyQuote(replyTo);
      composeInReplyTo = replyTo.message_id;
      composeReferences = replyTo.message_id;
      const accountIdentities = identities.filter(i => i.account_id === replyTo.account_id);
      const toAddresses = (replyTo.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
      const myIdentity = identities.find(i => i.identity_id === composeIdentityId);
      const myEmail = myIdentity?.email.toLowerCase() ?? '';
      const allRecipients = [...(replyTo.to ?? []), ...(replyTo.cc ?? [])];
      const ccAddresses = allRecipients
        .filter(addr => {
          const a = addr.toLowerCase();
          return !a.includes(myEmail) && a !== (replyTo.from ?? '').toLowerCase();
        })
        .join(', ');
      composeCc = ccAddresses;
      showComposeCcBcc = ccAddresses.length > 0;
      composeMode = 'replyAll';
    } else if (mode === 'forward' && options.replyTo) {
      const forwardFrom = options.replyTo;
      composeMode = 'forward';
      composeTo = '';
      const accountIdentities = identities.filter(i => i.account_id === forwardFrom.account_id);
      composeIdentityId = (accountIdentities.find(i => i.is_default) ?? accountIdentities[0] ?? identities[0])?.identity_id ?? '';
      composeSubject = /^fwd:/i.test(forwardFrom.subject ?? '')
        ? (forwardFrom.subject ?? '')
        : `Fwd: ${forwardFrom.subject || ''}`;
      composeBody = forwardQuote(forwardFrom);
      composeInReplyTo = null;
      composeReferences = null;
    } else {
      composeMode = 'new';
      composeTo = '';
      composeSubject = options.subject ?? '';
      composeBody = '';
      composeInReplyTo = null;
      composeReferences = null;
      composeIdentityId = (identities.find(i => i.is_default) ?? identities[0])?.identity_id ?? '';
    }

    reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
    void loadComposeTrustedKeys();
    dialog?.showModal();
  }

  export function close() {
    dialog?.close();
  }

  // ── Recipient suggestions ──────────────────────────────────────────────────

  function getRecipientToken(value: string): string {
    const parts = value.split(',');
    return parts[parts.length - 1].trim();
  }

  async function fetchRecipientSuggestions(token: string) {
    if (token.length < 2) { recipientSuggestions = []; return; }
    const q = token.toLowerCase();
    const contactMatches = contactSuggestions
      .filter(c => c.email.toLowerCase().includes(q) || c.name.toLowerCase().includes(q))
      .slice(0, 5)
      .map(c => c.name ? `${c.name} <${c.email}>` : c.email);
    const contactEmailSet = new Set(
      contactSuggestions
        .filter(c => c.email.toLowerCase().includes(q) || c.name.toLowerCase().includes(q))
        .map(c => c.email.toLowerCase())
    );
    try {
      const res = await apiFetch(`/api/email/suggest-addrs?q=${encodeURIComponent(token)}`);
      if (res.ok) {
        const data = await res.json();
        const apiOnly = (data.addrs as string[]).filter(a => !contactEmailSet.has(a.toLowerCase()));
        recipientSuggestions = [...contactMatches, ...apiOnly].slice(0, 7);
      } else {
        recipientSuggestions = contactMatches;
      }
    } catch { recipientSuggestions = contactMatches; }
  }

  function onRecipientInput(field: 'to' | 'cc' | 'bcc', value: string) {
    onEnsureContactSuggestions();
    recipientActiveField = field;
    recipientSuggestionIdx = -1;
    const token = getRecipientToken(value);
    if (recipientSuggestTimer) clearTimeout(recipientSuggestTimer);
    recipientSuggestTimer = setTimeout(() => fetchRecipientSuggestions(token), 200);
  }

  function applyRecipientSuggestion(addr: string) {
    const getCurrent = () => recipientActiveField === 'to' ? composeTo : recipientActiveField === 'cc' ? composeCc : composeBcc;
    const setCurrent = (v: string) => { if (recipientActiveField === 'to') composeTo = v; else if (recipientActiveField === 'cc') composeCc = v; else composeBcc = v; };
    const current = getCurrent();
    const lastComma = current.lastIndexOf(',');
    const prefix = lastComma >= 0 ? current.slice(0, lastComma + 1) + ' ' : '';
    setCurrent(prefix + addr + ', ');
    recipientSuggestions = [];
    recipientSuggestionIdx = -1;
  }

  function onRecipientKeydown(e: KeyboardEvent) {
    if (!recipientSuggestions.length) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      recipientSuggestionIdx = Math.min(recipientSuggestionIdx + 1, recipientSuggestions.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      recipientSuggestionIdx = Math.max(recipientSuggestionIdx - 1, -1);
    } else if (e.key === 'Enter' && recipientSuggestionIdx >= 0) {
      e.preventDefault();
      applyRecipientSuggestion(recipientSuggestions[recipientSuggestionIdx]);
    } else if (e.key === 'Tab') {
      if (recipientSuggestionIdx >= 0) {
        e.preventDefault();
        applyRecipientSuggestion(recipientSuggestions[recipientSuggestionIdx]);
      } else if (recipientSuggestions.length === 1) {
        e.preventDefault();
        applyRecipientSuggestion(recipientSuggestions[0]);
      }
    } else if (e.key === 'Escape') {
      recipientSuggestions = [];
      recipientSuggestionIdx = -1;
    }
  }

  function onRecipientBlur() {
    setTimeout(() => {
      recipientSuggestions = [];
      recipientSuggestionIdx = -1;
      recipientActiveField = null;
    }, 150);
  }

  // ── Send ───────────────────────────────────────────────────────────────────

  async function unlockVaultForSigning(): Promise<boolean> {
    try {
      const vault = await readVault();
      if (!vault) return false;
      const device = localActiveDevice(vault);
      if (!device) return false;
      const { prfOutput } = await unlock(device.prf_salt, device.cred_id);
      const { raw, key: vaultKey } = await unwrapVaultKey(device, prfOutput);
      try {
        await validateVault(vault, vaultKey, device.cred_id);
        const keys = await decryptPrivateKeys(vaultKey, vault.keys);
        const { openVault } = await import('../../lib/pgp/session.ts');
        openVault(vaultKey, keys);
        rememberLocalDevice(device.cred_id);
        return true;
      } finally {
        eraseBytes(raw);
      }
    } catch {
      return false;
    }
  }

  async function sendEmail() {
    composeSending = true;
    composeError = '';
    try {
      let r: Response;
      if (composeEncrypt || composeSign) {
        const identity = identities.find(item => item.identity_id === composeIdentityId);
        if (!identity) throw new Error(tl('pgp_composeIdentityMissing'));
        const to = parseEnvelopeAddresses(composeTo);
        const cc = parseEnvelopeAddresses(composeCc);
        const bcc = parseEnvelopeAddresses(composeBcc);
        const allRecipients = [...to, ...cc, ...bcc];
        if (allRecipients.length === 0) throw new Error(tl('pgp_composeNoRecipients'));

        let signingKey = composeSign ? signingKeyForIdentity(composeIdentityId) : null;
        if (composeSign && !signingKey && composeCanSign) {
          await unlockVaultForSigning();
          signingKey = signingKeyForIdentity(composeIdentityId);
        }
        if (composeSign && !signingKey) throw new Error(tl('pgp_composeUnlockSigningKey'));

        let encryptionKeys: openpgp.PublicKey[] = [];
        if (composeEncrypt) {
          const selection = composeEncryptionSelection(
            composeIdentityId,
            composeTo,
            composeCc,
            composeBcc,
            await trustedPublicKeys(),
          );
          if (selection.missing.length > 0) throw new Error(tl('pgp_composeMissingKeys', { emails: selection.missing.join(', ') }));
          encryptionKeys = selection.keys;
        }
        const rawMime = await buildProtectedMime({
          identity,
          to,
          cc,
          subject: composeSubject,
          body: composeBody,
          inReplyTo: composeInReplyTo,
          references: composeReferences,
          attachments: composeAttachments,
          encrypt: composeEncrypt,
          sign: composeSign,
          encryptionKeys,
          signingKey,
        });
        const fd = new FormData();
        fd.append('identity_id', composeIdentityId);
        fd.append('envelope_to', JSON.stringify(to));
        fd.append('envelope_cc', JSON.stringify(cc));
        fd.append('envelope_bcc', JSON.stringify(bcc));
        fd.append('raw_mime', new Blob([rawMime], { type: 'message/rfc822' }), 'message.eml');
        r = await apiFetch('/api/email/send-raw', { method: 'POST', body: fd });
      } else {
        const fd = new FormData();
        fd.append('identity_id', composeIdentityId);
        fd.append('to', composeTo);
        fd.append('cc', composeCc);
        fd.append('bcc', composeBcc);
        fd.append('subject', composeSubject);
        fd.append('body', composeBody);
        if (composeInReplyTo) fd.append('in_reply_to', composeInReplyTo);
        if (composeReferences) fd.append('references', composeReferences);
        for (const file of composeAttachments) {
          fd.append('attachment', file, file.name);
        }
        r = await apiFetch('/api/email/send', { method: 'POST', body: fd });
      }
      if (!r.ok) {
        const text = await r.text();
        composeError = text || tl('mail_sendError');
      } else {
        dialog?.close();
      }
    } catch (error) {
      composeError = error instanceof Error ? error.message : tl('mail_sendError');
    }
    composeSending = false;
  }

  // ── File attachments ───────────────────────────────────────────────────────

  function openFilePicker() {
    filePickerOpen = true;
    composeFileInput.click();
  }

  function addAttachments(files: FileList | null) {
    filePickerOpen = false;
    if (!files) return;
    composeAttachments = [...composeAttachments, ...Array.from(files)];
    composeFileInput.value = '';
  }

  function removeAttachment(index: number) {
    composeAttachments = composeAttachments.filter((_, i) => i !== index);
  }

  function formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<dialog
  bind:this={dialog}
  class="compose-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) dialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); if (filePickerOpen) { filePickerOpen = false; } else { dialog?.close(); } }}
>
  <article>
    <header class="compose-header">
      <span class="compose-title">{composeMode === 'forward' ? tl('mail_forward') : composeMode === 'replyAll' ? tl('mail_replyAll') : composeMode === 'reply' ? tl('mail_reply') : tl('mail_newMessage')}</span>
      <button class="close-btn" aria-label={tl('mail_close')} on:click={() => dialog?.close()}></button>
    </header>
    <form on:submit|preventDefault={sendEmail} class="compose-form">
      {#if identities.length > 1}
        <div class="compose-row">
          <span class="compose-label">{tl('mail_composeFrom')}</span>
          <SelectDropdown
            options={identities.map(id => ({ value: id.identity_id, label: id.name, secondary: `<${id.email}>` }))}
            bind:value={composeIdentityId}
          />
        </div>
      {/if}
      <div class="compose-row">
        <span class="compose-label">{tl('mail_composeTo')}</span>
        <div class="compose-input-wrap">
          <input class="compose-input" bind:value={composeTo} placeholder={tl('mail_composeToPlaceholder')}
            on:input={(e) => onRecipientInput('to', e.currentTarget.value)}
            on:keydown={onRecipientKeydown}
            on:blur={onRecipientBlur}
          />
          {#if recipientActiveField === 'to' && recipientSuggestions.length > 0}
            <ul class="addr-suggestions" role="listbox">
              {#each recipientSuggestions as addr, i}
                <li role="option" aria-selected={i === recipientSuggestionIdx}>
                  <button type="button" class:active={i === recipientSuggestionIdx}
                    on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                  >{addr}</button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
        <div class="compose-security-controls">
          <button
            type="button"
            class="compose-security-toggle"
            class:active={composeEncrypt}
            disabled={!composeCanEncrypt || composeSending}
            title={tl('pgp_composeEncrypt')}
            aria-label={tl('pgp_composeEncrypt')}
            aria-pressed={composeEncrypt}
            on:click={toggleComposeEncrypt}
          >
            <UiIcon name="lock" size={16} />
          </button>
          <button
            type="button"
            class="compose-security-toggle"
            class:active={composeSign}
            disabled={!composeCanSign || composeSending}
            title={tl('pgp_composeSign')}
            aria-label={tl('pgp_composeSign')}
            aria-pressed={composeSign}
            on:click={toggleComposeSign}
          >
            <UiIcon name="signature" size={17} />
          </button>
        </div>
        {#if !showComposeCcBcc}
          <button type="button" class="ccbcc-toggle" on:click={() => { showComposeCcBcc = true; }}>{tl('mail_composeCcBcc')}</button>
        {/if}
      </div>
      {#if showComposeCcBcc}
        <div class="compose-row">
          <span class="compose-label">{tl('mail_composeCc')}</span>
          <div class="compose-input-wrap">
            <input class="compose-input" bind:value={composeCc} placeholder={tl('mail_composeCcPlaceholder')}
              on:input={(e) => onRecipientInput('cc', e.currentTarget.value)}
              on:keydown={onRecipientKeydown}
              on:blur={onRecipientBlur}
            />
            {#if recipientActiveField === 'cc' && recipientSuggestions.length > 0}
              <ul class="addr-suggestions" role="listbox">
                {#each recipientSuggestions as addr, i}
                  <li role="option" aria-selected={i === recipientSuggestionIdx}>
                    <button type="button" class:active={i === recipientSuggestionIdx}
                      on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                    >{addr}</button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        </div>
        <div class="compose-row">
          <span class="compose-label">{tl('mail_composeBcc')}</span>
          <div class="compose-input-wrap">
            <input class="compose-input" bind:value={composeBcc} placeholder={tl('mail_composeBccPlaceholder')}
              on:input={(e) => onRecipientInput('bcc', e.currentTarget.value)}
              on:keydown={onRecipientKeydown}
              on:blur={onRecipientBlur}
            />
            {#if recipientActiveField === 'bcc' && recipientSuggestions.length > 0}
              <ul class="addr-suggestions" role="listbox">
                {#each recipientSuggestions as addr, i}
                  <li role="option" aria-selected={i === recipientSuggestionIdx}>
                    <button type="button" class:active={i === recipientSuggestionIdx}
                      on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                    >{addr}</button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        </div>
      {/if}
      <div class="compose-row">
        <span class="compose-label">{tl('mail_composeSubject')}</span>
        <input class="compose-input" bind:value={composeSubject} />
      </div>
      <div class="compose-body-row">
        <textarea bind:value={composeBody} rows={10}></textarea>
      </div>
      {#if composeAttachments.length > 0}
        <ul class="compose-attachments">
          {#each composeAttachments as file, i}
            <li class="compose-attachment-chip">
              <span class="attachment-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg></span>
              <span class="attachment-name" title={file.name}>{file.name}</span>
              <span class="attachment-size">{formatFileSize(file.size)}</span>
              <button type="button" class="attachment-remove" aria-label={tl('mail_removeAttachment')} on:click={() => removeAttachment(i)}>✕</button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if composeError}
        <p class="error compose-error">{composeError}</p>
      {/if}
      <input
        bind:this={composeFileInput}
        type="file"
        multiple
        style="display:none"
        on:change={(e) => addAttachments(e.currentTarget.files)}
      />
      <footer class="compose-footer">
        <button type="button" class="compose-attach-btn" aria-label={tl('mail_attachFiles')} on:click={openFilePicker}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>
          {tl('mail_attach')}
        </button>
        <div class="compose-footer-right">
          <button type="button" class="secondary outline" on:click={() => dialog?.close()}>{tl('mail_discard')}</button>
          <button type="submit" class="primary" disabled={composeSending}>
            {composeSending ? tl('mail_sending') : tl('mail_send')}
          </button>
        </div>
      </footer>
    </form>
  </article>
</dialog>

<style>
  .compose-dialog {
    max-width: 44rem;
    width: 95vw;
    max-height: 90dvh;
    padding: 0;
    overflow: hidden;
  }

  .compose-dialog article {
    padding: 0;
    margin: 0;
    overflow: hidden;
  }

  .compose-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.6rem 1rem;
    margin: 0 !important;
    border-bottom: 1px solid var(--pico-muted-border-color);
    background: var(--pico-card-sectioning-background-color);
  }

  .compose-title {
    font-size: 0.9rem;
    font-weight: 600;
    color: var(--pico-color);
  }

  .compose-form {
    display: flex;
    flex-direction: column;
    font-size: 0.9rem;
    color: var(--pico-color);
  }

  .compose-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color);
  }

  .compose-label {
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    width: 3.5rem;
    flex-shrink: 0;
    text-align: right;
  }

  .compose-input-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .compose-input {
    flex: 1;
    min-width: 0;
    border: none !important;
    outline: none !important;
    box-shadow: none !important;
    background: transparent !important;
    padding: 0.45rem 0 !important;
    margin: 0 !important;
    font-size: 0.9rem;
    color: inherit;
  }

  .addr-suggestions {
    position: absolute; top: 100%; left: 0; right: 0; z-index: 10;
    margin: 0.125rem 0 0; padding: 0.25rem; list-style: none;
    background: var(--pico-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    box-shadow: 0 0.25rem 0.75rem rgba(0,0,0,0.15);
  }
  .addr-suggestions li { list-style: none; padding: 0; margin: 0; }
  .addr-suggestions button {
    width: 100%; text-align: left; border: none; background: none; margin: 0;
    padding: 0.375rem 0.75rem; cursor: pointer; font-size: 0.875rem;
    border-radius: calc(var(--pico-border-radius, 0.25rem) - 0.125rem);
    color: var(--pico-color); transition: background 0.1s;
  }
  .addr-suggestions button:hover,
  .addr-suggestions button.active {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
  }

  .compose-input:focus {
    border: none !important;
    box-shadow: none !important;
  }

  .ccbcc-toggle {
    font-size: 0.75rem;
    color: var(--pico-muted-color);
    background: none;
    border: none;
    padding: 0.2rem 0.4rem;
    cursor: pointer;
    flex-shrink: 0;
    margin: 0;
    line-height: 1;
  }

  .ccbcc-toggle:hover {
    color: var(--pico-color);
    background: var(--pico-secondary-background);
    border-radius: 0.25rem;
  }

  .compose-security-controls {
    display: flex;
    align-items: center;
    gap: 0.15rem;
    flex-shrink: 0;
  }

  .compose-security-toggle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.7rem;
    height: 1.7rem;
    padding: 0;
    margin: 0;
    border: none;
    border-radius: 0.3rem;
    background: transparent;
    color: var(--pico-muted-color);
  }

  .compose-security-toggle:hover:not(:disabled) {
    color: var(--pico-color);
    background: var(--pico-secondary-background);
  }

  .compose-security-toggle.active {
    color: var(--pico-primary);
    background: color-mix(in srgb, var(--pico-primary) 15%, transparent);
  }

  .compose-security-toggle:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .compose-body-row {
    padding: 0.25rem 1rem 0;
  }

  .compose-body-row textarea {
    width: 100%;
    box-sizing: border-box;
    border: none !important;
    outline: none !important;
    box-shadow: none !important;
    background: transparent !important;
    resize: vertical;
    padding: 0.5rem 0 !important;
    margin: 0 !important;
    font-size: 0.9rem;
    color: var(--pico-color);
  }

  .compose-body-row textarea:focus {
    border: none !important;
    box-shadow: none !important;
  }

  .compose-error {
    margin: 0.25rem 1rem;
    font-size: 0.85rem;
  }

  .compose-attachments {
    list-style: none;
    margin: 0;
    padding: 0.4rem 1rem;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    border-top: 1px solid var(--pico-muted-border-color);
  }

  .attachment-remove {
    background: none;
    border: none;
    padding: 0 0.1rem;
    margin: 0;
    cursor: pointer;
    color: var(--pico-muted-color);
    font-size: 0.7rem;
    line-height: 1;
    flex-shrink: 0;
  }

  .attachment-remove:hover {
    color: var(--pico-color);
  }

  .compose-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.6rem 1rem;
    border-top: 1px solid var(--pico-muted-border-color);
    background: var(--pico-card-sectioning-background-color);
    margin: 0;
  }

  .compose-footer-right {
    display: flex;
    gap: 0.5rem;
  }

  .compose-footer button {
    margin: 0;
    padding: 0.4rem 1rem;
    font-size: 0.85rem;
  }

  .compose-attach-btn {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    background: none;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.35rem 0.7rem !important;
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    cursor: pointer;
  }

  .compose-attach-btn:hover {
    color: var(--pico-color);
    border-color: var(--pico-color);
    background: none;
  }
</style>

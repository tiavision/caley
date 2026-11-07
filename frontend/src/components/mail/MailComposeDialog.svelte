<script lang="ts">
  import { getLocale, makeTl } from '../../lib/i18n.ts';
  import { apiFetch } from '../../lib/api.ts';
  import { readVault, decryptPrivateKeys, localActiveDevice, rememberLocalDevice, unwrapVaultKey, validateVault, eraseBytes } from '../../lib/pgp/vault.ts';
  import { unlock } from '../../lib/pgp/webauthn.ts';
  import { getKeys, isUnlocked } from '../../lib/pgp/session.ts';
  import { parsePublicKey, listPublicKeys, type StoredPublicKey } from '../../lib/pgp/public-keys.ts';
  import { buildProtectedMime, parseEnvelopeAddresses } from '../../lib/pgp/compose.ts';
  import { prepareEmailHtml, cidReferencesFromHtml } from '../../lib/mail-html.ts';
  import MailHtmlFrame from './MailHtmlFrame.svelte';
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
  let composeForwardHtml: string | null = null;
  let composeForwardRef: { account_id: string; mailbox: string; uid: number } | null = null;
  let composeForwardQuoteText: string | null = null;
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
    if (!identities.find(item => item.identity_id === identityId)) return false;
    return keys.some(({ record }) =>
      record.is_own && fingerprints.has(record.fingerprint.toUpperCase())
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

  function cleanQuotedBody(parsedText: string): string {
    return parsedText.replace(/\r\n?/g, '\n').trim();
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

  function replyQuote(message: Message, parsedText: string): string {
    const header = quoteHeaderLines(message, false);
    const quotedLines = [...header, '', cleanQuotedBody(parsedText)]
      .join('\n')
      .split('\n')
      .map(line => line ? `> ${line}` : '>')
      .join('\n');
    return `\n\n${quotedLines}`;
  }

  function forwardQuote(message: Message, parsedText: string): string {
    const lines = [
      '',
      '',
      '--- Forwarded message ---',
      ...quoteHeaderLines(message, true),
      '',
      cleanQuotedBody(parsedText),
    ];
    return lines.join('\n').replace(/\n+$/, '');
  }

  function buildForwardHtml(msg: Message, parsedHtml: string | null): string | null {
    if (!parsedHtml) return null;
    // Strip outer html/head/body document wrappers if present
    let bodyContent = parsedHtml;
    const bodyMatch = bodyContent.match(/<body[^>]*>([\s\S]*?)<\/body>/i);
    if (bodyMatch) {
      bodyContent = bodyMatch[1];
    } else {
      bodyContent = bodyContent
        .replace(/^[\s\S]*?<html[^>]*>/i, '')
        .replace(/<\/html>\s*$/i, '')
        .replace(/^[\s\S]*?<\/head>/i, '')
        .trim();
    }
    const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const headerLines: string[] = [];
    if (msg.from) headerLines.push(`From: ${esc(msg.from)}`);
    const date = formatMessageDateTime(msg.sent_at);
    if (date) headerLines.push(`Date: ${esc(date)}`);
    if (msg.to.length > 0) headerLines.push(`To: ${esc(msg.to.join(', '))}`);
    if (msg.cc.length > 0) headerLines.push(`Cc: ${esc(msg.cc.join(', '))}`);
    headerLines.push(`Subject: ${esc(msg.subject || '')}`);
    const headerHtml = headerLines.map(l => `<div>${l}</div>`).join('');
    return `<br><div style="border-left:2px solid #ccc;padding-left:8px;margin:8px 0"><div>---------- Forwarded message ---------</div>${headerHtml}<br>${bodyContent}</div>`;
  }

  // Replies go to Reply-To when the original message sets it, otherwise to the sender.
  function replyRecipients(msg: Message): string[] {
    if (msg.reply_to?.length) return msg.reply_to;
    return msg.from ? [msg.from] : [];
  }

  // ── Public open/close interface ────────────────────────────────────────────

  export function open(options: {
    mode?: 'new' | 'reply' | 'replyAll' | 'forward';
    replyTo?: Message;
    subject?: string;
    attachments?: File[];
    parsedText?: string;
    parsedHtml?: string | null;
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
      composeTo = replyRecipients(replyTo).join(', ');
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = replyQuote(replyTo, options.parsedText ?? '');
      composeInReplyTo = replyTo.message_id;
      composeReferences = replyTo.message_id;
      composeForwardHtml = null;
      composeForwardRef = null;
      composeForwardQuoteText = null;
      const accountIdentities = identities.filter(i => i.account_id === replyTo.account_id);
      const toAddresses = (replyTo.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
    } else if (mode === 'replyAll' && options.replyTo) {
      const replyTo = options.replyTo;
      composeMode = 'reply';
      composeTo = replyRecipients(replyTo).join(', ');
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = replyQuote(replyTo, options.parsedText ?? '');
      composeInReplyTo = replyTo.message_id;
      composeReferences = replyTo.message_id;
      composeForwardHtml = null;
      composeForwardRef = null;
      composeForwardQuoteText = null;
      const accountIdentities = identities.filter(i => i.account_id === replyTo.account_id);
      const toAddresses = (replyTo.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
      const myIdentity = identities.find(i => i.identity_id === composeIdentityId);
      const myEmail = myIdentity?.email.toLowerCase() ?? '';
      const allRecipients = [...(replyTo.to ?? []), ...(replyTo.cc ?? [])];
      const excluded = new Set([replyTo.from ?? '', ...replyRecipients(replyTo)].map(a => a.toLowerCase()));
      const ccAddresses = allRecipients
        .filter(addr => {
          const a = addr.toLowerCase();
          return !a.includes(myEmail) && !excluded.has(a);
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
      const toAddresses = (forwardFrom.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
      composeSubject = /^fwd:/i.test(forwardFrom.subject ?? '')
        ? (forwardFrom.subject ?? '')
        : `Fwd: ${forwardFrom.subject || ''}`;
      const fwdHtml = buildForwardHtml(forwardFrom, options.parsedHtml ?? null);
      if (fwdHtml) {
        composeBody = '';
        composeForwardHtml = fwdHtml;
        composeForwardQuoteText = forwardQuote(forwardFrom, options.parsedText ?? '');
      } else {
        composeBody = forwardQuote(forwardFrom, options.parsedText ?? '');
        composeForwardHtml = null;
        composeForwardQuoteText = null;
      }
      composeForwardRef = { account_id: forwardFrom.account_id, mailbox: forwardFrom.mailbox, uid: forwardFrom.uid };
      composeInReplyTo = null;
      composeReferences = null;
    } else {
      composeMode = 'new';
      composeTo = '';
      composeSubject = options.subject ?? '';
      composeBody = '';
      composeForwardHtml = null;
      composeForwardRef = null;
      composeForwardQuoteText = null;
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
      const identity = identities.find(item => item.identity_id === composeIdentityId);
      if (!identity) throw new Error(tl('pgp_composeIdentityMissing'));
      const to = parseEnvelopeAddresses(composeTo);
      const cc = parseEnvelopeAddresses(composeCc);
      const bcc = parseEnvelopeAddresses(composeBcc);

      let signingKey = composeSign ? signingKeyForIdentity(composeIdentityId) : null;
      if (composeSign && !signingKey && composeCanSign) {
        await unlockVaultForSigning();
        signingKey = signingKeyForIdentity(composeIdentityId);
      }
      if (composeSign && !signingKey) throw new Error(isUnlocked() ? tl('pgp_composeNoSigningKey') : tl('pgp_composeUnlockSigningKey'));

      let encryptionKeys: openpgp.PublicKey[] = [];
      if (composeEncrypt) {
        const allRecipients = [...to, ...cc, ...bcc];
        if (allRecipients.length === 0) throw new Error(tl('pgp_composeNoRecipients'));
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

      // Fetch inline images referenced in the forward HTML so they can be embedded in the MIME.
      const inlineImages: Array<{ cid: string; contentType: string; bytes: Uint8Array }> = [];
      if (composeForwardHtml && composeForwardRef) {
        const cids = cidReferencesFromHtml(composeForwardHtml);
        for (const cid of cids) {
          try {
            const url = `/api/email/proxy-image?account_id=${encodeURIComponent(composeForwardRef.account_id)}&mailbox=${encodeURIComponent(composeForwardRef.mailbox)}&uid=${composeForwardRef.uid}&cid=${encodeURIComponent(cid)}`;
            const res = await apiFetch(url);
            if (res.ok) {
              inlineImages.push({
                cid,
                contentType: res.headers.get('content-type') ?? 'image/jpeg',
                bytes: new Uint8Array(await res.arrayBuffer()),
              });
            }
          } catch { /* skip on error */ }
        }
      }

      const rawMime = await buildProtectedMime({
        identity,
        to,
        cc,
        subject: composeSubject,
        body: composeForwardQuoteText !== null ? composeBody + composeForwardQuoteText : composeBody,
        bodyHtml: (() => {
          if (!composeForwardHtml) return undefined;
          if (!composeBody.trim()) return composeForwardHtml;
          const introHtml = composeBody
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            .replace(/\r?\n/g, '<br>');
          return `<div>${introHtml}</div>${composeForwardHtml}`;
        })(),
        inlineImages,
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
      const r = await apiFetch('/api/email/send-raw', { method: 'POST', body: fd });
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
  id="mail-compose"
  bind:this={dialog}
  class="compose-dialog"
  data-layout="stack"
  data-gap="none"
  data-pad="none"
  on:cancel={(e) => { e.preventDefault(); if (filePickerOpen) { filePickerOpen = false; } else { dialog?.close(); } }}
>
  <header class="compose-header" data-pad="sm">
    <h2>{composeMode === 'forward' ? tl('mail_forward') : composeMode === 'replyAll' ? tl('mail_replyAll') : composeMode === 'reply' ? tl('mail_reply') : tl('mail_newMessage')}</h2>
    <button type="button" commandfor="mail-compose" command="close" aria-label={tl('mail_close')}></button>
  </header>
  <form on:submit|preventDefault={sendEmail} class="compose-form" data-layout="stack" data-gap="none" data-grow>
    {#if identities.length > 1}
      <div class="compose-row" data-layout="row" data-gap="sm" data-wrap="none">
        <label class="compose-label" for="compose-from">{tl('mail_composeFrom')}</label>
        <select id="compose-from" class="compose-input" data-grow bind:value={composeIdentityId}>
          {#each identities as id}
            <option value={id.identity_id}>{id.name} &lt;{id.email}&gt;</option>
          {/each}
        </select>
      </div>
    {/if}
    <div class="compose-row" data-layout="row" data-gap="sm" data-wrap="none">
      <label class="compose-label" for="compose-to">{tl('mail_composeTo')}</label>
          <div class="compose-input-wrap" data-combo data-grow>
            <input
              id="compose-to"
              class="compose-input"
              role="combobox"
              autocomplete="off"
              aria-autocomplete="list"
              aria-controls="compose-to-list"
              aria-expanded={recipientActiveField === 'to' && recipientSuggestions.length > 0}
              bind:value={composeTo}
              placeholder={tl('mail_composeToPlaceholder')}
              on:input={(e) => onRecipientInput('to', e.currentTarget.value)}
              on:keydown={onRecipientKeydown}
              on:blur={onRecipientBlur}
            />
            {#if recipientActiveField === 'to' && recipientSuggestions.length > 0}
              <div data-menu role="listbox" id="compose-to-list">
                {#each recipientSuggestions as addr, i}
                  <div role="option" tabindex="-1" aria-selected={i === recipientSuggestionIdx}
                    on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                  >{addr}</div>
                {/each}
              </div>
            {/if}
          </div>
      <button
        type="button"
        data-variant="ghost"
        data-size="sm"
        data-icon="lock"
        disabled={!composeCanEncrypt || composeSending}
        aria-label={tl('pgp_composeEncrypt')}
        aria-pressed={composeEncrypt}
        data-tooltip
        on:click={toggleComposeEncrypt}
      ></button>
      <button
        type="button"
        data-variant="ghost"
        data-size="sm"
        data-icon="signature"
        disabled={!composeCanSign || composeSending}
        aria-label={tl('pgp_composeSign')}
        aria-pressed={composeSign}
        data-tooltip
        on:click={toggleComposeSign}
      ></button>
      {#if !showComposeCcBcc}
        <button type="button" class="nowrap" data-variant="ghost" data-size="sm" on:click={() => { showComposeCcBcc = true; }}>{tl('mail_composeCcBcc')}</button>
      {/if}
    </div>
    {#if showComposeCcBcc}
      <div class="compose-row" data-layout="row" data-gap="sm" data-wrap="none">
        <label class="compose-label" for="compose-cc">{tl('mail_composeCc')}</label>
          <div class="compose-input-wrap" data-combo data-grow>
            <input
              id="compose-cc"
              class="compose-input"
              role="combobox"
              autocomplete="off"
              aria-autocomplete="list"
              aria-controls="compose-cc-list"
              aria-expanded={recipientActiveField === 'cc' && recipientSuggestions.length > 0}
              bind:value={composeCc}
              placeholder={tl('mail_composeCcPlaceholder')}
              on:input={(e) => onRecipientInput('cc', e.currentTarget.value)}
              on:keydown={onRecipientKeydown}
              on:blur={onRecipientBlur}
            />
            {#if recipientActiveField === 'cc' && recipientSuggestions.length > 0}
              <div data-menu role="listbox" id="compose-cc-list">
                {#each recipientSuggestions as addr, i}
                  <div role="option" tabindex="-1" aria-selected={i === recipientSuggestionIdx}
                    on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                  >{addr}</div>
                {/each}
              </div>
            {/if}
          </div>
      </div>
      <div class="compose-row" data-layout="row" data-gap="sm" data-wrap="none">
        <label class="compose-label" for="compose-bcc">{tl('mail_composeBcc')}</label>
          <div class="compose-input-wrap" data-combo data-grow>
            <input
              id="compose-bcc"
              class="compose-input"
              role="combobox"
              autocomplete="off"
              aria-autocomplete="list"
              aria-controls="compose-bcc-list"
              aria-expanded={recipientActiveField === 'bcc' && recipientSuggestions.length > 0}
              bind:value={composeBcc}
              placeholder={tl('mail_composeBccPlaceholder')}
              on:input={(e) => onRecipientInput('bcc', e.currentTarget.value)}
              on:keydown={onRecipientKeydown}
              on:blur={onRecipientBlur}
            />
            {#if recipientActiveField === 'bcc' && recipientSuggestions.length > 0}
              <div data-menu role="listbox" id="compose-bcc-list">
                {#each recipientSuggestions as addr, i}
                  <div role="option" tabindex="-1" aria-selected={i === recipientSuggestionIdx}
                    on:mousedown={(e) => { e.preventDefault(); applyRecipientSuggestion(addr); }}
                  >{addr}</div>
                {/each}
              </div>
            {/if}
          </div>
      </div>
    {/if}
    <div class="compose-row" data-layout="row" data-gap="sm" data-wrap="none">
      <label class="compose-label" for="compose-subject">{tl('mail_composeSubject')}</label>
      <input id="compose-subject" class="compose-input" data-grow bind:value={composeSubject} />
    </div>
    <textarea class="compose-body" data-grow aria-label={tl('mail_newMessage')} bind:value={composeBody} rows={composeForwardHtml ? 4 : 10}></textarea>
    {#if composeForwardHtml && composeForwardRef}
      <div class="compose-forward-preview">
        <MailHtmlFrame
          srcdoc={prepareEmailHtml(composeForwardHtml, {
            accountId: composeForwardRef.account_id,
            mailbox: composeForwardRef.mailbox,
            uid: composeForwardRef.uid,
          })}
          title="Forwarded message"
          style="height:240px"
        />
      </div>
    {/if}
    {#if composeAttachments.length > 0}
      <ul class="compose-attachments" data-layout="row" data-gap="xs" data-pad="sm">
        {#each composeAttachments as file, i}
          <li class="mail-attachment-chip" data-icon="paperclip">
            <span class="mail-attachment-name" data-truncate title={file.name}>{file.name}</span>
            <span data-muted>{formatFileSize(file.size)}</span>
            <button type="button" class="mail-attachment-remove" data-variant="ghost" data-icon="x" aria-label={tl('mail_removeAttachment')} on:click={() => removeAttachment(i)}></button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if composeError}
      <p class="compose-error" data-tone="danger" role="alert">{composeError}</p>
    {/if}
    <input
      bind:this={composeFileInput}
      type="file"
      multiple
      hidden
      aria-label={tl('mail_attachFiles')}
      on:change={(e) => addAttachments(e.currentTarget.files)}
    />
    <footer class="compose-footer" data-layout="row" data-gap="sm" data-justify="between" data-pad="sm">
      <button type="button" data-variant="ghost" data-icon="paperclip" on:click={openFilePicker}>{tl('mail_attach')}</button>
      <div data-layout="row" data-gap="sm">
        <button type="button" data-variant="secondary" on:click={() => dialog?.close()}>{tl('mail_discard')}</button>
        <button type="submit" data-icon="send" disabled={composeSending} aria-busy={composeSending}>
          {composeSending ? tl('mail_sending') : tl('mail_send')}
        </button>
      </div>
    </footer>
  </form>
</dialog>

<style>
  .compose-dialog {
    --tm-dialog-width: 44rem;
    block-size: clamp(320px, 60dvh, 560px);
    min-inline-size: min(360px, 100% - 1rem);
    min-block-size: 280px;
    resize: both;
    overflow: hidden;
  }

  .compose-header,
  .compose-row,
  .compose-attachments {
    border-block-end: 1px solid var(--tm-border);
  }

  .compose-header {
    background: var(--tm-surface);
  }

  .compose-header h2 {
    font-size: var(--tm-text-md);
  }

  .compose-form {
    min-block-size: 0;
    overflow: hidden;
  }

  .compose-row {
    padding-inline: var(--tm-space-md);
  }

  .compose-row:focus-within {
    box-shadow: inset 0 -2px 0 var(--tm-accent);
  }

  .compose-label {
    flex-shrink: 0;
    inline-size: 3.5rem;
    text-align: end;
    font-size: var(--tm-text-sm);
    color: var(--tm-text-muted);
  }

  .nowrap {
    flex-shrink: 0;
    white-space: nowrap;
  }

  .compose-input-wrap {
    min-inline-size: 0;
  }

  .compose-input,
  .compose-body {
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    outline: none;
  }

  .compose-input {
    inline-size: 100%;
    padding-inline: 0;
  }

  .compose-body {
    flex-shrink: 1;
    min-block-size: 4rem;
    padding: var(--tm-space-sm) var(--tm-space-md);
    resize: none;
  }

  .compose-forward-preview {
    flex-shrink: 0;
    border-block-start: 1px solid var(--tm-border);
    padding-inline: var(--tm-space-md);
  }

  .compose-attachments {
    flex-shrink: 0;
    border-block-start: 1px solid var(--tm-border);
  }

  .compose-error {
    padding: var(--tm-space-xs) var(--tm-space-md);
  }

  .compose-footer {
    flex-shrink: 0;
    border-block-start: 1px solid var(--tm-border);
    background: var(--tm-surface);
  }
</style>

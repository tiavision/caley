<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { getUser } from '../lib/user.ts';
  import { getLocale, makeTl } from '../lib/i18n.ts';
  import { type NewMailNotification, showNewMailNotification } from '../lib/mail-notifications.ts';
  import SelectDropdown from './SelectDropdown.svelte';
  import UiIcon from './UiIcon.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import { readVault, decryptPrivateKeys, localActiveDevice, rememberLocalDevice, unwrapVaultKey, validateVault, eraseBytes } from '../lib/pgp/vault.ts';
  import { unlock } from '../lib/pgp/webauthn.ts';
  import { decryptMessage, verifySignedMessage, type DecryptedAttachment } from '../lib/pgp/keyring.ts';
  import { parsePublicKey, listPublicKeys, readVerificationKeys, savePublicKey, type StoredPublicKey } from '../lib/pgp/public-keys.ts';
  import { openVault, getKeys, isUnlocked } from '../lib/pgp/session.ts';
  import { autocryptKeys } from '../lib/pgp/autocrypt.ts';
  import { buildProtectedMime, parseEnvelopeAddresses } from '../lib/pgp/compose.ts';
  import { prepareEmailHtml, htmlToPlainText, imageOriginsFromHtml } from '../lib/mail-html.ts';
  import type { CalendarJson, CalDavAccountJson } from '../types/calendar.ts';
  import type * as openpgp from 'openpgp';

  export let standalone = false;
  export let initialMessage: { accountId: string; mailbox: string; uid: number } | null = null;

  let locale = 'en';
  $: tl = makeTl(locale);

  // ── Types ────────────────────────────────────────────────────────────────

  interface Account {
    account_id: string;
    display_name: string;
    host: string;
    port: number;
    username: string;
    use_tls: boolean;
    smtp_host: string | null;
    smtp_port: number | null;
  }

  interface Identity {
    identity_id: string;
    account_id: string;
    name: string;
    email: string;
    is_default: boolean;
    sort_order: number;
  }

  interface Mailbox {
    account_id: string;
    name: string;
    delimiter: string | null;
    subscribed: boolean;
  }

  interface SidebarMailbox {
    mailbox: Mailbox;
    unread_count: number;
  }

  interface SidebarAccountSection {
    account: Account;
    collapsed: boolean;
    unread_count: number;
    mailboxes: SidebarMailbox[];
  }

  interface VirtualFolder {
    folder_id: string;
    display_name: string;
    sort_order: number;
    only_flagged: boolean;
    only_attachments: boolean;
    rules: { account_id: string | null; mailbox_pattern: string | null }[];
  }

  interface Attachment {
    filename: string;
    content_type: string;
    size: number;
  }

  interface Message {
    uid: number;
    account_id: string;
    mailbox: string;
    rowid: number;
    message_id: string | null;
    thread_id: string | null;
    from: string | null;
    to: string[];
    cc: string[];
    subject: string | null;
    sent_at: number | null;
    flags: string[];
    has_attachments: boolean;
    body_text: string | null;
    body_html: string | null;
    is_read: boolean;
    is_flagged: boolean;
    is_encrypted: boolean;
    is_signed: boolean;
    attachments: Attachment[];
  }

  interface ThreadNode {
    uid: number;
    account_id: string;
    mailbox: string;
    message_id: string | null;
    parent_message_id: string | null;
    from: string | null;
    subject: string | null;
    sent_at: number | null;
    is_read: boolean;
  }

  interface ThreadTreeNode {
    message: ThreadNode;
    children: ThreadTreeNode[];
  }

  function canPreviewContentType(value: string | null | undefined): boolean {
    const contentType = value?.split(';', 1)[0].trim().toLowerCase() ?? '';
    return [
      'application/pdf',
      'text/plain',
      'image/png',
      'image/jpeg',
      'image/gif',
      'image/webp',
      'image/avif',
    ].includes(contentType) ||
      (contentType.startsWith('audio/') && contentType.length > 'audio/'.length) ||
      (contentType.startsWith('video/') && contentType.length > 'video/'.length);
  }

  function canPreviewAttachment(attachment: Attachment): boolean {
    return canPreviewContentType(attachment.content_type);
  }

  function isPgpTransportAttachment(attachment: Attachment): boolean {
    const filename = attachment.filename.toLowerCase();
    const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
    return filename === 'encrypted.asc' ||
      (/^(encrypted|message|msg)\.(asc|pgp|gpg)$/.test(filename) &&
        (contentType === 'application/octet-stream' || contentType === 'application/pgp-encrypted'));
  }

  function isPgpSignatureAttachment(attachment: Attachment): boolean {
    const filename = attachment.filename.toLowerCase();
    const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
    return contentType === 'application/pgp-signature' ||
      /^signature\.(asc|sig|pgp|gpg)$/.test(filename);
  }

  function isPgpPublicKeyAttachment(attachment: Attachment): boolean {
    const filename = attachment.filename.toLowerCase();
    const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
    return contentType === 'application/pgp-keys' ||
      (!isPgpTransportAttachment(attachment) && !isPgpSignatureAttachment(attachment) &&
        /\.(asc|pgp|gpg|pub)$/.test(filename));
  }

  function isIcsAttachment(attachment: Attachment): boolean {
    const filename = attachment.filename.toLowerCase();
    const contentType = attachment.content_type?.split(';', 1)[0].trim().toLowerCase() ?? '';
    return contentType === 'text/calendar' ||
      contentType === 'application/ics' ||
      contentType === 'application/icalendar' ||
      /\.(ics|ical)$/.test(filename);
  }

  // ── State ────────────────────────────────────────────────────────────────

  let accounts: Account[] = [];
  let identities: Identity[] = [];
  let mailboxesByAccount: Record<string, Mailbox[]> = {};
  let calendars: CalendarJson[] = [];
  let calendarAccounts: CalDavAccountJson[] = [];
  let virtualFolders: VirtualFolder[] = [];
  let syncingMailboxes: Set<string> = new Set();
  let mailboxUnreadCounts: Record<string, Record<string, number>> = {};
  let folderUnreadCounts: Record<string, number> = {};
  const COLLAPSED_ACCOUNTS_KEY = 'mail-collapsed-accounts';
  let collapsedAccountIds: Set<string> = new Set();
  let sidebarAccountSections: SidebarAccountSection[] = [];

  let selectedAccountId: string | null = null;
  let selectedMailbox: string | null = null;
  let selectedFolderId: string | null = null;

  let messages: Message[] = [];
  let totalMessages = 0;
  let currentPage = 1;
  const pageSize = 50;
  let messagesLoading = false;
  let listScrollEl: HTMLElement;
  let pageInputActive = false;
  let pageInputValue = '';
  let pageInputEl: HTMLInputElement | null = null;

  let searchQuery = '';
  let searching = false;
  let searchHasNext = false;
  let searchDebounce: ReturnType<typeof setTimeout> | null = null;

  // Search filter state
  let filterAddrOpen = false;
  let filterDateOpen = false;
  let filterAddrs: string[] = [];
  let filterDateFrom = '';
  let filterDateTo = '';
  let addrFilterInput = '';
  let contactsLoaded = false;
  let contactSuggestions: { name: string; email: string }[] = [];
  let addrDbSuggestions: string[] = [];
  let addrSuggestDebounce: ReturnType<typeof setTimeout> | null = null;
  let addrFilterEl: HTMLDivElement;
  let dateFilterEl: HTMLDivElement;
  let addrPopoverEl: HTMLDivElement;
  let datePopoverEl: HTMLDivElement;
  let addrPopoverStyle = '';
  let datePopoverStyle = '';
  $: filterAddrActive = filterAddrs.length > 0;
  $: filterDateActive = filterDateFrom !== '' || filterDateTo !== '';
  $: filterActive = filterAddrActive || filterDateActive;

  let selectedMessage: Message | null = null;
  let listCursor: Message | null = null;
  let threadMessages: ThreadNode[] = [];
  let threadTree: ThreadTreeNode[] = [];
  let threadRequest = 0;
  let queuedReaderNavigation: -1 | 1 | null = null;
  let messageLoading = false;
  let standaloneUnavailable = false;
  let markedMessageKeys = new Set<string>();
  let selectionAnchorKey: string | null = null;
  let bulkActionBusy = false;
  let bulkActionError = '';
  let bulkMoveOpen = false;

  let sseSource: EventSource | null = null;

  // Account form
  let accountForm = emptyAccountForm();
  let accountSaving = false;
  let accountError = '';
  let accountStep: 'basic' | 'details' = 'basic';
  let autodiscoverResult: { provider_name?: string; imap_host: string; imap_port: number; imap_use_tls: boolean; imap_username: string; smtp_host: string; smtp_port: number } | null = null;
  let autodiscovering = false;
  let autodiscoverError = '';
  let showAdvanced = false;

  // Identity form
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
  let identityDialog: HTMLDialogElement;
  let identityAccountId = '';   // which account's identities are being managed
  let identityMode: 'idle' | 'add' | 'edit' = 'idle';

  // Compose form
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
  let composeReplyAll = false;
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

  // Virtual folder form
  let folderName = '';
  let folderSaving = false;
  let folderSelectedMailboxes: Set<string> = new Set();
  let folderOnlyFlagged = false;
  let folderOnlyAttachments = false;
  let editingFolder: VirtualFolder | null = null;

  let displayName = '';

  // PGP decryption state (per selected message)
  let pgpDecrypting = false;
  let pgpDecryptError = '';
  let pgpDecryptedHtml = '';
  let pgpDecryptedText = '';
  let pgpAttachments: (Attachment & { url: string; data: Uint8Array })[] = [];
  let pgpBadges: string[] = [];
  let pgpUnlocking = false;
  let pgpUnlockError = '';
  let pgpPublicKeyImporting = '';
  let icsImportingAttachment = '';
  let pgpPublicKeyImportError = '';
  let pgpPublicKeyImportSuccess = '';
  let pgpImportablePublicKeyAttachments = new Set<string>();
  let pgpKeyUnlocked = isUnlocked();
  let pgpDecryptGeneration = 0;
  $: pgpIconBadges = pgpBadges.filter(badge => badge === 'encrypted' || badge === 'signed' || badge === 'verified');
  $: pgpTextBadges = pgpBadges.filter(badge => badge !== 'encrypted' && badge !== 'signed' && badge !== 'verified');

  function clearPgpAttachments() {
    for (const attachment of pgpAttachments) URL.revokeObjectURL(attachment.url);
    pgpAttachments = [];
  }

  function clearPgpContent() {
    pgpDecryptGeneration++;
    pgpDecrypting = false;
    clearPgpAttachments();
    pgpDecryptedHtml = '';
    pgpDecryptedText = '';
    pgpBadges = [];
    pgpDecryptError = '';
    pgpPublicKeyImportError = '';
    pgpPublicKeyImportSuccess = '';
    pgpImportablePublicKeyAttachments = new Set();
    pgpKeyUnlocked = isUnlocked();
    iframeKey++;
  }

  function clearPgpContentWhenHidden() {
    if (document.hidden) clearPgpContent();
  }

  function attachmentDataUrl(attachment: DecryptedAttachment): string {
    let binary = '';
    const chunkSize = 0x8000;
    for (let offset = 0; offset < attachment.data.length; offset += chunkSize) {
      binary += String.fromCharCode(...attachment.data.subarray(offset, offset + chunkSize));
    }
    return `data:${attachment.contentType};base64,${btoa(binary)}`;
  }

  function insertDecryptedInlineImages(html: string, attachments: DecryptedAttachment[]): string {
    let result = html;
    for (const attachment of attachments) {
      if (!attachment.contentId || !/^image\/(png|jpeg|gif|webp|avif)$/i.test(attachment.contentType)) continue;
      const escapedCid = attachment.contentId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      result = result.replace(new RegExp(`cid:${escapedCid}`, 'gi'), attachmentDataUrl(attachment));
    }
    return result;
  }

  function bareMailbox(value: string | null): string {
    const match = /<([^<>]+)>/.exec(value ?? '');
    return (match?.[1] ?? value ?? '').trim().toLowerCase();
  }

  function keyHasEmail(key: { users: { userID?: { email?: string } }[] }, email: string): boolean {
    return key.users.some(user => user.userID?.email?.trim().toLowerCase() === email.toLowerCase());
  }

  async function trustedPublicKeys(): Promise<{ record: StoredPublicKey; key: openpgp.PublicKey }[]> {
    const records = (await listPublicKeys()).filter(record => record.trust === 'trusted');
    const parsed: { record: StoredPublicKey; key: openpgp.PublicKey }[] = [];
    for (const record of records) {
      try {
        parsed.push({ record, key: await parsePublicKey(record.armored) });
      } catch {
        // Corrupt stored keys are unavailable for compose encryption.
      }
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

  async function fetchRawMessage(msg: Message): Promise<Uint8Array> {
    const url = `/api/email/raw?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error('raw message fetch failed');
    return new Uint8Array(await res.arrayBuffer());
  }

  async function harvestAutocrypt(rawBytes: Uint8Array, msg: Message) {
    const sender = bareMailbox(msg.from);
    for (const key of await autocryptKeys(rawBytes, msg.from)) {
      await savePublicKey(key, 'harvested_autocrypt', sender).catch(() => {});
    }
  }

  function isSelectedMessage(msg: Message): boolean {
    return selectedMessage?.uid === msg.uid &&
      selectedMessage?.account_id === msg.account_id &&
      selectedMessage?.mailbox === msg.mailbox;
  }

  async function loadMessagePublicKeyCandidates(
    msg: Message,
    attachments: (Attachment & { data?: Uint8Array })[],
  ) {
    const candidates = attachments.filter(isPgpPublicKeyAttachment);
    if (candidates.length === 0) {
      if (isSelectedMessage(msg)) pgpImportablePublicKeyAttachments = new Set();
      return;
    }
    const storedFingerprints = new Set(
      (await listPublicKeys()).map(record => record.fingerprint.toUpperCase()),
    );
    const importable = new Set<string>();
    for (const candidate of candidates) {
      try {
        let bytes = candidate.data;
        if (bytes) {
          // Decrypted attachment data is already available in memory.
        } else {
          const url = `/api/email/attachment?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}&filename=${encodeURIComponent(candidate.filename)}`;
          const response = await fetch(url);
          if (!response.ok) continue;
          bytes = new Uint8Array(await response.arrayBuffer());
        }
        const key = await parsePublicKey(new TextDecoder().decode(bytes));
        if (!storedFingerprints.has(key.getFingerprint().toUpperCase())) importable.add(candidate.filename);
      } catch {
        // Only parsed OpenPGP public-key attachments receive an import action.
      }
    }
    if (isSelectedMessage(msg)) pgpImportablePublicKeyAttachments = importable;
  }

  async function pgpUnlock(decryptSelected = true): Promise<boolean> {
    pgpUnlocking = true;
    pgpUnlockError = '';
    let unlocked = false;
    try {
      const vault = await readVault();
      if (!vault) throw new Error('no vault');
      const device = localActiveDevice(vault);
      if (!device) throw new Error('no local approved device');
      const { prfOutput } = await unlock(device.prf_salt, device.cred_id);
      const { raw, key: vaultKey } = await unwrapVaultKey(device, prfOutput);
      try {
        await validateVault(vault, vaultKey, device.cred_id);
        const keys = await decryptPrivateKeys(vaultKey, vault.keys);
        openVault(vaultKey, keys);
        rememberLocalDevice(device.cred_id);
        pgpKeyUnlocked = true;
        reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
        if (decryptSelected && selectedMessage?.is_encrypted) {
          await pgpDecrypt(selectedMessage);
        }
        unlocked = true;
      } finally {
        eraseBytes(raw);
      }
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      console.error('[pgp] unlock vault failed', e);
      pgpUnlockError = `${tl('pgp_unlockError')} ${msg}`;
    }
    pgpUnlocking = false;
    return unlocked;
  }

  async function pgpDecrypt(msg: Message) {
    const keys = getKeys();
    if (keys.length === 0) return;
    const generation = ++pgpDecryptGeneration;
    pgpDecrypting = true;
    pgpDecryptError = '';
    pgpDecryptedHtml = '';
    pgpDecryptedText = '';
    clearPgpAttachments();
    pgpBadges = [];
    try {
      const rawBytes = await fetchRawMessage(msg);
      const verificationKeys = await listPublicKeys()
        .then(readVerificationKeys)
        .catch(() => []);
      const result = await decryptMessage(keys, rawBytes, verificationKeys);
      if (generation !== pgpDecryptGeneration || selectedMessage?.uid !== msg.uid ||
          selectedMessage?.account_id !== msg.account_id || selectedMessage?.mailbox !== msg.mailbox) return;
      pgpBadges = result.badges;
      if (result.badges.includes('decrypt_failed') || result.badges.includes('missing_key') || result.badges.includes('unsupported_alg')) {
        pgpDecryptError = tl('pgp_decryptFailed');
      } else {
        pgpAttachments = result.attachments.map(attachment => ({
          filename: attachment.filename,
          content_type: attachment.contentType,
          size: attachment.data.byteLength,
          url: URL.createObjectURL(new Blob(
            [attachment.data],
            { type: canPreviewContentType(attachment.contentType) ? attachment.contentType : 'application/octet-stream' },
          )),
          data: attachment.data,
        }));
        await loadMessagePublicKeyCandidates(msg, pgpAttachments);
        if (result.html) {
          const withInlineImages = insertDecryptedInlineImages(result.html, result.attachments);
          pgpDecryptedHtml = prepareEmailHtml(withInlineImages);
          pgpDecryptedText = result.text ?? htmlToPlainText(result.html);
          viewMode = htmlAddsValue(result.html) ? 'html' : 'text';
        } else if (result.text) {
          pgpDecryptedText = result.text;
        }
      }
    } catch {
      if (generation === pgpDecryptGeneration) pgpDecryptError = tl('pgp_decryptFailed');
    }
    if (generation === pgpDecryptGeneration) pgpDecrypting = false;
  }

  async function importMessagePublicKey(attachment: Attachment, decryptedData?: Uint8Array) {
    pgpPublicKeyImporting = attachment.filename;
    pgpPublicKeyImportError = '';
    pgpPublicKeyImportSuccess = '';
    try {
      const msg = selectedMessage;
      if (!msg) throw new Error('no message');
      let bytes = decryptedData;
      if (!bytes) {
        const url = `/api/email/attachment?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}&filename=${encodeURIComponent(attachment.filename)}`;
        const res = await fetch(url);
        if (!res.ok) throw new Error('attachment fetch failed');
        bytes = new Uint8Array(await res.arrayBuffer());
      }
      const armored = new TextDecoder().decode(bytes);
      const key = await parsePublicKey(armored);
      await savePublicKey(key, 'message_attachment');
      pgpPublicKeyImportSuccess = tl('pgp_publicKeyImported');
      const attachments = msg.is_encrypted ? pgpAttachments : msg.attachments;
      await loadMessagePublicKeyCandidates(msg, attachments);
      if (msg.is_encrypted && getKeys().length > 0) {
        await pgpDecrypt(msg);
      } else if (msg.is_signed) {
        const rawBytes = await fetchRawMessage(msg);
        const verificationKeys = await listPublicKeys().then(readVerificationKeys).catch(() => []);
        pgpBadges = await verifySignedMessage(rawBytes, verificationKeys);
      }
    } catch {
      pgpPublicKeyImportError = tl('pgp_publicKeyImportError');
    }
    pgpPublicKeyImporting = '';
  }

  async function openIcsImport(attachment: Attachment, decryptedData?: Uint8Array) {
    icsImportingAttachment = attachment.filename;
    pgpPublicKeyImportError = '';
    pgpPublicKeyImportSuccess = '';
    try {
      const msg = selectedMessage;
      if (!msg) throw new Error('no message');
      let bytes = decryptedData;
      if (!bytes) {
        const url = `/api/email/attachment?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}&filename=${encodeURIComponent(attachment.filename)}`;
        const res = await fetch(url);
        if (!res.ok) throw new Error('attachment fetch failed');
        bytes = new Uint8Array(await res.arrayBuffer());
      }
      if (calendarAccounts.length === 0) await loadCalendarData();
      const file = new File([bytes], attachment.filename, { type: attachment.content_type || 'text/calendar' });
      importDialog?.open(file);
    } catch {
      pgpPublicKeyImportError = tl('cal_importError');
    } finally {
      icsImportingAttachment = '';
    }
  }

  function pgpBadgeLabel(badge: string): string {
    const map: Record<string, string> = {
      encrypted: tl('pgp_badgeEncrypted'),
      signed: tl('pgp_badgeSigned'),
      verified: tl('pgp_badgeVerified'),
      unverified: tl('pgp_badgeUnverified'),
      missing_key: tl('pgp_badgeMissingKey'),
      unsupported_alg: tl('pgp_badgeUnsupportedAlg'),
      verification_failed: tl('pgp_badgeVerificationFailed'),
    };
    return map[badge] ?? badge;
  }

  // HTML email rendering
  let viewMode: 'html' | 'text' = 'text';
  let imageOrigins: string[] = [];
  let blacklistOrigins: string[] = [];
  let originsEmailOnly = false;
  let originsFilter = '';
  let originsDialog: HTMLDialogElement;
  let iframeKey = 0;

  // Dialog refs
  let accountDialog: HTMLDialogElement;
  let composeDialog: HTMLDialogElement;
  let folderDialog: HTMLDialogElement;
  let threadDialog: HTMLDialogElement;
  let importDialog: ImportDialog;
  let sidebarOpen = false;

  let usesDetailOverlay = false;
  let isPhone = false;
  let compactMediaQuery: MediaQueryList | null = null;
  let phoneMediaQuery: MediaQueryList | null = null;
  let compactMediaQueryListener: ((e: MediaQueryListEvent) => void) | null = null;
  let phoneMediaQueryListener: ((e: MediaQueryListEvent) => void) | null = null;

  let iframeEl: HTMLIFrameElement | null = null;
  let iframeResizeObserver: ResizeObserver | null = null;
  let detailEl: HTMLElement | null = null;
  let detailBodyEl: HTMLElement | null = null;
  let detailHeaderEl: HTMLDivElement | null = null;
  let detailSubjectLineEl: HTMLDivElement | null = null;
  let detailControlsEl: HTMLDivElement | null = null;
  let detailHeaderResizeObserver: ResizeObserver | null = null;
  let detailHeaderStacked = false;

  function fitIframeToContent() {
    if (!iframeEl || !usesDetailOverlay) return;
    iframeResizeObserver?.disconnect();
    try {
      const doc = iframeEl.contentDocument;
      if (!doc) return;
      const setHeight = () => {
        const root = doc.documentElement;
        const body = doc.body;
        const h = Math.max(
          root.scrollHeight,
          root.offsetHeight,
          root.clientHeight,
          body?.scrollHeight ?? 0,
          body?.offsetHeight ?? 0,
          body?.clientHeight ?? 0,
        );
        if (h > 0 && iframeEl) iframeEl.style.height = Math.ceil(h) + 2 + 'px';
      };
      setHeight();
      iframeResizeObserver = new ResizeObserver(setHeight);
      iframeResizeObserver.observe(doc.documentElement);
      if (doc.body) iframeResizeObserver.observe(doc.body);
    } catch {}
  }

  $: if (!usesDetailOverlay && iframeEl) {
    iframeEl.style.height = '';
    iframeResizeObserver?.disconnect();
  }

  function intrinsicWidth(element: HTMLElement): number {
    const clone = element.cloneNode(true) as HTMLElement;
    clone.style.position = 'fixed';
    clone.style.left = '-10000px';
    clone.style.top = '0';
    clone.style.width = 'max-content';
    clone.style.maxWidth = 'none';
    clone.style.flex = 'none';
    clone.style.whiteSpace = 'nowrap';
    clone.style.visibility = 'hidden';
    clone.style.pointerEvents = 'none';
    document.body.appendChild(clone);
    const width = clone.getBoundingClientRect().width;
    clone.remove();
    return width;
  }

  function updateDetailHeaderLayout() {
    if (!detailHeaderEl || !detailSubjectLineEl || !detailControlsEl || !selectedMessage) {
      detailHeaderStacked = false;
      return;
    }
    const style = getComputedStyle(detailHeaderEl);
    const gap = parseFloat(style.columnGap || style.gap) || 0;
    const requiredWidth = intrinsicWidth(detailSubjectLineEl) + intrinsicWidth(detailControlsEl) + gap;
    detailHeaderStacked = requiredWidth > detailHeaderEl.clientWidth;
  }

  function observeDetailHeader(node: HTMLDivElement) {
    detailHeaderEl = node;
    detailHeaderResizeObserver?.disconnect();
    detailHeaderResizeObserver = new ResizeObserver(updateDetailHeaderLayout);
    detailHeaderResizeObserver.observe(node);
    void tick().then(updateDetailHeaderLayout);
    return {
      destroy() {
        if (detailHeaderEl === node) detailHeaderEl = null;
        detailHeaderResizeObserver?.disconnect();
        detailHeaderResizeObserver = null;
      },
    };
  }

  // Resize state
  let sidebarWidth = 220;
  let listWidth = 320;
  let resizing: 'sidebar' | 'list' | null = null;
  let resizeStartX = 0;
  let resizeStartWidth = 0;

  // Detail UI state
  let actionsOpen = false;
  let moveSubmenuOpen = false;
  let metaExpanded = false;

  // Drag & drop state
  let draggedMessage: Message | null = null;
  let dragOverMailbox: string | null = null;
  let actionsEl: HTMLDivElement;
  let actionsMenuPos = { top: 0, right: 0 };
  let attachmentsExpanded = false;
  let imagesNoticeDismissed = false;
  let detailOverlayVisible = false;
  const messageRowEls = new Map<string, HTMLButtonElement>();

  function messageKey(msg: Message): string {
    return `${msg.account_id}\u0000${msg.mailbox}\u0000${msg.uid}`;
  }

  function identityKey(accountId: string, mailbox: string, uid: number): string {
    return `${accountId}\u0000${mailbox}\u0000${uid}`;
  }

  function sameMessage(a: Message | null, b: Message | null): boolean {
    return !!a && !!b && messageKey(a) === messageKey(b);
  }

  function clearMarkedSelection() {
    markedMessageKeys = new Set();
    selectionAnchorKey = null;
    bulkActionError = '';
    bulkMoveOpen = false;
  }

  function showSelection() {
    detailOverlayVisible = true;
    actionsOpen = false;
    moveSubmenuOpen = false;
  }

  function toggleMarkedMessage(msg: Message) {
    const key = messageKey(msg);
    const next = new Set(markedMessageKeys);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    markedMessageKeys = next;
    selectionAnchorKey = key;
    bulkActionError = '';
    bulkMoveOpen = false;
    if (next.size > 0) showSelection();
  }

  function markMessageRange(msg: Message) {
    const targetIndex = messages.findIndex(candidate => sameMessage(candidate, msg));
    const anchorIndex = selectionAnchorKey
      ? messages.findIndex(candidate => messageKey(candidate) === selectionAnchorKey)
      : -1;
    if (targetIndex < 0 || anchorIndex < 0) {
      const next = new Set(markedMessageKeys);
      next.add(messageKey(msg));
      markedMessageKeys = next;
      selectionAnchorKey = messageKey(msg);
    } else {
      const from = Math.min(anchorIndex, targetIndex);
      const to = Math.max(anchorIndex, targetIndex);
      const next = new Set(markedMessageKeys);
      for (const candidate of messages.slice(from, to + 1)) next.add(messageKey(candidate));
      markedMessageKeys = next;
    }
    bulkActionError = '';
    bulkMoveOpen = false;
    showSelection();
  }

  function handleMessageRowClick(e: MouseEvent, msg: Message) {
    if (e.ctrlKey || e.metaKey) {
      toggleMarkedMessage(msg);
    } else if (e.shiftKey) {
      markMessageRange(msg);
    } else {
      clearMarkedSelection();
      openMessageFromList(msg);
    }
  }

  function registerMessageRow(node: HTMLButtonElement, msg: Message) {
    let key = messageKey(msg);
    messageRowEls.set(key, node);
    return {
      update(next: Message) {
        messageRowEls.delete(key);
        key = messageKey(next);
        messageRowEls.set(key, node);
      },
      destroy() {
        messageRowEls.delete(key);
      },
    };
  }

  async function focusListMessage(msg: Message | null) {
    if (!msg || standalone) return;
    await tick();
    const row = messageRowEls.get(messageKey(msg));
    row?.scrollIntoView({ block: 'nearest' });
    row?.focus();
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────

  function setupResponsiveState() {
    compactMediaQuery = window.matchMedia('(max-width: 1023px)');
    phoneMediaQuery = window.matchMedia('(max-width: 639px)');
    usesDetailOverlay = compactMediaQuery.matches;
    isPhone = phoneMediaQuery.matches;
    compactMediaQueryListener = (e: MediaQueryListEvent) => {
      usesDetailOverlay = e.matches;
      if (e.matches) void tick().then(fitIframeToContent);
    };
    phoneMediaQueryListener = (e: MediaQueryListEvent) => {
      isPhone = e.matches;
      if (!e.matches) sidebarOpen = false;
    };
    compactMediaQuery.addEventListener('change', compactMediaQueryListener);
    phoneMediaQuery.addEventListener('change', phoneMediaQueryListener);
  }

  onMount(async () => {
    setupResponsiveState();

    const user = await getUser();
    if (!user.ok) {
      window.location.href = '/';
      return;
    }
    displayName = user.display_name || '';
    locale = getLocale();
    loadCollapsedAccounts();

    if (standalone) {
      await loadStandalone();
    } else {
      await loadAll();
    }
    connectSSE();

    const pendingCompose = sessionStorage.getItem('caley_compose');
    if (pendingCompose) {
      sessionStorage.removeItem('caley_compose');
      try {
        const { subject, filename, mimeType, content } = JSON.parse(pendingCompose);
        composeSubject = subject ?? '';
        composeAttachments = [new File([content], filename, { type: mimeType })];
        composeIdentityId = (identities.find(i => i.is_default) ?? identities[0])?.identity_id ?? '';
        reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
        void loadComposeTrustedKeys();
        await tick();
        composeDialog?.showModal();
      } catch {}
    }

    if (usesDetailOverlay) void tick().then(fitIframeToContent);

    window.addEventListener('mousemove', handleResizeMove);
    window.addEventListener('mouseup', stopResize);
    window.addEventListener('keydown', handleMailKeydown);
    document.addEventListener('visibilitychange', clearPgpContentWhenHidden);
    window.addEventListener('pagehide', clearPgpContent);
  });

  onDestroy(() => {
    sseSource?.close();
    detailHeaderResizeObserver?.disconnect();
    if (compactMediaQuery && compactMediaQueryListener) {
      compactMediaQuery.removeEventListener('change', compactMediaQueryListener);
    }
    if (phoneMediaQuery && phoneMediaQueryListener) {
      phoneMediaQuery.removeEventListener('change', phoneMediaQueryListener);
    }
    window.removeEventListener('mousemove', handleResizeMove);
    window.removeEventListener('mouseup', stopResize);
    window.removeEventListener('keydown', handleMailKeydown);
    window.removeEventListener('click', handleWindowClick);
    document.removeEventListener('visibilitychange', clearPgpContentWhenHidden);
    window.removeEventListener('pagehide', clearPgpContent);
    removeFilterListeners();
    clearPgpAttachments();
  });

  // ── Data loading ─────────────────────────────────────────────────────────

  async function loadAll() {
    await Promise.all([loadAccounts(), loadIdentities(), loadVirtualFolders(), loadImageOrigins(), loadBlacklistOrigins(), loadUnreadCounts(), loadCalendarData()]);
    for (const acc of accounts) {
      await loadMailboxes(acc.account_id);
    }
    // Restore saved folder + page
    try {
      const raw = localStorage.getItem('mail-view-state');
      if (raw) {
        const s = JSON.parse(raw);
        if (s.folderId && virtualFolders.find((f: { id: string }) => f.id === s.folderId)) {
          await selectVirtualFolder(s.folderId);
          return;
        }
        if (s.accountId && s.mailbox) {
          const mbs = mailboxesByAccount[s.accountId] || [];
          if (mbs.find((m: { name: string }) => m.name === s.mailbox)) {
            await selectMailbox(s.accountId, s.mailbox);
            return;
          }
        }
      }
    } catch {}
    // Auto-select first inbox
    if (!selectedAccountId && accounts.length > 0) {
      const firstAcc = accounts[0];
      const mbs = mailboxesByAccount[firstAcc.account_id] || [];
      const inbox = mbs.find(m => m.name.toUpperCase() === 'INBOX') || mbs[0];
      if (inbox) {
        await selectMailbox(firstAcc.account_id, inbox.name);
      }
    }
  }

  function locatorMessage(locator: { accountId: string; mailbox: string; uid: number }): Message {
    return {
      uid: locator.uid,
      account_id: locator.accountId,
      mailbox: locator.mailbox,
      rowid: 0,
      message_id: null,
      thread_id: null,
      from: null,
      to: [],
      cc: [],
      subject: null,
      sent_at: null,
      flags: [],
      has_attachments: false,
      body_text: null,
      body_html: null,
      is_read: true,
      is_flagged: false,
      is_encrypted: false,
      is_signed: false,
      attachments: [],
    };
  }

  async function loadStandalone() {
    if (!initialMessage || !Number.isInteger(initialMessage.uid) || initialMessage.uid <= 0) {
      standaloneUnavailable = true;
      return;
    }
    selectedAccountId = initialMessage.accountId;
    selectedMailbox = initialMessage.mailbox;
    selectedMessage = locatorMessage(initialMessage);
    messageLoading = true;
    await Promise.all([
      loadIdentities(),
      loadMailboxes(initialMessage.accountId),
      loadImageOrigins(),
      loadBlacklistOrigins(),
      loadCalendarData(),
    ]);
    await openMessage(locatorMessage(initialMessage));
    await tick();
    detailEl?.focus();
  }

  async function loadAccounts() {
    try {
      const r = await fetch('/api/email/accounts');
      if (r.ok) {
        const data = await r.json();
        accounts = data.accounts || [];
      }
    } catch {}
  }

  async function loadCalendarData() {
    try {
      const [calendarRes, accountRes] = await Promise.all([
        fetch('/api/calendars'),
        fetch('/api/caldav/accounts'),
      ]);
      if (calendarRes.ok) calendars = await calendarRes.json();
      if (accountRes.ok) calendarAccounts = (await accountRes.json()).accounts || [];
    } catch {}
  }

  async function loadIdentities() {
    try {
      const r = await fetch('/api/email/identities');
      if (r.ok) {
        const data = await r.json();
        identities = data.identities || [];
      }
    } catch {}
  }

  async function loadMailboxes(accountId: string) {
    try {
      const r = await fetch(`/api/email/mailboxes?account_id=${encodeURIComponent(accountId)}`);
      if (r.ok) {
        const data = await r.json();
        mailboxesByAccount = { ...mailboxesByAccount, [accountId]: data.mailboxes || [] };
      }
    } catch {}
  }

  async function loadVirtualFolders() {
    try {
      const r = await fetch('/api/email/folders');
      if (r.ok) {
        const data = await r.json();
        virtualFolders = data.folders || [];
      }
    } catch {}
  }

  async function loadUnreadCounts() {
    try {
      const r = await fetch('/api/email/unread-counts');
      if (r.ok) {
        const data = await r.json();
        mailboxUnreadCounts = data.mailboxes || {};
        folderUnreadCounts = data.folders || {};
      }
    } catch {}
  }

  function loadCollapsedAccounts() {
    try {
      const raw = localStorage.getItem(COLLAPSED_ACCOUNTS_KEY);
      const ids = raw ? JSON.parse(raw) : [];
      collapsedAccountIds = Array.isArray(ids)
        ? new Set(ids.filter((id): id is string => typeof id === 'string'))
        : new Set();
    } catch {
      collapsedAccountIds = new Set();
    }
  }

  function saveCollapsedAccounts() {
    try {
      localStorage.setItem(COLLAPSED_ACCOUNTS_KEY, JSON.stringify([...collapsedAccountIds]));
    } catch {}
  }

  function isAccountCollapsed(accountId: string): boolean {
    return collapsedAccountIds.has(accountId);
  }

  function toggleAccountCollapsed(accountId: string) {
    const next = new Set(collapsedAccountIds);
    if (next.has(accountId)) next.delete(accountId);
    else next.add(accountId);
    collapsedAccountIds = next;
    saveCollapsedAccounts();
  }

  async function loadImageOrigins() {
    try {
      const r = await fetch('/api/email/image-origins');
      if (r.ok) {
        const data = await r.json();
        imageOrigins = data.origins || [];
      }
    } catch {}
  }

  async function allowOrigin(origin: string) {
    try {
      await fetch('/api/email/image-origins', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin }),
      });
      imageOrigins = [...imageOrigins, origin];
      iframeKey++; // force iframe reload so images are re-fetched
    } catch {}
  }

  async function revokeOrigin(origin: string) {
    try {
      await fetch(`/api/email/image-origins?origin=${encodeURIComponent(origin)}`, { method: 'DELETE' });
      imageOrigins = imageOrigins.filter(o => o !== origin);
    } catch {}
  }

  async function loadBlacklistOrigins() {
    try {
      const r = await fetch('/api/email/image-blacklist');
      if (r.ok) {
        const data = await r.json();
        blacklistOrigins = data.origins || [];
      }
    } catch {}
  }

  async function blockOriginSilently(origin: string) {
    try {
      await fetch('/api/email/image-blacklist', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ origin }),
      });
      blacklistOrigins = [...blacklistOrigins, origin];
    } catch {}
  }

  async function unblockOrigin(origin: string) {
    try {
      await fetch(`/api/email/image-blacklist?origin=${encodeURIComponent(origin)}`, { method: 'DELETE' });
      blacklistOrigins = blacklistOrigins.filter(o => o !== origin);
    } catch {}
  }

  async function promoteFromBlacklist(origin: string) {
    await unblockOrigin(origin);
    await allowOrigin(origin);
  }

  function openOriginsGlobal() {
    originsEmailOnly = false;
    originsFilter = '';
    originsDialog?.showModal();
  }

  function openOriginsForEmail() {
    originsEmailOnly = true;
    originsFilter = '';
    originsDialog?.showModal();
  }

  async function selectMailbox(accountId: string, mailbox: string) {
    clearMarkedSelection();
    selectedAccountId = accountId;
    selectedMailbox = mailbox;
    selectedFolderId = null;
    selectedMessage = null;
    listCursor = null;
    queuedReaderNavigation = null;
    detailOverlayVisible = false;
    currentPage = 1;
    saveNavState();
    resetSearch();
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
    sidebarOpen = false;
    await loadMessages();
    if (queuedReaderNavigation) {
      const step = queuedReaderNavigation;
      queuedReaderNavigation = null;
      await moveCursor(step, true);
    }
    fetch(
      `/api/email/sync?account_id=${encodeURIComponent(accountId)}&mailbox=${encodeURIComponent(mailbox)}`,
      { method: 'POST' },
    );
  }

  async function selectVirtualFolder(folderId: string) {
    clearMarkedSelection();
    selectedFolderId = folderId;
    selectedAccountId = null;
    selectedMailbox = null;
    selectedMessage = null;
    listCursor = null;
    queuedReaderNavigation = null;
    detailOverlayVisible = false;
    currentPage = 1;
    saveNavState();
    resetSearch();
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
    sidebarOpen = false;
    await loadMessages();
    if (queuedReaderNavigation) {
      const step = queuedReaderNavigation;
      queuedReaderNavigation = null;
      await moveCursor(step, true);
    }
  }

  function resetSearch() {
    if (searchDebounce) { clearTimeout(searchDebounce); searchDebounce = null; }
    searchQuery = '';
    searching = false;
  }

  function anyFilterActive() {
    return filterAddrs.length > 0 || filterDateFrom !== '' || filterDateTo !== '';
  }

  function onSearchInput() {
    if (searchDebounce) clearTimeout(searchDebounce);
    const q = searchQuery.trim();
    if (q === '' && !anyFilterActive()) {
      clearAllFilters();
      return;
    }
    searchDebounce = setTimeout(() => runSearch(), 250);
  }

  async function runSearch() {
    clearMarkedSelection();
    const q = searchQuery.trim();
    if (q === '' && !anyFilterActive()) {
      await loadMessages();
      return;
    }
    searching = true;
    selectedMessage = null;
    listCursor = null;
    detailOverlayVisible = false;
    currentPage = 1;
    searchHasNext = false;
    await execSearch('&direction=forward');
  }

  async function clearAllFilters() {
    clearMarkedSelection();
    if (searchDebounce) { clearTimeout(searchDebounce); searchDebounce = null; }
    searchQuery = '';
    searching = false;
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
    currentPage = 1;
    await loadMessages();
  }

  async function loadContactSuggestions() {
    if (contactsLoaded) return;
    try {
      const r = await fetch('/api/contacts');
      if (r.ok) {
        const data = await r.json();
        const seen = new Set<string>();
        const suggestions: { name: string; email: string }[] = [];
        for (const c of (data.contacts || [])) {
          const emails: { value: string; type_label: string }[] = c.emails || [];
          for (const e of emails) {
            if (e.value && !seen.has(e.value.toLowerCase())) {
              seen.add(e.value.toLowerCase());
              suggestions.push({ name: c.fn_name || '', email: e.value });
            }
          }
        }
        contactSuggestions = suggestions;
        contactsLoaded = true;
      }
    } catch {}
  }

  async function fetchAddrSuggestions(q: string) {
    if (!q.trim()) { addrDbSuggestions = []; return; }
    try {
      const r = await fetch(`/api/email/suggest-addrs?q=${encodeURIComponent(q)}`);
      if (r.ok) {
        const data = await r.json();
        addrDbSuggestions = data.addrs || [];
      }
    } catch {}
  }

  function onAddrInput() {
    if (addrSuggestDebounce) clearTimeout(addrSuggestDebounce);
    addrSuggestDebounce = setTimeout(() => fetchAddrSuggestions(addrFilterInput), 300);
  }

  function computePopoverStyle(anchorEl: HTMLElement, popEl: HTMLElement): string {
    const rect = anchorEl.getBoundingClientRect();
    const width = popEl.offsetWidth;
    const top = rect.bottom + 4;
    const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
    return `position: fixed; top: ${top}px; left: ${left}px; right: auto;`;
  }

  async function positionAddrPopover() {
    await tick();
    if (addrFilterEl && addrPopoverEl) addrPopoverStyle = computePopoverStyle(addrFilterEl, addrPopoverEl);
  }

  async function positionDatePopover() {
    await tick();
    if (dateFilterEl && datePopoverEl) datePopoverStyle = computePopoverStyle(dateFilterEl, datePopoverEl);
  }

  function removeFilterListeners() {
    window.removeEventListener('click', handleFilterOutsideClick);
    window.removeEventListener('resize', positionAddrPopover);
    window.removeEventListener('resize', positionDatePopover);
  }

  function toggleAddrFilter() {
    const willOpen = !filterAddrOpen;
    filterDateOpen = false;
    filterAddrOpen = willOpen;
    removeFilterListeners();
    if (willOpen) {
      loadContactSuggestions();
      positionAddrPopover();
      setTimeout(() => {
        window.addEventListener('click', handleFilterOutsideClick);
        window.addEventListener('resize', positionAddrPopover);
      }, 0);
    } else {
      addrDbSuggestions = [];
      addrFilterInput = '';
    }
  }

  function toggleDateFilter() {
    const willOpen = !filterDateOpen;
    filterAddrOpen = false;
    filterDateOpen = willOpen;
    removeFilterListeners();
    if (willOpen) {
      positionDatePopover();
      setTimeout(() => {
        window.addEventListener('click', handleFilterOutsideClick);
        window.addEventListener('resize', positionDatePopover);
      }, 0);
    }
  }

  function handleFilterOutsideClick(e: MouseEvent) {
    const target = e.target as Node;
    if (!addrFilterEl?.contains(target) && !dateFilterEl?.contains(target)) {
      filterAddrOpen = false;
      filterDateOpen = false;
      removeFilterListeners();
    }
  }

  function cursorParams(direction: string, msg?: Message): string {
    if (!msg) return `&direction=${direction}`;
    const sa = msg.sent_at != null ? `&cursor_sent_at=${msg.sent_at}` : '';
    return `${sa}&cursor_rowid=${msg.rowid}&direction=${direction}`;
  }

  async function loadMessages(params = '') {
    messagesLoading = true;
    try {
      let base: string;
      if (selectedFolderId) {
        base = `/api/email/messages?folder_id=${encodeURIComponent(selectedFolderId)}&limit=${pageSize}`;
      } else if (selectedAccountId && selectedMailbox) {
        base = `/api/email/messages?account_id=${encodeURIComponent(selectedAccountId)}&mailbox=${encodeURIComponent(selectedMailbox)}&limit=${pageSize}`;
      } else {
        return;
      }
      const r = await fetch(base + params);
      if (r.ok) {
        const data = await r.json();
        messages = data.messages || [];
        totalMessages = data.total || 0;
      }
    } catch {}
    messagesLoading = false;
  }

  async function execSearch(params = '', goingBackward = false) {
    const q = searchQuery.trim();
    let scope = '';
    if (selectedFolderId) {
      scope = `&folder_id=${encodeURIComponent(selectedFolderId)}`;
    } else if (selectedAccountId && selectedMailbox) {
      scope = `&account_id=${encodeURIComponent(selectedAccountId)}&mailbox=${encodeURIComponent(selectedMailbox)}`;
    }
    let filterParams = '';
    if (filterAddrs.length > 0) filterParams += `&addrs=${encodeURIComponent(filterAddrs.join(','))}`;
    if (filterDateFrom) filterParams += `&date_from=${encodeURIComponent(filterDateFrom)}`;
    if (filterDateTo) filterParams += `&date_to=${encodeURIComponent(filterDateTo)}`;
    messagesLoading = true;
    try {
      const r = await fetch(`/api/email/search?q=${encodeURIComponent(q)}${scope}${filterParams}&limit=${pageSize + 1}${params}`);
      if (r.ok) {
        const data = await r.json();
        const msgs: Message[] = data.messages || [];
        if (goingBackward) {
          // The backend fetches ASC+LIMIT then reverses to DESC. If 51 results were
          // returned, the first element (newest) is the look-ahead for the forward
          // direction — skip it. searchHasNext is always true: we came from a forward page.
          searchHasNext = true;
          messages = msgs.length > pageSize ? msgs.slice(1) : msgs;
        } else {
          searchHasNext = msgs.length > pageSize;
          messages = msgs.slice(0, pageSize);
        }
      }
    } catch {}
    messagesLoading = false;
  }

  async function nextPage() {
    if (!hasNext) return;
    clearMarkedSelection();
    const params = cursorParams('forward', messages.at(-1));
    currentPage++;
    if (searching) {
      await execSearch(params);
    } else {
      saveNavState();
      await loadMessages(params);
    }
    listCursor = null;
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function prevPage() {
    if (!hasPrev) return;
    clearMarkedSelection();
    const params = cursorParams('backward', messages[0]);
    currentPage--;
    if (searching) {
      await execSearch(params, true);
    } else {
      saveNavState();
      await loadMessages(params);
    }
    listCursor = null;
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function firstPage() {
    if (!hasPrev) return;
    clearMarkedSelection();
    currentPage = 1;
    if (searching) {
      await execSearch('&direction=forward');
    } else {
      saveNavState();
      await loadMessages('&direction=forward');
    }
    listCursor = null;
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function lastPage() {
    if (!hasNext || searching) return;
    clearMarkedSelection();
    currentPage = totalPages;
    saveNavState();
    await loadMessages('&direction=last');
    listCursor = null;
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function goToPage(page: number) {
    clearMarkedSelection();
    const p = Math.max(1, Math.min(page, totalPages));
    const off = (p - 1) * pageSize;
    currentPage = p;
    saveNavState();
    await loadMessages(`&offset=${off}`);
    listCursor = null;
    listScrollEl?.scrollTo({ top: 0 });
  }

  function saveNavState() {
    try {
      localStorage.setItem('mail-view-state', JSON.stringify({
        accountId: selectedAccountId,
        mailbox: selectedMailbox,
        folderId: selectedFolderId,
      }));
    } catch {}
  }

  function activatePageInput() {
    pageInputActive = true;
    pageInputValue = String(currentPage);
    tick().then(() => { pageInputEl?.select(); });
  }

  function confirmPageInput() {
    pageInputActive = false;
    const n = parseInt(pageInputValue, 10);
    if (!isNaN(n)) goToPage(n);
  }

  function cancelPageInput() {
    pageInputActive = false;
  }

  async function openMessage(msg: Message) {
    const activeThreadRequest = ++threadRequest;
    threadMessages = [];
    messageLoading = true;
    standaloneUnavailable = false;
    pgpDecryptGeneration++;
    pgpDecrypting = false;
    selectedMessage = msg;
    actionsOpen = false;
    metaExpanded = false;
    attachmentsExpanded = false;
    imagesNoticeDismissed = false;
    viewMode = 'text'; // reset while loading
    pgpDecryptedHtml = '';
    pgpDecryptedText = '';
    clearPgpAttachments();
    pgpDecryptError = '';
    pgpBadges = [];
    pgpPublicKeyImportError = '';
    pgpPublicKeyImportSuccess = '';
    pgpImportablePublicKeyAttachments = new Set();
    pgpKeyUnlocked = isUnlocked();
    let loaded = false;
    try {
      const url = `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`;
      const r = await fetch(url);
      if (r.ok) {
        loaded = true;
        const full = await r.json() as Message;
        viewMode = htmlAddsValue(full.body_html) ? 'html' : 'text';
        selectedMessage = full;
        void loadThread(full, activeThreadRequest);
        messageLoading = false;
        if (full.is_encrypted && getKeys().length > 0) {
          void pgpDecrypt(full);
        }
        // Update in list
        messages = messages.map(m =>
          m.uid === msg.uid && m.account_id === msg.account_id && m.mailbox === msg.mailbox ? full : m
        );
        // Mark as read
        if (!full.is_read) {
          await setFlag(full, '\\Seen', true);
        }
        try {
          const rawBytes = await fetchRawMessage(full);
          await harvestAutocrypt(rawBytes, full);
          await loadMessagePublicKeyCandidates(full, full.attachments);
          if (full.is_signed && !full.is_encrypted) {
            const verificationKeys = await listPublicKeys().then(readVerificationKeys).catch(() => []);
            pgpBadges = await verifySignedMessage(rawBytes, verificationKeys);
          }
        } catch {
          // Reading ordinary mail must not fail when raw/key harvesting is unavailable.
        }
      }
    } catch {}
    messageLoading = false;
    if (standalone && !loaded) {
      clearPgpAttachments();
      selectedMessage = null;
      standaloneUnavailable = true;
    }
  }

  function clearOpenMessage() {
    threadRequest++;
    threadMessages = [];
    threadDialog?.close();
    clearPgpAttachments();
    selectedMessage = null;
    detailOverlayVisible = false;
  }

  function returnToListFromReader() {
    if (!selectedMessage || standalone) return;
    listCursor = messages.find(m => sameMessage(m, selectedMessage)) ?? listCursor;
    actionsOpen = false;
    moveSubmenuOpen = false;
    if (usesDetailOverlay) detailOverlayVisible = false;
    void focusListMessage(listCursor);
  }

  function openMessageFromList(msg: Message, focusReader = false) {
    clearMarkedSelection();
    listCursor = msg;
    detailOverlayVisible = true;
    void openMessage(msg);
    if (focusReader) void tick().then(() => detailEl?.focus());
  }

  async function loadThread(msg: Message, request: number) {
    try {
      const url = `/api/email/thread?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`;
      const response = await fetch(url);
      if (!response.ok || request !== threadRequest) return;
      const data = await response.json();
      if (request === threadRequest) threadMessages = data.messages || [];
    } catch {
      if (request === threadRequest) threadMessages = [];
    }
  }

  function isCurrentThreadMessage(node: ThreadNode): boolean {
    if (!selectedMessage) return false;
    if (node.message_id && selectedMessage.message_id) {
      return node.message_id === selectedMessage.message_id;
    }
    return node.account_id === selectedMessage.account_id &&
      node.mailbox === selectedMessage.mailbox && node.uid === selectedMessage.uid;
  }

  function buildThreadTree(nodes: ThreadNode[]): ThreadTreeNode[] {
    const ids = new Set(nodes.map(node => node.message_id).filter((id): id is string => !!id));
    const childrenByParent = new Map<string, ThreadNode[]>();
    for (const node of nodes) {
      if (!node.parent_message_id) continue;
      const children = childrenByParent.get(node.parent_message_id) ?? [];
      children.push(node);
      childrenByParent.set(node.parent_message_id, children);
    }

    const included = new Set<ThreadNode>();
    const branch = (node: ThreadNode, ancestry: Set<ThreadNode>): ThreadTreeNode => {
      included.add(node);
      const nextAncestry = new Set(ancestry).add(node);
      const children = node.message_id
        ? (childrenByParent.get(node.message_id) ?? [])
            .filter(child => !nextAncestry.has(child))
            .map(child => branch(child, nextAncestry))
        : [];
      return { message: node, children };
    };
    const roots = nodes.filter(node => !node.parent_message_id || !ids.has(node.parent_message_id));
    const tree = roots.map(node => branch(node, new Set()));
    for (const node of nodes) {
      if (!included.has(node)) tree.push(branch(node, new Set()));
    }
    return tree;
  }

  function openThreadMessage(node: ThreadNode) {
    threadDialog?.close();
    clearMarkedSelection();
    const exact = messages.find(message =>
      message.account_id === node.account_id && message.mailbox === node.mailbox && message.uid === node.uid
    );
    const visible = exact ?? (node.message_id
      ? messages.find(message => message.message_id === node.message_id)
      : undefined);
    if (!standalone && visible) {
      openMessageFromList(visible, true);
      return;
    }
    if (!standalone) listCursor = null;
    detailOverlayVisible = true;
    void openMessage(locatorMessage({ accountId: node.account_id, mailbox: node.mailbox, uid: node.uid }));
  }

  $: threadTree = buildThreadTree(threadMessages);

  async function setFlag(msg: Message, flag: string, set: boolean): Promise<boolean> {
    try {
      const response = await fetch('/api/email/message/flags', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          account_id: msg.account_id,
          mailbox: msg.mailbox,
          uid: msg.uid,
          flags: [flag],
          set,
        }),
      });
      if (!response.ok) return false;
      // Optimistic update
      const newFlags = set
        ? [...msg.flags, flag].filter((f, i, a) => a.indexOf(f) === i)
        : msg.flags.filter(f => f !== flag);
      const updated = { ...msg, flags: newFlags, is_read: newFlags.includes('\\Seen'), is_flagged: newFlags.includes('\\Flagged') };
      messages = messages.map(m =>
        m.uid === msg.uid && m.account_id === msg.account_id && m.mailbox === msg.mailbox ? updated : m
      );
      if (selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id &&
          selectedMessage?.mailbox === msg.mailbox) {
        selectedMessage = updated;
      }
      return true;
    } catch {
      return false;
    }
  }

  async function selectAfterRemoval(msg: Message, originalIndex: number, hadNext: boolean, hadPrev: boolean) {
    if (standalone) return;
    let next: Message | undefined = messages[originalIndex];
    if (!next && hadNext) {
      currentPage++;
      const params = cursorParams('forward', msg);
      if (searching) {
        await execSearch(params);
      } else {
        saveNavState();
        await loadMessages(params);
      }
      next = messages[0];
    }
    next ??= messages[originalIndex - 1];
    if (!next && hadPrev) {
      currentPage--;
      const params = cursorParams('backward', msg);
      if (searching) {
        await execSearch(params, true);
      } else {
        saveNavState();
        await loadMessages(params);
      }
      next = messages.at(-1);
    }
    listCursor = next ?? null;
    await focusListMessage(listCursor);
  }

  async function removeVisibleMessage(msg: Message) {
    const originalIndex = messages.findIndex(m => sameMessage(m, msg));
    const hadNext = hasNext;
    const hadPrev = hasPrev;
    messages = messages.filter(m => !sameMessage(m, msg));
    if (!searching) totalMessages = Math.max(0, totalMessages - 1);
    const affectedCursor = sameMessage(listCursor, msg);
    const affectedReader = sameMessage(selectedMessage, msg);
    if (affectedReader) clearOpenMessage();
    if (standalone) {
      if (affectedReader) standaloneUnavailable = true;
      return;
    }
    if (affectedCursor || affectedReader) {
      await selectAfterRemoval(msg, Math.max(originalIndex, 0), hadNext, hadPrev);
    }
  }

  async function deleteMessage(msg: Message) {
    const inTrash = folderKind(msg.mailbox) === 'trash';
    if (inTrash && !confirm(tl('mail_deleteMessageConfirm'))) return;
    await fetch(
      `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`,
      { method: 'DELETE' }
    );
    await removeVisibleMessage(msg);
  }

  async function moveMessage(msg: Message, destMailbox: string) {
    await fetch('/api/email/message/move', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid, dest_mailbox: destMailbox }),
    });
    await removeVisibleMessage(msg);
  }

  function archiveMailboxFor(msg: Message): string | null {
    const candidates = (mailboxesByAccount[msg.account_id] || [])
      .filter(mb => mb.subscribed && mb.name !== msg.mailbox && folderKind(mb.name) === 'archive');
    return candidates.length === 1 ? candidates[0].name : null;
  }

  function archiveMessage(msg: Message) {
    const archiveMailbox = archiveMailboxFor(msg);
    if (archiveMailbox) void moveMessage(msg, archiveMailbox);
  }

  function keepFailedSelection(failed: Set<string>) {
    markedMessageKeys = failed;
    selectionAnchorKey = failed.size > 0 ? [...failed][0] : null;
    bulkActionError = failed.size > 0 ? tl('mail_bulkActionError') : '';
    bulkMoveOpen = false;
  }

  async function applyBulkFlags(flag: string, set: boolean) {
    bulkActionBusy = true;
    bulkActionError = '';
    const failed = new Set<string>();
    for (const msg of selectedMessages) {
      if (!(await setFlag(msg, flag, set))) failed.add(messageKey(msg));
    }
    keepFailedSelection(failed);
    await loadUnreadCounts();
    bulkActionBusy = false;
  }

  async function requestMoveMessage(msg: Message, destMailbox: string): Promise<boolean> {
    try {
      const response = await fetch('/api/email/message/move', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid, dest_mailbox: destMailbox }),
      });
      return response.ok;
    } catch {
      return false;
    }
  }

  async function applyBulkMove(destinationFor: (msg: Message) => string | null) {
    bulkActionBusy = true;
    bulkActionError = '';
    const failed = new Set<string>();
    for (const msg of selectedMessages) {
      const destination = destinationFor(msg);
      if (!destination || !(await requestMoveMessage(msg, destination))) failed.add(messageKey(msg));
    }
    await loadMessages();
    keepFailedSelection(new Set([...failed].filter(key => messages.some(msg => messageKey(msg) === key))));
    await loadUnreadCounts();
    bulkActionBusy = false;
  }

  async function applyBulkDelete() {
    if (selectedMessages.some(msg => folderKind(msg.mailbox) === 'trash') &&
        !confirm(tl('mail_bulkDeleteConfirm', { n: selectedMessages.length }))) return;
    bulkActionBusy = true;
    bulkActionError = '';
    const failed = new Set<string>();
    for (const msg of selectedMessages) {
      try {
        const response = await fetch(
          `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`,
          { method: 'DELETE' },
        );
        if (!response.ok) failed.add(messageKey(msg));
      } catch {
        failed.add(messageKey(msg));
      }
    }
    await loadMessages();
    keepFailedSelection(new Set([...failed].filter(key => messages.some(msg => messageKey(msg) === key))));
    await loadUnreadCounts();
    bulkActionBusy = false;
  }

  function isTypingTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    return target.isContentEditable ||
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement;
  }

  async function moveCursor(step: -1 | 1, openInReader: boolean) {
    const current = openInReader ? selectedMessage : listCursor;
    let index = current ? messages.findIndex(msg => sameMessage(msg, current)) : -1;
    let target: Message | undefined;
    if (index < 0) {
      target = step === 1 ? messages[0] : messages.at(-1);
    } else {
      target = messages[index + step];
    }
    if (!target && step === 1 && hasNext) {
      await nextPage();
      target = messages[0];
    } else if (!target && step === -1 && hasPrev) {
      await prevPage();
      target = messages.at(-1);
    }
    if (!target) return;
    listCursor = target;
    if (openInReader) {
      openMessageFromList(target, true);
    } else {
      await focusListMessage(target);
    }
  }

  function scrollReader(key: string) {
    const scroller = (standalone || usesDetailOverlay ? detailEl : detailBodyEl);
    if (!scroller) return;
    if (key === 'Home') {
      scroller.scrollTo({ top: 0 });
    } else if (key === 'End') {
      scroller.scrollTo({ top: scroller.scrollHeight });
    } else {
      const delta = scroller.clientHeight * 0.9 * (key === 'PageDown' ? 1 : -1);
      scroller.scrollBy({ top: delta });
    }
  }

  function shortcutMessage(inReader: boolean): Message | null {
    return inReader ? selectedMessage : (listCursor ?? selectedMessage);
  }

  function handleMailKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || isTypingTarget(e.target) || document.querySelector('dialog[open]')) return;
    const target = e.target instanceof HTMLElement ? e.target : null;
    const inReader = !!(detailEl && target && detailEl.contains(target) && selectedMessage);
    const inList = !standalone && !!target?.closest('.message-list');
    const inListRow = inList && !!target?.closest('.message-row');
    const isSpace = e.code === 'Space' || e.key === ' ';

    if (!standalone && selectedMessages.length > 0 && !e.altKey && !e.ctrlKey && !e.metaKey &&
        !e.shiftKey && e.key === 'Escape') {
      e.preventDefault();
      clearMarkedSelection();
      return;
    }

    if (inListRow && listCursor && !e.altKey && isSpace &&
        ((e.ctrlKey || e.metaKey) && !e.shiftKey)) {
      e.preventDefault();
      toggleMarkedMessage(listCursor);
      return;
    }

    if (inListRow && listCursor && !e.altKey && !e.ctrlKey && !e.metaKey && e.shiftKey && isSpace) {
      e.preventDefault();
      markMessageRange(listCursor);
      return;
    }

    if (inList && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        void moveCursor(e.key === 'ArrowDown' ? 1 : -1, false);
        return;
      }
      if (inListRow && e.key === 'Enter' && listCursor) {
        e.preventDefault();
        openMessageFromList(listCursor, true);
        return;
      }
    }

    if (selectedMessages.length > 0) return;

    if (!standalone && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey &&
        (e.key === 'j' || e.key === 'k')) {
      e.preventDefault();
      const step = e.key === 'j' ? 1 : -1;
      if (messagesLoading && !listCursor && !selectedMessage) {
        queuedReaderNavigation = step;
      } else {
        void moveCursor(step, true);
      }
      return;
    }

    if (inReader && !e.altKey && !e.ctrlKey && !e.metaKey) {
      if (!standalone && !e.shiftKey && e.key === 'Escape') {
        e.preventDefault();
        returnToListFromReader();
        return;
      }
      if (!e.shiftKey && ['PageUp', 'PageDown', 'Home', 'End'].includes(e.key)) {
        e.preventDefault();
        scrollReader(e.key);
        return;
      }
    }

    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const msg = shortcutMessage(inReader);
    if (!msg) return;
    if (!e.shiftKey && e.key === 'r') {
      e.preventDefault();
      openCompose(msg);
    } else if (!e.shiftKey && e.key === 'a') {
      e.preventDefault();
      openComposeReplyAll(msg);
    } else if (!e.shiftKey && e.key === 'f') {
      e.preventDefault();
      openComposeForward(msg);
    } else if (!e.shiftKey && e.key === 'e' && archiveMailboxFor(msg)) {
      e.preventDefault();
      archiveMessage(msg);
    } else if (e.key === '#') {
      e.preventDefault();
      void deleteMessage(msg);
    } else if (e.shiftKey && e.key.toLowerCase() === 'i') {
      e.preventDefault();
      void setFlag(msg, '\\Seen', true);
    } else if (e.shiftKey && e.key.toLowerCase() === 'u') {
      e.preventDefault();
      void setFlag(msg, '\\Seen', false);
    }
  }

  function openPopout(msg: Message) {
    const params = new URLSearchParams({
      account_id: msg.account_id,
      mailbox: msg.mailbox,
      uid: String(msg.uid),
    });
    window.open(`/mail/view/?${params}`, '_blank', 'popup=yes,width=980,height=760,noopener,noreferrer');
  }

  // ── Account management ───────────────────────────────────────────────────

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

  function openAddAccount() {
    accountForm = emptyAccountForm();
    accountError = '';
    accountStep = 'basic';
    autodiscoverResult = null;
    autodiscoverError = '';
    showAdvanced = false;
    accountDialog?.showModal();
  }

  function openEditAccount(acc: Account) {
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
        await loadAccounts();
        await loadIdentities();
        for (const acc of accounts) {
          await loadMailboxes(acc.account_id);
        }
      }
    } catch {
      accountError = tl('mail_saveAccountError');
    }
    accountSaving = false;
  }

  async function removeAccount(acc: Account) {
    if (!confirm(tl('mail_removeAccountConfirm', { name: acc.display_name }))) return;
    await fetch(`/api/email/account?account_id=${encodeURIComponent(acc.account_id)}`, { method: 'DELETE' });
    await loadAccounts();
    mailboxesByAccount = Object.fromEntries(
      Object.entries(mailboxesByAccount).filter(([id]) => id !== acc.account_id)
    );
    if (collapsedAccountIds.has(acc.account_id)) {
      const next = new Set(collapsedAccountIds);
      next.delete(acc.account_id);
      collapsedAccountIds = next;
      saveCollapsedAccounts();
    }
    if (selectedAccountId === acc.account_id) {
      selectedAccountId = null;
      selectedMailbox = null;
      messages = [];
    }
  }

  // ── Identity management ──────────────────────────────────────────────────

  function openManageIdentities(accountId: string) {
    identityAccountId = accountId;
    identityMode = 'idle';
    identityForm = emptyIdentityForm();
    identityForm.account_id = accountId;
    identityError = '';
    identityDialog?.showModal();
  }

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
        await loadIdentities();
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
    await loadIdentities();
  }

  async function deleteIdentity(id: Identity) {
    if (!confirm(tl('mail_removeIdentityConfirm', { name: id.name, email: id.email }))) return;
    await fetch(`/api/email/identity?identity_id=${encodeURIComponent(id.identity_id)}`, { method: 'DELETE' });
    await loadIdentities();
  }

  // ── Compose ──────────────────────────────────────────────────────────────

  function openCompose(replyTo?: Message) {
    composeError = '';
    composeSending = false;
    composeCc = '';
    composeBcc = '';
    composeAttachments = [];
    showComposeCcBcc = false;
    composeReplyAll = false;
    composeEncrypt = false;
    composeSign = false;
    composeSignOptedOut = false;
    if (replyTo) {
      composeMode = 'reply';
      composeTo = replyTo.from || '';
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = replyQuote(replyTo);
      composeInReplyTo = replyTo.message_id;
      composeReferences = replyTo.message_id;
      // Try to match the identity that was the recipient of the original message
      const accountIdentities = identities.filter(i => i.account_id === replyTo.account_id);
      const toAddresses = (replyTo.to ?? []).map(a => a.toLowerCase());
      const matched = accountIdentities.find(i => toAddresses.some(a => a.includes(i.email.toLowerCase())));
      const defaultIdentity = accountIdentities.find(i => i.is_default);
      composeIdentityId = (matched ?? defaultIdentity ?? identities[0])?.identity_id ?? '';
    } else {
      composeTo = '';
      composeSubject = '';
      composeBody = '';
      composeInReplyTo = null;
      composeReferences = null;
      composeMode = 'new';
      composeIdentityId = (identities.find(i => i.is_default) ?? identities[0])?.identity_id ?? '';
    }
    reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
    void loadComposeTrustedKeys();
    composeDialog?.showModal();
  }

  function openComposeReplyAll(replyTo: Message) {
    openCompose(replyTo);
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
    composeReplyAll = true;
    composeMode = 'replyAll';
  }

  function openComposeForward(forwardFrom: Message) {
    openCompose();
    composeMode = 'forward';
    const accountIdentities = identities.filter(i => i.account_id === forwardFrom.account_id);
    composeIdentityId = (accountIdentities.find(i => i.is_default) ?? accountIdentities[0] ?? identities[0])?.identity_id ?? '';
    composeSubject = /^fwd:/i.test(forwardFrom.subject ?? '')
      ? (forwardFrom.subject ?? '')
      : `Fwd: ${forwardFrom.subject || ''}`;
    composeBody = forwardQuote(forwardFrom);
    composeInReplyTo = null;
    composeReferences = null;
    reconcileComposeSigning(composeIdentityId, composeSignOptedOut, composeTrustedKeys, composeStoredSigningFingerprints);
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
          await pgpUnlock(false);
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
        r = await fetch('/api/email/send-raw', { method: 'POST', body: fd });
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
        r = await fetch('/api/email/send', { method: 'POST', body: fd });
      }
      if (!r.ok) {
        const text = await r.text();
        composeError = text || tl('mail_sendError');
      } else {
        composeDialog?.close();
      }
    } catch (error) {
      composeError = error instanceof Error ? error.message : tl('mail_sendError');
    }
    composeSending = false;
  }

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

  // ── Virtual folders ──────────────────────────────────────────────────────

  function openFolderCreate() {
    editingFolder = null;
    folderName = '';
    folderSelectedMailboxes = new Set();
    folderOnlyFlagged = false;
    folderOnlyAttachments = false;
    folderDialog?.showModal();
  }

  function openFolderEdit(folder: VirtualFolder) {
    editingFolder = folder;
    folderName = folder.display_name;
    folderOnlyFlagged = folder.only_flagged ?? false;
    folderOnlyAttachments = folder.only_attachments ?? false;
    // A single rule with both fields null means "all mailboxes" — show nothing checked
    const isMatchAll = folder.rules.length === 1 &&
      folder.rules[0].account_id === null &&
      folder.rules[0].mailbox_pattern === null;
    folderSelectedMailboxes = isMatchAll
      ? new Set()
      : new Set(folder.rules.map(r => `${r.account_id ?? ''}::${r.mailbox_pattern ?? ''}`));
    folderDialog?.showModal();
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
      folderDialog?.close();
      await loadVirtualFolders();
    } catch {}
    folderSaving = false;
  }

  async function deleteFolder(folder: VirtualFolder) {
    await fetch(`/api/email/folder?folder_id=${encodeURIComponent(folder.folder_id)}`, { method: 'DELETE' });
    if (selectedFolderId === folder.folder_id) {
      clearMarkedSelection();
      selectedFolderId = null;
      messages = [];
    }
    await loadVirtualFolders();
  }

  // ── SSE ──────────────────────────────────────────────────────────────────

  function connectSSE() {
    sseSource = new EventSource('/api/events/stream');
    sseSource.addEventListener('new_email', (e) => {
      showNewMailNotification(JSON.parse(e.data) as NewMailNotification, tl);
    });
    sseSource.addEventListener('emails_refetch', async (e) => {
      const data = JSON.parse(e.data);
      if (standalone && selectedMessage && data.account_id === selectedMessage.account_id &&
          data.mailbox === selectedMessage.mailbox) {
        await openMessage(selectedMessage);
      } else if (!searching && data.account_id === selectedAccountId && data.mailbox === selectedMailbox) {
        await loadMessages();
      }
      if (!standalone) loadUnreadCounts();
    });
    sseSource.addEventListener('email_updated', (e) => {
      const data = JSON.parse(e.data);
      const updateFlags = (m: Message) => m.uid === data.uid && m.account_id === data.account_id && m.mailbox === data.mailbox
        ? { ...m, flags: data.flags, is_read: data.flags.includes('\\Seen'), is_flagged: data.flags.includes('\\Flagged') }
        : m;
      messages = messages.map(m =>
        updateFlags(m)
      );
      if (selectedMessage) selectedMessage = updateFlags(selectedMessage);
      if (!standalone) loadUnreadCounts();
    });
    sseSource.addEventListener('email_removed', (e) => {
      const data = JSON.parse(e.data);
      const nextMarked = new Set(markedMessageKeys);
      nextMarked.delete(identityKey(data.account_id, data.mailbox, data.uid));
      if (nextMarked.size !== markedMessageKeys.size) {
        markedMessageKeys = nextMarked;
        if (nextMarked.size === 0) selectionAnchorKey = null;
      }
      const before = messages.length;
      messages = messages.filter(m => !(m.uid === data.uid && m.account_id === data.account_id && m.mailbox === data.mailbox));
      if (messages.length < before) totalMessages = Math.max(0, totalMessages - 1);
      if (selectedMessage?.uid === data.uid && selectedMessage?.account_id === data.account_id &&
          selectedMessage?.mailbox === data.mailbox) {
        clearOpenMessage();
        if (standalone) standaloneUnavailable = true;
      }
      if (!standalone) loadUnreadCounts();
    });
    sseSource.addEventListener('accounts_changed', async () => {
      if (standalone && selectedAccountId) {
        await loadMailboxes(selectedAccountId);
      } else {
        await loadAccounts();
        for (const acc of accounts) await loadMailboxes(acc.account_id);
      }
    });
    sseSource.addEventListener('mailboxes_changed', async (e) => {
      const data = JSON.parse(e.data);
      if (!standalone || data.account_id === selectedAccountId) await loadMailboxes(data.account_id);
    });
    sseSource.addEventListener('sync_started', (e) => {
      const { account_id, mailbox } = JSON.parse(e.data);
      syncingMailboxes = new Set([...syncingMailboxes, `${account_id}|${mailbox}`]);
    });
    sseSource.addEventListener('sync_completed', (e) => {
      const { account_id, mailbox } = JSON.parse(e.data);
      const next = new Set(syncingMailboxes);
      next.delete(`${account_id}|${mailbox}`);
      syncingMailboxes = next;
      if (!standalone) loadUnreadCounts();
    });
    sseSource.onerror = () => {
      sseSource?.close();
      setTimeout(connectSSE, 5000);
    };
  }

  // ── Helpers ──────────────────────────────────────────────────────────────

  function formatDate(ts: number | null): string {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    const now = new Date();
    if (d.toDateString() === now.toDateString()) {
      return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    }
    const opts: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
    if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
    return d.toLocaleDateString([], opts);
  }

  function formatMessageDateTime(ts: number | null): string {
    return ts ? new Date(ts * 1000).toLocaleString() : '';
  }

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

  // Tags that convey something plain text cannot (links, images, tables, lists,
  // headings, formatting). Structural wrappers (html/head/body/div/p/span/br/font)
  // are excluded so "basically plain text" HTML is treated as plain text.
  const MEANINGFUL_HTML =
    /<(a\s[^>]*href|img|table|thead|tbody|tr|td|th|ul|ol|li|h[1-6]|blockquote|hr|b|strong|i|em|u|s|strike|code|pre|mark)\b/i;

  function htmlAddsValue(html: string | null): boolean {
    return !!html && MEANINGFUL_HTML.test(html);
  }

  type BodySegment = { text: string } | { href: string; text: string };

  function linkifySegments(text: string): BodySegment[] {
    const re = /(https?:\/\/[^\s<>()]+[^\s<>().,;:!?'"])/g;
    const out: BodySegment[] = [];
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      if (m.index > last) out.push({ text: text.slice(last, m.index) });
      out.push({ href: m[0], text: m[0] });
      last = m.index + m[0].length;
    }
    if (last < text.length) out.push({ text: text.slice(last) });
    return out;
  }

  // ── Resize ───────────────────────────────────────────────────────────────

  function startResize(which: 'sidebar' | 'list', e: MouseEvent) {
    resizing = which;
    resizeStartX = e.clientX;
    resizeStartWidth = which === 'sidebar' ? sidebarWidth : listWidth;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    e.preventDefault();
  }

  function handleResizeMove(e: MouseEvent) {
    if (!resizing) return;
    const delta = e.clientX - resizeStartX;
    if (resizing === 'sidebar') {
      sidebarWidth = Math.max(140, Math.min(400, resizeStartWidth + delta));
    } else {
      listWidth = Math.max(180, Math.min(600, resizeStartWidth + delta));
    }
  }

  function resizeWithKeyboard(which: 'sidebar' | 'list', e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const delta = (e.shiftKey ? 25 : 10) * (e.key === 'ArrowRight' ? 1 : -1);
    if (which === 'sidebar') {
      sidebarWidth = Math.max(140, Math.min(400, sidebarWidth + delta));
    } else {
      listWidth = Math.max(180, Math.min(600, listWidth + delta));
    }
  }

  function stopResize() {
    if (!resizing) return;
    resizing = null;
    document.body.style.cursor = '';
    document.body.style.userSelect = '';
  }

  // ── Actions dropdown ─────────────────────────────────────────────────────

  function toggleActions(e: MouseEvent) {
    e.stopPropagation();
    actionsOpen = !actionsOpen;
    if (actionsOpen) {
      setTimeout(() => window.addEventListener('click', handleWindowClick), 0);
    } else {
      moveSubmenuOpen = false;
      window.removeEventListener('click', handleWindowClick);
    }
  }

  function handleWindowClick(e: MouseEvent) {
    const target = e.target as Node;
    if (!actionsEl?.contains(target)) {
      actionsOpen = false;
      moveSubmenuOpen = false;
      window.removeEventListener('click', handleWindowClick);
    }
  }

  function humanSize(bytes: number): string {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  }

  function senderName(from: string | null): string {
    if (!from) return '';
    const match = from.match(/^(.+?)\s*<[^>]+>/);
    return match ? match[1].trim() : from;
  }

  type FolderKind = 'inbox' | 'sent' | 'drafts' | 'trash' | 'junk' | 'archive' | 'folder';

  function folderKind(name: string): FolderKind {
    const leaf = (name.split(/[\/.]/).pop() || name).toLowerCase().trim();
    if (leaf === 'inbox') return 'inbox';
    if (leaf.includes('sent')) return 'sent';
    if (leaf.includes('draft')) return 'drafts';
    if (leaf.includes('trash') || leaf.includes('deleted') || leaf.includes('bin')) return 'trash';
    if (leaf.includes('junk') || leaf.includes('spam')) return 'junk';
    if (leaf.includes('archive')) return 'archive';
    return 'folder';
  }

  const FOLDER_PRIORITY: Record<FolderKind, number> = {
    inbox: 0, drafts: 1, sent: 2, junk: 3, archive: 4, trash: 5, folder: 6,
  };

  function folderSortKey(name: string): [number, string] {
    return [FOLDER_PRIORITY[folderKind(name)], name.toLowerCase()];
  }

  $: sidebarAccountSections = accounts.map(account => {
    const collapsed = collapsedAccountIds.has(account.account_id);
    const mailboxes = (mailboxesByAccount[account.account_id] || [])
      .filter(mailbox => mailbox.subscribed)
      .toSorted((a, b) => {
        const [ap, an] = folderSortKey(a.name);
        const [bp, bn] = folderSortKey(b.name);
        return ap !== bp ? ap - bp : an.localeCompare(bn);
      })
      .map(mailbox => ({
        mailbox,
        unread_count: mailboxUnreadCounts[account.account_id]?.[mailbox.name] ?? 0,
      }));
    const unread_count = mailboxes.reduce((sum, item) => sum + item.unread_count, 0);
    return {
      account,
      collapsed,
      unread_count,
      mailboxes: collapsed ? mailboxes.filter(item => item.unread_count > 0) : mailboxes,
    };
  });

  $: isSentFolder = !!(selectedMailbox && /sent/i.test(selectedMailbox));
  $: selectedMessages = messages.filter(msg => markedMessageKeys.has(messageKey(msg)));
  $: bulkAccountId = selectedMessages.length > 0 &&
      selectedMessages.every(msg => msg.account_id === selectedMessages[0].account_id)
    ? selectedMessages[0].account_id
    : null;
  $: bulkSourceMailboxes = new Set(selectedMessages.map(msg => msg.mailbox));
  $: bulkMoveDestinations = bulkAccountId
    ? (mailboxesByAccount[bulkAccountId] || []).filter(mb => mb.subscribed && !bulkSourceMailboxes.has(mb.name))
    : [];
  $: canBulkArchive = selectedMessages.length > 0 && selectedMessages.every(msg => archiveMailboxFor(msg) !== null);
  $: primaryMetaLabel = isSentFolder ? tl('mail_metaTo') : tl('mail_metaFrom');
  $: FOLDER_NAMES = {
    inbox: tl('mail_folder_inbox'),
    sent: tl('mail_folder_sent'),
    drafts: tl('mail_folder_drafts'),
    trash: tl('mail_folder_trash'),
    junk: tl('mail_folder_junk'),
    archive: tl('mail_folder_archive'),
    folder: '',
  };
  $: primaryMetaValue = selectedMessage
    ? (isSentFolder ? selectedMessage.to.join(', ') : (selectedMessage.from || ''))
    : '';
  $: layoutColumns = standalone || isPhone
    ? '1fr'
    : usesDetailOverlay
      ? `${sidebarWidth}px 5px 1fr`
      : `${sidebarWidth}px 5px ${listWidth}px 5px 1fr`;
  $: {
    selectedMessage;
    usesDetailOverlay;
    locale;
    standalone;
    void tick().then(updateDetailHeaderLayout);
  }

  $: totalPages = totalMessages === 0 ? 0 : Math.ceil(totalMessages / pageSize);
  $: hasPrev = currentPage > 1;
  $: hasNext = searching ? searchHasNext : currentPage < totalPages;
  $: pageStart = totalMessages === 0 ? 0 : (currentPage - 1) * pageSize + 1;
  $: pageEnd = Math.min(currentPage * pageSize, totalMessages);

  $: renderedHtml = selectedMessage?.body_html
    ? prepareEmailHtml(selectedMessage.body_html, {
      accountId: selectedMessage.account_id,
      mailbox: selectedMessage.mailbox,
      uid: selectedMessage.uid,
    })
    : '';
  $: htmlHasValue = htmlAddsValue(selectedMessage?.body_html ?? null);
  $: encryptedHtmlHasValue = htmlAddsValue(pgpDecryptedHtml || null);
  $: canToggleBodyView = selectedMessage?.is_encrypted ? encryptedHtmlHasValue : htmlHasValue;
  $: displayedText = selectedMessage?.is_encrypted
    ? pgpDecryptedText
    : (selectedMessage?.body_text ?? '');
  $: bodyTextSegments = linkifySegments(displayedText);
  $: visibleServerAttachments = selectedMessage?.attachments.filter(att =>
    !isPgpSignatureAttachment(att) &&
    (!selectedMessage?.is_encrypted || !isPgpTransportAttachment(att))
  ) ?? [];

  $: emailOrigins = imageOriginsFromHtml(renderedHtml);

  $: blockedOrigins = emailOrigins.filter(o => !imageOrigins.includes(o) && !blacklistOrigins.includes(o));
  $: allowedEmailOrigins = emailOrigins.filter(o => imageOrigins.includes(o));
  $: silentEmailOrigins = emailOrigins.filter(o => blacklistOrigins.includes(o));
</script>

{#snippet folderIcon(kind: string)}
  <span class="folder-icon">
    {#if kind === 'inbox'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>
    {:else if kind === 'sent'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
    {:else if kind === 'drafts'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>
    {:else if kind === 'trash'}
      <UiIcon name="trash" size={15} />
    {:else if kind === 'junk'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="7.86 2 16.14 2 22 7.86 22 16.14 16.14 22 7.86 22 2 16.14 2 7.86"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
    {:else if kind === 'archive'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="21 8 21 21 3 21 3 8"/><rect x="1" y="3" width="22" height="5"/><line x1="10" y1="12" x2="14" y2="12"/></svg>
    {:else if kind === 'virtual'}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg>
    {:else}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
    {/if}
  </span>
{/snippet}

{#snippet threadBranch(nodes: ThreadTreeNode[])}
  {#each nodes as branch}
    {@const node = branch.message}
    <li>
      <button
        class="thread-message"
        class:current={isCurrentThreadMessage(node)}
        aria-current={isCurrentThreadMessage(node) ? 'true' : undefined}
        on:click={() => openThreadMessage(node)}
      >
        <span class="thread-message-from">{senderName(node.from) || tl('mail_unknownSender')}</span>
        <span class="thread-message-subject">{node.subject || tl('mail_noSubject')}</span>
        <span class="thread-message-date">{formatDate(node.sent_at)}</span>
      </button>
      {#if branch.children.length > 0}
        <ul>{@render threadBranch(branch.children)}</ul>
      {/if}
    </li>
  {/each}
{/snippet}

<div class="mail-page" class:standalone>
  <!-- ── Layout ─────────────────────────────────────────────────────────────── -->
  <div class="mail-layout" class:standalone style="grid-template-columns: {layoutColumns}">
    {#if !standalone}
    <!-- ── Sidebar ─────────────────────────────────────────────────────────── -->
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
              on:click={() => selectVirtualFolder(folder.folder_id)}
              on:keydown={e => e.key === 'Enter' && selectVirtualFolder(folder.folder_id)}
            >
              {@render folderIcon('virtual')}
              <span class="folder-name">{folder.display_name}</span>
              {#if folderUnread > 0}
                <span class="unread-badge">{folderUnread > 999 ? '999+' : folderUnread}</span>
              {/if}
              <button class="icon-btn" on:click|stopPropagation={() => { sidebarOpen = false; openFolderEdit(folder); }} title={tl('mail_editVirtualFolder')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
              <button class="icon-btn delete-btn" on:click|stopPropagation={() => { sidebarOpen = false; deleteFolder(folder); }} title={tl('mail_removeAttachment')}>×</button>
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
              on:click={() => toggleAccountCollapsed(acc.account_id)}
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
            <button class="icon-btn" on:click={() => { sidebarOpen = false; openManageIdentities(acc.account_id); }} title={tl('mail_identities')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 3.6-7 8-7s8 3 8 7"/></svg></button>
            <button class="icon-btn" on:click={() => { sidebarOpen = false; openEditAccount(acc); }} title={tl('mail_editAccountHeading')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
            <button class="icon-btn delete-btn" on:click={() => { sidebarOpen = false; removeAccount(acc); }} title={tl('mail_removeAttachment')}>×</button>
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
              on:click={() => selectMailbox(acc.account_id, mb.name)}
              on:dragover={(e) => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { e.preventDefault(); dragOverMailbox = `${acc.account_id}|${mb.name}`; } }}
              on:dragleave={() => { if (dragOverMailbox === `${acc.account_id}|${mb.name}`) dragOverMailbox = null; }}
              on:drop={() => { if (draggedMessage?.account_id === acc.account_id && draggedMessage?.mailbox !== mb.name) { moveMessage(draggedMessage, mb.name); } draggedMessage = null; dragOverMailbox = null; }}
            >
              {@render folderIcon(folderKind(mb.name))}
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
        <button class="link-btn" on:click={() => { sidebarOpen = false; openAddAccount(); }}>+ {tl('mail_addAccount')}</button>
        <button class="link-btn" on:click={() => { sidebarOpen = false; openFolderCreate(); }}>+ {tl('mail_addVirtualFolder')}</button>
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
      on:mousedown={(e) => startResize('sidebar', e)}
      on:keydown={(e) => resizeWithKeyboard('sidebar', e)}
    ></div>

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
            <div class="search-filter-wrap" bind:this={addrFilterEl}>
              <button
                class="filter-icon-btn"
                class:active={filterAddrActive}
                title={tl('mail_filterBySender')}
                on:click|stopPropagation={toggleAddrFilter}
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
                        <span class="filter-chip">
                          {addr}
                          <button class="chip-remove" on:click={() => { filterAddrs = filterAddrs.filter(a => a !== addr); runSearch(); }}>×</button>
                        </span>
                      {/each}
                    </div>
                    <button class="filter-clear-btn" on:click={() => { filterAddrs = []; runSearch(); }}>
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
                            runSearch();
                          }}
                        />
                        <span class="suggestion-name">{s.name || s.email}</span>
                        {#if s.name}<span class="suggestion-email">{s.email}</span>{/if}
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
                            runSearch();
                          }}
                        />
                        <span class="suggestion-name">{addr}</span>
                      </label>
                    {/each}
                    {#if addrFilterInput
                      && !contactSuggestions.some(c => c.email.toLowerCase() === addrFilterInput.toLowerCase())
                      && !addrDbSuggestions.includes(addrFilterInput.toLowerCase())}
                      <button class="filter-add-custom" on:click={() => {
                        const v = addrFilterInput.trim();
                        if (v && !filterAddrs.includes(v)) { filterAddrs = [...filterAddrs, v]; runSearch(); }
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
                on:click|stopPropagation={toggleDateFilter}
                aria-pressed={filterDateActive}
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>
              </button>
              {#if filterDateOpen}
                <div class="filter-popover filter-popover-date" role="dialog" aria-label={tl('mail_filterByDate')} bind:this={datePopoverEl} style={datePopoverStyle}>
                  <label class="filter-date-label">
                    {tl('mail_filterDateFrom')}
                    <input type="date" bind:value={filterDateFrom} on:change={runSearch} />
                  </label>
                  <label class="filter-date-label">
                    {tl('mail_filterDateTo')}
                    <input type="date" bind:value={filterDateTo} on:change={runSearch} />
                  </label>
                  {#if filterDateActive}
                    <button class="filter-clear-btn" on:click={() => { filterDateFrom = ''; filterDateTo = ''; runSearch(); }}>
                      {tl('mail_filterClearDate')}
                    </button>
                  {/if}
                </div>
              {/if}
            </div>
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
              on:click={(e) => handleMessageRowClick(e, msg)}
              draggable="true"
              on:dragstart={(e) => { draggedMessage = msg; if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'; }}
              on:dragend={() => { draggedMessage = null; dragOverMailbox = null; }}
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
            <button class="page-btn" on:click={firstPage} disabled={!hasPrev} aria-label={tl('mail_firstPageAria')} title={tl('mail_firstPageAria')}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="5" y1="6" x2="5" y2="18"/><polyline points="15 18 9 12 15 6"/></svg>
            </button>
            <button class="page-btn" on:click={prevPage} disabled={!hasPrev} aria-label={tl('mail_prevPageAria')} title={tl('mail_prevPage')}>
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
              on:keydown={(e) => { if (e.key === 'Enter') confirmPageInput(); if (e.key === 'Escape') cancelPageInput(); }}
              on:blur={confirmPageInput}
            />
            <span class="page-status-of">/ {totalPages}</span>
          {:else if !searching}
            <button class="page-status page-status-btn" on:click={activatePageInput} title={tl('mail_jumpToPageAria')}>
              {tl('mail_pageStatus', { start: pageStart, end: pageEnd, total: totalMessages })}
            </button>
          {:else}
            <span class="page-status">{currentPage}</span>
          {/if}
          <div class="page-btn-group">
            <button class="page-btn" on:click={nextPage} disabled={!hasNext} aria-label={tl('mail_nextPageAria')} title={tl('mail_nextPage')}>
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>
            </button>
            {#if !searching}
              <button class="page-btn" on:click={lastPage} disabled={!hasNext} aria-label={tl('mail_lastPageAria')} title={tl('mail_lastPageAria')}>
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
      on:mousedown={(e) => startResize('list', e)}
      on:keydown={(e) => resizeWithKeyboard('list', e)}
    ></div>
    {/if}

    <!-- ── Message detail ────────────────────────────────────────────────────── -->
    <section
      class="message-detail"
      class:standalone
      class:is-open={!standalone && usesDetailOverlay && (selectedMessages.length > 0 || !!selectedMessage) && detailOverlayVisible}
      bind:this={detailEl}
      tabindex="-1"
      aria-label={tl('nav_mail')}
    >
      {#if !standalone && selectedMessages.length > 0}
        <div class="bulk-selection-header">
          {#if usesDetailOverlay}
            <button class="detail-close-btn" aria-label={tl('mail_back')} on:click={clearMarkedSelection}>←</button>
          {/if}
          <h2>{tl('mail_selectedCount', { n: selectedMessages.length })}</h2>
          <button class="bulk-clear-btn" on:click={clearMarkedSelection} disabled={bulkActionBusy}>
            {tl('mail_clearSelection')}
          </button>
        </div>
        <div class="bulk-selection-body" aria-busy={bulkActionBusy}>
          {#if bulkActionError}<p class="bulk-action-error">{bulkActionError}</p>{/if}
          <div class="bulk-actions">
            <button on:click={() => applyBulkFlags('\\Seen', true)} disabled={bulkActionBusy}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><polyline points="2 4 12 13 22 4"/></svg>
              {tl('mail_markRead')}
            </button>
            <button on:click={() => applyBulkFlags('\\Seen', false)} disabled={bulkActionBusy}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 12c-2.5 4-5.8 6-10 6s-7.5-2-10-6c2.5-4 5.8-6 10-6s7.5 2 10 6Z"/><circle cx="12" cy="12" r="3"/></svg>
              {tl('mail_markUnread')}
            </button>
            <button on:click={() => applyBulkFlags('\\Flagged', true)} disabled={bulkActionBusy}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 15.1 8.5 22 9.3 17 14.1 18.3 21 12 17.6 5.7 21 7 14.1 2 9.3 8.9 8.5 12 2"/></svg>
              {tl('mail_flag')}
            </button>
            <button on:click={() => applyBulkFlags('\\Flagged', false)} disabled={bulkActionBusy}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="12 2 15.1 8.5 22 9.3 17 14.1 18.3 21 12 17.6 5.7 21 7 14.1 2 9.3 8.9 8.5 12 2"/><line x1="3" y1="3" x2="21" y2="21"/></svg>
              {tl('mail_unflag')}
            </button>
            {#if canBulkArchive}
              <button class="bulk-section-start" on:click={() => applyBulkMove(msg => archiveMailboxFor(msg))} disabled={bulkActionBusy}>
                {@render folderIcon('archive')}
                {tl('mail_archive')}
              </button>
            {/if}
            {#if bulkMoveDestinations.length > 0}
              <button class="bulk-move-toggle" class:bulk-section-start={!canBulkArchive} on:click={() => bulkMoveOpen = !bulkMoveOpen} disabled={bulkActionBusy}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                {tl('mail_moveToFolder')}
                <span class="move-caret">{bulkMoveOpen ? '▴' : '▾'}</span>
              </button>
            {/if}
            <button class="bulk-delete-btn bulk-section-start" on:click={applyBulkDelete} disabled={bulkActionBusy}>
              <UiIcon name="trash" size={14} strokeWidth={2.2} />
              {tl('mail_delete')}
            </button>
          </div>
          {#if bulkMoveOpen && bulkMoveDestinations.length > 0}
            <div class="bulk-destinations" aria-label={tl('mail_moveToFolder')}>
              {#each bulkMoveDestinations as mailbox}
                <button on:click={() => applyBulkMove(() => mailbox.name)} disabled={bulkActionBusy}>
                  {@render folderIcon(folderKind(mailbox.name))}
                  <span>{FOLDER_NAMES[folderKind(mailbox.name)] || mailbox.name}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {:else if selectedMessage}
        <div class="detail-header">
          <div class="detail-subject-row" class:stacked={detailHeaderStacked} use:observeDetailHeader>
            <div class="detail-subject-line" bind:this={detailSubjectLineEl}>
              {#if !standalone && usesDetailOverlay}<button class="detail-close-btn" aria-label={tl('mail_back')} on:click={returnToListFromReader}>←</button>{/if}
              <span class="detail-subject">{selectedMessage.subject || tl('mail_noSubject')}</span>
            </div>
            <div class="detail-controls" bind:this={detailControlsEl}>
              <button
                class="star-btn"
                class:flagged={selectedMessage.is_flagged}
                on:click={() => setFlag(selectedMessage!, '\\Flagged', !selectedMessage!.is_flagged)}
                aria-label={selectedMessage.is_flagged ? tl('mail_unflag') : tl('mail_flag')}
                title={selectedMessage.is_flagged ? tl('mail_unflag') : tl('mail_flag')}
              >★</button>
              <button class="inline-action-btn" on:click={() => openCompose(selectedMessage!)} aria-label={tl('mail_reply')} title={tl('mail_reply')}>
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>
              </button>
              <div class="actions-wrapper" bind:this={actionsEl}>
                <button class="actions-toggle" on:click={toggleActions} aria-label={tl('mail_messageActions')}>{#if usesDetailOverlay}⋮{:else}{tl('mail_actions')} <span class="actions-caret">▾</span>{/if}</button>
                {#if actionsOpen}
                  <div class="actions-menu" role="menu">
                  <button role="menuitem" on:click={() => { actionsOpen = false; openComposeReplyAll(selectedMessage!); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="8 14 3 9 8 4"/><polyline points="14 14 9 9 14 4"/><path d="M21 20v-7a4 4 0 0 0-4-4H3"/></svg>
                    {tl('mail_replyAll')}
                  </button>
                  <button role="menuitem" on:click={() => { actionsOpen = false; openComposeForward(selectedMessage!); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 14 20 9 15 4"/><path d="M4 20v-7a4 4 0 0 1 4-4h12"/></svg>
                    {tl('mail_forward')}
                  </button>
                  <div class="menu-separator"></div>
                  <button role="menuitem" on:click={() => { actionsOpen = false; setFlag(selectedMessage!, '\\Seen', !selectedMessage!.is_read); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><polyline points="2 4 12 13 22 4"/></svg>
                    {selectedMessage.is_read ? tl('mail_markUnread') : tl('mail_markRead')}
                  </button>
                  {#if canToggleBodyView}
                    {#if !selectedMessage.is_encrypted && emailOrigins.length > 0}
                      <button role="menuitem" on:click={() => { actionsOpen = false; openOriginsForEmail(); }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>{tl('mail_imageSourcesEmail')}</button>
                    {/if}

                    <div class="menu-separator"></div>
                    <button role="menuitem" class:menu-item-active={viewMode === 'html'} on:click={() => { viewMode = 'html'; actionsOpen = false; }}>{tl('mail_viewAsHtml')}</button>
                    <button role="menuitem" class:menu-item-active={viewMode === 'text'} on:click={() => { viewMode = 'text'; actionsOpen = false; }}>{tl('mail_viewAsText')}</button>
                  {/if}
                  <div class="menu-separator"></div>
                  {#if archiveMailboxFor(selectedMessage)}
                    <button role="menuitem" on:click={() => { actionsOpen = false; archiveMessage(selectedMessage!); }}>
                      {@render folderIcon('archive')}
                      {tl('mail_archive')}
                    </button>
                  {/if}
                  <button role="menuitem" class="move-to-folder-btn" on:click|stopPropagation={() => moveSubmenuOpen = !moveSubmenuOpen}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>
                    {tl('mail_moveToFolder')}
                    <span class="move-caret">{moveSubmenuOpen ? '▴' : '▾'}</span>
                  </button>
                  {#if moveSubmenuOpen}
                    <div class="move-submenu" role="menu">
                      {#each (mailboxesByAccount[selectedMessage.account_id] || []).filter(mb => mb.subscribed && mb.name !== selectedMessage.mailbox) as mb}
                        <button role="menuitem" on:click={() => { actionsOpen = false; moveSubmenuOpen = false; moveMessage(selectedMessage!, mb.name); }}>
                          {@render folderIcon(folderKind(mb.name))}
                          <span>{FOLDER_NAMES[folderKind(mb.name)] || mb.name}</span>
                        </button>
                      {/each}
                    </div>
                  {/if}
                  <div class="menu-separator"></div>
                  <button role="menuitem" class="menu-item-danger" on:click={() => { actionsOpen = false; deleteMessage(selectedMessage!); }}>
                    <UiIcon name="trash" size={14} strokeWidth={2.2} />
                    {tl('mail_delete')}
                  </button>
                  </div>
                {/if}
              </div>
              {#if !standalone}
                <button class="inline-action-btn" on:click={() => openPopout(selectedMessage!)} aria-label={tl('mail_popout')} title={tl('mail_popout')}>
                  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v7a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h7"/></svg>
                </button>
              {/if}
            </div>
          </div>
        </div>
        {#if messageLoading}
          <div class="loading">{tl('mail_loading')}</div>
        {:else}
          <div class="detail-meta">
            <div class="meta-primary-row">
              <button type="button" class="meta-primary" aria-expanded={metaExpanded}
                on:click={() => metaExpanded = !metaExpanded}>
                <span class="meta-label">{primaryMetaLabel}:</span>
                <span class="meta-value">{primaryMetaValue}</span>
                <span class="meta-toggle" aria-hidden="true">{metaExpanded ? '▲' : '▼'}</span>
              </button>
              {#if pgpIconBadges.length > 0}
                <span class="pgp-security-icons">
                  {#each pgpIconBadges as badge}
                    <span
                      class="pgp-security-icon pgp-security-icon-{badge}"
                      title={pgpBadgeLabel(badge)}
                      aria-label={pgpBadgeLabel(badge)}
                      role="img"
                    >
                      {#if badge === 'encrypted'}
                        <UiIcon name="lock" size={16} />
                      {:else if badge === 'signed'}
                        <UiIcon name="signature" size={17} />
                      {:else}
                        <UiIcon name="verified" size={16} />
                      {/if}
                    </span>
                  {/each}
                </span>
              {/if}
              {#if threadMessages.length > 1}
                <button
                  class="inline-action-btn thread-trigger"
                  on:click={() => threadDialog?.showModal()}
                  title={tl('mail_threads')}
                  aria-label={tl('mail_threads')}
                >
                  <UiIcon name="tree" size={16} strokeWidth={2.2} />
                </button>
              {/if}
            </div>
            {#if metaExpanded}
              {#if isSentFolder}<div><strong>{tl('mail_metaFrom')}:</strong> {selectedMessage.from || ''}</div>{/if}
              {#if !isSentFolder}<div><strong>{tl('mail_metaTo')}:</strong> {selectedMessage.to.join(', ')}</div>{/if}
              {#if selectedMessage.cc.length > 0}
                <div><strong>{tl('mail_metaCc')}:</strong> {selectedMessage.cc.join(', ')}</div>
              {/if}
              <div><strong>{tl('mail_metaDate')}:</strong> {selectedMessage.sent_at ? new Date(selectedMessage.sent_at * 1000).toLocaleString() : ''}</div>
            {/if}
          </div>
          {#if visibleServerAttachments.length > 0}
            <div class="attachments">
              <button class="attachments-toggle" on:click={() => attachmentsExpanded = !attachmentsExpanded}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>
                {visibleServerAttachments.length === 1 ? tl('mail_attachment1') : tl('mail_attachmentsN', { n: visibleServerAttachments.length })}
                <span class="attachments-caret">{attachmentsExpanded ? '▲' : '▼'}</span>
              </button>
              {#if attachmentsExpanded}
                <div class="attachments-list">
                  {#each visibleServerAttachments as att}
                    <div class="pgp-attachment-row">
                      <a
                        class="compose-attachment-chip attachment-chip-dl"
                        href="/api/email/attachment?account_id={encodeURIComponent(selectedMessage.account_id)}&mailbox={encodeURIComponent(selectedMessage.mailbox)}&uid={selectedMessage.uid}&filename={encodeURIComponent(att.filename)}"
                        download={canPreviewAttachment(att) ? undefined : att.filename}
                        target={canPreviewAttachment(att) ? '_blank' : undefined}
                        rel={canPreviewAttachment(att) ? 'noopener' : undefined}
                      >
                        <span class="attachment-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg></span>
                        <span class="attachment-name" title={att.filename}>{att.filename}</span>
                        <span class="attachment-size">{humanSize(att.size)}</span>
                      </a>
                      {#if pgpImportablePublicKeyAttachments.has(att.filename)}
                        <button
                          class="pgp-import-public-btn"
                          on:click={() => importMessagePublicKey(att)}
                          disabled={pgpPublicKeyImporting === att.filename}
                          aria-busy={pgpPublicKeyImporting === att.filename}
                          title={tl('pgp_importPublicKeyButton')}
                          aria-label={tl('pgp_importPublicKeyButton')}
                        >
                          <UiIcon name="import" />
                        </button>
                      {/if}
                      {#if isIcsAttachment(att)}
                        <button
                          class="pgp-import-public-btn"
                          on:click={() => openIcsImport(att)}
                          disabled={icsImportingAttachment === att.filename}
                          aria-busy={icsImportingAttachment === att.filename}
                          title={tl('cal_import')}
                          aria-label={tl('cal_import')}
                        >
                          <UiIcon name="import" />
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
          {#if selectedMessage.is_encrypted && pgpAttachments.length > 0}
            <div class="attachments">
              <button class="attachments-toggle" on:click={() => attachmentsExpanded = !attachmentsExpanded}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>
                {pgpAttachments.length === 1 ? tl('mail_attachment1') : tl('mail_attachmentsN', { n: pgpAttachments.length })}
                <span class="attachments-caret">{attachmentsExpanded ? '▲' : '▼'}</span>
              </button>
              {#if attachmentsExpanded}
                <div class="attachments-list">
                  {#each pgpAttachments as att}
                    <div class="pgp-attachment-row">
                      <a
                        class="compose-attachment-chip attachment-chip-dl"
                        href={att.url}
                        download={canPreviewAttachment(att) ? undefined : att.filename}
                        target={canPreviewAttachment(att) ? '_blank' : undefined}
                        rel={canPreviewAttachment(att) ? 'noopener' : undefined}
                      >
                        <span class="attachment-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg></span>
                        <span class="attachment-name" title={att.filename}>{att.filename}</span>
                        <span class="attachment-size">{humanSize(att.size)}</span>
                      </a>
                      {#if pgpImportablePublicKeyAttachments.has(att.filename)}
                        <button
                          class="pgp-import-public-btn"
                          on:click={() => importMessagePublicKey(att, att.data)}
                          disabled={pgpPublicKeyImporting === att.filename}
                          aria-busy={pgpPublicKeyImporting === att.filename}
                          title={tl('pgp_importPublicKeyButton')}
                          aria-label={tl('pgp_importPublicKeyButton')}
                        >
                          <UiIcon name="import" />
                        </button>
                      {/if}
                      {#if isIcsAttachment(att)}
                        <button
                          class="pgp-import-public-btn"
                          on:click={() => openIcsImport(att, att.data)}
                          disabled={icsImportingAttachment === att.filename}
                          aria-busy={icsImportingAttachment === att.filename}
                          title={tl('cal_import')}
                          aria-label={tl('cal_import')}
                        >
                          <UiIcon name="import" />
                        </button>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
          <div class="detail-body" bind:this={detailBodyEl}>
            {#if pgpTextBadges.length > 0}
              <div class="pgp-badges">
                {#each pgpTextBadges as badge}
                  <span class="pgp-badge pgp-badge-{badge}">{pgpBadgeLabel(badge)}</span>
                {/each}
              </div>
            {/if}
            {#if pgpPublicKeyImportError}<p class="pgp-error">{pgpPublicKeyImportError}</p>{/if}
            {#if pgpPublicKeyImportSuccess}<p class="pgp-success">{pgpPublicKeyImportSuccess}</p>{/if}
            {#if selectedMessage.is_encrypted}
              <!-- PGP encrypted message -->
              {#if viewMode === 'html' && encryptedHtmlHasValue}
                <!-- Decrypted HTML: sandboxed iframe WITHOUT allow-same-origin -->
                {#key iframeKey}
                  <iframe
                    bind:this={iframeEl}
                    on:load={fitIframeToContent}
                    sandbox="allow-popups"
                    srcdoc={pgpDecryptedHtml}
                    title={tl('nav_mail')}
                    class="html-body"
                  ></iframe>
                {/key}
              {:else if pgpDecryptedText}
                <pre class="body-text">{#each bodyTextSegments as seg}{#if 'href' in seg}<a href={seg.href} target="_blank" rel="noopener noreferrer">{seg.text}</a>{:else}{seg.text}{/if}{/each}</pre>
              {:else if pgpDecryptError}
                <p class="pgp-error">{pgpDecryptError}</p>
              {:else if pgpDecrypting}
                <p aria-busy="true">{tl('pgp_decrypting')}</p>
              {:else}
                <div class="pgp-unlock-prompt">
                  <p>{tl('pgp_unlockToRead')}</p>
                  {#if pgpUnlockError}<p class="pgp-error">{pgpUnlockError}</p>{/if}
                  {#if pgpKeyUnlocked}
                    <button class="pgp-decrypt-btn" on:click={() => pgpDecrypt(selectedMessage!)} disabled={pgpDecrypting}>
                      {pgpDecrypting ? tl('pgp_decrypting') : tl('pgp_decryptButton')}
                    </button>
                  {:else}
                    <button class="pgp-decrypt-btn" on:click={() => pgpUnlock()} disabled={pgpUnlocking} aria-busy={pgpUnlocking}>
                      {pgpUnlocking ? tl('pgp_unlocking') : tl('pgp_unlockVaultButton')}
                    </button>
                  {/if}
                </div>
              {/if}
            {:else if viewMode === 'html' && htmlHasValue}
              {#if blockedOrigins.length > 0 && !imagesNoticeDismissed}
                <div class="images-notice">
                  <span>{tl('mail_imagesBlocked')}</span>
                  {#each blockedOrigins as origin}
                    <span class="origin-split-btn">
                      <button class="origin-split-allow" on:click={() => allowOrigin(origin)}>{tl('mail_allowOrigin', { origin })}</button>
                      <button class="origin-split-block" on:click={() => blockOriginSilently(origin)} title={tl('mail_blockSilently')}><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="4.93" y1="4.93" x2="19.07" y2="19.07"/></svg></button>
                    </span>
                  {/each}
                  <button class="dismiss-notice-btn" on:click={() => imagesNoticeDismissed = true} aria-label={tl('mail_dismiss')}>×</button>
                </div>
              {/if}
              {#key iframeKey}
                <iframe
                  bind:this={iframeEl}
                  on:load={fitIframeToContent}
                  scrolling={usesDetailOverlay ? 'no' : undefined}
                  sandbox="allow-popups allow-same-origin"
                  srcdoc={renderedHtml}
                  title={tl('nav_mail')}
                  class="html-body"
                ></iframe>
              {/key}
            {:else if selectedMessage.body_text !== null}
              <pre class="body-text">{#each bodyTextSegments as seg}{#if 'href' in seg}<a href={seg.href} target="_blank" rel="noopener noreferrer">{seg.text}</a>{:else}{seg.text}{/if}{/each}</pre>
            {:else}
              <div class="loading">{tl('mail_loadingBody')}</div>
            {/if}
          </div>
        {/if}
      {:else}
        <div class="empty">{standalone && standaloneUnavailable ? tl('mail_messageUnavailable') : tl('mail_selectMessage')}</div>
      {/if}
    </section>
  </div>
</div>

<ImportDialog
  bind:this={importDialog}
  {calendars}
  accounts={calendarAccounts}
  {locale}
  onimported={() => { void loadCalendarData(); }}
/>

<!-- ── Conversation threads dialog ───────────────────────────────────────── -->
<dialog
  bind:this={threadDialog}
  class="thread-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) threadDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); threadDialog?.close(); }}
>
  <article>
    <header>
      <h3>{tl('mail_threadsHeading')}</h3>
      <button class="close-btn" aria-label={tl('mail_close')} on:click={() => threadDialog?.close()}></button>
    </header>
    <ul class="thread-tree" aria-label={tl('mail_threadsHeading')}>
      {@render threadBranch(threadTree)}
    </ul>
  </article>
</dialog>

{#if !standalone}
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
      <!-- Step 1: name + email only -->
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
      <!-- Step 2: password + advanced -->
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
{/if}

<!-- ── Image origins trigger button ───────────────────────────────────────── -->
{#if !standalone}
<button
  class="origins-trigger"
  on:click={openOriginsGlobal}
  title={tl('mail_manageImageSources')}
  aria-label={tl('mail_manageImageSources')}
>
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>
  {#if imageOrigins.length + blacklistOrigins.length > 0}
    <span class="origins-badge">{imageOrigins.length + blacklistOrigins.length}</span>
  {/if}
</button>
{/if}

<!-- ── Image origins dialog ────────────────────────────────────────────────── -->
<dialog
  bind:this={originsDialog}
  class="origins-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) originsDialog?.close(); }}
>
  <article>
    <header>
      <span>{originsEmailOnly ? tl('mail_imageSourcesThisEmail') : tl('mail_imageSources')}</span>
      <button class="close-btn" aria-label={tl('mail_close')} on:click={() => originsDialog?.close()}></button>
    </header>
    <div class="origins-filter-wrap">
      <input type="search" class="origins-filter" placeholder={tl('mail_originsFilter')} bind:value={originsFilter} />
    </div>
    <div class="origins-sections">
      {#if originsEmailOnly}
        <section class="origins-section">
          <h6>{tl('mail_originsBlocked')}</h6>
          {#if blockedOrigins.filter(o => !originsFilter || o.includes(originsFilter)).length === 0}
            <p class="origins-empty">{originsFilter ? tl('mail_originsNoMatches') : tl('mail_originsNoneBlockedEmail')}</p>
          {:else}
            <ul class="origins-list">
              {#each blockedOrigins.filter(o => !originsFilter || o.includes(originsFilter)) as origin}
                <li>
                  <span>{origin}</span>
                  <button class="allow-btn" on:click={() => allowOrigin(origin)}>{tl('mail_originsAllow')}</button>
                  <button class="icon-btn delete-btn" on:click={() => blockOriginSilently(origin)} title={tl('mail_blockSilently')}>×</button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/if}
      <section class="origins-section">
        <h6>{tl('mail_originsAllowed')}</h6>
        {#if (originsEmailOnly ? allowedEmailOrigins : imageOrigins).filter(o => !originsFilter || o.includes(originsFilter)).length === 0}
          <p class="origins-empty">{originsFilter ? tl('mail_originsNoMatches') : tl('mail_originsNoneYet')}</p>
        {:else}
          <ul class="origins-list">
            {#each (originsEmailOnly ? allowedEmailOrigins : imageOrigins).filter(o => !originsFilter || o.includes(originsFilter)) as origin}
              <li>
                <span>{origin}</span>
                <button class="icon-btn delete-btn" on:click={() => revokeOrigin(origin)} title={tl('mail_removeAttachment')}>×</button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
      <section class="origins-section">
        <h6>{tl('mail_originsIgnored')}</h6>
        {#if (originsEmailOnly ? silentEmailOrigins : blacklistOrigins).filter(o => !originsFilter || o.includes(originsFilter)).length === 0}
          <p class="origins-empty">{originsFilter ? tl('mail_originsNoMatches') : tl('mail_originsNoneYet')}</p>
        {:else}
          <ul class="origins-list">
            {#each (originsEmailOnly ? silentEmailOrigins : blacklistOrigins).filter(o => !originsFilter || o.includes(originsFilter)) as origin}
              <li>
                <span>{origin}</span>
                <button class="allow-btn" on:click={() => promoteFromBlacklist(origin)} title={tl('mail_originsAllow')}>{tl('mail_originsAllow')}</button>
                <button class="icon-btn delete-btn" on:click={() => unblockOrigin(origin)} title={tl('mail_removeAttachment')}>×</button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    </div>
  </article>
</dialog>

<!-- ── Compose dialog ─────────────────────────────────────────────────────── -->
<dialog
  bind:this={composeDialog}
  class="compose-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) composeDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); if (filePickerOpen) { filePickerOpen = false; } else { composeDialog?.close(); } }}
>
  <article>
    <header class="compose-header">
      <span class="compose-title">{composeMode === 'forward' ? tl('mail_forward') : composeMode === 'replyAll' ? tl('mail_replyAll') : composeMode === 'reply' ? tl('mail_reply') : tl('mail_newMessage')}</span>
      <button class="close-btn" aria-label={tl('mail_close')} on:click={() => composeDialog?.close()}></button>
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
        <input class="compose-input" bind:value={composeTo} placeholder={tl('mail_composeToPlaceholder')} />
        <div class="compose-security-controls">
          <button
            type="button"
            class="compose-security-toggle"
            class:active={composeEncrypt}
            disabled={!composeCanEncrypt}
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
            disabled={!composeCanSign || pgpUnlocking}
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
          <input class="compose-input" bind:value={composeCc} placeholder={tl('mail_composeCcPlaceholder')} />
        </div>
        <div class="compose-row">
          <span class="compose-label">{tl('mail_composeBcc')}</span>
          <input class="compose-input" bind:value={composeBcc} placeholder={tl('mail_composeBccPlaceholder')} />
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
          <button type="button" class="secondary outline" on:click={() => composeDialog?.close()}>{tl('mail_discard')}</button>
          <button type="submit" class="primary" disabled={composeSending}>
            {composeSending ? tl('mail_sending') : tl('mail_send')}
          </button>
        </div>
      </footer>
    </form>
  </article>
</dialog>

{#if !standalone}
<!-- ── Virtual folder form ─────────────────────────────────────────────────── -->
<dialog
  bind:this={folderDialog}
  on:click={(e) => { if (e.target === e.currentTarget) folderDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); folderDialog?.close(); }}
>
  <article>
    <header>
      <h3>{editingFolder ? tl('mail_editVirtualFolder') : tl('mail_createVirtualFolder')}</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label={tl('mail_close')} on:click={() => folderDialog?.close()}></button>
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
        <button type="button" class="secondary outline" on:click={() => folderDialog?.close()}>{tl('mail_cancel')}</button>
        <button type="submit" class="primary" disabled={folderSaving}>{editingFolder ? tl('mail_save') : tl('mail_create')}</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- ── FAB ────────────────────────────────────────────────────────────────── -->
<button class="fab" on:click={() => openCompose()} aria-label={tl('mail_newMessage')}>
  <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
  </svg>
</button>
{/if}

<style>
  .folder-mailbox-label {
    font-size: 0.875rem;
    font-weight: 500;
    margin: 1rem 0 0.25rem;
  }
  .folder-mailbox-list {
    max-height: 280px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    margin-bottom: 0.5rem;
    border: 1px solid var(--pico-form-element-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.75rem;
  }
  .folder-account-group h4 {
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--pico-muted-color);
    margin: 0 0 0.25rem;
  }
  .folder-account-group label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    cursor: pointer;
    margin-bottom: 0.15rem;
  }
  .folder-hint {
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    margin: 0.25rem 0 0.75rem;
  }
  .folder-filters {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-bottom: 1rem;
  }
  .folder-filters label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    cursor: pointer;
  }
  .folder-account-group label input[type="checkbox"] {
    margin: 0;
  }

  .mail-page {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .sidebar-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.2);
    z-index: 60;
    border: none;
    padding: 0;
    cursor: pointer;
  }

  .folder-toggle {
    display: none;
    background: none;
    border: none;
    padding: 0.2rem;
    cursor: pointer;
    color: inherit;
    flex-shrink: 0;
  }

  .list-header-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .list-header-row h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    flex: 1;
  }

  .mail-layout {
    display: grid;
    overflow: hidden;
  }

  .mail-layout.standalone {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .resize-handle {
    cursor: col-resize;
    background: var(--pico-muted-border-color, #ddd);
    border: none;
    border-radius: 0;
    margin: 0;
    padding: 0;
    min-width: 3px;
    transition: background 0.12s;
    position: relative;
    z-index: 1;
    width: 3px;
  }

  .resize-handle:hover,
  .resize-handle:focus-visible {
    background: var(--pico-primary, #0172ad);
  }

  /* Sidebar */
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

  .folder-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder-icon {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    color: var(--pico-muted-color, #888);
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0 0.2rem;
    opacity: 0.4;
    font-size: 0.8rem;
    color: inherit;
  }

  .icon-btn:hover {
    opacity: 1;
  }

  .delete-btn:hover {
    color: var(--pico-del-color, #c0392b);
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
    color: var(--pico-primary, #0172ad);
    font-size: 0.8rem;
    text-align: left;
    padding: 0;
  }

  /* Message list */
  .message-list {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
    margin: 0;
    position: relative;
    z-index: 2;
  }

  .message-list-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .message-list-header {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
    position: relative;
  }

  .search-input {
    width: 100%;
    margin: 0;
    padding: 0.35rem 0.6rem;
    font-size: 0.85rem;
    height: auto;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: var(--pico-border-radius, 4px);
    background: var(--pico-form-element-background-color, transparent);
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    width: 100%;
  }

  .search-row .search-input {
    flex: 1;
    min-width: 0;
    width: auto;
  }

  .search-filter-wrap {
    flex-shrink: 0;
  }

  .filter-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: none;
    border: 1px solid transparent;
    border-radius: var(--pico-border-radius, 4px);
    cursor: pointer;
    color: var(--pico-muted-color, #888);
    transition: color 0.15s, border-color 0.15s;
    flex-shrink: 0;
  }

  .filter-icon-btn:hover {
    color: var(--pico-color, inherit);
    border-color: var(--pico-muted-border-color, #ddd);
  }

  .filter-icon-btn.active {
    color: var(--pico-primary, #1565c0);
    border-color: var(--pico-primary, #1565c0);
  }

  .filter-popover {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 100;
    background: var(--pico-background-color, #fff);
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: var(--pico-border-radius, 6px);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
    padding: 0.5rem;
    min-width: 300px;
    max-width: 380px;
    overflow-x: hidden;
  }

  .filter-popover-date {
    min-width: 180px;
    max-width: 220px;
  }

  .filter-popover-input {
    width: 100%;
    margin: 0 0 0.4rem;
    padding: 0.3rem 0.5rem;
    font-size: 0.82rem;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: var(--pico-border-radius, 4px);
    background: var(--pico-form-element-background-color, transparent);
    box-sizing: border-box;
  }

  .filter-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    margin-bottom: 0.4rem;
  }

  .filter-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    padding: 0.1rem 0.35rem;
    border-radius: 999px;
    background: var(--pico-primary-background, #e3f0fb);
    color: var(--pico-primary, #1565c0);
    font-size: 0.75rem;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-remove {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    line-height: 1;
    font-size: 0.9rem;
    color: inherit;
    opacity: 0.6;
    flex-shrink: 0;
  }

  .chip-remove:hover {
    opacity: 1;
  }

  .filter-suggestions {
    max-height: 300px;
    overflow-y: auto;
  }

  .filter-suggestion-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.25rem 0.3rem;
    border-radius: var(--pico-border-radius, 4px);
    cursor: pointer;
    font-size: 0.82rem;
    margin: 0;
  }

  .filter-suggestion-row:hover {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .suggestion-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .suggestion-email {
    color: var(--pico-muted-color, #888);
    font-size: 0.75rem;
    flex-shrink: 0;
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .filter-add-custom {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.25rem 0.3rem;
    font-size: 0.82rem;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--pico-primary, #1565c0);
    border-radius: var(--pico-border-radius, 4px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .filter-add-custom:hover {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .filter-date-label {
    display: flex;
    flex-direction: column;
    font-size: 0.8rem;
    gap: 0.15rem;
    margin-bottom: 0.5rem;
    color: var(--pico-muted-color, #666);
  }

  .filter-date-label input[type='date'] {
    font-size: 0.82rem;
    margin: 0;
    height: auto;
  }

  .filter-clear-btn {
    font-size: 0.78rem;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--pico-muted-color, #888);
    padding: 0;
    text-decoration: underline;
  }

  .count {
    font-size: 0.75rem;
    color: var(--pico-muted-color, #888);
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

  .flag-star {
    font-size: 0.75rem;
  }

  .attach-icon {
    display: inline-flex;
    align-items: center;
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

  /* Message detail */
  .message-detail {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    margin-bottom: 0;
  }

  .message-detail:focus {
    outline: none;
  }

  .message-detail.standalone {
    display: flex;
    flex: 1;
    min-height: 0;
    background: var(--pico-card-background-color, white);
  }

  .bulk-selection-header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
  }

  .bulk-selection-header h2 {
    flex: 1;
    margin: 0;
    font-size: 1.05rem;
  }

  .bulk-clear-btn {
    padding: 0.3rem 0.65rem;
    margin: 0;
    width: auto;
    font-size: 0.82rem;
    background: none;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    color: inherit;
  }

  .bulk-selection-body {
    padding: 1rem;
    overflow-y: auto;
  }

  .bulk-actions {
    width: min(250px, 100%);
    overflow: hidden;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: 6px;
    background: var(--pico-background-color, white);
  }

  .bulk-actions button,
  .bulk-destinations button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    margin: 0;
    padding: 0.45rem 0.9rem;
    border: none;
    border-radius: 0;
    background: none;
    color: inherit;
    text-align: left;
    font-size: 0.85rem;
    font-weight: normal;
    white-space: nowrap;
  }

  .bulk-actions button:hover:not(:disabled),
  .bulk-destinations button:hover:not(:disabled) {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .bulk-actions button:disabled,
  .bulk-destinations button:disabled {
    opacity: 0.55;
  }

  .bulk-actions .bulk-section-start {
    border-top: 1px solid var(--pico-muted-border-color, #eee);
  }

  .bulk-move-toggle .move-caret {
    margin-left: auto;
  }

  .bulk-delete-btn {
    color: var(--pico-del-color, #c0392b) !important;
  }

  .bulk-destinations {
    width: min(250px, 100%);
    margin-top: 0.65rem;
    overflow: hidden;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: 6px;
    background: var(--pico-card-background-color, #fafafa);
  }

  .bulk-action-error {
    color: var(--pico-del-color, #c0392b);
    margin: 0 0 0.75rem;
  }

  .detail-header {
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
  }

  .detail-subject-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .detail-controls {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-left: auto;
    flex-shrink: 0;
  }

  .detail-subject-line {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    min-width: 0;
  }

  .detail-subject-row.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: 0.35rem;
  }

  .detail-subject-row.stacked .detail-controls {
    order: 0;
    width: 100%;
    justify-content: flex-end;
  }

  .detail-subject-row.stacked .detail-subject-line {
    order: 1;
    width: 100%;
  }

  .detail-subject-row.stacked .detail-subject {
    white-space: normal;
    overflow-wrap: anywhere;
  }

  .detail-close-btn {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 1.2rem;
    line-height: 1;
    padding: 0.1rem 0.35rem;
    color: var(--pico-muted-color, #888);
    flex-shrink: 0;
  }

  .detail-close-btn:hover {
    color: var(--pico-color, #333);
  }

  .detail-subject {
    flex: 1;
    min-width: 0;
    font-size: 1.05rem;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
  }

  .actions-wrapper {
    position: relative;
    flex-shrink: 0;
  }

  .actions-toggle {
    background: none;
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 4px;
    cursor: pointer;
    padding: 0.25rem 0.65rem;
    font-size: 0.8rem;
    font-weight: 500;
    line-height: 1.4;
    color: inherit;
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    white-space: nowrap;
  }

  .actions-toggle:hover {
    background: var(--pico-muted-border-color, #f0f0f0);
  }

  .actions-caret {
    font-size: 0.7rem;
    opacity: 0.7;
  }

  .actions-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    background: var(--pico-background-color, white);
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0,0,0,.14);
    z-index: 200;
    min-width: 175px;
    overflow: hidden;
  }

  .inline-action-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.15rem 0.25rem;
    line-height: 1;
    color: var(--pico-muted-color, #bbb);
    flex-shrink: 0;
    transition: color 0.15s;
  }

  .inline-action-btn:hover {
    color: var(--pico-color, #333);
  }

  .star-btn {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 1.1rem;
    padding: 0.1rem 0.25rem;
    line-height: 1;
    color: var(--pico-muted-color, #bbb);
    flex-shrink: 0;
    transition: color 0.15s;
  }

  .star-btn.flagged {
    color: #f5a623;
  }

  .star-btn:hover {
    color: #f5a623;
  }

  .actions-menu button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    text-align: left;
    padding: 0.45rem 0.9rem;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 0.85rem;
    color: inherit;
    white-space: nowrap;
  }

  .actions-menu button:hover {
    background: var(--pico-muted-border-color, #f5f5f5);
  }

  .menu-separator {
    height: 1px;
    background: var(--pico-muted-border-color, #eee);
    margin: 0.2rem 0;
  }

  .menu-item-danger {
    color: var(--pico-del-color, #c0392b) !important;
  }

  .menu-item-active::before {
    content: '✓ ';
  }

  .move-to-folder-btn {
    justify-content: space-between !important;
  }

  .move-to-folder-btn > :first-child {
    margin-right: 0.25rem;
  }

  .move-caret {
    margin-left: auto;
    font-size: 0.75rem;
    color: var(--pico-muted-color, #888);
  }

  .move-submenu {
    max-height: 180px;
    overflow-y: auto;
    border-top: 1px solid var(--pico-muted-border-color, #eee);
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    background: var(--pico-card-background-color, #fafafa);
  }

  .move-submenu button {
    padding-left: 1.4rem !important;
    font-size: 0.82rem !important;
  }

  .sidebar-item.drag-target {
    outline: 1px dashed var(--pico-primary, #2563eb);
  }

  .sidebar-item.drag-over {
    background: var(--pico-primary-background, #eff6ff);
    outline: 1px solid var(--pico-primary, #2563eb);
  }

  .detail-meta {
    padding: 0.5rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    font-size: 0.82rem;
    color: var(--pico-muted-color, #555);
    flex-shrink: 0;
  }

  .detail-meta div {
    margin-bottom: 0.1rem;
  }

  .meta-primary-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .meta-primary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
    flex: 1;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
  }

  .meta-label {
    font-weight: 600;
    flex-shrink: 0;
  }

  .meta-value {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta-toggle {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 0.65rem;
    color: var(--pico-muted-color, #aaa);
    padding: 0.1rem 0.2rem;
    flex-shrink: 0;
    line-height: 1;
  }

  .pgp-security-icons {
    display: inline-flex;
    align-items: center;
    gap: 0.15rem;
    flex-shrink: 0;
  }

  .pgp-security-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.7rem;
    height: 1.7rem;
    border-radius: 0.3rem;
    color: var(--pico-primary);
    background: color-mix(in srgb, var(--pico-primary) 15%, transparent);
  }

  .pgp-security-icon-verified {
    color: var(--pico-ins-color);
    background: color-mix(in srgb, var(--pico-ins-color) 20%, transparent);
  }

  .thread-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .thread-trigger:hover {
    color: var(--pico-color, #333);
  }

  .meta-toggle:hover {
    color: var(--pico-color, inherit);
  }

  .attachments {
    padding: 0.4rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    flex-shrink: 0;
  }

  .attachments-toggle {
    background: none !important;
    border: none !important;
    box-shadow: none !important;
    cursor: pointer;
    padding: 0 !important;
    margin: 0 !important;
    font-size: 0.82rem !important;
    color: var(--pico-muted-color, #666) !important;
    display: inline-flex !important;
    align-items: center;
    gap: 0.3rem;
    width: auto !important;
    min-width: unset !important;
  }

  .attachments-toggle:hover {
    color: var(--pico-color, #333) !important;
  }

  .attachments-caret {
    font-size: 0.65rem;
    opacity: 0.7;
  }

  .attachments-list {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.4rem;
  }

  .attachment-chip-dl {
    text-decoration: none;
    color: inherit;
  }

  .attachment-chip-dl:hover {
    background: var(--pico-card-background-color);
    color: inherit;
  }

  .pgp-attachment-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .pgp-import-public-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    margin: 0;
    padding: 0.25rem;
    width: 1.7rem;
    height: 1.7rem;
    background: none;
    border: none;
    color: var(--pico-muted-color);
  }

  .pgp-import-public-btn:hover:not(:disabled) {
    background: none;
    color: var(--pico-color);
  }

  .pgp-import-public-btn:disabled {
    opacity: 0.45;
  }

  .detail-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
  }

  .body-text {
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 0.88rem;
    line-height: 1.6;
    margin: 0;
    padding: 0.5rem 0.75rem;
    background: var(--pico-card-background-color, #fafafa);
    border-radius: var(--pico-border-radius, 4px);
  }

  .view-toggle {
    display: flex;
    gap: 0;
    margin-bottom: 0.75rem;
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 4px;
    overflow: hidden;
    width: fit-content;
  }

  .view-toggle button {
    padding: 0.3rem 0.75rem;
    border: none;
    background: none;
    cursor: pointer;
    font-size: 0.8rem;
    color: inherit;
    border-right: 1px solid var(--pico-muted-border-color, #ccc);
  }

  .view-toggle button:last-child {
    border-right: none;
  }

  .view-toggle button.active {
    background: var(--pico-primary, #0172ad);
    color: white;
  }

  .images-notice {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    background: var(--pico-card-background-color, #f9f9f9);
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
    margin-bottom: 0.75rem;
    color: var(--pico-muted-color, #666);
  }

  .dismiss-notice-btn {
    background: none !important;
    border: none !important;
    box-shadow: none !important;
    cursor: pointer;
    padding: 0 0.1rem !important;
    margin: 0 0 0 auto !important;
    font-size: 1.1rem !important;
    line-height: 1;
    color: var(--pico-muted-color, #aaa) !important;
    width: auto !important;
    min-width: unset !important;
    display: inline-flex !important;
    align-items: center;
  }

  .dismiss-notice-btn:hover {
    color: var(--pico-color, #333) !important;
  }

  .allow-btn {
    padding: 0.2rem 0.6rem;
    font-size: 0.78rem;
    border: 1px solid var(--pico-primary, #0172ad);
    border-radius: 3px;
    background: none;
    color: var(--pico-primary, #0172ad);
    cursor: pointer;
  }

  .allow-btn:hover {
    background: var(--pico-primary, #0172ad);
    color: white;
  }

  .origin-split-btn {
    display: inline-flex;
    border: 1px solid var(--pico-primary, #0172ad);
    border-radius: 3px;
    overflow: hidden;
    flex-shrink: 0;
  }

  .origin-split-allow {
    padding: 0.2rem 0.6rem;
    font-size: 0.78rem;
    background: none;
    border: none;
    color: var(--pico-primary, #0172ad);
    cursor: pointer;
    white-space: nowrap;
  }

  .origin-split-allow:hover {
    background: var(--pico-primary, #0172ad);
    color: white;
  }

  .origin-split-block {
    padding: 0.2rem 0.4rem;
    background: none;
    border: none;
    border-left: 1px solid color-mix(in srgb, var(--pico-del-color, #c0392b) 40%, transparent);
    color: var(--pico-del-color, #c0392b);
    cursor: pointer;
    display: flex;
    align-items: center;
  }

  .origin-split-block:hover {
    background: var(--pico-del-color, #c0392b);
    color: white;
  }

  .html-body {
    width: 100%;
    border: none;
    flex: 1;
    min-height: 0;
  }

  .origins-trigger {
    position: fixed;
    bottom: 2.75rem;
    right: 6.5rem;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.35rem 0.6rem;
    background: var(--pico-background-color, white);
    border-radius: 6px;
    font-size: 0.78rem;
    cursor: pointer;
    z-index: 50;
    box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.25);
  }

  .origins-trigger:hover {
    color: var(--pico-color, #333);
    border-color: var(--pico-primary, #0172ad);
  }

  .origins-badge {
    font-size: 0.7rem;
  }

  .origins-dialog {
    width: 480px;
    max-width: 95vw;
  }

  .origins-dialog article {
    margin: 0;
  }

  .origins-dialog header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    font-weight: 500;
  }

  .origins-filter-wrap {
    padding: 0.6rem 1rem 0;
  }

  .origins-filter {
    width: 100%;
    margin: 0;
  }

  .origins-sections {
    display: flex;
    flex-direction: column;
    gap: 0;
    padding: 0 1rem 1rem;
  }

  .origins-section h6 {
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--pico-muted-color, #888);
    margin: 1rem 0 0.4rem;
  }

  .origins-empty {
    font-size: 0.82rem;
    color: var(--pico-muted-color, #aaa);
    margin: 0;
  }

  .origins-list {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .origins-list li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0;
    font-size: 0.84rem;
  }

  .origins-list li span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Shared */
  .loading,
  .empty {
    padding: 2rem 1rem;
    text-align: center;
    color: var(--pico-muted-color, #888);
    font-size: 0.9rem;
  }

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

  .compose-attachment-chip {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    background: var(--pico-card-sectioning-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: 1rem;
    padding: 0.15rem 0.5rem 0.15rem 0.4rem;
    font-size: 0.78rem;
    max-width: 18rem;
  }

  .attachment-icon {
    font-size: 0.8rem;
    line-height: 1;
  }

  .attachment-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 10rem;
  }

  .attachment-size {
    color: var(--pico-muted-color);
    flex-shrink: 0;
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

  /* FAB */
  .fab {
    position: fixed;
    bottom: 2rem;
    right: 2rem;
    z-index: 20;
    width: 3.5rem;
    height: 3.5rem;
    border-radius: 50%;
    border: none;
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.25);
    transition: transform 0.15s, box-shadow 0.15s;
  }

  .fab:hover {
    transform: scale(1.05);
    box-shadow: 0 0.35rem 1.25rem rgba(0,0,0,0.3);
  }

  /* Identity management */
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

  /* Responsive */
  @media (max-width: 1023px) {
    .message-detail {
      display: none;
    }

    .message-detail.is-open {
      display: flex;
      position: fixed;
      inset: 0;
      z-index: 101;
      background: var(--pico-card-background-color, white);
      overflow-y: auto;
      box-shadow: inset 0 3px 12px rgba(0,0,0,0.08);
    }

    .message-detail.standalone {
      display: flex;
      position: static;
      overflow-y: auto;
      box-shadow: none;
    }

    .message-detail.is-open .detail-header {
      background: var(--pico-card-sectioning-background-color);
      border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    }

    .detail-body {
      flex: none;
      overflow-y: visible;
    }

    .html-body {
      flex: none;
    }

    .detail-resize-handle {
      display: none;
    }

    .mail-layout {
      display: grid;
      flex: 1;
      min-height: 0;
    }

    .message-list {
      min-height: 0;
    }

    .fab {
      bottom: 1.5rem;
      right: 1.5rem;
    }

    .origins-trigger {
      bottom: 2.25rem;
      right: 5.5rem;
    }
  }

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

  @media (max-width: 639px) {
    .mail-layout {
      position: relative;
      display: flex;
    }

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

    .message-list {
      flex: 1;
    }

    .folder-toggle {
      display: flex;
      align-items: center;
    }
  }

  @media (min-width: 1024px) {
    .mail-layout {
      flex: 1;
      min-height: 0;
    }
  }

  .sync-spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--pico-muted-border-color, #ccc);
    border-top-color: var(--pico-primary, #0172ad);
    border-radius: 50%;
    flex-shrink: 0;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

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

  .pgp-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-bottom: 0.75rem;
  }

  .pgp-badge {
    font-size: 0.7rem;
    padding: 0.15rem 0.45rem;
    border-radius: 1rem;
    font-weight: 600;
    background: var(--pico-muted-border-color);
    color: var(--pico-color);
  }

  .pgp-badge-verification_failed,
  .pgp-badge-decrypt_failed { background: color-mix(in srgb, var(--pico-del-color) 15%, transparent); color: var(--pico-del-color); }

  .pgp-unlock-prompt {
    padding: 1.5rem;
    text-align: center;
  }

  .pgp-decrypt-btn {
    margin-top: 0.5rem;
  }

  .pgp-error {
    color: var(--pico-del-color);
    font-size: 0.875rem;
    margin: 0.25rem 0;
  }

  .pgp-success {
    color: var(--pico-ins-color);
    font-size: 0.875rem;
    margin: 0.25rem 0;
  }
</style>

<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { getUser } from '../lib/user.ts';
  import { getLocale, makeTl } from '../lib/i18n.ts';
  import { type NewMailNotification, showNewMailNotification } from '../lib/mail-notifications.ts';
  import SelectDropdown from './SelectDropdown.svelte';
  import UiIcon from './UiIcon.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import MailComposeDialog from './mail/MailComposeDialog.svelte';
  import MailAccountDialog from './mail/MailAccountDialog.svelte';
  import MailFolderDialog from './mail/MailFolderDialog.svelte';
  import MailSidebar from './mail/MailSidebar.svelte';
  import FolderIcon from './mail/FolderIcon.svelte';
  import MailMessageList from './mail/MailMessageList.svelte';
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
  import type { Account, Attachment, BodySegment, FolderKind, Identity, Mailbox, Message, SidebarAccountSection, SidebarMailbox, ThreadNode, ThreadTreeNode, VirtualFolder } from './mail/MailTypes.ts';
  import { attachmentDataUrl, bareMailbox, buildThreadTree, canPreviewAttachment, canPreviewContentType, computePopoverStyle, folderKind, FOLDER_PRIORITY, folderSortKey, formatDate, formatMessageDateTime, htmlAddsValue, humanSize, identityKey, insertDecryptedInlineImages, intrinsicWidth, isIcsAttachment, isPgpPublicKeyAttachment, isPgpSignatureAttachment, isPgpTransportAttachment, keyHasEmail, linkifySegments, messageKey, sameMessage, senderName } from './mail/mailUtils.ts';

  export let standalone = false;
  export let initialMessage: { accountId: string; mailbox: string; uid: number } | null = null;

  let locale = 'en';
  $: tl = makeTl(locale);

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
  let accountDialog: MailAccountDialog;
  let composeDialog: MailComposeDialog;
  let folderDialog: MailFolderDialog;
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
        const attachments = [new File([content], filename, { type: mimeType })];
        await tick();
        composeDialog?.open({ mode: 'new', subject, attachments });
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
      composeDialog?.open({ mode: 'reply', replyTo: msg });
    } else if (!e.shiftKey && e.key === 'a') {
      e.preventDefault();
      composeDialog?.open({ mode: 'replyAll', replyTo: msg });
    } else if (!e.shiftKey && e.key === 'f') {
      e.preventDefault();
      composeDialog?.open({ mode: 'forward', replyTo: msg });
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

  $: {
    const totalUnread = sidebarAccountSections.reduce((sum, s) => sum + s.unread_count, 0);
    document.title = totalUnread > 0 ? `(${totalUnread > 999 ? '999+' : totalUnread}) Caley - Mail` : 'Caley - Mail';
  }

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
    <MailSidebar
      {tl}
      {isPhone}
      bind:sidebarOpen
      {virtualFolders}
      {selectedFolderId}
      {folderUnreadCounts}
      {sidebarAccountSections}
      {selectedAccountId}
      {selectedMailbox}
      {syncingMailboxes}
      {FOLDER_NAMES}
      {sidebarWidth}
      bind:draggedMessage
      bind:dragOverMailbox
      onSelectVirtualFolder={selectVirtualFolder}
      onSelectMailbox={selectMailbox}
      onToggleCollapsed={toggleAccountCollapsed}
      onRemoveAccount={removeAccount}
      onMoveMessage={moveMessage}
      onStartResize={startResize}
      onResizeWithKeyboard={resizeWithKeyboard}
      onEditFolder={(f) => folderDialog?.openEdit(f)}
      onDeleteFolder={(f) => folderDialog?.deleteFolder(f)}
      onOpenAddAccount={() => accountDialog?.openAdd()}
      onOpenEditAccount={(acc) => accountDialog?.openEdit(acc)}
      onOpenIdentities={(id) => accountDialog?.openIdentities(id)}
      onOpenCreateFolder={() => folderDialog?.openCreate()}
    />

    <!-- ── Message list ────────────────────────────────────────────────────── -->
    <MailMessageList
      {tl}
      {standalone}
      {isPhone}
      bind:sidebarOpen
      {selectedFolderId}
      {selectedMailbox}
      {selectedAccountId}
      {virtualFolders}
      {FOLDER_NAMES}
      {accounts}
      {messages}
      {totalMessages}
      {currentPage}
      {messagesLoading}
      {searching}
      bind:searchQuery
      {selectedMessage}
      bind:listCursor
      {markedMessageKeys}
      {selectedMessages}
      {filterAddrActive}
      {filterDateActive}
      bind:filterAddrOpen
      bind:filterDateOpen
      bind:filterAddrs
      bind:filterDateFrom
      bind:filterDateTo
      bind:addrFilterInput
      {contactSuggestions}
      {addrDbSuggestions}
      bind:addrFilterEl
      bind:addrPopoverEl
      {addrPopoverStyle}
      bind:dateFilterEl
      bind:datePopoverEl
      {datePopoverStyle}
      bind:draggedMessage
      bind:dragOverMailbox
      {hasPrev}
      {hasNext}
      {pageStart}
      {pageEnd}
      {totalPages}
      bind:pageInputActive
      bind:pageInputValue
      {listWidth}
      bind:listScrollEl
      bind:pageInputEl
      onSearchInput={onSearchInput}
      onToggleAddrFilter={toggleAddrFilter}
      onToggleDateFilter={toggleDateFilter}
      onAddrInput={onAddrInput}
      onRunSearch={runSearch}
      onMessageRowClick={handleMessageRowClick}
      {registerMessageRow}
      onFirstPage={firstPage}
      onPrevPage={prevPage}
      onNextPage={nextPage}
      onLastPage={lastPage}
      onActivatePageInput={activatePageInput}
      onConfirmPageInput={confirmPageInput}
      onCancelPageInput={cancelPageInput}
      onStartResize={startResize}
      onResizeWithKeyboard={resizeWithKeyboard}
      onDragStart={(msg, e) => { draggedMessage = msg; if (e.dataTransfer) e.dataTransfer.effectAllowed = "move"; }}
      onDragEnd={() => { draggedMessage = null; dragOverMailbox = null; }}
    />
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
                <FolderIcon kind="archive" />
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
                  <FolderIcon kind={folderKind(mailbox.name)} />
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
              <button class="inline-action-btn" on:click={() => composeDialog?.open({ mode: 'reply', replyTo: selectedMessage! })} aria-label={tl('mail_reply')} title={tl('mail_reply')}>
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>
              </button>
              <div class="actions-wrapper" bind:this={actionsEl}>
                <button class="actions-toggle" on:click={toggleActions} aria-label={tl('mail_messageActions')}>{#if usesDetailOverlay}⋮{:else}{tl('mail_actions')} <span class="actions-caret">▾</span>{/if}</button>
                {#if actionsOpen}
                  <div class="actions-menu" role="menu">
                  <button role="menuitem" on:click={() => { actionsOpen = false; composeDialog?.open({ mode: 'replyAll', replyTo: selectedMessage! }); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="8 14 3 9 8 4"/><polyline points="14 14 9 9 14 4"/><path d="M21 20v-7a4 4 0 0 0-4-4H3"/></svg>
                    {tl('mail_replyAll')}
                  </button>
                  <button role="menuitem" on:click={() => { actionsOpen = false; composeDialog?.open({ mode: 'forward', replyTo: selectedMessage! }); }}>
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
                      <FolderIcon kind="archive" />
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
                          <FolderIcon kind={folderKind(mb.name)} />
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
                    sandbox="allow-popups allow-popups-to-escape-sandbox"
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
                  sandbox="allow-popups allow-same-origin allow-popups-to-escape-sandbox"
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
<MailAccountDialog
  bind:this={accountDialog}
  {identities}
  {accounts}
  onAccountsChanged={async () => { await loadAccounts(); await loadIdentities(); for (const acc of accounts) await loadMailboxes(acc.account_id); }}
  onIdentitiesChanged={loadIdentities}
/>
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

{#if !standalone}
<!-- ── Compose dialog ─────────────────────────────────────────────────────── -->
<MailComposeDialog
  bind:this={composeDialog}
  {identities}
  {contactSuggestions}
  onEnsureContactSuggestions={loadContactSuggestions}
/>

<!-- ── Virtual folder form ─────────────────────────────────────────────────── -->
<MailFolderDialog
  bind:this={folderDialog}
  {accounts}
  {mailboxesByAccount}
  onSaved={loadVirtualFolders}
  onDeleted={(folderId) => {
    if (selectedFolderId === folderId) {
      clearMarkedSelection();
      selectedFolderId = null;
      messages = [];
    }
  }}
/>

<!-- ── FAB ────────────────────────────────────────────────────────────────── -->
<button class="fab" on:click={() => composeDialog?.open()} aria-label={tl('mail_newMessage')}>
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

  .account-collapse-btn:focus-visible {
    opacity: 1;
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

  .filter-suggestions {
    max-height: 300px;
    overflow-y: auto;
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

  .message-row-icons {
    display: flex;
    align-items: center;
    gap: 0.1rem;
    flex-shrink: 0;
    font-size: 0.75rem;
    line-height: 1;
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

    .mail-layout {
      display: grid;
      flex: 1;
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

    .mail-sidebar.open {
      transform: translateX(0);
    }

    .resize-handle {
      display: none;
    }

  }

  @media (min-width: 1024px) {
    .mail-layout {
      flex: 1;
      min-height: 0;
    }
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

<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { getUser } from '../lib/user.ts';
  import { getLocale, makeTl } from '../lib/i18n.ts';
  import SelectDropdown from './SelectDropdown.svelte';

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
    attachments: Attachment[];
  }

  // ── State ────────────────────────────────────────────────────────────────

  let accounts: Account[] = [];
  let identities: Identity[] = [];
  let mailboxesByAccount: Record<string, Mailbox[]> = {};
  let virtualFolders: VirtualFolder[] = [];
  let syncingMailboxes: Set<string> = new Set();
  let mailboxUnreadCounts: Record<string, Record<string, number>> = {};
  let folderUnreadCounts: Record<string, number> = {};

  let selectedAccountId: string | null = null;
  let selectedMailbox: string | null = null;
  let selectedFolderId: string | null = null;

  let messages: Message[] = [];
  let totalMessages = 0;
  let messagesOffset = 0;
  const pageSize = 50;
  let messagesLoading = false;
  let listScrollEl: HTMLElement;

  let searchQuery = '';
  let searching = false;
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
  let messageLoading = false;

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
  let composeSending = false;
  let composeError = '';
  let showComposeCcBcc = false;
  let composeAttachments: File[] = [];
  let composeReplyAll = false;
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
  let sidebarDialog: HTMLDialogElement;

  let isMobile = false;
  let mediaQueryListener: ((e: MediaQueryListEvent) => void) | null = null;

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

  // ── Lifecycle ────────────────────────────────────────────────────────────

  onMount(async () => {
    const user = await getUser();
    if (!user.ok) {
      window.location.href = '/';
      return;
    }
    displayName = user.display_name || '';
    locale = getLocale();

    await loadAll();
    connectSSE();

    const mql = window.matchMedia('(max-width: 767px)');
    isMobile = mql.matches;
    mediaQueryListener = (e: MediaQueryListEvent) => { isMobile = e.matches; };
    mql.addEventListener('change', mediaQueryListener);

    window.addEventListener('mousemove', handleResizeMove);
    window.addEventListener('mouseup', stopResize);
  });

  onDestroy(() => {
    sseSource?.close();
    if (mediaQueryListener) {
      const mql = window.matchMedia('(max-width: 767px)');
      mql.removeEventListener('change', mediaQueryListener);
    }
    window.removeEventListener('mousemove', handleResizeMove);
    window.removeEventListener('mouseup', stopResize);
    window.removeEventListener('click', handleWindowClick);
    removeFilterListeners();
  });

  // ── Data loading ─────────────────────────────────────────────────────────

  async function loadAll() {
    await Promise.all([loadAccounts(), loadIdentities(), loadVirtualFolders(), loadImageOrigins(), loadBlacklistOrigins(), loadUnreadCounts()]);
    for (const acc of accounts) {
      await loadMailboxes(acc.account_id);
    }
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

  async function loadAccounts() {
    try {
      const r = await fetch('/api/email/accounts');
      if (r.ok) {
        const data = await r.json();
        accounts = data.accounts || [];
      }
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
    selectedAccountId = accountId;
    selectedMailbox = mailbox;
    selectedFolderId = null;
    selectedMessage = null;
    messagesOffset = 0;
    resetSearch();
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
    sidebarDialog?.close();
    await loadMessages();
    fetch(
      `/api/email/sync?account_id=${encodeURIComponent(accountId)}&mailbox=${encodeURIComponent(mailbox)}`,
      { method: 'POST' },
    );
  }

  async function selectVirtualFolder(folderId: string) {
    selectedFolderId = folderId;
    selectedAccountId = null;
    selectedMailbox = null;
    selectedMessage = null;
    messagesOffset = 0;
    resetSearch();
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
    sidebarDialog?.close();
    await loadMessages();
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
    const q = searchQuery.trim();
    if (q === '' && !anyFilterActive()) {
      await loadMessages();
      return;
    }
    let scope = '';
    if (selectedFolderId) {
      scope = `&folder_id=${encodeURIComponent(selectedFolderId)}`;
    } else if (selectedAccountId && selectedMailbox) {
      scope = `&account_id=${encodeURIComponent(selectedAccountId)}&mailbox=${encodeURIComponent(selectedMailbox)}`;
    }
    let filterParams = '';
    if (filterAddrs.length > 0) {
      filterParams += `&addrs=${encodeURIComponent(filterAddrs.join(','))}`;
    }
    if (filterDateFrom) filterParams += `&date_from=${encodeURIComponent(filterDateFrom)}`;
    if (filterDateTo) filterParams += `&date_to=${encodeURIComponent(filterDateTo)}`;
    searching = true;
    selectedMessage = null;
    messagesLoading = true;
    try {
      const r = await fetch(`/api/email/search?q=${encodeURIComponent(q)}${scope}${filterParams}`);
      if (r.ok) {
        const data = await r.json();
        messages = data.messages || [];
      }
    } catch {}
    messagesLoading = false;
  }

  async function clearAllFilters() {
    if (searchDebounce) { clearTimeout(searchDebounce); searchDebounce = null; }
    searchQuery = '';
    searching = false;
    filterAddrs = [];
    filterDateFrom = '';
    filterDateTo = '';
    filterAddrOpen = false;
    filterDateOpen = false;
    removeFilterListeners();
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

  async function loadMessages() {
    messagesLoading = true;
    try {
      let url: string;
      if (selectedFolderId) {
        url = `/api/email/messages?folder_id=${encodeURIComponent(selectedFolderId)}&limit=${pageSize}&offset=${messagesOffset}`;
      } else if (selectedAccountId && selectedMailbox) {
        url = `/api/email/messages?account_id=${encodeURIComponent(selectedAccountId)}&mailbox=${encodeURIComponent(selectedMailbox)}&limit=${pageSize}&offset=${messagesOffset}`;
      } else {
        return;
      }
      const r = await fetch(url);
      if (r.ok) {
        const data = await r.json();
        messages = data.messages || [];
        totalMessages = data.total || 0;
      }
    } catch {}
    messagesLoading = false;
    if (messages.length === 0 && messagesOffset > 0 && totalMessages > 0) {
      messagesOffset = Math.max(0, (Math.ceil(totalMessages / pageSize) - 1) * pageSize);
      await loadMessages();
    }
  }

  async function nextPage() {
    if (!hasNext) return;
    messagesOffset += pageSize;
    await loadMessages();
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function prevPage() {
    if (!hasPrev) return;
    messagesOffset = Math.max(0, messagesOffset - pageSize);
    await loadMessages();
    listScrollEl?.scrollTo({ top: 0 });
  }

  async function openMessage(msg: Message) {
    messageLoading = true;
    selectedMessage = msg;
    actionsOpen = false;
    metaExpanded = false;
    attachmentsExpanded = false;
    imagesNoticeDismissed = false;
    viewMode = 'text'; // reset while loading
    try {
      const url = `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`;
      const r = await fetch(url);
      if (r.ok) {
        const full = await r.json();
        viewMode = htmlAddsValue(full.body_html) ? 'html' : 'text';
        selectedMessage = full;
        messageLoading = false;
        // Update in list
        messages = messages.map(m =>
          m.uid === msg.uid && m.account_id === msg.account_id && m.mailbox === msg.mailbox ? full : m
        );
        // Mark as read
        if (!full.is_read) {
          await setFlag(full, '\\Seen', true);
        }
      }
    } catch {}
    messageLoading = false;
  }

  function closeDetail() {
    selectedMessage = null;
  }

  async function setFlag(msg: Message, flag: string, set: boolean) {
    try {
      await fetch('/api/email/message/flags', {
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
      // Optimistic update
      const newFlags = set
        ? [...msg.flags, flag].filter((f, i, a) => a.indexOf(f) === i)
        : msg.flags.filter(f => f !== flag);
      const updated = { ...msg, flags: newFlags, is_read: newFlags.includes('\\Seen'), is_flagged: newFlags.includes('\\Flagged') };
      messages = messages.map(m =>
        m.uid === msg.uid && m.account_id === msg.account_id ? updated : m
      );
      if (selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id) {
        selectedMessage = updated;
      }
    } catch {}
  }

  async function deleteMessage(msg: Message) {
    const inTrash = folderKind(msg.mailbox) === 'trash';
    if (inTrash && !confirm(tl('mail_deleteMessageConfirm'))) return;
    await fetch(
      `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`,
      { method: 'DELETE' }
    );
    messages = messages.filter(m => !(m.uid === msg.uid && m.account_id === msg.account_id));
    if (selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id) {
      closeDetail();
    }
  }

  async function moveMessage(msg: Message, destMailbox: string) {
    await fetch('/api/email/message/move', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ account_id: msg.account_id, mailbox: msg.mailbox, uid: msg.uid, dest_mailbox: destMailbox }),
    });
    messages = messages.filter(m => !(m.uid === msg.uid && m.account_id === msg.account_id));
    if (selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id) {
      closeDetail();
    }
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
    if (replyTo) {
      composeTo = replyTo.from || '';
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = `\n\n--- Original message ---\nFrom: ${replyTo.from || ''}\n${replyTo.body_text || ''}`;
      composeInReplyTo = replyTo.message_id;
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
      composeIdentityId = (identities.find(i => i.is_default) ?? identities[0])?.identity_id ?? '';
    }
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
  }

  async function sendEmail() {
    composeSending = true;
    composeError = '';
    try {
      const fd = new FormData();
      fd.append('identity_id', composeIdentityId);
      fd.append('to', composeTo);
      fd.append('cc', composeCc);
      fd.append('bcc', composeBcc);
      fd.append('subject', composeSubject);
      fd.append('body', composeBody);
      if (composeInReplyTo) fd.append('in_reply_to', composeInReplyTo);
      for (const file of composeAttachments) {
        fd.append('attachment', file, file.name);
      }
      const r = await fetch('/api/email/send', { method: 'POST', body: fd });
      if (!r.ok) {
        const text = await r.text();
        composeError = text || tl('mail_sendError');
      } else {
        composeDialog?.close();
      }
    } catch {
      composeError = tl('mail_sendError');
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
      selectedFolderId = null;
      messages = [];
    }
    await loadVirtualFolders();
  }

  // ── SSE ──────────────────────────────────────────────────────────────────

  function connectSSE() {
    sseSource = new EventSource('/api/events/stream');
    sseSource.addEventListener('emails_refetch', async (e) => {
      const data = JSON.parse(e.data);
      if (!searching && data.account_id === selectedAccountId && data.mailbox === selectedMailbox) {
        await loadMessages();
      }
      loadUnreadCounts();
    });
    sseSource.addEventListener('email_updated', (e) => {
      const data = JSON.parse(e.data);
      messages = messages.map(m =>
        m.uid === data.uid && m.account_id === data.account_id
          ? { ...m, flags: data.flags, is_read: data.flags.includes('\\Seen'), is_flagged: data.flags.includes('\\Flagged') }
          : m
      );
      loadUnreadCounts();
    });
    sseSource.addEventListener('email_removed', (e) => {
      const data = JSON.parse(e.data);
      const before = messages.length;
      messages = messages.filter(m => !(m.uid === data.uid && m.account_id === data.account_id));
      if (messages.length < before) totalMessages = Math.max(0, totalMessages - 1);
      loadUnreadCounts();
    });
    sseSource.addEventListener('accounts_changed', async () => {
      await loadAccounts();
      for (const acc of accounts) await loadMailboxes(acc.account_id);
    });
    sseSource.addEventListener('mailboxes_changed', async (e) => {
      const data = JSON.parse(e.data);
      await loadMailboxes(data.account_id);
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
      loadUnreadCounts();
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

  $: isSentFolder = !!(selectedMailbox && /sent/i.test(selectedMailbox));
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
  $: layoutColumns = isMobile ? '1fr' : `${sidebarWidth}px 5px ${listWidth}px 5px 1fr`;

  $: pageStart = totalMessages === 0 ? 0 : messagesOffset + 1;
  $: pageEnd = Math.min(messagesOffset + messages.length, totalMessages);
  $: hasPrev = messagesOffset > 0;
  $: hasNext = messagesOffset + pageSize < totalMessages;

  $: htmlHasValue = htmlAddsValue(selectedMessage?.body_html ?? null);
  $: bodyTextSegments = linkifySegments(selectedMessage?.body_text ?? '');

  $: emailOrigins = (() => {
    if (!selectedMessage?.body_html) return [];
    const found = new Set<string>();
    const re = /\/api\/email\/proxy-image\?url=([^"&\s]+)/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(selectedMessage.body_html)) !== null) {
      try {
        const decoded = decodeURIComponent(m[1]);
        const u = new URL(decoded);
        found.add(`${u.protocol}//${u.host}`);
      } catch {}
    }
    return [...found];
  })();

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
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
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

<div class="mail-page">
  <!-- ── Toolbar ────────────────────────────────────────────────────────────── -->
  <div class="mail-toolbar">
    <button class="sidebar-toggle" on:click={() => sidebarDialog?.showModal()} aria-label={tl('mail_aria_folders')}>☰</button>
  </div>

  <!-- ── Layout ─────────────────────────────────────────────────────────────── -->
  <div class="mail-layout" style="grid-template-columns: {layoutColumns}">
    <!-- ── Sidebar ─────────────────────────────────────────────────────────── -->
    <aside class="mail-sidebar">
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
              <button class="icon-btn" on:click|stopPropagation={() => openFolderEdit(folder)} title={tl('mail_editVirtualFolder')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
              <button class="icon-btn delete-btn" on:click|stopPropagation={() => deleteFolder(folder)} title={tl('mail_removeAttachment')}>×</button>
            </div>
          {/each}
        </div>
      {/if}

      <!-- Accounts + mailboxes -->
      {#each accounts as acc}
        <div class="sidebar-section">
          <div class="sidebar-section-header">
            <span class="account-name" title={acc.username}>{acc.display_name}</span>
            <button class="icon-btn" on:click={() => openManageIdentities(acc.account_id)} title={tl('mail_identities')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 3.6-7 8-7s8 3 8 7"/></svg></button>
            <button class="icon-btn" on:click={() => openEditAccount(acc)} title={tl('mail_editAccountHeading')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
            <button class="icon-btn delete-btn" on:click={() => removeAccount(acc)} title={tl('mail_removeAttachment')}>×</button>
          </div>
          {#each (mailboxesByAccount[acc.account_id] || []).toSorted((a, b) => {
            const [ap, an] = folderSortKey(a.name);
            const [bp, bn] = folderSortKey(b.name);
            return ap !== bp ? ap - bp : an.localeCompare(bn);
          }) as mb}
            {#if mb.subscribed}
              {@const syncKey = `${acc.account_id}|${mb.name}`}
              {@const mbUnread = mailboxUnreadCounts[acc.account_id]?.[mb.name] ?? 0}
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
                {:else if mbUnread > 0}
                  <span class="unread-badge">{mbUnread > 999 ? '999+' : mbUnread}</span>
                {/if}
              </button>
            {/if}
          {/each}
        </div>
      {/each}

      <!-- Add account / folder buttons -->
      <div class="sidebar-actions sidebar-add">
        <button class="link-btn" on:click={openAddAccount}>+ {tl('mail_addAccount')}</button>
        <button class="link-btn" on:click={openFolderCreate}>+ {tl('mail_addVirtualFolder')}</button>
      </div>
    </aside>

    <div class="resize-handle" role="separator" aria-orientation="vertical" on:mousedown={(e) => startResize('sidebar', e)}></div>

    <!-- ── Message list ────────────────────────────────────────────────────── -->
    <section class="message-list">
      <div class="message-list-header">
        {#if selectedFolderId}
          <h2>{virtualFolders.find(f => f.folder_id === selectedFolderId)?.display_name || tl('mail_aria_folders')}</h2>
        {:else if selectedMailbox}
          <h2>{FOLDER_NAMES[folderKind(selectedMailbox)] || selectedMailbox}</h2>
        {:else}
          <h2>{tl('nav_mail')}</h2>
        {/if}
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
              class:selected={selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id}
              on:click={() => openMessage(msg)}
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
                  {#if msg.has_attachments}<span class="attach-icon" title={tl('mail_hasAttachments')}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 10.56a4.94 4.94 0 0 1-7-7l9.19-10.56a3.33 3.33 0 0 1 4.7 4.7l-9.2 10.56a1.67 1.67 0 0 1-2.35-2.35l9.2-10.56"/></svg></span>{/if}
                </span>
              </div>
            </button>
          {/each}
        {/if}
      </div>

      {#if totalMessages > 0 && !searching}
        <nav class="pagination-bar">
          <button class="page-btn" on:click={prevPage} disabled={!hasPrev} aria-label={tl('mail_prevPageAria')}>
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="15 18 9 12 15 6"/></svg>
            {tl('mail_prevPage')}
          </button>
          <span class="page-status">{tl('mail_pageStatus', { start: pageStart, end: pageEnd, total: totalMessages })}</span>
          <button class="page-btn" on:click={nextPage} disabled={!hasNext} aria-label={tl('mail_nextPageAria')}>
            {tl('mail_nextPage')}
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 18 15 12 9 6"/></svg>
          </button>
        </nav>
      {/if}
    </section>

    <div class="resize-handle" role="separator" aria-orientation="vertical" on:mousedown={(e) => startResize('list', e)}></div>

    <!-- ── Message detail ────────────────────────────────────────────────────── -->
    <section class="message-detail" class:is-open={isMobile && !!selectedMessage}>
      {#if selectedMessage}
        <div class="detail-header">
          <div class="detail-subject-row">
            {#if isMobile}<button class="detail-close-btn" aria-label={tl('mail_back')} on:click={closeDetail}>←</button>{/if}
            <span class="detail-subject">{selectedMessage.subject || tl('mail_noSubject')}</span>
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
              <button class="actions-toggle" on:click={toggleActions} aria-label={tl('mail_messageActions')}>{#if isMobile}⋮{:else}{tl('mail_actions')} <span class="actions-caret">▾</span>{/if}</button>
              {#if actionsOpen}
                <div class="actions-menu" role="menu">
                  <button role="menuitem" on:click={() => { actionsOpen = false; openComposeReplyAll(selectedMessage!); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="8 14 3 9 8 4"/><polyline points="14 14 9 9 14 4"/><path d="M21 20v-7a4 4 0 0 0-4-4H3"/></svg>
                    {tl('mail_replyAll')}
                  </button>
                  <div class="menu-separator"></div>
                  <button role="menuitem" on:click={() => { actionsOpen = false; setFlag(selectedMessage!, '\\Seen', !selectedMessage!.is_read); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><polyline points="2 4 12 13 22 4"/></svg>
                    {selectedMessage.is_read ? tl('mail_markUnread') : tl('mail_markRead')}
                  </button>
                  {#if htmlHasValue}
                    {#if emailOrigins.length > 0}
                      <button role="menuitem" on:click={() => { actionsOpen = false; openOriginsForEmail(); }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>{tl('mail_imageSourcesEmail')}</button>
                    {/if}

                    <div class="menu-separator"></div>
                    <button role="menuitem" class:menu-item-active={viewMode === 'html'} on:click={() => { viewMode = 'html'; actionsOpen = false; }}>{tl('mail_viewAsHtml')}</button>
                    <button role="menuitem" class:menu-item-active={viewMode === 'text'} on:click={() => { viewMode = 'text'; actionsOpen = false; }}>{tl('mail_viewAsText')}</button>
                  {/if}
                  <div class="menu-separator"></div>
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
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
                    {tl('mail_delete')}
                  </button>
                </div>
              {/if}
            </div>
          </div>
        </div>
        {#if messageLoading}
          <div class="loading">{tl('mail_loading')}</div>
        {:else}
          <div class="detail-meta">
            <div class="meta-primary" tabindex="0" aria-expanded={metaExpanded}
              on:click={() => metaExpanded = !metaExpanded}
              on:keydown={e => (e.key === 'Enter' || e.key === ' ') && (metaExpanded = !metaExpanded)}>
              <span class="meta-label">{primaryMetaLabel}:</span>
              <span class="meta-value">{primaryMetaValue}</span>
              <span class="meta-toggle" aria-hidden="true">{metaExpanded ? '▲' : '▼'}</span>
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
          {#if selectedMessage.attachments && selectedMessage.attachments.length > 0}
            <div class="attachments">
              <button class="attachments-toggle" on:click={() => attachmentsExpanded = !attachmentsExpanded}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg>
                {selectedMessage.attachments.length === 1 ? tl('mail_attachment1') : tl('mail_attachmentsN', { n: selectedMessage.attachments.length })}
                <span class="attachments-caret">{attachmentsExpanded ? '▲' : '▾'}</span>
              </button>
              {#if attachmentsExpanded}
                <div class="attachments-list">
                  {#each selectedMessage.attachments as att}
                    <a
                      class="compose-attachment-chip attachment-chip-dl"
                      href="/api/email/attachment?account_id={encodeURIComponent(selectedMessage.account_id)}&mailbox={encodeURIComponent(selectedMessage.mailbox)}&uid={selectedMessage.uid}&filename={encodeURIComponent(att.filename)}"
                      download={att.filename}
                    >
                      <span class="attachment-icon"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48"/></svg></span>
                      <span class="attachment-name" title={att.filename}>{att.filename}</span>
                      <span class="attachment-size">{humanSize(att.size)}</span>
                    </a>
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
          <div class="detail-body">
            {#if viewMode === 'html' && htmlHasValue}
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
                  sandbox="allow-popups allow-same-origin"
                  srcdoc={selectedMessage.body_html}
                  title={tl('nav_mail')}
                  class="html-body"
                ></iframe>
              {/key}
            {:else if selectedMessage.body_text}
              <pre class="body-text">{#each bodyTextSegments as seg}{#if 'href' in seg}<a href={seg.href} target="_blank" rel="noopener noreferrer">{seg.text}</a>{:else}{seg.text}{/if}{/each}</pre>
            {:else}
              <div class="loading">{tl('mail_loadingBody')}</div>
            {/if}
          </div>
        {/if}
      {:else}
        <div class="empty">{tl('mail_selectMessage')}</div>
      {/if}
    </section>
  </div>
</div>

<!-- ── Mobile sidebar drawer ──────────────────────────────────────────────── -->
<dialog
  bind:this={sidebarDialog}
  class="sidebar-drawer"
  on:click={(e) => { if (e.target === e.currentTarget) sidebarDialog?.close(); }}
>
  <div class="drawer-content">
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
            <button class="icon-btn" on:click|stopPropagation={() => { sidebarDialog?.close(); openFolderEdit(folder); }} title={tl('mail_editVirtualFolder')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
            <button class="icon-btn delete-btn" on:click|stopPropagation={() => deleteFolder(folder)} title={tl('mail_removeAttachment')}>×</button>
          </div>
        {/each}
      </div>
    {/if}

    {#each accounts as acc}
      <div class="sidebar-section">
        <div class="sidebar-section-header">
          <span class="account-name" title={acc.username}>{acc.display_name}</span>
          <button class="icon-btn" on:click={() => { sidebarDialog?.close(); openManageIdentities(acc.account_id); }} title={tl('mail_identities')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 3.6-7 8-7s8 3 8 7"/></svg></button>
          <button class="icon-btn" on:click={() => { sidebarDialog?.close(); openEditAccount(acc); }} title={tl('mail_editAccountHeading')}><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/></svg></button>
          <button class="icon-btn delete-btn" on:click={() => removeAccount(acc)} title={tl('mail_removeAttachment')}>×</button>
        </div>
        {#each (mailboxesByAccount[acc.account_id] || []) as mb}
          {#if mb.subscribed}
            {@const syncKey = `${acc.account_id}|${mb.name}`}
            {@const mbUnread = mailboxUnreadCounts[acc.account_id]?.[mb.name] ?? 0}
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
              {:else if mbUnread > 0}
                <span class="unread-badge">{mbUnread > 999 ? '999+' : mbUnread}</span>
              {/if}
            </button>
          {/if}
        {/each}
      </div>
    {/each}

    <div class="sidebar-actions">
      <button class="link-btn" on:click={() => { sidebarDialog?.close(); openAddAccount(); }}>+ {tl('mail_addAccount')}</button>
      <button class="link-btn" on:click={() => { sidebarDialog?.close(); openFolderCreate(); }}>+ {tl('mail_addVirtualFolder')}</button>
    </div>
  </div>
</dialog>

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

<!-- ── Image origins trigger button ───────────────────────────────────────── -->
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
      <span class="compose-title">{composeInReplyTo ? (composeReplyAll ? tl('mail_replyAll') : tl('mail_reply')) : tl('mail_newMessage')}</span>
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

  .mail-toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
  }

  .sidebar-toggle {
    display: none;
    background: none;
    border: none;
    cursor: pointer;
    font-size: 1.25rem;
    padding: 0.25rem;
    color: inherit;
  }

  .mail-layout {
    display: grid;
    overflow: hidden;
  }

  .resize-handle {
    cursor: col-resize;
    background: var(--pico-muted-border-color, #ddd);
    transition: background 0.12s;
    position: relative;
    z-index: 1;
    width: 3px;
  }

  .resize-handle:hover {
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

  .message-list-header h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
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
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    border-top: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
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

  /* Message detail */
  .message-detail {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    margin-bottom: 0;
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

  .meta-primary {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    cursor: pointer;
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

  /* Dialog overrides */
  .sidebar-drawer {
    margin: 0 auto 0 0;
    height: 100%;
    max-height: 100dvh;
    width: 280px;
    max-width: 80vw;
    border-radius: 0;
    border: none;
    border-right: 1px solid var(--pico-muted-border-color, #ddd);
    padding: 0;
  }

  .sidebar-drawer::backdrop {
    background: rgba(0,0,0,0.2);
  }

  .sidebar-drawer .drawer-content {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow-y: auto;
    background: var(--pico-card-background-color, #f4f4f4);
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
  @media (max-width: 767px) {
    .mail-sidebar {
      display: none;
    }

    .message-detail {
      display: none;
    }

    .message-detail.is-open {
      display: flex;
      position: fixed;
      inset: 0;
      z-index: 100;
      background: var(--pico-card-background-color, white);
      overflow-y: auto;
      box-shadow: inset 0 3px 12px rgba(0,0,0,0.08);
    }

    .message-detail.is-open .detail-header {
      background: var(--pico-card-sectioning-background-color);
      border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    }

    .resize-handle {
      display: none;
    }

    .mail-layout {
      display: flex;
      flex: 1;
      min-height: 0;
    }

    .message-list {
      flex: 1;
      min-height: 0;
    }

    .sidebar-toggle {
      display: block;
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

  @media (min-width: 768px) {
    .mail-toolbar {
      display: none;
    }

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
</style>

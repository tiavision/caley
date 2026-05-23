<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { getUser } from '../lib/user.ts';

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
  let mailboxesByAccount: Record<string, Mailbox[]> = {};
  let virtualFolders: VirtualFolder[] = [];

  let selectedAccountId: string | null = null;
  let selectedMailbox: string | null = null;
  let selectedFolderId: string | null = null;

  let messages: Message[] = [];
  let totalMessages = 0;
  let messagesOffset = 0;
  const pageSize = 50;
  let messagesLoading = false;

  let selectedMessage: Message | null = null;
  let messageLoading = false;

  let sseSource: EventSource | null = null;

  // Account form
  let accountForm = emptyAccountForm();
  let accountSaving = false;
  let accountError = '';

  // Compose form
  let composeTo = '';
  let composeSubject = '';
  let composeBody = '';
  let composeAccountId = '';
  let composeInReplyTo: string | null = null;
  let composeSending = false;
  let composeError = '';

  // Virtual folder form
  let folderName = '';
  let folderSaving = false;

  let displayName = '';
  let authMode = '';

  // HTML email rendering
  let viewMode: 'html' | 'text' = 'text';
  let imageOrigins: string[] = [];
  let iframeKey = 0;

  // Dialog refs
  let accountDialog: HTMLDialogElement;
  let composeDialog: HTMLDialogElement;
  let folderDialog: HTMLDialogElement;
  let detailDialog: HTMLDialogElement;
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
  let metaExpanded = false;
  let desktopActionsEl: HTMLDivElement;
  let mobileActionsEl: HTMLDivElement;
  let actionsMenuPos = { top: 0, right: 0 };
  let attachmentsExpanded = false;
  let imagesNoticeDismissed = false;

  // ── Lifecycle ────────────────────────────────────────────────────────────

  onMount(async () => {
    const user = await getUser();
    if (!user.ok) {
      window.location.href = '/settings/';
      return;
    }
    displayName = user.display_name || '';
    authMode = user.auth_mode || '';

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
  });

  // ── Data loading ─────────────────────────────────────────────────────────

  async function loadAll() {
    await Promise.all([loadAccounts(), loadVirtualFolders(), loadImageOrigins()]);
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
        composeAccountId = accounts[0]?.account_id || '';
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

  async function selectMailbox(accountId: string, mailbox: string) {
    selectedAccountId = accountId;
    selectedMailbox = mailbox;
    selectedFolderId = null;
    selectedMessage = null;
    messagesOffset = 0;
    sidebarDialog?.close();
    await loadMessages();
  }

  async function selectVirtualFolder(folderId: string) {
    selectedFolderId = folderId;
    selectedAccountId = null;
    selectedMailbox = null;
    selectedMessage = null;
    messagesOffset = 0;
    sidebarDialog?.close();
    await loadMessages();
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
        selectedMessage = full;
        viewMode = htmlAddsValue(full.body_html) ? 'html' : 'text';
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
    if (isMobile && !detailDialog?.open) {
      detailDialog?.showModal();
    }
  }

  function closeDetail() {
    detailDialog?.close();
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
    if (!confirm('Delete this message?')) return;
    await fetch(
      `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`,
      { method: 'DELETE' }
    );
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
    accountDialog?.showModal();
  }

  function openEditAccount(acc: Account) {
    accountForm = {
      account_id: acc.account_id,
      display_name: acc.display_name,
      host: acc.host,
      port: acc.port,
      username: acc.username,
      password: '',
      use_tls: acc.use_tls,
      smtp_host: acc.smtp_host || '',
      smtp_port: acc.smtp_port || 587,
    };
    accountError = '';
    accountDialog?.showModal();
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
        accountError = text || 'Failed to save account.';
      } else {
        accountDialog?.close();
        await loadAccounts();
        for (const acc of accounts) {
          await loadMailboxes(acc.account_id);
        }
      }
    } catch {
      accountError = 'Failed to save account.';
    }
    accountSaving = false;
  }

  async function removeAccount(acc: Account) {
    if (!confirm(`Remove account "${acc.display_name}"? This deletes all cached messages.`)) return;
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

  // ── Compose ──────────────────────────────────────────────────────────────

  function openCompose(replyTo?: Message) {
    composeError = '';
    composeSending = false;
    if (replyTo) {
      composeTo = replyTo.from || '';
      composeSubject = replyTo.subject?.startsWith('Re:') ? replyTo.subject : `Re: ${replyTo.subject || ''}`;
      composeBody = `\n\n--- Original message ---\nFrom: ${replyTo.from || ''}\n${replyTo.body_text || ''}`;
      composeInReplyTo = replyTo.message_id;
      composeAccountId = replyTo.account_id;
    } else {
      composeTo = '';
      composeSubject = '';
      composeBody = '';
      composeInReplyTo = null;
      composeAccountId = accounts[0]?.account_id || '';
    }
    composeDialog?.showModal();
  }

  async function sendEmail() {
    composeSending = true;
    composeError = '';
    try {
      const r = await fetch('/api/email/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          account_id: composeAccountId,
          to: composeTo.split(',').map(s => s.trim()).filter(Boolean),
          subject: composeSubject,
          body: composeBody,
          in_reply_to: composeInReplyTo,
        }),
      });
      if (!r.ok) {
        const text = await r.text();
        composeError = text || 'Failed to send.';
      } else {
        composeDialog?.close();
      }
    } catch {
      composeError = 'Failed to send.';
    }
    composeSending = false;
  }

  // ── Virtual folders ──────────────────────────────────────────────────────

  async function createUnifiedInbox() {
    folderSaving = true;
    try {
      await fetch('/api/email/folder', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          display_name: folderName || 'Unified Inbox',
          rules: [{ account_id: null, mailbox_pattern: 'INBOX' }],
        }),
      });
      folderName = '';
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
      if (data.account_id === selectedAccountId && data.mailbox === selectedMailbox) {
        await loadMessages();
      }
    });
    sseSource.addEventListener('email_updated', (e) => {
      const data = JSON.parse(e.data);
      messages = messages.map(m =>
        m.uid === data.uid && m.account_id === data.account_id
          ? { ...m, flags: data.flags, is_read: data.flags.includes('\\Seen'), is_flagged: data.flags.includes('\\Flagged') }
          : m
      );
    });
    sseSource.addEventListener('email_removed', (e) => {
      const data = JSON.parse(e.data);
      messages = messages.filter(m => !(m.uid === data.uid && m.account_id === data.account_id));
    });
    sseSource.addEventListener('accounts_changed', async () => {
      await loadAccounts();
      for (const acc of accounts) await loadMailboxes(acc.account_id);
    });
    sseSource.addEventListener('mailboxes_changed', async (e) => {
      const data = JSON.parse(e.data);
      await loadMailboxes(data.account_id);
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
    return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
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
      window.removeEventListener('click', handleWindowClick);
    }
  }

  function handleWindowClick(e: MouseEvent) {
    const target = e.target as Node;
    if (!desktopActionsEl?.contains(target) && !mobileActionsEl?.contains(target)) {
      actionsOpen = false;
      window.removeEventListener('click', handleWindowClick);
    }
  }

  function humanSize(bytes: number): string {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
  }

  function senderName(from: string | null): string {
    if (!from) return '(unknown)';
    const match = from.match(/^(.+?)\s*<[^>]+>/);
    return match ? match[1].trim() : from;
  }

  $: isSentFolder = !!(selectedMailbox && /sent/i.test(selectedMailbox));
  $: primaryMetaLabel = isSentFolder ? 'To' : 'From';
  $: primaryMetaValue = selectedMessage
    ? (isSentFolder ? selectedMessage.to.join(', ') : (selectedMessage.from || ''))
    : '';
  $: layoutColumns = isMobile ? '1fr' : `${sidebarWidth}px 5px ${listWidth}px 5px 1fr`;

  $: unreadCount = messages.filter(m => !m.is_read).length;

  $: htmlHasValue = htmlAddsValue(selectedMessage?.body_html ?? null);
  $: bodyTextSegments = linkifySegments(selectedMessage?.body_text ?? '');

  $: blockedOrigins = (() => {
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
    return [...found].filter(o => !imageOrigins.includes(o));
  })();
</script>

<div class="mail-page">
  <!-- ── Toolbar ────────────────────────────────────────────────────────────── -->
  <div class="mail-toolbar">
    <button class="sidebar-toggle" on:click={() => sidebarDialog?.showModal()} aria-label="Folders">☰</button>
  </div>

  <!-- ── Layout ─────────────────────────────────────────────────────────────── -->
  <div class="mail-layout" style="grid-template-columns: {layoutColumns}">
    <!-- ── Sidebar ─────────────────────────────────────────────────────────── -->
    <aside class="mail-sidebar">
      <!-- Virtual folders -->
      {#if virtualFolders.length > 0}
        <div class="sidebar-section">
          <div class="sidebar-section-header">
            <span>Folders</span>
          </div>
          {#each virtualFolders as folder}
            <div
              class="sidebar-item"
              class:active={selectedFolderId === folder.folder_id}
              role="button"
              tabindex="0"
              on:click={() => selectVirtualFolder(folder.folder_id)}
              on:keydown={e => e.key === 'Enter' && selectVirtualFolder(folder.folder_id)}
            >
              <span class="folder-icon">📂</span>
              <span class="folder-name">{folder.display_name}</span>
              <button class="icon-btn delete-btn" on:click|stopPropagation={() => deleteFolder(folder)} title="Remove folder">×</button>
            </div>
          {/each}
        </div>
      {/if}

      <!-- Accounts + mailboxes -->
      {#each accounts as acc}
        <div class="sidebar-section">
          <div class="sidebar-section-header">
            <span class="account-name" title={acc.username}>{acc.display_name}</span>
            <button class="icon-btn" on:click={() => openEditAccount(acc)} title="Edit account">✏</button>
            <button class="icon-btn delete-btn" on:click={() => removeAccount(acc)} title="Remove account">×</button>
          </div>
          {#each (mailboxesByAccount[acc.account_id] || []) as mb}
            {#if mb.subscribed}
              <button
                class="sidebar-item"
                class:active={selectedAccountId === acc.account_id && selectedMailbox === mb.name}
                on:click={() => selectMailbox(acc.account_id, mb.name)}
              >
                <span class="folder-icon">{mb.name.toUpperCase() === 'INBOX' ? '📥' : '📁'}</span>
                <span class="folder-name">{mb.name}</span>
              </button>
            {/if}
          {/each}
        </div>
      {/each}

      <!-- Add account / folder buttons -->
      <div class="sidebar-actions">
        <button class="link-btn" on:click={openAddAccount}>+ Add account</button>
        <button class="link-btn" on:click={() => { folderName = 'Unified Inbox'; folderDialog?.showModal(); }}>+ Unified inbox</button>
      </div>
    </aside>

    <div class="resize-handle" role="separator" aria-orientation="vertical" on:mousedown={(e) => startResize('sidebar', e)}></div>

    <!-- ── Message list ────────────────────────────────────────────────────── -->
    <section class="message-list">
      <div class="message-list-header">
        {#if selectedFolderId}
          <h2>{virtualFolders.find(f => f.folder_id === selectedFolderId)?.display_name || 'Folder'}</h2>
        {:else if selectedMailbox}
          <h2>{selectedMailbox}</h2>
        {:else}
          <h2>Mail</h2>
        {/if}
        {#if totalMessages > 0}
          <span class="count">{totalMessages} messages{unreadCount > 0 ? `, ${unreadCount} unread` : ''}</span>
        {/if}
      </div>

      {#if messagesLoading}
        <div class="loading">Loading…</div>
      {:else if messages.length === 0}
        <div class="empty">
          {#if accounts.length === 0}
            No accounts configured. Add an IMAP account to get started.
          {:else if !selectedAccountId && !selectedFolderId}
            Select a mailbox.
          {:else}
            No messages.
          {/if}
        </div>
      {:else}
        {#each messages as msg}
          <button
            class="message-row"
            class:unread={!msg.is_read}
            class:selected={selectedMessage?.uid === msg.uid && selectedMessage?.account_id === msg.account_id}
            on:click={() => openMessage(msg)}
          >
            <div class="message-row-top">
              <span class="from">{senderName(msg.from)}</span>
              <span class="date">{formatDate(msg.sent_at)}</span>
            </div>
            <div class="message-row-subject-line">
              <span class="message-row-subject">{msg.subject || '(no subject)'}</span>
              <span class="message-row-icons">
                {#if msg.is_flagged}<span class="flag-star" title="Flagged">⭐</span>{/if}
                {#if msg.has_attachments}<span class="attach-icon" title="Has attachments"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 10.56a4.94 4.94 0 0 1-7-7l9.19-10.56a3.33 3.33 0 0 1 4.7 4.7l-9.2 10.56a1.67 1.67 0 0 1-2.35-2.35l9.2-10.56"/></svg></span>{/if}
              </span>
            </div>
          </button>
        {/each}

        {#if totalMessages > messages.length + messagesOffset}
          <button class="load-more" on:click={async () => { messagesOffset += pageSize; await loadMessages(); }}>
            Load more
          </button>
        {/if}
      {/if}
    </section>

    <div class="resize-handle" role="separator" aria-orientation="vertical" on:mousedown={(e) => startResize('list', e)}></div>

    <!-- ── Message detail (desktop) ─────────────────────────────────────────── -->
    <section class="message-detail">
      {#if messageLoading}
        <div class="loading">Loading message…</div>
      {:else if selectedMessage}
        <div class="detail-header">
          <div class="detail-subject-row">
            <span class="detail-subject">{selectedMessage.subject || '(no subject)'}</span>
            <button
              class="star-btn"
              class:flagged={selectedMessage.is_flagged}
              on:click={() => setFlag(selectedMessage!, '\\Flagged', !selectedMessage!.is_flagged)}
              aria-label={selectedMessage.is_flagged ? 'Unflag' : 'Flag'}
              title={selectedMessage.is_flagged ? 'Unflag' : 'Flag'}
            >★</button>
            <button class="inline-action-btn" on:click={() => openCompose(selectedMessage!)} aria-label="Reply" title="Reply">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>
            </button>
            <div class="actions-wrapper" bind:this={desktopActionsEl}>
              <button class="actions-toggle" on:click={toggleActions} aria-label="Message actions">Actions <span class="actions-caret">▾</span></button>
              {#if actionsOpen}
                <div class="actions-menu" role="menu">
                  <button role="menuitem" on:click={() => { actionsOpen = false; setFlag(selectedMessage!, '\\Seen', !selectedMessage!.is_read); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><polyline points="2 4 12 13 22 4"/></svg>
                    {selectedMessage.is_read ? 'Mark unread' : 'Mark read'}
                  </button>
                  {#if htmlHasValue}
                    <div class="menu-separator"></div>
                    <button role="menuitem" class:menu-item-active={viewMode === 'html'} on:click={() => { viewMode = 'html'; actionsOpen = false; }}>View as HTML</button>
                    <button role="menuitem" class:menu-item-active={viewMode === 'text'} on:click={() => { viewMode = 'text'; actionsOpen = false; }}>View as plain text</button>
                  {/if}
                  <div class="menu-separator"></div>
                  <button role="menuitem" class="menu-item-danger" on:click={() => { actionsOpen = false; deleteMessage(selectedMessage!); }}>
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
                    Delete
                  </button>
                </div>
              {/if}
            </div>
          </div>
        </div>
        <div class="detail-meta">
          <div class="meta-primary" tabindex="0" aria-expanded={metaExpanded}
            on:click={() => metaExpanded = !metaExpanded}
            on:keydown={e => (e.key === 'Enter' || e.key === ' ') && (metaExpanded = !metaExpanded)}>
            <span class="meta-label">{primaryMetaLabel}:</span>
            <span class="meta-value">{primaryMetaValue}</span>
            <span class="meta-toggle" aria-hidden="true">{metaExpanded ? '▲' : '▼'}</span>
          </div>
          {#if metaExpanded}
            {#if isSentFolder}<div><strong>From:</strong> {selectedMessage.from || ''}</div>{/if}
            {#if !isSentFolder}<div><strong>To:</strong> {selectedMessage.to.join(', ')}</div>{/if}
            {#if selectedMessage.cc.length > 0}
              <div><strong>CC:</strong> {selectedMessage.cc.join(', ')}</div>
            {/if}
            <div><strong>Date:</strong> {selectedMessage.sent_at ? new Date(selectedMessage.sent_at * 1000).toLocaleString() : ''}</div>
          {/if}
        </div>
        {#if selectedMessage.attachments && selectedMessage.attachments.length > 0}
          <div class="attachments">
            <button class="attachments-toggle" on:click={() => attachmentsExpanded = !attachmentsExpanded}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21.44 11.05l-9.19 10.56a4.94 4.94 0 0 1-7-7l9.19-10.56a3.33 3.33 0 0 1 4.7 4.7l-9.2 10.56a1.67 1.67 0 0 1-2.35-2.35l9.2-10.56"/></svg> {selectedMessage.attachments.length} attachment{selectedMessage.attachments.length === 1 ? '' : 's'}
              <span class="attachments-caret">{attachmentsExpanded ? '▲' : '▾'}</span>
            </button>
            {#if attachmentsExpanded}
              <div class="attachments-list">
                {#each selectedMessage.attachments as att}
                  <a
                    class="attachment-link"
                    href="/api/email/attachment?account_id={encodeURIComponent(selectedMessage.account_id)}&mailbox={encodeURIComponent(selectedMessage.mailbox)}&uid={selectedMessage.uid}&filename={encodeURIComponent(att.filename)}"
                    download={att.filename}
                  >
                    {att.filename} ({humanSize(att.size)})
                  </a>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
        <div class="detail-body">
          {#if messageLoading && !selectedMessage.body_text && !selectedMessage.body_html}
            <div class="loading">Loading body…</div>
          {:else}
            {#if viewMode === 'html' && htmlHasValue}
              {#if blockedOrigins.length > 0 && !imagesNoticeDismissed}
                <div class="images-notice">
                  <span>Images are blocked.</span>
                  {#each blockedOrigins as origin}
                    <button class="allow-btn" on:click={() => allowOrigin(origin)}>
                      Allow {origin}
                    </button>
                  {/each}
                  <button class="dismiss-notice-btn" on:click={() => imagesNoticeDismissed = true} aria-label="Dismiss">×</button>
                </div>
              {/if}
              {#key iframeKey}
                <iframe
                  sandbox="allow-popups allow-same-origin"
                  srcdoc={selectedMessage.body_html}
                  title="Email body"
                  class="html-body"
                ></iframe>
              {/key}
            {:else if selectedMessage.body_text}
              <pre class="body-text">{#each bodyTextSegments as seg}{#if 'href' in seg}<a href={seg.href} target="_blank" rel="noopener noreferrer">{seg.text}</a>{:else}{seg.text}{/if}{/each}</pre>
            {:else}
              <div class="loading">Loading body…</div>
            {/if}
          {/if}
        </div>
      {:else}
        <div class="empty">Select a message.</div>
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
          <span>Folders</span>
        </div>
        {#each virtualFolders as folder}
          <div
            class="sidebar-item"
            class:active={selectedFolderId === folder.folder_id}
            role="button"
            tabindex="0"
            on:click={() => selectVirtualFolder(folder.folder_id)}
            on:keydown={e => e.key === 'Enter' && selectVirtualFolder(folder.folder_id)}
          >
            <span class="folder-icon">📂</span>
            <span class="folder-name">{folder.display_name}</span>
            <button class="icon-btn delete-btn" on:click|stopPropagation={() => deleteFolder(folder)} title="Remove folder">×</button>
          </div>
        {/each}
      </div>
    {/if}

    {#each accounts as acc}
      <div class="sidebar-section">
        <div class="sidebar-section-header">
          <span class="account-name" title={acc.username}>{acc.display_name}</span>
          <button class="icon-btn" on:click={() => { sidebarDialog?.close(); openEditAccount(acc); }} title="Edit account">✏</button>
          <button class="icon-btn delete-btn" on:click={() => removeAccount(acc)} title="Remove account">×</button>
        </div>
        {#each (mailboxesByAccount[acc.account_id] || []) as mb}
          {#if mb.subscribed}
            <button
              class="sidebar-item"
              class:active={selectedAccountId === acc.account_id && selectedMailbox === mb.name}
              on:click={() => selectMailbox(acc.account_id, mb.name)}
            >
              <span class="folder-icon">{mb.name.toUpperCase() === 'INBOX' ? '📥' : '📁'}</span>
              <span class="folder-name">{mb.name}</span>
            </button>
          {/if}
        {/each}
      </div>
    {/each}

    <div class="sidebar-actions">
      <button class="link-btn" on:click={() => { sidebarDialog?.close(); openAddAccount(); }}>+ Add account</button>
      <button class="link-btn" on:click={() => { folderName = 'Unified Inbox'; sidebarDialog?.close(); folderDialog?.showModal(); }}>+ Unified inbox</button>
    </div>
  </div>
</dialog>

<!-- ── Mobile email detail dialog ─────────────────────────────────────────── -->
<dialog
  bind:this={detailDialog}
  class="email-detail-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) closeDetail(); }}
  on:cancel={(e) => { e.preventDefault(); closeDetail(); }}
>
  <article>
    <header>
      <h3>{selectedMessage?.subject || '(no subject)'}</h3>
      <div class="header-actions">
        {#if selectedMessage}
          <button
              class="star-btn"
              class:flagged={selectedMessage.is_flagged}
              on:click={() => setFlag(selectedMessage!, '\\Flagged', !selectedMessage!.is_flagged)}
              aria-label={selectedMessage.is_flagged ? 'Unflag' : 'Flag'}
              title={selectedMessage.is_flagged ? 'Unflag' : 'Flag'}
            >★</button>
            <button class="inline-action-btn" on:click={() => { detailDialog?.close(); openCompose(selectedMessage!); }} aria-label="Reply" title="Reply">
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="9 14 4 9 9 4"/><path d="M20 20v-7a4 4 0 0 0-4-4H4"/></svg>
            </button>
            <div class="actions-wrapper" bind:this={mobileActionsEl}>
            <button class="actions-toggle" on:click={toggleActions} aria-label="Message actions">⋮</button>
            {#if actionsOpen}
              <div class="actions-menu" role="menu">
                <button role="menuitem" on:click={() => { actionsOpen = false; setFlag(selectedMessage!, '\\Seen', !selectedMessage!.is_read); }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="4" width="20" height="16" rx="2"/><polyline points="2 4 12 13 22 4"/></svg>
                  {selectedMessage.is_read ? 'Mark unread' : 'Mark read'}
                </button>
                {#if htmlHasValue}
                  <div class="menu-separator"></div>
                  <button role="menuitem" class:menu-item-active={viewMode === 'html'} on:click={() => { viewMode = 'html'; actionsOpen = false; }}>View as HTML</button>
                  <button role="menuitem" class:menu-item-active={viewMode === 'text'} on:click={() => { viewMode = 'text'; actionsOpen = false; }}>View as plain text</button>
                {/if}
                <div class="menu-separator"></div>
                <button role="menuitem" class="menu-item-danger" on:click={() => { actionsOpen = false; deleteMessage(selectedMessage!); }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14H6L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4h6v2"/></svg>
                  Delete
                </button>
              </div>
            {/if}
          </div>
        {/if}
        <button class="close-btn" aria-label="Close" on:click={closeDetail}></button>
      </div>
    </header>
    {#if selectedMessage}
      <div class="detail-meta">
        <div class="meta-primary">
          <span class="meta-label">{primaryMetaLabel}:</span>
          <span class="meta-value">{primaryMetaValue}</span>
          <button class="meta-toggle" on:click={() => metaExpanded = !metaExpanded} aria-label={metaExpanded ? 'Collapse details' : 'Expand details'}>{metaExpanded ? '▲' : '▼'}</button>
        </div>
        {#if metaExpanded}
          <div><strong>From:</strong> {selectedMessage.from || ''}</div>
          <div><strong>To:</strong> {selectedMessage.to.join(', ')}</div>
          {#if selectedMessage.cc.length > 0}
            <div><strong>CC:</strong> {selectedMessage.cc.join(', ')}</div>
          {/if}
          <div><strong>Date:</strong> {selectedMessage.sent_at ? new Date(selectedMessage.sent_at * 1000).toLocaleString() : ''}</div>
        {/if}
      </div>
      {#if selectedMessage.attachments && selectedMessage.attachments.length > 0}
        <div class="attachments">
          {#each selectedMessage.attachments as att}
            <a
              class="attachment-link"
              href="/api/email/attachment?account_id={encodeURIComponent(selectedMessage.account_id)}&mailbox={encodeURIComponent(selectedMessage.mailbox)}&uid={selectedMessage.uid}&filename={encodeURIComponent(att.filename)}"
              download={att.filename}
            >
              {att.filename} ({humanSize(att.size)})
            </a>
          {/each}
        </div>
      {/if}
      <div class="detail-body">
        {#if viewMode === 'html' && htmlHasValue}
          {#if blockedOrigins.length > 0}
            <div class="images-notice">
              <span>Images are blocked.</span>
              {#each blockedOrigins as origin}
                <button class="allow-btn" on:click={() => allowOrigin(origin)}>
                  Allow {origin}
                </button>
              {/each}
            </div>
          {/if}
          {#key iframeKey}
            <iframe
              sandbox="allow-popups allow-same-origin"
              srcdoc={selectedMessage.body_html}
              title="Email body"
              class="html-body"
            ></iframe>
          {/key}
        {:else if selectedMessage.body_text}
          <pre class="body-text">{#each bodyTextSegments as seg}{#if 'href' in seg}<a href={seg.href} target="_blank" rel="noopener noreferrer">{seg.text}</a>{:else}{seg.text}{/if}{/each}</pre>
        {:else}
          <div class="loading">Loading body…</div>
        {/if}
      </div>
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
      <h3>{accountForm.account_id ? 'Edit account' : 'Add IMAP account'}</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label="Close" on:click={() => accountDialog?.close()}></button>
      </div>
    </header>
    <form on:submit|preventDefault={saveAccount}>
      <label>
        Display name
        <input bind:value={accountForm.display_name} placeholder="e.g. Work" />
      </label>
      <label>
        IMAP host
        <input bind:value={accountForm.host} placeholder="imap.example.com" />
      </label>
      <label>
        Port
        <input type="number" bind:value={accountForm.port} />
      </label>
      <label>
        Username / email
        <input bind:value={accountForm.username} placeholder="you@example.com" />
      </label>
      <label>
        Password
        <input type="password" bind:value={accountForm.password} placeholder={accountForm.account_id ? 'Leave blank to keep current' : ''} />
      </label>
      <label class="checkbox-label">
        <input type="checkbox" bind:checked={accountForm.use_tls} /> Use TLS (IMAPS)
      </label>
      <details>
        <summary>SMTP (sending)</summary>
        <label>
          SMTP host (leave blank to use IMAP host)
          <input bind:value={accountForm.smtp_host} placeholder="smtp.example.com" />
        </label>
        <label>
          SMTP port
          <input type="number" bind:value={accountForm.smtp_port} />
        </label>
      </details>
      {#if accountError}
        <p class="error">{accountError}</p>
      {/if}
      <footer>
        <button type="button" class="secondary outline" on:click={() => accountDialog?.close()}>Cancel</button>
        <button type="submit" class="primary" disabled={accountSaving}>
          {accountSaving ? 'Testing connection…' : 'Save'}
        </button>
      </footer>
    </form>
  </article>
</dialog>

<!-- ── Trusted image sources panel ────────────────────────────────────────── -->
{#if imageOrigins.length > 0}
  <details class="origins-panel">
    <summary>Trusted image sources ({imageOrigins.length})</summary>
    <ul class="origins-list">
      {#each imageOrigins as origin}
        <li>
          <span>{origin}</span>
          <button class="icon-btn delete-btn" on:click={() => revokeOrigin(origin)} title="Remove">×</button>
        </li>
      {/each}
    </ul>
  </details>
{/if}

<!-- ── Compose dialog ─────────────────────────────────────────────────────── -->
<dialog
  bind:this={composeDialog}
  class="compose-dialog"
  on:click={(e) => { if (e.target === e.currentTarget) composeDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); composeDialog?.close(); }}
>
  <article>
    <header>
      <h3>New message</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label="Close" on:click={() => composeDialog?.close()}></button>
      </div>
    </header>
    <form on:submit|preventDefault={sendEmail}>
      {#if accounts.length > 1}
        <label>
          From account
          <select bind:value={composeAccountId}>
            {#each accounts as acc}
              <option value={acc.account_id}>{acc.display_name} ({acc.username})</option>
            {/each}
          </select>
        </label>
      {/if}
      <label>
        To
        <input bind:value={composeTo} placeholder="recipient@example.com, ..." />
      </label>
      <label>
        Subject
        <input bind:value={composeSubject} />
      </label>
      <label>
        Body
        <textarea bind:value={composeBody} rows={12}></textarea>
      </label>
      {#if composeError}
        <p class="error">{composeError}</p>
      {/if}
      <footer>
        <button type="button" class="secondary outline" on:click={() => composeDialog?.close()}>Cancel</button>
        <button type="submit" class="primary" disabled={composeSending}>
          {composeSending ? 'Sending…' : 'Send'}
        </button>
      </footer>
    </form>
  </article>
</dialog>

<!-- ── Unified inbox / folder form ────────────────────────────────────────── -->
<dialog
  bind:this={folderDialog}
  on:click={(e) => { if (e.target === e.currentTarget) folderDialog?.close(); }}
  on:cancel={(e) => { e.preventDefault(); folderDialog?.close(); }}
>
  <article>
    <header>
      <h3>Create unified folder</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label="Close" on:click={() => folderDialog?.close()}></button>
      </div>
    </header>
    <form on:submit|preventDefault={createUnifiedInbox}>
      <p>This creates a virtual folder that shows INBOX from all your accounts.</p>
      <label>
        Name
        <input bind:value={folderName} />
      </label>
      <footer>
        <button type="button" class="secondary outline" on:click={() => folderDialog?.close()}>Cancel</button>
        <button type="submit" class="primary" disabled={folderSaving}>Create</button>
      </footer>
    </form>
  </article>
</dialog>

<!-- ── FAB ────────────────────────────────────────────────────────────────── -->
<button class="fab" on:click={() => openCompose()} aria-label="Compose">
  <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
  </svg>
</button>

<style>
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
    padding-bottom: 1rem;
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
    text-transform: uppercase;
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
    color: var(--pico-primary, #0172ad);
    font-weight: 500;
  }

  .folder-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .folder-icon {
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .icon-btn {
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
  }

  @media (min-width: 768px) {
    .message-list {
      overflow-y: auto;
    }
  }

  .message-list-header {
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
  }

  .message-list-header h2 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }

  .count {
    font-size: 0.75rem;
    color: var(--pico-muted-color, #888);
  }

  .message-row {
    display: block;
    width: 100%;
    padding: 0.6rem 1rem;
    border: none;
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

  .message-row.unread .from,
  .message-row.unread .message-row-subject {
    font-weight: 600;
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

  .load-more {
    width: 100%;
    padding: 0.5rem;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--pico-primary, #0172ad);
    font-size: 0.85rem;
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

  .attachment-link {
    display: inline-block;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--pico-muted-border-color, #ddd);
    border-radius: 4px;
    font-size: 0.8rem;
    text-decoration: none;
    color: var(--pico-primary, #2a6496);
    background: var(--pico-card-background-color, #f8f8f8);
  }

  .attachment-link:hover {
    background: var(--pico-primary-background, #e8f0f8);
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

  .html-body {
    width: 100%;
    border: none;
    flex: 1;
    min-height: 0;
  }

  .origins-panel {
    position: fixed;
    bottom: 2.75rem;
    right: 6.5rem;
    transform: translateY(50%);
    background: var(--pico-background-color, white);
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 6px;
    padding: 0.75rem 1rem;
    font-size: 0.82rem;
    z-index: 50;
    min-width: 220px;
    box-shadow: 0 2px 8px rgba(0,0,0,.12);
  }

  .origins-panel summary {
    cursor: pointer;
    color: var(--pico-primary, #0172ad);
    font-size: 0.82rem;
  }

  .origins-list {
    list-style: none;
    padding: 0.5rem 0 0;
    margin: 0;
  }

  .origins-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.2rem 0;
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

  .email-detail-dialog {
    max-width: 48rem;
    width: 95vw;
    max-height: 90dvh;
    padding: 0;
  }

  .email-detail-dialog article {
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    max-height: 90dvh;
  }

  .email-detail-dialog article header {
    padding: 1rem 1.5rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    margin-bottom: 0;
    flex-shrink: 0;
  }

  .email-detail-dialog .detail-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.5rem;
  }

  .compose-dialog {
    max-width: 48rem;
    width: 95vw;
    max-height: 90dvh;
    padding: 0;
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

  /* Responsive */
  @media (max-width: 767px) {
    .mail-sidebar {
      display: none;
    }

    .message-detail {
      display: none;
    }

    .resize-handle {
      display: none;
    }

    .sidebar-toggle {
      display: block;
    }

    .fab {
      bottom: 1.5rem;
      right: 1.5rem;
    }

    .origins-panel {
      bottom: 2.25rem;
      right: 5.5rem;
      transform: translateY(50%);
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
</style>

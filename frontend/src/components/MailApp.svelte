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
  let showAccountForm = false;
  let accountForm = emptyAccountForm();
  let accountSaving = false;
  let accountError = '';

  // Compose form
  let showCompose = false;
  let composeTo = '';
  let composeSubject = '';
  let composeBody = '';
  let composeAccountId = '';
  let composeInReplyTo: string | null = null;
  let composeSending = false;
  let composeError = '';

  // Virtual folder form
  let showFolderForm = false;
  let folderName = '';
  let folderSaving = false;

  let displayName = '';
  let authMode = '';

  // HTML email rendering
  let viewMode: 'html' | 'text' = 'text';
  let imageOrigins: string[] = [];
  let iframeKey = 0;

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
  });

  onDestroy(() => {
    sseSource?.close();
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
    await loadMessages();
  }

  async function selectVirtualFolder(folderId: string) {
    selectedFolderId = folderId;
    selectedAccountId = null;
    selectedMailbox = null;
    selectedMessage = null;
    messagesOffset = 0;
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
    viewMode = 'text'; // reset while loading
    try {
      const url = `/api/email/message?account_id=${encodeURIComponent(msg.account_id)}&mailbox=${encodeURIComponent(msg.mailbox)}&uid=${msg.uid}`;
      const r = await fetch(url);
      if (r.ok) {
        const full = await r.json();
        selectedMessage = full;
        viewMode = full.body_html ? 'html' : 'text';
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
      selectedMessage = null;
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
    showAccountForm = true;
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
    showAccountForm = true;
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
        showAccountForm = false;
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
    showCompose = true;
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
        showCompose = false;
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
      showFolderForm = false;
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

  function senderName(from: string | null): string {
    if (!from) return '(unknown)';
    const match = from.match(/^(.+?)\s*<[^>]+>/);
    return match ? match[1].trim() : from;
  }

  $: unreadCount = messages.filter(m => !m.is_read).length;

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

<div class="mail-layout">
  <!-- ── Sidebar ─────────────────────────────────────────────────────────── -->
  <aside class="mail-sidebar">
    <div class="sidebar-header">
      <nav class="top-nav">
        <a href="/" title="Calendar">📅</a>
        <a href="/contacts/" title="Contacts">👤</a>
        <a href="/mail/" class="active" title="Mail">✉</a>
      </nav>
    </div>

    <button class="compose-btn" on:click={() => openCompose()}>Compose</button>

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
      <button class="link-btn" on:click={() => { showFolderForm = true; folderName = 'Unified Inbox'; }}>+ Unified inbox</button>
    </div>

    {#if displayName}
      <div class="sidebar-footer">
        <span class="user-name">{displayName}</span>
        <a href="/logout" class="logout-link">Sign out</a>
      </div>
    {/if}
  </aside>

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
          <div class="message-row-subject">{msg.subject || '(no subject)'}</div>
          <div class="message-row-meta">
            {#if msg.is_flagged}<span class="flag-star" title="Flagged">⭐</span>{/if}
            {#if msg.has_attachments}<span title="Has attachments">📎</span>{/if}
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

  <!-- ── Message detail ─────────────────────────────────────────────────── -->
  <section class="message-detail">
    {#if messageLoading}
      <div class="loading">Loading message…</div>
    {:else if selectedMessage}
      <div class="detail-header">
        <div class="detail-subject">{selectedMessage.subject || '(no subject)'}</div>
        <div class="detail-actions">
          <button on:click={() => openCompose(selectedMessage!)}>Reply</button>
          <button
            on:click={() => setFlag(selectedMessage!, '\\Flagged', !selectedMessage!.is_flagged)}
            title={selectedMessage.is_flagged ? 'Unflag' : 'Flag'}
          >
            {selectedMessage.is_flagged ? '⭐ Flagged' : '☆ Flag'}
          </button>
          <button
            on:click={() => setFlag(selectedMessage!, '\\Seen', !selectedMessage!.is_read)}
          >
            {selectedMessage.is_read ? 'Mark unread' : 'Mark read'}
          </button>
          <button on:click={() => deleteMessage(selectedMessage!)} class="danger-btn">Delete</button>
        </div>
      </div>
      <div class="detail-meta">
        <div><strong>From:</strong> {selectedMessage.from || ''}</div>
        <div><strong>To:</strong> {selectedMessage.to.join(', ')}</div>
        {#if selectedMessage.cc.length > 0}
          <div><strong>CC:</strong> {selectedMessage.cc.join(', ')}</div>
        {/if}
        <div><strong>Date:</strong> {selectedMessage.sent_at ? new Date(selectedMessage.sent_at * 1000).toLocaleString() : ''}</div>
      </div>
      <div class="detail-body">
        {#if messageLoading && !selectedMessage.body_text && !selectedMessage.body_html}
          <div class="loading">Loading body…</div>
        {:else}
          {#if selectedMessage.body_html}
            <div class="view-toggle">
              <button
                class:active={viewMode === 'html'}
                on:click={() => viewMode = 'html'}
              >HTML</button>
              <button
                class:active={viewMode === 'text'}
                on:click={() => viewMode = 'text'}
              >Plain text</button>
            </div>
          {/if}

          {#if viewMode === 'html' && selectedMessage.body_html}
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
                sandbox="allow-popups"
                srcdoc={selectedMessage.body_html}
                title="Email body"
                class="html-body"
              ></iframe>
            {/key}
          {:else if selectedMessage.body_text}
            <pre class="body-text">{selectedMessage.body_text}</pre>
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

<!-- ── Account form dialog ───────────────────────────────────────────────── -->
{#if showAccountForm}
  <div class="modal-backdrop" on:click={() => showAccountForm = false}>
    <div class="modal" on:click|stopPropagation>
      <h3>{accountForm.account_id ? 'Edit account' : 'Add IMAP account'}</h3>
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
      <div class="modal-actions">
        <button on:click={() => showAccountForm = false}>Cancel</button>
        <button class="primary" on:click={saveAccount} disabled={accountSaving}>
          {accountSaving ? 'Testing connection…' : 'Save'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Trusted image sources dialog ─────────────────────────────────────── -->
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

<!-- ── Compose dialog ────────────────────────────────────────────────────── -->
{#if showCompose}
  <div class="modal-backdrop" on:click={() => showCompose = false}>
    <div class="modal compose-modal" on:click|stopPropagation>
      <h3>New message</h3>
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
      <div class="modal-actions">
        <button on:click={() => showCompose = false}>Cancel</button>
        <button class="primary" on:click={sendEmail} disabled={composeSending}>
          {composeSending ? 'Sending…' : 'Send'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Unified inbox / folder form ──────────────────────────────────────── -->
{#if showFolderForm}
  <div class="modal-backdrop" on:click={() => showFolderForm = false}>
    <div class="modal" on:click|stopPropagation>
      <h3>Create unified folder</h3>
      <p>This creates a virtual folder that shows INBOX from all your accounts.</p>
      <label>
        Name
        <input bind:value={folderName} />
      </label>
      <div class="modal-actions">
        <button on:click={() => showFolderForm = false}>Cancel</button>
        <button class="primary" on:click={createUnifiedInbox} disabled={folderSaving}>Create</button>
      </div>
    </div>
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    height: 100vh;
    overflow: hidden;
  }

  .mail-layout {
    display: grid;
    grid-template-columns: 220px 320px 1fr;
    height: 100vh;
    overflow: hidden;
    font-size: 0.9rem;
  }

  /* Sidebar */
  .mail-sidebar {
    background: var(--pico-card-background-color, #f4f4f4);
    border-right: 1px solid var(--pico-muted-border-color, #ddd);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding-bottom: 1rem;
  }

  .sidebar-header {
    padding: 0.75rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
  }

  .top-nav {
    display: flex;
    gap: 0.5rem;
  }

  .top-nav a {
    text-decoration: none;
    opacity: 0.6;
    font-size: 1.1rem;
  }

  .top-nav a.active {
    opacity: 1;
  }

  .compose-btn {
    margin: 0.75rem;
    padding: 0.5rem 1rem;
    background: var(--pico-primary, #0172ad);
    color: white;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.9rem;
    text-align: left;
  }

  .compose-btn:hover {
    opacity: 0.9;
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

  .sidebar-footer {
    margin-top: auto;
    padding: 0.75rem;
    border-top: 1px solid var(--pico-muted-border-color, #ddd);
    font-size: 0.75rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .user-name {
    color: var(--pico-muted-color, #666);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .logout-link {
    font-size: 0.75rem;
    color: var(--pico-muted-color, #666);
    flex-shrink: 0;
  }

  /* Message list */
  .message-list {
    border-right: 1px solid var(--pico-muted-border-color, #ddd);
    display: flex;
    flex-direction: column;
    overflow: hidden;
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

  .message-row-subject {
    font-size: 0.82rem;
    color: var(--pico-secondary, #444);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .message-row-meta {
    font-size: 0.7rem;
  }

  .flag-star {
    font-size: 0.7rem;
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
  }

  .detail-header {
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #ddd);
    flex-shrink: 0;
  }

  .detail-subject {
    font-size: 1.05rem;
    font-weight: 600;
    margin-bottom: 0.5rem;
  }

  .detail-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .detail-actions button {
    padding: 0.25rem 0.6rem;
    border: 1px solid var(--pico-muted-border-color, #ccc);
    background: none;
    cursor: pointer;
    border-radius: 4px;
    font-size: 0.8rem;
    color: inherit;
  }

  .detail-actions button:hover {
    background: var(--pico-muted-border-color, #f0f0f0);
  }

  .danger-btn {
    color: var(--pico-del-color, #c0392b) !important;
    border-color: var(--pico-del-color, #c0392b) !important;
  }

  .detail-meta {
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--pico-muted-border-color, #eee);
    font-size: 0.82rem;
    color: var(--pico-muted-color, #555);
    flex-shrink: 0;
  }

  .detail-meta div {
    margin-bottom: 0.1rem;
  }

  .detail-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
  }

  .body-text {
    white-space: pre-wrap;
    word-break: break-word;
    font-family: inherit;
    font-size: 0.88rem;
    line-height: 1.5;
    margin: 0;
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
    min-height: 300px;
    border: none;
    display: block;
    flex: 1;
  }

  .origins-panel {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
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

  /* Modals */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .modal {
    background: var(--pico-background-color, white);
    border-radius: 8px;
    padding: 1.5rem;
    width: 480px;
    max-width: 95vw;
    max-height: 90vh;
    overflow-y: auto;
  }

  .compose-modal {
    width: 580px;
  }

  .modal h3 {
    margin: 0 0 1rem;
    font-size: 1rem;
  }

  .modal label {
    display: block;
    margin-bottom: 0.75rem;
    font-size: 0.85rem;
    font-weight: 500;
  }

  .modal input,
  .modal select,
  .modal textarea {
    display: block;
    width: 100%;
    margin-top: 0.2rem;
    padding: 0.4rem 0.6rem;
    border: 1px solid var(--pico-muted-border-color, #ccc);
    border-radius: 4px;
    font-size: 0.85rem;
    font-family: inherit;
    box-sizing: border-box;
    background: var(--pico-background-color, white);
    color: inherit;
  }

  .modal textarea {
    resize: vertical;
  }

  .checkbox-label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .checkbox-label input {
    width: auto;
    display: inline;
    margin: 0;
  }

  .modal details {
    margin-bottom: 0.75rem;
  }

  .modal summary {
    cursor: pointer;
    font-size: 0.85rem;
    color: var(--pico-primary, #0172ad);
    margin-bottom: 0.5rem;
  }

  .modal-actions {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    margin-top: 1rem;
  }

  .modal-actions button {
    padding: 0.4rem 1rem;
    border-radius: 4px;
    border: 1px solid var(--pico-muted-border-color, #ccc);
    background: none;
    cursor: pointer;
    font-size: 0.85rem;
    color: inherit;
  }

  .modal-actions button.primary {
    background: var(--pico-primary, #0172ad);
    color: white;
    border-color: var(--pico-primary, #0172ad);
  }

  .modal-actions button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .error {
    color: var(--pico-del-color, #c0392b);
    font-size: 0.82rem;
    margin: 0.5rem 0 0;
  }

  @media (max-width: 768px) {
    .mail-layout {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }

    .mail-sidebar {
      display: none;
    }

    .message-detail {
      display: none;
    }

    .message-list {
      border-right: none;
    }
  }
</style>

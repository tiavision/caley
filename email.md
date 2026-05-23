# Architecture: Adding Email (IMAP + SMTP) to Caley

## Context

Caley already has a well-structured CalDAV sync pipeline (background tasks, SSE broadcaster, FTS5 search, incremental sync via ctag/sync_token, WebDAV Push). This document describes how to extend it with email: reading mail via IMAP, sending via SMTP, full-text search, real-time updates via SSE, and support for multiple accounts per user with user-defined aggregate folders (unified inbox, etc.).

---

## 1. SQLite FTS5 at Email Scale

**Short answer: Yes, with one hard constraint — index text content only, not raw MIME.**

### What SQLite FTS5 can handle

- FTS5 index is roughly 3–5× the size of indexed text (without trigram; trigram is heavier, ~7–10×).
- SQLite itself scales to hundreds of GB; the limiting factor for email is *what you index*, not the row count.
- At 100K emails with 10KB average stripped-text body: ~1GB of body text → ~3–5GB FTS index. Totally workable.
- At 500K emails × 10KB: ~5GB body text → 15–25GB FTS index. Still functional but queries slow noticeably on spinning disk; on SSD it's fine.
- **Trigram tokenizer would be painful at this scale** — it produces far more tokens per document. Stick with `unicode61` or a Snowball stemmer for email.

### The critical distinction: index stripped text, not raw MIME

Raw email bodies are multipart MIME, often base64-encoded HTML with inline images. Indexing the raw wire format would:
- Balloon the FTS index with garbage tokens (base64 chars, HTML tags, MIME boundaries)
- Destroy search quality

**Indexed columns hold stripped, plain text only**: prefer plain text MIME parts; fall back to stripping HTML tags from HTML parts. This mirrors exactly how `summary_idx`/`description_idx`/`location_idx` work in the current event schema — the `_idx` columns hold normalized text, not raw wire format.

### Existing `search.rs` patterns carry over unchanged

The `SearchMode` enum, `normalize_text()`, and `build_fts_query()` are content-agnostic. A `messages_fts` virtual table uses the same tokenizer-switching and `_idx`-column pattern. `search.rs` needs no changes.

---

## 2. Storage Architecture: SQLite vs. Filesystem vs. Hybrid

### Recommendation: hybrid

- **SQLite stores**: headers (From, To, Subject, Date, Message-ID, In-Reply-To, References), stripped body text, `_idx` columns for FTS, flags (read/starred/etc.), per-message metadata (size, has_attachments, thread_id).
- **SQLite also stores**: raw message text *if* it is under 256KB. Most emails are text-only and small; storing inline avoids filesystem lookups for the common case.
- **Filesystem stores**: attachments (PDFs, images, Office docs). Path: `$DATA_DIR/attachments/{user_id}/{message_uid_hash}/{filename}`.
- **SQLite stores**: attachment metadata (filename, content-type, size, FS path) in `cached_attachments`.

This mirrors how the contacts schema stores `photo` as a base64 string — the same "small things inline, large things by reference" pattern.

---

## 3. Schema

```sql
-- Multiple IMAP accounts per user
CREATE TABLE imap_accounts (
    account_id   TEXT PRIMARY KEY,  -- UUID
    user_id      TEXT NOT NULL,
    display_name TEXT NOT NULL,
    host         TEXT NOT NULL,     -- encrypted
    port         INTEGER NOT NULL,
    username     TEXT NOT NULL,     -- encrypted
    password     TEXT NOT NULL,     -- encrypted
    use_tls      INTEGER NOT NULL DEFAULT 1,
    smtp_host    TEXT,              -- encrypted; NULL = use IMAP host
    smtp_port    INTEGER,
    sort_order   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX imap_accounts_user ON imap_accounts(user_id);

-- Mailboxes per account
CREATE TABLE cached_mailboxes (
    account_id    TEXT NOT NULL,
    name          TEXT NOT NULL,    -- e.g. "INBOX", "Sent"
    delimiter     TEXT,
    uidvalidity   INTEGER,          -- IMAP UIDVALIDITY; change forces full resync
    last_synced   INTEGER,
    subscribed    INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (account_id, name)
);

-- Messages (headers cached on sync, body fetched on demand)
CREATE TABLE cached_messages (
    account_id      TEXT NOT NULL,
    mailbox         TEXT NOT NULL,
    uid             INTEGER NOT NULL,  -- IMAP UID
    message_id      TEXT,              -- RFC 2822 Message-ID
    thread_id       TEXT,              -- derived from References/In-Reply-To
    from_addr       TEXT,
    to_addrs        TEXT,              -- JSON array
    cc_addrs        TEXT,              -- JSON array
    subject         TEXT,
    sent_at         INTEGER,           -- Unix timestamp
    flags           TEXT,              -- JSON array: ["\\Seen", "\\Flagged", ...]
    has_attachments INTEGER NOT NULL DEFAULT 0,
    body_text       TEXT,              -- NULL until message is opened
    raw_message     TEXT,              -- NULL if stored on FS (large messages)
    raw_path        TEXT,              -- FS path if raw_message is NULL
    subject_idx     TEXT,              -- FTS-normalized
    body_idx        TEXT,              -- FTS-normalized; NULL until body fetched
    from_idx        TEXT,              -- FTS-normalized
    PRIMARY KEY (account_id, mailbox, uid)
);
CREATE INDEX cached_messages_sent   ON cached_messages(account_id, sent_at);
CREATE INDEX cached_messages_thread ON cached_messages(thread_id);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject_idx, body_idx, from_idx,
    content=cached_messages, content_rowid=rowid,
    tokenize='unicode61 remove_diacritics 2'
);
-- Triggers on INSERT/UPDATE/DELETE keep messages_fts in sync

-- Attachments on disk
CREATE TABLE cached_attachments (
    id           INTEGER PRIMARY KEY,
    account_id   TEXT NOT NULL,
    mailbox      TEXT NOT NULL,
    uid          INTEGER NOT NULL,
    filename     TEXT,
    content_type TEXT,
    size         INTEGER,
    fs_path      TEXT NOT NULL
);

-- User-defined aggregate folders
CREATE TABLE virtual_folders (
    folder_id    TEXT PRIMARY KEY,  -- UUID
    user_id      TEXT NOT NULL,
    display_name TEXT NOT NULL,
    sort_order   INTEGER NOT NULL DEFAULT 0
);

-- Rules: (account, mailbox LIKE pattern) → folder
CREATE TABLE virtual_folder_rules (
    folder_id       TEXT NOT NULL,
    account_id      TEXT,           -- NULL = all accounts for this user
    mailbox_pattern TEXT,           -- LIKE pattern, e.g. 'INBOX' or '%'
    PRIMARY KEY (folder_id, COALESCE(account_id,''), COALESCE(mailbox_pattern,''))
);
```

---

## 4. IMAP Sync vs. CalDAV Sync

| CalDAV concept | IMAP equivalent |
|---|---|
| `sync_token` (RFC 6578) | UIDVALIDITY + highest seen UID |
| `ctag` mismatch → full resync | UIDVALIDITY change → delete + re-fetch |
| `SyncTrigger::Calendar(href)` | `EmailSyncTrigger::Mailbox(name)` |
| WebDAV Push webhook | Periodic poll (IMAP IDLE as future optimization) |
| `SyncManager` per-user loop | `EmailSyncManager` per-account task |

### What's different

**Connection is stateful.** IMAP is a persistent TCP connection with selected-mailbox state. The sync task holds a live `Session<TlsStream<TcpStream>>` and reconnects if the connection drops. There is no equivalent to the stateless CalDAV HTTP client that is re-created per call.

**Incremental sync:**
1. SELECT mailbox → get UIDVALIDITY
2. UIDVALIDITY changed → delete all cached, full-fetch
3. Otherwise: UID SEARCH ALL → compare with cached UIDs → fetch headers for new UIDs → mark deleted
4. Headers are fetched via `UID FETCH <uids> (FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (...)])`
5. Body fetched lazily on first open via `UID FETCH <uid> (BODY[])`

---

## 5. SSE Extension

```rust
EmailsRefetch { account_id: String, mailbox: String },
EmailRemoved  { account_id: String, uid: u32, mailbox: String },
EmailUpdated  { account_id: String, uid: u32, mailbox: String, flags: Vec<String> },
AccountsChanged,
MailboxesChanged { account_id: String },
```

Virtual folder events are derived client-side from account+mailbox events.

---

## 6. Backend Crate Additions

- `async-imap` — async IMAP client (tokio-native)
- `tokio-rustls` — TLS for IMAP connections (consistent with existing rustls usage)
- `webpki-roots` — root certificates for TLS
- `rustls-pki-types` — server name type for TLS handshake
- `mail-parser` — RFC 2822 / MIME parsing, plain-text extraction
- `lettre` — async SMTP send with rustls

---

## 7. API Endpoints

| Method | Path | Description |
|---|---|---|
| GET | `/api/email/accounts` | List IMAP accounts |
| POST | `/api/email/account` | Add account |
| PUT | `/api/email/account` | Update account |
| DELETE | `/api/email/account` | Remove account |
| GET | `/api/email/mailboxes?account_id=` | List mailboxes |
| GET | `/api/email/folders` | List virtual folders |
| POST | `/api/email/folder` | Create virtual folder |
| DELETE | `/api/email/folder?folder_id=` | Delete virtual folder |
| GET | `/api/email/messages?account_id=&mailbox=&offset=&limit=` | Message list (headers) |
| GET | `/api/email/messages?folder_id=&offset=&limit=` | Virtual folder message list |
| GET | `/api/email/message?account_id=&mailbox=&uid=` | Full message (fetches body) |
| PATCH | `/api/email/message/flags` | Set/clear flags |
| DELETE | `/api/email/message?account_id=&mailbox=&uid=` | Delete message |
| POST | `/api/email/send` | Send email |
| GET | `/api/email/search?q=&account_id=&limit=` | FTS search |

---

## 8. Frontend

New Vite entry point: `mail/index.html` → `src/main/mail.ts` → `MailApp.svelte`

Three-pane layout:
- **Left sidebar**: accounts list with their mailboxes, virtual folders below
- **Middle**: message list (subject, sender, date, unread indicator)
- **Right**: message detail (headers, body, attachments)

SSE stream extended to handle `email_*` events; real-time updates follow the same pattern as calendar/contacts.

---

## 9. Evaluation Summary

| Assumption | Verdict |
|---|---|
| SQLite FTS5 can handle multi-GB email data | **Holds, with conditions**: index stripped text only; avoid trigram at large scale |
| Full emails stored in SQLite | **Partial**: inline if <256KB, otherwise FS path; always store headers in SQLite |
| Attachments on disk | **Yes**: `cached_attachments` table with FS paths |
| Multiple accounts per user | **Yes**: `imap_accounts` keyed by UUID `account_id` threading through all tables |
| Unified/aggregate folders | **Yes**: `virtual_folders` + `virtual_folder_rules`; query-time JOIN, no materialized state |
| Sync manager pattern reuses cleanly | **Yes**: same tokio task + `tokio::select!`; spawned per-account |
| SSE broadcaster reuses cleanly | **Yes**: extend `SseMessage` enum with `account_id` discriminator |
| IMAP IDLE replaces WebDAV Push | **Future**: initial impl uses periodic poll; IDLE fits the same select loop |
| FTS `search.rs` reuses cleanly | **Yes**: new `messages_fts` table; same tokenizer and query builder |
| Main structural risk | IMAP stateful connection: must hold live session + handle reconnects |

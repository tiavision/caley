CREATE TABLE IF NOT EXISTS imap_accounts (
    account_id   TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL,
    display_name TEXT NOT NULL,
    host         TEXT NOT NULL,
    port         INTEGER NOT NULL,
    username     TEXT NOT NULL,
    password     TEXT NOT NULL,
    use_tls      INTEGER NOT NULL DEFAULT 1,
    smtp_host    TEXT,
    smtp_port    INTEGER,
    sort_order   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS imap_accounts_user ON imap_accounts(user_id);

CREATE TABLE IF NOT EXISTS cached_mailboxes (
    account_id  TEXT NOT NULL,
    name        TEXT NOT NULL,
    delimiter   TEXT,
    uidvalidity INTEGER,
    last_synced INTEGER,
    subscribed  INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (account_id, name)
);

CREATE TABLE IF NOT EXISTS cached_messages (
    account_id      TEXT NOT NULL,
    mailbox         TEXT NOT NULL,
    uid             INTEGER NOT NULL,
    message_id      TEXT,
    thread_id       TEXT,
    from_addr       TEXT,
    to_addrs        TEXT,
    cc_addrs        TEXT,
    subject         TEXT,
    sent_at         INTEGER,
    flags           TEXT,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    body_text       TEXT,
    subject_idx     TEXT,
    body_idx        TEXT,
    from_idx        TEXT,
    PRIMARY KEY (account_id, mailbox, uid)
);
CREATE INDEX IF NOT EXISTS cached_messages_sent   ON cached_messages(account_id, sent_at);
CREATE INDEX IF NOT EXISTS cached_messages_thread ON cached_messages(thread_id);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    subject_idx,
    body_idx,
    from_idx,
    content='cached_messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON cached_messages BEGIN
    INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx)
    VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx);
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON cached_messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx)
    VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx);
END;

CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON cached_messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx)
    VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx);
    INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx)
    VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx);
END;

CREATE TABLE IF NOT EXISTS cached_attachments (
    id           INTEGER PRIMARY KEY,
    account_id   TEXT NOT NULL,
    mailbox      TEXT NOT NULL,
    uid          INTEGER NOT NULL,
    filename     TEXT,
    content_type TEXT,
    size         INTEGER,
    fs_path      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS virtual_folders (
    folder_id    TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL,
    display_name TEXT NOT NULL,
    sort_order   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS virtual_folder_rules (
    folder_id       TEXT NOT NULL,
    account_id      TEXT,
    mailbox_pattern TEXT,
    PRIMARY KEY (folder_id, COALESCE(account_id,''), COALESCE(mailbox_pattern,''))
);

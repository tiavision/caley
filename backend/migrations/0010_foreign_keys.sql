-- Add FOREIGN KEY constraints to all tables that reference user_settings,
-- imap_accounts, or virtual_folders.
--
-- SQLite requires recreating a table to add FK constraints. Pattern per table:
--   CREATE TABLE foo_new ( ... REFERENCES parent(id) ON DELETE CASCADE );
--   INSERT INTO foo_new SELECT * FROM foo;
--   DROP TABLE foo;
--   ALTER TABLE foo_new RENAME TO foo;
--
-- Step 1: backfill any user_settings rows that are missing for existing users
-- (e.g. OIDC users who logged in before oidc_post_login called ensure_user).
INSERT OR IGNORE INTO user_settings (user_id)
SELECT user_id FROM caldav_accounts
UNION SELECT user_id FROM deactivated_calendars
UNION SELECT user_id FROM cached_calendars
UNION SELECT user_id FROM cached_events
UNION SELECT user_id FROM push_subscriptions
UNION SELECT user_id FROM cached_addressbooks
UNION SELECT user_id FROM cached_contacts
UNION SELECT user_id FROM contact_push_subscriptions
UNION SELECT user_id FROM imap_accounts
UNION SELECT user_id FROM virtual_folders
UNION SELECT user_id FROM email_image_whitelist;

-- ── Tables referencing user_settings ─────────────────────────────────────────

CREATE TABLE deactivated_calendars_new (
    user_id       TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    calendar_href TEXT NOT NULL,
    PRIMARY KEY (user_id, calendar_href)
);
INSERT INTO deactivated_calendars_new SELECT * FROM deactivated_calendars;
DROP TABLE deactivated_calendars;
ALTER TABLE deactivated_calendars_new RENAME TO deactivated_calendars;

CREATE TABLE cached_calendars_new (
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    href        TEXT NOT NULL,
    name        TEXT NOT NULL,
    color       TEXT,
    ctag        TEXT,
    sync_token  TEXT,
    last_synced INTEGER,
    PRIMARY KEY (user_id, href)
);
INSERT INTO cached_calendars_new SELECT * FROM cached_calendars;
DROP TABLE cached_calendars;
ALTER TABLE cached_calendars_new RENAME TO cached_calendars;

-- cached_events has an FTS content table — drop triggers and FTS first, then
-- recreate the base table, then restore FTS and triggers.
DROP TRIGGER IF EXISTS events_fts_ai;
DROP TRIGGER IF EXISTS events_fts_ad;
DROP TRIGGER IF EXISTS events_fts_au;
DROP TABLE IF EXISTS events_fts;

CREATE TABLE cached_events_new (
    user_id         TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    calendar_href   TEXT NOT NULL,
    uid             TEXT NOT NULL,
    etag            TEXT,
    href            TEXT,
    raw_ical        TEXT NOT NULL,
    summary         TEXT,
    dtstart         TEXT,
    dtend           TEXT,
    description     TEXT NOT NULL DEFAULT '',
    location        TEXT NOT NULL DEFAULT '',
    is_all_day      INTEGER NOT NULL DEFAULT 0,
    is_recurring    INTEGER NOT NULL DEFAULT 0,
    last_modified   INTEGER,
    summary_idx     TEXT NOT NULL DEFAULT '',
    description_idx TEXT NOT NULL DEFAULT '',
    location_idx    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, calendar_href, uid)
);
INSERT INTO cached_events_new SELECT * FROM cached_events;
DROP TABLE cached_events;
ALTER TABLE cached_events_new RENAME TO cached_events;

CREATE INDEX IF NOT EXISTS cached_events_date ON cached_events (user_id, dtstart, dtend);

CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    summary_idx,
    description_idx,
    location_idx,
    content='cached_events',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

INSERT INTO events_fts(events_fts) VALUES('rebuild');

CREATE TRIGGER IF NOT EXISTS events_fts_ai AFTER INSERT ON cached_events BEGIN
    INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
    VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
END;

CREATE TRIGGER IF NOT EXISTS events_fts_ad AFTER DELETE ON cached_events BEGIN
    INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
    VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
END;

CREATE TRIGGER IF NOT EXISTS events_fts_au AFTER UPDATE ON cached_events BEGIN
    INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
    VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
    INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
    VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
END;

CREATE TABLE push_subscriptions_new (
    user_id       TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    calendar_href TEXT NOT NULL,
    webhook_token TEXT,
    transport     TEXT,
    expires_at    INTEGER,
    PRIMARY KEY (user_id, calendar_href)
);
INSERT INTO push_subscriptions_new SELECT * FROM push_subscriptions;
DROP TABLE push_subscriptions;
ALTER TABLE push_subscriptions_new RENAME TO push_subscriptions;

CREATE TABLE caldav_accounts_new (
    account_id      TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    display_name    TEXT,
    caldav_base_url TEXT NOT NULL,
    caldav_username TEXT NOT NULL,
    caldav_password TEXT NOT NULL,
    sort_order      INTEGER NOT NULL DEFAULT 0
);
INSERT INTO caldav_accounts_new SELECT * FROM caldav_accounts;
DROP TABLE caldav_accounts;
ALTER TABLE caldav_accounts_new RENAME TO caldav_accounts;
CREATE INDEX IF NOT EXISTS caldav_accounts_user ON caldav_accounts(user_id);

CREATE TABLE cached_addressbooks_new (
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    href        TEXT NOT NULL,
    name        TEXT NOT NULL,
    ctag        TEXT,
    sync_token  TEXT,
    last_synced INTEGER,
    PRIMARY KEY (user_id, href)
);
INSERT INTO cached_addressbooks_new SELECT * FROM cached_addressbooks;
DROP TABLE cached_addressbooks;
ALTER TABLE cached_addressbooks_new RENAME TO cached_addressbooks;

CREATE TABLE cached_contacts_new (
    user_id          TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    addressbook_href TEXT NOT NULL,
    uid              TEXT NOT NULL,
    etag             TEXT,
    href             TEXT,
    raw_vcard        TEXT NOT NULL,
    fn_name          TEXT NOT NULL DEFAULT '',
    emails           TEXT NOT NULL DEFAULT '[]',
    phones           TEXT NOT NULL DEFAULT '[]',
    org              TEXT NOT NULL DEFAULT '',
    addresses        TEXT NOT NULL DEFAULT '[]',
    photo            TEXT,
    PRIMARY KEY (user_id, addressbook_href, uid)
);
INSERT INTO cached_contacts_new SELECT * FROM cached_contacts;
DROP TABLE cached_contacts;
ALTER TABLE cached_contacts_new RENAME TO cached_contacts;
CREATE INDEX IF NOT EXISTS cached_contacts_name ON cached_contacts (user_id, fn_name COLLATE NOCASE);

CREATE TABLE contact_push_subscriptions_new (
    user_id          TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    addressbook_href TEXT NOT NULL,
    webhook_token    TEXT,
    transport        TEXT,
    expires_at       INTEGER,
    PRIMARY KEY (user_id, addressbook_href)
);
INSERT INTO contact_push_subscriptions_new SELECT * FROM contact_push_subscriptions;
DROP TABLE contact_push_subscriptions;
ALTER TABLE contact_push_subscriptions_new RENAME TO contact_push_subscriptions;

CREATE TABLE imap_accounts_new (
    account_id   TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
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
INSERT INTO imap_accounts_new SELECT * FROM imap_accounts;
DROP TABLE imap_accounts;
ALTER TABLE imap_accounts_new RENAME TO imap_accounts;
CREATE INDEX IF NOT EXISTS imap_accounts_user ON imap_accounts(user_id);

CREATE TABLE virtual_folders_new (
    folder_id    TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    sort_order   INTEGER NOT NULL DEFAULT 0
);
INSERT INTO virtual_folders_new SELECT * FROM virtual_folders;
DROP TABLE virtual_folders;
ALTER TABLE virtual_folders_new RENAME TO virtual_folders;

CREATE TABLE email_image_whitelist_new (
    user_id TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    origin  TEXT NOT NULL,
    PRIMARY KEY (user_id, origin)
);
INSERT INTO email_image_whitelist_new SELECT * FROM email_image_whitelist;
DROP TABLE email_image_whitelist;
ALTER TABLE email_image_whitelist_new RENAME TO email_image_whitelist;

-- ── Tables referencing imap_accounts ─────────────────────────────────────────

CREATE TABLE cached_mailboxes_new (
    account_id  TEXT NOT NULL REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    delimiter   TEXT,
    uidvalidity INTEGER,
    last_synced INTEGER,
    subscribed  INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (account_id, name)
);
INSERT INTO cached_mailboxes_new SELECT * FROM cached_mailboxes;
DROP TABLE cached_mailboxes;
ALTER TABLE cached_mailboxes_new RENAME TO cached_mailboxes;

-- cached_messages has an FTS content table — drop triggers and FTS first.
DROP TRIGGER IF EXISTS messages_fts_ai;
DROP TRIGGER IF EXISTS messages_fts_ad;
DROP TRIGGER IF EXISTS messages_fts_au;
DROP TABLE IF EXISTS messages_fts;

CREATE TABLE cached_messages_new (
    account_id      TEXT NOT NULL REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
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
    body_html       TEXT,
    size            INTEGER,
    PRIMARY KEY (account_id, mailbox, uid)
);
INSERT INTO cached_messages_new SELECT * FROM cached_messages;
DROP TABLE cached_messages;
ALTER TABLE cached_messages_new RENAME TO cached_messages;

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

INSERT INTO messages_fts(messages_fts) VALUES('rebuild');

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

CREATE TABLE cached_attachments_new (
    id           INTEGER PRIMARY KEY,
    account_id   TEXT NOT NULL REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    mailbox      TEXT NOT NULL,
    uid          INTEGER NOT NULL,
    filename     TEXT,
    content_type TEXT,
    size         INTEGER,
    part_index   INTEGER NOT NULL DEFAULT 0,
    content_id   TEXT
);
INSERT INTO cached_attachments_new SELECT * FROM cached_attachments;
DROP TABLE cached_attachments;
ALTER TABLE cached_attachments_new RENAME TO cached_attachments;

-- ── Tables referencing virtual_folders ───────────────────────────────────────

CREATE TABLE virtual_folder_rules_new (
    folder_id       TEXT NOT NULL REFERENCES virtual_folders(folder_id) ON DELETE CASCADE,
    account_id      TEXT NOT NULL DEFAULT '',
    mailbox_pattern TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (folder_id, account_id, mailbox_pattern)
);
INSERT INTO virtual_folder_rules_new SELECT * FROM virtual_folder_rules;
DROP TABLE virtual_folder_rules;
ALTER TABLE virtual_folder_rules_new RENAME TO virtual_folder_rules;

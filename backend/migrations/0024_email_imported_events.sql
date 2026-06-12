CREATE TABLE IF NOT EXISTS email_imported_events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    email_account_id TEXT    NOT NULL,
    email_mailbox    TEXT    NOT NULL,
    email_uid        INTEGER NOT NULL,
    cal_account_id   TEXT    NOT NULL,
    calendar_href    TEXT    NOT NULL,
    event_uid        TEXT    NOT NULL,
    created_at       INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(email_account_id, email_mailbox, email_uid, event_uid)
);
CREATE INDEX IF NOT EXISTS idx_email_imported_events_lookup
    ON email_imported_events(email_account_id, email_mailbox, email_uid);

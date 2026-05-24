CREATE TABLE IF NOT EXISTS email_identities (
    identity_id  TEXT PRIMARY KEY,
    account_id   TEXT NOT NULL,
    user_id      TEXT NOT NULL,
    name         TEXT NOT NULL,
    email        TEXT NOT NULL,
    is_default   INTEGER NOT NULL DEFAULT 0,
    sort_order   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (account_id) REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES user_settings(user_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_email_identities_account ON email_identities(account_id);
CREATE INDEX IF NOT EXISTS idx_email_identities_user   ON email_identities(user_id);

-- Decouple CalDAV credentials from the user-identity record.
-- `user_settings` becomes the canonical per-user row (referenced by many tables)
-- and must exist even when no CalDAV account is configured. CalDAV credentials
-- move into `caldav_accounts`, modelled to hold multiple accounts per user.

CREATE TABLE IF NOT EXISTS caldav_accounts (
    account_id      TEXT PRIMARY KEY,
    user_id         TEXT NOT NULL,
    display_name    TEXT,
    caldav_base_url TEXT NOT NULL,
    caldav_username TEXT NOT NULL,
    caldav_password TEXT NOT NULL,
    sort_order      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS caldav_accounts_user ON caldav_accounts(user_id);

-- Move existing single-account credentials over verbatim (already encrypted).
INSERT INTO caldav_accounts (account_id, user_id, caldav_base_url, caldav_username, caldav_password)
SELECT user_id || ':default', user_id, caldav_base_url, caldav_username, caldav_password
FROM user_settings;

-- Reduce user_settings to the identity record (drop the CalDAV columns).
CREATE TABLE user_settings_new (
    user_id      TEXT PRIMARY KEY,
    display_name TEXT
);
INSERT INTO user_settings_new (user_id)
SELECT user_id FROM user_settings;
DROP TABLE user_settings;
ALTER TABLE user_settings_new RENAME TO user_settings;

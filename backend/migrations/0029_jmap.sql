-- JMAP accounts (RFC 8620/8621) beside IMAP ones. An account's `protocol` is 'imap' or 'jmap'; a JMAP account keeps
-- its session URL (encrypted, like the IMAP host) and how it signs in: 'password' (HTTP Basic), 'bearer' (an API
-- token, kept in `password`) or 'oidc' (the user's own OIDC access token, oidc_tokens below).
ALTER TABLE imap_accounts ADD COLUMN protocol TEXT NOT NULL DEFAULT 'imap';
ALTER TABLE imap_accounts ADD COLUMN jmap_url TEXT;
ALTER TABLE imap_accounts ADD COLUMN auth TEXT NOT NULL DEFAULT 'password';

-- The tokens of a user's last OIDC sign-in, encrypted, so that background work (the JMAP sync) can act for the user
-- between requests; the refresh token renews the access token when it expires.
CREATE TABLE oidc_tokens (
    user_id       TEXT PRIMARY KEY REFERENCES user_settings(user_id) ON DELETE CASCADE,
    access_token  TEXT NOT NULL,
    refresh_token TEXT,
    -- unix seconds; NULL: the provider did not say
    expires_at    INTEGER
);

-- A JMAP account's mailboxes: the server's id, the name the cache knows the mailbox by (its path, `/`-separated;
-- the inbox is `INBOX` as in IMAP), its role, and the next UID to give a message there. UIDs are never reused.
CREATE TABLE jmap_mailboxes (
    account_id TEXT NOT NULL REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    mailbox_id TEXT NOT NULL,
    name       TEXT NOT NULL,
    role       TEXT,
    next_uid   INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (account_id, mailbox_id)
);
CREATE UNIQUE INDEX jmap_mailboxes_name ON jmap_mailboxes(account_id, name);

-- The JMAP email behind a cached message: one row per mailbox the email is in, the cached message's UID there.
CREATE TABLE jmap_messages (
    account_id TEXT NOT NULL,
    mailbox    TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    email_id   TEXT NOT NULL,
    blob_id    TEXT NOT NULL,
    PRIMARY KEY (account_id, mailbox, uid),
    FOREIGN KEY (account_id, mailbox, uid)
        REFERENCES cached_messages(account_id, mailbox, uid) ON DELETE CASCADE
);
CREATE INDEX jmap_messages_email ON jmap_messages(account_id, email_id);

-- The JMAP states the cache is at, per account and type ('Mailbox', 'Email'): Foo/changes starts from them.
CREATE TABLE jmap_states (
    account_id TEXT NOT NULL REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    type       TEXT NOT NULL,
    state      TEXT NOT NULL,
    PRIMARY KEY (account_id, type)
);

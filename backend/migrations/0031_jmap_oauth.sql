-- JMAP accounts that sign in at the mail server itself (auth = 'oauth', RFC 6749 authorization code with PKCE): the
-- OAuth client the server has for Caley (its id, and its secret encrypted; none for a public client), and the
-- account's tokens (encrypted) with the token endpoint that renews them.
ALTER TABLE imap_accounts ADD COLUMN oauth_client_id TEXT;
ALTER TABLE imap_accounts ADD COLUMN oauth_client_secret TEXT;

CREATE TABLE jmap_oauth_tokens (
    account_id    TEXT PRIMARY KEY REFERENCES imap_accounts(account_id) ON DELETE CASCADE,
    token_url     TEXT NOT NULL,
    access_token  TEXT NOT NULL,
    refresh_token TEXT,
    -- unix seconds; NULL: the server did not say
    expires_at    INTEGER
);

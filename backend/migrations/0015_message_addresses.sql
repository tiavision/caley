CREATE TABLE IF NOT EXISTS message_addresses (
    account_id TEXT    NOT NULL,
    mailbox    TEXT    NOT NULL,
    uid        INTEGER NOT NULL,
    field      TEXT    NOT NULL,   -- 'from' | 'to' | 'cc'
    addr       TEXT    NOT NULL,   -- lowercased email, display name stripped
    FOREIGN KEY (account_id, mailbox, uid)
        REFERENCES cached_messages(account_id, mailbox, uid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS message_addresses_addr ON message_addresses(addr);
CREATE INDEX IF NOT EXISTS message_addresses_msg  ON message_addresses(account_id, mailbox, uid);

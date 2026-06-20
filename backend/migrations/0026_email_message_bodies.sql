CREATE TABLE cached_message_bodies (
    account_id        TEXT NOT NULL,
    mailbox           TEXT NOT NULL,
    uid               INTEGER NOT NULL,
    body_text         TEXT,
    body_html         TEXT,
    autocrypt_headers TEXT NOT NULL DEFAULT '[]',
    is_signed         INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, mailbox, uid),
    FOREIGN KEY (account_id, mailbox, uid)
        REFERENCES cached_messages(account_id, mailbox, uid) ON DELETE CASCADE
);

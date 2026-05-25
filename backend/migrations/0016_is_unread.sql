ALTER TABLE cached_messages ADD COLUMN is_unread INTEGER NOT NULL DEFAULT 1;

UPDATE cached_messages
SET is_unread = CASE
    WHEN EXISTS (SELECT 1 FROM json_each(flags) WHERE value = '\Seen') THEN 0
    ELSE 1
END;

CREATE INDEX IF NOT EXISTS cached_messages_unread
    ON cached_messages(account_id, mailbox, is_unread);

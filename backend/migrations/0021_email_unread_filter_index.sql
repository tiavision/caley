CREATE INDEX IF NOT EXISTS cached_messages_unread_sent
    ON cached_messages(sent_at)
    WHERE is_unread = 1;

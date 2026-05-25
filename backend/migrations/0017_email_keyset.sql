CREATE INDEX IF NOT EXISTS cached_messages_folder_sent
    ON cached_messages(account_id, mailbox, sent_at);

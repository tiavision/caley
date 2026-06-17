ALTER TABLE email_imported_events ADD COLUMN email_message_id TEXT;
ALTER TABLE email_imported_events ADD COLUMN email_thread_id TEXT;

UPDATE email_imported_events
SET email_message_id = (
        SELECT cm.message_id
        FROM cached_messages cm
        WHERE cm.account_id = email_imported_events.email_account_id
          AND cm.mailbox = email_imported_events.email_mailbox
          AND cm.uid = email_imported_events.email_uid
        LIMIT 1
    ),
    email_thread_id = (
        SELECT cm.thread_id
        FROM cached_messages cm
        WHERE cm.account_id = email_imported_events.email_account_id
          AND cm.mailbox = email_imported_events.email_mailbox
          AND cm.uid = email_imported_events.email_uid
        LIMIT 1
    )
WHERE email_message_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_email_imported_events_message_id
    ON email_imported_events(email_account_id, email_message_id)
    WHERE email_message_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_email_imported_events_thread_id
    ON email_imported_events(email_account_id, email_thread_id)
    WHERE email_thread_id IS NOT NULL;

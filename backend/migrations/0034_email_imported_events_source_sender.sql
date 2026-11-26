-- The sender and Date of the mail an event was imported from: a mail elsewhere counts as that mail only when its
-- Message-ID, sender and Date all match (a Message-ID is whatever a sender writes). Rows imported before this keep
-- matching by Message-ID alone (email_source_known = 0) unless their mail is still where it was imported from.
ALTER TABLE email_imported_events ADD COLUMN email_from_addr TEXT;
ALTER TABLE email_imported_events ADD COLUMN email_sent_at INTEGER;
ALTER TABLE email_imported_events ADD COLUMN email_source_known INTEGER NOT NULL DEFAULT 0;

UPDATE email_imported_events
SET email_from_addr = (
        SELECT cm.from_addr FROM cached_messages cm
         WHERE cm.account_id = email_imported_events.email_account_id
           AND cm.mailbox = email_imported_events.email_mailbox
           AND cm.uid = email_imported_events.email_uid
    ),
    email_sent_at = (
        SELECT cm.sent_at FROM cached_messages cm
         WHERE cm.account_id = email_imported_events.email_account_id
           AND cm.mailbox = email_imported_events.email_mailbox
           AND cm.uid = email_imported_events.email_uid
    ),
    email_source_known = 1
WHERE EXISTS (
    SELECT 1 FROM cached_messages cm
     WHERE cm.account_id = email_imported_events.email_account_id
       AND cm.mailbox = email_imported_events.email_mailbox
       AND cm.uid = email_imported_events.email_uid
       AND cm.message_id IS email_imported_events.email_message_id
);

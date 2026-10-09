-- JSON array of Reply-To addresses. NULL means not parsed yet (rows cached
-- before this column existed); '[]' means the message has no Reply-To.
ALTER TABLE cached_messages ADD COLUMN reply_to_addrs TEXT;

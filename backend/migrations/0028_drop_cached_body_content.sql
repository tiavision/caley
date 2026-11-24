-- The reader reads every message from its source (the .eml, GET /email/message/raw); the cached bodies' text, HTML
-- and Autocrypt headers are no longer read. A row still says the body was synced, and whether it is signed.
ALTER TABLE cached_message_bodies DROP COLUMN body_text;
ALTER TABLE cached_message_bodies DROP COLUMN body_html;
ALTER TABLE cached_message_bodies DROP COLUMN autocrypt_headers;

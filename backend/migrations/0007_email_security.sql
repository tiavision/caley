ALTER TABLE cached_messages ADD COLUMN body_html TEXT;

CREATE TABLE IF NOT EXISTS email_image_whitelist (
    user_id TEXT NOT NULL,
    origin  TEXT NOT NULL,
    PRIMARY KEY (user_id, origin)
);

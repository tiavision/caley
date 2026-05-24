CREATE TABLE IF NOT EXISTS email_image_blacklist (
    user_id TEXT NOT NULL,
    origin  TEXT NOT NULL,
    PRIMARY KEY (user_id, origin),
    FOREIGN KEY (user_id) REFERENCES user_settings(user_id) ON DELETE CASCADE
);

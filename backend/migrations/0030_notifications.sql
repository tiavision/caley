-- Web Push subscriptions of the user's devices (notifications.rs): the app on Android registers one per
-- server through UnifiedPush. `p256dh` and `auth` are the device's RFC 8291 keys (base64url).
CREATE TABLE IF NOT EXISTS notification_subscriptions (
    endpoint    TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    p256dh      TEXT NOT NULL,
    auth        TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS notification_subscriptions_user ON notification_subscriptions(user_id);

-- Secrets of the server itself, encrypted with ENCRYPTION_KEY (crypto.rs): the VAPID signing key.
CREATE TABLE IF NOT EXISTS server_secrets (
    name        TEXT PRIMARY KEY,
    value       TEXT NOT NULL
);

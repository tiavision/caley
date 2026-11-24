-- Calendar reminders by Web Push (reminders.rs). A device's timezone (IANA), which all-day and floating
-- events' reminders are timed in; and the reminders sent, so each goes out once.
ALTER TABLE notification_subscriptions ADD COLUMN timezone TEXT;

CREATE TABLE IF NOT EXISTS sent_reminders (
    user_id     TEXT NOT NULL REFERENCES user_settings(user_id) ON DELETE CASCADE,
    key         TEXT NOT NULL,
    fire_at     INTEGER NOT NULL,
    PRIMARY KEY (user_id, key)
);
CREATE INDEX IF NOT EXISTS sent_reminders_fire_at ON sent_reminders(fire_at);

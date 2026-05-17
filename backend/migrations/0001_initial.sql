CREATE TABLE IF NOT EXISTS user_settings (
    user_id TEXT PRIMARY KEY,
    caldav_base_url TEXT NOT NULL,
    caldav_username TEXT NOT NULL,
    caldav_password TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS deactivated_calendars (
    user_id TEXT NOT NULL,
    calendar_href TEXT NOT NULL,
    PRIMARY KEY (user_id, calendar_href)
);

CREATE TABLE IF NOT EXISTS cached_calendars (
    user_id      TEXT NOT NULL,
    href         TEXT NOT NULL,
    name         TEXT NOT NULL,
    color        TEXT,
    ctag         TEXT,
    sync_token   TEXT,
    last_synced  INTEGER,
    PRIMARY KEY (user_id, href)
);

CREATE TABLE IF NOT EXISTS cached_events (
    user_id        TEXT NOT NULL,
    calendar_href  TEXT NOT NULL,
    uid            TEXT NOT NULL,
    etag           TEXT,
    href           TEXT,
    raw_ical       TEXT NOT NULL,
    summary        TEXT,
    dtstart        TEXT,
    dtend          TEXT,
    is_all_day     INTEGER NOT NULL DEFAULT 0,
    is_recurring   INTEGER NOT NULL DEFAULT 0,
    last_modified  INTEGER,
    PRIMARY KEY (user_id, calendar_href, uid)
);

CREATE INDEX IF NOT EXISTS cached_events_date
    ON cached_events (user_id, dtstart, dtend);

CREATE TABLE IF NOT EXISTS push_subscriptions (
    user_id          TEXT NOT NULL,
    calendar_href    TEXT NOT NULL,
    webhook_token    TEXT,
    transport        TEXT,
    expires_at       INTEGER,
    PRIMARY KEY (user_id, calendar_href)
);

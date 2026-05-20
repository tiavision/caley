CREATE TABLE IF NOT EXISTS cached_addressbooks (
    user_id      TEXT NOT NULL,
    href         TEXT NOT NULL,
    name         TEXT NOT NULL,
    ctag         TEXT,
    sync_token   TEXT,
    last_synced  INTEGER,
    PRIMARY KEY (user_id, href)
);

CREATE TABLE IF NOT EXISTS cached_contacts (
    user_id           TEXT NOT NULL,
    addressbook_href  TEXT NOT NULL,
    uid               TEXT NOT NULL,
    etag              TEXT,
    href              TEXT,
    raw_vcard         TEXT NOT NULL,
    fn_name           TEXT NOT NULL DEFAULT '',
    emails            TEXT NOT NULL DEFAULT '[]',
    phones            TEXT NOT NULL DEFAULT '[]',
    org               TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, addressbook_href, uid)
);

CREATE INDEX IF NOT EXISTS cached_contacts_name
    ON cached_contacts (user_id, fn_name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS contact_push_subscriptions (
    user_id           TEXT NOT NULL,
    addressbook_href  TEXT NOT NULL,
    webhook_token     TEXT,
    transport         TEXT,
    expires_at        INTEGER,
    PRIMARY KEY (user_id, addressbook_href)
);

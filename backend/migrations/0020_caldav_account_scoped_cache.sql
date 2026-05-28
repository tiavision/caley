-- Scope CalDAV calendar and contact cache data by CalDAV account.
-- The earlier schema allowed multiple CalDAV accounts, but cached calendars,
-- events, addressbooks, contacts, and push subscriptions were still keyed only
-- by app user plus href. That collides when two accounts expose the same href.

DROP TRIGGER IF EXISTS events_fts_ai;
DROP TRIGGER IF EXISTS events_fts_ad;
DROP TRIGGER IF EXISTS events_fts_au;
DROP TABLE IF EXISTS events_fts;

CREATE TABLE deactivated_calendars_new (
    account_id    TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    calendar_href TEXT NOT NULL,
    PRIMARY KEY (account_id, calendar_href)
);
INSERT INTO deactivated_calendars_new (account_id, calendar_href)
SELECT ca.account_id, d.calendar_href
FROM deactivated_calendars d
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = d.user_id;
DROP TABLE deactivated_calendars;
ALTER TABLE deactivated_calendars_new RENAME TO deactivated_calendars;

CREATE TABLE cached_calendars_new (
    account_id  TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    href        TEXT NOT NULL,
    name        TEXT NOT NULL,
    color       TEXT,
    ctag        TEXT,
    sync_token  TEXT,
    last_synced INTEGER,
    PRIMARY KEY (account_id, href)
);
INSERT INTO cached_calendars_new (account_id, href, name, color, ctag, sync_token, last_synced)
SELECT ca.account_id, c.href, c.name, c.color, c.ctag, c.sync_token, c.last_synced
FROM cached_calendars c
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = c.user_id;
DROP TABLE cached_calendars;
ALTER TABLE cached_calendars_new RENAME TO cached_calendars;

CREATE TABLE cached_events_new (
    account_id      TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    calendar_href   TEXT NOT NULL,
    uid             TEXT NOT NULL,
    etag            TEXT,
    href            TEXT,
    raw_ical        TEXT NOT NULL,
    summary         TEXT,
    dtstart         TEXT,
    dtend           TEXT,
    description     TEXT NOT NULL DEFAULT '',
    location        TEXT NOT NULL DEFAULT '',
    is_all_day      INTEGER NOT NULL DEFAULT 0,
    is_recurring    INTEGER NOT NULL DEFAULT 0,
    last_modified   INTEGER,
    summary_idx     TEXT NOT NULL DEFAULT '',
    description_idx TEXT NOT NULL DEFAULT '',
    location_idx    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (account_id, calendar_href, uid)
);
INSERT INTO cached_events_new
    (account_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend,
     description, location, is_all_day, is_recurring, last_modified,
     summary_idx, description_idx, location_idx)
SELECT ca.account_id, e.calendar_href, e.uid, e.etag, e.href, e.raw_ical, e.summary,
       e.dtstart, e.dtend, e.description, e.location, e.is_all_day, e.is_recurring,
       e.last_modified, e.summary_idx, e.description_idx, e.location_idx
FROM cached_events e
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = e.user_id;
DROP TABLE cached_events;
ALTER TABLE cached_events_new RENAME TO cached_events;
CREATE INDEX IF NOT EXISTS cached_events_date ON cached_events (account_id, dtstart, dtend);

CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    summary_idx,
    description_idx,
    location_idx,
    content='cached_events',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO events_fts(events_fts) VALUES('rebuild');
CREATE TRIGGER IF NOT EXISTS events_fts_ai AFTER INSERT ON cached_events BEGIN
    INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
    VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
END;
CREATE TRIGGER IF NOT EXISTS events_fts_ad AFTER DELETE ON cached_events BEGIN
    INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
    VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
END;
CREATE TRIGGER IF NOT EXISTS events_fts_au AFTER UPDATE ON cached_events BEGIN
    INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
    VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
    INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
    VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
END;

CREATE TABLE push_subscriptions_new (
    account_id    TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    calendar_href TEXT NOT NULL,
    webhook_token TEXT,
    transport     TEXT,
    expires_at    INTEGER,
    PRIMARY KEY (account_id, calendar_href)
);
INSERT INTO push_subscriptions_new (account_id, calendar_href, webhook_token, transport, expires_at)
SELECT ca.account_id, p.calendar_href, p.webhook_token, p.transport, p.expires_at
FROM push_subscriptions p
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = p.user_id;
DROP TABLE push_subscriptions;
ALTER TABLE push_subscriptions_new RENAME TO push_subscriptions;

CREATE TABLE cached_addressbooks_new (
    account_id  TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    href        TEXT NOT NULL,
    name        TEXT NOT NULL,
    ctag        TEXT,
    sync_token  TEXT,
    last_synced INTEGER,
    PRIMARY KEY (account_id, href)
);
INSERT INTO cached_addressbooks_new (account_id, href, name, ctag, sync_token, last_synced)
SELECT ca.account_id, a.href, a.name, a.ctag, a.sync_token, a.last_synced
FROM cached_addressbooks a
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = a.user_id;
DROP TABLE cached_addressbooks;
ALTER TABLE cached_addressbooks_new RENAME TO cached_addressbooks;

CREATE TABLE cached_contacts_new (
    account_id       TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    addressbook_href TEXT NOT NULL,
    uid              TEXT NOT NULL,
    etag             TEXT,
    href             TEXT,
    raw_vcard        TEXT NOT NULL,
    fn_name          TEXT NOT NULL DEFAULT '',
    emails           TEXT NOT NULL DEFAULT '[]',
    phones           TEXT NOT NULL DEFAULT '[]',
    org              TEXT NOT NULL DEFAULT '',
    addresses        TEXT NOT NULL DEFAULT '[]',
    photo            TEXT,
    PRIMARY KEY (account_id, addressbook_href, uid)
);
INSERT INTO cached_contacts_new
    (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails,
     phones, org, addresses, photo)
SELECT ca.account_id, c.addressbook_href, c.uid, c.etag, c.href, c.raw_vcard,
       c.fn_name, c.emails, c.phones, c.org, c.addresses, c.photo
FROM cached_contacts c
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = c.user_id;
DROP TABLE cached_contacts;
ALTER TABLE cached_contacts_new RENAME TO cached_contacts;
CREATE INDEX IF NOT EXISTS cached_contacts_name ON cached_contacts (account_id, fn_name COLLATE NOCASE);

CREATE TABLE contact_push_subscriptions_new (
    account_id       TEXT NOT NULL REFERENCES caldav_accounts(account_id) ON DELETE CASCADE,
    addressbook_href TEXT NOT NULL,
    webhook_token    TEXT,
    transport        TEXT,
    expires_at       INTEGER,
    PRIMARY KEY (account_id, addressbook_href)
);
INSERT INTO contact_push_subscriptions_new
    (account_id, addressbook_href, webhook_token, transport, expires_at)
SELECT ca.account_id, p.addressbook_href, p.webhook_token, p.transport, p.expires_at
FROM contact_push_subscriptions p
JOIN (
    SELECT account_id, user_id
    FROM caldav_accounts
    WHERE account_id IN (
        SELECT account_id
        FROM caldav_accounts ca2
        WHERE ca2.user_id = caldav_accounts.user_id
        ORDER BY sort_order, account_id
        LIMIT 1
    )
) ca ON ca.user_id = p.user_id;
DROP TABLE contact_push_subscriptions;
ALTER TABLE contact_push_subscriptions_new RENAME TO contact_push_subscriptions;

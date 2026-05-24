ALTER TABLE cached_messages ADD COLUMN to_idx TEXT;

UPDATE cached_messages
SET to_idx = (
    SELECT group_concat(value, ' ')
    FROM json_each(to_addrs)
)
WHERE to_addrs IS NOT NULL;

DROP TRIGGER IF EXISTS messages_fts_ai;
DROP TRIGGER IF EXISTS messages_fts_ad;
DROP TRIGGER IF EXISTS messages_fts_au;
DROP TABLE IF EXISTS messages_fts;

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject_idx,
    body_idx,
    from_idx,
    to_idx,
    content='cached_messages',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_ai AFTER INSERT ON cached_messages BEGIN
    INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx, to_idx)
    VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx, new.to_idx);
END;

CREATE TRIGGER messages_fts_ad AFTER DELETE ON cached_messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx, to_idx)
    VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx, old.to_idx);
END;

CREATE TRIGGER messages_fts_au AFTER UPDATE ON cached_messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx, to_idx)
    VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx, old.to_idx);
    INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx, to_idx)
    VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx, new.to_idx);
END;

INSERT INTO messages_fts(messages_fts) VALUES('rebuild');

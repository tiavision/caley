-- Add per-row index columns so the FTS content can differ from display text
-- (e.g. stemmed tokens for de/en modes, raw text for default/trigram).
ALTER TABLE cached_events ADD COLUMN summary_idx     TEXT NOT NULL DEFAULT '';
ALTER TABLE cached_events ADD COLUMN description_idx TEXT NOT NULL DEFAULT '';
ALTER TABLE cached_events ADD COLUMN location_idx    TEXT NOT NULL DEFAULT '';

-- Populate from existing data (default mode = no stemming)
UPDATE cached_events SET
    summary_idx     = COALESCE(summary, ''),
    description_idx = COALESCE(description, ''),
    location_idx    = COALESCE(location, '');

-- Rebuild FTS around the new _idx columns with unicode61 tokenizer (baseline)
DROP TRIGGER IF EXISTS events_fts_ai;
DROP TRIGGER IF EXISTS events_fts_ad;
DROP TRIGGER IF EXISTS events_fts_au;
DROP TABLE IF EXISTS events_fts;

CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    summary_idx,
    description_idx,
    location_idx,
    content='cached_events',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

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

INSERT INTO events_fts(events_fts) VALUES('rebuild');

CREATE TABLE IF NOT EXISTS app_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO app_config(key, value) VALUES('search_mode', 'default');

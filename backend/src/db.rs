use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

pub struct UserSettings {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
}

pub struct CachedCalendar {
    pub href: String,
    pub name: String,
    pub color: Option<String>,
}

pub type Db = Mutex<Connection>;

fn encrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::encrypt(value, key)
}

fn decrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::decrypt(value, key)
}

pub fn init_db(path: &str) -> anyhow::Result<Connection> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
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
        );",
    )?;
    Ok(conn)
}

pub fn get_deactivated_calendars(db: &Connection, user_id: &str, key: &[u8; 32]) -> anyhow::Result<HashSet<String>> {
    let mut stmt = db.prepare(
        "SELECT calendar_href FROM deactivated_calendars WHERE user_id = ?1",
    )?;
    let rows = stmt.query_map(params![user_id], |row| row.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for row in rows {
        let enc_href = row?;
        set.insert(decrypt_field(&enc_href, key)?);
    }
    Ok(set)
}

pub fn deactivate_calendar(db: &Connection, user_id: &str, calendar_href: &str, key: &[u8; 32]) -> anyhow::Result<()> {
    let enc_href = encrypt_field(calendar_href, key)?;
    db.execute(
        "INSERT OR IGNORE INTO deactivated_calendars (user_id, calendar_href) VALUES (?1, ?2)",
        params![user_id, enc_href],
    )?;
    Ok(())
}

pub fn activate_calendar(db: &Connection, user_id: &str, calendar_href: &str, key: &[u8; 32]) -> anyhow::Result<()> {
    let mut stmt = db.prepare(
        "SELECT rowid, calendar_href FROM deactivated_calendars WHERE user_id = ?1",
    )?;
    let rows = stmt.query_map(params![user_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (rowid, enc_href) = row?;
        if decrypt_field(&enc_href, key)? == calendar_href {
            db.execute(
                "DELETE FROM deactivated_calendars WHERE rowid = ?1",
                params![rowid],
            )?;
            break;
        }
    }
    Ok(())
}

pub fn get_settings(db: &Connection, user_id: &str, key: &[u8; 32]) -> anyhow::Result<Option<UserSettings>> {
    let mut stmt = db.prepare(
        "SELECT caldav_base_url, caldav_username, caldav_password FROM user_settings WHERE user_id = ?1",
    )?;
    let result = stmt.query_row(params![user_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    });
    match result {
        Ok((enc_url, enc_user, enc_pass)) => {
            let caldav_base_url = decrypt_field(&enc_url, key)?;
            let caldav_username = decrypt_field(&enc_user, key)?;
            let caldav_password = decrypt_field(&enc_pass, key)?;
            Ok(Some(UserSettings {
                caldav_base_url,
                caldav_username,
                caldav_password,
            }))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn upsert_settings(
    db: &Connection,
    user_id: &str,
    settings: &UserSettings,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let enc_url = encrypt_field(&settings.caldav_base_url, key)?;
    let enc_user = encrypt_field(&settings.caldav_username, key)?;
    let enc_pass = encrypt_field(&settings.caldav_password, key)?;
    db.execute(
        "INSERT INTO user_settings (user_id, caldav_base_url, caldav_username, caldav_password)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
        params![user_id, enc_url, enc_user, enc_pass],
    )?;
    Ok(())
}

/// Wipe all cached data for a user — called when CalDAV credentials change.
pub fn clear_user_cache(db: &Connection, user_id: &str) -> anyhow::Result<()> {
    db.execute("DELETE FROM cached_calendars WHERE user_id = ?1", params![user_id])?;
    db.execute("DELETE FROM cached_events WHERE user_id = ?1", params![user_id])?;
    db.execute("DELETE FROM push_subscriptions WHERE user_id = ?1", params![user_id])?;
    Ok(())
}

// ── Calendar cache ────────────────────────────────────────────────────────────

pub fn upsert_cached_calendar(
    db: &Connection,
    user_id: &str,
    href: &str,
    name: &str,
    color: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    db.execute(
        "INSERT INTO cached_calendars (user_id, href, name, color, last_synced)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(user_id, href) DO UPDATE SET
            name = excluded.name,
            color = excluded.color,
            last_synced = excluded.last_synced",
        params![user_id, href, name, color, now],
    )?;
    Ok(())
}

pub fn get_cached_calendars(db: &Connection, user_id: &str) -> anyhow::Result<Vec<CachedCalendar>> {
    let mut stmt = db.prepare(
        "SELECT href, name, color FROM cached_calendars WHERE user_id = ?1 ORDER BY name",
    )?;
    let rows = stmt.query_map(params![user_id], |row| {
        Ok(CachedCalendar {
            href: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
        })
    })?;
    let mut cals = Vec::new();
    for row in rows {
        cals.push(row?);
    }
    Ok(cals)
}

pub fn get_calendar_sync_state(
    db: &Connection,
    user_id: &str,
    href: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let result = db.query_row(
        "SELECT ctag, sync_token FROM cached_calendars WHERE user_id = ?1 AND href = ?2",
        params![user_id, href],
        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
    );
    match result {
        Ok(pair) => Ok(pair),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
        Err(e) => Err(e.into()),
    }
}

pub fn update_calendar_sync_state(
    db: &Connection,
    user_id: &str,
    href: &str,
    ctag: Option<&str>,
    sync_token: Option<&str>,
) -> anyhow::Result<()> {
    db.execute(
        "UPDATE cached_calendars SET ctag = ?3, sync_token = ?4 WHERE user_id = ?1 AND href = ?2",
        params![user_id, href, ctag, sync_token],
    )?;
    Ok(())
}

// ── Event cache ───────────────────────────────────────────────────────────────

fn ical_dt_to_date_str(s: &str) -> String {
    // "20260519T090000Z" or "20260519T090000" → "2026-05-19"
    // "20260519" → "2026-05-19"
    let date_part = s.split('T').next().unwrap_or(s).trim();
    if date_part.len() >= 8 {
        format!("{}-{}-{}", &date_part[..4], &date_part[4..6], &date_part[6..8])
    } else {
        date_part.to_string()
    }
}

fn extract_ical_metadata(raw_ical: &str) -> (String, String, String, String, bool, bool) {
    // Returns (uid, summary, dtstart_date, dtend_date, is_all_day, is_recurring)
    let calendar = match ical::IcalParser::new(raw_ical.as_bytes()).next() {
        Some(Ok(c)) => c,
        _ => return (String::new(), String::new(), String::new(), String::new(), false, false),
    };

    let mut uid = String::new();
    let mut summary = String::new();
    let mut dtstart_date = String::new();
    let mut dtend_date = String::new();
    let mut is_all_day = false;
    let mut is_recurring = false;

    for event in &calendar.events {
        for prop in &event.properties {
            match prop.name.as_str() {
                "UID" => uid = prop.value.clone().unwrap_or_default(),
                "SUMMARY" => summary = prop.value.clone().unwrap_or_default(),
                "DTSTART" => {
                    if let Some(ref val) = prop.value {
                        dtstart_date = ical_dt_to_date_str(val);
                        // Check VALUE=DATE param
                        let value_is_date = prop.params.as_ref().map_or(false, |params| {
                            params.iter().any(|(k, v)| k == "VALUE" && v.iter().any(|s| s == "DATE"))
                        });
                        is_all_day = value_is_date || !val.contains('T');
                    }
                }
                "DTEND" | "DUE" => {
                    if let Some(ref val) = prop.value {
                        dtend_date = ical_dt_to_date_str(val);
                    }
                }
                "RRULE" | "RDATE" => is_recurring = true,
                _ => {}
            }
        }
        // Only process first VEVENT for metadata (override VEVENTs share same UID)
        if !uid.is_empty() {
            break;
        }
    }

    // If no dtend, use dtstart
    if dtend_date.is_empty() {
        dtend_date = dtstart_date.clone();
    }

    (uid, summary, dtstart_date, dtend_date, is_all_day, is_recurring)
}

pub fn upsert_cached_event_raw(
    db: &Connection,
    user_id: &str,
    calendar_href: &str,
    event_href: &str,
    etag: &str,
    raw_ical: &str,
) -> anyhow::Result<()> {
    let (uid, summary, dtstart, dtend, is_all_day, is_recurring) = extract_ical_metadata(raw_ical);
    if uid.is_empty() {
        return Ok(()); // skip malformed events
    }
    let now = chrono::Utc::now().timestamp();
    db.execute(
        "INSERT INTO cached_events
             (user_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, is_all_day, is_recurring, last_modified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(user_id, calendar_href, uid) DO UPDATE SET
             etag = excluded.etag,
             href = excluded.href,
             raw_ical = excluded.raw_ical,
             summary = excluded.summary,
             dtstart = excluded.dtstart,
             dtend = excluded.dtend,
             is_all_day = excluded.is_all_day,
             is_recurring = excluded.is_recurring,
             last_modified = excluded.last_modified",
        params![
            user_id, calendar_href, uid, etag, event_href, raw_ical,
            summary, dtstart, dtend,
            is_all_day as i32, is_recurring as i32,
            now
        ],
    )?;
    Ok(())
}

pub fn delete_event_by_href(
    db: &Connection,
    user_id: &str,
    calendar_href: &str,
    event_href: &str,
) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM cached_events WHERE user_id = ?1 AND calendar_href = ?2 AND href = ?3",
        params![user_id, calendar_href, event_href],
    )?;
    Ok(())
}

pub fn delete_cached_event(
    db: &Connection,
    user_id: &str,
    calendar_href: &str,
    uid: &str,
) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM cached_events WHERE user_id = ?1 AND calendar_href = ?2 AND uid = ?3",
        params![user_id, calendar_href, uid],
    )?;
    Ok(())
}

pub fn clear_calendar_events(
    db: &Connection,
    user_id: &str,
    calendar_href: &str,
) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM cached_events WHERE user_id = ?1 AND calendar_href = ?2",
        params![user_id, calendar_href],
    )?;
    Ok(())
}

/// Returns (calendar_href, raw_ical) pairs for events in the date range or recurring.
pub fn get_raw_events(
    db: &Connection,
    user_id: &str,
    calendar_hrefs: &[String],
    start: &str,
    end: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT calendar_href, raw_ical FROM cached_events
         WHERE user_id = ?
         AND (is_recurring = 1 OR (dtstart <= ? AND dtend >= ?))
         AND calendar_href IN ({})",
        placeholders
    );

    let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(user_id.to_owned()),
        Box::new(end.to_owned()),
        Box::new(start.to_owned()),
    ];
    for href in calendar_hrefs {
        all_params.push(Box::new(href.clone()));
    }
    let params_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = db.prepare(&query)?;
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Returns (calendar_href, dtstart_date) for events in the month range.
/// Used for the month grid dot display.
pub fn get_event_dates_for_month(
    db: &Connection,
    user_id: &str,
    calendar_hrefs: &[String],
    month_start: &str,
    month_end: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT calendar_href, dtstart FROM cached_events
         WHERE user_id = ?
         AND is_recurring = 0
         AND dtstart >= ?
         AND dtstart <= ?
         AND calendar_href IN ({})",
        placeholders
    );

    let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(user_id.to_owned()),
        Box::new(month_start.to_owned()),
        Box::new(month_end.to_owned()),
    ];
    for href in calendar_hrefs {
        all_params.push(Box::new(href.clone()));
    }
    let params_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = db.prepare(&query)?;
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// For month grid, also get recurring events so we can expand them.
pub fn get_recurring_raw_events(
    db: &Connection,
    user_id: &str,
    calendar_hrefs: &[String],
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "SELECT calendar_href, raw_ical FROM cached_events
         WHERE user_id = ?
         AND is_recurring = 1
         AND calendar_href IN ({})",
        placeholders
    );

    let mut all_params: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(user_id.to_owned())];
    for href in calendar_hrefs {
        all_params.push(Box::new(href.clone()));
    }
    let params_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = db.prepare(&query)?;
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

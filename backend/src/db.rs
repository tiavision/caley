use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;
use std::path::Path;

pub use sqlx::SqlitePool as Db;

pub struct UserSettings {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
}

#[derive(sqlx::FromRow)]
pub struct CachedCalendar {
    pub href: String,
    pub name: String,
    pub color: Option<String>,
}

fn encrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::encrypt(value, key)
}

fn decrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::decrypt(value, key)
}

pub async fn init_db(path: &str) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

pub async fn get_deactivated_calendars(
    pool: &SqlitePool,
    user_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<HashSet<String>> {
    let rows = sqlx::query("SELECT calendar_href FROM deactivated_calendars WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    let mut set = HashSet::new();
    for row in rows {
        let enc_href: String = row.get(0);
        set.insert(decrypt_field(&enc_href, key)?);
    }
    Ok(set)
}

pub async fn deactivate_calendar(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let enc_href = encrypt_field(calendar_href, key)?;
    sqlx::query(
        "INSERT INTO deactivated_calendars (user_id, calendar_href) VALUES (?, ?)
         ON CONFLICT(user_id, calendar_href) DO NOTHING",
    )
    .bind(user_id)
    .bind(enc_href)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn activate_calendar(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let rows =
        sqlx::query("SELECT rowid, calendar_href FROM deactivated_calendars WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    for row in rows {
        let rowid: i64 = row.get(0);
        let enc_href: String = row.get(1);
        if decrypt_field(&enc_href, key)? == calendar_href {
            sqlx::query("DELETE FROM deactivated_calendars WHERE rowid = ?")
                .bind(rowid)
                .execute(pool)
                .await?;
            break;
        }
    }
    Ok(())
}

pub async fn get_settings(
    pool: &SqlitePool,
    user_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<Option<UserSettings>> {
    let row = sqlx::query(
        "SELECT caldav_base_url, caldav_username, caldav_password FROM user_settings WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let enc_url: String = r.get(0);
            let enc_user: String = r.get(1);
            let enc_pass: String = r.get(2);
            Ok(Some(UserSettings {
                caldav_base_url: decrypt_field(&enc_url, key)?,
                caldav_username: decrypt_field(&enc_user, key)?,
                caldav_password: decrypt_field(&enc_pass, key)?,
            }))
        }
        None => Ok(None),
    }
}

pub async fn upsert_settings(
    pool: &SqlitePool,
    user_id: &str,
    settings: &UserSettings,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let enc_url = encrypt_field(&settings.caldav_base_url, key)?;
    let enc_user = encrypt_field(&settings.caldav_username, key)?;
    let enc_pass = encrypt_field(&settings.caldav_password, key)?;
    sqlx::query(
        "INSERT INTO user_settings (user_id, caldav_base_url, caldav_username, caldav_password)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
    )
    .bind(user_id)
    .bind(enc_url)
    .bind(enc_user)
    .bind(enc_pass)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_user_cache(pool: &SqlitePool, user_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM cached_calendars WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cached_events WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn upsert_push_subscription(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    webhook_token: &str,
    transport: &str,
    expires_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO push_subscriptions (user_id, calendar_href, webhook_token, transport, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(user_id, calendar_href) DO UPDATE SET
             webhook_token = excluded.webhook_token,
             transport = excluded.transport,
             expires_at = excluded.expires_at",
    )
    .bind(user_id)
    .bind(calendar_href)
    .bind(webhook_token)
    .bind(transport)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_push_subscription(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
) -> anyhow::Result<Option<(String, i64)>> {
    let row = sqlx::query(
        "SELECT webhook_token, expires_at FROM push_subscriptions WHERE user_id = ? AND calendar_href = ?",
    )
    .bind(user_id)
    .bind(calendar_href)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

pub async fn get_user_and_calendar_from_push_token(
    pool: &SqlitePool,
    token: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let row =
        sqlx::query("SELECT user_id, calendar_href FROM push_subscriptions WHERE webhook_token = ?")
            .bind(token)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

// ── Calendar cache ────────────────────────────────────────────────────────────

pub async fn upsert_cached_calendar(
    pool: &SqlitePool,
    user_id: &str,
    href: &str,
    name: &str,
    color: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO cached_calendars (user_id, href, name, color, last_synced)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(user_id, href) DO UPDATE SET
            name = excluded.name,
            color = excluded.color,
            last_synced = excluded.last_synced",
    )
    .bind(user_id)
    .bind(href)
    .bind(name)
    .bind(color)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_cached_calendars(
    pool: &SqlitePool,
    user_id: &str,
) -> anyhow::Result<Vec<CachedCalendar>> {
    let cals = sqlx::query_as::<_, CachedCalendar>(
        "SELECT href, name, color FROM cached_calendars WHERE user_id = ? ORDER BY name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(cals)
}

pub async fn get_calendar_sync_state(
    pool: &SqlitePool,
    user_id: &str,
    href: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let row = sqlx::query(
        "SELECT ctag, sync_token FROM cached_calendars WHERE user_id = ? AND href = ?",
    )
    .bind(user_id)
    .bind(href)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok((r.get(0), r.get(1))),
        None => Ok((None, None)),
    }
}

/// Remove cached data for calendars that no longer exist on the CalDAV server.
pub async fn remove_stale_calendars(
    pool: &SqlitePool,
    user_id: &str,
    fresh_hrefs: &HashSet<String>,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    let placeholders = fresh_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    if !fresh_hrefs.is_empty() {
        let sql = format!(
            "DELETE FROM push_subscriptions WHERE user_id = ? AND calendar_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(user_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_events WHERE user_id = ? AND calendar_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(user_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_calendars WHERE user_id = ? AND href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(user_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM push_subscriptions WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_events WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_calendars WHERE user_id = ?")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
    }

    // deactivated_calendars hrefs are encrypted with random nonces, so decrypt each to compare
    let rows = sqlx::query(
        "SELECT rowid, calendar_href FROM deactivated_calendars WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    for row in rows {
        let rowid: i64 = row.get(0);
        let enc_href: String = row.get(1);
        if let Ok(decrypted) = decrypt_field(&enc_href, key) {
            if !fresh_hrefs.contains(&decrypted) {
                sqlx::query("DELETE FROM deactivated_calendars WHERE rowid = ?")
                    .bind(rowid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;
    Ok(())
}

/// Delete all cached data for a single calendar. Used when a calendar returns 404.
pub async fn delete_calendar_cache(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = ? AND calendar_href = ?")
        .bind(user_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM cached_events WHERE user_id = ? AND calendar_href = ?")
        .bind(user_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    // deactivated_calendars hrefs are encrypted, so decrypt each to compare
    let rows = sqlx::query(
        "SELECT rowid, calendar_href FROM deactivated_calendars WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    for row in rows {
        let rowid: i64 = row.get(0);
        let enc_href: String = row.get(1);
        if let Ok(decrypted) = decrypt_field(&enc_href, key) {
            if decrypted == calendar_href {
                sqlx::query("DELETE FROM deactivated_calendars WHERE rowid = ?")
                    .bind(rowid)
                    .execute(&mut *tx)
                    .await?;
                break;
            }
        }
    }

    sqlx::query("DELETE FROM cached_calendars WHERE user_id = ? AND href = ?")
        .bind(user_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

// ── Event cache ───────────────────────────────────────────────────────────────

fn ical_dt_to_date_str(s: &str) -> String {
    let date_part = s.split('T').next().unwrap_or(s).trim();
    if date_part.len() >= 8 {
        format!("{}-{}-{}", &date_part[..4], &date_part[4..6], &date_part[6..8])
    } else {
        date_part.to_string()
    }
}

fn extract_ical_metadata(raw_ical: &str) -> (String, String, String, String, String, String, bool, bool) {
    let calendar = match ical::IcalParser::new(raw_ical.as_bytes()).next() {
        Some(Ok(c)) => c,
        _ => return (String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), false, false),
    };

    let mut uid = String::new();
    let mut summary = String::new();
    let mut dtstart_date = String::new();
    let mut dtend_date = String::new();
    let mut description = String::new();
    let mut location = String::new();
    let mut is_all_day = false;
    let mut is_recurring = false;

    for event in &calendar.events {
        for prop in &event.properties {
            match prop.name.as_str() {
                "UID" => uid = prop.value.clone().unwrap_or_default(),
                "SUMMARY" => summary = prop.value.clone().unwrap_or_default(),
                "DESCRIPTION" => description = prop.value.clone().unwrap_or_default(),
                "LOCATION" => location = prop.value.clone().unwrap_or_default(),
                "DTSTART" => {
                    if let Some(ref val) = prop.value {
                        dtstart_date = ical_dt_to_date_str(val);
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
        if !uid.is_empty() {
            break;
        }
    }

    if dtend_date.is_empty() {
        dtend_date = dtstart_date.clone();
    }

    (uid, summary, dtstart_date, dtend_date, description, location, is_all_day, is_recurring)
}

pub async fn upsert_cached_event_raw(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    event_href: &str,
    etag: &str,
    raw_ical: &str,
) -> anyhow::Result<()> {
    let (uid, summary, dtstart, dtend, description, location, is_all_day, is_recurring) =
        extract_ical_metadata(raw_ical);
    if uid.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO cached_events
             (user_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, calendar_href, uid) DO UPDATE SET
             etag = excluded.etag,
             href = excluded.href,
             raw_ical = excluded.raw_ical,
             summary = excluded.summary,
             dtstart = excluded.dtstart,
             dtend = excluded.dtend,
             description = excluded.description,
             location = excluded.location,
             is_all_day = excluded.is_all_day,
             is_recurring = excluded.is_recurring,
             last_modified = excluded.last_modified",
    )
    .bind(user_id)
    .bind(calendar_href)
    .bind(&uid)
    .bind(etag)
    .bind(event_href)
    .bind(raw_ical)
    .bind(&summary)
    .bind(&dtstart)
    .bind(&dtend)
    .bind(&description)
    .bind(&location)
    .bind(is_all_day as i32)
    .bind(is_recurring as i32)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_event_by_href(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    event_href: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM cached_events WHERE user_id = ? AND calendar_href = ? AND href = ?",
    )
    .bind(user_id)
    .bind(calendar_href)
    .bind(event_href)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_cached_event(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    uid: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM cached_events WHERE user_id = ? AND calendar_href = ? AND uid = ?",
    )
    .bind(user_id)
    .bind(calendar_href)
    .bind(uid)
    .execute(pool)
    .await?;
    Ok(())
}

pub struct EventRef {
    pub uid: String,
    pub calendar_href: String,
}

pub struct SyncApplyResult {
    pub changed: Vec<EventRef>,
    pub deleted: Vec<EventRef>,
}

/// Apply incremental sync results atomically: upsert changed events, delete removed
/// events, and update the sync token — all in a single transaction.
pub async fn apply_incremental_sync(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    changed: Vec<(String, String, String)>,
    deleted: Vec<String>,
    new_token: &str,
    stored_ctag: Option<&str>,
) -> anyhow::Result<SyncApplyResult> {
    let mut tx = pool.begin().await?;
    let now = chrono::Utc::now().timestamp();

    let mut changed_result: Vec<EventRef> = Vec::new();
    for (href, etag, raw_ical) in changed {
        let (uid, summary, dtstart, dtend, description, location, is_all_day, is_recurring) =
            extract_ical_metadata(&raw_ical);
        if uid.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO cached_events
                 (user_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, calendar_href, uid) DO UPDATE SET
                 etag = excluded.etag,
                 href = excluded.href,
                 raw_ical = excluded.raw_ical,
                 summary = excluded.summary,
                 dtstart = excluded.dtstart,
                 dtend = excluded.dtend,
                 description = excluded.description,
                 location = excluded.location,
                 is_all_day = excluded.is_all_day,
                 is_recurring = excluded.is_recurring,
                 last_modified = excluded.last_modified",
        )
        .bind(user_id)
        .bind(calendar_href)
        .bind(&uid)
        .bind(&etag)
        .bind(&href)
        .bind(&raw_ical)
        .bind(&summary)
        .bind(&dtstart)
        .bind(&dtend)
        .bind(&description)
        .bind(&location)
        .bind(is_all_day as i32)
        .bind(is_recurring as i32)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        changed_result.push(EventRef { uid, calendar_href: calendar_href.to_string() });
    }

    let mut deleted_result: Vec<EventRef> = Vec::new();
    for href in deleted {
        let uid: Option<String> = sqlx::query_scalar(
            "SELECT uid FROM cached_events WHERE user_id = ? AND calendar_href = ? AND href = ?",
        )
        .bind(user_id)
        .bind(calendar_href)
        .bind(&href)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(uid) = uid {
            deleted_result.push(EventRef { uid, calendar_href: calendar_href.to_string() });
        }
        sqlx::query(
            "DELETE FROM cached_events WHERE user_id = ? AND calendar_href = ? AND href = ?",
        )
        .bind(user_id)
        .bind(calendar_href)
        .bind(&href)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE cached_calendars SET ctag = ?, sync_token = ? WHERE user_id = ? AND href = ?",
    )
    .bind(stored_ctag)
    .bind(new_token)
    .bind(user_id)
    .bind(calendar_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(SyncApplyResult { changed: changed_result, deleted: deleted_result })
}

/// Replace all cached events for a calendar atomically: clear, insert all, update
/// sync state — all in a single transaction.
pub async fn replace_calendar_events(
    pool: &SqlitePool,
    user_id: &str,
    calendar_href: &str,
    all_raw: Vec<(String, String, String)>,
    new_ctag: Option<&str>,
    new_token: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let now = chrono::Utc::now().timestamp();

    sqlx::query("DELETE FROM cached_events WHERE user_id = ? AND calendar_href = ?")
        .bind(user_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    for (href, etag, raw_ical) in all_raw {
        let (uid, summary, dtstart, dtend, description, location, is_all_day, is_recurring) =
            extract_ical_metadata(&raw_ical);
        if uid.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO cached_events
                 (user_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, calendar_href, uid) DO UPDATE SET
                 etag = excluded.etag,
                 href = excluded.href,
                 raw_ical = excluded.raw_ical,
                 summary = excluded.summary,
                 dtstart = excluded.dtstart,
                 dtend = excluded.dtend,
                 description = excluded.description,
                 location = excluded.location,
                 is_all_day = excluded.is_all_day,
                 is_recurring = excluded.is_recurring,
                 last_modified = excluded.last_modified",
        )
        .bind(user_id)
        .bind(calendar_href)
        .bind(&uid)
        .bind(&etag)
        .bind(&href)
        .bind(&raw_ical)
        .bind(&summary)
        .bind(&dtstart)
        .bind(&dtend)
        .bind(&description)
        .bind(&location)
        .bind(is_all_day as i32)
        .bind(is_recurring as i32)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE cached_calendars SET ctag = ?, sync_token = ? WHERE user_id = ? AND href = ?",
    )
    .bind(new_ctag)
    .bind(new_token)
    .bind(user_id)
    .bind(calendar_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn delete_user(pool: &SqlitePool, user_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_settings WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM deactivated_calendars WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cached_calendars WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM cached_events WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns (calendar_href, raw_ical) pairs for events in the date range or recurring.
pub async fn get_raw_events(
    pool: &SqlitePool,
    user_id: &str,
    calendar_hrefs: &[String],
    start: &str,
    end: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT calendar_href, raw_ical FROM cached_events
         WHERE user_id = ?
         AND (is_recurring = 1 OR (dtstart <= ? AND dtend >= ?))
         AND calendar_href IN ({})",
        placeholders
    );

    let mut q = sqlx::query(&sql).bind(user_id).bind(end).bind(start);
    for href in calendar_hrefs {
        q = q.bind(href.as_str());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Returns (calendar_href, dtstart_date) for non-recurring events in the month range.
pub async fn get_event_dates_for_month(
    pool: &SqlitePool,
    user_id: &str,
    calendar_hrefs: &[String],
    month_start: &str,
    month_end: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT calendar_href, dtstart FROM cached_events
         WHERE user_id = ?
         AND is_recurring = 0
         AND dtstart >= ?
         AND dtstart <= ?
         AND calendar_href IN ({})",
        placeholders
    );

    let mut q = sqlx::query(&sql).bind(user_id).bind(month_start).bind(month_end);
    for href in calendar_hrefs {
        q = q.bind(href.as_str());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Returns (calendar_href, raw_ical) for all recurring events in the given calendars.
pub async fn get_recurring_raw_events(
    pool: &SqlitePool,
    user_id: &str,
    calendar_hrefs: &[String],
) -> anyhow::Result<Vec<(String, String)>> {
    if calendar_hrefs.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT calendar_href, raw_ical FROM cached_events
         WHERE user_id = ?
         AND is_recurring = 1
         AND calendar_href IN ({})",
        placeholders
    );

    let mut q = sqlx::query(&sql).bind(user_id);
    for href in calendar_hrefs {
        q = q.bind(href.as_str());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
}

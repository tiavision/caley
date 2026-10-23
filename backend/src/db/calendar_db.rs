use sqlx::{AssertSqlSafe, Row, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

use crate::caldav::unescape_ical_text;
use crate::search::SearchMode;

#[allow(unused_imports)]
use chrono::Utc;

fn encrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::encrypt(value, key)
}

fn decrypt_field(value: &str, key: &[u8; 32]) -> anyhow::Result<String> {
    crate::crypto::decrypt(value, key)
}

#[derive(sqlx::FromRow)]
pub struct CachedCalendar {
    pub account_id: String,
    pub href: String,
    pub name: String,
    pub color: Option<String>,
}

pub struct CalDavAccount {
    pub account_id: String,
    pub display_name: Option<String>,
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub caldav_password: String,
    pub sort_order: i64,
}

// ── Calendar deactivation ─────────────────────────────────────────────────────

pub async fn get_deactivated_calendars(
    pool: &SqlitePool,
    account_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<HashSet<String>> {
    let rows = sqlx::query("SELECT calendar_href FROM deactivated_calendars WHERE account_id = ?")
        .bind(account_id)
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
    account_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let enc_href = encrypt_field(calendar_href, key)?;
    sqlx::query(
        "INSERT INTO deactivated_calendars (account_id, calendar_href) VALUES (?, ?)
         ON CONFLICT(account_id, calendar_href) DO NOTHING",
    )
    .bind(account_id)
    .bind(enc_href)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn activate_calendar(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let rows =
        sqlx::query("SELECT rowid, calendar_href FROM deactivated_calendars WHERE account_id = ?")
            .bind(account_id)
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

// ── CalDAV accounts ───────────────────────────────────────────────────────────

pub async fn list_caldav_accounts(
    pool: &SqlitePool,
    user_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<Vec<CalDavAccount>> {
    let rows = sqlx::query(
        "SELECT account_id, display_name, caldav_base_url, caldav_username, caldav_password, sort_order
         FROM caldav_accounts
         WHERE user_id = ?
         ORDER BY sort_order, account_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(CalDavAccount {
                account_id: r.get(0),
                display_name: r.get(1),
                caldav_base_url: decrypt_field(&r.get::<String, _>(2), key)?,
                caldav_username: decrypt_field(&r.get::<String, _>(3), key)?,
                caldav_password: decrypt_field(&r.get::<String, _>(4), key)?,
                sort_order: r.get(5),
            })
        })
        .collect()
}

pub async fn get_caldav_account(
    pool: &SqlitePool,
    user_id: &str,
    account_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<Option<CalDavAccount>> {
    let row = sqlx::query(
        "SELECT account_id, display_name, caldav_base_url, caldav_username, caldav_password, sort_order
         FROM caldav_accounts
         WHERE user_id = ? AND account_id = ?",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;

    row.map(|r| {
        Ok(CalDavAccount {
            account_id: r.get(0),
            display_name: r.get(1),
            caldav_base_url: decrypt_field(&r.get::<String, _>(2), key)?,
            caldav_username: decrypt_field(&r.get::<String, _>(3), key)?,
            caldav_password: decrypt_field(&r.get::<String, _>(4), key)?,
            sort_order: r.get(5),
        })
    })
    .transpose()
}

pub async fn upsert_caldav_account(
    pool: &SqlitePool,
    user_id: &str,
    account_id: Option<&str>,
    display_name: Option<&str>,
    settings: &super::UserSettings,
    key: &[u8; 32],
) -> anyhow::Result<String> {
    let account_id = account_id
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{user_id}:{}", Uuid::new_v4()));
    let sort_order: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sort_order) + 1, 0) FROM caldav_accounts WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    let enc_url = encrypt_field(&settings.caldav_base_url, key)?;
    let enc_user = encrypt_field(&settings.caldav_username, key)?;
    let enc_pass = encrypt_field(&settings.caldav_password, key)?;
    sqlx::query(
        "INSERT INTO caldav_accounts
            (account_id, user_id, display_name, caldav_base_url, caldav_username, caldav_password, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET
            display_name = excluded.display_name,
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
    )
    .bind(&account_id)
    .bind(user_id)
    .bind(display_name)
    .bind(enc_url)
    .bind(enc_user)
    .bind(enc_pass)
    .bind(sort_order)
    .execute(pool)
    .await?;
    Ok(account_id)
}

pub async fn delete_caldav_account(
    pool: &SqlitePool,
    user_id: &str,
    account_id: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM caldav_accounts WHERE user_id = ? AND account_id = ?")
        .bind(user_id)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_user_id_for_caldav_account(
    pool: &SqlitePool,
    account_id: &str,
) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar("SELECT user_id FROM caldav_accounts WHERE account_id = ?")
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn get_caldav_account_password(
    pool: &SqlitePool,
    user_id: &str,
    account_id: &str,
    key: &[u8; 32],
) -> anyhow::Result<Option<String>> {
    let row = sqlx::query(
        "SELECT caldav_password FROM caldav_accounts WHERE user_id = ? AND account_id = ?",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => {
            let enc: String = r.get(0);
            Ok(Some(decrypt_field(&enc, key)?))
        }
        None => Ok(None),
    }
}

// ── Push subscriptions ────────────────────────────────────────────────────────

pub async fn upsert_push_subscription(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    webhook_token: &str,
    transport: &str,
    expires_at: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO push_subscriptions (account_id, calendar_href, webhook_token, transport, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, calendar_href) DO UPDATE SET
             webhook_token = excluded.webhook_token,
             transport = excluded.transport,
             expires_at = excluded.expires_at",
    )
    .bind(account_id)
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
    account_id: &str,
    calendar_href: &str,
) -> anyhow::Result<Option<(String, i64)>> {
    let row = sqlx::query(
        "SELECT webhook_token, expires_at FROM push_subscriptions WHERE account_id = ? AND calendar_href = ?",
    )
    .bind(account_id)
    .bind(calendar_href)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1))))
}

pub async fn get_user_and_calendar_from_push_token(
    pool: &SqlitePool,
    token: &str,
) -> anyhow::Result<Option<(String, String, String)>> {
    let row = sqlx::query(
        "SELECT ca.user_id, p.account_id, p.calendar_href
         FROM push_subscriptions p
         JOIN caldav_accounts ca ON ca.account_id = p.account_id
         WHERE p.webhook_token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
}

// ── Calendar cache ────────────────────────────────────────────────────────────

pub async fn upsert_cached_calendar(
    pool: &SqlitePool,
    account_id: &str,
    href: &str,
    name: &str,
    color: Option<&str>,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO cached_calendars (account_id, href, name, color, last_synced)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id, href) DO UPDATE SET
            name = excluded.name,
            color = excluded.color,
            last_synced = excluded.last_synced",
    )
    .bind(account_id)
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
        "SELECT c.account_id, c.href, c.name, c.color
         FROM cached_calendars c
         JOIN caldav_accounts ca ON ca.account_id = c.account_id
         WHERE ca.user_id = ?
         ORDER BY ca.sort_order, c.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(cals)
}

pub async fn get_calendar_sync_state(
    pool: &SqlitePool,
    account_id: &str,
    href: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let row = sqlx::query(
        "SELECT ctag, sync_token FROM cached_calendars WHERE account_id = ? AND href = ?",
    )
    .bind(account_id)
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
    account_id: &str,
    fresh_hrefs: &HashSet<String>,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    let placeholders = fresh_hrefs
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");

    if !fresh_hrefs.is_empty() {
        let sql = format!(
            "DELETE FROM push_subscriptions WHERE account_id = ? AND calendar_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_events WHERE account_id = ? AND calendar_href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;

        let sql = format!(
            "DELETE FROM cached_calendars WHERE account_id = ? AND href NOT IN ({placeholders})"
        );
        let mut q = sqlx::query(AssertSqlSafe(sql)).bind(account_id);
        for h in fresh_hrefs {
            q = q.bind(h.as_str());
        }
        q.execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM push_subscriptions WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_events WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM cached_calendars WHERE account_id = ?")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
    }

    // deactivated_calendars hrefs are encrypted with random nonces, so decrypt each to compare
    let rows =
        sqlx::query("SELECT rowid, calendar_href FROM deactivated_calendars WHERE account_id = ?")
            .bind(account_id)
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
    account_id: &str,
    calendar_href: &str,
    key: &[u8; 32],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM push_subscriptions WHERE account_id = ? AND calendar_href = ?")
        .bind(account_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM cached_events WHERE account_id = ? AND calendar_href = ?")
        .bind(account_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    // deactivated_calendars hrefs are encrypted, so decrypt each to compare
    let rows =
        sqlx::query("SELECT rowid, calendar_href FROM deactivated_calendars WHERE account_id = ?")
            .bind(account_id)
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

    sqlx::query("DELETE FROM cached_calendars WHERE account_id = ? AND href = ?")
        .bind(account_id)
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
        format!(
            "{}-{}-{}",
            &date_part[..4],
            &date_part[4..6],
            &date_part[6..8]
        )
    } else {
        date_part.to_string()
    }
}

fn extract_ical_metadata(
    raw_ical: &str,
) -> (String, String, String, String, String, String, bool, bool) {
    let calendar = match ical::IcalParser::new(raw_ical.as_bytes()).next() {
        Some(Ok(c)) => c,
        _ => {
            return (
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                false,
                false,
            );
        }
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
                "SUMMARY" => {
                    summary = prop
                        .value
                        .as_deref()
                        .map(unescape_ical_text)
                        .unwrap_or_default()
                }
                "DESCRIPTION" => {
                    description = prop
                        .value
                        .as_deref()
                        .map(unescape_ical_text)
                        .unwrap_or_default()
                }
                "LOCATION" => {
                    location = prop
                        .value
                        .as_deref()
                        .map(unescape_ical_text)
                        .unwrap_or_default()
                }
                "DTSTART" => {
                    if let Some(ref val) = prop.value {
                        dtstart_date = ical_dt_to_date_str(val);
                        let value_is_date = prop.params.as_ref().map_or(false, |params| {
                            params
                                .iter()
                                .any(|(k, v)| k == "VALUE" && v.iter().any(|s| s == "DATE"))
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

    (
        uid,
        summary,
        dtstart_date,
        dtend_date,
        description,
        location,
        is_all_day,
        is_recurring,
    )
}

pub async fn upsert_cached_event_raw(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    event_href: &str,
    etag: &str,
    raw_ical: &str,
    mode: &SearchMode,
) -> anyhow::Result<()> {
    let (uid, summary, dtstart, dtend, description, location, is_all_day, is_recurring) =
        extract_ical_metadata(raw_ical);
    if uid.is_empty() {
        return Ok(());
    }
    let summary_idx = mode.normalize_text(&summary);
    let description_idx = mode.normalize_text(&description);
    let location_idx = mode.normalize_text(&location);
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        "INSERT INTO cached_events
             (account_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified, summary_idx, description_idx, location_idx)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(account_id, calendar_href, uid) DO UPDATE SET
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
             last_modified = excluded.last_modified,
             summary_idx = excluded.summary_idx,
             description_idx = excluded.description_idx,
             location_idx = excluded.location_idx",
    )
    .bind(account_id)
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
    .bind(&summary_idx)
    .bind(&description_idx)
    .bind(&location_idx)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_event_by_href(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    event_href: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM cached_events WHERE account_id = ? AND calendar_href = ? AND href = ?",
    )
    .bind(account_id)
    .bind(calendar_href)
    .bind(event_href)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_cached_event(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    uid: &str,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM cached_events WHERE account_id = ? AND calendar_href = ? AND uid = ?")
        .bind(account_id)
        .bind(calendar_href)
        .bind(uid)
        .execute(pool)
        .await?;
    Ok(())
}

pub struct EventRef {
    pub account_id: String,
    pub uid: String,
    pub calendar_href: String,
}

pub struct SyncApplyResult {
    pub changed: Vec<EventRef>,
    pub deleted: Vec<EventRef>,
}

/// Apply incremental sync results atomically.
pub async fn apply_incremental_sync(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    changed: Vec<(String, String, String)>,
    deleted: Vec<String>,
    new_token: &str,
    stored_ctag: Option<&str>,
    mode: &SearchMode,
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
        let summary_idx = mode.normalize_text(&summary);
        let description_idx = mode.normalize_text(&description);
        let location_idx = mode.normalize_text(&location);
        sqlx::query(
            "INSERT INTO cached_events
                 (account_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified, summary_idx, description_idx, location_idx)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, calendar_href, uid) DO UPDATE SET
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
                 last_modified = excluded.last_modified,
                 summary_idx = excluded.summary_idx,
                 description_idx = excluded.description_idx,
                 location_idx = excluded.location_idx",
        )
        .bind(account_id)
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
        .bind(&summary_idx)
        .bind(&description_idx)
        .bind(&location_idx)
        .execute(&mut *tx)
        .await?;
        changed_result.push(EventRef {
            account_id: account_id.to_string(),
            uid,
            calendar_href: calendar_href.to_string(),
        });
    }

    let mut deleted_result: Vec<EventRef> = Vec::new();
    for href in deleted {
        let uid: Option<String> = sqlx::query_scalar(
            "SELECT uid FROM cached_events WHERE account_id = ? AND calendar_href = ? AND href = ?",
        )
        .bind(account_id)
        .bind(calendar_href)
        .bind(&href)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(uid) = uid {
            deleted_result.push(EventRef {
                account_id: account_id.to_string(),
                uid,
                calendar_href: calendar_href.to_string(),
            });
        }
        sqlx::query(
            "DELETE FROM cached_events WHERE account_id = ? AND calendar_href = ? AND href = ?",
        )
        .bind(account_id)
        .bind(calendar_href)
        .bind(&href)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE cached_calendars SET ctag = ?, sync_token = ? WHERE account_id = ? AND href = ?",
    )
    .bind(stored_ctag)
    .bind(new_token)
    .bind(account_id)
    .bind(calendar_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(SyncApplyResult {
        changed: changed_result,
        deleted: deleted_result,
    })
}

/// Replace all cached events for a calendar atomically.
pub async fn replace_calendar_events(
    pool: &SqlitePool,
    account_id: &str,
    calendar_href: &str,
    all_raw: Vec<(String, String, String)>,
    new_ctag: Option<&str>,
    new_token: Option<&str>,
    mode: &SearchMode,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let now = chrono::Utc::now().timestamp();

    sqlx::query("DELETE FROM cached_events WHERE account_id = ? AND calendar_href = ?")
        .bind(account_id)
        .bind(calendar_href)
        .execute(&mut *tx)
        .await?;

    for (href, etag, raw_ical) in all_raw {
        let (uid, summary, dtstart, dtend, description, location, is_all_day, is_recurring) =
            extract_ical_metadata(&raw_ical);
        if uid.is_empty() {
            continue;
        }
        let summary_idx = mode.normalize_text(&summary);
        let description_idx = mode.normalize_text(&description);
        let location_idx = mode.normalize_text(&location);
        sqlx::query(
            "INSERT INTO cached_events
                 (account_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend, description, location, is_all_day, is_recurring, last_modified, summary_idx, description_idx, location_idx)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_id, calendar_href, uid) DO UPDATE SET
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
                 last_modified = excluded.last_modified,
                 summary_idx = excluded.summary_idx,
                 description_idx = excluded.description_idx,
                 location_idx = excluded.location_idx",
        )
        .bind(account_id)
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
        .bind(&summary_idx)
        .bind(&description_idx)
        .bind(&location_idx)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE cached_calendars SET ctag = ?, sync_token = ? WHERE account_id = ? AND href = ?",
    )
    .bind(new_ctag)
    .bind(new_token)
    .bind(account_id)
    .bind(calendar_href)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

// ── Event queries ─────────────────────────────────────────────────────────────

/// Returns (account_id, calendar_href, raw_ical) tuples for events in the date range or recurring.
pub async fn get_raw_events(
    pool: &SqlitePool,
    calendar_keys: &[(String, String)],
    start: &str,
    end: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    if calendar_keys.is_empty() {
        return Ok(vec![]);
    }

    let placeholders = calendar_keys
        .iter()
        .map(|_| "(?, ?)")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT account_id, calendar_href, raw_ical FROM cached_events
         WHERE
         AND (is_recurring = 1 OR (dtstart <= ? AND dtend >= ?))
         AND (account_id, calendar_href) IN ({})",
        placeholders
    )
    .replacen("WHERE\n         AND", "WHERE", 1);

    let mut q = sqlx::query(AssertSqlSafe(sql)).bind(end).bind(start);
    for (account_id, href) in calendar_keys {
        q = q.bind(account_id.as_str()).bind(href.as_str());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect())
}

// ── FTS schema ────────────────────────────────────────────────────────────────

/// Check whether the active search mode matches the stored one and rebuild the FTS
/// index if they differ.
pub async fn ensure_fts_schema(pool: &SqlitePool, mode: &SearchMode) -> anyhow::Result<()> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_config WHERE key = 'search_mode'")
            .fetch_optional(pool)
            .await?;

    let mode_str = mode.as_str();
    if stored.as_deref() == Some(mode_str.as_str()) {
        return Ok(());
    }

    println!("[search] mode changed to '{mode_str}', rebuilding FTS index…");

    let old_is_trigram = stored.as_deref() == Some("trigram");
    let new_is_trigram = mode_str == "trigram";

    if old_is_trigram != new_is_trigram {
        let tokenizer = mode.fts_tokenizer();
        sqlx::query("DROP TRIGGER IF EXISTS events_fts_ai")
            .execute(pool)
            .await?;
        sqlx::query("DROP TRIGGER IF EXISTS events_fts_ad")
            .execute(pool)
            .await?;
        sqlx::query("DROP TRIGGER IF EXISTS events_fts_au")
            .execute(pool)
            .await?;
        sqlx::query("DROP TABLE IF EXISTS events_fts")
            .execute(pool)
            .await?;

        sqlx::query(AssertSqlSafe(format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
                summary_idx, description_idx, location_idx,
                content='cached_events',
                content_rowid='rowid',
                tokenize='{tokenizer}'
            )"
        )))
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS events_fts_ai AFTER INSERT ON cached_events BEGIN
                INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
                VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
            END",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS events_fts_ad AFTER DELETE ON cached_events BEGIN
                INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
                VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
            END",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS events_fts_au AFTER UPDATE ON cached_events BEGIN
                INSERT INTO events_fts(events_fts, rowid, summary_idx, description_idx, location_idx)
                VALUES ('delete', old.rowid, old.summary_idx, old.description_idx, old.location_idx);
                INSERT INTO events_fts(rowid, summary_idx, description_idx, location_idx)
                VALUES (new.rowid, new.summary_idx, new.description_idx, new.location_idx);
            END",
        )
        .execute(pool)
        .await?;

        sqlx::query("DROP TRIGGER IF EXISTS messages_fts_ai")
            .execute(pool)
            .await?;
        sqlx::query("DROP TRIGGER IF EXISTS messages_fts_ad")
            .execute(pool)
            .await?;
        sqlx::query("DROP TRIGGER IF EXISTS messages_fts_au")
            .execute(pool)
            .await?;
        sqlx::query("DROP TABLE IF EXISTS messages_fts")
            .execute(pool)
            .await?;

        sqlx::query(AssertSqlSafe(format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                subject_idx, body_idx, from_idx, to_idx,
                content='cached_messages',
                content_rowid='rowid',
                tokenize='{tokenizer}'
            )"
        )))
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON cached_messages BEGIN
                INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx, to_idx)
                VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx, new.to_idx);
            END",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON cached_messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx, to_idx)
                VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx, old.to_idx);
            END",
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON cached_messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, subject_idx, body_idx, from_idx, to_idx)
                VALUES ('delete', old.rowid, old.subject_idx, old.body_idx, old.from_idx, old.to_idx);
                INSERT INTO messages_fts(rowid, subject_idx, body_idx, from_idx, to_idx)
                VALUES (new.rowid, new.subject_idx, new.body_idx, new.from_idx, new.to_idx);
            END",
        )
        .execute(pool)
        .await?;
    }

    let rows = sqlx::query("SELECT rowid, summary, description, location FROM cached_events")
        .fetch_all(pool)
        .await?;

    for row in &rows {
        let rowid: i64 = row.get(0);
        let summary: String = row.get::<Option<String>, _>(1).unwrap_or_default();
        let description: String = row.get::<Option<String>, _>(2).unwrap_or_default();
        let location: String = row.get::<Option<String>, _>(3).unwrap_or_default();

        sqlx::query(
            "UPDATE cached_events SET summary_idx = ?, description_idx = ?, location_idx = ? WHERE rowid = ?",
        )
        .bind(mode.normalize_text(&summary))
        .bind(mode.normalize_text(&description))
        .bind(mode.normalize_text(&location))
        .bind(rowid)
        .execute(pool)
        .await?;
    }

    sqlx::query("INSERT INTO events_fts(events_fts) VALUES('rebuild')")
        .execute(pool)
        .await?;

    let msg_rows =
        sqlx::query("SELECT rowid, subject, from_addr, to_addrs, body_idx FROM cached_messages")
            .fetch_all(pool)
            .await?;

    for row in &msg_rows {
        let rowid: i64 = row.get(0);
        let subject: String = row.get::<Option<String>, _>(1).unwrap_or_default();
        let from_addr: String = row.get::<Option<String>, _>(2).unwrap_or_default();
        let to_addrs_json: Option<String> = row.get(3);
        let body_idx: String = row.get::<Option<String>, _>(4).unwrap_or_default();

        let to_plain = to_addrs_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
            .map(|v| v.join(" "))
            .unwrap_or_default();

        sqlx::query(
            "UPDATE cached_messages
             SET subject_idx = ?, from_idx = ?, to_idx = ?, body_idx = ?
             WHERE rowid = ?",
        )
        .bind(mode.normalize_text(&subject))
        .bind(mode.normalize_text(&from_addr))
        .bind(mode.normalize_text(&to_plain))
        .bind(body_idx)
        .bind(rowid)
        .execute(pool)
        .await?;
    }

    sqlx::query("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO app_config(key, value) VALUES('search_mode', ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(mode_str)
    .execute(pool)
    .await?;

    println!("[search] FTS rebuild complete.");
    Ok(())
}

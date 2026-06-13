use axum::{
    extract::{Multipart, Query, RawQuery, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use chrono::NaiveDate;
use url::form_urlencoded;

use crate::caldav::{CalDavClient, preview_events_from_raw_ical, split_vcalendar_into_events};
use crate::db;
use crate::email_db;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;
use sqlx::Row;

use super::{EventJson, build_event_json, require_account_client};

#[derive(serde::Serialize)]
struct ImportResult {
    imported: usize,
    errors: usize,
}

#[derive(serde::Serialize)]
struct ImportPreviewEvent {
    uid: String,
    summary: String,
    dtstart: String,
    dtend: String,
    is_all_day: bool,
    location: String,
}

async fn read_ics_multipart(mut multipart: Multipart) -> Result<String, Response> {
    let mut ics_content: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            match field.text().await {
                Ok(text) => ics_content = Some(text),
                Err(e) => {
                    return Err(
                        (StatusCode::BAD_REQUEST, format!("Failed to read file: {e}"))
                            .into_response(),
                    );
                }
            }
        }
    }

    ics_content.ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing ICS file").into_response())
}

pub async fn preview_ics_import(
    State(_state): State<crate::oidc::AppState>,
    _user: CurrentUser,
    multipart: Multipart,
) -> Response {
    let ics = match read_ics_multipart(multipart).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let events = split_vcalendar_into_events(&ics);
    if events.is_empty() {
        return (StatusCode::BAD_REQUEST, "No events found in ICS file").into_response();
    }

    let previews: Vec<ImportPreviewEvent> = events
        .into_iter()
        .filter_map(|(_, wrapped_ical)| {
            preview_events_from_raw_ical(&wrapped_ical)
                .into_iter()
                .next()
                .map(|event| ImportPreviewEvent {
                    uid: event.uid,
                    summary: event.summary,
                    dtstart: event.dtstart,
                    dtend: event.dtend.unwrap_or_default(),
                    is_all_day: event.is_all_day,
                    location: event.location,
                })
        })
        .collect();

    if previews.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "No importable events found in ICS file",
        )
            .into_response();
    }

    Json(previews).into_response()
}

pub async fn import_ics(
    State(state): State<crate::oidc::AppState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Response {
    let mut account_id = String::new();
    let mut calendar_href = String::new();
    let mut ics_content: Option<String> = None;
    let mut email_account_id: Option<String> = None;
    let mut email_mailbox: Option<String> = None;
    let mut email_uid: Option<i64> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
            Some("account_id") => {
                account_id = field.text().await.unwrap_or_default();
            }
            Some("calendar_href") => {
                calendar_href = field.text().await.unwrap_or_default();
            }
            Some("file") => match field.text().await {
                Ok(text) => ics_content = Some(text),
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, format!("Failed to read file: {e}"))
                        .into_response();
                }
            },
            Some("email_account_id") => {
                email_account_id = Some(field.text().await.unwrap_or_default());
            }
            Some("email_mailbox") => {
                email_mailbox = Some(field.text().await.unwrap_or_default());
            }
            Some("email_uid") => {
                email_uid = field.text().await.ok().and_then(|s| s.parse().ok());
            }
            _ => {}
        }
    }

    if calendar_href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing calendar_href").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }
    let ics = match ics_content {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, "Missing ICS file").into_response(),
    };
    let client = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let events = split_vcalendar_into_events(&ics);
    if events.is_empty() {
        return (StatusCode::BAD_REQUEST, "No events found in ICS file").into_response();
    }

    let mut imported = 0usize;
    let mut errors = 0usize;

    for (uid, wrapped_ical) in events {
        match client.put_event(&calendar_href, &uid, &wrapped_ical).await {
            Ok(_) => {
                let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
                if let Err(e) = db::upsert_cached_event_raw(
                    &state.db,
                    &account_id,
                    &calendar_href,
                    &event_href,
                    "",
                    &wrapped_ical,
                    &state.search_mode,
                )
                .await
                {
                    eprintln!("Cache insert failed for {uid}: {e}");
                }
                if let (Some(ea), Some(em), Some(eu)) =
                    (&email_account_id, &email_mailbox, email_uid)
                {
                    if let Err(e) = sqlx::query(
                        "INSERT OR IGNORE INTO email_imported_events \
                         (email_account_id, email_mailbox, email_uid, cal_account_id, calendar_href, event_uid) \
                         VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind(ea)
                    .bind(em)
                    .bind(eu)
                    .bind(&account_id)
                    .bind(&calendar_href)
                    .bind(&uid)
                    .execute(&state.db)
                    .await
                    {
                        eprintln!("email_imported_events insert failed for {uid}: {e}");
                    }
                }
                imported += 1;
            }
            Err(e) => {
                eprintln!("PUT failed for {uid}: {e}");
                errors += 1;
            }
        }
    }

    state
        .sse_broadcaster
        .send(&user.user_id, crate::sse::SseMessage::EventsRefetch);
    state
        .sync_manager
        .trigger(&user.user_id, SyncTrigger::Incremental);

    Json(ImportResult { imported, errors }).into_response()
}

// ── Search ────────────────────────────────────────────────────────────────────

pub async fn search_events(
    State(state): State<AppState>,
    user: CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let mut q = String::new();
    let mut start: Option<String> = None;
    let mut end: Option<String> = None;
    let mut cal_keys: Vec<(String, String)> = Vec::new();

    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "q" => q = value.into_owned(),
                "start" => start = Some(value.into_owned()),
                "end" => end = Some(value.into_owned()),
                "cal" => {
                    let value = value.into_owned();
                    if let Some((account_id, href)) = value.split_once('\0') {
                        cal_keys.push((account_id.to_string(), href.to_string()));
                    } else if let Some((account_id, href)) = value.split_once('|') {
                        cal_keys.push((account_id.to_string(), href.to_string()));
                    }
                }
                _ => {}
            }
        }
    }

    let q = q.trim().to_string();
    if q.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing search query").into_response();
    }
    let q: String = q.chars().take(200).collect();

    let fts = state.search_mode.build_fts_query(&q);
    if fts.is_empty() {
        return Json(Vec::<EventJson>::new()).into_response();
    }
    let user_id = user.user_id.clone();

    let mut sql =
        r#"SELECT e.account_id, e.uid, e.calendar_href, e.dtstart, e.raw_ical, c.color AS calendar_color
    FROM events_fts
    JOIN cached_events e ON events_fts.rowid = e.rowid
    JOIN cached_calendars c ON c.account_id = e.account_id AND c.href = e.calendar_href
    JOIN caldav_accounts ca ON ca.account_id = e.account_id
    WHERE events_fts MATCH ? AND ca.user_id = ?"#
            .to_string();
    if start.is_some() {
        sql.push_str(" AND e.dtstart >= ?");
    }
    if end.is_some() {
        sql.push_str(" AND e.dtstart <= ?");
    }
    if !cal_keys.is_empty() {
        let placeholders = cal_keys
            .iter()
            .map(|_| "(?, ?)")
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(
            " AND (e.account_id, e.calendar_href) IN ({})",
            placeholders
        ));
    }
    sql.push_str(" ORDER BY e.dtstart DESC LIMIT 100");

    let mut query = sqlx::query(&sql).bind(fts).bind(user_id);
    if let Some(s) = start {
        query = query.bind(s);
    }
    if let Some(e) = end {
        query = query.bind(e);
    }
    for (account_id, href) in &cal_keys {
        query = query.bind(account_id).bind(href);
    }

    match query.fetch_all(&state.db).await {
        Ok(rows) => {
            let mut results: Vec<EventJson> = Vec::new();
            for r in &rows {
                let account_id: String = r.get(0);
                let calendar_href: String = r.get(2);
                let dtstart: String = r.get(3);
                let raw_ical: String = r.get(4);
                let color: Option<String> = r.get(5);
                let color_str = color.as_deref().unwrap_or("");
                if let Ok(date) = NaiveDate::parse_from_str(&dtstart, "%Y-%m-%d") {
                    if let Some(event) = CalDavClient::events_from_raw_ical(&raw_ical, date, date)
                        .into_iter()
                        .next()
                    {
                        results.push(build_event_json(
                            &event,
                            &account_id,
                            color_str,
                            &calendar_href,
                        ));
                    }
                }
            }
            Json(results).into_response()
        }
        Err(e) => {
            eprintln!("Search query failed: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Search failed").into_response()
        }
    }
}

// GET /api/email/message-events

#[derive(serde::Deserialize)]
pub struct MessageEventsQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
}

#[derive(serde::Serialize)]
pub struct MessageEventGroupJson {
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
    pub events: Vec<EventJson>,
}

pub async fn get_message_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<MessageEventsQuery>,
) -> Response {
    let rows = sqlx::query(
        "SELECT e.account_id, e.calendar_href, e.dtstart, e.raw_ical, c.color \
         FROM email_imported_events ie \
         JOIN cached_events e ON e.account_id = ie.cal_account_id \
             AND e.calendar_href = ie.calendar_href \
             AND e.uid = ie.event_uid \
         JOIN cached_calendars c ON c.account_id = e.account_id AND c.href = e.calendar_href \
         JOIN caldav_accounts ca ON ca.account_id = e.account_id \
         WHERE ie.email_account_id = ? AND ie.email_mailbox = ? AND ie.email_uid = ? \
           AND ca.user_id = ?",
    )
    .bind(&q.account_id)
    .bind(&q.mailbox)
    .bind(q.uid)
    .bind(&user.user_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[message-events] query failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut results: Vec<EventJson> = Vec::new();
    for r in &rows {
        let account_id: String = r.get(0);
        let calendar_href: String = r.get(1);
        let dtstart: String = r.get(2);
        let raw_ical: String = r.get(3);
        let color: Option<String> = r.get(4);
        let color_str = color.as_deref().unwrap_or("");
        if let Ok(date) = NaiveDate::parse_from_str(&dtstart, "%Y-%m-%d") {
            if let Some(event) = CalDavClient::events_from_raw_ical(&raw_ical, date, date)
                .into_iter()
                .next()
            {
                results.push(build_event_json(
                    &event,
                    &account_id,
                    color_str,
                    &calendar_href,
                ));
            }
        }
    }

    Json(results).into_response()
}

// GET /api/email/thread-events

pub async fn get_thread_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<MessageEventsQuery>,
) -> Response {
    match email_db::get_account(
        &state.db,
        &q.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("[thread-events] account lookup failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let rows = sqlx::query(
        "WITH RECURSIVE connected(id) AS (
             SELECT message_id FROM cached_messages
              WHERE account_id = ? AND mailbox = ? AND uid = ? AND message_id IS NOT NULL
             UNION
             SELECT thread_id FROM cached_messages
              WHERE account_id = ? AND mailbox = ? AND uid = ? AND thread_id IS NOT NULL
             UNION
             SELECT m.message_id
               FROM cached_messages m JOIN connected c
                 ON m.message_id = c.id OR m.thread_id = c.id
              WHERE m.account_id = ? AND m.message_id IS NOT NULL
             UNION
             SELECT m.thread_id
               FROM cached_messages m JOIN connected c
                 ON m.message_id = c.id OR m.thread_id = c.id
              WHERE m.account_id = ? AND m.thread_id IS NOT NULL
         ),
         thread_messages AS (
             SELECT DISTINCT m.account_id, m.mailbox, m.uid, m.sent_at, m.rowid
               FROM cached_messages m
              WHERE m.account_id = ?
                AND ((m.mailbox = ? AND m.uid = ?)
                  OR m.message_id IN (SELECT id FROM connected)
                  OR m.thread_id IN (SELECT id FROM connected))
         )
         SELECT tm.account_id, tm.mailbox, tm.uid,
                e.account_id, e.calendar_href, e.dtstart, e.raw_ical, c.color
           FROM thread_messages tm
           JOIN email_imported_events ie
             ON ie.email_account_id = tm.account_id
            AND ie.email_mailbox = tm.mailbox
            AND ie.email_uid = tm.uid
           JOIN cached_events e
             ON e.account_id = ie.cal_account_id
            AND e.calendar_href = ie.calendar_href
            AND e.uid = ie.event_uid
           JOIN cached_calendars c
             ON c.account_id = e.account_id AND c.href = e.calendar_href
           JOIN caldav_accounts ca ON ca.account_id = e.account_id
          WHERE ca.user_id = ?
          ORDER BY COALESCE(tm.sent_at, 9223372036854775807), tm.rowid, e.dtstart",
    )
    .bind(&q.account_id)
    .bind(&q.mailbox)
    .bind(q.uid)
    .bind(&q.account_id)
    .bind(&q.mailbox)
    .bind(q.uid)
    .bind(&q.account_id)
    .bind(&q.account_id)
    .bind(&q.account_id)
    .bind(&q.mailbox)
    .bind(q.uid)
    .bind(&user.user_id)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[thread-events] query failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut groups: Vec<MessageEventGroupJson> = Vec::new();
    for r in &rows {
        let message_account_id: String = r.get(0);
        let message_mailbox: String = r.get(1);
        let message_uid: i64 = r.get(2);
        let account_id: String = r.get(3);
        let calendar_href: String = r.get(4);
        let dtstart: String = r.get(5);
        let raw_ical: String = r.get(6);
        let color: Option<String> = r.get(7);
        let color_str = color.as_deref().unwrap_or("");
        let Some(event) = NaiveDate::parse_from_str(&dtstart, "%Y-%m-%d")
            .ok()
            .and_then(|date| {
                CalDavClient::events_from_raw_ical(&raw_ical, date, date)
                    .into_iter()
                    .next()
            })
        else {
            continue;
        };

        if let Some(group) = groups.iter_mut().find(|group| {
            group.account_id == message_account_id
                && group.mailbox == message_mailbox
                && group.uid == message_uid
        }) {
            group.events.push(build_event_json(
                &event,
                &account_id,
                color_str,
                &calendar_href,
            ));
        } else {
            groups.push(MessageEventGroupJson {
                account_id: message_account_id,
                mailbox: message_mailbox,
                uid: message_uid,
                events: vec![build_event_json(
                    &event,
                    &account_id,
                    color_str,
                    &calendar_href,
                )],
            });
        }
    }

    Json(groups).into_response()
}

// GET /api/calendar/event-emails

#[derive(serde::Deserialize)]
pub struct EventEmailsQuery {
    pub cal_account_id: String,
    pub calendar_href: String,
    pub event_uid: String,
}

#[derive(serde::Serialize)]
pub struct SourceEmailJson {
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
    pub from: Option<String>,
    pub subject: Option<String>,
    pub sent_at: Option<i64>,
}

pub async fn get_event_source_emails(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<EventEmailsQuery>,
) -> Response {
    let rows = sqlx::query(
        "SELECT ie.email_account_id, ie.email_mailbox, ie.email_uid, \
                cm.from_addr, cm.subject, cm.sent_at \
         FROM email_imported_events ie \
         JOIN caldav_accounts ca ON ca.account_id = ie.cal_account_id \
         LEFT JOIN cached_messages cm ON cm.account_id = ie.email_account_id \
             AND cm.mailbox = ie.email_mailbox AND cm.uid = ie.email_uid \
         WHERE ie.cal_account_id = ? AND ie.calendar_href = ? AND ie.event_uid = ? \
           AND ca.user_id = ?",
    )
    .bind(&q.cal_account_id)
    .bind(&q.calendar_href)
    .bind(&q.event_uid)
    .bind(&user.user_id)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let results: Vec<SourceEmailJson> = rows
                .iter()
                .map(|r| SourceEmailJson {
                    account_id: r.get(0),
                    mailbox: r.get(1),
                    uid: r.get(2),
                    from: r.get(3),
                    subject: r.get(4),
                    sent_at: r.get(5),
                })
                .collect();
            Json(results).into_response()
        }
        Err(e) => {
            eprintln!("[event-emails] query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// GET /api/event/ics
#[derive(serde::Deserialize)]
pub struct EventIcsQuery {
    pub account_id: String,
    pub calendar_href: String,
    pub uid: String,
}

pub async fn get_event_ics(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<EventIcsQuery>,
) -> Response {
    match db::get_user_id_for_caldav_account(&state.db, &q.account_id).await {
        Ok(Some(owner)) if owner == user.user_id => {}
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("[calendar] account lookup error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT raw_ical FROM cached_events WHERE account_id = ? AND calendar_href = ? AND uid = ?",
    )
    .bind(&q.account_id)
    .bind(&q.calendar_href)
    .bind(&q.uid)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    match raw {
        Some(ical) => (
            [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
            ical,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use axum_typed_multipart::{TryFromMultipart, TypedMultipart};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::caldav::{CalDavClient, preview_events_from_raw_ical, split_vcalendar_into_events};
use crate::db;
use crate::email_db;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use utoipa::{IntoParams, ToSchema};

use super::{EventJson, build_event_json, require_account_client};

#[derive(serde::Serialize, ToSchema)]
pub(crate) struct ImportResult {
    /// the calendar the events went to (a new one for `new_calendar_name`)
    account_id: String,
    calendar_href: String,
    /// events written to the calendar
    imported: usize,
    /// events the CalDAV server refused
    errors: usize,
}

/// An event of an ICS file, before the import.
#[derive(serde::Serialize, ToSchema)]
pub(crate) struct ImportPreviewEvent {
    uid: String,
    summary: String,
    dtstart: String,
    dtend: String,
    is_all_day: bool,
    location: String,
}

#[derive(TryFromMultipart, ToSchema)]
pub(crate) struct IcsFileForm {
    /// the ICS file (UTF-8)
    #[schema(value_type = String, format = Binary)]
    file: String,
}

/// An ICS file to import into a calendar: an existing one (`calendar_href`) or a new one of the
/// account (`new_calendar_name`). The `email_*` fields name the message it came from, so the
/// message shows the events and the events their source message.
#[derive(TryFromMultipart, ToSchema)]
pub(crate) struct ImportForm {
    account_id: String,
    /// the calendar to import into
    calendar_href: Option<String>,
    /// without `calendar_href`: the name of a calendar to create for the import
    new_calendar_name: Option<String>,
    /// the ICS file (UTF-8)
    #[schema(value_type = String, format = Binary)]
    file: String,
    email_account_id: Option<String>,
    email_mailbox: Option<String>,
    email_uid: Option<u32>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchQuery {
    /// the search text (at most 200 characters are used)
    q: String,
    /// earliest start, `YYYY-MM-DD`
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(value_type = Option<String>, format = Date)]
    start: Option<NaiveDate>,
    /// latest start, `YYYY-MM-DD`
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(value_type = Option<String>, format = Date)]
    end: Option<NaiveDate>,
    /// `1`: only the active calendars (when no `cal` is given)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    active: Option<u8>,
    /// calendars to search, each `account_id` and `href` joined by NUL or `|` (default: all)
    #[serde(default)]
    cal: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/import/preview",
    request_body(content = IcsFileForm, content_type = "multipart/form-data"),
    responses(
        (status = 200, body = Vec<ImportPreviewEvent>),
        (status = 400, description = "no file, or no importable events in it (plain text)")
    )
)]
pub async fn preview_ics_import(
    State(_state): State<crate::oidc::AppState>,
    _user: CurrentUser,
    TypedMultipart(IcsFileForm { file: ics }): TypedMultipart<IcsFileForm>,
) -> Response {
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

#[utoipa::path(
    post,
    path = "/import",
    request_body(content = ImportForm, content_type = "multipart/form-data"),
    responses(
        (status = 200, body = ImportResult),
        (status = 400, description = "a required field is missing, or no events in the file (plain text)"),
        (status = 404, description = "no such CalDAV account (plain text)"),
        (status = 502, description = "the CalDAV server refused to create the new calendar (plain text)")
    )
)]
pub async fn import_ics(
    State(state): State<crate::oidc::AppState>,
    user: CurrentUser,
    TypedMultipart(ImportForm {
        account_id,
        calendar_href,
        new_calendar_name,
        file: ics,
        email_account_id,
        email_mailbox,
        email_uid,
    }): TypedMultipart<ImportForm>,
) -> Response {
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }
    let calendar_href = calendar_href.filter(|h| !h.is_empty());
    let new_calendar_name = new_calendar_name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    if calendar_href.is_none() && new_calendar_name.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "Missing calendar_href or new_calendar_name",
        )
            .into_response();
    }
    let events = split_vcalendar_into_events(&ics);
    if events.is_empty() {
        return (StatusCode::BAD_REQUEST, "No events found in ICS file").into_response();
    }
    // the message the events come from is linked to them (event-emails shows it): only one of the user's
    let email_account = match &email_account_id {
        Some(ea) => match crate::email::owned_account(&state, &user.user_id, ea).await {
            Ok(account) => Some(account),
            Err(response) => return response,
        },
        None => None,
    };
    let source_message_identity =
        if let (Some(ea), Some(em), Some(eu)) = (&email_account, &email_mailbox, email_uid) {
            email_db::get_message(&state.db, ea.id(), em, eu)
                .await
                .ok()
                .flatten()
                .map(|msg| (msg.message_id, msg.thread_id))
        } else {
            None
        };
    let (account, client) = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let calendar_href = match (calendar_href, new_calendar_name) {
        (Some(href), _) => href,
        (None, Some(name)) => {
            match super::events::create_calendar(&state, &user, &client, account.id(), &name, None)
                .await
            {
                Ok(href) => href,
                Err(r) => return r,
            }
        }
        (None, None) => unreachable!("checked above"),
    };

    let mut imported = 0usize;
    let mut errors = 0usize;

    for (uid, wrapped_ical) in events {
        match client.put_event(&calendar_href, &uid, &wrapped_ical).await {
            Ok(_) => {
                let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
                if let Err(e) = db::upsert_cached_event_raw(
                    &state.db,
                    account.id(),
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
                    let (email_message_id, email_thread_id) = source_message_identity
                        .as_ref()
                        .map(|(message_id, thread_id)| (message_id.as_ref(), thread_id.as_ref()))
                        .unwrap_or((None, None));
                    if let Err(e) = sqlx::query(
                        "INSERT INTO email_imported_events \
                         (email_account_id, email_mailbox, email_uid, email_message_id, email_thread_id, cal_account_id, calendar_href, event_uid) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                         ON CONFLICT(email_account_id, email_mailbox, email_uid, event_uid) DO UPDATE SET \
                            email_message_id = COALESCE(email_imported_events.email_message_id, excluded.email_message_id), \
                            email_thread_id = COALESCE(email_imported_events.email_thread_id, excluded.email_thread_id)",
                    )
                    .bind(ea)
                    .bind(em)
                    .bind(i64::from(eu))
                    .bind(email_message_id)
                    .bind(email_thread_id)
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

    Json(ImportResult {
        account_id,
        calendar_href,
        imported,
        errors,
    })
    .into_response()
}

// ── Search ────────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/search",
    params(SearchQuery),
    responses(
        (status = 200, description = "at most 100 events, latest first", body = Vec<EventJson>),
        (status = 400, description = "no search text (plain text)")
    )
)]
pub async fn search_events(
    State(state): State<AppState>,
    user: CurrentUser,
    axum_extra::extract::Query(SearchQuery {
        q,
        start,
        end,
        active,
        cal,
    }): axum_extra::extract::Query<SearchQuery>,
) -> Response {
    let start = start.map(|d| d.format("%Y-%m-%d").to_string());
    let end = end.map(|d| d.format("%Y-%m-%d").to_string());
    let mut cal_keys: Vec<(String, String)> = cal
        .iter()
        .filter_map(|key| key.split_once('\0').or_else(|| key.split_once('|')))
        .map(|(account_id, href)| (account_id.to_string(), href.to_string()))
        .collect();
    // `active=1` without `cal`: the calendars shown in the calendar (not deactivated), as the search page's
    // default; it needs no calendar list in the browser, so the request can leave before the page has one.
    let active_only = active == Some(1);

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

    if cal_keys.is_empty() && active_only {
        let calendars = db::get_cached_calendars(&state.db, &user_id)
            .await
            .unwrap_or_default();
        for c in &calendars {
            let deactivated =
                db::get_deactivated_calendars(&state.db, &c.account_id, &state.encryption_key)
                    .await
                    .unwrap_or_default();
            if !deactivated.contains(&c.href) {
                cal_keys.push((c.account_id.clone(), c.href.clone()));
            }
        }
        if cal_keys.is_empty() {
            return Json(Vec::<EventJson>::new()).into_response();
        }
    }

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

    let mut query = sqlx::query(AssertSqlSafe(sql)).bind(fts).bind(user_id);
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
                            None,
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

/// A mail message.
#[derive(serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MessageEventsQuery {
    /// the mail account
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
}

/// A message of a thread and the events imported from it.
#[derive(serde::Serialize, ToSchema)]
pub struct MessageEventGroupJson {
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
    pub events: Vec<EventJson>,
}

#[utoipa::path(
    get,
    path = "/email/message-events",
    params(MessageEventsQuery),
    responses(
        (status = 200, description = "the events imported from the message", body = Vec<EventJson>),
        (status = 400, description = "a parameter is missing or invalid (plain text)")
    )
)]
pub async fn get_message_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<MessageEventsQuery>,
) -> Response {
    if let Err(response) = crate::email::owned_account(&state, &user.user_id, &q.account_id).await {
        return response;
    }
    let rows = sqlx::query(
        "WITH current_message AS (
             SELECT message_id
               FROM cached_messages
              WHERE account_id = ? AND mailbox = ? AND uid = ?
              LIMIT 1
         )
         SELECT e.account_id, e.calendar_href, e.dtstart, e.raw_ical, c.color \
         FROM email_imported_events ie \
         JOIN cached_events e ON e.account_id = ie.cal_account_id \
             AND e.calendar_href = ie.calendar_href \
             AND e.uid = ie.event_uid \
         JOIN cached_calendars c ON c.account_id = e.account_id AND c.href = e.calendar_href \
         JOIN caldav_accounts ca ON ca.account_id = e.account_id \
         WHERE ie.email_account_id = ? \
           AND ( \
                (ie.email_message_id IS NOT NULL \
                 AND ie.email_message_id = (SELECT message_id FROM current_message)) \
             OR (ie.email_mailbox = ? AND ie.email_uid = ?) \
           ) \
           AND ca.user_id = ?",
    )
    .bind(&q.account_id)
    .bind(&q.mailbox)
    .bind(q.uid)
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
                    None,
                ));
            }
        }
    }

    Json(results).into_response()
}

// GET /api/email/thread-events

#[utoipa::path(
    get,
    path = "/email/thread-events",
    params(MessageEventsQuery),
    responses(
        (status = 200, description = "the thread's messages with imported events, in thread order", body = Vec<MessageEventGroupJson>),
        (status = 400, description = "a parameter is missing or invalid (plain text)"),
        (status = 404, description = "no such mail account (empty body)")
    )
)]
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
             SELECT DISTINCT m.account_id, m.mailbox, m.uid, m.message_id, m.sent_at, m.rowid
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
            AND ((ie.email_message_id IS NOT NULL AND ie.email_message_id = tm.message_id)
              OR (ie.email_mailbox = tm.mailbox AND ie.email_uid = tm.uid))
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
                None,
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
                    None,
                )],
            });
        }
    }

    Json(groups).into_response()
}

// GET /api/calendar/event-emails

/// An event.
#[derive(serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventEmailsQuery {
    /// the CalDAV account
    pub cal_account_id: String,
    pub calendar_href: String,
    pub event_uid: String,
}

/// A mail message an event was imported from.
#[derive(serde::Serialize, ToSchema)]
pub struct SourceEmailJson {
    pub account_id: String,
    pub mailbox: String,
    pub uid: i64,
    #[schema(required)]
    pub from: Option<String>,
    #[schema(required)]
    pub subject: Option<String>,
    #[schema(required)]
    pub sent_at: Option<i64>,
}

async fn query_event_source_emails(
    pool: &SqlitePool,
    user_id: &str,
    q: &EventEmailsQuery,
) -> Result<Vec<SourceEmailJson>, sqlx::Error> {
    let rows = sqlx::query(
        "WITH source_candidates AS (
             SELECT ie.id,
                    cm.account_id AS account_id,
                    cm.mailbox AS mailbox,
                    cm.uid AS uid,
                    cm.from_addr AS from_addr,
                    cm.subject AS subject,
                    cm.sent_at AS sent_at,
                    CASE
                        WHEN cm.mailbox = ie.email_mailbox AND cm.uid = ie.email_uid THEN 1
                        ELSE 0
                    END AS original_match,
                    cm.rowid AS message_rowid
               FROM email_imported_events ie
               JOIN caldav_accounts ca ON ca.account_id = ie.cal_account_id
               JOIN cached_messages cm
                 ON cm.account_id = ie.email_account_id
                AND ie.email_message_id IS NOT NULL
                AND cm.message_id = ie.email_message_id
               JOIN imap_accounts ia ON ia.account_id = cm.account_id AND ia.user_id = ca.user_id
              WHERE ie.cal_account_id = ? AND ie.calendar_href = ? AND ie.event_uid = ?
                AND ca.user_id = ?
             UNION ALL
             SELECT ie.id,
                    cm.account_id AS account_id,
                    cm.mailbox AS mailbox,
                    cm.uid AS uid,
                    cm.from_addr AS from_addr,
                    cm.subject AS subject,
                    cm.sent_at AS sent_at,
                    1 AS original_match,
                    cm.rowid AS message_rowid
               FROM email_imported_events ie
               JOIN caldav_accounts ca ON ca.account_id = ie.cal_account_id
               JOIN cached_messages cm
                 ON cm.account_id = ie.email_account_id
                AND cm.mailbox = ie.email_mailbox
                AND cm.uid = ie.email_uid
               JOIN imap_accounts ia ON ia.account_id = cm.account_id AND ia.user_id = ca.user_id
              WHERE ie.cal_account_id = ? AND ie.calendar_href = ? AND ie.event_uid = ?
                AND ca.user_id = ?
                AND NOT EXISTS (
                    SELECT 1
                      FROM cached_messages cm2
                     WHERE cm2.account_id = ie.email_account_id
                       AND ie.email_message_id IS NOT NULL
                       AND cm2.message_id = ie.email_message_id
                )
         ),
         ranked_sources AS (
             SELECT *,
                    ROW_NUMBER() OVER (
                        PARTITION BY id
                        ORDER BY original_match ASC, message_rowid DESC
                    ) AS rank
               FROM source_candidates
         )
         SELECT account_id, mailbox, uid, from_addr, subject, sent_at
           FROM ranked_sources
          WHERE rank = 1",
    )
    .bind(&q.cal_account_id)
    .bind(&q.calendar_href)
    .bind(&q.event_uid)
    .bind(user_id)
    .bind(&q.cal_account_id)
    .bind(&q.calendar_href)
    .bind(&q.event_uid)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| SourceEmailJson {
            account_id: r.get(0),
            mailbox: r.get(1),
            uid: r.get(2),
            from: r.get(3),
            subject: r.get(4),
            sent_at: r.get(5),
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/calendar/event-emails",
    params(EventEmailsQuery),
    responses(
        (status = 200, body = Vec<SourceEmailJson>),
        (status = 400, description = "a parameter is missing (plain text)")
    )
)]
pub async fn get_event_source_emails(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<EventEmailsQuery>,
) -> Response {
    match query_event_source_emails(&state.db, &user.user_id, &q).await {
        Ok(results) => Json(results).into_response(),
        Err(e) => {
            eprintln!("[event-emails] query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// GET /api/event/ics
#[derive(serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventIcsQuery {
    /// the CalDAV account
    pub account_id: String,
    pub calendar_href: String,
    pub uid: String,
}

#[utoipa::path(
    get,
    path = "/event/ics",
    params(EventIcsQuery),
    responses(
        (status = 200, description = "the event's iCalendar data as stored", body = String, content_type = "text/calendar"),
        (status = 400, description = "a parameter is missing (plain text)"),
        (status = 404, description = "no such account or event (empty body)")
    )
)]
pub async fn get_event_ics(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<EventIcsQuery>,
) -> Response {
    let account = match super::owned_account(&state, &user.user_id, &q.account_id).await {
        Ok(account) => account,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let raw: Option<String> = sqlx::query_scalar(
        "SELECT raw_ical FROM cached_events WHERE account_id = ? AND calendar_href = ? AND uid = ?",
    )
    .bind(account.id().as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn source_email_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE caldav_accounts (
                account_id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE cached_messages (
                account_id TEXT NOT NULL,
                mailbox TEXT NOT NULL,
                uid INTEGER NOT NULL,
                message_id TEXT,
                thread_id TEXT,
                from_addr TEXT,
                subject TEXT,
                sent_at INTEGER,
                PRIMARY KEY (account_id, mailbox, uid)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE email_imported_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                email_account_id TEXT NOT NULL,
                email_mailbox TEXT NOT NULL,
                email_uid INTEGER NOT NULL,
                email_message_id TEXT,
                email_thread_id TEXT,
                cal_account_id TEXT NOT NULL,
                calendar_href TEXT NOT NULL,
                event_uid TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO caldav_accounts (account_id, user_id) VALUES ('cal', 'user')")
            .execute(&pool)
            .await
            .unwrap();
        // the mail account the source messages are in: the same user's
        sqlx::query(
            "CREATE TABLE imap_accounts (account_id TEXT PRIMARY KEY, user_id TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO imap_accounts (account_id, user_id) VALUES ('mail', 'user')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn event_source_email_resolves_current_message_after_move() {
        let pool = source_email_pool().await;
        sqlx::query(
            "INSERT INTO email_imported_events
             (email_account_id, email_mailbox, email_uid, email_message_id, email_thread_id, cal_account_id, calendar_href, event_uid)
             VALUES ('mail', 'INBOX', 10, '<invite@example>', '<thread@example>', 'cal', '/cal', 'event')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cached_messages
             (account_id, mailbox, uid, message_id, thread_id, from_addr, subject, sent_at)
             VALUES ('mail', 'Archive', 88, '<invite@example>', '<thread@example>', 'Host <host@example>', 'Invite', 123)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let sources = query_event_source_emails(
            &pool,
            "user",
            &EventEmailsQuery {
                cal_account_id: "cal".into(),
                calendar_href: "/cal".into(),
                event_uid: "event".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].mailbox, "Archive");
        assert_eq!(sources[0].uid, 88);
        assert_eq!(sources[0].subject.as_deref(), Some("Invite"));
    }

    #[tokio::test]
    async fn event_source_email_keeps_legacy_exact_location_fallback() {
        let pool = source_email_pool().await;
        sqlx::query(
            "INSERT INTO email_imported_events
             (email_account_id, email_mailbox, email_uid, cal_account_id, calendar_href, event_uid)
             VALUES ('mail', 'INBOX', 10, 'cal', '/cal', 'event')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cached_messages
             (account_id, mailbox, uid, message_id, from_addr, subject, sent_at)
             VALUES ('mail', 'INBOX', 10, '<invite@example>', 'Host <host@example>', 'Invite', 123)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let sources = query_event_source_emails(
            &pool,
            "user",
            &EventEmailsQuery {
                cal_account_id: "cal".into(),
                calendar_href: "/cal".into(),
                event_uid: "event".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].mailbox, "INBOX");
        assert_eq!(sources[0].uid, 10);
        assert_eq!(sources[0].subject.as_deref(), Some("Invite"));
    }
}

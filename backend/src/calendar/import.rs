use axum::{
    extract::{Multipart, Query, RawQuery, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use chrono::NaiveDate;
use url::form_urlencoded;

use crate::caldav::{CalDavClient, preview_events_from_raw_ical, split_vcalendar_into_events};
use crate::db;
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
                    return Err((StatusCode::BAD_REQUEST, format!("Failed to read file: {e}"))
                        .into_response());
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
        return (StatusCode::BAD_REQUEST, "No importable events found in ICS file").into_response();
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
        let placeholders = cal_keys.iter().map(|_| "(?, ?)").collect::<Vec<_>>().join(", ");
        sql.push_str(&format!(" AND (e.account_id, e.calendar_href) IN ({})", placeholders));
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
                        results.push(build_event_json(&event, &account_id, color_str, &calendar_href));
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

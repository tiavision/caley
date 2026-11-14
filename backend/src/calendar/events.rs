use axum::{
    extract::{RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use url::form_urlencoded;

use super::{
    CalendarJson, build_event_json, extract_vevent, parse_alarm_from_fields, parse_event_form,
    parse_form_field, require_account_client,
};
use crate::caldav::{
    Alarm, Attendee, CalDavClient, PartStat, RecurrenceInfo, add_exdate, add_recurrence_override,
    generate_ical_event, generate_new_uid, update_vcalendar_event,
};
use crate::db;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;
use utoipa::{IntoParams, ToSchema};

// The handlers parse their forms and queries by hand (fields repeat, missing ones have
// defaults), so these structs only describe the requests.

/// An event to create (no `uid`) or save. The alarm and attendee fields repeat, once
/// per alarm / attendee, in the same order.
#[allow(dead_code)]
#[derive(ToSchema)]
pub(crate) struct EventForm {
    /// empty or missing: a new event
    uid: Option<String>,
    summary: String,
    description: Option<String>,
    location: Option<String>,
    /// `YYYY-MM-DD`
    date: String,
    /// `HH:MM`
    start_time: Option<String>,
    /// `HH:MM`
    end_time: Option<String>,
    /// IANA timezone (default `UTC`)
    tz: Option<String>,
    /// `true` for an all-day event
    is_all_day: Option<String>,
    calendar_href: String,
    account_id: String,
    /// `series` (default) or `this_instance` (an override of one occurrence)
    scope: Option<String>,
    /// the occurrence's original start (`this_instance`)
    original_dtstart: Option<String>,
    /// the timezone the recurring master was authored in (default `tz`)
    original_tz: Option<String>,
    /// the calendar the event was in, when it moves
    original_calendar_href: Option<String>,
    /// the account the event was in, when it moves
    original_account_id: Option<String>,
    /// `RecurrenceInfo` as JSON; empty or `{}` for none
    recurrence: Option<String>,
    alarm_direction: Vec<String>,
    alarm_value: Vec<String>,
    alarm_unit: Vec<String>,
    alarm_datetime: Vec<String>,
    attendee_cn: Vec<String>,
    attendee_email: Vec<String>,
    attendee_partstat: Vec<String>,
}

#[allow(dead_code)]
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeleteEventQuery {
    uid: String,
    account_id: String,
    calendar_href: String,
    /// `this_instance` (with `original_dtstart`) deletes one occurrence; else the event
    scope: Option<String>,
    /// the occurrence's original start (`this_instance`)
    original_dtstart: Option<String>,
    /// the occurrence's timezone (default `UTC`)
    tz: Option<String>,
}

#[allow(dead_code)]
#[derive(ToSchema)]
pub(crate) struct CreateCalendarForm {
    account_id: String,
    name: String,
    /// CSS color; empty for none
    color: Option<String>,
}

#[utoipa::path(
    post,
    path = "/event",
    request_body(content = EventForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "saved (empty body); the event arrives as an `event_updated` SSE event"),
        (status = 400, description = "a required field is missing (plain text)"),
        (status = 404, description = "no such CalDAV account or event (plain text)"),
        (status = 500, description = "the CalDAV server refused it (plain text)")
    )
)]
pub async fn save_event(
    State(state): State<AppState>,
    user: CurrentUser,
    body: String,
) -> Response {
    let form = parse_event_form(body);
    let raw_uid = parse_form_field(&form, "uid").unwrap_or("").to_string();
    let is_edit = !raw_uid.is_empty();
    let mut uid = raw_uid;
    if uid.is_empty() {
        uid = generate_new_uid();
    }
    let summary = parse_form_field(&form, "summary")
        .unwrap_or_default()
        .to_string();
    let description = parse_form_field(&form, "description")
        .unwrap_or_default()
        .to_string();
    let location = parse_form_field(&form, "location")
        .unwrap_or_default()
        .to_string();
    let date = parse_form_field(&form, "date")
        .unwrap_or_default()
        .to_string();
    let start_time = parse_form_field(&form, "start_time");
    let end_time = parse_form_field(&form, "end_time");
    let tz = parse_form_field(&form, "tz").unwrap_or("UTC").to_string();
    let is_all_day = parse_form_field(&form, "is_all_day") == Some("true");
    let calendar_href = match parse_form_field(&form, "calendar_href") {
        Some(h) => h.to_string(),
        None => return (StatusCode::BAD_REQUEST, "Missing calendar_href").into_response(),
    };
    let account_id = match parse_form_field(&form, "account_id") {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return (StatusCode::BAD_REQUEST, "Missing account_id").into_response(),
    };
    let scope = parse_form_field(&form, "scope")
        .unwrap_or("series")
        .to_string();
    let original_dtstart = parse_form_field(&form, "original_dtstart");
    // Timezone the recurring master was authored in; the RECURRENCE-ID must
    // stay in that zone even when the edited override is saved in `tz`.
    let original_tz = parse_form_field(&form, "original_tz")
        .filter(|s| !s.is_empty())
        .unwrap_or(&tz)
        .to_string();
    let original_calendar_href =
        parse_form_field(&form, "original_calendar_href").map(|s| s.to_string());
    let original_account_id = parse_form_field(&form, "original_account_id")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let recurrence: Option<RecurrenceInfo> = parse_form_field(&form, "recurrence")
        .filter(|r| !r.is_empty() && *r != "{}")
        .and_then(|r| serde_json::from_str(r).ok());

    let alarm_dirs: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "alarm_direction")
        .map(|(_, v)| v.as_str())
        .collect();
    let alarm_vals: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "alarm_value")
        .map(|(_, v)| v.as_str())
        .collect();
    let alarm_units: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "alarm_unit")
        .map(|(_, v)| v.as_str())
        .collect();
    let alarm_dts: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "alarm_datetime")
        .map(|(_, v)| v.as_str())
        .collect();
    let alarms: Vec<Alarm> = alarm_dirs
        .iter()
        .enumerate()
        .filter_map(|(i, dir)| {
            parse_alarm_from_fields(
                dir,
                alarm_vals.get(i).copied().unwrap_or(""),
                alarm_units.get(i).copied().unwrap_or("1"),
                alarm_dts.get(i).copied().unwrap_or(""),
                &tz,
            )
        })
        .collect();

    let att_cns: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "attendee_cn")
        .map(|(_, v)| v.as_str())
        .collect();
    let att_emails: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "attendee_email")
        .map(|(_, v)| v.as_str())
        .collect();
    let att_partstats: Vec<&str> = form
        .iter()
        .filter(|(k, _)| k == "attendee_partstat")
        .map(|(_, v)| v.as_str())
        .collect();
    let attendees: Vec<Attendee> = att_emails
        .iter()
        .enumerate()
        .filter_map(|(i, email)| {
            let email = email.trim();
            if email.is_empty() {
                return None;
            }
            Some(Attendee {
                cn: att_cns.get(i).copied().unwrap_or("").to_string(),
                email: email.to_string(),
                partstat: PartStat::from_ical(att_partstats.get(i).copied().unwrap_or("")),
            })
        })
        .collect();

    if summary.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing summary").into_response();
    }
    if date.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing date").into_response();
    }

    let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
    let client = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if scope == "this_instance" {
        let raw_ical = match fetch_event_ical(&state, &client, &account_id, &calendar_href, &uid)
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => return (StatusCode::NOT_FOUND, "Event not found").into_response(),
            Err(e) => {
                eprintln!("Failed to fetch event for override: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read event").into_response();
            }
        };
        let orig_dt = original_dtstart.unwrap_or("");
        let rid_value = if is_all_day {
            orig_dt.split('T').next().unwrap_or(orig_dt)
        } else {
            orig_dt
        };
        let modified = match update_vcalendar_event(
            &raw_ical,
            &summary,
            &description,
            &location,
            &date,
            start_time,
            end_time,
            &tz,
            is_all_day,
            Some((rid_value, &original_tz)),
            None,
            &alarms,
            &attendees,
        ) {
            Some(updated) => updated,
            None => {
                let override_ical = generate_ical_event(
                    &uid,
                    &summary,
                    &description,
                    &location,
                    &date,
                    start_time,
                    end_time,
                    &tz,
                    is_all_day,
                    Some((rid_value, &original_tz)),
                    None,
                    &alarms,
                    &attendees,
                );
                let override_vevent = extract_vevent(&override_ical);
                add_recurrence_override(&raw_ical, &override_vevent, rid_value)
            }
        };

        match client.put_event(&calendar_href, &uid, &modified).await {
            Ok(_) => {
                optimistic_upsert(
                    &state,
                    &user.user_id,
                    &account_id,
                    &calendar_href,
                    &event_href,
                    &modified,
                    &uid,
                )
                .await;
                state
                    .sync_manager
                    .trigger(&user.user_id, SyncTrigger::Incremental);
                StatusCode::OK.into_response()
            }
            Err(e) => {
                eprintln!("Failed to update event instance: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to save event: {e}"),
                )
                    .into_response()
            }
        }
    } else {
        let fetch_href = original_calendar_href.as_deref().unwrap_or(&calendar_href);
        let original_client = match original_account_id.as_deref() {
            Some(orig_account_id) if orig_account_id != account_id => {
                match require_account_client(&state, &user, orig_account_id).await {
                    Ok(c) => Some(c),
                    Err(r) => return r,
                }
            }
            _ => None,
        };
        let ical = if is_edit {
            let original = fetch_event_ical(
                &state,
                original_client.as_ref().unwrap_or(&client),
                original_account_id.as_deref().unwrap_or(&account_id),
                fetch_href,
                &uid,
            )
            .await
            .and_then(|o| o.ok_or_else(|| anyhow::anyhow!("event not in cache")));
            match original {
                Ok(orig_ical) => {
                    match update_vcalendar_event(
                        &orig_ical,
                        &summary,
                        &description,
                        &location,
                        &date,
                        start_time,
                        end_time,
                        &tz,
                        is_all_day,
                        None,
                        recurrence.as_ref(),
                        &alarms,
                        &attendees,
                    ) {
                        Some(updated) => updated,
                        None => {
                            eprintln!(
                                "update_vcalendar_event: master VEVENT not found in {uid}, falling back"
                            );
                            generate_ical_event(
                                &uid,
                                &summary,
                                &description,
                                &location,
                                &date,
                                start_time,
                                end_time,
                                &tz,
                                is_all_day,
                                None,
                                recurrence.as_ref(),
                                &alarms,
                                &attendees,
                            )
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Failed to fetch original event {uid} for non-destructive update: {e}"
                    );
                    generate_ical_event(
                        &uid,
                        &summary,
                        &description,
                        &location,
                        &date,
                        start_time,
                        end_time,
                        &tz,
                        is_all_day,
                        None,
                        recurrence.as_ref(),
                        &alarms,
                        &attendees,
                    )
                }
            }
        } else {
            generate_ical_event(
                &uid,
                &summary,
                &description,
                &location,
                &date,
                start_time,
                end_time,
                &tz,
                is_all_day,
                None,
                recurrence.as_ref(),
                &alarms,
                &attendees,
            )
        };

        match client.put_event(&calendar_href, &uid, &ical).await {
            Ok(_) => {
                if let Some(ref orig) = original_calendar_href {
                    let moved_account = original_account_id
                        .as_deref()
                        .is_some_and(|id| id != account_id);
                    if (!orig.is_empty()) && (*orig != calendar_href || moved_account) {
                        let delete_client = original_client.as_ref().unwrap_or(&client);
                        if let Err(e) = delete_client.delete_event(orig, &uid).await {
                            eprintln!("Failed to delete event from old calendar: {e}");
                        } else {
                            let old_href = format!("{}/{}.ics", orig.trim_end_matches('/'), uid);
                            let _ = db::delete_event_by_href(
                                &state.db,
                                original_account_id.as_deref().unwrap_or(&account_id),
                                orig,
                                &old_href,
                            )
                            .await;
                        }
                    }
                }
                optimistic_upsert(
                    &state,
                    &user.user_id,
                    &account_id,
                    &calendar_href,
                    &event_href,
                    &ical,
                    &uid,
                )
                .await;
                state
                    .sync_manager
                    .trigger(&user.user_id, SyncTrigger::Incremental);
                StatusCode::OK.into_response()
            }
            Err(e) => {
                eprintln!("Failed to save event: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to save event: {e}"),
                )
                    .into_response()
            }
        }
    }
}

#[utoipa::path(
    delete,
    path = "/event",
    params(DeleteEventQuery),
    responses(
        (status = 200, description = "deleted (empty body)"),
        (status = 400, description = "a required parameter is missing (plain text)"),
        (status = 404, description = "no such CalDAV account or event (plain text)"),
        (status = 500, description = "the CalDAV server refused it (plain text)")
    )
)]
pub async fn delete_event(
    State(state): State<AppState>,
    user: CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let mut uid = String::new();
    let mut account_id = String::new();
    let mut calendar_href = String::new();
    let mut scope = String::new();
    let mut original_dtstart = String::new();
    let mut tz = String::new();
    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "uid" => uid = value.into_owned(),
                "account_id" => account_id = value.into_owned(),
                "calendar_href" => calendar_href = value.into_owned(),
                "scope" => scope = value.into_owned(),
                "original_dtstart" => original_dtstart = value.into_owned(),
                "tz" => tz = value.into_owned(),
                _ => {}
            }
        }
    }
    if uid.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing uid").into_response();
    }
    if calendar_href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing calendar_href").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }

    let client = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if scope == "this_instance" && !original_dtstart.is_empty() {
        let raw_ical =
            match fetch_event_ical(&state, &client, &account_id, &calendar_href, &uid).await {
                Ok(Some(s)) => s,
                Ok(None) => return (StatusCode::NOT_FOUND, "Event not found").into_response(),
                Err(e) => {
                    eprintln!("Failed to GET event for instance delete: {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to get event: {e}"),
                    )
                        .into_response();
                }
            };
        let tz_ref = if tz.is_empty() { "UTC" } else { &tz };
        let modified = add_exdate(&raw_ical, &original_dtstart, tz_ref);
        return match client.put_event(&calendar_href, &uid, &modified).await {
            Ok(_) => {
                let event_href = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
                optimistic_upsert(
                    &state,
                    &user.user_id,
                    &account_id,
                    &calendar_href,
                    &event_href,
                    &modified,
                    &uid,
                )
                .await;
                state
                    .sync_manager
                    .trigger(&user.user_id, SyncTrigger::Incremental);
                StatusCode::OK.into_response()
            }
            Err(e) => {
                eprintln!("Failed to PUT event after instance delete: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to update event: {e}"),
                )
                    .into_response()
            }
        };
    }

    match client.delete_event(&calendar_href, &uid).await {
        Ok(_) => {
            let _ = db::delete_cached_event(&state.db, &account_id, &calendar_href, &uid).await;
            state.sse_broadcaster.send(
                &user.user_id,
                crate::sse::SseMessage::EventRemoved {
                    account_id: account_id.clone(),
                    uid: uid.clone(),
                    calendar_href: calendar_href.clone(),
                },
            );
            state
                .sync_manager
                .trigger(&user.user_id, SyncTrigger::Incremental);
            StatusCode::OK.into_response()
        }
        Err(e) => {
            eprintln!("Failed to delete event: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete event: {e}"),
            )
                .into_response()
        }
    }
}

/// Current iCalendar data of an event. Demo mode has no CalDAV server, so the
/// local cache is the source of truth there; `Ok(None)` means not cached.
async fn fetch_event_ical(
    state: &AppState,
    client: &CalDavClient,
    account_id: &str,
    calendar_href: &str,
    uid: &str,
) -> anyhow::Result<Option<String>> {
    if state.demo_mode {
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT raw_ical FROM cached_events WHERE account_id = ? AND calendar_href = ? AND uid = ? LIMIT 1",
        )
        .bind(account_id)
        .bind(calendar_href)
        .bind(uid)
        .fetch_optional(&state.db)
        .await?;
        return Ok(raw);
    }
    client.get_event(calendar_href, uid).await.map(Some)
}

pub(super) async fn optimistic_upsert(
    state: &AppState,
    user_id: &str,
    account_id: &str,
    calendar_href: &str,
    event_href: &str,
    raw_ical: &str,
    uid: &str,
) {
    if let Err(e) = db::upsert_cached_event_raw(
        &state.db,
        account_id,
        calendar_href,
        event_href,
        "",
        raw_ical,
        &state.search_mode,
    )
    .await
    {
        eprintln!("Optimistic cache update failed: {e}");
    }
    let event_json = if !raw_ical.contains("RRULE:") {
        let today = chrono::Local::now().date_naive();
        let wide_start = today - chrono::TimeDelta::days(365);
        let wide_end = today + chrono::TimeDelta::days(730);
        let calendars = db::get_cached_calendars(&state.db, user_id)
            .await
            .unwrap_or_default();
        let color = calendars
            .iter()
            .find(|c| c.account_id == account_id && c.href == calendar_href)
            .and_then(|c| c.color.as_deref())
            .unwrap_or("");
        CalDavClient::events_from_raw_ical(raw_ical, wide_start, wide_end)
            .into_iter()
            .next()
            .and_then(|ev| {
                serde_json::to_string(&build_event_json(
                    &ev,
                    account_id,
                    color,
                    calendar_href,
                    None,
                ))
                .ok()
            })
    } else {
        None
    };
    state.sse_broadcaster.send(
        user_id,
        crate::sse::SseMessage::EventUpdated {
            account_id: account_id.to_string(),
            uid: uid.to_string(),
            calendar_href: calendar_href.to_string(),
            event_json,
        },
    );
}

#[utoipa::path(
    post,
    path = "/calendar",
    request_body(content = CreateCalendarForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, body = CalendarJson),
        (status = 400, description = "missing `name` or `account_id` (plain text)"),
        (status = 404, description = "no such CalDAV account (plain text)"),
        (status = 502, description = "the CalDAV server refused MKCALENDAR (plain text)")
    )
)]
pub async fn create_calendar_handler(
    State(state): State<crate::oidc::AppState>,
    user: CurrentUser,
    body: String,
) -> Response {
    let mut account_id = String::new();
    let mut name = String::new();
    let mut color: Option<String> = None;
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "account_id" => account_id = value.into_owned(),
            "name" => name = value.into_owned(),
            "color" => {
                let v = value.into_owned();
                if !v.is_empty() {
                    color = Some(v);
                }
            }
            _ => {}
        }
    }

    let name = name.trim().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing calendar name").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }
    let client = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    match client.create_calendar(&name, color.as_deref()).await {
        Ok(href) => {
            if let Err(e) =
                db::upsert_cached_calendar(&state.db, &account_id, &href, &name, color.as_deref())
                    .await
            {
                eprintln!("Failed to cache new calendar: {e}");
            }
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
            state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);
            Json(CalendarJson {
                account_id,
                href,
                name,
                color,
                is_active: true,
            })
            .into_response()
        }
        Err(e) => {
            eprintln!("MKCALENDAR failed: {e}");
            (StatusCode::BAD_GATEWAY, format!("{e}")).into_response()
        }
    }
}

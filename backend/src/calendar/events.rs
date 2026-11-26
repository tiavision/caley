use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use axum_extra::extract::{Form, Query};
use serde::Deserialize;

use super::{
    CalendarJson, build_event_json, extract_vevent, parse_alarm_from_fields, require_account_client,
};
use crate::caldav::{
    Alarm, Attendee, CalDavClient, PartStat, RecurrenceInfo, add_exdate, add_recurrence_override,
    generate_ical_event, generate_new_uid, update_vcalendar_event,
};
use crate::db;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;
use utoipa::{IntoParams, ToSchema};

/// What an event write applies to.
#[derive(Clone, Copy, PartialEq, Eq, ToSchema)]
#[schema(rename_all = "snake_case")]
pub(crate) enum EventScope {
    /// the event (all occurrences of a recurring one)
    Series,
    /// one occurrence of a recurring event (`original_dtstart`)
    ThisInstance,
}

impl std::str::FromStr for EventScope {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "series" => Ok(EventScope::Series),
            "this_instance" => Ok(EventScope::ThisInstance),
            _ => Err(format!(
                "invalid scope `{s}` (expected `series` or `this_instance`)"
            )),
        }
    }
}

/// `RecurrenceInfo` sent as JSON in a form field; empty or `{}` is none.
fn recurrence_json<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<RecurrenceInfo>, D::Error> {
    use serde::Deserialize as _;
    match Option::<String>::deserialize(de)?.as_deref() {
        None | Some("") | Some("{}") => Ok(None),
        Some(json) => serde_json::from_str(json)
            .map(Some)
            .map_err(|e| serde::de::Error::custom(format!("invalid recurrence: {e}"))),
    }
}

/// An event to create (no `uid`) or save. The alarm and attendee fields repeat, once
/// per alarm / attendee, in the same order. Empty optional fields count as missing.
#[derive(Deserialize, ToSchema)]
pub(crate) struct EventForm {
    /// missing: a new event
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    uid: Option<String>,
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: String,
    /// `YYYY-MM-DD`
    date: String,
    /// `HH:MM` (default `00:00`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    start_time: Option<String>,
    /// `HH:MM` (default `start_time`, or `01:00` without one)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    end_time: Option<String>,
    /// IANA timezone (default `UTC`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    tz: Option<String>,
    /// `true` for an all-day event
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    is_all_day: Option<bool>,
    calendar_href: String,
    account_id: String,
    /// default `series`
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    scope: Option<EventScope>,
    /// the occurrence's original start (`this_instance`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    original_dtstart: Option<String>,
    /// the timezone the recurring master was authored in (default `tz`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    original_tz: Option<String>,
    /// the calendar the event was in, when it moves
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    original_calendar_href: Option<String>,
    /// the account the event was in, when it moves
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    original_account_id: Option<String>,
    /// `RecurrenceInfo` as JSON; empty or `{}` for none
    #[serde(default, deserialize_with = "recurrence_json")]
    #[schema(value_type = Option<String>)]
    recurrence: Option<RecurrenceInfo>,
    /// `before-start`, `after-start`, `before-end`, `after-end` or `absolute`
    #[serde(default)]
    alarm_direction: Vec<String>,
    /// the amount of `alarm_unit`s (relative alarms)
    #[serde(default)]
    alarm_value: Vec<String>,
    /// minutes per unit (default `1`)
    #[serde(default)]
    alarm_unit: Vec<String>,
    /// `YYYY-MM-DDTHH:MM` in `tz` (absolute alarms)
    #[serde(default)]
    alarm_datetime: Vec<String>,
    #[serde(default)]
    attendee_cn: Vec<String>,
    /// an attendee without an address is left out
    #[serde(default)]
    attendee_email: Vec<String>,
    /// iCalendar PARTSTAT (default `NEEDS-ACTION`)
    #[serde(default)]
    attendee_partstat: Vec<String>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeleteEventQuery {
    uid: String,
    account_id: String,
    calendar_href: String,
    /// `this_instance` (with `original_dtstart`) deletes one occurrence; else the event
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(inline)]
    scope: Option<EventScope>,
    /// the occurrence's original start (`this_instance`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    original_dtstart: Option<String>,
    /// the occurrence's timezone (default `UTC`)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    tz: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct CreateCalendarForm {
    account_id: String,
    name: String,
    /// CSS color; empty or missing for none
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
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
    Form(EventForm {
        uid,
        summary,
        description,
        location,
        date,
        start_time,
        end_time,
        tz,
        is_all_day,
        calendar_href,
        account_id,
        scope,
        original_dtstart,
        original_tz,
        original_calendar_href,
        original_account_id,
        recurrence,
        alarm_direction,
        alarm_value,
        alarm_unit,
        alarm_datetime,
        attendee_cn,
        attendee_email,
        attendee_partstat,
    }): Form<EventForm>,
) -> Response {
    let is_edit = uid.is_some();
    let uid = uid.unwrap_or_else(generate_new_uid);
    let start_time = start_time.as_deref();
    let end_time = end_time.as_deref();
    let tz = tz.unwrap_or_else(|| "UTC".to_string());
    let is_all_day = is_all_day.unwrap_or(false);
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }
    let scope = scope.unwrap_or(EventScope::Series);
    let original_dtstart = original_dtstart.as_deref();
    // Timezone the recurring master was authored in; the RECURRENCE-ID must
    // stay in that zone even when the edited override is saved in `tz`.
    let original_tz = original_tz.unwrap_or_else(|| tz.clone());

    let alarms: Vec<Alarm> = alarm_direction
        .iter()
        .enumerate()
        .filter_map(|(i, dir)| {
            parse_alarm_from_fields(
                dir,
                alarm_value.get(i).map_or("", String::as_str),
                alarm_unit.get(i).map_or("1", String::as_str),
                alarm_datetime.get(i).map_or("", String::as_str),
                &tz,
            )
        })
        .collect();

    let attendees: Vec<Attendee> = attendee_email
        .iter()
        .enumerate()
        .filter_map(|(i, email)| {
            let email = email.trim();
            if email.is_empty() {
                return None;
            }
            Some(Attendee {
                cn: attendee_cn.get(i).cloned().unwrap_or_default(),
                email: email.to_string(),
                partstat: PartStat::from_ical(attendee_partstat.get(i).map_or("", String::as_str)),
            })
        })
        .collect();

    if summary.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing summary").into_response();
    }
    if date.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing date").into_response();
    }

    let (account, client) = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let event_href = resolve_event_href(&state, account.id(), &calendar_href, &uid).await;

    if scope == EventScope::ThisInstance {
        let raw_ical = match fetch_event_ical(&state, &client, account.id(), &calendar_href, &uid)
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

        match client.put_event(&event_href, &modified).await {
            Ok(_) => {
                optimistic_upsert(
                    &state,
                    &user.user_id,
                    account.id(),
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
        let original = match original_account_id.as_deref() {
            Some(orig_account_id) if orig_account_id != account_id => {
                match require_account_client(&state, &user, orig_account_id).await {
                    Ok(c) => Some(c),
                    Err(r) => return r,
                }
            }
            _ => None,
        };
        let (original_account, original_client) = match &original {
            Some((a, c)) => (a, Some(c)),
            None => (&account, None),
        };
        let ical = if is_edit {
            let original = fetch_event_ical(
                &state,
                original_client.unwrap_or(&client),
                original_account.id(),
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

        match client.put_event(&event_href, &ical).await {
            Ok(_) => {
                if let Some(ref orig) = original_calendar_href {
                    let moved_account = original_account_id
                        .as_deref()
                        .is_some_and(|id| id != account_id);
                    if (!orig.is_empty()) && (*orig != calendar_href || moved_account) {
                        let delete_client = original_client.unwrap_or(&client);
                        let old_href =
                            resolve_event_href(&state, original_account.id(), orig, &uid).await;
                        if let Err(e) = delete_client.delete_event(&old_href).await {
                            eprintln!("Failed to delete event from old calendar: {e}");
                        } else {
                            let _ = db::delete_event_by_href(
                                &state.db,
                                original_account.id().as_str(),
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
                    account.id(),
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
    Query(DeleteEventQuery {
        uid,
        account_id,
        calendar_href,
        scope,
        original_dtstart,
        tz,
    }): Query<DeleteEventQuery>,
) -> Response {
    if uid.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing uid").into_response();
    }
    if calendar_href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing calendar_href").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }

    let (account, client) = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    if let (Some(EventScope::ThisInstance), Some(original_dtstart)) = (scope, original_dtstart) {
        let raw_ical =
            match fetch_event_ical(&state, &client, account.id(), &calendar_href, &uid).await {
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
        let modified = add_exdate(&raw_ical, &original_dtstart, tz.as_deref().unwrap_or("UTC"));
        let event_href = resolve_event_href(&state, account.id(), &calendar_href, &uid).await;
        return match client.put_event(&event_href, &modified).await {
            Ok(_) => {
                optimistic_upsert(
                    &state,
                    &user.user_id,
                    account.id(),
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

    let href = resolve_event_href(&state, account.id(), &calendar_href, &uid).await;
    match client.delete_event(&href).await {
        Ok(_) => {
            let _ = db::delete_cached_event(&state.db, account.id(), &calendar_href, &uid).await;
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
    account: db::CalDavAccountId<'_>,
    calendar_href: &str,
    uid: &str,
) -> anyhow::Result<Option<String>> {
    if state.demo_mode {
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT raw_ical FROM cached_events WHERE account_id = ? AND calendar_href = ? AND uid = ? LIMIT 1",
        )
        .bind(account.as_str())
        .bind(calendar_href)
        .bind(uid)
        .fetch_optional(&state.db)
        .await?;
        return Ok(raw);
    }
    let href = resolve_event_href(state, account, calendar_href, uid).await;
    client.get_event(&href).await.map(Some)
}

/// The href of the event `uid` in `calendar_href`: the one the server gave it when it is cached, else
/// where a new one goes (`caldav::new_event_href`, a name the UID cannot turn into another path).
pub(super) async fn resolve_event_href(
    state: &AppState,
    account: db::CalDavAccountId<'_>,
    calendar_href: &str,
    uid: &str,
) -> String {
    match db::get_event_href(&state.db, account, calendar_href, uid).await {
        Ok(Some(href)) => href,
        _ => crate::caldav::new_event_href(calendar_href, uid),
    }
}

pub(super) async fn optimistic_upsert(
    state: &AppState,
    user_id: &str,
    account: db::CalDavAccountId<'_>,
    calendar_href: &str,
    event_href: &str,
    raw_ical: &str,
    uid: &str,
) {
    let account_id = account.as_str();
    if let Err(e) = db::upsert_cached_event_raw(
        &state.db,
        account,
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
    Form(CreateCalendarForm {
        account_id,
        name,
        color,
    }): Form<CreateCalendarForm>,
) -> Response {
    let name = name.trim().to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing calendar name").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id").into_response();
    }
    let (account, client) = match require_account_client(&state, &user, &account_id).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    match create_calendar(
        &state,
        &user,
        &client,
        account.id(),
        &name,
        color.as_deref(),
    )
    .await
    {
        Ok(href) => Json(CalendarJson {
            account_id,
            href,
            name,
            color,
            is_active: true,
        })
        .into_response(),
        Err(r) => r,
    }
}

/// Creates a calendar on the account's CalDAV server (MKCALENDAR) and caches it: its href, or a
/// 502 with the server's error.
pub(super) async fn create_calendar(
    state: &AppState,
    user: &CurrentUser,
    client: &CalDavClient,
    account: db::CalDavAccountId<'_>,
    name: &str,
    color: Option<&str>,
) -> Result<String, Response> {
    match client.create_calendar(name, color).await {
        Ok(href) => {
            if let Err(e) =
                db::upsert_cached_calendar(&state.db, account.as_str(), &href, name, color).await
            {
                eprintln!("Failed to cache new calendar: {e}");
            }
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
            state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);
            Ok(href)
        }
        Err(e) => {
            eprintln!("MKCALENDAR failed: {e}");
            Err((StatusCode::BAD_GATEWAY, format!("{e}")).into_response())
        }
    }
}

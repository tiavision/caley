use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use axum_extra::extract::{Form, Query};
use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::caldav::{Alarm, CalDavClient, PartStat, RecurrenceInfo};
use crate::db;
use crate::oidc::{AppState, CurrentUser};

pub(crate) mod events;
pub(crate) mod import;

pub(super) const NEUTRAL_DOT_COLOR: &str = "var(--tm-text-muted)";

// ── JSON response types ────────────────────────────────────────────────────────

#[derive(Serialize, Clone, ToSchema)]
pub struct CalendarJson {
    pub account_id: String,
    pub href: String,
    pub name: String,
    #[schema(required)]
    pub color: Option<String>,
    pub is_active: bool,
}

/// An alarm as the event form edits it.
#[derive(Serialize, Clone, ToSchema)]
pub struct AlarmFields {
    /// `before-start`, `after-start`, `before-end`, `after-end` or `absolute`
    pub direction: String,
    /// the amount of `unit`s (relative alarms; empty for absolute ones)
    pub value: String,
    /// minutes per unit: `1`, `60` or `1440` (empty for absolute alarms)
    pub unit: String,
    /// UTC, `YYYYMMDDTHHMMSSZ` (absolute alarms; empty for relative ones)
    pub datetime: String,
}

#[derive(Serialize, Clone, ToSchema)]
pub struct AttendeeFields {
    pub cn: String,
    pub email: String,
    pub partstat: PartStat,
}

/// An event (one occurrence of a recurring one), times in the viewer's timezone.
#[derive(Serialize, Clone, ToSchema)]
pub struct EventJson {
    pub account_id: String,
    pub uid: String,
    /// `HH:MM-HH:MM`, `HH:MM` or `All day`
    pub time_range: String,
    pub summary: String,
    /// the calendar's color, or a CSS variable for calendars without one
    pub color: String,
    pub dtstart: String,
    pub dtend: String,
    pub is_all_day: bool,
    pub tz: String,
    pub location: String,
    pub description: String,
    pub is_recurring: bool,
    #[schema(required)]
    pub recurrence: Option<RecurrenceInfo>,
    pub calendar_href: String,
    pub master_dtstart: String,
    pub alarms: Vec<AlarmFields>,
    pub attendees: Vec<AttendeeFields>,
}

/// A day of the week view with its events, all-day ones first.
#[derive(Serialize, ToSchema)]
pub struct DayEventsJson {
    /// `YYYY-MM-DD`
    pub date_iso: String,
    pub day_name: String,
    pub is_today: bool,
    pub events: Vec<EventJson>,
}

/// A cell of the month grid.
#[derive(Serialize, ToSchema)]
pub struct MonthDayJson {
    pub day_number: u32,
    pub date_str: String,
    pub is_current_month: bool,
    /// the distinct colors of the day's events
    pub event_colors: Vec<String>,
}

/// A month as whole weeks (Monday first).
#[derive(Serialize, ToSchema)]
pub struct MonthGridJson {
    /// `YYYY-MM`
    pub month: String,
    pub month_name: String,
    pub month_number: u32,
    pub year: i32,
    pub days: Vec<MonthDayJson>,
    pub prev_month: String,
    pub next_month: String,
}

// ── Alarm helpers ─────────────────────────────────────────────────────────────

fn alarm_to_fields(alarm: &Alarm) -> AlarmFields {
    if alarm.trigger_type == "absolute" {
        return AlarmFields {
            direction: "absolute".into(),
            value: String::new(),
            unit: String::new(),
            datetime: alarm.datetime.clone().unwrap_or_default(),
        };
    }
    let mins = alarm.minutes.unwrap_or(0);
    let direction = match (alarm.trigger_type.as_str(), mins < 0) {
        ("start", true) => "before-start",
        ("start", false) => "after-start",
        ("end", true) => "before-end",
        _ => "after-end",
    };
    let abs = mins.unsigned_abs();
    let (val, unit) = if abs % 1440 == 0 {
        (abs / 1440, 1440u64)
    } else if abs % 60 == 0 {
        (abs / 60, 60u64)
    } else {
        (abs, 1u64)
    };
    AlarmFields {
        direction: direction.into(),
        value: val.to_string(),
        unit: unit.to_string(),
        datetime: String::new(),
    }
}

pub(super) fn parse_alarm_from_fields(
    direction: &str,
    value: &str,
    unit: &str,
    datetime: &str,
    tz: &str,
) -> Option<Alarm> {
    if direction == "absolute" {
        if datetime.is_empty() {
            return None;
        }
        let naive = NaiveDateTime::parse_from_str(datetime, "%Y-%m-%dT%H:%M").ok()?;
        let event_tz: chrono_tz::Tz = tz.parse().ok()?;
        let zoned = event_tz.from_local_datetime(&naive).single()?;
        let utc = zoned.with_timezone(&Utc);
        Some(Alarm {
            trigger_type: "absolute".into(),
            minutes: None,
            datetime: Some(utc.format("%Y%m%dT%H%M%SZ").to_string()),
        })
    } else {
        let val = value.parse::<i64>().ok().filter(|&v| v > 0)?;
        let mult = unit.parse::<i64>().unwrap_or(1).max(1);
        let total = val * mult;
        let (trigger_type, sign): (&str, i64) = match direction {
            "before-start" => ("start", -1),
            "after-start" => ("start", 1),
            "before-end" => ("end", -1),
            "after-end" => ("end", 1),
            _ => return None,
        };
        Some(Alarm {
            trigger_type: trigger_type.into(),
            minutes: Some(sign * total),
            datetime: None,
        })
    }
}

pub(super) fn extract_vevent(ical: &str) -> String {
    let ical = ical.replace("\r\n", "\n");
    if let Some(start) = ical.find("BEGIN:VEVENT") {
        let from_start = &ical[start..];
        if let Some(end) = from_start.find("\nEND:VEVENT") {
            let vevent = &from_start[..end + 1 + 10];
            return vevent.replace('\n', "\r\n");
        }
    }
    ical.replace('\n', "\r\n")
}

// ── Event building helper ─────────────────────────────────────────────────────

/// The user's CalDAV/CardDAV account `account_id`: the one way a handler turns an id from a request into an
/// account, whose [`db::CalDavAccount::id`] the cache functions require. 404 when it is not the user's (or does not
/// exist: the two are not told apart).
pub(crate) async fn owned_account(
    state: &AppState,
    user_id: &str,
    account_id: &str,
) -> Result<db::CalDavAccount, Response> {
    match db::get_caldav_account(&state.db, user_id, account_id, &state.encryption_key).await {
        Ok(Some(account)) => Ok(account),
        Ok(None) => Err((StatusCode::NOT_FOUND, "CalDAV account not found").into_response()),
        Err(e) => {
            eprintln!("Database error: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response())
        }
    }
}

/// The user's account `account_id` (as [`owned_account`]) with a client for its server (demo mode: none).
pub(crate) async fn require_account_client(
    state: &AppState,
    user: &CurrentUser,
    account_id: &str,
) -> Result<(db::CalDavAccount, CalDavClient), Response> {
    let account = owned_account(state, &user.user_id, account_id).await?;
    if state.demo_mode {
        return Ok((account, CalDavClient::demo()));
    }
    let client = CalDavClient::new(
        account.caldav_base_url.clone(),
        account.caldav_username.clone(),
        account.caldav_password.clone(),
    )
    .map_err(|e| {
        eprintln!("CalDAV client error: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "CalDAV configuration error",
        )
            .into_response()
    })?;
    Ok((account, client))
}

/// Today's date in the viewer's timezone, falling back to server-local time
/// when the request carried no usable `tz` parameter.
pub(super) fn viewer_today(viewer_tz: Option<chrono_tz::Tz>) -> NaiveDate {
    match viewer_tz {
        Some(tz) => chrono::Utc::now().with_timezone(&tz).date_naive(),
        None => chrono::Local::now().date_naive(),
    }
}

/// Viewer-local (date, start "HH:MM", end "HH:MM") for an expanded event.
/// Floating and all-day events pass through their wall-clock parts, as does
/// everything when no viewer timezone is known.
pub(super) fn viewer_local_parts(
    event: &crate::caldav::Event,
    viewer_tz: Option<chrono_tz::Tz>,
) -> (NaiveDate, Option<String>, Option<String>) {
    use chrono::TimeZone as _;
    let (Some(tz), Some(start_ts)) = (viewer_tz, event.start_ts) else {
        return (event.date, event.start_time.clone(), event.end_time.clone());
    };
    let start = tz.timestamp_opt(start_ts, 0).unwrap();
    let end_time = event
        .end_ts
        .map(|ts| tz.timestamp_opt(ts, 0).unwrap().format("%H:%M").to_string());
    (
        start.date_naive(),
        Some(start.format("%H:%M").to_string()),
        end_time,
    )
}

pub(super) fn build_event_json(
    event: &crate::caldav::Event,
    account_id: &str,
    color: &str,
    calendar_href: &str,
    viewer_tz: Option<chrono_tz::Tz>,
) -> EventJson {
    let (_, start_time, end_time) = viewer_local_parts(event, viewer_tz);
    let time_range = match (start_time, end_time) {
        (Some(s), Some(e)) => format!("{}-{}", s, e),
        (Some(s), None) => s,
        (None, _) => "All day".to_string(),
    };
    let dot_color = if color.is_empty() {
        NEUTRAL_DOT_COLOR.to_string()
    } else {
        color.to_string()
    };
    let alarms = event.alarms.iter().map(alarm_to_fields).collect();
    let attendees = event
        .attendees
        .iter()
        .map(|a| AttendeeFields {
            cn: a.cn.clone(),
            email: a.email.clone(),
            partstat: a.partstat.clone(),
        })
        .collect();
    EventJson {
        account_id: account_id.to_string(),
        uid: event.uid.clone(),
        time_range,
        summary: event.summary.clone(),
        color: dot_color,
        dtstart: event.dtstart.clone(),
        dtend: event.dtend.clone().unwrap_or_default(),
        is_all_day: event.is_all_day,
        tz: event.tz.clone(),
        location: event.location.clone(),
        description: event.description.clone(),
        is_recurring: event.is_recurring,
        recurrence: event.recurrence.clone(),
        calendar_href: calendar_href.to_string(),
        master_dtstart: event.master_dtstart.clone(),
        alarms,
        attendees,
    }
}

// ── Queries and forms ────────────────────────────────────────────────────────

/// A viewer's IANA timezone; one this server does not know is no timezone (the server's is used),
/// as browsers may name zones the tz database here lacks.
pub(super) fn viewer_tz<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<chrono_tz::Tz>, D::Error> {
    use serde::Deserialize as _;
    Ok(Option::<String>::deserialize(de)?.and_then(|s| s.parse().ok()))
}

/// The first day of a `YYYY-MM` month.
pub(crate) struct Month(NaiveDate);

impl std::str::FromStr for Month {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d")
            .map(Month)
            .map_err(|_| format!("invalid month `{s}` (YYYY-MM)"))
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EventsQuery {
    /// first day, `YYYY-MM-DD` (default: Monday of the current week)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(value_type = Option<String>, format = Date)]
    start: Option<NaiveDate>,
    /// last day, `YYYY-MM-DD` (default: Sunday of the current week)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(value_type = Option<String>, format = Date)]
    end: Option<NaiveDate>,
    /// the viewer's IANA timezone (default, also for an unknown one: the server's)
    #[serde(default, deserialize_with = "viewer_tz")]
    #[param(value_type = Option<String>)]
    tz: Option<chrono_tz::Tz>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MonthQuery {
    /// `YYYY-MM` (default: the current month)
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    #[param(value_type = Option<String>)]
    month: Option<Month>,
    /// the viewer's IANA timezone (default, also for an unknown one: the server's)
    #[serde(default, deserialize_with = "viewer_tz")]
    #[param(value_type = Option<String>)]
    tz: Option<chrono_tz::Tz>,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct ToggleCalendarForm {
    account_id: String,
    href: String,
    /// `false` hides the calendar, `true` (or missing) shows it
    #[serde(default, deserialize_with = "crate::api::empty_as_none")]
    active: Option<bool>,
}

// ── Read handlers (serve from cache) ─────────────────────────────────────────

#[utoipa::path(get, path = "/calendars", responses((status = 200, body = Vec<CalendarJson>)))]

pub async fn list_calendars(State(state): State<AppState>, user: CurrentUser) -> Response {
    let calendars = match db::get_cached_calendars(&state.db, &user.user_id).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("DB error listing calendars: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };
    let mut result: Vec<CalendarJson> = Vec::new();
    for c in calendars {
        let deactivated =
            db::get_deactivated_calendars(&state.db, &c.account_id, &state.encryption_key)
                .await
                .unwrap_or_default();
        result.push(CalendarJson {
            is_active: !deactivated.contains(&c.href),
            account_id: c.account_id,
            href: c.href,
            name: c.name,
            color: c.color,
        });
    }

    Json(result).into_response()
}

#[utoipa::path(
    get,
    path = "/events",
    params(EventsQuery),
    responses((status = 200, description = "one entry per day from `start` to `end`", body = Vec<DayEventsJson>))
)]
pub async fn list_events(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(EventsQuery {
        start,
        end,
        tz: viewer_tz,
    }): Query<EventsQuery>,
) -> Response {
    let today = viewer_today(viewer_tz);
    let weekday = today.weekday().num_days_from_monday();
    let week_start = today - chrono::TimeDelta::days(weekday as i64);
    let start = start.unwrap_or(week_start);
    let end = end.unwrap_or(week_start + chrono::TimeDelta::days(6));

    let calendars = db::get_cached_calendars(&state.db, &user.user_id)
        .await
        .unwrap_or_default();
    let color_map: HashMap<String, String> = calendars
        .iter()
        .map(|c| {
            (
                format!("{}\0{}", c.account_id, c.href),
                c.color.clone().unwrap_or_default(),
            )
        })
        .collect();

    let mut active_hrefs: Vec<(String, String)> = Vec::new();
    for c in &calendars {
        let deactivated =
            db::get_deactivated_calendars(&state.db, &c.account_id, &state.encryption_key)
                .await
                .unwrap_or_default();
        if !deactivated.contains(&c.href) {
            active_hrefs.push((c.account_id.clone(), c.href.clone()));
        }
    }

    // Source-tz date columns and source-tz expansion can differ from the
    // viewer-local bucket date by up to a day (real-world offsets < ±14h),
    // so fetch and expand with one day of slack on both sides.
    let fetch_start = start - chrono::TimeDelta::days(1);
    let fetch_end = end + chrono::TimeDelta::days(1);
    let start_str = fetch_start.format("%Y-%m-%d").to_string();
    let end_str = fetch_end.format("%Y-%m-%d").to_string();

    let raw_pairs = db::get_raw_events(&state.db, &active_hrefs, &start_str, &end_str)
        .await
        .unwrap_or_default();

    let mut all_events: Vec<(NaiveDate, EventJson)> = Vec::new();
    for (account_id, calendar_href, raw_ical) in raw_pairs {
        let color = color_map
            .get(&format!("{}\0{}", account_id, calendar_href))
            .map(|s| s.as_str())
            .unwrap_or("");
        for event in CalDavClient::events_from_raw_ical(&raw_ical, fetch_start, fetch_end) {
            let (local_date, _, _) = viewer_local_parts(&event, viewer_tz);
            let json = build_event_json(&event, &account_id, color, &calendar_href, viewer_tz);
            all_events.push((local_date, json));
        }
    }

    let num_days = (end - start).num_days() + 1;
    let days: Vec<DayEventsJson> = (0..num_days)
        .map(|i| {
            let day = start + chrono::TimeDelta::days(i);
            let mut events: Vec<EventJson> = all_events
                .iter()
                .filter(|(d, _)| *d == day)
                .map(|(_, ev)| ev.clone())
                .collect();
            events.sort_by(|a, b| {
                let a_all_day = a.time_range == "All day";
                let b_all_day = b.time_range == "All day";
                if a_all_day != b_all_day {
                    b_all_day.cmp(&a_all_day)
                } else {
                    a.time_range.cmp(&b.time_range)
                }
            });
            DayEventsJson {
                date_iso: day.format("%Y-%m-%d").to_string(),
                day_name: day.format("%A").to_string(),
                is_today: day == today,
                events,
            }
        })
        .collect();

    Json(days).into_response()
}

#[utoipa::path(get, path = "/calendar", params(MonthQuery), responses((status = 200, body = MonthGridJson)))]
pub async fn list_calendar(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(MonthQuery {
        month,
        tz: viewer_tz,
    }): Query<MonthQuery>,
) -> Response {
    let today = viewer_today(viewer_tz);

    let first_day = match month {
        Some(Month(first)) => first,
        None => today.with_day(1).unwrap_or(today),
    };

    let next_first = if first_day.month() == 12 {
        NaiveDate::from_ymd_opt(first_day.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(first_day.year(), first_day.month() + 1, 1).unwrap()
    };
    let last_day = next_first - chrono::TimeDelta::days(1);
    let prev_first = if first_day.month() == 1 {
        NaiveDate::from_ymd_opt(first_day.year() - 1, 12, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(first_day.year(), first_day.month() - 1, 1).unwrap()
    };

    let calendars = db::get_cached_calendars(&state.db, &user.user_id)
        .await
        .unwrap_or_default();
    let color_map: HashMap<String, String> = calendars
        .iter()
        .map(|c| {
            (
                format!("{}\0{}", c.account_id, c.href),
                c.color.clone().unwrap_or_default(),
            )
        })
        .collect();

    let mut active_hrefs: Vec<(String, String)> = Vec::new();
    for c in &calendars {
        let deactivated =
            db::get_deactivated_calendars(&state.db, &c.account_id, &state.encryption_key)
                .await
                .unwrap_or_default();
        if !deactivated.contains(&c.href) {
            active_hrefs.push((c.account_id.clone(), c.href.clone()));
        }
    }

    // ±1 day slack: the SQL date columns and expansion window are source-tz
    // while the dots are bucketed by viewer-local date.
    let fetch_start = first_day - chrono::TimeDelta::days(1);
    let fetch_end = last_day + chrono::TimeDelta::days(1);

    let mut colors_map: HashMap<NaiveDate, Vec<String>> = HashMap::new();

    let raw_pairs = db::get_raw_events(
        &state.db,
        &active_hrefs,
        &fetch_start.format("%Y-%m-%d").to_string(),
        &fetch_end.format("%Y-%m-%d").to_string(),
    )
    .await
    .unwrap_or_default();
    for (account_id, calendar_href, raw_ical) in raw_pairs {
        let color = color_map
            .get(&format!("{}\0{}", account_id, calendar_href))
            .map(|s| s.as_str())
            .unwrap_or("");
        let dot_color = if color.is_empty() {
            NEUTRAL_DOT_COLOR.to_string()
        } else {
            color.to_string()
        };
        for event in CalDavClient::events_from_raw_ical(&raw_ical, fetch_start, fetch_end) {
            let (date, _, _) = viewer_local_parts(&event, viewer_tz);
            if date >= first_day && date <= last_day {
                colors_map.entry(date).or_default().push(dot_color.clone());
            }
        }
    }

    for colors in colors_map.values_mut() {
        colors.sort();
        colors.dedup();
    }

    let start_offset = first_day.weekday().num_days_from_monday();
    let days_in_month = last_day.day();
    let total_cells = ((start_offset + days_in_month + 6) / 7) * 7;
    let grid_start = first_day - chrono::TimeDelta::days(start_offset as i64);

    let days: Vec<MonthDayJson> = (0..total_cells)
        .map(|i| {
            let cell_date = grid_start + chrono::TimeDelta::days(i as i64);
            let is_current_month =
                cell_date.month() == first_day.month() && cell_date.year() == first_day.year();
            let event_colors = colors_map.remove(&cell_date).unwrap_or_default();
            MonthDayJson {
                day_number: cell_date.day(),
                date_str: cell_date.format("%Y-%m-%d").to_string(),
                is_current_month,
                event_colors,
            }
        })
        .collect();

    let tpl = MonthGridJson {
        month: first_day.format("%Y-%m").to_string(),
        month_name: first_day.format("%B").to_string(),
        month_number: first_day.month(),
        year: first_day.year(),
        days,
        prev_month: prev_first.format("%Y-%m").to_string(),
        next_month: next_first.format("%Y-%m").to_string(),
    };

    Json(tpl).into_response()
}

// ── Write handlers (CalDAV + optimistic cache update) ─────────────────────────

#[utoipa::path(
    post,
    path = "/calendar/toggle",
    request_body(content = ToggleCalendarForm, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "saved (empty body)"),
        (status = 400, description = "missing `href` or `account_id` (plain text)"),
        (status = 404, description = "no such CalDAV account (plain text)")
    )
)]
pub async fn toggle_calendar(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(ToggleCalendarForm {
        account_id,
        href,
        active,
    }): Form<ToggleCalendarForm>,
) -> Response {
    let active = active.unwrap_or(true);
    if href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing href parameter").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id parameter").into_response();
    }
    let account = match owned_account(&state, &user.user_id, &account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };

    let result = if active {
        db::activate_calendar(&state.db, account.id(), &href, &state.encryption_key).await
    } else {
        db::deactivate_calendar(&state.db, account.id(), &href, &state.encryption_key).await
    };

    match result {
        Ok(_) => {
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
            StatusCode::OK.into_response()
        }
        Err(e) => {
            eprintln!("Database error toggling calendar: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// This module's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    use axum::extract::DefaultBodyLimit;
    const ICS_LIMIT: usize = 10 * 1024 * 1024;
    let (s, p, m) = routes!(import::import_ics);
    let import = (s, p, m.layer(DefaultBodyLimit::max(ICS_LIMIT)));
    let (s, p, m) = routes!(import::preview_ics_import);
    let preview = (s, p, m.layer(DefaultBodyLimit::max(ICS_LIMIT)));
    OpenApiRouter::new()
        .routes(routes!(list_calendars))
        .routes(routes!(toggle_calendar))
        .routes(routes!(list_events))
        .routes(routes!(list_calendar, events::create_calendar_handler))
        .routes(import)
        .routes(preview)
        .routes(routes!(events::save_event, events::delete_event))
        .routes(routes!(import::get_event_ics))
        .routes(routes!(import::get_message_events))
        .routes(routes!(import::get_thread_events))
        .routes(routes!(import::get_event_source_emails))
        .routes(routes!(import::search_events))
}

#[cfg(test)]
mod viewer_local_tests {
    use super::*;

    fn timed_event(
        date: &str,
        start_time: &str,
        start_ts: Option<i64>,
        end_ts: Option<i64>,
    ) -> crate::caldav::Event {
        crate::caldav::Event {
            uid: "test".into(),
            summary: String::new(),
            description: String::new(),
            location: String::new(),
            dtstart: String::new(),
            dtend: None,
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            start_time: Some(start_time.to_string()),
            end_time: None,
            start_ts,
            end_ts,
            is_all_day: false,
            tz: String::new(),
            is_recurring: false,
            recurrence: None,
            master_dtstart: String::new(),
            alarms: vec![],
            attendees: vec![],
        }
    }

    #[test]
    fn anchored_event_shifts_to_viewer_date_and_time() {
        // 2026-01-15T23:30:00-05:00 == 2026-01-16T04:30:00Z
        let ev = timed_event("2026-01-15", "23:30", Some(1768537800), Some(1768541400));
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let (date, start, end) = viewer_local_parts(&ev, Some(berlin));
        assert_eq!(date.to_string(), "2026-01-16");
        assert_eq!(start.as_deref(), Some("05:30"));
        assert_eq!(end.as_deref(), Some("06:30"));
    }

    #[test]
    fn floating_event_passes_wall_clock_through() {
        let ev = timed_event("2026-01-15", "09:00", None, None);
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let (date, start, _) = viewer_local_parts(&ev, Some(berlin));
        assert_eq!(date.to_string(), "2026-01-15");
        assert_eq!(start.as_deref(), Some("09:00"));
    }

    #[test]
    fn all_day_event_passes_through() {
        let mut ev = timed_event("2026-01-15", "00:00", None, None);
        ev.is_all_day = true;
        ev.start_time = None;
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let (date, start, end) = viewer_local_parts(&ev, Some(berlin));
        assert_eq!(date.to_string(), "2026-01-15");
        assert_eq!(start, None);
        assert_eq!(end, None);
    }

    #[test]
    fn missing_viewer_tz_passes_through() {
        let ev = timed_event("2026-01-15", "23:30", Some(1768537800), None);
        let (date, start, _) = viewer_local_parts(&ev, None);
        assert_eq!(date.to_string(), "2026-01-15");
        assert_eq!(start.as_deref(), Some("23:30"));
    }
}

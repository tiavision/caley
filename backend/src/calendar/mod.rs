use axum::{
    extract::{RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use serde::Serialize;
use std::collections::HashMap;
use url::form_urlencoded;

use crate::caldav::{Alarm, CalDavClient, PartStat, RecurrenceInfo};
use crate::db;
use crate::oidc::{AppState, CurrentUser};

pub(crate) mod events;
pub(crate) mod import;

pub use events::{create_calendar_handler, delete_event, save_event};
pub use import::{
    get_event_ics, get_event_source_emails, get_message_events, get_thread_events, import_ics,
    preview_ics_import, search_events,
};

pub(super) const NEUTRAL_DOT_COLOR: &str = "var(--pico-muted-color)";

// ── JSON response types ────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct CalendarJson {
    pub account_id: String,
    pub href: String,
    pub name: String,
    pub color: Option<String>,
    pub is_active: bool,
}

#[derive(Serialize, Clone)]
pub struct AlarmFields {
    pub direction: String,
    pub value: String,
    pub unit: String,
    pub datetime: String,
}

#[derive(Serialize, Clone)]
pub struct AttendeeFields {
    pub cn: String,
    pub email: String,
    pub partstat: PartStat,
}

#[derive(Serialize, Clone)]
pub struct EventJson {
    pub account_id: String,
    pub uid: String,
    pub time_range: String,
    pub summary: String,
    pub color: String,
    pub dtstart: String,
    pub dtend: String,
    pub is_all_day: bool,
    pub tz: String,
    pub location: String,
    pub description: String,
    pub is_recurring: bool,
    pub recurrence: Option<RecurrenceInfo>,
    pub calendar_href: String,
    pub master_dtstart: String,
    pub alarms: Vec<AlarmFields>,
    pub attendees: Vec<AttendeeFields>,
}

#[derive(Serialize)]
pub struct DayEventsJson {
    pub date_iso: String,
    pub day_name: String,
    pub is_today: bool,
    pub events: Vec<EventJson>,
}

#[derive(Serialize)]
pub struct MonthDayJson {
    pub day_number: u32,
    pub date_str: String,
    pub is_current_month: bool,
    pub event_colors: Vec<String>,
}

#[derive(Serialize)]
pub struct MonthGridJson {
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

// ── Form parsing helpers ──────────────────────────────────────────────────────

pub(super) fn parse_form_field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub(super) fn parse_event_form(body: String) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
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

pub(super) async fn require_account_client(
    state: &AppState,
    user: &CurrentUser,
    account_id: &str,
) -> Result<CalDavClient, Response> {
    if state.demo_mode {
        return Ok(CalDavClient::demo());
    }
    match db::get_caldav_account(&state.db, &user.user_id, account_id, &state.encryption_key).await
    {
        Ok(Some(account)) => CalDavClient::new(
            account.caldav_base_url,
            account.caldav_username,
            account.caldav_password,
        )
        .map_err(|e| {
            eprintln!("CalDAV client error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "CalDAV configuration error",
            )
                .into_response()
        }),
        Ok(None) => Err((StatusCode::NOT_FOUND, "CalDAV account not found").into_response()),
        Err(e) => {
            eprintln!("Database error: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response())
        }
    }
}

pub(super) fn build_event_json(
    event: &crate::caldav::Event,
    account_id: &str,
    color: &str,
    calendar_href: &str,
) -> EventJson {
    let time_range = match (&event.start_time, &event.end_time) {
        (Some(s), Some(e)) => format!("{}-{}", s, e),
        (Some(s), None) => s.clone(),
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

// ── Read handlers (serve from cache) ─────────────────────────────────────────

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

pub async fn list_events(
    State(state): State<AppState>,
    user: CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let today = chrono::Local::now().date_naive();
    let weekday = today.weekday().num_days_from_monday();
    let week_start = today - chrono::TimeDelta::days(weekday as i64);

    let mut start: Option<NaiveDate> = None;
    let mut end: Option<NaiveDate> = None;
    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "start" => start = NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
                "end" => end = NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
                _ => {}
            }
        }
    }
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

    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let raw_pairs = db::get_raw_events(&state.db, &active_hrefs, &start_str, &end_str)
        .await
        .unwrap_or_default();

    let mut all_events: Vec<(NaiveDate, EventJson)> = Vec::new();
    for (account_id, calendar_href, raw_ical) in raw_pairs {
        let color = color_map
            .get(&format!("{}\0{}", account_id, calendar_href))
            .map(|s| s.as_str())
            .unwrap_or("");
        for event in CalDavClient::events_from_raw_ical(&raw_ical, start, end) {
            let json = build_event_json(&event, &account_id, color, &calendar_href);
            all_events.push((event.date, json));
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

pub async fn list_calendar(
    State(state): State<AppState>,
    user: CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let today = chrono::Local::now().date_naive();
    let mut month_param: Option<String> = None;
    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            if key == "month" {
                month_param = Some(value.into_owned());
            }
        }
    }

    let first_day = if let Some(ref m) = month_param {
        NaiveDate::parse_from_str(&format!("{}-01", m), "%Y-%m-%d")
            .unwrap_or_else(|_| today.with_day(1).unwrap_or(today))
    } else {
        today.with_day(1).unwrap_or(today)
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

    let month_start_str = first_day.format("%Y-%m-%d").to_string();
    let month_end_str = last_day.format("%Y-%m-%d").to_string();

    let mut colors_map: HashMap<NaiveDate, Vec<String>> = HashMap::new();

    let non_recurring =
        db::get_event_dates_for_month(&state.db, &active_hrefs, &month_start_str, &month_end_str)
            .await
            .unwrap_or_default();
    for (account_id, calendar_href, dtstart_str) in non_recurring {
        if let Ok(date) = NaiveDate::parse_from_str(&dtstart_str, "%Y-%m-%d") {
            let color = color_map
                .get(&format!("{}\0{}", account_id, calendar_href))
                .map(|s| s.as_str())
                .unwrap_or("");
            let dot_color = if color.is_empty() {
                NEUTRAL_DOT_COLOR.to_string()
            } else {
                color.to_string()
            };
            colors_map.entry(date).or_default().push(dot_color);
        }
    }

    let recurring_raw = db::get_recurring_raw_events(&state.db, &active_hrefs)
        .await
        .unwrap_or_default();
    for (account_id, calendar_href, raw_ical) in recurring_raw {
        let color = color_map
            .get(&format!("{}\0{}", account_id, calendar_href))
            .map(|s| s.as_str())
            .unwrap_or("");
        let dot_color = if color.is_empty() {
            NEUTRAL_DOT_COLOR.to_string()
        } else {
            color.to_string()
        };
        for event in CalDavClient::events_from_raw_ical(&raw_ical, first_day, last_day) {
            colors_map
                .entry(event.date)
                .or_default()
                .push(dot_color.clone());
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

pub async fn toggle_calendar(
    State(state): State<AppState>,
    user: CurrentUser,
    body: String,
) -> Response {
    let mut href = String::new();
    let mut account_id = String::new();
    let mut active = true;
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "account_id" => account_id = value.into_owned(),
            "href" => href = value.into_owned(),
            "active" => active = value != "false",
            _ => {}
        }
    }
    if href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing href parameter").into_response();
    }
    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing account_id parameter").into_response();
    }
    match db::get_user_id_for_caldav_account(&state.db, &account_id).await {
        Ok(Some(owner)) if owner == user.user_id => {}
        Ok(_) => return (StatusCode::NOT_FOUND, "CalDAV account not found").into_response(),
        Err(e) => {
            eprintln!("Database error checking account: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    }

    let result = if active {
        db::activate_calendar(&state.db, &account_id, &href, &state.encryption_key).await
    } else {
        db::deactivate_calendar(&state.db, &account_id, &href, &state.encryption_key).await
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

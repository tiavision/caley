use axum::{
    extract::{Multipart, RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeZone as _, Utc};
use serde::Serialize;
use std::collections::HashMap;
use url::form_urlencoded;

use crate::caldav::{
    Alarm, Attendee, CalDavClient, PartStat, RecurrenceInfo, add_exdate, add_recurrence_override,
    generate_ical_event, generate_new_uid, split_vcalendar_into_events, update_vcalendar_event,
};
use crate::db;
use crate::oidc::{AppState, CurrentUser, require_client};
use crate::sync::SyncTrigger;
use sqlx::Row;

const NEUTRAL_DOT_COLOR: &str = "var(--pico-muted-color)";

// ── JSON response types ────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct CalendarJson {
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

fn parse_alarm_from_fields(
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

fn parse_form_field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn parse_event_form(body: String) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn extract_vevent(ical: &str) -> String {
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

fn build_event_json(event: &crate::caldav::Event, color: &str, calendar_href: &str) -> EventJson {
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
    let deactivated =
        db::get_deactivated_calendars(&state.db, &user.user_id, &state.encryption_key)
            .await
            .unwrap_or_default();

    let result: Vec<CalendarJson> = calendars
        .into_iter()
        .map(|c| {
            let is_active = !deactivated.contains(&c.href);
            CalendarJson {
                is_active,
                href: c.href,
                name: c.name,
                color: c.color,
            }
        })
        .collect();

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
    let deactivated =
        db::get_deactivated_calendars(&state.db, &user.user_id, &state.encryption_key)
            .await
            .unwrap_or_default();

    let color_map: HashMap<String, String> = calendars
        .iter()
        .map(|c| (c.href.clone(), c.color.clone().unwrap_or_default()))
        .collect();

    let active_hrefs: Vec<String> = calendars
        .iter()
        .filter(|c| !deactivated.contains(&c.href))
        .map(|c| c.href.clone())
        .collect();

    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let raw_pairs = db::get_raw_events(
        &state.db,
        &user.user_id,
        &active_hrefs,
        &start_str,
        &end_str,
    )
    .await
    .unwrap_or_default();

    let mut all_events: Vec<(NaiveDate, EventJson)> = Vec::new();
    for (calendar_href, raw_ical) in raw_pairs {
        let color = color_map
            .get(&calendar_href)
            .map(|s| s.as_str())
            .unwrap_or("");
        for event in CalDavClient::events_from_raw_ical(&raw_ical, start, end) {
            let json = build_event_json(&event, color, &calendar_href);
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
    let deactivated =
        db::get_deactivated_calendars(&state.db, &user.user_id, &state.encryption_key)
            .await
            .unwrap_or_default();

    let color_map: HashMap<String, String> = calendars
        .iter()
        .map(|c| (c.href.clone(), c.color.clone().unwrap_or_default()))
        .collect();

    let active_hrefs: Vec<String> = calendars
        .iter()
        .filter(|c| !deactivated.contains(&c.href))
        .map(|c| c.href.clone())
        .collect();

    let month_start_str = first_day.format("%Y-%m-%d").to_string();
    let month_end_str = last_day.format("%Y-%m-%d").to_string();

    // Build color dot map: date → colors (non-recurring events in range)
    let mut colors_map: HashMap<NaiveDate, Vec<String>> = HashMap::new();

    // Non-recurring events directly in range
    let non_recurring = db::get_event_dates_for_month(
        &state.db,
        &user.user_id,
        &active_hrefs,
        &month_start_str,
        &month_end_str,
    )
    .await
    .unwrap_or_default();
    for (calendar_href, dtstart_str) in non_recurring {
        if let Ok(date) = NaiveDate::parse_from_str(&dtstart_str, "%Y-%m-%d") {
            let color = color_map
                .get(&calendar_href)
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

    // Recurring events need expansion
    let recurring_raw = db::get_recurring_raw_events(&state.db, &user.user_id, &active_hrefs)
        .await
        .unwrap_or_default();
    for (calendar_href, raw_ical) in recurring_raw {
        let color = color_map
            .get(&calendar_href)
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
    let mut active = true;
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
            "href" => href = value.into_owned(),
            "active" => active = value != "false",
            _ => {}
        }
    }
    if href.is_empty() {
        return (StatusCode::BAD_REQUEST, "Missing href parameter").into_response();
    }

    let result = if active {
        db::activate_calendar(&state.db, &user.user_id, &href, &state.encryption_key).await
    } else {
        db::deactivate_calendar(&state.db, &user.user_id, &href, &state.encryption_key).await
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

pub async fn save_event(
    State(state): State<AppState>,
    user: CurrentUser,
    body: String,
) -> Response {
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

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
    let scope = parse_form_field(&form, "scope")
        .unwrap_or("series")
        .to_string();
    let original_dtstart = parse_form_field(&form, "original_dtstart");
    let original_calendar_href =
        parse_form_field(&form, "original_calendar_href").map(|s| s.to_string());
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

    if scope == "this_instance" {
        let raw_ical = match client.get_event(&calendar_href, &uid).await {
            Ok(s) => s,
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
        // Try non-destructive update of an existing override VEVENT first.
        // Falls back to generating a fresh override when none exists yet.
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
            Some((rid_value, &tz)),
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
                    Some((rid_value, &tz)),
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
        // For edits, fetch the original and update non-destructively to preserve
        // unknown properties (ORGANIZER, CLASS, X-*, EXDATE, exception VEVENTs, …).
        // New events skip the fetch and generate from scratch.
        let fetch_href = original_calendar_href.as_deref().unwrap_or(&calendar_href);
        let ical = if is_edit {
            let original = client.get_event(fetch_href, &uid).await;
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
                    if *orig != calendar_href && !orig.is_empty() {
                        if let Err(e) = client.delete_event(orig, &uid).await {
                            eprintln!("Failed to delete event from old calendar: {e}");
                        } else {
                            let old_href = format!("{}/{}.ics", orig.trim_end_matches('/'), uid);
                            let _ =
                                db::delete_event_by_href(&state.db, &user.user_id, orig, &old_href)
                                    .await;
                        }
                    }
                }
                optimistic_upsert(
                    &state,
                    &user.user_id,
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

pub async fn delete_event(
    State(state): State<AppState>,
    user: CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let mut uid = String::new();
    let mut calendar_href = String::new();
    let mut scope = String::new();
    let mut original_dtstart = String::new();
    let mut tz = String::new();
    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "uid" => uid = value.into_owned(),
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

    if scope == "this_instance" && !original_dtstart.is_empty() {
        let raw_ical = match client.get_event(&calendar_href, &uid).await {
            Ok(s) => s,
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
            let _ = db::delete_cached_event(&state.db, &user.user_id, &calendar_href, &uid).await;
            state.sse_broadcaster.send(
                &user.user_id,
                crate::sse::SseMessage::EventRemoved {
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

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn optimistic_upsert(
    state: &AppState,
    user_id: &str,
    calendar_href: &str,
    event_href: &str,
    raw_ical: &str,
    uid: &str,
) {
    if let Err(e) = db::upsert_cached_event_raw(
        &state.db,
        user_id,
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
    // For non-recurring events embed the full EventJson so the frontend can patch without a round-trip.
    let event_json = if !raw_ical.contains("RRULE:") {
        let today = chrono::Local::now().date_naive();
        let wide_start = today - chrono::TimeDelta::days(365);
        let wide_end = today + chrono::TimeDelta::days(730);
        let calendars = db::get_cached_calendars(&state.db, user_id)
            .await
            .unwrap_or_default();
        let color = calendars
            .iter()
            .find(|c| c.href == calendar_href)
            .and_then(|c| c.color.as_deref())
            .unwrap_or("");
        CalDavClient::events_from_raw_ical(raw_ical, wide_start, wide_end)
            .into_iter()
            .next()
            .and_then(|ev| serde_json::to_string(&build_event_json(&ev, color, calendar_href)).ok())
    } else {
        None
    };
    state.sse_broadcaster.send(
        user_id,
        crate::sse::SseMessage::EventUpdated {
            uid: uid.to_string(),
            calendar_href: calendar_href.to_string(),
            event_json,
        },
    );
}

pub async fn create_calendar_handler(
    State(state): State<crate::oidc::AppState>,
    user: CurrentUser,
    body: String,
) -> Response {
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let mut name = String::new();
    let mut color: Option<String> = None;
    for (key, value) in form_urlencoded::parse(body.as_bytes()) {
        match key.as_ref() {
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

    match client.create_calendar(&name, color.as_deref()).await {
        Ok(href) => {
            if let Err(e) =
                db::upsert_cached_calendar(&state.db, &user.user_id, &href, &name, color.as_deref())
                    .await
            {
                eprintln!("Failed to cache new calendar: {e}");
            }
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
            state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);
            Json(CalendarJson {
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

#[derive(serde::Serialize)]
struct ImportResult {
    imported: usize,
    errors: usize,
}

pub async fn import_ics(
    State(state): State<crate::oidc::AppState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Response {
    let client = match require_client(&state, &user).await {
        Ok(c) => c,
        Err(r) => return r,
    };

    let mut calendar_href = String::new();
    let mut ics_content: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        match field.name() {
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
    let ics = match ics_content {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, "Missing ICS file").into_response(),
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
                    &user.user_id,
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
    let mut cal_hrefs: Vec<String> = Vec::new();

    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "q" => q = value.into_owned(),
                "start" => start = Some(value.into_owned()),
                "end" => end = Some(value.into_owned()),
                "cal" => cal_hrefs.push(value.into_owned()),
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
        r#"SELECT e.uid, e.calendar_href, e.dtstart, e.raw_ical, c.color AS calendar_color
    FROM events_fts
    JOIN cached_events e ON events_fts.rowid = e.rowid
    JOIN cached_calendars c ON c.user_id = e.user_id AND c.href = e.calendar_href
    WHERE events_fts MATCH ? AND e.user_id = ?"#
            .to_string();
    if start.is_some() {
        sql.push_str(" AND e.dtstart >= ?");
    }
    if end.is_some() {
        sql.push_str(" AND e.dtstart <= ?");
    }
    if !cal_hrefs.is_empty() {
        let placeholders = cal_hrefs.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        sql.push_str(&format!(" AND e.calendar_href IN ({})", placeholders));
    }
    sql.push_str(" ORDER BY e.dtstart DESC LIMIT 100");

    let mut query = sqlx::query(&sql).bind(fts).bind(user_id);
    if let Some(s) = start {
        query = query.bind(s);
    }
    if let Some(e) = end {
        query = query.bind(e);
    }
    for href in &cal_hrefs {
        query = query.bind(href.clone());
    }

    match query.fetch_all(&state.db).await {
        Ok(rows) => {
            let mut results: Vec<EventJson> = Vec::new();
            for r in &rows {
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
                        results.push(build_event_json(&event, color_str, &calendar_href));
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

use askama::Template;
use axum::{
    extract::{RawQuery, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use axum_oidc::{EmptyAdditionalClaims, OidcClaims};
use crate::caldav::Calendar;
use chrono::{Datelike, NaiveDate};
use std::collections::{HashMap, HashSet};
use url::form_urlencoded;

use crate::oidc::{require_client, AppState};
use crate::db;
use crate::caldav::{generate_ical_event, add_recurrence_override, add_exdate, generate_new_uid, RecurrenceInfo};

const NEUTRAL_DOT_COLOR: &str = "var(--pico-muted-color)";

#[derive(Template)]
#[template(path = "calendars.html", whitespace = "minimize")]
struct CalendarsTemplate {
    calendars: Vec<Calendar>,
    deactivated: HashSet<String>,
}

#[derive(Template)]
#[template(path = "weekly.html", whitespace = "minimize")]
struct WeeklyTemplate {
    days: Vec<DayView>,
}

struct DayView {
    day_name: String,
    date_iso: String,
    is_today: bool,
    events: Vec<EventView>,
}

#[derive(Clone)]
struct EventView {
    uid: String,
    time_range: String,
    summary: String,
    color: String,
    dtstart: String,
    dtend: String,
    is_all_day: bool,
    tz: String,
    location: String,
    description: String,
    is_recurring: bool,
    recurrence_json: String,
    calendar_href: String,
    master_dtstart: String,
}

#[derive(Template)]
#[template(path = "calendar.html", whitespace = "minimize")]
struct CalendarTemplate {
    month: String,
    month_name: String,
    month_number: u32,
    year: i32,
    days: Vec<CalendarDay>,
    prev_month: String,
    next_month: String,
}

struct CalendarDay {
    day_number: u32,
    date_str: String,
    is_current_month: bool,
    event_colors: Vec<String>,
}

pub async fn list_calendars(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> Response {
    let client = match require_client(&state, &claims) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let user_id = match crate::oidc::extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response(),
    };

    let deactivated = {
        let db = state.db.lock().unwrap();
        match db::get_deactivated_calendars(&db, &user_id, &state.encryption_key) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Database error fetching deactivated calendars: {e}");
                HashSet::new()
            }
        }
    };

    match client.calendars().await {
        Ok(calendars) => {
            let tpl = CalendarsTemplate { calendars, deactivated };
            match tpl.render() {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn parse_form_field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn parse_event_form(body: String) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

pub async fn save_event(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    body: String,
) -> Response {
    let client = match require_client(&state, &claims) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let form = parse_event_form(body);
    let mut uid = match parse_form_field(&form, "uid") {
        Some(u) => u.to_string(),
        None => String::new(),
    };
    if uid.is_empty() {
        uid = generate_new_uid();
    }
    let summary = parse_form_field(&form, "summary").unwrap_or_default().to_string();
    let description = parse_form_field(&form, "description").unwrap_or_default().to_string();
    let location = parse_form_field(&form, "location").unwrap_or_default().to_string();
    let date = parse_form_field(&form, "date").unwrap_or_default().to_string();
    let start_time = parse_form_field(&form, "start_time");
    let end_time = parse_form_field(&form, "end_time");
    let tz = parse_form_field(&form, "tz").unwrap_or("UTC").to_string();
    let is_all_day = parse_form_field(&form, "is_all_day") == Some("true");
    let calendar_href = match parse_form_field(&form, "calendar_href") {
        Some(h) => h.to_string(),
        None => return (StatusCode::BAD_REQUEST, "<p class='error'>Missing calendar_href</p>").into_response(),
    };
    let scope = parse_form_field(&form, "scope").unwrap_or("series").to_string();
    let original_dtstart = parse_form_field(&form, "original_dtstart");
    let original_calendar_href = parse_form_field(&form, "original_calendar_href").map(|s| s.to_string());
    let recurrence: Option<RecurrenceInfo> = parse_form_field(&form, "recurrence")
        .filter(|r| !r.is_empty() && *r != "{}")
        .and_then(|r| serde_json::from_str(r).ok());

    if summary.is_empty() {
        return (StatusCode::BAD_REQUEST, "<p class='error'>Missing summary</p>").into_response();
    }
    if date.is_empty() {
        return (StatusCode::BAD_REQUEST, "<p class='error'>Missing date</p>").into_response();
    }

    if scope == "this_instance" {
        match client.get_event(&calendar_href, &uid).await {
            Ok(raw_ical) => {
                let orig_dt = original_dtstart.unwrap_or("");
                let rid_value = if is_all_day {
                    orig_dt.split('T').next().unwrap_or(orig_dt)
                } else {
                    orig_dt
                };
                let override_ical = generate_ical_event(
                    &uid, &summary, &description, &location,
                    &date, start_time, end_time, &tz, is_all_day,
                    Some((rid_value, &tz)), None,
                );
                let override_vevent = extract_vevent(&override_ical);
                let modified = add_recurrence_override(&raw_ical, &override_vevent, rid_value);

                match client.put_event(&calendar_href, &uid, &modified).await {
                    Ok(_) => (
                        StatusCode::OK,
                        [("HX-Trigger", "calendarChange")],
                        "OK",
                    ).into_response(),
                    Err(e) => {
                        eprintln!("Failed to update event instance: {e}");
                        (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to save event: {e}</p>")).into_response()
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch original event for override: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to read original event</p>")).into_response()
            }
        }
    } else {
        let ical = generate_ical_event(
            &uid, &summary, &description, &location,
            &date, start_time, end_time, &tz, is_all_day, None,
            recurrence.as_ref(),
        );

        match client.put_event(&calendar_href, &uid, &ical).await {
            Ok(_) => {
                if let Some(ref orig) = original_calendar_href {
                    if *orig != calendar_href && !orig.is_empty() {
                        if let Err(e) = client.delete_event(orig, &uid).await {
                            eprintln!("Failed to delete event from old calendar: {e}");
                        }
                    }
                }
                (
                    StatusCode::OK,
                    [("HX-Trigger", "calendarChange")],
                    "OK",
                ).into_response()
            }
            Err(e) => {
                eprintln!("Failed to update event: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to save event: {e}</p>")).into_response()
            }
        }
    }
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

pub async fn delete_event(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let client = match require_client(&state, &claims) {
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
        // Only exclude this occurrence: add EXDATE to master and drop the override.
        let raw_ical = match client.get_event(&calendar_href, &uid).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to GET event for instance delete: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to get event: {e}</p>")).into_response();
            }
        };
        let tz_ref = if tz.is_empty() { "UTC" } else { &tz };
        let modified = add_exdate(&raw_ical, &original_dtstart, tz_ref);
        return match client.put_event(&calendar_href, &uid, &modified).await {
            Ok(_) => (
                StatusCode::OK,
                [("HX-Trigger", "calendarChange")],
                "OK",
            ).into_response(),
            Err(e) => {
                eprintln!("Failed to PUT event after instance delete: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to update event: {e}</p>")).into_response()
            }
        };
    }

    match client.delete_event(&calendar_href, &uid).await {
        Ok(_) => (
            StatusCode::OK,
            [("HX-Trigger", "calendarChange")],
            "OK",
        ).into_response(),
        Err(e) => {
            eprintln!("Failed to delete event: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("<p class='error'>Failed to delete event: {e}</p>")).into_response()
        }
    }
}

pub async fn toggle_calendar(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    body: String,
) -> Response {
    let user_id = match crate::oidc::extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response(),
    };

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

    let db = state.db.lock().unwrap();
    let result = if active {
        db::activate_calendar(&db, &user_id, &href, &state.encryption_key)
    } else {
        db::deactivate_calendar(&db, &user_id, &href, &state.encryption_key)
    };

    match result {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => {
            eprintln!("Database error toggling calendar: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

pub async fn list_events(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let client = match require_client(&state, &claims) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let today = chrono::Local::now().date_naive();

    let weekday = today.weekday().num_days_from_monday();
    let week_start = today - chrono::TimeDelta::days(weekday as i64);
    let week_end = week_start + chrono::TimeDelta::days(6);

    let mut cal_params: Vec<String> = Vec::new();
    let mut start: Option<NaiveDate> = None;
    let mut end: Option<NaiveDate> = None;

    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "cal" => cal_params.push(value.into_owned()),
                "start" => start = NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
                "end" => end = NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
                _ => {}
            }
        }
    }

    let start = start.unwrap_or(week_start);
    let end = end.unwrap_or(week_end);

    let mut all_events: Vec<(NaiveDate, EventView)> = Vec::new();

    for entry in &cal_params {
        let (href, color) = match entry.split_once('|') {
            Some((h, c)) => (h, c),
            None => (entry.as_str(), ""),
        };
        let cal_href = href.to_string();

        match client.events(href, start, end).await {
            Ok(events) => {
                for event in events {
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
                    let recurrence_json = if event.is_recurring {
                        match &event.recurrence {
                            Some(r) => serde_json::to_string(r).unwrap_or_default(),
                            None => "{}".to_string(),
                        }
                    } else {
                        String::new()
                    };
                    all_events.push((
                        event.date,
                        EventView {
                            uid: event.uid,
                            time_range,
                            summary: event.summary,
                            color: dot_color,
                            dtstart: event.dtstart,
                            dtend: event.dtend.unwrap_or_default(),
                            is_all_day: event.is_all_day,
                            tz: event.tz,
                            location: event.location,
                            description: event.description,
                            is_recurring: event.is_recurring,
                            recurrence_json,
                            calendar_href: cal_href.clone(),
                            master_dtstart: event.master_dtstart,
                        },
                    ));
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch events for {href}: {e}");
            }
        }
    }

    let mut days = Vec::new();
    for i in 0..7 {
        let day = start + chrono::TimeDelta::days(i as i64);
        let mut events: Vec<EventView> = all_events
            .iter()
            .filter(|(d, _)| *d == day)
            .map(|(_, ev)| ev.clone())
            .collect();

        events.sort_by(|a, b| {
            let a_all_day = a.time_range == "All day";
            let b_all_day = b.time_range == "All day";
            if a_all_day != b_all_day {
                a_all_day.cmp(&b_all_day).reverse()
            } else {
                a.time_range.cmp(&b.time_range)
            }
        });

        days.push(DayView {
            day_name: day.format("%A").to_string(),
            date_iso: day.format("%Y-%m-%d").to_string(),
            is_today: day == today,
            events,
        });
    }

    let tpl = WeeklyTemplate { days };
    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn list_calendar(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let client = match require_client(&state, &claims) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let today = chrono::Local::now().date_naive();

    let mut cal_params: Vec<String> = Vec::new();
    let mut month_param: Option<String> = None;

    if let Some(query) = raw_query {
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "cal" => cal_params.push(value.into_owned()),
                "month" => month_param = Some(value.into_owned()),
                _ => {}
            }
        }
    }

    let first_day = if let Some(ref m) = month_param {
        NaiveDate::parse_from_str(&format!("{}-01", m), "%Y-%m-%d")
            .unwrap_or(today.with_day(1).unwrap_or(today))
    } else {
        today.with_day(1).unwrap_or(today)
    };

    let next_first = if first_day.month() == 12 {
        NaiveDate::from_ymd_opt(first_day.year() + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(first_day.year(), first_day.month() + 1, 1).unwrap()
    };
    let last_day = next_first - chrono::TimeDelta::days(1);

    let start_offset = first_day.weekday().num_days_from_monday();
    let days_in_month = last_day.day();
    let total_cells = ((start_offset + days_in_month + 6) / 7) * 7;

    let grid_start = first_day - chrono::TimeDelta::days(start_offset as i64);

    let mut colors_map: HashMap<NaiveDate, Vec<String>> = HashMap::new();

    for entry in &cal_params {
        let (href, color) = match entry.split_once('|') {
            Some((h, c)) => (h, c),
            None => (entry.as_str(), ""),
        };

        let dot_color = if color.is_empty() {
            NEUTRAL_DOT_COLOR.to_string()
        } else {
            color.to_string()
        };

        match client.events(href, first_day, last_day).await {
            Ok(events) => {
                for event in events {
                    colors_map
                        .entry(event.date)
                        .or_default()
                        .push(dot_color.clone());
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch events for {href}: {e}");
            }
        }
    }

    for colors in colors_map.values_mut() {
        colors.sort();
        colors.dedup();
    }

    let mut days = Vec::with_capacity(total_cells as usize);
    for i in 0..total_cells {
        let cell_date = grid_start + chrono::TimeDelta::days(i as i64);
        let is_current_month =
            cell_date.month() == first_day.month() && cell_date.year() == first_day.year();
        let colors = colors_map.remove(&cell_date).unwrap_or_default();
        days.push(CalendarDay {
            day_number: cell_date.day(),
            date_str: cell_date.format("%Y-%m-%d").to_string(),
            is_current_month,
            event_colors: colors,
        });
    }

    let prev_first = if first_day.month() == 1 {
        NaiveDate::from_ymd_opt(first_day.year() - 1, 12, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(first_day.year(), first_day.month() - 1, 1).unwrap()
    };

    let tpl = CalendarTemplate {
        month: first_day.format("%Y-%m").to_string(),
        month_name: first_day.format("%B").to_string(),
        month_number: first_day.month(),
        year: first_day.year(),
        days,
        prev_month: prev_first.format("%Y-%m").to_string(),
        next_month: next_first.format("%Y-%m").to_string(),
    };

    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
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
    is_today: bool,
    events: Vec<EventView>,
}

#[derive(Clone)]
struct EventView {
    time_range: String,
    summary: String,
    color: String,
}

#[derive(Template)]
#[template(path = "calendar.html", whitespace = "minimize")]
struct CalendarTemplate {
    month: String,
    month_name: String,
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
                    all_events.push((
                        event.date,
                        EventView {
                            time_range,
                            summary: event.summary,
                            color: dot_color,
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
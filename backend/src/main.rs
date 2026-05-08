mod caldav;

use askama::Template;
use axum::{
    extract::{RawQuery, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use caldav::{CalDavClient, Calendar};
use chrono::{Datelike, NaiveDate};
use clap::Parser;
use dotenvy::dotenv;
use std::collections::HashMap;
use tower_http::services::ServeDir;
use url::form_urlencoded;

const NEUTRAL_DOT_COLOR: &str = "var(--pico-muted-color)";

#[derive(Template)]
#[template(path = "calendars.html")]
struct CalendarsTemplate {
    calendars: Vec<Calendar>,
}

#[derive(Template)]
#[template(path = "weekly.html")]
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
#[template(path = "calendar.html")]
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

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_BASE_URL")]
    base_url: String,
    #[arg(env = "CALDAV_USERNAME")]
    username: String,
    #[arg(env = "CALDAV_PASSWORD")]
    password: String,
    #[arg(env = "CALDAV_PORT", default_value = "3000")]
    port: u16,
}

#[derive(Clone)]
struct AppState {
    client: CalDavClient,
}



async fn list_calendars(State(state): State<AppState>) -> impl IntoResponse {
    match state.client.calendars().await {
        Ok(calendars) => {
            let tpl = CalendarsTemplate { calendars };
            match tpl.render() {
                Ok(html) => Html(html).into_response(),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_events(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
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
                "start" => start =
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
                "end" => end =
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok(),
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

        match state.client.events(href, start, end).await {
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
        let day = start + chrono::TimeDelta::days(i);
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

async fn list_calendar(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
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

        match state.client.events(href, first_day, last_day).await {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let client = CalDavClient::new(args.base_url, args.username, args.password)?;
    let state = AppState { client };

    let app = Router::new()
        .route("/api/calendars", get(list_calendars))
        .route("/api/events", get(list_events))
        .route("/api/calendar", get(list_calendar))
        .with_state(state)
        .fallback_service(
            ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist"))
        );

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

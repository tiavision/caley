mod caldav;
mod db;

use askama::Template;
use axum::{
    body::Body,
    extract::{Form, RawQuery, State},
    http::{Request, StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_oidc::{
    ClearSessionFlag, EmptyAdditionalClaims, OidcAuthLayer, OidcClaims, OidcLoginLayer,
    OidcRpInitiatedLogout,
};
use caldav::{CalDavClient, Calendar};
use chrono::{Datelike, NaiveDate};
use clap::Parser;
use db::{Db, UserSettings};
use dotenvy::dotenv;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};
use url::form_urlencoded;

const NEUTRAL_DOT_COLOR: &str = "var(--pico-muted-color)";

#[derive(Template)]
#[template(path = "calendars.html", whitespace = "minimize")]
struct CalendarsTemplate {
    calendars: Vec<Calendar>,
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

#[derive(Template)]
#[template(path = "settings.html", whitespace = "minimize")]
struct SettingsTemplate {
    base_url: String,
    username: String,
    password: String,
}

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_PORT", default_value = "3000")]
    port: u16,
    #[arg(long, env = "OIDC_ISSUER")]
    oidc_issuer: String,
    #[arg(long, env = "OIDC_CLIENT_ID")]
    oidc_client_id: String,
    #[arg(long, env = "OIDC_CLIENT_SECRET")]
    oidc_client_secret: String,
    #[arg(long, env = "APP_BASE_URL")]
    app_base_url: String,
    #[arg(long, env = "OIDC_LOGOUT", default_value = "true")]
    oidc_logout: bool,
    #[arg(long, env = "STATIC_DIR")]
    static_dir: Option<String>,
    #[arg(long, env = "DATABASE_PATH", default_value = "./data/caley.db")]
    database_path: String,
    #[arg(long, env = "OIDC_USER_ID_CLAIM", default_value = "sub")]
    oidc_user_id_claim: String,
}

#[derive(Clone)]
struct AppState {
    db: Arc<Db>,
    do_oidc_logout: bool,
    oidc_user_id_claim: String,
}

fn extract_user_id(claims: &OidcClaims<EmptyAdditionalClaims>, claim: &str) -> Option<String> {
    match claim {
        "sub" => Some(claims.subject().as_str().to_owned()),
        "preferred_username" => claims
            .preferred_username()
            .map(|u| u.as_str().to_owned()),
        "email" => claims.email().map(|e| e.as_str().to_owned()),
        "name" => claims
            .name()
            .and_then(|n| n.get(None).map(|s| s.as_str().to_owned())),
        _ => None,
    }
}

fn require_client(
    state: &AppState,
    claims: &OidcClaims<EmptyAdditionalClaims>,
) -> Result<CalDavClient, Response> {
    let user_id = match extract_user_id(claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID from OIDC claims").into_response());
        }
    };

    let db = state.db.lock().unwrap();
    let settings = match db::get_settings(&db, &user_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response());
        }
    };

    match settings {
        Some(s) => CalDavClient::new(s.caldav_base_url, s.caldav_username, s.caldav_password)
            .map_err(|e| {
                eprintln!("CalDAV client error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "CalDAV configuration error").into_response()
            }),
        None => {
            let mut response = (StatusCode::OK, "Redirecting to settings\u{2026}").into_response();
            response.headers_mut().insert("HX-Redirect", "/settings".parse().unwrap());
            Err(response)
        }
    }
}

async fn list_calendars(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> Response {
    let client = match require_client(&state, &claims) {
        Ok(c) => c,
        Err(r) => return r,
    };

    match client.calendars().await {
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

async fn settings_page(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> Response {
    let user_id = match extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response();
        }
    };

    let db = state.db.lock().unwrap();
    let existing = match db::get_settings(&db, &user_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let (base_url, username, password) = match existing {
        Some(ref s) => (s.caldav_base_url.clone(), s.caldav_username.clone(), s.caldav_password.clone()),
        None => (String::new(), String::new(), String::new()),
    };

    let tpl = SettingsTemplate {
        base_url,
        username,
        password,
    };
    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct SettingsForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

async fn save_settings(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
    Form(form): Form<SettingsForm>,
) -> Response {
    let user_id = match extract_user_id(&claims, &state.oidc_user_id_claim) {
        Some(id) => id,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not determine user ID").into_response();
        }
    };

    let settings = UserSettings {
        caldav_base_url: form.caldav_base_url,
        caldav_username: form.caldav_username,
        caldav_password: form.caldav_password,
    };

    let client = match CalDavClient::new(
        settings.caldav_base_url.clone(),
        settings.caldav_username.clone(),
        settings.caldav_password.clone(),
    ) {
        Ok(c) => c,
        Err(e) => {
            return format!(
                r#"<div class="error-msg">Connection failed: {}</div>"#,
                html_escape(&e.to_string()),
            )
            .into_response();
        }
    };

    match client.calendars().await {
        Ok(_) => {}
        Err(e) => {
            return format!(
                r#"<div class="error-msg">Connection test failed: {}</div>"#,
                html_escape(&e.to_string()),
            )
            .into_response();
        }
    }

    let db = state.db.lock().unwrap();
    if let Err(e) = db::upsert_settings(&db, &user_id, &settings) {
        eprintln!("Database error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save settings").into_response();
    }

    r#"<div class="success-msg">Settings saved! Redirecting...</div><script>setTimeout(function(){ window.location.href='/'; }, 500);</script>"#.into_response()
}

async fn user_info(
    State(state): State<AppState>,
    claims: OidcClaims<EmptyAdditionalClaims>,
) -> impl IntoResponse {
    let display_name = claims
        .name()
        .and_then(|n| n.get(None).map(|s| s.as_str().to_owned()))
        .or_else(|| claims.preferred_username().map(|u| u.as_str().to_owned()))
        .or_else(|| Some(claims.subject().as_str().to_owned()))
        .unwrap_or_default();

    let has_settings = {
        let user_id = extract_user_id(&claims, &state.oidc_user_id_claim);
        match user_id {
            Some(uid) => {
                let db = state.db.lock().unwrap();
                db::get_settings(&db, &uid).ok().flatten().is_some()
            }
            None => false,
        }
    };

    let settings_link = if has_settings {
        r#" | <a href="/settings">Settings</a>"#
    } else {
        r#" | <a href="/settings">Set up CalDAV</a>"#
    };

    Html(format!(
        r#"<span>{}</span>{} | <a href="/logout">Logout</a>"#,
        html_escape(&display_name),
        settings_link,
    ))
}

async fn logout_handler(
    State(state): State<AppState>,
    logout: OidcRpInitiatedLogout,
) -> Response {
    let response = if state.do_oidc_logout {
        match logout.uri() {
            Ok(uri) => {
                let s = uri.to_string();
                Redirect::to(&s).into_response()
            }
            Err(_) => Redirect::to("/").into_response(),
        }
    } else {
        Redirect::to("/").into_response()
    };
    let mut response = response;
    response.extensions_mut().insert(ClearSessionFlag);
    response
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Clone)]
struct CatchLayer<L> {
    inner: L,
}

impl<L> CatchLayer<L> {
    fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<S, L: Layer<S>> Layer<S> for CatchLayer<L> {
    type Service = CatchService<L::Service>;

    fn layer(&self, inner: S) -> Self::Service {
        CatchService {
            inner: self.inner.layer(inner),
        }
    }
}

#[derive(Clone)]
struct CatchService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for CatchService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Error: std::fmt::Debug + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, std::convert::Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map(|_| Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            match inner.call(req).await {
                Ok(response) => Ok(response),
                Err(e) => {
                    eprintln!("middleware error: {:?}", e);
                    Ok(Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Internal Server Error"))
                        .unwrap())
                }
            }
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let db_conn = db::init_db(&args.database_path)?;
    let db = Arc::new(Mutex::new(db_conn));

    let state = AppState {
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
    };

    let app_base_uri: Uri = args.app_base_url.parse()?;

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store);

    let oidc_auth_layer = OidcAuthLayer::<EmptyAdditionalClaims>::discover_client(
        app_base_uri,
        args.oidc_issuer,
        args.oidc_client_id,
        Some(args.oidc_client_secret),
        vec!["openid".into(), "profile".into()],
    )
    .await?;

    let app = Router::new()
        .route("/api/calendars", get(list_calendars))
        .route("/api/events", get(list_events))
        .route("/api/calendar", get(list_calendar))
        .route("/api/user", get(user_info))
        .route("/settings", get(settings_page))
        .route("/api/settings", post(save_settings))
        .route("/logout", get(logout_handler))
        .with_state(state)
        .fallback_service(ServeDir::new(
            args.static_dir
                .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string()),
        ))
        .layer(CatchLayer::new(OidcLoginLayer::<EmptyAdditionalClaims>::new()))
        .layer(CatchLayer::new(oidc_auth_layer))
        .layer(session_layer);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
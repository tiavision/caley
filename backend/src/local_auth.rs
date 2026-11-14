use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use tower_sessions::Session;
use url::Url;

use crate::db::UserSettings;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;

#[derive(serde::Deserialize)]
pub struct LoginForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

// ── Login brute-force throttling ────────────────────────────────────────────
// Keyed by the derived identity (username@host) rather than IP, since brute
// force targets a specific account and IPs are easily rotated / proxied.

const MAX_LOGIN_ATTEMPTS: u32 = 10;
const LOGIN_WINDOW: Duration = Duration::from_secs(300);

struct LoginAttempt {
    count: u32,
    window_start: Instant,
}

fn login_attempts() -> &'static Mutex<HashMap<String, LoginAttempt>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, LoginAttempt>>> = OnceLock::new();
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn login_rate_limited(key: &str) -> bool {
    let map = login_attempts().lock().unwrap_or_else(|p| p.into_inner());
    match map.get(key) {
        Some(a) if a.window_start.elapsed() < LOGIN_WINDOW => a.count >= MAX_LOGIN_ATTEMPTS,
        _ => false,
    }
}

fn record_login_failure(key: &str) {
    let mut map = login_attempts().lock().unwrap_or_else(|p| p.into_inner());
    if map.len() > 10_000 {
        map.retain(|_, a| a.window_start.elapsed() < LOGIN_WINDOW);
    }
    let entry = map.entry(key.to_owned()).or_insert_with(|| LoginAttempt {
        count: 0,
        window_start: Instant::now(),
    });
    if entry.window_start.elapsed() >= LOGIN_WINDOW {
        entry.count = 0;
        entry.window_start = Instant::now();
    }
    entry.count += 1;
}

fn clear_login_attempts(key: &str) {
    login_attempts()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(key);
}

pub fn derive_user_id(base_url: &str, username: &str) -> String {
    if let Ok(parsed) = Url::parse(base_url) {
        if let Some(host) = parsed.host_str() {
            return format!("{}@{}", username, host);
        }
    }
    format!("{}@{}", username, base_url)
}

pub async fn handle_login(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<LoginForm>,
) -> Response {
    let user_id = derive_user_id(&form.caldav_base_url, &form.caldav_username);

    if state.demo_mode {
        // In demo mode accept any credentials and bind them to the fixed demo user.
        let demo_user_id = crate::demo::DEMO_USER_ID;
        if let Err(e) = crate::db::ensure_user(&state.db, demo_user_id, Some("Demo User")).await {
            eprintln!("Demo login: ensure_user failed: {e}");
        }
        if let Err(e) = session.cycle_id().await {
            eprintln!("Session error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Session error"})),
            )
                .into_response();
        }
        let _ = session.insert("user_id", demo_user_id).await;
        let _ = session.insert("display_name", "Demo User").await;
        state.sync_manager.ensure_task_for(demo_user_id);
        // where the login page goes next (landing.rs)
        let redirect = crate::landing::take(&session, "").await;
        return Json(json!({"status": "ok", "redirect": redirect})).into_response();
    }

    if login_rate_limited(&user_id) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "Too many login attempts. Please wait and try again."})),
        )
            .into_response();
    }

    let client = match crate::caldav::CalDavClient::new(
        form.caldav_base_url.clone(),
        form.caldav_username.clone(),
        form.caldav_password.clone(),
    ) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Connection failed: {e}")})),
            )
                .into_response();
        }
    };

    if let Err(e) = client.calendars().await {
        record_login_failure(&user_id);
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Connection test failed: {e}")})),
        )
            .into_response();
    }

    let display_name = client
        .get_display_name()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| form.caldav_username.clone());

    if let Err(e) = crate::db::ensure_user(&state.db, &user_id, Some(&display_name)).await {
        eprintln!("Database error creating user on login: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to create user"})),
        )
            .into_response();
    }

    let settings = UserSettings {
        caldav_base_url: form.caldav_base_url,
        caldav_username: form.caldav_username,
        caldav_password: form.caldav_password,
    };

    if let Err(e) =
        crate::db::upsert_settings(&state.db, &user_id, &settings, &state.encryption_key).await
    {
        eprintln!("Database error saving settings on login: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to save credentials"})),
        )
            .into_response();
    }

    // Rotate the session ID on the privilege transition to prevent session fixation.
    if let Err(e) = session.cycle_id().await {
        eprintln!("Session error: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Session error"})),
        )
            .into_response();
    }
    if let Err(e) = session.insert("user_id", &user_id).await {
        eprintln!("Session error: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Session error"})),
        )
            .into_response();
    }
    if let Err(e) = session.insert("display_name", &display_name).await {
        eprintln!("Session error: {e}");
    }

    clear_login_attempts(&user_id);

    state.sync_manager.ensure_task_for(&user_id);
    state
        .sync_manager
        .trigger(&user_id, SyncTrigger::Incremental);

    // where the login page goes next (landing.rs)
    let redirect = crate::landing::take(&session, "").await;
    Json(json!({"status": "ok", "redirect": redirect})).into_response()
}

pub async fn delete_account(
    State(state): State<AppState>,
    user: CurrentUser,
    session: Session,
) -> Response {
    state.email_sync_manager.stop_all_for(&user.user_id).await;

    if let Err(e) = crate::db::delete_user(&state.db, &user.user_id).await {
        eprintln!("Error deleting user {}: {e}", user.user_id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to delete account"})),
        )
            .into_response();
    }

    // Clean up attachment files for this user
    let user_att_dir = state.data_dir.join("attachments").join(&user.user_id);
    let _ = std::fs::remove_dir_all(&user_att_dir);

    session.flush().await.ok();

    Json(json!({"redirect": "/"})).into_response()
}

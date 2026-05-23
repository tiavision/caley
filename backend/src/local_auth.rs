use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Json, Redirect, Response},
};
use serde_json::json;
use tower_sessions::Session;
use url::Url;

use crate::db::UserSettings;
use crate::oidc::{AppState, AuthMode, CurrentUser};
use crate::sync::SyncTrigger;

#[derive(serde::Deserialize)]
pub struct LoginForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
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
                .into_response()
        }
    };

    if let Err(e) = client.calendars().await {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Connection test failed: {e}")})),
        )
            .into_response();
    }

    let user_id = derive_user_id(&form.caldav_base_url, &form.caldav_username);

    let display_name = client
        .get_display_name()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| form.caldav_username.clone());

    let settings = UserSettings {
        caldav_base_url: form.caldav_base_url,
        caldav_username: form.caldav_username,
        caldav_password: form.caldav_password,
    };

    if let Err(e) = crate::db::upsert_settings(&state.db, &user_id, &settings, &state.encryption_key).await {
        eprintln!("Database error saving settings on login: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Failed to save credentials"}))).into_response();
    }

    if let Err(e) = session.insert("user_id", &user_id).await {
        eprintln!("Session error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Session error"}))).into_response();
    }
    if let Err(e) = session.insert("display_name", &display_name).await {
        eprintln!("Session error: {e}");
    }

    state.sync_manager.ensure_task_for(&user_id);
    state.sync_manager.trigger(&user_id, SyncTrigger::Incremental);

    Json(json!({"status": "ok"})).into_response()
}

pub async fn logout_handler(session: Session) -> impl IntoResponse {
    session.flush().await.ok();
    Redirect::to("/settings/")
}

pub async fn delete_account(
    State(state): State<AppState>,
    user: CurrentUser,
    session: Session,
) -> Response {
    if let Err(e) = crate::db::delete_user(&state.db, &user.user_id).await {
        eprintln!("Error deleting user {}: {e}", user.user_id);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "Failed to delete account"}))).into_response();
    }

    // Clean up attachment files for this user
    let user_att_dir = state.data_dir.join("attachments").join(&user.user_id);
    let _ = std::fs::remove_dir_all(&user_att_dir);

    let redirect = match state.auth_mode {
        AuthMode::Local => {
            session.flush().await.ok();
            "/settings/"
        }
        AuthMode::Oidc => "/calendar/",
    };

    Json(json!({"redirect": redirect})).into_response()
}

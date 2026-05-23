use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::db::UserSettings;
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;

#[derive(serde::Deserialize)]
pub(crate) struct SettingsForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

pub async fn settings_form(
    State(state): State<AppState>,
    user: CurrentUser,
) -> Response {
    let existing = match crate::db::get_settings(&state.db, &user.user_id, &state.encryption_key).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let (base_url, username, has_password) = match existing {
        Some(ref s) => (s.caldav_base_url.clone(), s.caldav_username.clone(), true),
        None => (String::new(), String::new(), false),
    };

    Json(json!({
        "caldav_base_url": base_url,
        "caldav_username": username,
        "has_password": has_password,
    }))
    .into_response()
}

pub async fn save_settings(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<SettingsForm>,
) -> Response {
    let password = if form.caldav_password.is_empty() {
        match crate::db::get_settings(&state.db, &user.user_id, &state.encryption_key).await {
            Ok(Some(s)) => s.caldav_password,
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Password is required for new connections."})),
                )
                    .into_response()
            }
        }
    } else {
        form.caldav_password
    };

    let settings = UserSettings {
        caldav_base_url: form.caldav_base_url,
        caldav_username: form.caldav_username,
        caldav_password: password,
    };

    let client = match crate::caldav::CalDavClient::new(
        settings.caldav_base_url.clone(),
        settings.caldav_username.clone(),
        settings.caldav_password.clone(),
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

    if let Err(e) = crate::db::ensure_user(&state.db, &user.user_id, user.display_name.as_deref()).await {
        eprintln!("ensure_user error: {e}");
    }
    if let Err(e) = crate::db::clear_user_cache(&state.db, &user.user_id).await {
        eprintln!("Cache clear error: {e}");
    }
    if let Err(e) = crate::db::upsert_settings(&state.db, &user.user_id, &settings, &state.encryption_key).await {
        eprintln!("Database error saving settings: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save settings").into_response();
    }

    state.sync_manager.ensure_task_for(&user.user_id);
    state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);

    Json(json!({"status": "ok"})).into_response()
}

use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;

use crate::db::{CalDavAccount, UserSettings};
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;

#[derive(serde::Deserialize)]
pub(crate) struct SettingsForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct CalDavAccountForm {
    account_id: Option<String>,
    display_name: Option<String>,
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct DeleteCalDavAccountQuery {
    account_id: String,
}

fn account_label(account: &CalDavAccount) -> String {
    account
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| account.caldav_username.clone())
}

pub async fn list_caldav_accounts(State(state): State<AppState>, user: CurrentUser) -> Response {
    let accounts =
        match crate::db::list_caldav_accounts(&state.db, &user.user_id, &state.encryption_key).await
        {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Database error listing CalDAV accounts: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };
    let calendars = crate::db::get_cached_calendars(&state.db, &user.user_id)
        .await
        .unwrap_or_default();
    let addressbooks = crate::db::get_cached_addressbooks(&state.db, &user.user_id)
        .await
        .unwrap_or_default();

    let mut result = Vec::new();
    for account in accounts {
        let deactivated =
            crate::db::get_deactivated_calendars(&state.db, &account.account_id, &state.encryption_key)
                .await
                .unwrap_or_default();
        let account_calendars: Vec<_> = calendars
            .iter()
            .filter(|c| c.account_id == account.account_id)
            .map(|c| {
                json!({
                    "account_id": c.account_id,
                    "href": c.href,
                    "name": c.name,
                    "color": c.color,
                    "is_active": !deactivated.contains(&c.href),
                })
            })
            .collect();
        let account_addressbooks: Vec<_> = addressbooks
            .iter()
            .filter(|a| a.account_id == account.account_id)
            .map(|a| {
                json!({
                    "account_id": a.account_id,
                    "href": a.href,
                    "name": a.name,
                })
            })
            .collect();
        result.push(json!({
            "account_id": account.account_id,
            "display_name": account_label(&account),
            "caldav_base_url": account.caldav_base_url,
            "caldav_username": account.caldav_username,
            "has_password": true,
            "sort_order": account.sort_order,
            "calendars": account_calendars,
            "addressbooks": account_addressbooks,
        }));
    }

    Json(json!({ "accounts": result })).into_response()
}

pub async fn save_caldav_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<CalDavAccountForm>,
) -> Response {
    let account_id = form.account_id.as_deref().filter(|s| !s.trim().is_empty());
    let password = if form.caldav_password.is_empty() {
        match account_id {
            Some(id) => match crate::db::get_caldav_account_password(
                &state.db,
                &user.user_id,
                id,
                &state.encryption_key,
            )
            .await
            {
                Ok(Some(p)) => p,
                _ => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "Password is required for new connections."})),
                    )
                        .into_response();
                }
            },
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Password is required for new connections."})),
                )
                    .into_response();
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

    let discovered_display_name = if state.demo_mode {
        None
    } else {
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
                    .into_response();
            }
        };

        if let Err(e) = client.calendars().await {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Connection test failed: {e}")})),
            )
                .into_response();
        }

        client.get_display_name().await.ok().flatten()
    };
    let display_name = form
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or(discovered_display_name)
        .unwrap_or_else(|| settings.caldav_username.clone());

    if let Err(e) =
        crate::db::ensure_user(&state.db, &user.user_id, user.display_name.as_deref()).await
    {
        eprintln!("ensure_user error: {e}");
    }

    let saved_id = match crate::db::upsert_caldav_account(
        &state.db,
        &user.user_id,
        account_id,
        Some(&display_name),
        &settings,
        &state.encryption_key,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Database error saving CalDAV account: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save account").into_response();
        }
    };

    state.sync_manager.ensure_task_for(&user.user_id);
    state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);
    state
        .sse_broadcaster
        .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
    state
        .sse_broadcaster
        .send(&user.user_id, crate::sse::SseMessage::ContactsRefetch);

    Json(json!({"status": "ok", "account_id": saved_id})).into_response()
}

pub async fn delete_caldav_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<DeleteCalDavAccountQuery>,
) -> Response {
    match crate::db::delete_caldav_account(&state.db, &user.user_id, &q.account_id).await {
        Ok(true) => {
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::CalendarsChanged);
            state
                .sse_broadcaster
                .send(&user.user_id, crate::sse::SseMessage::ContactsRefetch);
            Json(json!({"status": "ok"})).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "Account not found").into_response(),
        Err(e) => {
            eprintln!("Database error deleting CalDAV account: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete account").into_response()
        }
    }
}

pub async fn settings_form(State(state): State<AppState>, user: CurrentUser) -> Response {
    let existing =
        match crate::db::get_settings(&state.db, &user.user_id, &state.encryption_key).await {
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
                    .into_response();
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

    if !state.demo_mode {
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
                    .into_response();
            }
        };

        if let Err(e) = client.calendars().await {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Connection test failed: {e}")})),
            )
                .into_response();
        }
    }

    if let Err(e) =
        crate::db::ensure_user(&state.db, &user.user_id, user.display_name.as_deref()).await
    {
        eprintln!("ensure_user error: {e}");
    }
    if let Err(e) = crate::db::clear_user_cache(&state.db, &user.user_id).await {
        eprintln!("Cache clear error: {e}");
    }
    if let Err(e) =
        crate::db::upsert_settings(&state.db, &user.user_id, &settings, &state.encryption_key).await
    {
        eprintln!("Database error saving settings: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save settings").into_response();
    }

    state.sync_manager.ensure_task_for(&user.user_id);
    state.sync_manager.trigger(&user.user_id, SyncTrigger::Full);

    Json(json!({"status": "ok"})).into_response()
}

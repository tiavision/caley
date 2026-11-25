use axum::{
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::{ApiError, StatusOk};
use crate::calendar::CalendarJson;
use crate::contacts::AddressbookJson;
use crate::db::{CalDavAccount, UserSettings};
use crate::oidc::{AppState, CurrentUser};
use crate::sync::SyncTrigger;

#[derive(serde::Deserialize, ToSchema)]
pub(crate) struct SettingsForm {
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

#[derive(serde::Deserialize, ToSchema)]
pub(crate) struct CalDavAccountForm {
    account_id: Option<String>,
    display_name: Option<String>,
    caldav_base_url: String,
    caldav_username: String,
    caldav_password: String,
}

#[derive(serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeleteCalDavAccountQuery {
    account_id: String,
}

/// A CalDAV account with its cached calendars and address books.
#[derive(serde::Serialize, ToSchema)]
pub struct CalDavAccountJson {
    pub account_id: String,
    pub display_name: String,
    pub caldav_base_url: String,
    pub caldav_username: String,
    /// always true: a saved account has a password (it is never sent)
    pub has_password: bool,
    pub sort_order: i64,
    pub calendars: Vec<CalendarJson>,
    pub addressbooks: Vec<AddressbookJson>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct CalDavAccounts {
    pub accounts: Vec<CalDavAccountJson>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct CalDavAccountSaved {
    /// always `ok`
    pub status: &'static str,
    pub account_id: String,
}

/// The single CalDAV login of the settings form (the first account).
#[derive(serde::Serialize, ToSchema)]
pub struct SettingsFormJson {
    pub caldav_base_url: String,
    pub caldav_username: String,
    pub has_password: bool,
}

fn account_label(account: &CalDavAccount) -> String {
    account
        .display_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| account.caldav_username.clone())
}

/// The CalDAV accounts, each with its calendars and address books.
#[utoipa::path(get, path = "/caldav/accounts", responses((status = 200, body = CalDavAccounts)))]
pub async fn list_caldav_accounts(State(state): State<AppState>, user: CurrentUser) -> Response {
    let accounts = match crate::db::list_caldav_accounts(
        &state.db,
        &user.user_id,
        &state.encryption_key,
    )
    .await
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
        let deactivated = crate::db::get_deactivated_calendars(
            &state.db,
            &account.account_id,
            &state.encryption_key,
        )
        .await
        .unwrap_or_default();
        let account_calendars: Vec<_> = calendars
            .iter()
            .filter(|c| c.account_id == account.account_id)
            .map(|c| CalendarJson {
                account_id: c.account_id.clone(),
                href: c.href.clone(),
                name: c.name.clone(),
                color: c.color.clone(),
                is_active: !deactivated.contains(&c.href),
            })
            .collect();
        let account_addressbooks: Vec<_> = addressbooks
            .iter()
            .filter(|a| a.account_id == account.account_id)
            .map(|a| AddressbookJson {
                account_id: a.account_id.clone(),
                href: a.href.clone(),
                name: a.name.clone(),
            })
            .collect();
        result.push(CalDavAccountJson {
            display_name: account_label(&account),
            account_id: account.account_id,
            caldav_base_url: account.caldav_base_url,
            caldav_username: account.caldav_username,
            has_password: true,
            sort_order: account.sort_order,
            calendars: account_calendars,
            addressbooks: account_addressbooks,
        });
    }

    Json(CalDavAccounts { accounts: result }).into_response()
}

/// Adds (no `account_id`) or updates a CalDAV account after testing the connection; an empty
/// password keeps the stored one. PUT is the same.
#[utoipa::path(
    method(post, put),
    path = "/caldav/account",
    request_body(content = CalDavAccountForm, content_type = "application/x-www-form-urlencoded"),
    responses((status = 200, body = CalDavAccountSaved), (status = 400, body = ApiError))
)]
pub async fn save_caldav_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<CalDavAccountForm>,
) -> Response {
    let account_id = form.account_id.as_deref().filter(|s| !s.trim().is_empty());
    // An account_id names an account of the user's to update: never one to create (ids are made here) nor
    // another user's. Checked before anything else, the connection test included.
    let existing = match account_id {
        Some(id) => match crate::calendar::owned_account(&state, &user.user_id, id).await {
            Ok(account) => Some(account),
            Err(response) => return response,
        },
        None => None,
    };
    let password = if form.caldav_password.is_empty() {
        match existing {
            Some(account) => account.caldav_password,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiError::new("Password is required for new connections.")),
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
                    Json(ApiError::new(format!("Connection failed: {e}"))),
                )
                    .into_response();
            }
        };

        if let Err(e) = client.calendars().await {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("Connection test failed: {e}"))),
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
        Err(e) if e.is::<crate::email_db::NotOwned>() => {
            return (StatusCode::NOT_FOUND, "CalDAV account not found").into_response();
        }
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

    Json(CalDavAccountSaved {
        status: "ok",
        account_id: saved_id,
    })
    .into_response()
}

#[utoipa::path(
    delete,
    path = "/caldav/account",
    params(DeleteCalDavAccountQuery),
    responses((status = 200, body = StatusOk), (status = 404, description = "no such account"))
)]
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
            Json(StatusOk::OK).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, "Account not found").into_response(),
        Err(e) => {
            eprintln!("Database error deleting CalDAV account: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to delete account",
            )
                .into_response()
        }
    }
}

#[utoipa::path(get, path = "/settings/form", responses((status = 200, body = SettingsFormJson)))]
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

    Json(SettingsFormJson {
        caldav_base_url: base_url,
        caldav_username: username,
        has_password,
    })
    .into_response()
}

/// Saves the single CalDAV login after testing it; an empty password keeps the stored one.
#[utoipa::path(
    post,
    path = "/settings",
    request_body(content = SettingsForm, content_type = "application/x-www-form-urlencoded"),
    responses((status = 200, body = StatusOk), (status = 400, body = ApiError))
)]
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
                    Json(ApiError::new("Password is required for new connections.")),
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
                    Json(ApiError::new(format!("Connection failed: {e}"))),
                )
                    .into_response();
            }
        };

        if let Err(e) = client.calendars().await {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError::new(format!("Connection test failed: {e}"))),
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

    Json(StatusOk::OK).into_response()
}

/// This module's routes, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(settings_form))
        .routes(routes!(save_settings))
        .routes(routes!(list_caldav_accounts))
        .routes(routes!(save_caldav_account, delete_caldav_account))
}

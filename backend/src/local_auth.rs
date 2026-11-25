use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde_json::json;
use tower_sessions::Session;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::ApiError;

use crate::oidc::{AppState, CurrentUser};

/// The demo's sign-in (`POST /auth/login`, registered only with `DEMO_MODE`, main.rs): no credentials, the
/// session is the fixed demo user's. Answers where the login page goes next.
pub async fn demo_login(State(state): State<AppState>, session: Session) -> Response {
    let demo_user_id = crate::demo::DEMO_USER_ID;
    if let Err(e) = crate::db::ensure_user(&state.db, demo_user_id, Some("Demo User")).await {
        eprintln!("Demo login: ensure_user failed: {e}");
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
    let _ = session.insert("user_id", demo_user_id).await;
    let _ = session.insert("display_name", "Demo User").await;
    state.sync_manager.ensure_task_for(demo_user_id);
    // where the login page goes next (landing.rs)
    let redirect = crate::landing::take(&session, "").await;
    Json(json!({"status": "ok", "redirect": redirect})).into_response()
}

/// The account was deleted and the session ended.
#[derive(serde::Serialize, ToSchema)]
pub struct AccountDeleted {
    /// where to go next: always `/`, the login page
    pub redirect: &'static str,
}

/// Deletes the user with all their data (accounts, caches, mail files) and ends the session.
#[utoipa::path(
    delete,
    path = "/account",
    responses((status = 200, body = AccountDeleted), (status = 500, body = ApiError))
)]
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
            Json(ApiError::new("Failed to delete account")),
        )
            .into_response();
    }

    // Clean up attachment files for this user
    let user_att_dir = state.data_dir.join("attachments").join(&user.user_id);
    let _ = std::fs::remove_dir_all(&user_att_dir);

    session.flush().await.ok();

    Json(AccountDeleted { redirect: "/" }).into_response()
}

/// This module's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(delete_account))
}

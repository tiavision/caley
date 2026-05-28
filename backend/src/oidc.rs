use crate::caldav::CalDavClient;
use crate::db;
use crate::email_sync::EmailSyncManager;
use crate::search::SearchMode;
use axum::{
    body::Body,
    extract::{FromRequestParts, State},
    http::{Request, StatusCode, request::Parts},
    response::{IntoResponse, Json, Redirect, Response},
};
use axum_oidc::{ClearSessionFlag, EmptyAdditionalClaims, OidcClaims, OidcRpInitiatedLogout};
use clap::ValueEnum;
use futures_util::future::BoxFuture;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};
use tower_sessions::Session;
use webauthn_rs::Webauthn;

#[derive(Clone, Debug, ValueEnum)]
pub enum UserIdClaim {
    Sub,
    PreferredUsername,
    Email,
    Name,
}

#[derive(Clone)]
pub struct AppState {
    pub db: db::Db,
    pub do_oidc_logout: bool,
    pub oidc_user_id_claim: UserIdClaim,
    pub encryption_key: [u8; 32],
    pub sync_manager: Arc<crate::sync::SyncManager>,
    pub email_sync_manager: Arc<EmailSyncManager>,
    pub sse_broadcaster: Arc<crate::sse::SseBroadcaster>,
    pub oidc_enabled: bool,
    pub search_mode: SearchMode,
    pub map_url: String,
    pub data_dir: PathBuf,
    pub webauthn: Arc<Webauthn>,
}

pub struct CurrentUser {
    pub user_id: String,
    pub display_name: Option<String>,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Prefer an OIDC session (claims populated by OidcAuthLayer) when present,
        // otherwise fall back to a CalDAV-credential session. Both can be active.
        if let Some(claims) = parts.extensions.get::<OidcClaims<EmptyAdditionalClaims>>() {
            if let Some(user_id) = extract_user_id(claims, &state.oidc_user_id_claim) {
                let display_name = claims
                    .name()
                    .and_then(|n| n.get(None).map(|s| s.as_str().to_owned()))
                    .or_else(|| claims.preferred_username().map(|u| u.as_str().to_owned()))
                    .or_else(|| Some(claims.subject().as_str().to_owned()));
                return Ok(CurrentUser {
                    user_id,
                    display_name,
                });
            }
        }

        if let Some(session) = parts.extensions.get::<Session>().cloned() {
            if let Some(user_id) = session.get::<String>("user_id").await.ok().flatten() {
                let display_name = session.get::<String>("display_name").await.ok().flatten();
                return Ok(CurrentUser {
                    user_id,
                    display_name,
                });
            }
        }

        Err((StatusCode::UNAUTHORIZED, "Unauthorized").into_response())
    }
}

pub fn extract_user_id(
    claims: &OidcClaims<EmptyAdditionalClaims>,
    claim: &UserIdClaim,
) -> Option<String> {
    match claim {
        UserIdClaim::Sub => Some(claims.subject().as_str().to_owned()),
        UserIdClaim::PreferredUsername => {
            claims.preferred_username().map(|u| u.as_str().to_owned())
        }
        UserIdClaim::Email => claims.email().map(|e| e.as_str().to_owned()),
        UserIdClaim::Name => claims
            .name()
            .and_then(|n| n.get(None).map(|s| s.as_str().to_owned())),
    }
}

/// Build a CalDAV client for the current user. Returns an error response if
/// credentials are missing or the client cannot be constructed.
pub async fn require_client(
    state: &AppState,
    user: &CurrentUser,
) -> Result<CalDavClient, Response> {
    let _ = state.sync_manager.ensure_task_for(&user.user_id);

    let settings = match db::get_settings(&state.db, &user.user_id, &state.encryption_key).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Database error: {e}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response());
        }
    };

    match settings {
        Some(s) => {
            let client = CalDavClient::new(s.caldav_base_url, s.caldav_username, s.caldav_password)
                .map_err(|e| {
                    eprintln!("CalDAV client error: {e}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "CalDAV configuration error",
                    )
                        .into_response()
                })?;
            Ok(client)
        }
        None => Err((
            StatusCode::PRECONDITION_FAILED,
            "CalDAV credentials not configured",
        )
            .into_response()),
    }
}

pub async fn user_info(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    let display_name = user.display_name.unwrap_or_else(|| user.user_id.clone());

    let has_settings = db::get_settings(&state.db, &user.user_id, &state.encryption_key)
        .await
        .ok()
        .flatten()
        .is_some();

    let _ = state.sync_manager.ensure_task_for(&user.user_id);

    Json(json!({
        "display_name": display_name,
        "has_settings": has_settings,
        "has_caldav": has_settings,
        "oidc_enabled": state.oidc_enabled,
        "map_url": state.map_url,
    }))
}

/// Public endpoint so the login page can decide whether to offer OIDC.
pub async fn auth_config(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "oidc_enabled": state.oidc_enabled,
    }))
}

/// Unified logout for both auth styles. `OidcRpInitiatedLogout` is only present
/// for users with a valid OIDC session, so it is extracted optionally — a
/// CalDAV-credential user simply gets a session flush.
pub async fn logout_handler(
    State(state): State<AppState>,
    session: Session,
    logout: Option<OidcRpInitiatedLogout>,
) -> Response {
    match logout {
        Some(logout) if state.do_oidc_logout => {
            let mut response = match logout.uri() {
                Ok(uri) => Redirect::to(&uri.to_string()).into_response(),
                Err(_) => Redirect::to("/").into_response(),
            };
            response.extensions_mut().insert(ClearSessionFlag);
            session.flush().await.ok();
            response
        }
        _ => {
            session.flush().await.ok();
            Redirect::to("/").into_response()
        }
    }
}

/// Handler behind `/auth/oidc`. The OidcLoginLayer wrapping this route forces the
/// IdP redirect when unauthenticated and exchanges the code on return; by the
/// time this runs the user is authenticated, so just send them to the app.
pub async fn oidc_post_login(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    if let Err(e) = db::ensure_user(&state.db, &user.user_id, user.display_name.as_deref()).await {
        eprintln!("ensure_user failed for OIDC user {}: {e}", user.user_id);
    }
    Redirect::to("/calendar/")
}

#[derive(Clone)]
pub struct RequireAuthLayer;

impl<S> Layer<S> for RequireAuthLayer {
    type Service = RequireAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireAuthService { inner }
    }
}

#[derive(Clone)]
pub struct RequireAuthService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for RequireAuthService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let inner = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, inner);

        let path = request.uri().path().to_string();
        // Always-public paths: the login page, auth endpoints, and static assets.
        let is_static = path.starts_with("/assets/")
            || path.rsplit('/').next().map_or(false, |f| f.contains('.'));
        if path == "/"
            || path.starts_with("/auth/")
            || path == "/logout"
            || path == "/api/auth/config"
            || is_static
        {
            return Box::pin(async move { inner.call(request).await });
        }

        // Authenticated via OIDC claims (already in extensions) or a CalDAV session.
        let has_claims = request
            .extensions()
            .get::<OidcClaims<EmptyAdditionalClaims>>()
            .is_some();
        let session = request.extensions().get::<Session>().cloned();
        Box::pin(async move {
            let authenticated = has_claims
                || match &session {
                    Some(s) => s.get::<String>("user_id").await.ok().flatten().is_some(),
                    None => false,
                };
            if authenticated {
                return inner.call(request).await;
            }
            if path.starts_with("/api/") {
                return Ok((
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": "Unauthorized"})),
                )
                    .into_response());
            }
            Ok(Redirect::temporary("/").into_response())
        })
    }
}

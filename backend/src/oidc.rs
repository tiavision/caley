use crate::db;
use crate::email_sync::EmailSyncManager;
use crate::landing;
use crate::public_origin::RequestOrigin;
use crate::search::SearchMode;
use axum::{
    body::Body,
    extract::{FromRequestParts, Query, State},
    http::{Method, Request, StatusCode, request::Parts},
    response::{IntoResponse, Json, Redirect, Response},
};
use clap::ValueEnum;
use futures_util::future::BoxFuture;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient};
use openidconnect::{
    AuthorizationCode, CsrfToken, EndSessionUrl, EndpointMaybeSet, EndpointNotSet, EndpointSet,
    LogoutRequest, Nonce, PkceCodeChallenge, PkceCodeVerifier, PostLogoutRedirectUrl, RedirectUrl,
    Scope, TokenResponse,
};
use serde_json::json;
use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll},
};
use tower::{Layer, Service};
use tower_sessions::Session;
use webauthn_rs::Webauthn;

/// The concrete CoreClient type produced by `from_provider_metadata` after discovery.
/// HasAuthUrl=Set (authorization_endpoint always present in discovery),
/// HasTokenUrl=MaybeSet, HasUserInfoUrl=MaybeSet (both optional in discovery).
pub type OidcCoreClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

pub struct OidcProvider {
    pub client: OidcCoreClient,
    pub end_session_endpoint: Option<EndSessionUrl>,
    pub http_client: OidcHttpClient,
}

/// Adapter so `openidconnect` (which bundles reqwest 0.12) can use our reqwest 0.13 client.
pub struct OidcHttpClient(pub reqwest::Client);

impl<'c> openidconnect::AsyncHttpClient<'c> for OidcHttpClient {
    type Error = openidconnect::HttpClientError<reqwest::Error>;
    type Future = BoxFuture<'c, Result<openidconnect::HttpResponse, Self::Error>>;

    fn call(&'c self, request: openidconnect::HttpRequest) -> Self::Future {
        Box::pin(async move {
            let response = self
                .0
                .execute(request.try_into().map_err(Box::new)?)
                .await
                .map_err(Box::new)?;
            let mut builder = openidconnect::http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            builder
                .body(response.bytes().await.map_err(Box::new)?.to_vec())
                .map_err(openidconnect::HttpClientError::Http)
        })
    }
}

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
    pub demo_mode: bool,
    pub oidc_provider: Option<Arc<OidcProvider>>,
    pub origin: Arc<crate::public_origin::OriginConfig>,
}

pub struct CurrentUser {
    pub user_id: String,
    pub display_name: Option<String>,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
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
    claims: &openidconnect::IdTokenClaims<
        openidconnect::EmptyAdditionalClaims,
        openidconnect::core::CoreGenderClaim,
    >,
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
            .and_then(|n| n.get(None))
            .map(|s| s.as_str().to_owned()),
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

/// Unified logout. For OIDC sessions, performs RP-initiated logout using the
/// end_session_endpoint from discovery. CalDAV sessions just get a flush.
pub async fn logout_handler(
    State(state): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    session: Session,
) -> Response {
    let is_oidc = session
        .get::<bool>("oidc_session")
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
    let id_token_hint = session.get::<String>("id_token_hint").await.ok().flatten();
    session.flush().await.ok();

    if is_oidc && state.do_oidc_logout {
        if let Some(provider) = &state.oidc_provider {
            if let Some(end_session_endpoint) = &provider.end_session_endpoint {
                let post_logout = format!("{origin}/");
                let redirect_uri = PostLogoutRedirectUrl::new(post_logout)
                    .unwrap_or_else(|_| PostLogoutRedirectUrl::new("/".to_owned()).unwrap());
                let mut url = LogoutRequest::from(end_session_endpoint.clone())
                    .set_post_logout_redirect_uri(redirect_uri)
                    .http_get_url();
                // id_token_hint is stored as a raw JWT string; append it directly.
                if let Some(hint) = id_token_hint {
                    url.query_pairs_mut().append_pair("id_token_hint", &hint);
                }
                return Redirect::to(url.as_str()).into_response();
            }
        }
    }
    Redirect::to("/").into_response()
}

/// Handles both legs of the OIDC authorization code flow at `GET /auth/oidc`.
///
/// - Error leg: IdP rejected a `prompt=none` silent auth attempt → show login page.
/// - Initiate leg: no `code` param → generate PKCE/state, redirect to IdP.
/// - Callback leg: `code` + `state` present → exchange code, verify token, write session.
pub async fn oidc_handler(
    State(state): State<AppState>,
    RequestOrigin(origin): RequestOrigin,
    session: Session,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    // Error leg: IdP returned an error (e.g. prompt=none with no active session).
    if params.contains_key("error") {
        return Redirect::to("/").into_response();
    }

    let provider = match &state.oidc_provider {
        Some(p) => Arc::clone(p),
        None => return Redirect::to("/").into_response(),
    };

    if let Some(code) = params.get("code").cloned() {
        // Callback leg.
        let csrf_token: CsrfToken = match session.get("oidc_csrf").await.ok().flatten() {
            Some(t) => t,
            None => return Redirect::to("/").into_response(),
        };
        let nonce: Nonce = match session.get("oidc_nonce").await.ok().flatten() {
            Some(n) => n,
            None => return Redirect::to("/").into_response(),
        };
        let pkce_verifier: PkceCodeVerifier =
            match session.get("oidc_pkce_verifier").await.ok().flatten() {
                Some(v) => v,
                None => return Redirect::to("/").into_response(),
            };
        // the redirect URI of the authorization request (the token request must repeat it)
        let redirect_uri: Option<RedirectUrl> =
            session.get("oidc_redirect_uri").await.ok().flatten();

        // Remove one-time-use keys before making any network calls.
        session.remove_value("oidc_csrf").await.ok();
        session.remove_value("oidc_nonce").await.ok();
        session.remove_value("oidc_pkce_verifier").await.ok();
        session.remove_value("oidc_redirect_uri").await.ok();

        // Validate state parameter to prevent CSRF.
        if params.get("state").map(|s| s.as_str()) != Some(csrf_token.secret()) {
            return Redirect::to("/").into_response();
        }

        let code_req = match provider.client.exchange_code(AuthorizationCode::new(code)) {
            Ok(req) => match &redirect_uri {
                Some(uri) => req.set_redirect_uri(Cow::Borrowed(uri)),
                None => req,
            }
            .set_pkce_verifier(pkce_verifier),
            Err(e) => {
                eprintln!("OIDC token endpoint not available: {e}");
                return Redirect::to("/").into_response();
            }
        };
        let token_response = match code_req.request_async(&provider.http_client).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("OIDC token request failed: {e}");
                return Redirect::to("/").into_response();
            }
        };

        let id_token = match token_response.id_token() {
            Some(t) => t,
            None => {
                eprintln!("OIDC response missing ID token");
                return Redirect::to("/").into_response();
            }
        };

        let verifier = provider.client.id_token_verifier();
        let claims = match id_token.claims(&verifier, &nonce) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("OIDC claims verification failed: {e}");
                return Redirect::to("/").into_response();
            }
        };

        let id_token_hint = id_token.to_string();

        let user_id = extract_user_id(claims, &state.oidc_user_id_claim)
            .unwrap_or_else(|| claims.subject().as_str().to_owned());
        let display_name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|s| s.as_str().to_owned())
            .or_else(|| claims.preferred_username().map(|u| u.as_str().to_owned()))
            .or_else(|| Some(claims.subject().as_str().to_owned()));

        if let Err(e) = db::ensure_user(&state.db, &user_id, display_name.as_deref()).await {
            eprintln!("ensure_user failed for OIDC user {user_id}: {e}");
        }

        if let Err(e) = session.cycle_id().await {
            eprintln!("session cycle_id failed: {e}");
        }
        session.insert("user_id", &user_id).await.ok();
        if let Some(ref dn) = display_name {
            session.insert("display_name", dn).await.ok();
        }
        session.insert("oidc_session", true).await.ok();
        session.insert("id_token_hint", &id_token_hint).await.ok();

        let _ = state.sync_manager.ensure_task_for(&user_id);

        // the page wanted before the sign-in, else the last one used (landing.rs)
        return Redirect::to(&crate::landing::take(&session, "").await).into_response();
    }

    // Initiate leg: generate PKCE + state and redirect to IdP.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    // back to the origin the sign-in started from (public_origin.rs), which must be registered at the IdP
    let redirect_uri = match RedirectUrl::new(format!("{origin}/auth/oidc")) {
        Ok(uri) => uri,
        Err(e) => {
            eprintln!("OIDC redirect URI invalid: {e}");
            return Redirect::to("/").into_response();
        }
    };

    let mut auth_req = provider
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("profile".to_owned()))
        .set_pkce_challenge(pkce_challenge)
        .set_redirect_uri(Cow::Borrowed(&redirect_uri));

    // Silent re-auth: RequireAuthLayer appends ?auto=1 for unauthenticated page requests
    // when OIDC is enabled. If the IdP has an active session it redirects back immediately;
    // if not, it responds with error=login_required which we handle in the error leg above.
    if params.get("auto").map(|v| v == "1").unwrap_or(false) {
        auth_req = auth_req.add_extra_param("prompt", "none");
    }

    let (auth_url, csrf_token, nonce) = auth_req.url();

    session.insert("oidc_csrf", &csrf_token).await.ok();
    session.insert("oidc_nonce", &nonce).await.ok();
    session
        .insert("oidc_pkce_verifier", &pkce_verifier)
        .await
        .ok();
    session
        .insert("oidc_redirect_uri", &redirect_uri)
        .await
        .ok();

    Redirect::to(auth_url.as_str()).into_response()
}

#[derive(Clone)]
pub struct RequireAuthLayer {
    pub oidc_enabled: bool,
}

impl<S> Layer<S> for RequireAuthLayer {
    type Service = RequireAuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireAuthService {
            inner,
            oidc_enabled: self.oidc_enabled,
        }
    }
}

#[derive(Clone)]
pub struct RequireAuthService<S> {
    inner: S,
    oidc_enabled: bool,
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
        let oidc_enabled = self.oidc_enabled;

        let path = request.uri().path().to_string();
        // Always-public paths: auth endpoints and static assets referenced before authentication,
        // such as the web app manifest and icons. (The login page, `/` or `/de/`, is public too: below.)
        let is_static = is_public_static_asset(&path);
        if path.starts_with("/auth/")
            || path == "/logout"
            || path == "/api/auth/config"
            || is_static
        {
            return Box::pin(async move { inner.call(request).await });
        }

        let session = request.extensions().get::<Session>().cloned();
        Box::pin(async move {
            let authenticated = match &session {
                Some(s) => s.get::<String>("user_id").await.ok().flatten().is_some(),
                None => false,
            };
            // The login page: signed in, a redirect to the page wanted or last used (landing.rs), so
            // the login page is never painted on the way; signed out, the page, keeping `?next=`.
            if landing::is_login_page(&path) {
                if let (Some(s), true) = (&session, request.method() == Method::GET) {
                    let next = landing::next_param(request.uri().query());
                    if authenticated {
                        if let Some(next) = next {
                            landing::remember_next(s, &next).await;
                        }
                        let to = landing::take(s, landing::locale_prefix(&path)).await;
                        return Ok(Redirect::to(&to).into_response());
                    }
                    if let Some(next) = next {
                        landing::remember_next(s, &next).await;
                    }
                }
                return inner.call(request).await;
            }
            if authenticated {
                if let Some(s) = &session {
                    landing::record(s, request.method(), request.headers(), &path).await;
                }
                return inner.call(request).await;
            }
            if path.starts_with("/api/") {
                return Ok((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Unauthorized"})),
                )
                    .into_response());
            }
            // A page, signed out: keep it for after the sign-in.
            let wanted = match request.uri().query() {
                Some(q) => format!("{path}?{q}"),
                None => path.clone(),
            };
            // Silent re-auth: if OIDC is enabled, redirect to the OIDC handler with
            // prompt=none so users who are still logged into the IdP are re-authenticated
            // without seeing the login page. The IdP returns error=login_required if no
            // session exists, and oidc_handler falls back to showing the login page.
            if oidc_enabled {
                if let Some(s) = &session {
                    landing::remember_next(s, &wanted).await;
                }
                return Ok(Redirect::temporary("/auth/oidc?auto=1").into_response());
            }
            // the login page in the language of the page asked for (`/de/mail/` → `/de/`)
            let login = format!("{}/", landing::locale_prefix(&path));
            if !landing::valid_next(&wanted) {
                return Ok(Redirect::temporary(&login).into_response());
            }
            let to: String =
                url::form_urlencoded::Serializer::for_suffix(format!("{login}?"), login.len() + 1)
                    .append_pair("next", &wanted)
                    .finish();
            Ok(Redirect::temporary(&to).into_response())
        })
    }
}

fn is_public_static_asset(path: &str) -> bool {
    path.starts_with("/assets/")
        || path.starts_with("/icons/")
        || matches!(
            path,
            "/site.webmanifest"
                | "/favicon.ico"
                | "/favicon.png"
                | "/caley-favicon.png"
                | "/caley-logo.png"
                | "/apple-touch-icon.png"
        )
}

/// This module's routes that need a signed-in user, relative to `/api` (api.rs). Not yet described by OpenAPI: plain routes.
#[allow(unused_imports)]
pub fn routes() -> utoipa_axum::router::OpenApiRouter<crate::oidc::AppState> {
    use axum::routing::{delete, get, patch, post, put};
    utoipa_axum::router::OpenApiRouter::new()
        .route("/user", get(user_info))
}

/// This module's public routes, relative to `/api` (api.rs). Not yet described by OpenAPI: plain routes.
#[allow(unused_imports)]
pub fn public_routes() -> utoipa_axum::router::OpenApiRouter<crate::oidc::AppState> {
    use axum::routing::{delete, get, patch, post, put};
    utoipa_axum::router::OpenApiRouter::new()
        .route("/auth/config", get(auth_config))
}

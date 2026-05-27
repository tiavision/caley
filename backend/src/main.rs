mod calendar;
mod caldav;
mod carddav;
mod contacts;
mod crypto;
mod db;
mod email;
mod email_autodiscover;
mod email_db;
mod email_sync;
mod imap_client;
mod local_auth;
mod oidc;
mod pgp;
mod search;
mod settings;
mod sse;
mod sync;
mod version;

use axum::error_handling::HandleErrorLayer;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use axum_oidc::error::MiddlewareError;
use axum_oidc::{EmptyAdditionalClaims, OidcAuthLayer, OidcLoginLayer};
use clap::Parser;
use dotenvy::dotenv;
use oidc::{AppState, UserIdClaim};
use search::SearchMode;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_sessions::{cookie::SameSite, Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;
use webauthn_rs::prelude::*;

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_PORT", default_value = "3000")]
    port: u16,
    #[arg(long, env = "OIDC_ISSUER")]
    oidc_issuer: Option<String>,
    #[arg(long, env = "OIDC_CLIENT_ID")]
    oidc_client_id: Option<String>,
    #[arg(long, env = "OIDC_CLIENT_SECRET")]
    oidc_client_secret: Option<String>,
    #[arg(long, env = "APP_BASE_URL")]
    app_base_url: String,
    #[arg(long, env = "OIDC_LOGOUT", default_value = "true")]
    oidc_logout: bool,
    #[arg(long, env = "STATIC_DIR")]
    static_dir: Option<String>,
    #[arg(long, env = "DATABASE_PATH", default_value = "./data/caley.db")]
    database_path: String,
    #[arg(long, env = "OIDC_USER_ID_CLAIM", default_value = "sub")]
    oidc_user_id_claim: UserIdClaim,
    #[arg(long, env = "ENCRYPTION_KEY")]
    encryption_key: String,
    #[arg(long, env = "SYNC_INTERVAL_MINS", default_value = "5")]
    sync_interval_mins: u64,
    #[arg(long, env = "SEARCH_MODE", default_value = "default")]
    search_mode: String,
    #[arg(long, env = "MAP_URL", default_value = "https://www.openstreetmap.org/search?query={q}")]
    map_url: String,
    #[arg(long, env = "WEBAUTHN_EXTRA_ORIGINS")]
    webauthn_extra_origins: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    rustls::crypto::ring::default_provider().install_default().unwrap();
    let args = Cli::parse();

    let db = db::init_db(&args.database_path).await?;

    let search_mode = args.search_mode.parse::<SearchMode>().unwrap_or_else(|e| {
        eprintln!("Warning: invalid SEARCH_MODE — {e}");
        SearchMode::Default
    });
    db::ensure_fts_schema(&db, &search_mode).await?;

    let encryption_key = crypto::derive_key(&args.encryption_key);

    let sse_broadcaster = Arc::new(sse::SseBroadcaster::new());

    let use_oidc = args.oidc_issuer.is_some()
        && args.oidc_client_id.is_some()
        && args.oidc_client_secret.is_some();

    let data_dir = PathBuf::from(&args.database_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./data"));

    let sync_manager = sync::SyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.app_base_url.clone(),
        args.sync_interval_mins,
        search_mode.clone(),
    );

    let email_sync_manager = email_sync::EmailSyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.sync_interval_mins,
        search_mode.clone(),
        data_dir.clone(),
    );

    let session_store = SqliteStore::new(db.clone());
    session_store.migrate().await?;

    // Build the WebAuthn instance from APP_BASE_URL (rp_id = host, origin = full URL).
    let webauthn = {
        let origin = url::Url::parse(&args.app_base_url)
            .map_err(|e| anyhow::anyhow!("Invalid APP_BASE_URL for WebAuthn: {e}"))?;
        let rp_id = origin
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("APP_BASE_URL has no host"))?
            .to_owned();
        let mut builder = WebauthnBuilder::new(&rp_id, &origin)
            .map_err(|e| anyhow::anyhow!("WebauthnBuilder error: {e:?}"))?
            .rp_name("Caley");
        for extra in &args.webauthn_extra_origins {
            let extra_url = url::Url::parse(extra)
                .map_err(|e| anyhow::anyhow!("Invalid WEBAUTHN_EXTRA_ORIGINS entry: {e}"))?;
            builder = builder.append_allowed_origin(&extra_url);
        }
        Arc::new(builder.build().map_err(|e| anyhow::anyhow!("Webauthn build error: {e:?}"))?)
    };

    let state = AppState {
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
        encryption_key,
        sync_manager,
        email_sync_manager,
        sse_broadcaster,
        oidc_enabled: use_oidc,
        search_mode,
        map_url: args.map_url,
        data_dir,
        webauthn,
    };

    let static_dir = args.static_dir
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string());

    let session_layer = SessionManagerLayer::new(session_store)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(7)));

    // Public — no authentication required
    let public = Router::new()
        .route("/api/push/webhook", post(sse::push_webhook))
        .route("/api/email/navigate", get(email::navigate))
        .route("/api/auth/config", get(oidc::auth_config))
        .with_state(state.clone());

    // Shared private routes (both auth modes)
    let shared_private = Router::new()
        .route("/api/calendars", get(calendar::list_calendars))
        .route("/api/calendar/toggle", post(calendar::toggle_calendar))
        .route("/api/events", get(calendar::list_events))
        .route("/api/events/stream", get(sse::events_stream))
        .route("/api/calendar", get(calendar::list_calendar))
        .route("/api/calendar", post(calendar::create_calendar_handler))
        .route(
            "/api/import",
            post(calendar::import_ics)
                .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route("/api/user", get(oidc::user_info))
        .route("/api/settings/form", get(settings::settings_form))
        .route("/api/settings", post(settings::save_settings))
        .route("/api/event", post(calendar::save_event))
        .route("/api/event", delete(calendar::delete_event))
        .route("/api/search", get(calendar::search_events))
        .route("/api/contacts", get(contacts::list_contacts))
        .route("/api/contact", post(contacts::save_contact))
        .route("/api/contact", delete(contacts::delete_contact))
        .route("/api/contact/photo", get(contacts::get_contact_photo))
        .route("/api/account", delete(local_auth::delete_account))
        // Email routes
        .route("/api/email/autodiscover", get(email::autodiscover_handler))
        .route("/api/email/accounts", get(email::list_accounts))
        .route("/api/email/account", post(email::save_account))
        .route("/api/email/account", put(email::save_account))
        .route("/api/email/account", delete(email::delete_account))
        .route("/api/email/mailboxes", get(email::list_mailboxes))
        .route("/api/email/sync", post(email::trigger_mailbox_sync))
        .route("/api/email/unread-counts", get(email::get_unread_counts))
        .route("/api/email/folders", get(email::list_virtual_folders))
        .route("/api/email/folder", post(email::create_virtual_folder))
        .route("/api/email/folder", put(email::update_virtual_folder))
        .route("/api/email/folder", delete(email::delete_virtual_folder))
        .route("/api/email/messages", get(email::list_messages))
        .route("/api/email/message", get(email::get_message))
        .route("/api/email/thread", get(email::get_thread))
        .route("/api/email/message/flags", patch(email::update_flags))
        .route("/api/email/message/move", post(email::move_message))
        .route("/api/email/message", delete(email::delete_message))
        .route("/api/email/send", post(email::send_email))
        .route(
            "/api/email/send-raw",
            post(email::send_raw_email)
                .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route("/api/email/search", get(email::search))
        .route("/api/email/suggest-addrs", get(email::suggest_addrs))
        .route("/api/email/proxy-image", get(email::proxy_image))
        .route("/api/email/attachment", get(email::download_attachment))
        .route("/api/email/export", get(email::export_account))
        .route("/api/email/image-origins", get(email::list_image_origins))
        .route("/api/email/image-origins", post(email::add_image_origin))
        .route("/api/email/image-origins", delete(email::remove_image_origin))
        .route("/api/email/image-blacklist", get(email::list_image_blacklist))
        .route("/api/email/image-blacklist", post(email::add_image_blacklist_origin))
        .route("/api/email/image-blacklist", delete(email::remove_image_blacklist_origin))
        .route("/api/email/identities", get(email::list_identities))
        .route("/api/email/identity", post(email::save_identity))
        .route("/api/email/identity", put(email::save_identity))
        .route("/api/email/identity", delete(email::delete_identity))
        // WebAuthn + PGP endpoints
        .route("/api/webauthn/register/start", post(pgp::register_start))
        .route("/api/webauthn/register/finish", post(pgp::register_finish))
        .route("/api/webauthn/authenticate/start", post(pgp::authenticate_start))
        .route("/api/webauthn/authenticate/finish", post(pgp::authenticate_finish))
        .route("/api/webauthn/credentials", get(pgp::list_credentials))
        .route("/api/webauthn/credential", delete(pgp::delete_credential))
        .route("/api/pgp/vault", get(pgp::get_vault))
        .route("/api/pgp/vault", delete(pgp::reset_vault))
        .route("/api/pgp/vault/initialize", post(pgp::initialize_vault))
        .route("/api/pgp/vault", put(pgp::update_vault))
        .route("/api/pgp/devices/pending", post(pgp::create_pending_device))
        .route("/api/pgp/keys", get(pgp::list_public_keys))
        .route("/api/pgp/key", post(pgp::save_public_key))
        .route("/api/pgp/key", delete(pgp::delete_public_key))
        .route("/api/email/raw", get(pgp::raw_email));

    // Protected routes plus the always-available CalDAV login + unified logout.
    // RequireAuthLayer lets the public allow-list through and otherwise requires
    // either OIDC claims or a CalDAV session.
    let private = shared_private
        .route("/auth/login", post(local_auth::handle_login))
        .route("/logout", get(oidc::logout_handler))
        .with_state(state.clone())
        .fallback_service(ServeDir::new(&static_dir))
        .layer(oidc::RequireAuthLayer);

    let app = if use_oidc {
        let oidc_issuer = args.oidc_issuer.unwrap();
        let oidc_client_id = args.oidc_client_id.unwrap();
        let oidc_client_secret = args.oidc_client_secret.unwrap();
        let app_base_uri = args.app_base_url.parse()?;

        let oidc_auth_layer = OidcAuthLayer::<EmptyAdditionalClaims>::discover_client(
            app_base_uri,
            oidc_issuer,
            oidc_client_id,
            Some(oidc_client_secret),
            vec!["openid".into(), "profile".into()],
        )
        .await?;

        // Explicit OIDC trigger + callback route. OidcLoginLayer wraps ONLY this
        // route, so unauthenticated requests elsewhere are not bounced to the IdP.
        // The IdP redirect URI must be `{APP_BASE_URL}/auth/oidc`.
        let oidc_trigger = Router::new()
            .route("/auth/oidc", get(oidc::oidc_post_login))
            .with_state(state)
            .layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(|session: Session, err: MiddlewareError| async move {
                        match err {
                            MiddlewareError::RequestToken(_) | MiddlewareError::ClaimsVerification(_) => {
                                session.remove_value("axum-oidc").await.unwrap_or_default();
                                Redirect::to("/").into_response()
                            }
                            _ => err.into_response(),
                        }
                    }))
                    .layer(OidcLoginLayer::<EmptyAdditionalClaims>::new())
                    .into_inner(),
            );

        println!("Auth: CalDAV login + OIDC enabled");
        // OidcAuthLayer wraps everything (safe no-op when no OIDC session) so that
        // claims are populated before RequireAuthLayer and the trigger route run.
        Router::new()
            .merge(public)
            .merge(oidc_trigger)
            .merge(private)
            .layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(|session: Session, err: MiddlewareError| async move {
                        match err {
                            MiddlewareError::RequestToken(_) | MiddlewareError::ClaimsVerification(_) => {
                                session.remove_value("axum-oidc").await.unwrap_or_default();
                                Redirect::to("/").into_response()
                            }
                            _ => err.into_response(),
                        }
                    }))
                    .layer(oidc_auth_layer)
                    .into_inner(),
            )
            .layer(session_layer)
    } else {
        println!("Auth: CalDAV login (OIDC not configured)");
        Router::new()
            .merge(public)
            .merge(private)
            .layer(session_layer)
    };

    let app = app.layer(middleware::from_fn(security_headers));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

/// Defense-in-depth response headers applied to every response.
///
/// The production frontend (Vite build) loads only an external module script, so
/// `script-src 'self'` is safe; inline `style=` attributes require
/// `style-src 'unsafe-inline'`. Email HTML is rendered in a sandboxed srcdoc iframe
/// whose images are proxied to same-origin or `data:`, both covered below.
async fn security_headers(req: Request, next: Next) -> Response {
    const CSP: &str = "default-src 'self'; \
        base-uri 'self'; \
        object-src 'none'; \
        frame-ancestors 'none'; \
        form-action 'self'; \
        img-src 'self' data:; \
        style-src 'self' 'unsafe-inline'; \
        script-src 'self'; \
        frame-src 'self'";

    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    resp
}

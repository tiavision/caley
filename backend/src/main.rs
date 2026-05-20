mod calendar;
mod caldav;
mod crypto;
mod db;
mod local_auth;
mod oidc;
mod search;
mod settings;
mod sse;
mod sync;

use axum::error_handling::HandleErrorLayer;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{delete, get, post};
use axum::Router;
use axum_oidc::error::MiddlewareError;
use axum_oidc::{EmptyAdditionalClaims, OidcAuthLayer, OidcLoginLayer};
use clap::Parser;
use dotenvy::dotenv;
use oidc::{AppState, AuthMode, UserIdClaim};
use search::SearchMode;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_sessions::{cookie::SameSite, Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::SqliteStore;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
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

    let auth_mode = if use_oidc { AuthMode::Oidc } else { AuthMode::Local };

    let sync_manager = sync::SyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.app_base_url.clone(),
        args.sync_interval_mins,
        search_mode.clone(),
    );

    let session_store = SqliteStore::new(db.clone());
    session_store.migrate().await?;

    let state = AppState {
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
        encryption_key,
        sync_manager,
        sse_broadcaster,
        auth_mode: auth_mode.clone(),
        search_mode,
    };

    let static_dir = args.static_dir
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string());

    let session_layer = SessionManagerLayer::new(session_store)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(7)));

    // Public — CalDAV servers POST here without a session cookie
    let public = Router::new()
        .route("/api/push/webhook", post(sse::push_webhook))
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
        .route("/api/account", delete(local_auth::delete_account));

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

        let private = shared_private
            .route("/auth/callback", get(oidc::auth_callback_handler))
            .route("/logout", get(oidc::logout_handler))
            .with_state(state)
            .fallback_service(ServeDir::new(&static_dir))
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
                    .layer(HandleErrorLayer::new(|e| async { e }))
                    .layer(oidc::RequireAuthLayer { auth_mode: AuthMode::Oidc })
                    .layer(OidcLoginLayer::<EmptyAdditionalClaims>::new())
                    .into_inner(),
            );

        println!("Auth mode: OIDC");
        Router::new()
            .merge(public)
            .merge(private)
            .layer(session_layer)
    } else {
        let private = shared_private
            .route("/auth/login", post(local_auth::handle_login))
            .route("/logout", get(local_auth::logout_handler))
            .with_state(state)
            .fallback_service(ServeDir::new(&static_dir))
            .layer(oidc::RequireAuthLayer { auth_mode: AuthMode::Local });

        println!("Auth mode: local CalDAV login (no OIDC configured)");
        Router::new()
            .merge(public)
            .merge(private)
            .layer(session_layer)
    };

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

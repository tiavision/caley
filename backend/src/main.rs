mod calendar;
mod caldav;
mod crypto;
mod db;
mod oidc;
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
use oidc::{AppState, UserIdClaim};
use std::sync::{Arc, Mutex};
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_sessions::{cookie::SameSite, MemoryStore, Session, SessionManagerLayer};

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_PORT", default_value = "3000")]
    port: u16,
    #[arg(long, env = "OIDC_ISSUER")]
    oidc_issuer: String,
    #[arg(long, env = "OIDC_CLIENT_ID")]
    oidc_client_id: String,
    #[arg(long, env = "OIDC_CLIENT_SECRET")]
    oidc_client_secret: String,
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let db_conn = db::init_db(&args.database_path)?;
    let db = Arc::new(Mutex::new(db_conn));

    let encryption_key = crypto::derive_key(&args.encryption_key);

    let sse_broadcaster = Arc::new(sse::SseBroadcaster::new());
    let sync_manager = sync::SyncManager::new(
        Arc::clone(&db),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.app_base_url.clone(),
    );

    let state = AppState {
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
        encryption_key,
        sync_manager,
        sse_broadcaster,
    };

    let app_base_uri = args.app_base_url.parse()?;

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_same_site(SameSite::Lax);

    let oidc_auth_layer = OidcAuthLayer::<EmptyAdditionalClaims>::discover_client(
        app_base_uri,
        args.oidc_issuer,
        args.oidc_client_id,
        Some(args.oidc_client_secret),
        vec!["openid".into(), "profile".into()],
    )
    .await?;

    // Push webhook is outside the auth middleware so CalDAV servers can reach it
    let app = Router::new()
        .route("/api/push/webhook", post(sse::push_webhook))
        .route("/auth/callback", get(oidc::auth_callback_handler))
        .route("/api/calendars", get(calendar::list_calendars))
        .route("/api/calendar/toggle", post(calendar::toggle_calendar))
        .route("/api/events", get(calendar::list_events))
        .route("/api/events/stream", get(sse::events_stream))
        .route("/api/calendar", get(calendar::list_calendar))
        .route("/api/user", get(oidc::user_info))
        .route("/api/settings/form", get(settings::settings_form))
        .route("/api/settings", post(settings::save_settings))
        .route("/api/event", post(calendar::save_event))
        .route("/api/event", delete(calendar::delete_event))
        .route("/logout", get(oidc::logout_handler))
        .with_state(state)
        .fallback_service(ServeDir::new(
            args.static_dir
                .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string()),
        ))
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
                .layer(oidc::RequireAuthLayer)
                .layer(OidcLoginLayer::<EmptyAdditionalClaims>::new())
                .into_inner(),
        )
        .layer(session_layer);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

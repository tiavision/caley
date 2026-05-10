mod calendar;
mod caldav;
mod db;
mod oidc;
mod settings;

use axum::error_handling::HandleErrorLayer;
use axum::routing::{get, post};
use axum::Router;
use axum_oidc::{EmptyAdditionalClaims, OidcAuthLayer, OidcLoginLayer};
use clap::Parser;
use dotenvy::dotenv;
use oidc::AppState;
use std::sync::{Arc, Mutex};
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

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
    oidc_user_id_claim: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let db_conn = db::init_db(&args.database_path)?;
    let db = Arc::new(Mutex::new(db_conn));

    let state = AppState {
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
    };

    let app_base_uri = args.app_base_url.parse()?;

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store);

    let oidc_auth_layer = OidcAuthLayer::<EmptyAdditionalClaims>::discover_client(
        app_base_uri,
        args.oidc_issuer,
        args.oidc_client_id,
        Some(args.oidc_client_secret),
        vec!["openid".into(), "profile".into()],
    )
    .await?;

    let app = Router::new()
        .route("/api/calendars", get(calendar::list_calendars))
        .route("/api/calendar/toggle", post(calendar::toggle_calendar))
        .route("/api/events", get(calendar::list_events))
        .route("/api/calendar", get(calendar::list_calendar))
        .route("/api/user", get(oidc::user_info))
        .route("/settings", get(settings::settings_page))
        .route("/api/settings", post(settings::save_settings))
        .route("/logout", get(oidc::logout_handler))
        .with_state(state)
        .fallback_service(ServeDir::new(
            args.static_dir
                .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string()),
        ))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|e| async { e }))
                .layer(oidc_auth_layer)
                .layer(HandleErrorLayer::new(|e| async { e }))
                .layer(OidcLoginLayer::<EmptyAdditionalClaims>::new())
                .into_inner(),
        )
        .layer(session_layer);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
mod calendar;
mod caldav;
mod db;
mod oidc;
mod settings;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use axum_oidc::{EmptyAdditionalClaims, OidcAuthLayer, OidcLoginLayer};
use clap::Parser;
use dotenvy::dotenv;
use oidc::AppState;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tower::{Layer, Service};
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

#[derive(Clone)]
struct CatchLayer<L> {
    inner: L,
}

impl<L> CatchLayer<L> {
    fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<S, L: Layer<S>> Layer<S> for CatchLayer<L> {
    type Service = CatchService<L::Service>;

    fn layer(&self, inner: S) -> Self::Service {
        CatchService {
            inner: self.inner.layer(inner),
        }
    }
}

#[derive(Clone)]
struct CatchService<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for CatchService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Error: std::fmt::Debug + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response, std::convert::Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map(|_| Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            match inner.call(req).await {
                Ok(response) => Ok(response),
                Err(e) => {
                    eprintln!("middleware error: {:?}", e);
                    Ok(Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Internal Server Error"))
                        .unwrap())
                }
            }
        })
    }
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
        .layer(CatchLayer::new(OidcLoginLayer::<EmptyAdditionalClaims>::new()))
        .layer(CatchLayer::new(oidc_auth_layer))
        .layer(session_layer);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}
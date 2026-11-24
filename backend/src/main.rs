mod api;
mod caldav;
mod calendar;
mod carddav;
mod contacts;
mod crypto;
mod db;
mod demo;
mod email;
mod email_autodiscover;
mod email_db;
mod email_sync;
mod embed;
mod etag;
mod imap_client;
mod jmap;
mod landing;
mod notifications;
mod local_auth;
mod oidc;
mod oidc_tokens;
mod pgp;
mod public_origin;
mod reminders;
mod search;
mod session_store;
mod settings;
mod sse;
mod static_files;
mod sync;
mod version;
mod xml_util;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::serve::ListenerExt;
use clap::Parser;
use dotenvy::dotenv;
use oidc::{AppState, OidcProvider, UserIdClaim};
use openidconnect::core::CoreClient;
use openidconnect::{ClientId, ClientSecret, IssuerUrl, ProviderMetadataWithLogout, RedirectUrl};
use search::SearchMode;
use session_store::SqliteStore;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_sessions::{Expiry, SessionManagerLayer, cookie::SameSite};
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
    /// Scopes asked for at the OIDC sign-in besides `openid profile`, space-separated: `offline_access` gives a
    /// refresh token (JMAP accounts that sign in with OIDC use the access token between requests), and an identity
    /// provider may want a scope (or audience) of its own before Stalwart takes the token.
    #[arg(long, env = "OIDC_SCOPES", default_value = "")]
    oidc_scopes: String,
    #[arg(long, env = "OIDC_USER_ID_CLAIM", default_value = "sub")]
    oidc_user_id_claim: UserIdClaim,
    #[arg(long, env = "ENCRYPTION_KEY")]
    encryption_key: String,
    #[arg(long, env = "SYNC_INTERVAL_MINS", default_value = "5")]
    sync_interval_mins: u64,
    #[arg(long, env = "SEARCH_MODE", default_value = "default")]
    search_mode: String,
    #[arg(
        long,
        env = "MAP_URL",
        default_value = "https://www.openstreetmap.org/search?query={q}"
    )]
    map_url: String,
    /// Reverse proxies (IPs or CIDRs, comma-separated) whose `Forwarded` / `X-Forwarded-Proto` /
    /// `X-Forwarded-Host` headers give the URL a request was made to (public_origin.rs).
    #[arg(long, env = "TRUSTED_PROXIES", value_delimiter = ',')]
    trusted_proxies: Vec<String>,
    #[arg(long, env = "WEBAUTHN_EXTRA_ORIGINS")]
    webauthn_extra_origins: Vec<String>,
    #[arg(long, env = "DEMO_MODE", default_value_t = false)]
    demo_mode: bool,
    /// Push endpoints on loopback, private and other non-public addresses allowed (notifications.rs),
    /// for a push service such as ntfy on the server's own network.
    #[arg(long, env = "NOTIFICATIONS_ALLOW_PRIVATE", default_value_t = false)]
    notifications_allow_private: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `caley --openapi`: print the OpenAPI spec of the JSON API and exit, needing no configuration.
    // The Dockerfile writes it into the frontend, whose build checks the pages against it.
    if std::env::args().skip(1).eq(["--openapi"]) {
        println!("{}", api::spec().to_pretty_json()?);
        return Ok(());
    }
    dotenv().ok();
    rustls::crypto::ring::default_provider()
        .install_default()
        .unwrap();
    let args = Cli::parse();

    // In demo mode use an isolated database so the dev/prod DB is never touched.
    let database_path = if args.demo_mode {
        let data_dir = std::path::Path::new(&args.database_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("./data"));
        data_dir.join("demo.db").to_string_lossy().into_owned()
    } else {
        args.database_path.clone()
    };

    let db = db::init_db(&database_path).await?;

    let search_mode = args.search_mode.parse::<SearchMode>().unwrap_or_else(|e| {
        eprintln!("Warning: invalid SEARCH_MODE — {e}");
        SearchMode::Default
    });
    db::ensure_fts_schema(&db, &search_mode).await?;

    let encryption_key = crypto::derive_key(&args.encryption_key);

    let data_dir = PathBuf::from(&database_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./data"));

    if args.demo_mode {
        println!("Demo mode: seeding demo database");
        demo::seed(&db, &encryption_key, &data_dir).await?;
    }

    let use_oidc = args.oidc_issuer.is_some()
        && args.oidc_client_id.is_some()
        && args.oidc_client_secret.is_some();

    let sse_broadcaster = Arc::new(sse::SseBroadcaster::new());
    let notifier = Arc::new(notifications::Notifier::new(db.clone(), encryption_key, &args.app_base_url, args.notifications_allow_private)?);
    sse_broadcaster.set_notifier(Arc::clone(&notifier));
    reminders::spawn(db.clone(), encryption_key, Arc::clone(&notifier));

    let sync_manager = sync::SyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.app_base_url.clone(),
        args.sync_interval_mins,
        search_mode.clone(),
        args.demo_mode,
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
        Arc::new(
            builder
                .build()
                .map_err(|e| anyhow::anyhow!("Webauthn build error: {e:?}"))?,
        )
    };

    let mut token_url: Option<String> = None;
    let oidc_provider: Option<Arc<OidcProvider>> = if use_oidc {
        let http_client = oidc::OidcHttpClient(
            reqwest::Client::builder()
                .user_agent(version::USER_AGENT)
                .use_preconfigured_tls(imap_client::tls_config())
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        );
        let issuer = IssuerUrl::new(args.oidc_issuer.clone().unwrap())?;
        let provider_meta = ProviderMetadataWithLogout::discover_async(issuer, &http_client)
            .await
            .map_err(|e| anyhow::anyhow!("OIDC discovery failed: {e}"))?;
        let end_session_endpoint = provider_meta
            .additional_metadata()
            .end_session_endpoint
            .clone();
        token_url = provider_meta.token_endpoint().map(|u| u.to_string());
        let client = CoreClient::from_provider_metadata(
            provider_meta,
            ClientId::new(args.oidc_client_id.clone().unwrap()),
            Some(ClientSecret::new(args.oidc_client_secret.clone().unwrap())),
        )
        .set_redirect_uri(RedirectUrl::new(format!(
            "{}/auth/oidc",
            args.app_base_url
        ))?);
        Some(Arc::new(OidcProvider {
            client,
            end_session_endpoint,
            http_client,
        }))
    } else {
        None
    };

    let refresher = match (use_oidc, token_url) {
        (true, Some(token_url)) => Some(oidc_tokens::Refresher {
            http: jmap::http_client()?,
            token_url,
            client_id: args.oidc_client_id.clone().unwrap_or_default(),
            client_secret: args.oidc_client_secret.clone().unwrap_or_default(),
        }),
        _ => None,
    };
    let oidc_tokens = Arc::new(oidc_tokens::OidcTokens::new(
        db.clone(),
        encryption_key,
        refresher,
    ));

    let email_sync_manager = email_sync::EmailSyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.sync_interval_mins,
        search_mode.clone(),
        data_dir.clone(),
        args.demo_mode,
        Arc::clone(&oidc_tokens),
    );

    let state = AppState {
        oidc_tokens,
        oidc_scopes: args
            .oidc_scopes
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        db,
        do_oidc_logout: args.oidc_logout,
        oidc_user_id_claim: args.oidc_user_id_claim,
        encryption_key,
        sync_manager,
        email_sync_manager,
        sse_broadcaster,
        notifier,
        oidc_enabled: use_oidc,
        search_mode,
        map_url: args.map_url,
        data_dir,
        webauthn,
        demo_mode: args.demo_mode,
        oidc_provider,
        origin: Arc::new(public_origin::OriginConfig::new(
            &args.app_base_url,
            &args.trusted_proxies,
        )?),
    };

    if args.demo_mode {
        println!("Demo mode: active — no external network calls will be made");
    }

    let static_dir = args
        .static_dir
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../frontend/dist").to_string());

    let session_layer = SessionManagerLayer::new(session_store)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(time::Duration::days(7)));

    // The JSON API (api.rs): every route under /api, described by frontend/openapi.json.
    // Public — no authentication required
    let public = Router::new()
        .nest("/api", api::public().split_for_parts().0)
        .with_state(state.clone());

    // Shared private routes (both auth modes)
    let shared_private: Router<oidc::AppState> =
        Router::new().nest("/api", api::private().split_for_parts().0);

    // Protected routes: OIDC handler + CalDAV login + unified logout + everything else.
    // RequireAuthLayer allows public paths and requires a session user_id for all others.
    // When OIDC is enabled, unauthenticated page requests are redirected to /auth/oidc?auto=1
    // for silent re-authentication; API requests always get 401.
    //
    // The pages come with their API data (embed.rs): the layer calls these API routes in-process, with
    // the page request's session cookie, and sits inside RequireAuthLayer, so a signed-out request for an
    // app page is redirected before anything is fetched.
    let embed_api = Router::new()
        .merge(public.clone())
        .merge(shared_private.clone().with_state(state.clone()))
        .layer(session_layer.clone());
    let mut private = shared_private
        .route("/auth/oidc", get(oidc::oidc_handler))
        .route("/auth/jmap", get(jmap::oauth::callback))
        .route("/auth/login", post(local_auth::handle_login))
        .route("/logout", get(oidc::logout_handler))
        .with_state(state.clone())
        .fallback_service(static_files::service(&static_dir));
    if let Some(embed) = embed::layer(&static_dir, embed_api)? {
        private = private.layer(embed);
    }
    let private = private.layer(oidc::RequireAuthLayer {
        oidc_enabled: use_oidc,
    });

    if use_oidc {
        println!("Auth: CalDAV login + OIDC enabled");
    } else {
        println!("Auth: CalDAV login (OIDC not configured)");
    }

    let app = Router::new()
        .merge(public)
        .merge(private)
        .layer(session_layer);

    // Every response compressed (br, zstd, gzip or deflate, as the browser accepts), except what
    // tower-http's default predicate leaves alone: event streams (compressing them would hold events
    // back), images (already compressed) and bodies under 32 bytes.
    let app = app
        // validators for the API's JSON (etag.rs): a reload that changed nothing is a 304
        .layer(middleware::from_fn(etag::etag))
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await?;
    println!("Listening on {}", listener.local_addr()?);
    let listener = listener
        // An embedded page is two writes (the head at once, then its data): without TCP_NODELAY the
        // second can wait for the first's ACK (~40 ms).
        .tap_io(|tcp| {
            let _ = tcp.set_nodelay(true);
        });
    // the peer address tells a trusted proxy from a direct request (public_origin.rs)
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Defense-in-depth response headers applied to every response.
///
/// The production frontend (Vite build) loads only same-origin scripts (its inline `<script>`s
/// are JSON data, which is not executed), so `script-src 'self'`; inline `style=` attributes require
/// `style-src 'unsafe-inline'`. Email HTML is parsed in the page by sinter-mail and rendered as
/// part of it (no iframe, so `frame-src 'none'`); its images are proxied to same-origin or `data:`,
/// both covered below.
async fn security_headers(req: Request, next: Next) -> Response {
    const CSP: &str = "default-src 'self'; \
        base-uri 'self'; \
        object-src 'none'; \
        frame-ancestors 'none'; \
        form-action 'self'; \
        img-src 'self' data:; \
        manifest-src 'self'; \
        style-src 'self' 'unsafe-inline'; \
        script-src 'self'; \
        frame-src 'none'";

    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    resp
}

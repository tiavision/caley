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
mod imap_client;
mod local_auth;
mod oidc;
mod pgp;
mod search;
mod settings;
mod sse;
mod sync;
mod version;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use clap::Parser;
use dotenvy::dotenv;
use oidc::{AppState, OidcProvider, UserIdClaim};
use openidconnect::core::CoreClient;
use openidconnect::{ClientId, ClientSecret, IssuerUrl, ProviderMetadataWithLogout, RedirectUrl};
use search::SearchMode;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_sessions::{Expiry, SessionManagerLayer, cookie::SameSite};
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
    #[arg(
        long,
        env = "MAP_URL",
        default_value = "https://www.openstreetmap.org/search?query={q}"
    )]
    map_url: String,
    #[arg(long, env = "WEBAUTHN_EXTRA_ORIGINS")]
    webauthn_extra_origins: Vec<String>,
    #[arg(long, env = "DEMO_MODE", default_value_t = false)]
    demo_mode: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    if args.demo_mode {
        println!("Demo mode: seeding demo database");
        demo::seed(&db, &encryption_key).await?;
    }

    let sse_broadcaster = Arc::new(sse::SseBroadcaster::new());

    let use_oidc = args.oidc_issuer.is_some()
        && args.oidc_client_id.is_some()
        && args.oidc_client_secret.is_some();

    let data_dir = PathBuf::from(&database_path)
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
        args.demo_mode,
    );

    let email_sync_manager = email_sync::EmailSyncManager::new(
        db.clone(),
        encryption_key,
        Arc::clone(&sse_broadcaster),
        args.sync_interval_mins,
        search_mode.clone(),
        data_dir.clone(),
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

    let oidc_provider: Option<Arc<OidcProvider>> = if use_oidc {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let issuer = IssuerUrl::new(args.oidc_issuer.clone().unwrap())?;
        let provider_meta = ProviderMetadataWithLogout::discover_async(issuer, &http_client)
            .await
            .map_err(|e| anyhow::anyhow!("OIDC discovery failed: {e}"))?;
        let end_session_endpoint = provider_meta
            .additional_metadata()
            .end_session_endpoint
            .clone();
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
        demo_mode: args.demo_mode,
        oidc_provider,
        app_base_url: args.app_base_url.clone(),
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
        .route(
            "/api/import/preview",
            post(calendar::preview_ics_import)
                .layer(axum::extract::DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route("/api/user", get(oidc::user_info))
        .route("/api/settings/form", get(settings::settings_form))
        .route("/api/settings", post(settings::save_settings))
        .route("/api/caldav/accounts", get(settings::list_caldav_accounts))
        .route("/api/caldav/account", post(settings::save_caldav_account))
        .route("/api/caldav/account", put(settings::save_caldav_account))
        .route(
            "/api/caldav/account",
            delete(settings::delete_caldav_account),
        )
        .route("/api/event", post(calendar::save_event))
        .route("/api/event", delete(calendar::delete_event))
        .route("/api/event/ics", get(calendar::get_event_ics))
        .route("/api/search", get(calendar::search_events))
        .route("/api/contacts", get(contacts::list_contacts))
        .route("/api/contact", post(contacts::save_contact))
        .route("/api/contact", delete(contacts::delete_contact))
        .route("/api/contact/photo", get(contacts::get_contact_photo))
        .route("/api/contact/vcf", get(contacts::get_contact_vcf))
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
        .route(
            "/api/email/image-origins",
            delete(email::remove_image_origin),
        )
        .route(
            "/api/email/image-blacklist",
            get(email::list_image_blacklist),
        )
        .route(
            "/api/email/image-blacklist",
            post(email::add_image_blacklist_origin),
        )
        .route(
            "/api/email/image-blacklist",
            delete(email::remove_image_blacklist_origin),
        )
        .route("/api/email/identities", get(email::list_identities))
        .route("/api/email/identity", post(email::save_identity))
        .route("/api/email/identity", put(email::save_identity))
        .route("/api/email/identity", delete(email::delete_identity))
        // WebAuthn + PGP endpoints
        .route("/api/webauthn/register/start", post(pgp::register_start))
        .route("/api/webauthn/register/finish", post(pgp::register_finish))
        .route(
            "/api/webauthn/authenticate/start",
            post(pgp::authenticate_start),
        )
        .route(
            "/api/webauthn/authenticate/finish",
            post(pgp::authenticate_finish),
        )
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

    // Protected routes: OIDC handler + CalDAV login + unified logout + everything else.
    // RequireAuthLayer allows public paths and requires a session user_id for all others.
    // When OIDC is enabled, unauthenticated page requests are redirected to /auth/oidc?auto=1
    // for silent re-authentication; API requests always get 401.
    let private = shared_private
        .route("/auth/oidc", get(oidc::oidc_handler))
        .route("/auth/login", post(local_auth::handle_login))
        .route("/logout", get(oidc::logout_handler))
        .with_state(state.clone())
        .fallback_service(ServeDir::new(&static_dir))
        .layer(oidc::RequireAuthLayer {
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

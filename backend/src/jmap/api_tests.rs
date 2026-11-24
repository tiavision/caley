//! The mail handlers with JMAP accounts, called as axum calls them, against the fake server: saving an account
//! (its session checked first), listing it, flags, the trash, sending.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use super::fake::FakeJmap;
use super::tests::mail;
use crate::email::{AccountAuth, AccountBody, Protocol};
use crate::email_db;
use crate::oidc::{AppState, CurrentUser};

pub(crate) const USER: &str = "api-user@example.com";

/// An app state over a fresh database, as `main` builds it (no OIDC provider, not demo mode).
pub(crate) async fn app_state() -> AppState {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("caley-api-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = crate::db::init_db(dir.join("t.db").to_str().unwrap())
        .await
        .unwrap();
    crate::db::ensure_user(&db, USER, None).await.unwrap();
    let key = [5; 32];
    let broadcaster = Arc::new(crate::sse::SseBroadcaster::new());
    let tokens = Arc::new(crate::oidc_tokens::OidcTokens::new(db.clone(), key, None));
    let origin = url::Url::parse("http://localhost:3000").unwrap();
    let notifier = Arc::new(crate::notifications::Notifier::new(db.clone(), key, origin.as_str(), false).unwrap());
    AppState {
        sync_manager: crate::sync::SyncManager::new(
            db.clone(),
            key,
            Arc::clone(&broadcaster),
            origin.to_string(),
            60,
            crate::search::SearchMode::Default,
            false,
        ),
        email_sync_manager: crate::email_sync::EmailSyncManager::new(
            db.clone(),
            key,
            Arc::clone(&broadcaster),
            60,
            crate::search::SearchMode::Default,
            dir.clone(),
            false,
            Arc::clone(&tokens),
        ),
        db,
        do_oidc_logout: false,
        oidc_user_id_claim: crate::oidc::UserIdClaim::Sub,
        encryption_key: key,
        notifier,
        sse_broadcaster: broadcaster,
        oidc_enabled: true,
        search_mode: crate::search::SearchMode::Default,
        map_url: String::new(),
        data_dir: dir,
        webauthn: Arc::new(
            webauthn_rs::WebauthnBuilder::new("localhost", &origin)
                .unwrap()
                .build()
                .unwrap(),
        ),
        demo_mode: false,
        oidc_provider: None,
        origin: Arc::new(crate::public_origin::OriginConfig::new(origin.as_str(), &[]).unwrap()),
        oidc_tokens: tokens,
        oidc_scopes: Vec::new(),
    }
}

pub(crate) fn user() -> CurrentUser {
    CurrentUser {
        user_id: USER.into(),
        display_name: None,
    }
}

fn body(fake: &FakeJmap, auth: AccountAuth, username: &str, password: &str) -> AccountBody {
    AccountBody {
        account_id: None,
        display_name: "Fake".into(),
        protocol: Some(Protocol::Jmap),
        jmap_url: Some(fake.url.clone()),
        auth: Some(auth),
        host: String::new(),
        port: 0,
        username: username.into(),
        password: Some(password.into()),
        use_tls: None,
        smtp_host: Some("ignored.example".into()),
        smtp_port: Some(25),
        sender_name: Some("Me".into()),
        oauth_client_id: None,
        oauth_client_secret: None,
        authorize: None,
    }
}

async fn text(r: axum::response::Response) -> (StatusCode, String) {
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A signed-in session over the app's database, as the session layer gives handlers.
pub(crate) async fn session(state: &AppState) -> tower_sessions::Session {
    let store = crate::session_store::SqliteStore::new(state.db.clone());
    let _ = store.migrate().await;
    let session = tower_sessions::Session::new(None, Arc::new(store), None);
    session.insert("user_id", USER).await.unwrap();
    session
}

/// Saves an account with a session; the answer.
pub(crate) async fn save_in(
    state: &AppState,
    session: &tower_sessions::Session,
    body: AccountBody,
) -> axum::response::Response {
    save_from(state, session, "http://localhost:3000", body).await
}

/// Saves an account with a session, the page at `origin`; the answer.
pub(crate) async fn save_from(
    state: &AppState,
    session: &tower_sessions::Session,
    origin: &str,
    body: AccountBody,
) -> axum::response::Response {
    crate::email::save_account(
        State(state.clone()),
        user(),
        session.clone(),
        crate::public_origin::RequestOrigin(origin.into()),
        Json(body),
    )
    .await
    .into_response()
}

/// Saves a JMAP account; its id.
pub(crate) async fn save(state: &AppState, body: AccountBody) -> (StatusCode, String) {
    let r = save_in(state, &session(state).await, body).await;
    let (status, text) = text(r).await;
    if status == StatusCode::OK {
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        return (status, v["account_id"].as_str().unwrap().to_owned());
    }
    (status, text)
}

#[tokio::test]
async fn a_jmap_account_is_saved_once_its_session_answers() {
    let fake = FakeJmap::start().await;
    let state = app_state().await;
    let (status, id) = save(&state, body(&fake, AccountAuth::Password, "user", "secret")).await;
    assert_eq!(status, StatusCode::OK, "{id}");

    let account = email_db::get_account(&state.db, &id, USER, &state.encryption_key)
        .await
        .unwrap()
        .unwrap();
    assert!(account.is_jmap());
    assert_eq!(
        account.jmap_url.as_deref(),
        Some(format!("{}/.well-known/jmap", fake.url).as_str())
    );
    assert_eq!(account.host, "127.0.0.1", "the host of its URL");
    assert!(!account.use_tls, "an http:// URL");
    assert_eq!(account.smtp_host, None, "no SMTP server for JMAP");
    let identities = email_db::list_identities(&state.db, USER).await.unwrap();
    assert_eq!(
        identities[0].email, "user",
        "the default identity has the username"
    );

    let r = crate::email::list_accounts(State(state.clone()), user())
        .await
        .into_response();
    let (_, listed) = text(r).await;
    let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(v["accounts"][0]["protocol"], "jmap");
    assert_eq!(v["accounts"][0]["auth"], "password");
    assert!(
        v["accounts"][0]["jmap_url"]
            .as_str()
            .unwrap()
            .ends_with("/.well-known/jmap")
    );
    state.email_sync_manager.stop_idle_for(&id);
    state.email_sync_manager.stop_task_for(&id);
}

#[tokio::test]
async fn a_refused_sign_in_is_not_saved() {
    let fake = FakeJmap::start().await;
    let state = app_state().await;
    let (status, message) = save(&state, body(&fake, AccountAuth::Password, "user", "wrong")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("refused"), "{message}");
    assert!(
        email_db::list_accounts(&state.db, USER, &state.encryption_key)
            .await
            .unwrap()
            .is_empty()
    );

    let (status, message) = save(
        &state,
        AccountBody {
            jmap_url: Some("http://127.0.0.1:9".into()),
            ..body(&fake, AccountAuth::Password, "user", "secret")
        },
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("could not be used"), "{message}");
}

#[tokio::test]
async fn an_oidc_account_needs_a_kept_sign_in_and_no_password() {
    let fake = FakeJmap::start().await;
    fake.require_bearer("oidc-token");
    let state = app_state().await;
    let mut b = body(&fake, AccountAuth::Oidc, "", "");
    b.password = None;
    let (status, message) = save(&state, b).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(message.contains("sign in"), "{message}");

    state
        .oidc_tokens
        .store(USER, "oidc-token", None, None)
        .await
        .unwrap();
    let mut b = body(&fake, AccountAuth::Oidc, "", "");
    b.password = None;
    let (status, id) = save(&state, b).await;
    assert_eq!(status, StatusCode::OK, "{id}");
    let account = email_db::get_account(&state.db, &id, USER, &state.encryption_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.auth, "oidc");
    assert_eq!(
        account.username, "user@example.com",
        "the session's username"
    );
    assert_eq!(account.password, "");
    state.email_sync_manager.stop_idle_for(&id);
    state.email_sync_manager.stop_task_for(&id);
}

#[tokio::test]
async fn another_jmap_server_drops_the_cache_of_the_old_one() {
    let fake = FakeJmap::start().await;
    let other = FakeJmap::start().await;
    let state = app_state().await;
    let (_, id) = save(&state, body(&fake, AccountAuth::Password, "user", "secret")).await;
    state.email_sync_manager.stop_task_for(&id);
    state.email_sync_manager.stop_idle_for(&id);
    fake.deliver(
        &[&fake.mailbox_id("inbox")],
        &mail("Old server", "a@example.org"),
        &[],
    );
    let (account, _) = state.email_sync_manager.load_account(&id).await.unwrap();
    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();
    assert_eq!(
        email_db::list_mailboxes(&state.db, &id)
            .await
            .unwrap()
            .len(),
        5
    );

    let mut b = body(&other, AccountAuth::Password, "user", "");
    b.account_id = Some(id.clone());
    b.password = None;
    let (status, _) = save(&state, b).await;
    assert_eq!(status, StatusCode::OK);
    state.email_sync_manager.stop_task_for(&id);
    state.email_sync_manager.stop_idle_for(&id);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cached_messages WHERE account_id = ?")
        .bind(&id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(n, 0, "the old server's messages are gone");
    assert!(
        crate::email_db::jmap_db::jmap_state(&state.db, &id, "Email")
            .await
            .unwrap()
            .is_none()
    );
    let account = email_db::get_account(&state.db, &id, USER, &state.encryption_key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        account.password, "secret",
        "an empty password keeps the stored one"
    );
}

/// A saved, synced JMAP account over the fake: (state, account id).
async fn synced(fake: &FakeJmap) -> (AppState, String) {
    let state = app_state().await;
    let (_, id) = save(&state, body(fake, AccountAuth::Password, "user", "secret")).await;
    state.email_sync_manager.stop_task_for(&id);
    state.email_sync_manager.stop_idle_for(&id);
    let (account, _) = state.email_sync_manager.load_account(&id).await.unwrap();
    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();
    (state, id)
}

#[tokio::test]
async fn flag_and_delete_handlers_change_the_server() {
    let fake = FakeJmap::start().await;
    let inbox = fake.mailbox_id("inbox");
    let email = fake.deliver(&[&inbox], &mail("Handled", "a@example.org"), &[]);
    let (state, id) = synced(&fake).await;
    let uid: i64 = sqlx::query_scalar(
        "SELECT uid FROM cached_messages WHERE account_id = ? AND mailbox = 'INBOX'",
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await
    .unwrap();

    let r = crate::email::messages::update_flags(
        State(state.clone()),
        user(),
        Json(crate::email::messages::FlagsBody {
            account_id: id.clone(),
            mailbox: "INBOX".into(),
            uid: uid as u32,
            flags: vec!["\\Flagged".into()],
            set: true,
        }),
    )
    .await
    .into_response();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(fake.email(&email).unwrap().keywords.contains("$flagged"));

    // a deletion outside the trash is a move to the trash (its role)
    let r = crate::email::messages::delete_message(
        State(state.clone()),
        user(),
        axum::extract::Query(crate::email::messages::DeleteMessageQuery {
            account_id: id.clone(),
            mailbox: "INBOX".into(),
            uid: uid as u32,
        }),
    )
    .await
    .into_response();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let trash = fake.mailbox_id("trash");
    assert_eq!(
        fake.email(&email).unwrap().mailbox_ids,
        [trash].into_iter().collect()
    );
}

#[tokio::test]
async fn emptying_a_jmap_trash_by_date_is_unsupported() {
    let fake = FakeJmap::start().await;
    let (state, id) = synced(&fake).await;
    let r = crate::email::messages::empty_trash(
        State(state.clone()),
        user(),
        Json(crate::email::messages::EmptyTrashBody {
            account_id: id,
            mailbox: "Deleted Items".into(),
            days: 30,
        }),
    )
    .await
    .into_response();
    let (status, body) = text(r).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert!(body.contains("savedate_unsupported"));
}

#[tokio::test]
async fn the_user_says_whether_an_oidc_sign_in_is_kept() {
    let state = app_state().await;
    let read = |state: AppState| async move {
        let r = crate::oidc::user_info(State(state), user())
            .await
            .into_response();
        let (_, body) = text(r).await;
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["oidc_signed_in"].clone()
    };
    assert_eq!(read(state.clone()).await, false);
    state
        .oidc_tokens
        .store(USER, "t", None, None)
        .await
        .unwrap();
    assert_eq!(read(state.clone()).await, true);
}

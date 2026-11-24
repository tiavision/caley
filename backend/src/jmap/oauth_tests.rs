//! A JMAP account signing in at the mail server (`oauth`), against the fake server's OAuth endpoints: the account
//! dialog's save, the redirect to the server, the way back, the tokens, their renewal, and what fails.

use axum::extract::{Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::Response;

use super::api_tests::{USER, app_state, save_in, session};
use super::fake::FakeJmap;
use super::oauth::{self, CallbackQuery};
use super::tests::mail;
use crate::email::{AccountAuth, AccountBody, Protocol};
use crate::email_db;
use crate::oidc::AppState;

fn body(fake: &FakeJmap) -> AccountBody {
    AccountBody {
        account_id: None,
        display_name: "Mail server".into(),
        protocol: Some(Protocol::Jmap),
        jmap_url: Some(fake.url.clone()),
        auth: Some(AccountAuth::Oauth),
        host: String::new(),
        port: 0,
        username: "user@example.com".into(),
        password: None,
        use_tls: None,
        smtp_host: None,
        smtp_port: None,
        sender_name: Some("Me".into()),
        oauth_client_id: Some("caley".into()),
        oauth_client_secret: Some("client-secret".into()),
        authorize: None,
    }
}

async fn json(r: Response) -> (StatusCode, serde_json::Value) {
    let status = r.status();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn location(r: &Response) -> String {
    assert!(r.status().is_redirection(), "{}", r.status());
    r.headers()["location"].to_str().unwrap().to_owned()
}

async fn callback(state: &AppState, s: &tower_sessions::Session, query: &str) -> Response {
    let uri: Uri = format!("/auth/jmap?{query}").parse().unwrap();
    let q = Query::<CallbackQuery>::try_from_uri(&uri).unwrap();
    oauth::callback(State(state.clone()), s.clone(), q).await
}

/// Saves, follows the link to the server's sign-in, which signs in at once and redirects back; the redirect the
/// way back answers, and the authorization request's parameters.
async fn sign_in(
    state: &AppState,
    s: &tower_sessions::Session,
    b: AccountBody,
) -> (Response, std::collections::HashMap<String, String>) {
    let (status, saved) = json(save_in(state, s, b).await).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let link = saved["authorize"].as_str().expect("an authorization link");
    let start = link.strip_prefix("/auth/jmap?").unwrap();
    let to_server = location(&callback(state, s, start).await);
    let params: std::collections::HashMap<String, String> = url::Url::parse(&to_server)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let back = http.get(&to_server).send().await.unwrap();
    let back = back.headers()["location"].to_str().unwrap().to_owned();
    let query = url::Url::parse(&back).unwrap().query().unwrap().to_owned();
    (callback(state, s, &query).await, params)
}

async fn only_account(state: &AppState) -> email_db::ImapAccountRow {
    let mut all = email_db::list_accounts(&state.db, USER, &state.encryption_key)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    all.remove(0)
}

fn stop(state: &AppState, id: &str) {
    state.email_sync_manager.stop_task_for(id);
    state.email_sync_manager.stop_idle_for(id);
}

#[tokio::test]
async fn a_new_account_is_saved_when_the_servers_sign_in_comes_back() {
    let fake = FakeJmap::start().await;
    fake.deliver(
        &[&fake.mailbox_id("inbox")],
        &mail("Via OAuth", "a@example.org"),
        &[],
    );
    let state = app_state().await;
    let s = session(&state).await;

    // saving answers the link; nothing is saved yet
    let (_, saved) = json(save_in(&state, &s, body(&fake)).await).await;
    assert!(
        saved["authorize"]
            .as_str()
            .unwrap()
            .starts_with("/auth/jmap?start=")
    );
    assert!(
        email_db::list_accounts(&state.db, USER, &state.encryption_key)
            .await
            .unwrap()
            .is_empty()
    );

    let (back, params) = sign_in(&state, &s, body(&fake)).await;
    // the authorization request: the client, the redirect URI to register, the scopes, PKCE, the login hint
    assert_eq!(params["client_id"], "caley");
    assert_eq!(params["redirect_uri"], "http://localhost:3000/auth/jmap");
    assert_eq!(params["scope"], oauth::SCOPES);
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["code_challenge"].len(), 43);
    assert_eq!(params["login_hint"], "user@example.com");
    assert_eq!(params["response_type"], "code");

    let account = only_account(&state).await;
    assert!(
        location(&back).starts_with(&format!(
            "/mail/?account_id={}&mailbox=INBOX",
            account.account_id
        )),
        "{}",
        location(&back)
    );
    assert_eq!(account.auth, "oauth");
    assert_eq!(account.oauth_client_id.as_deref(), Some("caley"));
    assert_eq!(
        account.oauth_client_secret.as_deref(),
        Some("client-secret")
    );
    assert_eq!(account.password, "");
    assert!(oauth::has_tokens(&state.db, &account.account_id).await);
    let identities = email_db::list_identities(&state.db, USER).await.unwrap();
    assert_eq!(
        (identities[0].name.as_str(), identities[0].email.as_str()),
        ("Me", "user@example.com")
    );
    assert_eq!(
        fake.inner.lock().unwrap().oauth.token_requests,
        vec![("authorization_code".into(), "basic".into())]
    );

    // the sync signs in with the account's access token
    stop(&state, &account.account_id);
    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cached_messages WHERE account_id = ?")
        .bind(&account.account_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn an_expired_token_is_renewed_and_a_refused_renewal_asks_to_sign_in_again() {
    let fake = FakeJmap::start().await;
    fake.inner.lock().unwrap().oauth.expires_in = 0;
    let state = app_state().await;
    let s = session(&state).await;
    sign_in(&state, &s, body(&fake)).await;
    let account = only_account(&state).await;
    stop(&state, &account.account_id);

    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();
    let requests = fake.inner.lock().unwrap().oauth.token_requests.clone();
    assert_eq!(
        requests.last().unwrap(),
        &("refresh_token".to_owned(), "basic".to_owned())
    );
    // the rotated refresh token was kept: renewing again works
    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();

    fake.inner.lock().unwrap().oauth.refresh_tokens.clear();
    let e = state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap_err();
    assert!(e.downcast_ref::<oauth::SignInAgain>().is_some(), "{e}");
}

#[tokio::test]
async fn a_public_client_names_itself_in_the_token_request() {
    let fake = FakeJmap::start().await;
    fake.inner.lock().unwrap().oauth.client_secret = None;
    let state = app_state().await;
    let s = session(&state).await;
    let (back, _) = sign_in(
        &state,
        &s,
        AccountBody {
            oauth_client_secret: None,
            ..body(&fake)
        },
    )
    .await;
    assert!(
        location(&back).contains("account_id="),
        "{}",
        location(&back)
    );
    assert_eq!(fake.inner.lock().unwrap().oauth.token_requests[0].1, "body");
    assert_eq!(only_account(&state).await.oauth_client_secret, None);
}

#[tokio::test]
async fn editing_signs_in_again_only_when_asked_or_needed() {
    let fake = FakeJmap::start().await;
    let state = app_state().await;
    let s = session(&state).await;
    sign_in(&state, &s, body(&fake)).await;
    let id = only_account(&state).await.account_id;
    stop(&state, &id);
    let edit = || AccountBody {
        account_id: Some(id.clone()),
        display_name: "Renamed".into(),
        oauth_client_secret: None,
        ..body(&fake)
    };

    // another name: saved as it is, the secret and the tokens kept
    let (_, saved) = json(save_in(&state, &s, edit()).await).await;
    assert!(saved["authorize"].is_null(), "{saved}");
    let account = only_account(&state).await;
    assert_eq!(account.display_name, "Renamed");
    assert_eq!(
        account.oauth_client_secret.as_deref(),
        Some("client-secret")
    );
    assert!(oauth::has_tokens(&state.db, &id).await);

    // asked to, or another client: sign in again
    let (_, saved) = json(
        save_in(
            &state,
            &s,
            AccountBody {
                authorize: Some(true),
                ..edit()
            },
        )
        .await,
    )
    .await;
    assert!(saved["authorize"].is_string());
    let (_, saved) = json(
        save_in(
            &state,
            &s,
            AccountBody {
                oauth_client_id: Some("other".into()),
                ..edit()
            },
        )
        .await,
    )
    .await;
    assert!(saved["authorize"].is_string());
}

#[tokio::test]
async fn what_fails_comes_back_to_the_mail_page_as_an_error() {
    let fake = FakeJmap::start().await;
    let state = app_state().await;
    let s = session(&state).await;
    let error = |r: &Response| {
        let to = location(r);
        let url = url::Url::parse(&format!("http://x{to}")).unwrap();
        assert_eq!(url.path(), "/mail/");
        url.query_pairs()
            .find(|(k, _)| k == "account_error")
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };

    // an unknown or used state
    assert!(error(&callback(&state, &s, "code=c&state=nope").await).contains("expired"));
    assert!(error(&callback(&state, &s, "start=nope").await).contains("expired"));

    // the user said no at the server
    let (_, saved) = json(save_in(&state, &s, body(&fake)).await).await;
    let link = saved["authorize"].as_str().unwrap();
    let state_param = link.rsplit('=').next().unwrap();
    let r = callback(
        &state,
        &s,
        &format!("error=access_denied&error_description=No&state={state_param}"),
    )
    .await;
    assert!(error(&r).contains("refused the sign-in: No"));

    // a code the server does not take (a wrong client secret)
    let (_, saved) = json(
        save_in(
            &state,
            &s,
            AccountBody {
                oauth_client_secret: Some("wrong".into()),
                ..body(&fake)
            },
        )
        .await,
    )
    .await;
    let start = saved["authorize"]
        .as_str()
        .unwrap()
        .strip_prefix("/auth/jmap?")
        .unwrap()
        .to_owned();
    let to_server = location(&callback(&state, &s, &start).await);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let back = http.get(&to_server).send().await.unwrap().headers()["location"]
        .to_str()
        .unwrap()
        .to_owned();
    let r = callback(&state, &s, url::Url::parse(&back).unwrap().query().unwrap()).await;
    assert!(error(&r).contains("refused the code"), "{}", error(&r));
    assert!(
        email_db::list_accounts(&state.db, USER, &state.encryption_key)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_server_without_oauth_is_refused_when_saving() {
    let state = app_state().await;
    let s = session(&state).await;
    let r = save_in(
        &state,
        &s,
        AccountBody {
            jmap_url: Some("http://127.0.0.1:9".into()),
            ..body(&FakeJmap::start().await)
        },
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = save_in(
        &state,
        &s,
        AccountBody {
            oauth_client_id: None,
            ..body(&FakeJmap::start().await)
        },
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn without_a_session_user_the_way_back_goes_to_the_login_page() {
    let state = app_state().await;
    let store = crate::session_store::SqliteStore::new(state.db.clone());
    let s = tower_sessions::Session::new(None, std::sync::Arc::new(store), None);
    assert_eq!(location(&callback(&state, &s, "code=c&state=s").await), "/");
}

/// A domain's web server in front of the mail server, as Caddy on the user's domain: it hands `/.well-known/jmap` and
/// `/jmap/session` on to the mail server (whose session names the API on the mail server's own host) and answers
/// every other path with an empty 200, its OAuth metadata paths too.
async fn front_of(fake: &FakeJmap) -> String {
    use axum::{Router, http::HeaderMap, response::IntoResponse, routing::get};
    let to = fake.url.clone();
    let session = move |headers: HeaderMap| {
        let to = to.clone();
        async move {
            let mut req = reqwest::Client::new().get(format!("{to}/jmap/session"));
            if let Some(a) = headers.get("authorization") {
                req = req.header("authorization", a.to_str().unwrap());
            }
            let r = req.send().await.unwrap();
            let status = StatusCode::from_u16(r.status().as_u16()).unwrap();
            (
                status,
                [("content-type", "application/json")],
                r.bytes().await.unwrap(),
            )
                .into_response()
        }
    };
    let app = Router::new()
        .route(
            "/.well-known/jmap",
            get(|| async { axum::response::Redirect::temporary("/jmap/session") }),
        )
        .route("/jmap/session", get(session))
        .fallback(|| async { StatusCode::OK });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn the_oauth_metadata_is_found_on_the_host_of_the_api_when_another_host_serves_the_session() {
    let fake = FakeJmap::start().await;
    fake.inner.lock().unwrap().absolute_urls = true;
    let front = front_of(&fake).await;
    let state = app_state().await;
    let s = session(&state).await;
    let (back, params) = sign_in(
        &state,
        &s,
        AccountBody {
            jmap_url: Some(front.clone()),
            ..body(&fake)
        },
    )
    .await;
    assert!(
        location(&back).contains("account_id="),
        "{}",
        location(&back)
    );
    assert_eq!(params["client_id"], "caley");
    let account = only_account(&state).await;
    assert_eq!(
        account.jmap_url.as_deref(),
        Some(format!("{front}/.well-known/jmap").as_str())
    );
    stop(&state, &account.account_id);
    fake.deliver(
        &[&fake.mailbox_id("inbox")],
        &mail("Through the front", "a@example.org"),
        &[],
    );
    state
        .email_sync_manager
        .jmap_sync(&account, USER, None)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cached_messages WHERE account_id = ?")
        .bind(&account.account_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn a_host_answering_empty_metadata_is_no_oauth_server() {
    let fake = FakeJmap::start().await;
    let front = front_of(&fake).await;
    let state = app_state().await;
    let s = session(&state).await;
    // relative URLs in the session: the API is on the front's host, which has no metadata
    let r = save_in(
        &state,
        &s,
        AccountBody {
            jmap_url: Some(front.clone()),
            ..body(&fake)
        },
    )
    .await;
    let (status, _) = json(r).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

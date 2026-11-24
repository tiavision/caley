//! The JMAP client, sync, operations and OIDC sign-in against a real Stalwart. They run when `JMAP_TEST_URL` is set
//! (and are skipped otherwise): `scripts/stalwart-test.sh` starts and provisions one and writes the variables to
//! `.stalwart-test.env`:
//!
//! ```sh
//! scripts/stalwart-test.sh && set -a && . ./.stalwart-test.env && set +a && cargo test stalwart
//! ```
//!
//! - `JMAP_TEST_URL`, `JMAP_TEST_USER`, `JMAP_TEST_PASSWORD`: the account the tests work in (they make mailboxes
//!   named `caley-<random>` and remove them again).
//! - `JMAP_TEST_TO`, `JMAP_TEST_TO_PASSWORD`: a second account on the server, which the send test mails.
//! - `JMAP_TEST_OAUTH_CLIENT_ID`, `_SECRET`, `_REDIRECT`: an OAuth client of the server; the OIDC test signs the
//!   first account in through Stalwart's login API (the calls its login page makes), as Caley's OIDC login would
//!   when Stalwart is the identity provider.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::tests::{mail, mail_with_attachment};
use super::*;
use crate::email_db::{self, jmap_db};
use crate::email_sync::{EmailSyncManager, eml_path};
use crate::oidc_tokens::{OidcTokens, Refresher};
use crate::sse::SseBroadcaster;

struct Server {
    url: String,
    user: String,
    password: String,
}

fn server() -> Option<Server> {
    let url = std::env::var("JMAP_TEST_URL").ok()?;
    let _ = rustls::crypto::ring::default_provider().install_default();
    Some(Server {
        url,
        user: std::env::var("JMAP_TEST_USER").expect("JMAP_TEST_USER"),
        password: std::env::var("JMAP_TEST_PASSWORD").expect("JMAP_TEST_PASSWORD"),
    })
}

macro_rules! server_or_skip {
    () => {
        match server() {
            Some(s) => s,
            None => {
                eprintln!("skipped: JMAP_TEST_URL is not set (scripts/stalwart-test.sh)");
                return;
            }
        }
    };
}

fn tag() -> String {
    format!("caley-{}", &uuid::Uuid::new_v4().simple().to_string()[..10])
}

async fn connect(url: &str, user: &str, password: &str) -> Client {
    Client::connect(
        &http_client().unwrap(),
        url,
        Auth::Basic {
            username: user.into(),
            password: password.into(),
        },
    )
    .await
    .unwrap()
}

/// Creates a mailbox; its id.
async fn create_mailbox(client: &Client, name: &str, parent: Option<&str>) -> String {
    let r = client
        .method(
            "Mailbox/set",
            json!({ "accountId": client.account_id, "create": { "n": { "name": name, "parentId": parent } } }),
        )
        .await
        .unwrap();
    r.pointer("/created/n/id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Mailbox/set: {r}"))
        .to_owned()
}

async fn destroy_mailboxes(client: &Client, ids: &[&str]) {
    let _ = client
        .method(
            "Mailbox/set",
            json!({ "accountId": client.account_id, "destroy": ids, "onDestroyRemoveEmails": true }),
        )
        .await;
}

/// Imports a message into mailboxes, as delivered mail; its email id.
async fn import(client: &Client, mailboxes: &[&str], raw: &[u8], keywords: &[&str]) -> String {
    let blob = client.upload(raw.to_vec(), "message/rfc822").await.unwrap();
    let mailbox_ids: serde_json::Map<String, Value> = mailboxes
        .iter()
        .map(|m| ((*m).to_owned(), Value::Bool(true)))
        .collect();
    let keywords: serde_json::Map<String, Value> = keywords
        .iter()
        .map(|k| ((*k).to_owned(), Value::Bool(true)))
        .collect();
    let r = client
        .method(
            "Email/import",
            json!({ "accountId": client.account_id, "emails": { "m": {
                "blobId": blob, "mailboxIds": mailbox_ids, "keywords": keywords,
            } } }),
        )
        .await
        .unwrap();
    r.pointer("/created/m/id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("Email/import: {r}"))
        .to_owned()
}

async fn email(client: &Client, id: &str) -> Option<Email> {
    let (mut list, _) = client.emails(&[id.to_owned()]).await.unwrap();
    list.pop()
}

/// A Caley database with the server's account as a JMAP account, and its sync manager.
struct Cache {
    mgr: Arc<EmailSyncManager>,
    db: sqlx::SqlitePool,
    dir: std::path::PathBuf,
    tokens: Arc<OidcTokens>,
}

const USER: &str = "tester";
const ACC: &str = "stalwart";
const KEY: [u8; 32] = [9; 32];

async fn cache(s: &Server, auth: &str, refresher: Option<Refresher>) -> Cache {
    let dir = std::env::temp_dir().join(format!("caley-stalwart-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = crate::db::init_db(dir.join("t.db").to_str().unwrap())
        .await
        .unwrap();
    crate::db::ensure_user(&db, USER, None).await.unwrap();
    let password = if auth == "password" {
        s.password.as_str()
    } else {
        ""
    };
    email_db::upsert_account(
        &db,
        USER,
        ACC,
        "Stalwart",
        "localhost",
        443,
        &s.user,
        password,
        false,
        None,
        None,
        &email_db::AccountProtocol {
            protocol: "jmap",
            jmap_url: Some(&s.url),
            auth,
            oauth_client_id: None,
            oauth_client_secret: None,
        },
        &KEY,
    )
    .await
    .unwrap();
    let tokens = Arc::new(OidcTokens::new(db.clone(), KEY, refresher));
    let mgr = EmailSyncManager::new(
        db.clone(),
        KEY,
        Arc::new(SseBroadcaster::new()),
        60,
        crate::search::SearchMode::Default,
        dir.clone(),
        false,
        Arc::clone(&tokens),
    );
    Cache {
        mgr,
        db,
        dir,
        tokens,
    }
}

impl Cache {
    async fn sync(&self) {
        let account = email_db::get_account(&self.db, ACC, USER, &KEY)
            .await
            .unwrap()
            .unwrap();
        self.mgr.jmap_sync(&account, USER, None).await.unwrap();
    }

    async fn client(&self) -> Client {
        let account = email_db::get_account(&self.db, ACC, USER, &KEY)
            .await
            .unwrap()
            .unwrap();
        self.mgr.jmap_client(&account, USER).await.unwrap()
    }

    /// (uid, subject, flags) of a cached mailbox.
    async fn cached(&self, mailbox: &str) -> Vec<(u32, String, Vec<String>)> {
        let rows: Vec<(i64, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT uid, subject, flags FROM cached_messages WHERE account_id = ? AND mailbox = ? ORDER BY uid",
        )
        .bind(ACC)
        .bind(mailbox)
        .fetch_all(&self.db)
        .await
        .unwrap();
        rows.into_iter()
            .map(|(uid, subject, flags)| {
                (
                    uid as u32,
                    subject.unwrap_or_default(),
                    flags
                        .and_then(|f| serde_json::from_str(&f).ok())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    fn eml(&self, mailbox: &str, uid: u32) -> Option<Vec<u8>> {
        let bytes = std::fs::read(eml_path(&self.dir, ACC, mailbox, uid)).ok()?;
        Some(zstd::decode_all(bytes.as_slice()).unwrap())
    }
}

#[tokio::test]
async fn stalwart_session_headers_and_keywords() {
    let s = server_or_skip!();
    let client = connect(&s.url, &s.user, &s.password).await;
    assert!(
        client.session.event_source_url.is_some(),
        "Stalwart offers push"
    );
    let (mailboxes, _) = client.mailboxes().await.unwrap();
    let names = mailbox_names(&mailboxes);
    let inbox = mailboxes
        .iter()
        .find(|m| m.role.as_deref() == Some("inbox"))
        .unwrap();
    assert_eq!(names[&inbox.id], "INBOX");
    for role in ["sent", "trash", "drafts"] {
        assert!(
            mailboxes.iter().any(|m| m.role.as_deref() == Some(role)),
            "a {role} mailbox"
        );
    }

    let t = tag();
    let mbox = create_mailbox(&client, &t, None).await;
    let raw = format!(
        "From: Ann Example <ann@example.org>\r\nTo: {}\r\nSubject: {t} a long\r\n subject\r\n\
         Message-ID: <{t}@example.org>\r\nIn-Reply-To: <parent-{t}@example.org>\r\n\
         Date: Tue, 24 Nov 2026 10:00:00 +0000\r\n\r\nHi\r\n",
        s.user
    );
    let id = import(&client, &[&mbox], raw.as_bytes(), &["$seen", "$flagged"]).await;
    let e = email(&client, &id).await.unwrap();
    // the raw header fields read as the IMAP path reads a header fetch
    let h = crate::imap_client::header_from_bytes(1, &e.header_bytes(), &e.flags(), e.size);
    assert_eq!(
        h.subject.as_deref(),
        Some(format!("{t} a long subject").as_str())
    );
    assert_eq!(h.from_emails, vec!["ann@example.org"]);
    assert_eq!(
        h.message_id.as_deref(),
        Some(format!("{t}@example.org").as_str())
    );
    assert_eq!(
        h.thread_id.as_deref(),
        Some(format!("parent-{t}@example.org").as_str())
    );
    // Stalwart adds $hasnoattachment, which is no flag
    assert!(
        e.keywords.keys().any(|k| k == "$hasnoattachment"),
        "{:?}",
        e.keywords
    );
    assert_eq!(e.flags(), vec!["\\Flagged", "\\Seen"]);
    assert_eq!(client.download(&e.blob_id).await.unwrap(), raw.as_bytes());
    destroy_mailboxes(&client, &[&mbox]).await;
}

#[tokio::test]
async fn stalwart_sync_and_changes() {
    let s = server_or_skip!();
    let cache = cache(&s, "password", None).await;
    let client = cache.client().await;
    let t = tag();
    let parent = create_mailbox(&client, &t, None).await;
    let sub = create_mailbox(&client, "Sub", Some(&parent)).await;
    let plain = mail(&format!("{t} plain"), "Bob <bob@example.org>");
    let with_file = mail_with_attachment(&format!("{t} file"));
    let a = import(&client, &[&parent], &plain, &[]).await;
    let b = import(&client, &[&parent, &sub], &with_file, &["$flagged"]).await;

    // the first sync lists the account
    cache.sync().await;
    let sub_name = format!("{t}/Sub");
    let rows = cache.cached(&t).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    let a_row = rows
        .iter()
        .find(|r| r.1.ends_with("plain"))
        .unwrap()
        .clone();
    let b_row = rows.iter().find(|r| r.1.ends_with("file")).unwrap().clone();
    assert!(a_row.2.is_empty(), "{:?}", a_row.2);
    assert_eq!(b_row.2, vec!["\\Flagged"]);
    assert_eq!(cache.cached(&sub_name).await.len(), 1);
    assert_eq!(
        cache.eml(&t, a_row.0).unwrap(),
        plain,
        "the source as imported"
    );
    let atts = email_db::list_attachments(&cache.db, ACC, &t, b_row.0)
        .await
        .unwrap();
    assert_eq!(atts.len(), 1);
    assert!(
        jmap_db::jmap_state(&cache.db, ACC, "Email")
            .await
            .unwrap()
            .is_some()
    );

    // Caley's changes reach the server
    let seen = vec!["\\Seen".to_owned()];
    let failed = ops::apply_changes(
        &client,
        &cache.db,
        ACC,
        &[
            (
                &t,
                a_row.0,
                ops::Change::Flags {
                    flags: &seen,
                    set: true,
                },
            ),
            (&t, b_row.0, ops::Change::Delete),
        ],
    )
    .await
    .unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    let a_now = email(&client, &a).await.unwrap();
    assert!(a_now.keywords.contains_key("$seen"));
    let b_now = email(&client, &b).await.unwrap();
    assert_eq!(
        b_now.mailbox_ids.keys().collect::<Vec<_>>(),
        vec![&sub],
        "unlinked from one mailbox only"
    );

    // the server's changes reach the cache (Email/changes)
    let failed = ops::apply_changes(
        &client,
        &cache.db,
        ACC,
        &[(&t, a_row.0, ops::Change::Move { dest: &sub_name })],
    )
    .await
    .unwrap();
    assert!(failed.is_empty(), "{failed:?}");
    let c = import(
        &client,
        &[&parent],
        &mail(&format!("{t} later"), "c@example.org"),
        &[],
    )
    .await;
    cache.sync().await;
    let parent_rows = cache.cached(&t).await;
    assert_eq!(
        parent_rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
        vec![format!("{t} later")]
    );
    let mut sub_rows: Vec<String> = cache
        .cached(&sub_name)
        .await
        .into_iter()
        .map(|r| r.1)
        .collect();
    sub_rows.sort();
    assert_eq!(sub_rows, vec![format!("{t} file"), format!("{t} plain")]);
    let moved = cache
        .cached(&sub_name)
        .await
        .into_iter()
        .find(|r| r.1.ends_with("plain"))
        .unwrap();
    assert_eq!(moved.2, vec!["\\Seen"]);
    assert!(cache.eml(&sub_name, moved.0).is_some());

    // a rename, and an email destroyed on the server
    let mut rename = serde_json::Map::new();
    rename.insert(sub.clone(), json!({ "name": "Renamed" }));
    client
        .method(
            "Mailbox/set",
            json!({ "accountId": client.account_id, "update": rename }),
        )
        .await
        .unwrap();
    client
        .set_emails(serde_json::Map::new(), vec![c])
        .await
        .unwrap();
    cache.sync().await;
    assert!(cache.cached(&sub_name).await.is_empty());
    assert_eq!(cache.cached(&format!("{t}/Renamed")).await.len(), 2);
    assert!(cache.cached(&t).await.is_empty());

    destroy_mailboxes(&client, &[&sub]).await;
    destroy_mailboxes(&client, &[&parent]).await;
    cache.sync().await;
    assert!(cache.cached(&format!("{t}/Renamed")).await.is_empty());
}

#[tokio::test]
async fn stalwart_send() {
    let s = server_or_skip!();
    let (Ok(to), Ok(to_password)) = (
        std::env::var("JMAP_TEST_TO"),
        std::env::var("JMAP_TEST_TO_PASSWORD"),
    ) else {
        eprintln!("skipped: JMAP_TEST_TO is not set");
        return;
    };
    let cache = cache(&s, "password", None).await;
    cache.sync().await;
    let client = cache.client().await;
    let t = tag();
    let raw = format!(
        "From: {}\r\nTo: {to}\r\nSubject: {t}\r\nMessage-ID: <{t}@example.com>\r\n\
         Date: Tue, 24 Nov 2026 12:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nSent by the test.\r\n",
        s.user
    );
    let sent = ops::send_raw(
        &client,
        &cache.db,
        ACC,
        &s.user,
        std::slice::from_ref(&to),
        raw.into_bytes(),
    )
    .await
    .unwrap();
    let sent_id = jmap_db::jmap_mailbox_id(&cache.db, ACC, &sent)
        .await
        .unwrap();
    // Stalwart indexes for search a moment after it stores
    let mut in_sent = Vec::new();
    for _ in 0..60 {
        in_sent = client
            .query_ids(json!({ "subject": t, "inMailbox": sent_id }))
            .await
            .unwrap();
        if !in_sent.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(in_sent.len(), 1, "the copy is in {sent}");

    let bob = connect(&s.url, &to, &to_password).await;
    let mut arrived = Vec::new();
    for _ in 0..60 {
        arrived = bob.query_ids(json!({ "subject": t })).await.unwrap();
        if !arrived.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(arrived.len(), 1, "delivered to {to}");
    bob.set_emails(serde_json::Map::new(), arrived)
        .await
        .unwrap();
    client
        .set_emails(serde_json::Map::new(), in_sent)
        .await
        .unwrap();

    // an address the account has no identity for
    let e = ops::send_raw(
        &client,
        &cache.db,
        ACC,
        "nobody@elsewhere.example",
        &[to],
        b"x".to_vec(),
    )
    .await
    .unwrap_err();
    assert!(e.downcast_ref::<ops::SendError>().is_some(), "{e}");
}

#[tokio::test]
async fn stalwart_push() {
    let s = server_or_skip!();
    let client = connect(&s.url, &s.user, &s.password).await;
    let t = tag();
    let mbox = create_mailbox(&client, &t, None).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .listen("Email,Mailbox", 30, move |changed| {
                    let _ = tx.send(changed.clone());
                })
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    import(
        &client,
        &[&mbox],
        &mail(&format!("{t} pushed"), "p@example.org"),
        &[],
    )
    .await;
    let changed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let c = rx.recv().await.unwrap();
            if c.contains_key("Email") {
                return c;
            }
        }
    })
    .await
    .expect("a StateChange for the import");
    assert!(changed.contains_key("Email"));
    listener.abort();
    destroy_mailboxes(&client, &[&mbox]).await;
}

/// Signs in through Stalwart's login API as its login page does, and takes the code to the token endpoint.
async fn oauth_sign_in(s: &Server, scope: &str) -> Value {
    let client_id = std::env::var("JMAP_TEST_OAUTH_CLIENT_ID").unwrap();
    let secret = std::env::var("JMAP_TEST_OAUTH_CLIENT_SECRET").unwrap();
    let redirect = std::env::var("JMAP_TEST_OAUTH_REDIRECT").unwrap();
    let http = http_client().unwrap();
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", scope)
        .append_pair("state", "st")
        .append_pair("nonce", "nn")
        .finish();
    let login = format!("{}/api/auth", s.url);
    let post = |body: Value| {
        let http = http.clone();
        let login = login.clone();
        async move {
            let r = http
                .post(login)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            serde_json::from_slice::<Value>(&r.bytes().await.unwrap()).unwrap()
        }
    };
    let mut r = post(json!({
        "type": "authCode", "accountName": s.user, "accountSecret": s.password, "query": query,
    }))
    .await;
    if r["type"] == "consentRequired" {
        r = post(
            json!({ "type": "authConsent", "token": r["token"], "query": query, "consent": true }),
        )
        .await;
    }
    let location = url::Url::parse(
        r["location"]
            .as_str()
            .unwrap_or_else(|| panic!("login: {r}")),
    )
    .unwrap();
    let code = location
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", &code)
        .append_pair("redirect_uri", &redirect)
        .finish();
    let r = http
        .post(format!("{}/auth/token", s.url))
        .basic_auth(&client_id, Some(&secret))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .unwrap();
    serde_json::from_slice(&r.bytes().await.unwrap()).unwrap()
}

#[tokio::test]
async fn stalwart_oidc_sign_in_syncs_and_renews() {
    let s = server_or_skip!();
    if std::env::var("JMAP_TEST_OAUTH_CLIENT_ID").is_err() {
        eprintln!("skipped: JMAP_TEST_OAUTH_CLIENT_ID is not set");
        return;
    }
    let tokens = oauth_sign_in(&s, &format!("openid offline_access {MAIL_SCOPE}")).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"]
        .as_str()
        .expect("offline_access gives a refresh token");

    let refresher = Refresher {
        http: http_client().unwrap(),
        token_url: format!("{}/auth/token", s.url),
        client_id: std::env::var("JMAP_TEST_OAUTH_CLIENT_ID").unwrap(),
        client_secret: std::env::var("JMAP_TEST_OAUTH_CLIENT_SECRET").unwrap(),
    };
    let cache = cache(&s, "oidc", Some(refresher)).await;
    // stored as the OIDC callback stores it, then made to look expired: the sync renews it at Stalwart first
    cache
        .tokens
        .store(USER, &access, Some(refresh), Some(Duration::from_secs(0)))
        .await
        .unwrap();
    let client = cache.client().await;
    let renewed = cache.tokens.access_token(USER).await.unwrap();
    assert_ne!(renewed, access, "renewed at the token endpoint");

    let t = tag();
    let mbox = create_mailbox(&client, &t, None).await;
    import(
        &client,
        &[&mbox],
        &mail(&format!("{t} oidc"), "o@example.org"),
        &[],
    )
    .await;
    cache.sync().await;
    assert_eq!(cache.cached(&t).await.len(), 1);
    destroy_mailboxes(&client, &[&mbox]).await;
}

#[tokio::test]
async fn stalwart_a_token_without_the_mail_scope_names_it() {
    let s = server_or_skip!();
    if std::env::var("JMAP_TEST_OAUTH_CLIENT_ID").is_err() {
        return;
    }
    let tokens = oauth_sign_in(&s, "openid offline_access").await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let e = Client::connect(&http_client().unwrap(), &s.url, Auth::Bearer(access))
        .await
        .err()
        .unwrap();
    assert!(e.to_string().contains(MAIL_SCOPE), "{e}");
}

#[tokio::test]
async fn stalwart_wrong_password() {
    let s = server_or_skip!();
    let e = Client::connect(
        &http_client().unwrap(),
        &s.url,
        Auth::Basic {
            username: s.user.clone(),
            password: "wrong".into(),
        },
    )
    .await
    .err()
    .unwrap();
    assert!(is_unauthorized(&e), "{e}");
}

/// A JMAP account that signs in at Stalwart (auth = 'oauth'), through Caley's handlers: saving answers the link,
/// which redirects to Stalwart's sign-in; its login page is played through its login API (the calls the page makes),
/// and Stalwart's redirect back saves the account. Then the sync, and a renewal at Stalwart's token endpoint.
#[tokio::test]
async fn stalwart_an_account_signs_in_at_the_server() {
    use super::api_tests::{USER as APP_USER, app_state, save_from, session};
    use axum::extract::{Query, State};
    let s = server_or_skip!();
    let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("JMAP_TEST_OAUTH_CLIENT_ID"),
        std::env::var("JMAP_TEST_OAUTH_CLIENT_SECRET"),
    ) else {
        eprintln!("skipped: JMAP_TEST_OAUTH_CLIENT_ID is not set");
        return;
    };
    let app = app_state().await;
    let sess = session(&app).await;
    let callback = |query: String| {
        let (app, sess) = (app.clone(), sess.clone());
        async move {
            let uri: axum::http::Uri = format!("/auth/jmap?{query}").parse().unwrap();
            let r =
                super::oauth::callback(State(app), sess, Query::try_from_uri(&uri).unwrap()).await;
            r.headers()["location"].to_str().unwrap().to_owned()
        }
    };

    let body = crate::email::AccountBody {
        account_id: None,
        display_name: "Stalwart".into(),
        protocol: Some(crate::email::Protocol::Jmap),
        jmap_url: Some(s.url.clone()),
        auth: Some(crate::email::AccountAuth::Oauth),
        host: String::new(),
        port: 0,
        username: s.user.clone(),
        password: None,
        use_tls: None,
        smtp_host: None,
        smtp_port: None,
        sender_name: Some("Alice".into()),
        oauth_client_id: Some(client_id),
        oauth_client_secret: Some(client_secret),
        authorize: None,
    };
    let r = save_from(&app, &sess, "http://127.0.0.1:3000", body).await;
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    let saved: Value = serde_json::from_slice(&bytes).unwrap();
    let start = saved["authorize"]
        .as_str()
        .unwrap_or_else(|| panic!("{saved}"))
        .strip_prefix("/auth/jmap?")
        .unwrap();

    // Caley sends the browser to Stalwart's sign-in, whose page signs in through /api/auth with the request's query
    let to_server = url::Url::parse(&callback(start.to_owned()).await).unwrap();
    let query = to_server.query().unwrap().to_owned();
    let http = http_client().unwrap();
    let login = |body: Value| {
        let (http, url) = (http.clone(), format!("{}/api/auth", s.url));
        async move {
            let r = http
                .post(url)
                .header("content-type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .unwrap();
            serde_json::from_slice::<Value>(&r.bytes().await.unwrap()).unwrap()
        }
    };
    let mut r = login(json!({ "type": "authCode", "accountName": s.user, "accountSecret": s.password, "query": query })).await;
    if r["type"] == "consentRequired" {
        r = login(
            json!({ "type": "authConsent", "token": r["token"], "query": query, "consent": true }),
        )
        .await;
    }
    let back = url::Url::parse(
        r["location"]
            .as_str()
            .unwrap_or_else(|| panic!("login: {r}")),
    )
    .unwrap();
    assert_eq!(back.path(), "/auth/jmap");
    let to_mail = callback(back.query().unwrap().to_owned()).await;
    assert!(to_mail.contains("account_id="), "{to_mail}");

    let mut accounts = email_db::list_accounts(&app.db, APP_USER, &app.encryption_key)
        .await
        .unwrap();
    let account = accounts.remove(0);
    assert_eq!(account.auth, "oauth");
    let mgr = &app.email_sync_manager;
    mgr.stop_task_for(&account.account_id);
    mgr.stop_idle_for(&account.account_id);

    let client = mgr.jmap_client(&account, APP_USER).await.unwrap();
    let t = tag();
    let mbox = create_mailbox(&client, &t, None).await;
    import(
        &client,
        &[&mbox],
        &mail(&format!("{t} via oauth"), "o@example.org"),
        &[],
    )
    .await;
    mgr.jmap_sync(&account, APP_USER, None).await.unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cached_messages WHERE account_id = ? AND mailbox = ?",
    )
    .bind(&account.account_id)
    .bind(&t)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(n, 1);

    // expired: renewed at Stalwart's token endpoint with the refresh token
    sqlx::query("UPDATE jmap_oauth_tokens SET expires_at = 0")
        .execute(&app.db)
        .await
        .unwrap();
    mgr.jmap_sync(&account, APP_USER, None).await.unwrap();
    let expires: i64 = sqlx::query_scalar("SELECT expires_at FROM jmap_oauth_tokens")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(expires > crate::oidc_tokens::now(), "renewed");
    destroy_mailboxes(&client, &[&mbox]).await;
}

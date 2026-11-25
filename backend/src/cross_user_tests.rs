//! One user must never read, change, delete or take over another user's data through the API.
//!
//! The whole `/api` router runs here with two users: Alice, whose data is seeded (mail account, message and its
//! file, identity, virtual folder, CalDAV account, calendar, event, address book, contact, passkey, OpenPGP key and
//! vault, push subscription), and Mallory, who calls every route naming Alice's ids. After each call
//!
//! - the response must not contain Alice's data (every seeded text carries [`SECRET`]), and
//! - every row of Alice's (any table, found by its `user_id`, account, folder or credential columns) and every file
//!   of her account must be as it was.
//!
//! Every route of the OpenAPI spec must be listed: in [`cases`] (called with Alice's ids) or in [`NO_FOREIGN_IDS`]
//! (it names nothing another user could own, with the reason). A new route fails [`every_route_is_covered`] until it
//! is one or the other, so its cross-user behaviour is decided when it is added.
//!
//! The ownership rules the routes follow (see `email_db::MailAccountId`, `db::CalDavAccountId`): a handler turns an
//! id from a request into an account only through `email::owned_account` / `calendar::owned_account`, the
//! functions that read or change an account's data take the proof that gives, and an upsert of a row with a
//! client-named id never updates another user's row (`email_db::NotOwned`). [`handlers_never_skip_the_check`] keeps
//! the unchecked constructors out of the handler modules.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use sqlx::Row;
use tower::ServiceExt;

use crate::db::{self, CalDavAccountId};
use crate::email_db::{self, AccountProtocol, MailAccountId};
use crate::oidc::AppState;

const ALICE: &str = "alice@cal.example.com";
const MALLORY: &str = "mallory@cal.example.com";
/// In every text of Alice's: no answer to Mallory may contain it.
const SECRET: &str = "ALICESECRET";

const A_MAIL: &str = "alice-mail";
const A_CAL: &str = "alice-cal";
const A_IDENTITY: &str = "alice-identity";
const A_FOLDER: &str = "alice-folder";
const A_CRED: &str = "alice-cred";
const A_ENDPOINT: &str = "https://push.example/alice";
const M_MAIL: &str = "mallory-mail";
const M_CAL: &str = "mallory-cal";

/// The routes that take no id of anything a user owns: what they read or change is the caller's own by their user
/// id alone. (Each with why; a route that takes an id belongs in [`cases`].)
const NO_FOREIGN_IDS: &[(&str, &str, &str)] = &[
    ("GET", "/auth/config", "public, no user data"),
    (
        "GET",
        "/events/stream",
        "the caller's channel (SseBroadcaster is keyed by user id); a stream, never ends",
    ),
    (
        "GET",
        "/email/autodiscover",
        "looks up an address's servers, no user data",
    ),
    (
        "POST",
        "/settings",
        "the caller's own `{user_id}:default` CalDAV account; tests the connection first",
    ),
    (
        "POST",
        "/webauthn/register/start",
        "the caller's own credentials (the challenge excludes them)",
    ),
    (
        "POST",
        "/webauthn/register/finish",
        "needs the session's registration; a credential id of another user's is refused by \
         store_webauthn_credential (passkey_ids_are_never_taken_over)",
    ),
    (
        "POST",
        "/webauthn/authenticate/finish",
        "needs the session's challenge, made for the caller's credentials",
    ),
    (
        "POST",
        "/notifications/test",
        "sends to the caller's own devices",
    ),
    ("GET", "/notifications/vapid", "the server's public key"),
    (
        "DELETE",
        "/account",
        "deletes the caller (called last in the run, after every case)",
    ),
];

#[derive(Clone)]
enum Payload {
    None,
    Json(Value),
    Form(String),
    Multipart(Vec<(&'static str, String)>),
}

struct Case {
    method: Method,
    /// with its query
    uri: String,
    payload: Payload,
}

fn case(method: &str, uri: impl Into<String>, payload: Payload) -> Case {
    Case {
        method: method.parse().unwrap(),
        uri: uri.into(),
        payload,
    }
}

fn form(pairs: &[(&str, &str)]) -> Payload {
    Payload::Form(
        pairs
            .iter()
            .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

const MSG: &str = "account_id=alice-mail&mailbox=INBOX&uid=1";
const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:ev1\r\nDTSTART:20260105T100000Z\r\n\
    DTEND:20260105T110000Z\r\nSUMMARY:ALICESECRET event\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
/// What Mallory uploads (her own text: an answer may show it)
const MALLORY_ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:ev1\r\nDTSTART:20260105T100000Z\r\n\
    DTEND:20260105T110000Z\r\nSUMMARY:Mallory event\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Mallory's calls, each naming Alice's ids (or nothing, for the lists: they must not show Alice's data).
fn cases() -> Vec<Case> {
    let msg_ref = json!({ "account_id": A_MAIL, "mailbox": "INBOX", "uid": 1 });
    let device = json!({
        "cred_id": A_CRED, "public_jwk": {}, "encrypted_private_jwk": "x", "private_iv": "x",
        "prf_salt": "x", "hkdf_salt": "x", "pairing_nonce": "x",
    });
    let wrap = json!({ "cred_id": A_CRED, "ephemeral_public_jwk": {}, "hkdf_salt": "x", "iv": "x", "ciphertext": "x" });
    let account_body = |extra: Value| {
        let mut b = json!({
            "account_id": A_MAIL, "display_name": "Mallory", "host": "127.0.0.1", "port": 1,
            "username": "mallory", "password": "pw",
        });
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        Payload::Json(b)
    };
    let caldav_form = |id: &str| {
        form(&[
            ("account_id", id),
            ("caldav_base_url", "http://127.0.0.1:1/"),
            ("caldav_username", "mallory"),
            ("caldav_password", "pw"),
        ])
    };
    let event_form = |account_id: &str, original: Option<&str>| {
        let mut f = vec![
            ("uid", "ev1"),
            ("summary", "Mallory"),
            ("description", ""),
            ("location", ""),
            ("date", "2026-01-05"),
            ("calendar_href", "/cal/"),
            ("account_id", account_id),
        ];
        if let Some(o) = original {
            f.push(("original_account_id", o));
            f.push(("original_calendar_href", "/cal/"));
        }
        form(&f)
    };
    vec![
        // ── mail accounts ──
        case("GET", "/api/email/accounts", Payload::None),
        case("POST", "/api/email/account", account_body(json!({}))),
        case("PUT", "/api/email/account", account_body(json!({}))),
        case(
            "POST",
            "/api/email/account",
            account_body(json!({
                "protocol": "jmap", "auth": "oauth", "jmap_url": "https://jmap.example/",
                "oauth_client_id": "c", "authorize": true,
            })),
        ),
        case(
            "DELETE",
            format!("/api/email/account?account_id={A_MAIL}"),
            Payload::None,
        ),
        case("DELETE", "/api/email/account?account_id=..", Payload::None),
        case("DELETE", "/api/email/account?account_id=", Payload::None),
        case(
            "GET",
            format!("/api/email/mailboxes?account_id={A_MAIL}"),
            Payload::None,
        ),
        case(
            "POST",
            format!("/api/email/sync?account_id={A_MAIL}&mailbox=INBOX"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/export?account_id={A_MAIL}"),
            Payload::None,
        ),
        case("GET", "/api/email/unread-counts", Payload::None),
        // ── messages ──
        case(
            "GET",
            format!("/api/email/messages?account_id={A_MAIL}&mailbox=INBOX"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/messages?account_id={A_MAIL}&mailbox=INBOX&offset=0"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/messages?folder_id={A_FOLDER}"),
            Payload::None,
        ),
        case("GET", format!("/api/email/message?{MSG}"), Payload::None),
        case(
            "GET",
            format!("/api/email/message/raw?{MSG}"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/message/raw?{MSG}&omit=10"),
            Payload::None,
        ),
        case("GET", format!("/api/email/raw?{MSG}"), Payload::None),
        case("GET", format!("/api/email/thread?{MSG}"), Payload::None),
        case(
            "GET",
            format!("/api/email/thread-events?{MSG}"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/message-events?{MSG}"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/attachment?{MSG}&filename=secret.txt"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/proxy-image?{MSG}&cid=cid1"),
            Payload::None,
        ),
        case("DELETE", format!("/api/email/message?{MSG}"), Payload::None),
        case(
            "PATCH",
            "/api/email/message/flags",
            Payload::Json(
                json!({ "account_id": A_MAIL, "mailbox": "INBOX", "uid": 1, "flags": ["\\Seen"], "set": true }),
            ),
        ),
        case(
            "POST",
            "/api/email/message/move",
            Payload::Json(
                json!({ "account_id": A_MAIL, "mailbox": "INBOX", "uid": 1, "dest_mailbox": "Trash" }),
            ),
        ),
        case(
            "PATCH",
            "/api/email/messages/flags",
            Payload::Json(json!({ "messages": [msg_ref], "flags": ["\\Seen"], "set": true })),
        ),
        case(
            "POST",
            "/api/email/messages/move",
            Payload::Json(
                json!({ "messages": [{ "account_id": A_MAIL, "mailbox": "INBOX", "uid": 1, "dest_mailbox": "Trash" }] }),
            ),
        ),
        case(
            "POST",
            "/api/email/messages/delete",
            Payload::Json(json!({ "messages": [msg_ref] })),
        ),
        case(
            "POST",
            "/api/email/trash/empty",
            Payload::Json(json!({ "account_id": A_MAIL, "mailbox": "Trash", "days": 0 })),
        ),
        case(
            "GET",
            format!("/api/email/search?q={SECRET}"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/search?q={SECRET}&account_id={A_MAIL}&mailbox=INBOX"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/search?q={SECRET}&folder_id={A_FOLDER}"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/email/search?addrs=alice%40example.com"),
            Payload::None,
        ),
        case("GET", "/api/email/suggest-addrs?q=alice", Payload::None),
        // ── identities, folders, sending ──
        case("GET", "/api/email/identities", Payload::None),
        case(
            "POST",
            "/api/email/identity",
            Payload::Json(
                json!({ "identity_id": A_IDENTITY, "account_id": M_MAIL, "name": "Mallory", "email": "m@example.com" }),
            ),
        ),
        case(
            "PUT",
            "/api/email/identity",
            Payload::Json(
                json!({ "identity_id": A_IDENTITY, "account_id": A_MAIL, "name": "Mallory", "email": "m@example.com", "is_default": true }),
            ),
        ),
        case(
            "POST",
            "/api/email/identity",
            Payload::Json(
                json!({ "account_id": A_MAIL, "name": "Mallory", "email": "m@example.com" }),
            ),
        ),
        case(
            "DELETE",
            format!("/api/email/identity?identity_id={A_IDENTITY}"),
            Payload::None,
        ),
        case(
            "POST",
            "/api/email/send-raw",
            Payload::Multipart(vec![
                ("identity_id", A_IDENTITY.into()),
                ("envelope_to", "[\"m@example.com\"]".into()),
                ("raw_mime", "Subject: x\r\n\r\nx".into()),
            ]),
        ),
        case("GET", "/api/email/folders", Payload::None),
        case(
            "POST",
            "/api/email/folder",
            Payload::Json(
                json!({ "display_name": "Mallory", "rules": [{ "account_id": A_MAIL, "mailbox_pattern": null }] }),
            ),
        ),
        case(
            "POST",
            "/api/email/folder",
            Payload::Json(json!({ "folder_id": A_FOLDER, "display_name": "Mallory", "rules": [] })),
        ),
        case(
            "DELETE",
            format!("/api/email/folder?folder_id={A_FOLDER}"),
            Payload::None,
        ),
        // the image lists are keyed by the user: Alice's origin is Mallory's own row
        case("GET", "/api/email/image-origins", Payload::None),
        case(
            "POST",
            "/api/email/image-origins",
            Payload::Json(json!({ "origin": "https://alice.example" })),
        ),
        case(
            "DELETE",
            "/api/email/image-origins?origin=https%3A%2F%2Falice.example",
            Payload::None,
        ),
        case("GET", "/api/email/image-blacklist", Payload::None),
        case(
            "POST",
            "/api/email/image-blacklist",
            Payload::Json(json!({ "origin": "https://ignored.example" })),
        ),
        case(
            "DELETE",
            "/api/email/image-blacklist?origin=https%3A%2F%2Fignored.example",
            Payload::None,
        ),
        // ── CalDAV accounts, calendars, events ──
        case("GET", "/api/caldav/accounts", Payload::None),
        case("POST", "/api/caldav/account", caldav_form(A_CAL)),
        case("PUT", "/api/caldav/account", caldav_form(A_CAL)),
        case(
            "POST",
            "/api/caldav/account",
            caldav_form(&format!("{ALICE}:default")),
        ),
        case(
            "DELETE",
            format!("/api/caldav/account?account_id={A_CAL}"),
            Payload::None,
        ),
        case("GET", "/api/settings/form", Payload::None),
        case("GET", "/api/user", Payload::None),
        case("GET", "/api/calendars", Payload::None),
        case("GET", "/api/calendar?month=2026-01", Payload::None),
        case(
            "POST",
            "/api/calendar",
            form(&[("account_id", A_CAL), ("name", "Mallory")]),
        ),
        case(
            "POST",
            "/api/calendar/toggle",
            form(&[
                ("account_id", A_CAL),
                ("href", "/cal/"),
                ("active", "false"),
            ]),
        ),
        case(
            "GET",
            "/api/events?start=2026-01-01&end=2026-01-31",
            Payload::None,
        ),
        case("POST", "/api/event", event_form(A_CAL, None)),
        case("POST", "/api/event", event_form(M_CAL, Some(A_CAL))),
        case(
            "DELETE",
            format!("/api/event?uid=ev1&account_id={A_CAL}&calendar_href=%2Fcal%2F"),
            Payload::None,
        ),
        case(
            "DELETE",
            format!(
                "/api/event?uid=ev1&account_id={A_CAL}&calendar_href=%2Fcal%2F&scope=this_instance&original_dtstart=20260105T100000Z"
            ),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/event/ics?account_id={A_CAL}&calendar_href=%2Fcal%2F&uid=ev1"),
            Payload::None,
        ),
        case(
            "GET",
            format!(
                "/api/calendar/event-emails?cal_account_id={A_CAL}&calendar_href=%2Fcal%2F&event_uid=ev1"
            ),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/search?q={SECRET}&start=2025-01-01&end=2027-01-01"),
            Payload::None,
        ),
        case(
            "POST",
            "/api/import",
            Payload::Multipart(vec![
                ("account_id", A_CAL.into()),
                ("calendar_href", "/cal/".into()),
                ("file", MALLORY_ICS.into()),
            ]),
        ),
        case(
            "POST",
            "/api/import",
            Payload::Multipart(vec![
                ("account_id", M_CAL.into()),
                ("calendar_href", "/cal/".into()),
                ("file", MALLORY_ICS.into()),
                ("email_account_id", A_MAIL.into()),
                ("email_mailbox", "INBOX".into()),
                ("email_uid", "1".into()),
            ]),
        ),
        case(
            "POST",
            "/api/import/preview",
            Payload::Multipart(vec![("file", MALLORY_ICS.into())]),
        ),
        case("POST", "/api/push/webhook?token=", Payload::None),
        case("POST", "/api/push/webhook?token=unknown", Payload::None),
        // ── contacts ──
        case("GET", "/api/contacts", Payload::None),
        case(
            "POST",
            "/api/contact",
            Payload::Json(json!({
                "uid": "c1", "account_id": A_CAL, "addressbook_href": "/book/", "fn_name": "Mallory",
                "emails": [], "phones": [], "addresses": [], "org": "",
            })),
        ),
        case(
            "DELETE",
            "/api/contact",
            Payload::Json(
                json!({ "account_id": A_CAL, "uid": "c1", "addressbook_href": "/book/" }),
            ),
        ),
        case(
            "GET",
            format!("/api/contact/photo?account_id={A_CAL}&uid=c1&addressbook_href=%2Fbook%2F"),
            Payload::None,
        ),
        case(
            "GET",
            format!("/api/contact/vcf?account_id={A_CAL}&uid=c1&addressbook_href=%2Fbook%2F"),
            Payload::None,
        ),
        // ── passkeys, OpenPGP ──
        case("GET", "/api/webauthn/credentials", Payload::None),
        case(
            "DELETE",
            "/api/webauthn/credential",
            Payload::Json(json!({ "cred_id": A_CRED })),
        ),
        case(
            "POST",
            "/api/webauthn/authenticate/start",
            Payload::Json(json!({ "prf_salt": "AAAA", "credential_id": A_CRED })),
        ),
        case("GET", "/api/pgp/keys", Payload::None),
        case(
            "GET",
            "/api/pgp/keys?email=alice%40example.com",
            Payload::None,
        ),
        case(
            "POST",
            "/api/pgp/key",
            Payload::Json(
                json!({ "fingerprint": "ALICEFPR", "email": "m@example.com", "armored": "x" }),
            ),
        ),
        case(
            "DELETE",
            "/api/pgp/key",
            Payload::Json(json!({ "fingerprint": "ALICEFPR" })),
        ),
        case(
            "POST",
            "/api/pgp/key/accept",
            Payload::Json(json!({ "fingerprint": "ALICEHARVESTED" })),
        ),
        case("GET", "/api/pgp/vault", Payload::None),
        case(
            "POST",
            "/api/pgp/vault/initialize",
            Payload::Json(json!({
                "manifest": { "iv": "x", "ciphertext": "x" },
                "key": { "fingerprint": "ALICEFPR", "iv": "x", "ciphertext": "x" },
                "device": device, "wrap": wrap,
            })),
        ),
        case(
            "POST",
            "/api/pgp/devices/pending",
            Payload::Json(json!({ "device": device })),
        ),
        case(
            "PUT",
            "/api/pgp/vault",
            Payload::Json(json!({
                "expected_revision": 1,
                "manifest": { "iv": "x", "ciphertext": "x" },
                "delete_keys": ["ALICEFPR"],
                "approve_device": { "cred_id": A_CRED, "wrap": wrap },
                "remove_device": A_CRED,
            })),
        ),
        case("DELETE", "/api/pgp/vault", Payload::None),
        // ── notifications ──
        case(
            "PUT",
            "/api/notifications/subscription",
            Payload::Json(json!({
                "endpoint": A_ENDPOINT,
                "p256dh": MALLORY_P256DH,
                "auth": "AAAAAAAAAAAAAAAAAAAAAA",
            })),
        ),
        case(
            "DELETE",
            "/api/notifications/subscription",
            Payload::Json(json!({ "endpoint": A_ENDPOINT })),
        ),
    ]
}

/// Valid P-256 public keys (uncompressed: the curve's G for Mallory, 2G for Alice's device).
const MALLORY_P256DH: &str =
    "BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU";
const ALICE_P256DH: &str =
    "BHzyexiNA09-ilI4AwS1GsPAiWnid_IbNaYLSPxHZpl4B3dVENuO0EApPZrGn3Qw27p9reY86YIpngS3nSJ4c9E";

// ── the run ──────────────────────────────────────────────────────────────────

async fn state() -> AppState {
    let mut state = crate::jmap::api_tests::app_state().await;
    // push endpoints on any address, so a subscription reaches the database (it is not sent to)
    state.notifier = Arc::new(
        crate::notifications::Notifier::new(
            state.db.clone(),
            state.encryption_key,
            "http://localhost:3000",
            true,
        )
        .unwrap(),
    );
    for user in [ALICE, MALLORY] {
        db::ensure_user(&state.db, user, None).await.unwrap();
    }
    state
}

/// The `/api` router as `main` mounts it, the caller given by the `x-test-user` header (a session of theirs).
fn router(state: &AppState) -> Router {
    let api = Router::new()
        .nest("/api", crate::api::private().split_for_parts().0)
        .nest("/api", crate::api::public().split_for_parts().0)
        .with_state(state.clone());
    let store = Arc::new(crate::session_store::SqliteStore::new(state.db.clone()));
    api.layer(axum::middleware::from_fn(
        move |mut request: Request<Body>, next: axum::middleware::Next| {
            let store = Arc::clone(&store);
            async move {
                let _ = store.migrate().await;
                let session = tower_sessions::Session::new(None, store, None);
                if let Some(user) = request.headers().get("x-test-user") {
                    session
                        .insert("user_id", user.to_str().unwrap())
                        .await
                        .unwrap();
                }
                request.extensions_mut().insert(session);
                next.run(request).await
            }
        },
    ))
}

async fn seed(state: &AppState) {
    let pool = &state.db;
    let key = &state.encryption_key;
    let imap = AccountProtocol {
        protocol: "imap",
        jmap_url: None,
        auth: "password",
        oauth_client_id: None,
        oauth_client_secret: None,
    };
    for (user, id) in [(ALICE, A_MAIL), (MALLORY, M_MAIL)] {
        email_db::upsert_account(
            pool,
            user,
            id,
            &format!("{SECRET} mail"),
            "127.0.0.1",
            1,
            "user",
            "pw",
            false,
            None,
            None,
            &imap,
            key,
        )
        .await
        .unwrap();
    }
    // Mallory's display name is not Alice's secret
    sqlx::query("UPDATE imap_accounts SET display_name = 'Mallory' WHERE account_id = ?")
        .bind(M_MAIL)
        .execute(pool)
        .await
        .unwrap();
    for mailbox in ["INBOX", "Trash"] {
        email_db::upsert_mailbox(pool, A_MAIL, mailbox, Some("/"), Some(1), true)
            .await
            .unwrap();
    }
    for (mailbox, uid) in [("INBOX", 1), ("Trash", 2)] {
        email_db::upsert_message_header(
            pool,
            A_MAIL,
            mailbox,
            &email_db::MessageHeader {
                uid,
                message_id: Some(format!("<{uid}@alice>")),
                thread_id: Some("<1@alice>".into()),
                from_addr: Some(format!("{SECRET} <alice@example.com>")),
                to_addrs: Some("[\"alice@example.com\"]".into()),
                cc_addrs: None,
                reply_to_addrs: Some("[]".into()),
                subject: Some(format!("{SECRET} subject")),
                sent_at: Some(1_700_000_000),
                flags: Some("[]".into()),
                has_attachments: true,
                size: Some(100),
                from_emails: vec!["alice@example.com".into()],
                to_emails: vec!["alice@example.com".into()],
                cc_emails: vec![],
            },
            Some(&SECRET.to_lowercase()),
            Some(&SECRET.to_lowercase()),
            None,
        )
        .await
        .unwrap();
        let raw = format!(
            "From: alice@example.com\r\nSubject: {SECRET}\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\n{SECRET} body\r\n--b\r\nContent-Type: image/png\r\n\
             Content-ID: <cid1>\r\nContent-Disposition: attachment; filename=secret.txt\r\n\r\n{SECRET}\r\n--b--\r\n"
        );
        let path = crate::email_sync::eml_path(
            &state.data_dir,
            MailAccountId::internal(A_MAIL),
            mailbox,
            uid as u32,
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, zstd::encode_all(raw.as_bytes(), 3).unwrap()).unwrap();
        email_db::insert_attachment(
            pool,
            A_MAIL,
            mailbox,
            uid as u32,
            "secret.txt",
            "image/png",
            10,
            2,
            Some("cid1"),
        )
        .await
        .unwrap();
    }
    email_db::upsert_identity(
        pool,
        A_IDENTITY,
        MailAccountId::internal(A_MAIL),
        ALICE,
        &format!("{SECRET} name"),
        "alice@example.com",
        true,
    )
    .await
    .unwrap();
    email_db::create_virtual_folder(
        pool,
        A_FOLDER,
        ALICE,
        &format!("{SECRET} folder"),
        &[(None, None)],
        false,
        false,
    )
    .await
    .unwrap();

    for (user, id) in [(ALICE, A_CAL), (MALLORY, M_CAL)] {
        db::upsert_caldav_account(
            pool,
            user,
            Some(id),
            Some("calendars"),
            &db::UserSettings {
                caldav_base_url: "http://127.0.0.1:1/".into(),
                caldav_username: "user".into(),
                caldav_password: format!("{SECRET} password"),
            },
            key,
        )
        .await
        .unwrap();
    }
    db::upsert_cached_calendar(pool, A_CAL, "/cal/", &format!("{SECRET} calendar"), None)
        .await
        .unwrap();
    db::upsert_cached_event_raw(
        pool,
        CalDavAccountId::internal(A_CAL),
        "/cal/",
        "/cal/ev1.ics",
        "1",
        ICS,
        &crate::search::SearchMode::Default,
    )
    .await
    .unwrap();
    db::upsert_cached_addressbook(pool, A_CAL, "/book/", &format!("{SECRET} book"))
        .await
        .unwrap();
    db::upsert_contact(
        pool,
        CalDavAccountId::internal(A_CAL),
        "/book/",
        "c1",
        "1",
        "/book/c1.vcf",
        &format!("BEGIN:VCARD\r\nVERSION:3.0\r\nUID:c1\r\nFN:{SECRET}\r\nEND:VCARD\r\n"),
        &format!("{SECRET} contact"),
        "[\"alice@example.com\"]",
        "[]",
        "[]",
        "",
        Some("data:image/png;base64,QUxJQ0VTRUNSRVQ="),
    )
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO email_imported_events
         (email_account_id, email_mailbox, email_uid, email_message_id, cal_account_id, calendar_href, event_uid)
         VALUES (?, 'INBOX', 1, '<1@alice>', ?, '/cal/', 'ev1')",
    )
    .bind(A_MAIL)
    .bind(A_CAL)
    .execute(pool)
    .await
    .unwrap();

    db::store_webauthn_credential(
        pool,
        A_CRED,
        ALICE,
        "{}",
        Some(&format!("{SECRET} device")),
        true,
    )
    .await
    .unwrap();
    db::store_harvested_public_key(
        pool,
        ALICE,
        "ALICEHARVESTED",
        "bob@example.com",
        SECRET,
        "harvested_autocrypt",
    )
    .await
    .unwrap();
    db::upsert_pgp_public_key(
        pool,
        ALICE,
        "ALICEFPR",
        "alice@example.com",
        SECRET,
        true,
        "own",
        "trusted",
    )
    .await
    .unwrap();
    for sql in [
        "INSERT INTO pgp_vaults(user_id, revision, manifest_iv, encrypted_manifest, created_at, updated_at)
         VALUES (?1, 1, 'iv', 'ALICESECRET', 0, 0)",
        "INSERT INTO pgp_private_keys(user_id, fingerprint, iv, ciphertext) VALUES (?1, 'ALICEFPR', 'iv', 'ALICESECRET')",
        "INSERT INTO pgp_devices(cred_id, user_id, status, public_jwk, encrypted_private_jwk, private_iv, prf_salt,
         hkdf_salt, pairing_nonce, created_at)
         VALUES ('alice-cred', ?1, 'pending', '{}', 'ALICESECRET', 'x', 'x', 'x', 'x', 0)",
    ] {
        sqlx::query(sql).bind(ALICE).execute(pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO notification_subscriptions(endpoint, user_id, p256dh, auth, created_at) VALUES (?, ?, ?, ?, 0)",
    )
    .bind(A_ENDPOINT)
    .bind(ALICE)
    .bind(ALICE_P256DH)
    .bind("BBBBBBBBBBBBBBBBBBBBBB")
    .execute(pool)
    .await
    .unwrap();
}

/// Every row of Alice's, in every table: by its user, its account (mail or CalDAV), its folder or its credential.
async fn snapshot(state: &AppState) -> BTreeSet<String> {
    let pool = &state.db;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
           AND name NOT LIKE '%_fts%' AND name NOT LIKE '_sqlx%'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let owner_clauses = [
        ("user_id", "= ?1"),
        (
            "account_id",
            "IN (SELECT account_id FROM imap_accounts WHERE user_id = ?1 UNION SELECT account_id FROM caldav_accounts WHERE user_id = ?1 UNION SELECT ?2 UNION SELECT ?3)",
        ),
        ("email_account_id", "IN (?2)"),
        ("cal_account_id", "IN (?3)"),
        ("folder_id", "IN (?4)"),
        ("cred_id", "IN (?5)"),
        ("endpoint", "IN (?6)"),
    ];
    let mut rows = BTreeSet::new();
    for table in tables {
        let columns: Vec<String> =
            sqlx::query(sqlx::AssertSqlSafe(format!("PRAGMA table_info({table})")))
                .fetch_all(pool)
                .await
                .unwrap()
                .iter()
                .map(|r| r.get::<String, _>("name"))
                .collect();
        let conditions: Vec<String> = owner_clauses
            .iter()
            .filter(|(c, _)| columns.iter().any(|col| col == c))
            .map(|(c, cond)| format!("{c} {cond}"))
            .collect();
        if conditions.is_empty() {
            continue;
        }
        let select = columns
            .iter()
            .map(|c| format!("quote({c})"))
            .collect::<Vec<_>>()
            .join(" || '|' || ");
        let sql = format!(
            "SELECT {select} FROM {table} WHERE {}",
            conditions.join(" OR ")
        );
        let found: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(ALICE)
            .bind(A_MAIL)
            .bind(A_CAL)
            .bind(A_FOLDER)
            .bind(A_CRED)
            .bind(A_ENDPOINT)
            .fetch_all(pool)
            .await
            .unwrap();
        rows.extend(found.into_iter().map(|r| format!("{table}: {r}")));
    }
    // her mail files (and that the data directory, the database's, is still there)
    let dir = crate::email_sync::account_dir(&state.data_dir, MailAccountId::internal(A_MAIL));
    for entry in walk(&dir) {
        rows.insert(format!(
            "file: {}",
            entry.strip_prefix(&state.data_dir).unwrap().display()
        ));
    }
    rows
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

async fn call(app: &Router, user: &str, case: &Case) -> (StatusCode, String) {
    let builder = Request::builder()
        .method(case.method.clone())
        .uri(&case.uri)
        .header("x-test-user", user);
    let request = match &case.payload {
        Payload::None => builder.body(Body::empty()),
        Payload::Json(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        Payload::Form(f) => builder
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(f.clone())),
        Payload::Multipart(fields) => {
            let mut body = String::new();
            for (name, value) in fields {
                let file = if *name == "file" { "; filename=\"a.ics\"" } else { "" };
                body.push_str(&format!(
                    "--XBOUNDARY\r\nContent-Disposition: form-data; name=\"{name}\"{file}\r\n\r\n{value}\r\n"
                ));
            }
            body.push_str("--XBOUNDARY--\r\n");
            builder
                .header("content-type", "multipart/form-data; boundary=XBOUNDARY")
                .body(Body::from(body))
        }
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// What the spec says a case's URI is: its method and path template.
fn route_of(case: &Case, templates: &[(String, String)]) -> Option<(String, String)> {
    let path = case
        .uri
        .split('?')
        .next()
        .unwrap()
        .strip_prefix("/api")
        .unwrap();
    templates
        .iter()
        .find(|(m, t)| *m == case.method.as_str() && t == path)
        .cloned()
}

fn spec_routes() -> Vec<(String, String)> {
    let spec = serde_json::to_value(crate::api::spec()).unwrap();
    let mut routes = Vec::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for method in item.as_object().unwrap().keys() {
            if ["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                routes.push((method.to_uppercase(), path.clone()));
            }
        }
    }
    routes
}

#[tokio::test]
async fn no_user_reaches_another_users_data() {
    let state = state().await;
    seed(&state).await;
    let app = router(&state);
    let before = snapshot(&state).await;
    assert!(before.len() > 20, "the seed is Alice's data: {before:#?}");

    // Alice herself sees her data: the cases name real ids
    for uri in [
        format!("/api/email/message?{MSG}"),
        format!("/api/email/messages?account_id={A_MAIL}&mailbox=INBOX"),
    ] {
        let (status, body) = call(&app, ALICE, &case("GET", uri.clone(), Payload::None)).await;
        assert!(
            status.is_success() && body.contains(SECRET),
            "Alice's own {uri}: {status} {body}"
        );
    }

    let mut failures = Vec::new();
    for case in cases() {
        let (status, body) = call(&app, MALLORY, &case).await;
        let what = format!("{} {} → {status}", case.method, case.uri);
        if matches!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY | StatusCode::UNSUPPORTED_MEDIA_TYPE
        ) {
            // a body the handler never saw tests nothing
            failures.push(format!(
                "{what}: the request does not match the route ({body})"
            ));
        }
        if body.to_uppercase().contains(SECRET) {
            failures.push(format!("{what}: Alice's data in the answer: {body}"));
        }
        let now = snapshot(&state).await;
        if now != before {
            let gone: Vec<_> = before.difference(&now).collect();
            let new: Vec<_> = now.difference(&before).collect();
            failures.push(format!(
                "{what}: Alice's data changed; before {gone:#?}, now {new:#?}"
            ));
            break;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));

    // last: Mallory deletes her own account, and only hers
    let (status, _) = call(
        &app,
        MALLORY,
        &case("DELETE", "/api/account", Payload::None),
    )
    .await;
    assert!(status.is_success(), "{status}");
    assert_eq!(
        snapshot(&state).await,
        before,
        "deleting Mallory changed Alice's data"
    );
}

#[test]
fn every_route_is_covered() {
    let routes = spec_routes();
    let covered: BTreeSet<(String, String)> = cases()
        .iter()
        .filter_map(|c| route_of(c, &routes))
        .collect();
    for c in cases() {
        assert!(
            route_of(&c, &routes).is_some(),
            "{} {} is no route of the spec",
            c.method,
            c.uri
        );
    }
    let exempt: BTreeSet<(String, String)> = NO_FOREIGN_IDS
        .iter()
        .map(|(m, p, _)| (m.to_string(), p.to_string()))
        .collect();
    let missing: Vec<String> = routes
        .iter()
        .filter(|r| !covered.contains(*r) && !exempt.contains(*r))
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    assert!(
        missing.is_empty(),
        "routes without a cross-user case: add one to cases() in cross_user_tests.rs (Mallory naming Alice's ids), \
         or, if the route names nothing a user owns, list it in NO_FOREIGN_IDS with why:\n{}",
        missing.join("\n")
    );
    let stale: Vec<_> = exempt.iter().filter(|r| !routes.contains(r)).collect();
    assert!(
        stale.is_empty(),
        "NO_FOREIGN_IDS lists routes the spec does not have: {stale:?}"
    );
}

/// The unchecked id constructors are for background tasks only: a handler that used one would skip the ownership
/// check the types otherwise force.
#[test]
fn handlers_never_skip_the_check() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let handler_modules = [
        "email/mod.rs",
        "email/messages.rs",
        "email/attachments.rs",
        "email/send.rs",
        "calendar/mod.rs",
        "calendar/events.rs",
        "calendar/import.rs",
        "contacts.rs",
        "settings.rs",
        "pgp.rs",
        "notifications.rs",
        "local_auth.rs",
        "oidc.rs",
        "jmap/oauth.rs",
    ];
    for module in handler_modules {
        let source = std::fs::read_to_string(format!("{root}/{module}")).unwrap();
        // the module's own code, not its tests
        let code = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["MailAccountId::internal", "CalDavAccountId::internal"] {
            assert!(
                !code.contains(forbidden),
                "{module} uses {forbidden}: a handler gets the id from owned_account (the user's account) instead"
            );
        }
    }
}

// ── the database's own guard, below the handlers ─────────────────────────────

#[tokio::test]
async fn upserts_never_take_over_another_users_row() {
    let state = state().await;
    seed(&state).await;
    let pool = &state.db;
    let before = snapshot(&state).await;
    let imap = AccountProtocol {
        protocol: "imap",
        jmap_url: None,
        auth: "password",
        oauth_client_id: None,
        oauth_client_secret: None,
    };

    let e = email_db::upsert_account(
        pool,
        MALLORY,
        A_MAIL,
        "Mallory",
        "evil.example",
        993,
        "m",
        "pw",
        true,
        None,
        None,
        &imap,
        &state.encryption_key,
    )
    .await
    .unwrap_err();
    assert!(e.is::<email_db::NotOwned>(), "{e}");

    let e = db::upsert_caldav_account(
        pool,
        MALLORY,
        Some(A_CAL),
        None,
        &db::UserSettings {
            caldav_base_url: "https://evil.example/".into(),
            caldav_username: "m".into(),
            caldav_password: "pw".into(),
        },
        &state.encryption_key,
    )
    .await
    .unwrap_err();
    assert!(e.is::<email_db::NotOwned>(), "{e}");

    let e = email_db::upsert_identity(
        pool,
        A_IDENTITY,
        MailAccountId::internal(M_MAIL),
        MALLORY,
        "M",
        "m@x",
        true,
    )
    .await
    .unwrap_err();
    assert!(e.is::<email_db::NotOwned>(), "{e}");

    assert_eq!(snapshot(&state).await, before);
}

#[tokio::test]
async fn passkey_ids_are_never_taken_over() {
    let state = state().await;
    seed(&state).await;
    let before = snapshot(&state).await;
    // an authenticator may give any credential id, Alice's too
    let e =
        db::store_webauthn_credential(&state.db, A_CRED, MALLORY, "{\"mallory\":1}", None, true)
            .await
            .unwrap_err();
    assert!(e.is::<email_db::NotOwned>(), "{e}");
    db::update_webauthn_credential(&state.db, MALLORY, A_CRED, "{\"mallory\":1}", 9, 9)
        .await
        .unwrap();
    assert_eq!(snapshot(&state).await, before);
}

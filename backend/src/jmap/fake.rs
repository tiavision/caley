//! An in-memory JMAP server for the tests: the methods Caley calls, answered as RFC 8620/8621 say and shaped like
//! Stalwart's answers (a `/.well-known/jmap` redirect, relative URLs in the session, string ids, `s<n>` states).
//! Tests change its mail with the methods below (as other clients or delivered mail would) and read what Caley did.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response, Sse, sse::Event},
    routing::{get, post},
};
use serde_json::{Map, Value, json};
use tokio::sync::broadcast;

pub const ACCOUNT: &str = "acc1";

#[derive(Clone)]
pub struct FakeMailbox {
    pub name: String,
    pub parent_id: Option<String>,
    pub role: Option<String>,
    pub subscribed: bool,
}

#[derive(Clone)]
pub struct FakeEmail {
    pub blob_id: String,
    pub mailbox_ids: HashSet<String>,
    pub keywords: HashSet<String>,
    pub received_at: i64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Created,
    Updated,
    Destroyed,
}

#[derive(Default)]
pub struct Inner {
    pub mailboxes: BTreeMap<String, FakeMailbox>,
    pub emails: BTreeMap<String, FakeEmail>,
    pub blobs: HashMap<String, Vec<u8>>,
    /// (id, address)
    pub identities: Vec<(String, String)>,
    /// every Email change: (state after it, id, kind)
    log: Vec<(u64, String, Kind)>,
    email_state: u64,
    mailbox_state: u64,
    /// changes before this state can no longer be calculated
    oldest_state: u64,
    next_id: u64,
    pub submissions: Vec<Value>,
    /// the method names called, in order
    pub calls: Vec<String>,
    pub downloads: Vec<String>,
    /// what EmailSubmission/set refuses with (a SetError type), if anything
    pub refuse_submission: Option<String>,
    pub max_objects: usize,
    /// what the server accepts: `Basic <b64>` or `Bearer <token>`
    pub authorization: String,
    pub oauth: FakeOAuth,
}

/// The fake's OAuth server: one registered client, codes and tokens it gave.
#[derive(Default)]
pub struct FakeOAuth {
    pub client_id: String,
    /// `None`: a public client
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// code → (PKCE challenge, redirect URI, scope)
    codes: HashMap<String, (String, String, String)>,
    /// access tokens it accepts
    pub access_tokens: HashSet<String>,
    /// refresh tokens it accepts (each once: they rotate)
    pub refresh_tokens: HashSet<String>,
    /// the lifetime it gives access tokens
    pub expires_in: i64,
    /// the token requests (grant type, how the client authenticated: `basic` or `body`)
    pub token_requests: Vec<(String, String)>,
    /// the authorization requests' query parameters
    pub authorizations: Vec<HashMap<String, String>>,
}

impl Inner {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn changed(&mut self, id: &str, kind: Kind) {
        self.email_state += 1;
        self.log.push((self.email_state, id.to_owned(), kind));
    }

    fn email_json(&self, id: &str, e: &FakeEmail, properties: Option<&Vec<Value>>) -> Value {
        let raw = self.blobs.get(&e.blob_id).cloned().unwrap_or_default();
        let headers = header_fields(&raw);
        let all = json!({
            "id": id,
            "blobId": e.blob_id,
            "threadId": format!("T{id}"),
            "mailboxIds": e.mailbox_ids.iter().map(|m| (m.clone(), Value::Bool(true))).collect::<Map<_, _>>(),
            "keywords": e.keywords.iter().map(|k| (k.clone(), Value::Bool(true))).collect::<Map<_, _>>(),
            "size": raw.len(),
            "receivedAt": chrono::DateTime::from_timestamp(e.received_at, 0).unwrap().to_rfc3339(),
            "headers": headers.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect::<Vec<_>>(),
        });
        match properties {
            None => all,
            Some(props) => {
                let mut out = Map::new();
                for p in props.iter().filter_map(Value::as_str) {
                    if let Some(v) = all.get(p) {
                        out.insert(p.to_owned(), v.clone());
                    }
                }
                out.insert("id".into(), json!(id));
                Value::Object(out)
            }
        }
    }
}

/// The raw header fields of a message (name, value as in the message after the colon).
fn header_fields(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let head = text
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .replace("\r\n", "\n");
    let mut out: Vec<(String, String)> = Vec::new();
    for line in head.split('\n') {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push_str("\r\n");
                last.1.push_str(line);
            }
        } else if let Some((name, value)) = line.split_once(':') {
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    out
}

#[derive(Clone)]
pub struct FakeJmap {
    /// the URL to give Caley: the host (`http://127.0.0.1:port`), whose `/.well-known/jmap` redirects to the session
    pub url: String,
    pub inner: Arc<Mutex<Inner>>,
    push: broadcast::Sender<String>,
}

impl FakeJmap {
    /// A server with an Inbox, Sent, Drafts and Trash (Stalwart's names and roles), signed in with `user:secret`.
    pub async fn start() -> FakeJmap {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut inner = Inner {
            max_objects: 500,
            oauth: FakeOAuth {
                client_id: "caley".into(),
                client_secret: Some("client-secret".into()),
                redirect_uri: "http://localhost:3000/auth/jmap".into(),
                expires_in: 3600,
                ..Default::default()
            },
            authorization: format!("Basic {}", b64("user:secret")),
            ..Default::default()
        };
        for (id, name, role) in [
            ("a", "Inbox", "inbox"),
            ("b", "Deleted Items", "trash"),
            ("c", "Junk Mail", "junk"),
            ("d", "Drafts", "drafts"),
            ("e", "Sent Items", "sent"),
        ] {
            inner.mailboxes.insert(
                id.to_owned(),
                FakeMailbox {
                    name: name.to_owned(),
                    parent_id: None,
                    role: Some(role.to_owned()),
                    subscribed: true,
                },
            );
        }
        inner
            .identities
            .push(("i1".into(), "user@example.com".into()));
        let (push, _) = broadcast::channel(64);
        let fake = FakeJmap {
            url: String::new(),
            inner: Arc::new(Mutex::new(inner)),
            push,
        };
        let app = Router::new()
            .route(
                "/.well-known/jmap",
                get(|| async { Redirect::temporary("/jmap/session") }),
            )
            .route("/jmap/session", get(session))
            .route("/jmap/", post(api))
            .route("/jmap/download/{account}/{blob}/{name}", get(download))
            .route("/jmap/upload/{account}/", post(upload))
            .route("/jmap/eventsource/", get(eventsource))
            .route(
                "/.well-known/oauth-authorization-server",
                get(oauth_metadata),
            )
            .route("/authorize", get(oauth_authorize))
            .route("/token", post(oauth_token))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeJmap {
            url: format!("http://{addr}"),
            ..fake
        }
    }

    pub fn auth(&self) -> super::Auth {
        super::Auth::Basic {
            username: "user".into(),
            password: "secret".into(),
        }
    }

    /// Accept only this bearer token from now on.
    pub fn require_bearer(&self, token: &str) {
        self.inner.lock().unwrap().authorization = format!("Bearer {token}");
    }

    pub fn set_max_objects(&self, n: usize) {
        self.inner.lock().unwrap().max_objects = n;
    }

    pub fn mailbox_id(&self, role_or_name: &str) -> String {
        let inner = self.inner.lock().unwrap();
        inner
            .mailboxes
            .iter()
            .find(|(_, m)| m.role.as_deref() == Some(role_or_name) || m.name == role_or_name)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| panic!("no mailbox {role_or_name}"))
    }

    pub fn add_mailbox(&self, name: &str, parent: Option<&str>) -> String {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.id("m");
        inner.mailboxes.insert(
            id.clone(),
            FakeMailbox {
                name: name.to_owned(),
                parent_id: parent.map(str::to_owned),
                role: None,
                subscribed: true,
            },
        );
        inner.mailbox_state += 1;
        id
    }

    pub fn rename_mailbox(&self, id: &str, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.mailboxes.get_mut(id).unwrap().name = name.to_owned();
        inner.mailbox_state += 1;
    }

    /// Destroys a mailbox; its emails in no other mailbox are destroyed (onDestroyRemoveEmails).
    pub fn destroy_mailbox(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.mailboxes.remove(id);
        inner.mailbox_state += 1;
        let ids: Vec<String> = inner.emails.keys().cloned().collect();
        for eid in ids {
            let e = inner.emails.get_mut(&eid).unwrap();
            if e.mailbox_ids.remove(id) {
                if e.mailbox_ids.is_empty() {
                    inner.emails.remove(&eid);
                    inner.changed(&eid, Kind::Destroyed);
                } else {
                    inner.changed(&eid, Kind::Updated);
                }
            }
        }
    }

    /// A message arrives in mailboxes, with keywords.
    pub fn deliver(&self, mailboxes: &[&str], raw: &[u8], keywords: &[&str]) -> String {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.id("E");
        let blob_id = inner.id("B");
        inner.blobs.insert(blob_id.clone(), raw.to_vec());
        let received_at = 1_700_000_000 + inner.next_id as i64;
        inner.emails.insert(
            id.clone(),
            FakeEmail {
                blob_id,
                mailbox_ids: mailboxes.iter().map(|m| (*m).to_owned()).collect(),
                keywords: keywords.iter().map(|k| (*k).to_owned()).collect(),
                received_at,
            },
        );
        inner.changed(&id, Kind::Created);
        id
    }

    pub fn set_keyword(&self, email: &str, keyword: &str, set: bool) {
        let mut inner = self.inner.lock().unwrap();
        let e = inner.emails.get_mut(email).unwrap();
        if set {
            e.keywords.insert(keyword.to_owned());
        } else {
            e.keywords.remove(keyword);
        }
        inner.changed(email, Kind::Updated);
    }

    pub fn move_email(&self, email: &str, from: &str, to: &str) {
        let mut inner = self.inner.lock().unwrap();
        let e = inner.emails.get_mut(email).unwrap();
        e.mailbox_ids.remove(from);
        e.mailbox_ids.insert(to.to_owned());
        inner.changed(email, Kind::Updated);
    }

    pub fn destroy(&self, email: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.emails.remove(email);
        inner.changed(email, Kind::Destroyed);
    }

    /// The server forgets its change log: Email/changes from an older state is `cannotCalculateChanges`.
    pub fn forget_changes(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.oldest_state = inner.email_state;
    }

    pub fn email(&self, id: &str) -> Option<FakeEmail> {
        self.inner.lock().unwrap().emails.get(id).cloned()
    }

    /// Sends a StateChange to the push listeners.
    pub fn push_change(&self) {
        let state = format!("s{}", self.inner.lock().unwrap().email_state);
        let change = json!({
            "@type": "StateChange",
            "changed": { ACCOUNT: { "Email": state } },
        });
        let _ = self.push.send(change.to_string());
    }

    pub fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }

    pub fn downloads(&self) -> Vec<String> {
        self.inner.lock().unwrap().downloads.clone()
    }
}

fn b64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = s.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn authorized(fake: &FakeJmap, headers: &HeaderMap) -> bool {
    let inner = fake.inner.lock().unwrap();
    let Some(got) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    got == inner.authorization
        || got
            .strip_prefix("Bearer ")
            .is_some_and(|t| inner.oauth.access_tokens.contains(t))
}

/// The server's OAuth metadata (RFC 8414), its endpoints on its own origin.
async fn oauth_metadata(State(fake): State<FakeJmap>, headers: HeaderMap) -> Response {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let base = format!("http://{host}");
    let _ = fake;
    axum::Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

/// The sign-in page, signed in at once: checks the request and redirects back with a code (or `error`).
async fn oauth_authorize(
    State(fake): State<FakeJmap>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let mut inner = fake.inner.lock().unwrap();
    inner.oauth.authorizations.push(q.clone());
    let redirect = q.get("redirect_uri").cloned().unwrap_or_default();
    let state = q.get("state").cloned().unwrap_or_default();
    let back = |params: &[(&str, &str)]| {
        let mut url = url::Url::parse(&redirect).unwrap();
        url.query_pairs_mut()
            .extend_pairs(params)
            .append_pair("state", &state);
        Redirect::to(url.as_str()).into_response()
    };
    if q.get("client_id") != Some(&inner.oauth.client_id) || redirect != inner.oauth.redirect_uri {
        return (StatusCode::BAD_REQUEST, "unknown client or redirect URI").into_response();
    }
    if q.get("code_challenge_method").map(String::as_str) != Some("S256") {
        return back(&[
            ("error", "invalid_request"),
            ("error_description", "PKCE S256 required"),
        ]);
    }
    let code = inner.id("code");
    let challenge = q.get("code_challenge").cloned().unwrap_or_default();
    let scope = q.get("scope").cloned().unwrap_or_default();
    inner
        .oauth
        .codes
        .insert(code.clone(), (challenge, redirect.clone(), scope));
    back(&[("code", &code)])
}

fn token_error(error: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": error })),
    )
        .into_response()
}

/// The token endpoint: a code (its PKCE verifier checked) or a refresh token (rotated) for new tokens.
async fn oauth_token(State(fake): State<FakeJmap>, headers: HeaderMap, body: String) -> Response {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let form: HashMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    let mut inner = fake.inner.lock().unwrap();
    let basic = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .map(str::to_owned);
    let grant = form.get("grant_type").cloned().unwrap_or_default();
    // the client: Basic with its secret, or a public client naming itself
    let how = match (&basic, &inner.oauth.client_secret) {
        (Some(b), Some(secret)) if *b == b64(&format!("{}:{secret}", inner.oauth.client_id)) => {
            "basic"
        }
        (None, None) if form.get("client_id") == Some(&inner.oauth.client_id) => "body",
        _ => return token_error("invalid_client"),
    };
    inner
        .oauth
        .token_requests
        .push((grant.clone(), how.to_owned()));
    match grant.as_str() {
        "authorization_code" => {
            let code = form.get("code").cloned().unwrap_or_default();
            let Some((challenge, redirect, _)) = inner.oauth.codes.remove(&code) else {
                return token_error("invalid_grant");
            };
            let verifier = form.get("code_verifier").cloned().unwrap_or_default();
            let hashed = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            if hashed != challenge || form.get("redirect_uri") != Some(&redirect) {
                return token_error("invalid_grant");
            }
        }
        "refresh_token" => {
            let refresh = form.get("refresh_token").cloned().unwrap_or_default();
            if !inner.oauth.refresh_tokens.remove(&refresh) {
                return token_error("invalid_grant");
            }
        }
        _ => return token_error("unsupported_grant_type"),
    }
    let access = inner.id("at");
    let refresh = inner.id("rt");
    inner.oauth.access_tokens.insert(access.clone());
    inner.oauth.refresh_tokens.insert(refresh.clone());
    axum::Json(json!({
        "access_token": access, "token_type": "bearer", "refresh_token": refresh,
        "expires_in": inner.oauth.expires_in,
    }))
    .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [("www-authenticate", "Basic realm=\"Stalwart\"")],
        "",
    )
        .into_response()
}

async fn session(State(fake): State<FakeJmap>, headers: HeaderMap) -> Response {
    if !authorized(&fake, &headers) {
        return unauthorized();
    }
    let max = fake.inner.lock().unwrap().max_objects;
    axum::Json(json!({
        "capabilities": {
            super::CORE: { "maxObjectsInGet": max, "maxObjectsInSet": max, "maxCallsInRequest": 16 },
            super::MAIL: {},
            super::SUBMISSION: {},
        },
        "accounts": { ACCOUNT: { "name": "user@example.com", "isPersonal": true, "isReadOnly": false } },
        "primaryAccounts": { super::MAIL: ACCOUNT, super::SUBMISSION: ACCOUNT },
        "username": "user@example.com",
        "apiUrl": "/jmap/",
        "downloadUrl": "/jmap/download/{accountId}/{blobId}/{name}?accept={type}",
        "uploadUrl": "/jmap/upload/{accountId}/",
        "eventSourceUrl": "/jmap/eventsource/?types={types}&closeafter={closeafter}&ping={ping}",
        "state": "x",
    }))
    .into_response()
}

async fn download(
    State(fake): State<FakeJmap>,
    headers: HeaderMap,
    Path((_, blob, _)): Path<(String, String, String)>,
) -> Response {
    if !authorized(&fake, &headers) {
        return unauthorized();
    }
    let mut inner = fake.inner.lock().unwrap();
    inner.downloads.push(blob.clone());
    match inner.blobs.get(&blob) {
        Some(b) => (StatusCode::OK, b.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn upload(State(fake): State<FakeJmap>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&fake, &headers) {
        return unauthorized();
    }
    let mut inner = fake.inner.lock().unwrap();
    let id = inner.id("B");
    inner.blobs.insert(id.clone(), body.to_vec());
    axum::Json(
        json!({ "accountId": ACCOUNT, "blobId": id, "type": "message/rfc822", "size": body.len() }),
    )
    .into_response()
}

async fn eventsource(State(fake): State<FakeJmap>, headers: HeaderMap) -> Response {
    if !authorized(&fake, &headers) {
        return unauthorized();
    }
    let rx = fake.push.subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(data) => Some((
                Ok::<_, std::convert::Infallible>(Event::default().event("state").data(data)),
                rx,
            )),
            Err(_) => None,
        }
    });
    Sse::new(stream).into_response()
}

async fn api(State(fake): State<FakeJmap>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&fake, &headers) {
        return unauthorized();
    }
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "not JSON").into_response(),
    };
    let calls = req["methodCalls"].as_array().cloned().unwrap_or_default();
    let mut responses = Vec::new();
    let mut created_ids: HashMap<String, String> = HashMap::new();
    let mut inner = fake.inner.lock().unwrap();
    for call in calls {
        let name = call[0].as_str().unwrap_or_default().to_owned();
        let args = call[1].clone();
        let id = call[2].clone();
        inner.calls.push(name.clone());
        let (rname, rargs) = match handle(&mut inner, &name, &args, &mut created_ids) {
            Ok(r) => (name, r),
            Err(kind) => ("error".to_owned(), json!({ "type": kind })),
        };
        responses.push(json!([rname, rargs, id]));
    }
    axum::Json(json!({ "methodResponses": responses, "sessionState": "x" })).into_response()
}

fn handle(
    inner: &mut Inner,
    name: &str,
    args: &Value,
    created_ids: &mut HashMap<String, String>,
) -> Result<Value, &'static str> {
    if args["accountId"].as_str() != Some(ACCOUNT) {
        return Err("accountNotFound");
    }
    let state = format!("s{}", inner.email_state);
    match name {
        "Mailbox/get" => {
            let list: Vec<Value> = inner
                .mailboxes
                .iter()
                .map(|(id, m)| {
                    json!({
                        "id": id, "name": m.name, "parentId": m.parent_id, "role": m.role,
                        "isSubscribed": m.subscribed, "sortOrder": 0,
                    })
                })
                .collect();
            Ok(
                json!({ "accountId": ACCOUNT, "state": format!("m{}", inner.mailbox_state), "list": list, "notFound": [] }),
            )
        }
        "Email/get" => {
            let ids: Vec<String> = match &args["ids"] {
                Value::Null => inner.emails.keys().cloned().collect(),
                v => serde_json::from_value(v.clone()).unwrap_or_default(),
            };
            if ids.len() > inner.max_objects {
                return Err("requestTooLarge");
            }
            let props = args["properties"].as_array();
            let mut list = Vec::new();
            let mut not_found = Vec::new();
            for id in ids {
                match inner.emails.get(&id) {
                    Some(e) => list.push(inner.email_json(&id, e, props)),
                    None => not_found.push(id),
                }
            }
            Ok(json!({ "accountId": ACCOUNT, "state": state, "list": list, "notFound": not_found }))
        }
        "Email/query" => {
            let in_mailbox = args["filter"]["inMailbox"].as_str();
            let mut ids: Vec<(i64, String)> = inner
                .emails
                .iter()
                .filter(|(_, e)| in_mailbox.is_none_or(|m| e.mailbox_ids.contains(m)))
                .map(|(id, e)| (e.received_at, id.clone()))
                .collect();
            ids.sort_by(|a, b| b.cmp(a));
            let total = ids.len();
            let position = args["position"].as_u64().unwrap_or(0) as usize;
            let limit =
                (args["limit"].as_u64().unwrap_or(u64::MAX) as usize).min(inner.max_objects);
            let page: Vec<String> = ids
                .into_iter()
                .skip(position)
                .take(limit)
                .map(|(_, id)| id)
                .collect();
            Ok(json!({
                "accountId": ACCOUNT, "queryState": state, "canCalculateChanges": false,
                "position": position, "ids": page, "total": total,
            }))
        }
        "Email/changes" => {
            let since: u64 = args["sinceState"]
                .as_str()
                .and_then(|s| s.strip_prefix('s'))
                .and_then(|s| s.parse().ok())
                // Stalwart's answer to a state that is not one of its own
                .ok_or("invalidArguments")?;
            if since < inner.oldest_state || since > inner.email_state {
                return Err("cannotCalculateChanges");
            }
            let max = args["maxChanges"].as_u64().unwrap_or(u64::MAX).max(1);
            let upto = (since + max).min(inner.email_state);
            let mut first: HashMap<&str, Kind> = HashMap::new();
            let mut last: HashMap<&str, Kind> = HashMap::new();
            let mut order: Vec<&str> = Vec::new();
            for (s, id, kind) in &inner.log {
                if *s > since && *s <= upto {
                    if !first.contains_key(id.as_str()) {
                        order.push(id);
                    }
                    first.entry(id).or_insert(*kind);
                    last.insert(id, *kind);
                }
            }
            let (mut created, mut updated, mut destroyed) = (Vec::new(), Vec::new(), Vec::new());
            for id in order {
                match (first[id], last[id]) {
                    (Kind::Created, Kind::Destroyed) => {}
                    (Kind::Created, _) => created.push(id),
                    (_, Kind::Destroyed) => destroyed.push(id),
                    _ => updated.push(id),
                }
            }
            Ok(json!({
                "accountId": ACCOUNT, "oldState": format!("s{since}"), "newState": format!("s{upto}"),
                "hasMoreChanges": upto < inner.email_state,
                "created": created, "updated": updated, "destroyed": destroyed,
            }))
        }
        "Email/set" => {
            let mut updated = Map::new();
            let mut not_updated = Map::new();
            if let Some(update) = args["update"].as_object() {
                if update.len() > inner.max_objects {
                    return Err("requestTooLarge");
                }
                for (id, patch) in update {
                    let Some(e) = inner.emails.get_mut(id) else {
                        not_updated.insert(id.clone(), json!({ "type": "notFound" }));
                        continue;
                    };
                    for (key, value) in patch.as_object().unwrap() {
                        let (prop, item) = key.split_once('/').unwrap();
                        let item = item.replace("~1", "/").replace("~0", "~");
                        let set = value.as_bool() == Some(true);
                        let target = match prop {
                            "keywords" => &mut e.keywords,
                            "mailboxIds" => &mut e.mailbox_ids,
                            _ => return Err("invalidArguments"),
                        };
                        if set {
                            target.insert(item);
                        } else {
                            target.remove(&item);
                        }
                    }
                    if e.mailbox_ids.is_empty() {
                        not_updated.insert(id.clone(), json!({ "type": "invalidProperties" }));
                        continue;
                    }
                    updated.insert(id.clone(), Value::Null);
                    inner.changed(id, Kind::Updated);
                }
            }
            let mut destroyed = Vec::new();
            let mut not_destroyed = Map::new();
            if let Some(ids) = args["destroy"].as_array() {
                for id in ids.iter().filter_map(Value::as_str) {
                    if inner.emails.remove(id).is_some() {
                        inner.changed(id, Kind::Destroyed);
                        destroyed.push(id.to_owned());
                    } else {
                        not_destroyed.insert(id.to_owned(), json!({ "type": "notFound" }));
                    }
                }
            }
            Ok(json!({
                "accountId": ACCOUNT, "oldState": state, "newState": format!("s{}", inner.email_state),
                "updated": updated, "notUpdated": not_updated, "destroyed": destroyed, "notDestroyed": not_destroyed,
            }))
        }
        "Email/import" => {
            let mut created = Map::new();
            let mut not_created = Map::new();
            for (cid, e) in args["emails"].as_object().cloned().unwrap_or_default() {
                let blob = e["blobId"].as_str().unwrap_or_default().to_owned();
                if !inner.blobs.contains_key(&blob) {
                    not_created.insert(cid, json!({ "type": "blobNotFound" }));
                    continue;
                }
                let mailboxes: HashSet<String> = e["mailboxIds"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                if mailboxes.is_empty()
                    || mailboxes.iter().any(|m| !inner.mailboxes.contains_key(m))
                {
                    not_created.insert(cid, json!({ "type": "invalidProperties" }));
                    continue;
                }
                let id = inner.id("E");
                let received_at = 1_700_000_000 + inner.next_id as i64;
                inner.emails.insert(
                    id.clone(),
                    FakeEmail {
                        blob_id: blob.clone(),
                        mailbox_ids: mailboxes,
                        keywords: e["keywords"]
                            .as_object()
                            .map(|m| m.keys().cloned().collect())
                            .unwrap_or_default(),
                        received_at,
                    },
                );
                inner.changed(&id, Kind::Created);
                created_ids.insert(cid.clone(), id.clone());
                created.insert(
                    cid,
                    json!({ "id": id, "blobId": blob, "threadId": format!("T{id}") }),
                );
            }
            Ok(json!({
                "accountId": ACCOUNT, "oldState": state, "newState": format!("s{}", inner.email_state),
                "created": created, "notCreated": not_created,
            }))
        }
        "Identity/get" => {
            let list: Vec<Value> = inner
                .identities
                .iter()
                .map(|(id, email)| json!({ "id": id, "name": "", "email": email, "mayDelete": true }))
                .collect();
            Ok(json!({ "accountId": ACCOUNT, "state": "i0", "list": list, "notFound": [] }))
        }
        "EmailSubmission/set" => {
            let mut created = Map::new();
            let mut not_created = Map::new();
            for (cid, sub) in args["create"].as_object().cloned().unwrap_or_default() {
                let identity = sub["identityId"].as_str().unwrap_or_default();
                let email_id = sub["emailId"].as_str().unwrap_or_default();
                let email_id = match email_id.strip_prefix('#') {
                    Some(r) => created_ids.get(r).cloned().unwrap_or_default(),
                    None => email_id.to_owned(),
                };
                if let Some(kind) = inner.refuse_submission.clone() {
                    not_created.insert(
                        cid,
                        json!({ "type": kind, "description": "refused by the test" }),
                    );
                    continue;
                }
                if !inner.identities.iter().any(|(id, _)| id == identity) {
                    not_created.insert(cid, json!({ "type": "forbiddenFrom" }));
                    continue;
                }
                if !inner.emails.contains_key(&email_id) {
                    not_created.insert(cid, json!({ "type": "invalidProperties" }));
                    continue;
                }
                let mut recorded = sub.clone();
                recorded["emailId"] = json!(email_id);
                inner.submissions.push(recorded);
                let id = inner.id("S");
                created.insert(cid, json!({ "id": id }));
            }
            Ok(json!({ "accountId": ACCOUNT, "created": created, "notCreated": not_created }))
        }
        _ => Err("unknownMethod"),
    }
}

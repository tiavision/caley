//! JMAP accounts that sign in at the mail server itself (`auth = 'oauth'`): the authorization code flow with PKCE
//! (RFC 6749 §4.1, RFC 7636) against the server's OAuth endpoints (RFC 8414 metadata on the JMAP server's origin),
//! with the OAuth client the server has for Caley (its id and secret, entered in the account dialog). This is apart
//! from Caley's own sign-in (`oidc.rs`, e.g. PocketID): the server's tokens belong to the mail account.
//!
//! 1. Saving such an account (new, or on another server or client, or asked to) does not save it: it keeps what
//!    is to be saved in the session (`Pending`, under its `state`) and answers a link, `/auth/jmap?start=<state>`,
//!    which redirects to the server's authorization endpoint.
//! 2. The server sends the browser back to `/auth/jmap?code=…&state=…` (the redirect URI to register at the server:
//!    `{origin}/auth/jmap`). The code is exchanged for tokens, the JMAP session fetched with them, and only then the
//!    account saved and its tokens kept (`jmap_oauth_tokens`, encrypted); the browser goes to the mail page, or to it
//!    with `?account_error=` when something failed.
//! 3. The sync uses the access token and renews it with the refresh token (`offline_access`); a refused renewal is
//!    `SignInAgain`, and editing the account with "sign in again" runs the flow anew.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, anyhow};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tower_sessions::Session;

use crate::crypto;
use crate::email_db::{self, ImapAccountRow};
use crate::oidc::AppState;
use crate::oidc_tokens::{RenewError, TokenResponse, now, token_request};

/// The scopes asked for: mail (draft-ietf-mailmaint-oauth-public; Stalwart lists no mail account without it) and a
/// refresh token.
pub const SCOPES: &str = "offline_access urn:ietf:params:oauth:scope:mail";

/// A token this close to its expiry is renewed before use.
const MARGIN_SECS: i64 = 60;
/// How long an authorization may take.
const PENDING_SECS: i64 = 30 * 60;

/// The server's OAuth endpoints.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Endpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

/// The OAuth endpoints of the JMAP server at `session_url`: the authorization server metadata (RFC 8414), else its
/// OpenID configuration, on the origin of the session's `apiUrl` first (the session may be served by another host
/// than the server: a domain's `/.well-known/jmap` handed on to the mail server, whose API is on its own host), then
/// on the session URL's origins (after its redirects, and as given). An answer that is not metadata (a web server
/// answering every path with an empty 200) is passed over.
pub async fn discover(http: &reqwest::Client, session_url: &url::Url) -> Result<Endpoints> {
    let mut origins: Vec<String> = Vec::new();
    if let Ok((api, final_url)) = anonymous_session(http, session_url).await {
        origins.extend(api);
        origins.push(final_url.origin().ascii_serialization());
    }
    origins.push(session_url.origin().ascii_serialization());
    origins.dedup();
    let mut tried = Vec::new();
    for origin in origins.iter().filter(|o| *o != "null") {
        if tried.contains(origin) {
            continue;
        }
        tried.push(origin.clone());
        for path in [
            "/.well-known/oauth-authorization-server",
            "/.well-known/openid-configuration",
        ] {
            let url = super::rewrite(format!("{origin}{path}"));
            let Ok(r) = http
                .get(&url)
                .header("accept", "application/json")
                .timeout(Duration::from_secs(10))
                .send()
                .await
            else {
                continue;
            };
            if r.status().is_success()
                && let Ok(e) = r.json_bytes().await
            {
                return Ok(e);
            }
        }
    }
    Err(anyhow!(
        "the server offers no OAuth sign-in (no OAuth metadata at {})",
        tried.join(", ")
    ))
}

/// The session as the server answers it without credentials (Stalwart: its URLs, no account): the origin of its
/// `apiUrl` if it says one, and the URL it was answered from.
async fn anonymous_session(
    http: &reqwest::Client,
    session_url: &url::Url,
) -> Result<(Option<String>, url::Url)> {
    let r = http
        .get(super::rewrite(session_url.to_string()))
        .header("accept", "application/json")
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let final_url = r.url().clone();
    let body: serde_json::Value = serde_json::from_slice(&r.bytes().await?).unwrap_or_default();
    let api = body
        .get("apiUrl")
        .and_then(serde_json::Value::as_str)
        .and_then(|a| final_url.join(a).ok())
        .map(|u| u.origin().ascii_serialization());
    Ok((api, final_url))
}

trait JsonBytes {
    async fn json_bytes(self) -> Result<Endpoints>;
}

impl JsonBytes for reqwest::Response {
    async fn json_bytes(self) -> Result<Endpoints> {
        let bytes = self.bytes().await?;
        let e: Endpoints =
            serde_json::from_slice(&bytes).map_err(|e| anyhow!("not OAuth metadata: {e}"))?;
        if e.authorization_endpoint.is_empty() || e.token_endpoint.is_empty() {
            return Err(anyhow!("OAuth metadata without endpoints"));
        }
        Ok(e)
    }
}

/// An account waiting for its authorization, kept in the session under its `state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pending {
    pub account_id: String,
    pub is_new: bool,
    pub display_name: String,
    pub sender_name: Option<String>,
    /// the JMAP session URL
    pub jmap_url: String,
    pub username: String,
    pub client_id: String,
    /// the client secret, encrypted
    pub client_secret: Option<String>,
    pub endpoints: Endpoints,
    pub redirect_uri: String,
    pub verifier: String,
    pub created_at: i64,
}

fn pending_key(state: &str) -> String {
    format!("jmap_oauth:{state}")
}

/// 32 random bytes, base64url (43 characters: a PKCE verifier's length range, and an unguessable state).
fn random_token() -> String {
    use base64::Engine;
    let buf: [u8; 32] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl Pending {
    /// The authorization request (RFC 6749 §4.1.1) with its PKCE challenge, `login_hint` the username.
    pub fn authorization_url(&self, state: &str) -> Result<url::Url> {
        let mut url = url::Url::parse(&self.endpoints.authorization_endpoint)
            .map_err(|e| anyhow!("invalid authorization endpoint: {e}"))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", SCOPES)
            .append_pair("state", state)
            .append_pair("code_challenge", &challenge(&self.verifier))
            .append_pair("code_challenge_method", "S256");
        if !self.username.is_empty() {
            url.query_pairs_mut()
                .append_pair("login_hint", &self.username);
        }
        Ok(url)
    }
}

/// Keeps an account to authorize in the session; the link that starts the authorization.
pub async fn begin(session: &Session, mut pending: Pending) -> Result<String> {
    let state = random_token();
    pending.verifier = random_token();
    pending.created_at = now();
    session.insert(&pending_key(&state), &pending).await?;
    Ok(format!("/auth/jmap?start={state}"))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    start: Option<String>,
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /auth/jmap`: `?start=` redirects to the server's authorization endpoint, the server's answer (`code` and
/// `state`, or `error`) finishes the authorization; then the mail page.
pub async fn callback(
    State(app): State<AppState>,
    session: Session,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(user_id) = session.get::<String>("user_id").await.ok().flatten() else {
        return Redirect::to("/").into_response();
    };
    if let Some(state) = q.start {
        return match session
            .get::<Pending>(&pending_key(&state))
            .await
            .ok()
            .flatten()
        {
            Some(p) => match p.authorization_url(&state) {
                Ok(url) => Redirect::to(url.as_str()).into_response(),
                Err(e) => failed(&e.to_string()),
            },
            None => failed("the sign-in at the mail server has expired: add the account again"),
        };
    }
    let Some(state) = q.state else {
        return failed("the mail server answered without a state");
    };
    let pending = session
        .remove::<Pending>(&pending_key(&state))
        .await
        .ok()
        .flatten();
    let Some(pending) = pending.filter(|p| now() - p.created_at < PENDING_SECS) else {
        return failed("the sign-in at the mail server has expired: add the account again");
    };
    if let Some(error) = q.error {
        let detail = q.error_description.unwrap_or(error);
        return failed(&format!("the mail server refused the sign-in: {detail}"));
    }
    let Some(code) = q.code else {
        return failed("the mail server answered without a code");
    };
    match finish(&app, &user_id, &pending, &code).await {
        Ok(account_id) => {
            let to: String = url::form_urlencoded::Serializer::for_suffix("/mail/?".to_owned(), 7)
                .append_pair("account_id", &account_id)
                .append_pair("mailbox", "INBOX")
                .finish();
            Redirect::to(&to).into_response()
        }
        Err(e) => {
            eprintln!("[jmap] OAuth sign-in for {}: {e}", pending.account_id);
            failed(&e.to_string())
        }
    }
}

/// Back to the mail page, which shows the error.
fn failed(message: &str) -> Response {
    let to: String = url::form_urlencoded::Serializer::for_suffix("/mail/?".to_owned(), 7)
        .append_pair("account_error", message)
        .finish();
    Redirect::to(&to).into_response()
}

/// Exchanges the code, fetches the JMAP session with the token, saves the account and its tokens.
pub async fn finish(app: &AppState, user_id: &str, p: &Pending, code: &str) -> Result<String> {
    let mgr = &app.email_sync_manager;
    let secret = p
        .client_secret
        .as_deref()
        .map(|s| crypto::decrypt(s, &app.encryption_key))
        .transpose()?;
    let tokens = token_request(
        &mgr.http,
        &super::rewrite(p.endpoints.token_endpoint.clone()),
        &p.client_id,
        secret.as_deref(),
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &p.redirect_uri),
            ("code_verifier", &p.verifier),
        ],
    )
    .await
    .map_err(|e| match e {
        RenewError::Rejected(detail) => anyhow!("the mail server refused the code: {detail}"),
        RenewError::Failed(e) => e,
    })?;
    let client = super::Client::connect(
        &mgr.http,
        &p.jmap_url,
        super::Auth::Bearer(tokens.access_token.clone()),
    )
    .await?;

    let url = url::Url::parse(&p.jmap_url)?;
    let username = if p.username.is_empty() {
        client.session.username.clone().unwrap_or_default()
    } else {
        p.username.clone()
    };
    let existing =
        email_db::get_account(&app.db, &p.account_id, user_id, &app.encryption_key).await?;
    // held until the account is saved (as save_account does)
    let mut _no_sync = None;
    if let Some(existing) = &existing {
        // another server: what the cache holds is of the old one
        if existing.jmap_url.as_deref() != Some(p.jmap_url.as_str()) || !existing.is_jmap() {
            _no_sync = Some(mgr.jmap_lock(&p.account_id).await);
            mgr.stop_idle_for(&p.account_id);
            let _ = tokio::fs::remove_dir_all(crate::email_sync::account_dir(
                &app.data_dir,
                existing.id(),
            ))
            .await;
            email_db::reset_account_cache(&app.db, existing.id()).await?;
        }
    } else if !p.is_new {
        return Err(anyhow!("the account was removed meanwhile"));
    }
    email_db::upsert_account(
        &app.db,
        user_id,
        &p.account_id,
        &p.display_name,
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or(443),
        &username,
        "",
        url.scheme() == "https",
        None,
        None,
        &email_db::AccountProtocol {
            protocol: "jmap",
            jmap_url: Some(&p.jmap_url),
            auth: "oauth",
            oauth_client_id: Some(&p.client_id),
            oauth_client_secret: secret.as_deref(),
        },
        &app.encryption_key,
    )
    .await?;
    store_tokens(
        &app.db,
        &app.encryption_key,
        &p.account_id,
        &p.endpoints.token_endpoint,
        &tokens,
        None,
    )
    .await?;
    if p.is_new
        && let Some(name) = p.sender_name.as_deref().filter(|n| !n.is_empty())
        && let Some(account) =
            email_db::get_account(&app.db, &p.account_id, user_id, &app.encryption_key).await?
    {
        email_db::upsert_identity(
            &app.db,
            &uuid::Uuid::new_v4().to_string(),
            account.id(),
            user_id,
            name,
            &username,
            true,
        )
        .await?;
    }
    mgr.stop_idle_for(&p.account_id);
    mgr.ensure_task_for(&p.account_id);
    app.sse_broadcaster
        .send(user_id, crate::sse::SseMessage::AccountsChanged);
    Ok(p.account_id.clone())
}

/// Keeps an account's tokens (a renewal without a new refresh token keeps the old one, `keep_refresh`).
pub async fn store_tokens(
    db: &SqlitePool,
    key: &[u8; 32],
    account_id: &str,
    token_url: &str,
    t: &TokenResponse,
    keep_refresh: Option<&str>,
) -> Result<()> {
    let refresh = t.refresh_token.as_deref().or(keep_refresh);
    sqlx::query(
        "INSERT INTO jmap_oauth_tokens (account_id, token_url, access_token, refresh_token, expires_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(account_id) DO UPDATE SET
            token_url = excluded.token_url, access_token = excluded.access_token,
            refresh_token = excluded.refresh_token, expires_at = excluded.expires_at",
    )
    .bind(account_id)
    .bind(token_url)
    .bind(crypto::encrypt(&t.access_token, key)?)
    .bind(refresh.map(|r| crypto::encrypt(r, key)).transpose()?)
    .bind(t.expires_in.map(|s| now() + s))
    .execute(db)
    .await?;
    Ok(())
}

struct Stored {
    token_url: String,
    access: String,
    refresh: Option<String>,
    expires_at: Option<i64>,
}

async fn load(db: &SqlitePool, key: &[u8; 32], account_id: &str) -> Result<Option<Stored>> {
    let row = sqlx::query(
        "SELECT token_url, access_token, refresh_token, expires_at FROM jmap_oauth_tokens WHERE account_id = ?",
    )
    .bind(account_id)
    .fetch_optional(db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    Ok(Some(Stored {
        token_url: r.get(0),
        access: crypto::decrypt(r.get::<String, _>(1).as_str(), key)?,
        refresh: r
            .get::<Option<String>, _>(2)
            .map(|s| crypto::decrypt(&s, key))
            .transpose()?,
        expires_at: r.get(3),
    }))
}

/// Whether an account has tokens (it has been authorized).
pub async fn has_tokens(db: &SqlitePool, account_id: &str) -> bool {
    sqlx::query("SELECT 1 FROM jmap_oauth_tokens WHERE account_id = ?")
        .bind(account_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// The account's sign-in at the server is gone: it is to be signed in again (the account dialog).
#[derive(Debug)]
pub struct SignInAgain;

impl std::fmt::Display for SignInAgain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the sign-in at the mail server has expired: edit the account and sign in again"
        )
    }
}

impl std::error::Error for SignInAgain {}

/// One renewal per account at a time (a server that rotates refresh tokens takes each once).
#[derive(Default)]
pub struct Locks(std::sync::Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>);

impl Locks {
    fn of(&self, account_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        std::sync::Arc::clone(
            self.0
                .lock()
                .unwrap()
                .entry(account_id.to_owned())
                .or_default(),
        )
    }
}

/// A valid access token of an account, renewed first when it has expired (or is about to).
pub async fn access_token(
    http: &reqwest::Client,
    db: &SqlitePool,
    key: &[u8; 32],
    locks: &Locks,
    account: &ImapAccountRow,
) -> Result<String> {
    let fresh = |s: &Stored| s.expires_at.is_none_or(|t| t - MARGIN_SECS > now());
    let id = account.account_id.as_str();
    match load(db, key, id).await? {
        Some(s) if fresh(&s) => return Ok(s.access),
        None => return Err(SignInAgain.into()),
        _ => {}
    }
    let lock = locks.of(id);
    let _one = lock.lock().await;
    let Some(s) = load(db, key, id).await? else {
        return Err(SignInAgain.into());
    };
    if fresh(&s) {
        return Ok(s.access);
    }
    let (Some(refresh), Some(client_id)) =
        (s.refresh.as_deref(), account.oauth_client_id.as_deref())
    else {
        return Err(SignInAgain.into());
    };
    match token_request(
        http,
        &super::rewrite(s.token_url.clone()),
        client_id,
        account.oauth_client_secret.as_deref(),
        &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
    )
    .await
    {
        Ok(t) => {
            store_tokens(db, key, id, &s.token_url, &t, Some(refresh)).await?;
            Ok(t.access_token)
        }
        Err(RenewError::Rejected(detail)) => {
            eprintln!("[jmap] {id}: the server refused to renew the sign-in: {detail}");
            Err(SignInAgain.into())
        }
        Err(RenewError::Failed(e)) => Err(e),
    }
}

//! The tokens of a user's OIDC sign-in, kept (encrypted) for work done for the user outside a request: a JMAP account
//! that signs in with the user's own OIDC login (`auth = 'oidc'`, e.g. a Stalwart server that trusts the same
//! identity provider, or is it) sends the access token as its bearer token. When the access token has expired, the
//! refresh token renews it at the provider's token endpoint (RFC 6749 §6); without one the user signs in again.

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use tokio::sync::Mutex;

use crate::crypto;

/// A token this close to its expiry is renewed before use.
const MARGIN_SECS: i64 = 60;

/// Where and how to renew tokens: the provider's token endpoint and the client's credentials.
pub struct Refresher {
    pub http: reqwest::Client,
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
}

pub struct OidcTokens {
    db: SqlitePool,
    key: [u8; 32],
    refresher: Option<Refresher>,
    /// one renewal at a time: a provider that rotates refresh tokens accepts each only once
    refreshing: Mutex<()>,
}

/// Why there is no access token for a user.
#[derive(Debug, PartialEq)]
pub enum TokenError {
    /// no OIDC sign-in kept (the demo user, or tokens dropped after a failed renewal)
    NoToken,
    /// expired, and it could not be renewed: the user signs in again
    Expired,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::NoToken => write!(f, "no OIDC sign-in to use: sign in with OIDC"),
            TokenError::Expired => write!(f, "the OIDC sign-in has expired: sign in again"),
        }
    }
}

impl std::error::Error for TokenError {}

/// What a token endpoint answers (RFC 6749 §5.1).
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

impl OidcTokens {
    pub fn new(db: SqlitePool, key: [u8; 32], refresher: Option<Refresher>) -> Self {
        Self {
            db,
            key,
            refresher,
            refreshing: Mutex::new(()),
        }
    }

    /// Keeps the tokens of a sign-in (replacing the last). A response without a refresh token keeps the stored one.
    pub async fn store(
        &self,
        user_id: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_in: Option<Duration>,
    ) -> Result<()> {
        let expires_at = expires_in.map(|d| now() + d.as_secs() as i64);
        let access = crypto::encrypt(access_token, &self.key)?;
        let refresh = refresh_token
            .map(|t| crypto::encrypt(t, &self.key))
            .transpose()?;
        sqlx::query(
            "INSERT INTO oidc_tokens (user_id, access_token, refresh_token, expires_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(user_id) DO UPDATE SET
                access_token  = excluded.access_token,
                refresh_token = COALESCE(excluded.refresh_token, oidc_tokens.refresh_token),
                expires_at    = excluded.expires_at",
        )
        .bind(user_id)
        .bind(access)
        .bind(refresh)
        .bind(expires_at)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    /// Whether a sign-in is kept for the user (an OIDC JMAP account can be set up).
    pub async fn has(&self, user_id: &str) -> bool {
        sqlx::query("SELECT 1 FROM oidc_tokens WHERE user_id = ?")
            .bind(user_id)
            .fetch_optional(&self.db)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// A valid access token for the user, renewed first when it has expired (or is about to).
    pub async fn access_token(&self, user_id: &str) -> Result<String> {
        if let Some(token) = self.fresh(user_id).await? {
            return Ok(token);
        }
        let _guard = self.refreshing.lock().await;
        // renewed by another caller while this one waited
        if let Some(token) = self.fresh(user_id).await? {
            return Ok(token);
        }
        let Some((_, refresh, _)) = self.load(user_id).await? else {
            return Err(TokenError::NoToken.into());
        };
        let (Some(refresh), Some(refresher)) = (refresh, &self.refresher) else {
            return Err(TokenError::Expired.into());
        };
        match refresher.renew(&refresh).await {
            Ok(r) => {
                self.store(
                    user_id,
                    &r.access_token,
                    r.refresh_token.as_deref(),
                    r.expires_in.map(|s| Duration::from_secs(s.max(0) as u64)),
                )
                .await?;
                Ok(r.access_token)
            }
            Err(RenewError::Rejected(detail)) => {
                eprintln!("[oidc] renewing the token of {user_id} was refused: {detail}");
                Err(TokenError::Expired.into())
            }
            Err(RenewError::Failed(e)) => Err(e),
        }
    }

    /// The stored access token if it has not expired.
    async fn fresh(&self, user_id: &str) -> Result<Option<String>> {
        Ok(match self.load(user_id).await? {
            Some((access, _, expires_at)) if expires_at.is_none_or(|t| t - MARGIN_SECS > now()) => {
                Some(access)
            }
            _ => None,
        })
    }

    async fn load(&self, user_id: &str) -> Result<Option<(String, Option<String>, Option<i64>)>> {
        let row = sqlx::query(
            "SELECT access_token, refresh_token, expires_at FROM oidc_tokens WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;
        let Some(r) = row else { return Ok(None) };
        let access = crypto::decrypt(r.get::<String, _>(0).as_str(), &self.key)?;
        let refresh = r
            .get::<Option<String>, _>(1)
            .map(|s| crypto::decrypt(&s, &self.key))
            .transpose()?;
        Ok(Some((access, refresh, r.get(2))))
    }
}

pub(crate) enum RenewError {
    /// the provider answered with an error (`invalid_grant`: the refresh token is no longer valid)
    Rejected(String),
    /// the provider could not be reached, or answered nonsense: tried again later
    Failed(anyhow::Error),
}

impl Refresher {
    async fn renew(&self, refresh_token: &str) -> Result<TokenResponse, RenewError> {
        token_request(
            &self.http,
            &self.token_url,
            &self.client_id,
            Some(&self.client_secret),
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
        )
        .await
    }
}

/// A token request (RFC 6749 §4.1.3 a code, §6 a refresh token): a confidential client authenticates with
/// client_secret_basic, a public one (no secret) names itself with `client_id` in the form.
pub(crate) async fn token_request(
    http: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    form: &[(&str, &str)],
) -> Result<TokenResponse, RenewError> {
    let secret = client_secret.filter(|s| !s.is_empty());
    // built before the request: the serializer is not Send
    let body = {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in form {
            body.append_pair(k, v);
        }
        if secret.is_none() {
            body.append_pair("client_id", client_id);
        }
        body.finish()
    };
    let mut req = http.post(token_url);
    if let Some(secret) = secret {
        req = req.basic_auth(form_encode(client_id), Some(form_encode(secret)));
    }
    let resp = req
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| RenewError::Failed(anyhow!("token endpoint: {e}")))?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| RenewError::Failed(e.into()))?;
    if status.is_client_error() {
        return Err(RenewError::Rejected(
            String::from_utf8_lossy(&bytes).into_owned(),
        ));
    }
    if !status.is_success() {
        return Err(RenewError::Failed(anyhow!(
            "token endpoint answered {status}"
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| RenewError::Failed(anyhow!("token endpoint answered no token: {e}")))
}

/// client_secret_basic encodes the id and secret form-urlencoded first (RFC 6749 §2.3.1).
fn form_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::State as AxState, http::HeaderMap, routing::post};
    use std::sync::{Arc, Mutex as StdMutex};

    async fn pool() -> SqlitePool {
        let dir = tempdir();
        let pool = crate::db::init_db(dir.join("t.db").to_str().unwrap())
            .await
            .unwrap();
        crate::db::ensure_user(&pool, "u", None).await.unwrap();
        pool
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("caley-oidc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[derive(Clone, Default)]
    struct Seen {
        bodies: Arc<StdMutex<Vec<(String, String)>>>,
    }

    /// A token endpoint: `good` renews (rotating the refresh token), anything else is invalid_grant.
    async fn token_endpoint() -> (String, Seen) {
        async fn token(
            AxState(seen): AxState<Seen>,
            headers: HeaderMap,
            body: String,
        ) -> axum::response::Response {
            use axum::response::IntoResponse;
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            seen.bodies.lock().unwrap().push((auth, body.clone()));
            if body.contains("refresh_token=good") {
                axum::Json(serde_json::json!({
                    "access_token": "renewed",
                    "refresh_token": "good2",
                    "expires_in": 3600,
                    "token_type": "Bearer",
                }))
                .into_response()
            } else {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({ "error": "invalid_grant" })),
                )
                    .into_response()
            }
        }
        let seen = Seen::default();
        let app = Router::new()
            .route("/token", post(token))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/token"), seen)
    }

    fn refresher(token_url: String) -> Refresher {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Refresher {
            http: crate::jmap::http_client().unwrap(),
            token_url,
            client_id: "caley".into(),
            client_secret: "s3cret".into(),
        }
    }

    #[tokio::test]
    async fn a_valid_token_is_used_as_it_is() {
        let tokens = OidcTokens::new(pool().await, [7; 32], None);
        tokens
            .store("u", "at", Some("rt"), Some(Duration::from_secs(3600)))
            .await
            .unwrap();
        assert_eq!(tokens.access_token("u").await.unwrap(), "at");
        assert!(tokens.has("u").await);
    }

    #[tokio::test]
    async fn a_token_without_expiry_never_expires() {
        let tokens = OidcTokens::new(pool().await, [7; 32], None);
        tokens.store("u", "at", None, None).await.unwrap();
        assert_eq!(tokens.access_token("u").await.unwrap(), "at");
    }

    #[tokio::test]
    async fn no_sign_in_is_no_token() {
        let tokens = OidcTokens::new(pool().await, [7; 32], None);
        let e = tokens.access_token("u").await.unwrap_err();
        assert_eq!(e.downcast_ref::<TokenError>(), Some(&TokenError::NoToken));
        assert!(!tokens.has("u").await);
    }

    #[tokio::test]
    async fn an_expired_token_is_renewed_and_the_new_refresh_token_kept() {
        let (url, seen) = token_endpoint().await;
        let tokens = OidcTokens::new(pool().await, [7; 32], Some(refresher(url)));
        tokens
            .store("u", "old", Some("good"), Some(Duration::from_secs(10)))
            .await
            .unwrap();
        assert_eq!(tokens.access_token("u").await.unwrap(), "renewed");
        // the renewed one is stored: no second request
        assert_eq!(tokens.access_token("u").await.unwrap(), "renewed");
        let bodies = seen.bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].1.contains("grant_type=refresh_token"));
        assert!(bodies[0].0.starts_with("Basic "), "client_secret_basic");
        let (_, refresh, expires) = tokens.load("u").await.unwrap().unwrap();
        assert_eq!(refresh.as_deref(), Some("good2"));
        assert!(expires.unwrap() > now() + 3000);
    }

    #[tokio::test]
    async fn a_refused_renewal_says_sign_in_again() {
        let (url, _) = token_endpoint().await;
        let tokens = OidcTokens::new(pool().await, [7; 32], Some(refresher(url)));
        tokens
            .store("u", "old", Some("revoked"), Some(Duration::from_secs(0)))
            .await
            .unwrap();
        let e = tokens.access_token("u").await.unwrap_err();
        assert_eq!(e.downcast_ref::<TokenError>(), Some(&TokenError::Expired));
    }

    #[tokio::test]
    async fn an_expired_token_without_refresh_token_says_sign_in_again() {
        let (url, _) = token_endpoint().await;
        let tokens = OidcTokens::new(pool().await, [7; 32], Some(refresher(url)));
        tokens
            .store("u", "old", None, Some(Duration::from_secs(0)))
            .await
            .unwrap();
        let e = tokens.access_token("u").await.unwrap_err();
        assert_eq!(e.downcast_ref::<TokenError>(), Some(&TokenError::Expired));
    }

    #[tokio::test]
    async fn a_new_sign_in_without_refresh_token_keeps_the_old_one() {
        let tokens = OidcTokens::new(pool().await, [7; 32], None);
        tokens.store("u", "a", Some("r1"), None).await.unwrap();
        tokens.store("u", "b", None, None).await.unwrap();
        let (access, refresh, _) = tokens.load("u").await.unwrap().unwrap();
        assert_eq!((access.as_str(), refresh.as_deref()), ("b", Some("r1")));
    }

    #[tokio::test]
    async fn tokens_are_encrypted_at_rest() {
        let pool = pool().await;
        let tokens = OidcTokens::new(pool.clone(), [7; 32], None);
        tokens
            .store("u", "plain-access", Some("plain-refresh"), None)
            .await
            .unwrap();
        let row = sqlx::query("SELECT access_token, refresh_token FROM oidc_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        let (a, r): (String, String) = (row.get(0), row.get(1));
        assert!(!a.contains("plain") && !r.contains("plain"));
    }
}

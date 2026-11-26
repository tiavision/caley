//! Notifications to the user's devices while no page of theirs is open: Web Push (RFC 8030) to the
//! endpoints their devices registered, the payload encrypted for the device (RFC 8291, `aes128gcm`) and
//! signed by this server (VAPID, RFC 8292). The app on Android (native/) registers one per server
//! through UnifiedPush, whose distributor (e.g. ntfy) delivers it; any Web Push endpoint works.
//!
//! What is sent: every `NewEmail` the SSE broadcaster sends (sse.rs hands it over), calendar reminders
//! (reminders.rs), and a test (`POST /api/notifications/test`). An endpoint that answers 404 or 410 is gone and forgotten.
//! Not under `/api/push`: that is the CalDAV servers' push to Caley (sync.rs).
//!
//! The endpoints are URLs a signed-in user gives, so the server only posts to public addresses: an endpoint
//! whose host is (or resolves only to) a loopback, private, link-local, shared or other non-public address
//! is refused when subscribed, and sending resolves through `PublicOnly` (public_net.rs), so a name that
//! later resolves to such an address (DNS rebinding) gets nothing; redirects are not followed.
//! `NOTIFICATIONS_ALLOW_PRIVATE` allows them, for a push service (e.g. ntfy) on the server's own network.

use std::sync::Arc;
use std::time::Duration;

use aes_gcm::{Aes128Gcm, KeyInit, Nonce, aead::Aead};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::Row;
use tokio::sync::OnceCell;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::ApiError;
use crate::db::Db;
use crate::oidc::{AppState, CurrentUser};
use crate::public_net::{PublicOnly, is_public, literal_host_is_public, literal_ip};
use crate::sse::SseMessage;

/// How long a push service keeps a notification for a device that is offline.
const TTL_SECONDS: u64 = 24 * 60 * 60;
/// The most a notification's JSON may be: 4096 bytes on the wire (UnifiedPush) less the aes128gcm
/// header (86) and tag (16) and the padding delimiter (1).
const MAX_PAYLOAD: usize = 3993;

/// Sends the user's notifications (see the module); shared by the SSE broadcaster and the handlers.
pub struct Notifier {
    db: Db,
    key: [u8; 32],
    /// `sub` of the VAPID claims: the server's public https origin, if it has one
    contact: Option<String>,
    http: reqwest::Client,
    vapid: OnceCell<SigningKey>,
    /// endpoints on non-public addresses allowed (`NOTIFICATIONS_ALLOW_PRIVATE`)
    allow_private: bool,
}

/// What a device gets (JSON, encrypted for it): the app shows it as a notification.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Payload {
    NewEmail {
        account_id: String,
        mailbox: String,
        uid: u32,
        from: Option<String>,
        subject: Option<String>,
        /// the message on the server, without a language prefix (the server adds the visitor's)
        url: String,
    },
    /// a calendar alarm (reminders.rs)
    Reminder {
        summary: String,
        location: String,
        /// the occurrence's start and end, epoch seconds (in the device's timezone for floating times)
        start: Option<i64>,
        end: Option<i64>,
        all_day: bool,
        /// the occurrence's date (`YYYY-MM-DD`)
        date: String,
        /// the notification's tag, the one the calendar page gives its own (event-reminders.ts)
        tag: String,
        url: String,
    },
    Test,
}

impl Notifier {
    pub fn new(
        db: Db,
        key: [u8; 32],
        app_base_url: &str,
        allow_private: bool,
    ) -> anyhow::Result<Self> {
        let mut http = reqwest::Client::builder()
            .use_preconfigured_tls(crate::imap_client::tls_config())
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none());
        if !allow_private {
            http = http.dns_resolver(Arc::new(PublicOnly));
        }
        let http = http.build()?;
        let contact = app_base_url
            .starts_with("https://")
            .then(|| app_base_url.trim_end_matches('/').to_owned());
        Ok(Self {
            db,
            key,
            contact,
            http,
            vapid: OnceCell::new(),
            allow_private,
        })
    }

    /// A message the SSE broadcaster sends: new mail goes to the user's devices too.
    pub fn broadcast(self: &Arc<Self>, user_id: &str, message: &SseMessage) {
        if let SseMessage::NewEmail {
            account_id,
            uid,
            mailbox,
            from_addr,
            subject,
        } = message
        {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("account_id", account_id)
                .append_pair("mailbox", mailbox)
                .append_pair("uid", &uid.to_string())
                .finish();
            let url = format!("/mail/view/?{query}");
            let payload = Payload::NewEmail {
                account_id: account_id.clone(),
                mailbox: mailbox.clone(),
                uid: *uid,
                from: from_addr.as_deref().map(|s| truncate(s, 300)),
                subject: subject.as_deref().map(|s| truncate(s, 300)),
                url,
            };
            let this = Arc::clone(self);
            let user_id = user_id.to_owned();
            tokio::spawn(async move {
                this.send(&user_id, None, &payload).await;
            });
        }
    }

    /// Sends `payload` to each of the user's devices (those in `timezone`, if given; a device that gave
    /// none counts as UTC); returns to how many it went.
    pub(crate) async fn send(
        &self,
        user_id: &str,
        timezone: Option<&str>,
        payload: &Payload,
    ) -> usize {
        let subscriptions = match sqlx::query(
            "SELECT endpoint, p256dh, auth FROM notification_subscriptions
             WHERE user_id = ? AND (?2 IS NULL OR COALESCE(timezone, 'UTC') = ?2)",
        )
        .bind(user_id)
        .bind(timezone)
        .fetch_all(&self.db)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                eprintln!("[notifications] subscriptions: {e}");
                return 0;
            }
        };
        if subscriptions.is_empty() {
            return 0;
        }
        let json = serde_json::to_vec(payload).unwrap_or_default();
        if json.len() > MAX_PAYLOAD {
            eprintln!("[notifications] payload too large ({} bytes)", json.len());
            return 0;
        }
        let vapid = match self.vapid_key().await {
            Ok(key) => key,
            Err(e) => {
                eprintln!("[notifications] VAPID key: {e}");
                return 0;
            }
        };
        let mut sent = 0;
        for row in subscriptions {
            let endpoint: String = row.get(0);
            let (p256dh, auth): (String, String) = (row.get(1), row.get(2));
            match self.post(&endpoint, &p256dh, &auth, &json, vapid).await {
                Ok(()) => sent += 1,
                Err(Delivery::Gone) => {
                    let _ =
                        sqlx::query("DELETE FROM notification_subscriptions WHERE endpoint = ?")
                            .bind(&endpoint)
                            .execute(&self.db)
                            .await;
                }
                Err(Delivery::Failed(e)) => eprintln!("[notifications] {}: {e}", host(&endpoint)),
            }
        }
        sent
    }

    async fn post(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        json: &[u8],
        vapid: &SigningKey,
    ) -> Result<(), Delivery> {
        let failed = |e: String| Delivery::Failed(e);
        let ua_public = URL_SAFE_NO_PAD
            .decode(p256dh)
            .map_err(|e| failed(e.to_string()))?;
        let auth = URL_SAFE_NO_PAD
            .decode(auth)
            .map_err(|e| failed(e.to_string()))?;
        let body = encrypt(json, &ua_public, &auth, &random_secret(), rand::random())
            .map_err(|e| failed(e.to_string()))?;
        let url = url::Url::parse(endpoint).map_err(|e| failed(e.to_string()))?;
        // an address in the URL is not resolved (PublicOnly): checked here
        if !self.allow_private && !literal_host_is_public(&url) {
            return Err(failed("not a public address".into()));
        }
        let authorization = vapid_authorization(
            vapid,
            &url.origin().ascii_serialization(),
            self.contact.as_deref(),
        )
        .map_err(|e| failed(e.to_string()))?;
        let response = self
            .http
            .post(url)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", TTL_SECONDS.to_string())
            .header("Urgency", "normal")
            .header("Authorization", authorization)
            .body(body)
            .send()
            .await
            .map_err(|e| failed(e.to_string()))?;
        match response.status().as_u16() {
            200..=299 => Ok(()),
            404 | 410 => Err(Delivery::Gone),
            status => Err(failed(format!("HTTP {status}"))),
        }
    }

    /// The server's VAPID key: made once, kept encrypted in `server_secrets`.
    async fn vapid_key(&self) -> anyhow::Result<&SigningKey> {
        self.vapid
            .get_or_try_init(|| async {
                let stored = |db: Db| async move {
                    sqlx::query_scalar::<_, String>(
                        "SELECT value FROM server_secrets WHERE name = 'vapid'",
                    )
                    .fetch_optional(&db)
                    .await
                };
                if stored(self.db.clone()).await?.is_none() {
                    let secret = URL_SAFE_NO_PAD.encode(random_secret().to_bytes());
                    // a second server task making one at the same moment keeps the first
                    sqlx::query(
                        "INSERT OR IGNORE INTO server_secrets(name, value) VALUES('vapid', ?)",
                    )
                    .bind(crate::crypto::encrypt(&secret, &self.key)?)
                    .execute(&self.db)
                    .await?;
                }
                let encrypted = stored(self.db.clone())
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no VAPID key"))?;
                let bytes =
                    URL_SAFE_NO_PAD.decode(crate::crypto::decrypt(&encrypted, &self.key)?)?;
                Ok(SigningKey::from(SecretKey::from_slice(&bytes)?))
            })
            .await
    }
}

enum Delivery {
    Gone,
    Failed(String),
}

/// Whether the server may post to `endpoint`'s host: a public address, or a name with one.
async fn reachable_publicly(url: &url::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if let Some(ip) = literal_ip(host) {
        return is_public(ip);
    }
    match tokio::net::lookup_host((host, 0)).await {
        Ok(mut addrs) => addrs.any(|addr| is_public(addr.ip())),
        Err(_) => false,
    }
}

fn host(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// A new P-256 secret key (rejection of the rare 32 bytes that are not one).
fn random_secret() -> SecretKey {
    loop {
        if let Ok(key) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return key;
        }
    }
}

/// RFC 8291: `plaintext` for the device with public key `ua_public` (65 bytes, uncompressed) and auth secret
/// `auth` (16 bytes), from the server's one-time key `as_secret` with `salt`: the aes128gcm body (RFC 8188),
/// one record (rs 4096): salt ‖ rs ‖ idlen ‖ the server's public key ‖ the ciphertext.
fn encrypt(
    plaintext: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    as_secret: &SecretKey,
    salt: [u8; 16],
) -> anyhow::Result<Vec<u8>> {
    let ua_key = PublicKey::from_sec1_bytes(ua_public)?;
    let as_public = as_secret.public_key().to_sec1_point(false);
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_key.as_affine());

    // the input key material: the ECDH secret with the auth secret and both public keys
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public.as_bytes());
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&key_info, &mut ikm)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // the content encryption key and nonce (RFC 8188)
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // the one (last) record: the plaintext and its delimiter 0x02, no padding
    let mut record = plaintext.to_vec();
    record.push(2);
    let ciphertext = Aes128Gcm::new_from_slice(&cek)?
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .map_err(|e| anyhow::anyhow!("aes128gcm: {e}"))?;

    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + ciphertext.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_public.as_bytes().len() as u8);
    body.extend_from_slice(as_public.as_bytes());
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

/// RFC 8292: `vapid t=<JWT for the push service's origin, 12 hours>, k=<the server's public key>`.
fn vapid_authorization(
    key: &SigningKey,
    audience: &str,
    contact: Option<&str>,
) -> anyhow::Result<String> {
    let exp = chrono::Utc::now().timestamp() + 12 * 60 * 60;
    let mut claims = serde_json::json!({ "aud": audience, "exp": exp });
    if let Some(contact) = contact {
        claims["sub"] = serde_json::Value::String(contact.to_owned());
    }
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let unsigned = format!(
        "{header}.{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims)?)
    );
    let signature: Signature = key.sign(unsigned.as_bytes());
    Ok(format!(
        "vapid t={unsigned}.{}, k={}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes()),
        vapid_public(key)
    ))
}

/// The VAPID public key as devices are given it: uncompressed P-256 point, base64url.
fn vapid_public(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_sec1_point(false).as_bytes())
}

// ── API ──────────────────────────────────────────────────────────────────────

/// The server's VAPID public key, which a device registers its Web Push endpoint with.
#[derive(Serialize, ToSchema)]
pub struct VapidKey {
    /// uncompressed P-256 point, base64url (87 characters)
    pub public_key: String,
}

/// A device's Web Push subscription.
#[derive(Deserialize, ToSchema)]
pub struct Subscription {
    /// the push endpoint (https), as the device's push service gave it
    pub endpoint: String,
    /// the device's P-256 public key (RFC 8291), uncompressed, base64url
    pub p256dh: String,
    /// the device's 16-byte auth secret, base64url
    pub auth: String,
    /// the device's timezone (IANA), which reminders of all-day and floating events are timed in; UTC if none
    #[serde(default)]
    pub timezone: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct SubscriptionEndpoint {
    pub endpoint: String,
}

/// How many devices a test notification went to.
#[derive(Serialize, ToSchema)]
pub struct TestSent {
    pub sent: usize,
}

/// The key a device registers its Web Push endpoint with (VAPID).
#[utoipa::path(
    get,
    path = "/notifications/vapid",
    responses(
        (status = 200, body = VapidKey),
        (status = 500, description = "plain text: no key")
    )
)]
pub async fn vapid(State(state): State<AppState>, _user: CurrentUser) -> impl IntoResponse {
    match state.notifier.vapid_key().await {
        Ok(key) => Json(VapidKey {
            public_key: vapid_public(key),
        })
        .into_response(),
        Err(e) => {
            eprintln!("[notifications] VAPID key: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "no key").into_response()
        }
    }
}

/// Stores a device's subscription (again, for an endpoint the user had: its keys may be new).
#[utoipa::path(
    put,
    path = "/notifications/subscription",
    request_body = Subscription,
    responses(
        (status = 204, description = "stored"),
        (status = 400, body = ApiError, description = "not an https endpoint, keys of the wrong size, an unknown timezone, or an endpoint not on a public address (unless NOTIFICATIONS_ALLOW_PRIVATE)"),
        (status = 409, body = ApiError, description = "the endpoint is another user's subscription, with other keys"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn subscribe(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<Subscription>,
) -> impl IntoResponse {
    let url = url::Url::parse(&body.endpoint)
        .ok()
        .filter(|u| u.scheme() == "https" && u.host_str().is_some());
    let https = url.is_some();
    let key_ok = URL_SAFE_NO_PAD
        .decode(&body.p256dh)
        .is_ok_and(|k| k.len() == 65 && PublicKey::from_sec1_bytes(&k).is_ok());
    let auth_ok = URL_SAFE_NO_PAD
        .decode(&body.auth)
        .is_ok_and(|a| a.len() == 16);
    let timezone_ok = body
        .timezone
        .as_deref()
        .is_none_or(|tz| tz.parse::<chrono_tz::Tz>().is_ok());
    if !https || body.endpoint.len() > 1000 || !key_ok || !auth_ok || !timezone_ok {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("invalid subscription")),
        )
            .into_response();
    }
    if !state.notifier.allow_private && !reachable_publicly(url.as_ref().unwrap()).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiError::new("endpoint not on a public address")),
        )
            .into_response();
    }
    // An endpoint of another user's moves to this one only with the same keys (the same device's subscription,
    // signed in to another account): knowing the endpoint alone takes nothing over.
    let result = sqlx::query(
        "INSERT INTO notification_subscriptions(endpoint, user_id, p256dh, auth, timezone, created_at) VALUES(?, ?, ?, ?, ?, ?)
         ON CONFLICT(endpoint) DO UPDATE SET user_id = excluded.user_id, p256dh = excluded.p256dh, auth = excluded.auth,
           timezone = excluded.timezone
         WHERE notification_subscriptions.user_id = excluded.user_id
            OR (notification_subscriptions.p256dh = excluded.p256dh AND notification_subscriptions.auth = excluded.auth)",
    )
    .bind(&body.endpoint)
    .bind(&user.user_id)
    .bind(&body.p256dh)
    .bind(&body.auth)
    .bind(&body.timezone)
    .bind(chrono::Utc::now().timestamp())
    .execute(&state.db)
    .await;
    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::CONFLICT,
            Json(ApiError::new("endpoint of another subscription")),
        )
            .into_response(),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[notifications] subscribe: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Forgets a device's subscription (also when there was none).
#[utoipa::path(
    delete,
    path = "/notifications/subscription",
    request_body = SubscriptionEndpoint,
    responses(
        (status = 204, description = "deleted"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn unsubscribe(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<SubscriptionEndpoint>,
) -> impl IntoResponse {
    match sqlx::query("DELETE FROM notification_subscriptions WHERE endpoint = ? AND user_id = ?")
        .bind(&body.endpoint)
        .bind(&user.user_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[notifications] unsubscribe: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Sends a test notification to each of the user's devices.
#[utoipa::path(
    post,
    path = "/notifications/test",
    responses((status = 200, body = TestSent))
)]
pub async fn test(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    Json(TestSent {
        sent: state
            .notifier
            .send(&user.user_id, None, &Payload::Test)
            .await,
    })
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(vapid))
        .routes(routes!(subscribe, unsubscribe))
        .routes(routes!(test))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(s).unwrap()
    }

    /// RFC 8291 §5: the worked example (its keys, salt and message) gives its body.
    #[test]
    fn encrypts_as_rfc_8291_example() {
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let as_secret =
            SecretKey::from_slice(&b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body = encrypt(
            b"When I grow up, I want to be a watermelon",
            &ua_public,
            &auth,
            &as_secret,
            salt,
        )
        .unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    /// RFC 8291 the other way: what a device does with the body, with its secret key and auth secret.
    fn decrypt(body: &[u8], ua_secret: &SecretKey, auth: &[u8]) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        let id_len = rest[4] as usize;
        let (as_public, ciphertext) = rest[5..].split_at(id_len);
        let ua_public = ua_secret.public_key().to_sec1_point(false);
        let shared = p256::ecdh::diffie_hellman(
            ua_secret.to_nonzero_scalar(),
            PublicKey::from_sec1_bytes(as_public).unwrap().as_affine(),
        );
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(ua_public.as_bytes());
        key_info.extend_from_slice(as_public);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
            .expand(&key_info, &mut ikm)
            .unwrap();
        let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
            .unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce)
            .unwrap();
        let mut plain = Aes128Gcm::new_from_slice(&cek)
            .unwrap()
            .decrypt(&Nonce::from(nonce), ciphertext)
            .unwrap();
        assert_eq!(plain.pop(), Some(2), "the last record's delimiter");
        plain
    }

    /// New mail the broadcaster sends, with no page of the user's open, reaches the user's device: the
    /// payload encrypted for it, with VAPID and the Web Push headers.
    #[tokio::test]
    async fn new_mail_reaches_the_users_devices() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("caley-notify-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init_db(dir.join("t.db").to_str().unwrap())
            .await
            .unwrap();
        crate::db::ensure_user(&db, "u@example", None)
            .await
            .unwrap();

        // the device's push service: records what is posted to it
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(axum::http::HeaderMap, Vec<u8>)>(1);
        let service = axum::Router::new().route(
            "/push/device",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let tx = tx.clone();
                    async move {
                        tx.send((headers, body.to_vec())).await.unwrap();
                        StatusCode::CREATED
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/push/device", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

        let ua_secret = random_secret();
        let auth: [u8; 16] = rand::random();
        sqlx::query("INSERT INTO notification_subscriptions(endpoint, user_id, p256dh, auth, created_at) VALUES(?, ?, ?, ?, 0)")
            .bind(&endpoint)
            .bind("u@example")
            .bind(URL_SAFE_NO_PAD.encode(ua_secret.public_key().to_sec1_point(false).as_bytes()))
            .bind(URL_SAFE_NO_PAD.encode(auth))
            .execute(&db)
            .await
            .unwrap();

        let broadcaster = crate::sse::SseBroadcaster::new();
        broadcaster.set_notifier(Arc::new(
            Notifier::new(db, [7; 32], "https://caley.example", true).unwrap(),
        ));
        broadcaster.send(
            "u@example",
            SseMessage::NewEmail {
                account_id: "work".into(),
                uid: 42,
                mailbox: "INBOX".into(),
                from_addr: Some("Alice <alice@example.com>".into()),
                subject: Some("Lunch?".into()),
            },
        );

        let (headers, body) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(headers["content-encoding"], "aes128gcm");
        assert_eq!(headers["ttl"], TTL_SECONDS.to_string());
        assert!(
            headers["authorization"]
                .to_str()
                .unwrap()
                .starts_with("vapid t=")
        );
        let payload: serde_json::Value =
            serde_json::from_slice(&decrypt(&body, &ua_secret, &auth)).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({
                "kind": "new_email",
                "account_id": "work",
                "mailbox": "INBOX",
                "uid": 42,
                "from": "Alice <alice@example.com>",
                "subject": "Lunch?",
                "url": "/mail/view/?account_id=work&mailbox=INBOX&uid=42",
            })
        );
    }

    /// Without NOTIFICATIONS_ALLOW_PRIVATE nothing is posted to a private address: not by its name (the
    /// resolver), not as an address in the URL.
    #[tokio::test]
    async fn private_endpoints_get_nothing() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("caley-private-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init_db(dir.join("t.db").to_str().unwrap())
            .await
            .unwrap();
        let notifier = Notifier::new(db, [7; 32], "https://caley.example", false).unwrap();
        let key = SigningKey::from(random_secret());
        let device = random_secret().public_key().to_sec1_point(false);
        let p256dh = URL_SAFE_NO_PAD.encode(device.as_bytes());
        let auth = URL_SAFE_NO_PAD.encode([1u8; 16]);

        // a push service on this machine, counting the connections it gets
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        tokio::spawn(async move {
            while listener.accept().await.is_ok() {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });

        for endpoint in [
            format!("http://localhost:{port}/push"),
            format!("http://127.0.0.1:{port}/push"),
        ] {
            assert!(
                matches!(
                    notifier.post(&endpoint, &p256dh, &auth, b"{}", &key).await,
                    Err(Delivery::Failed(_))
                ),
                "{endpoint}"
            );
        }
        assert_eq!(
            connections.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no connection to a private address"
        );

        // allowed, the same service is reached
        let allowing =
            Notifier::new(notifier.db.clone(), [7; 32], "https://caley.example", true).unwrap();
        let _ = allowing
            .post(
                &format!("http://localhost:{port}/push"),
                &p256dh,
                &auth,
                b"{}",
                &key,
            )
            .await;
        assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// The VAPID header's JWT verifies with the key it names.
    #[test]
    fn vapid_jwt_verifies() {
        use p256::ecdsa::{VerifyingKey, signature::Verifier};
        let key = SigningKey::from(random_secret());
        let header =
            vapid_authorization(&key, "https://push.example", Some("https://caley.example"))
                .unwrap();
        let (t, k) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        let (unsigned, signature) = t.rsplit_once('.').unwrap();
        let verifying = VerifyingKey::from_sec1_bytes(&b64(k)).unwrap();
        let signature = Signature::from_slice(&b64(signature)).unwrap();
        verifying.verify(unsigned.as_bytes(), &signature).unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&b64(unsigned.split('.').nth(1).unwrap())).unwrap();
        assert_eq!(claims["aud"], "https://push.example");
        assert_eq!(claims["sub"], "https://caley.example");
    }
}

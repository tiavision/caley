use axum::{
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use tower_sessions::Session;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use webauthn_rs::prelude::*;

use crate::db;
use crate::email_db;
use crate::email_sync::eml_path;
use crate::oidc::{AppState, CurrentUser};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn user_id_to_uuid(user_id: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(user_id.as_bytes());
    let bytes: [u8; 16] = hash[..16].try_into().unwrap();
    Uuid::from_bytes(bytes)
}

fn configure_prf_registration(challenge: &mut serde_json::Value) {
    if let Some(pk) = challenge.get_mut("publicKey") {
        if let Some(obj) = pk.as_object_mut() {
            // Android credential providers are only offered for discoverable
            // passkey registration. Do not constrain authenticator attachment:
            // that would exclude third-party passkey managers.
            if let Some(selection) = obj
                .entry("authenticatorSelection".to_owned())
                .or_insert_with(|| json!({}))
                .as_object_mut()
            {
                selection.insert("residentKey".to_owned(), json!("required"));
                selection.insert("requireResidentKey".to_owned(), json!(true));
            }

            if let Some(extensions) = obj
                .entry("extensions".to_owned())
                .or_insert_with(|| json!({}))
                .as_object_mut()
            {
                extensions.insert("prf".to_owned(), json!({}));
            }
        }
    }
}

fn inject_prf_authentication(challenge: &mut serde_json::Value, prf_salt_b64: &str) {
    if let Some(pk) = challenge.get_mut("publicKey") {
        if let Some(obj) = pk.as_object_mut() {
            obj.insert(
                "extensions".to_owned(),
                json!({ "prf": { "eval": { "first": prf_salt_b64 } } }),
            );
        }
    }
}

/// The WebAuthn credential a registration or an authentication ended with.
#[derive(Serialize, ToSchema)]
pub struct WebAuthnCredentialId {
    /// the credential ID, base64url without padding
    pub cred_id: String,
}

// ── Register start ────────────────────────────────────────────────────────────

/// Starts a passkey registration for the signed-in user (excluding the credentials already
/// enrolled): the challenge to pass to `navigator.credentials.create()`. The registration state is
/// kept in the session for `/webauthn/register/finish`.
#[utoipa::path(
    post,
    path = "/webauthn/register/start",
    responses(
        (status = 200, body = Object, description = "WebAuthn `CreationChallengeResponse` (`{ publicKey: PublicKeyCredentialCreationOptions }`), with a required resident key and the `prf` extension requested"),
        (status = 500, description = "plain text: WebAuthn or session error")
    )
)]
pub async fn register_start(
    State(state): State<AppState>,
    session: Session,
    user: CurrentUser,
) -> impl IntoResponse {
    let user_uuid = user_id_to_uuid(&user.user_id);
    let existing = db::get_webauthn_cred_ids(&state.db, &user.user_id)
        .await
        .unwrap_or_default();

    let exclude: Option<Vec<CredentialID>> = if existing.is_empty() {
        None
    } else {
        let ids = existing
            .iter()
            .filter_map(|b64| URL_SAFE_NO_PAD.decode(b64).ok())
            .map(CredentialID::from)
            .collect();
        Some(ids)
    };

    match state.webauthn.start_passkey_registration(
        user_uuid,
        &user.user_id,
        user.display_name.as_deref().unwrap_or(&user.user_id),
        exclude,
    ) {
        Ok((ccr, reg_state)) => {
            let state_json = match serde_json::to_string(&reg_state) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[pgp] serialize reg_state: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
                }
            };
            if let Err(e) = session.insert("pgp_reg_state", state_json).await {
                eprintln!("[pgp] session insert reg_state: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "session error").into_response();
            }
            let mut challenge = match serde_json::to_value(&ccr) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[pgp] serialize ccr: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
                }
            };
            configure_prf_registration(&mut challenge);
            Json(challenge).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] start_passkey_registration: {e:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, "webauthn error").into_response()
        }
    }
}

// ── Register finish ───────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct RegisterFinishBody {
    /// WebAuthn `RegisterPublicKeyCredential`: the result of `navigator.credentials.create()`
    #[schema(value_type = Object)]
    pub credential: serde_json::Value,
    pub device_label: Option<String>,
    /// whether the authenticator reported the `prf` extension as enabled (default false)
    pub has_prf: Option<bool>,
}

/// Finishes the passkey registration started in the session and stores the credential.
#[utoipa::path(
    post,
    path = "/webauthn/register/finish",
    request_body = RegisterFinishBody,
    responses(
        (status = 200, body = WebAuthnCredentialId),
        (status = 400, description = "plain text: no registration in progress, invalid credential or registration failed"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn register_finish(
    State(state): State<AppState>,
    session: Session,
    user: CurrentUser,
    Json(body): Json<RegisterFinishBody>,
) -> impl IntoResponse {
    let reg_state_json = match session.get::<String>("pgp_reg_state").await {
        Ok(Some(s)) => s,
        _ => return (StatusCode::BAD_REQUEST, "no registration in progress").into_response(),
    };
    let _ = session.remove::<String>("pgp_reg_state").await;

    let reg_state: PasskeyRegistration = match serde_json::from_str(&reg_state_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[pgp] deserialize reg_state: {e}");
            return (StatusCode::BAD_REQUEST, "invalid registration state").into_response();
        }
    };

    let reg_cred: RegisterPublicKeyCredential = match serde_json::from_value(body.credential) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[pgp] deserialize credential: {e}");
            return (StatusCode::BAD_REQUEST, "invalid credential").into_response();
        }
    };

    match state
        .webauthn
        .finish_passkey_registration(&reg_cred, &reg_state)
    {
        Ok(passkey) => {
            let cred_id = URL_SAFE_NO_PAD.encode(passkey.cred_id());
            let passkey_json = match serde_json::to_string(&passkey) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[pgp] serialize passkey: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
                }
            };
            let has_prf = body.has_prf.unwrap_or(false);
            if let Err(e) = db::store_webauthn_credential(
                &state.db,
                &cred_id,
                &user.user_id,
                &passkey_json,
                body.device_label.as_deref(),
                has_prf,
            )
            .await
            {
                eprintln!("[pgp] store_webauthn_credential: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
            Json(WebAuthnCredentialId { cred_id }).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] finish_passkey_registration: {e:?}");
            (StatusCode::BAD_REQUEST, "registration failed").into_response()
        }
    }
}

// ── Authenticate start ────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AuthStartBody {
    /// the salt for the `prf` extension's `eval.first`, base64url
    pub prf_salt: String,
    /// only this credential (base64url ID) is allowed; all the user's credentials without it
    pub credential_id: Option<String>,
}

/// Starts a passkey authentication with the user's credentials: the challenge to pass to
/// `navigator.credentials.get()`, evaluating the `prf` extension with `prf_salt`. The state is kept
/// in the session for `/webauthn/authenticate/finish`.
#[utoipa::path(
    post,
    path = "/webauthn/authenticate/start",
    request_body = AuthStartBody,
    responses(
        (status = 200, body = Object, description = "WebAuthn `RequestChallengeResponse` (`{ publicKey: PublicKeyCredentialRequestOptions }`), its `extensions` set to `{ prf: { eval: { first: prf_salt } } }`"),
        (status = 404, description = "plain text: no credentials enrolled"),
        (status = 500, description = "plain text: database, WebAuthn or session error")
    )
)]
pub async fn authenticate_start(
    State(state): State<AppState>,
    session: Session,
    user: CurrentUser,
    Json(body): Json<AuthStartBody>,
) -> impl IntoResponse {
    let rows = match db::get_webauthn_credentials(&state.db, &user.user_id).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[pgp] get_webauthn_credentials: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let passkeys: Vec<Passkey> = rows
        .iter()
        .filter(|r| {
            body.credential_id
                .as_ref()
                .map_or(true, |id| id == &r.cred_id)
        })
        .filter_map(|r| serde_json::from_str::<Passkey>(&r.passkey).ok())
        .collect();

    if passkeys.is_empty() {
        return (StatusCode::NOT_FOUND, "no credentials enrolled").into_response();
    }

    match state.webauthn.start_passkey_authentication(&passkeys) {
        Ok((rcr, auth_state)) => {
            let state_json = match serde_json::to_string(&auth_state) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[pgp] serialize auth_state: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
                }
            };
            if let Err(e) = session.insert("pgp_auth_state", state_json).await {
                eprintln!("[pgp] session insert auth_state: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "session error").into_response();
            }
            let mut challenge = match serde_json::to_value(&rcr) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[pgp] serialize rcr: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
                }
            };
            inject_prf_authentication(&mut challenge, &body.prf_salt);
            Json(challenge).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] start_passkey_authentication: {e:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, "webauthn error").into_response()
        }
    }
}

// ── Authenticate finish ───────────────────────────────────────────────────────

/// Finishes the passkey authentication started in the session and updates the credential's
/// counter: the credential that was used.
#[utoipa::path(
    post,
    path = "/webauthn/authenticate/finish",
    request_body(content = Object, description = "WebAuthn `PublicKeyCredential`: the result of `navigator.credentials.get()`"),
    responses(
        (status = 200, body = WebAuthnCredentialId),
        (status = 400, description = "plain text: no authentication in progress or invalid credential"),
        (status = 401, description = "plain text: authentication failed"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn authenticate_finish(
    State(state): State<AppState>,
    session: Session,
    user: CurrentUser,
    Json(credential): Json<serde_json::Value>,
) -> impl IntoResponse {
    let auth_state_json = match session.get::<String>("pgp_auth_state").await {
        Ok(Some(s)) => s,
        _ => return (StatusCode::BAD_REQUEST, "no authentication in progress").into_response(),
    };
    let _ = session.remove::<String>("pgp_auth_state").await;

    let auth_state: PasskeyAuthentication = match serde_json::from_str(&auth_state_json) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[pgp] deserialize auth_state: {e}");
            return (StatusCode::BAD_REQUEST, "invalid auth state").into_response();
        }
    };

    let auth_cred: PublicKeyCredential = match serde_json::from_value(credential) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[pgp] deserialize auth credential: {e}");
            return (StatusCode::BAD_REQUEST, "invalid credential").into_response();
        }
    };

    let rows = match db::get_webauthn_credentials(&state.db, &user.user_id).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[pgp] get_webauthn_credentials: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    match state
        .webauthn
        .finish_passkey_authentication(&auth_cred, &auth_state)
    {
        Ok(auth_result) => {
            let matched_id = URL_SAFE_NO_PAD.encode(auth_result.cred_id());
            for row in &rows {
                if row.cred_id != matched_id {
                    continue;
                }
                if let Ok(mut passkey) = serde_json::from_str::<Passkey>(&row.passkey) {
                    passkey.update_credential(&auth_result);
                    if let Ok(updated_json) = serde_json::to_string(&passkey) {
                        let now = chrono::Utc::now().timestamp();
                        let _ = db::update_webauthn_credential(
                            &state.db,
                            &matched_id,
                            &updated_json,
                            auth_result.counter(),
                            now,
                        )
                        .await;
                    }
                }
                break;
            }
            Json(WebAuthnCredentialId {
                cred_id: matched_id,
            })
            .into_response()
        }
        Err(e) => {
            eprintln!("[pgp] finish_passkey_authentication: {e:?}");
            (StatusCode::UNAUTHORIZED, "authentication failed").into_response()
        }
    }
}

// ── List credentials ─────────────────────────────────────────────────────────

/// An enrolled WebAuthn credential (passkey).
#[derive(Serialize, ToSchema)]
pub struct CredentialInfo {
    /// the credential ID, base64url without padding
    pub cred_id: String,
    /// null when none was given
    #[schema(required = true)]
    pub device_label: Option<String>,
    /// whether the credential supports the `prf` extension (needed to unlock the vault)
    pub has_prf: bool,
}

/// The signed-in user's WebAuthn credentials.
#[utoipa::path(
    get,
    path = "/webauthn/credentials",
    responses(
        (status = 200, body = Vec<CredentialInfo>),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn list_credentials(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match db::get_webauthn_credentials(&state.db, &user.user_id).await {
        Ok(rows) => {
            let list: Vec<CredentialInfo> = rows
                .into_iter()
                .map(|r| CredentialInfo {
                    cred_id: r.cred_id,
                    device_label: r.device_label,
                    has_prf: r.has_prf,
                })
                .collect();
            Json(list).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] list_credentials: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Delete credential ─────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct DeleteCredentialBody {
    pub cred_id: String,
}

/// Deletes one of the user's WebAuthn credentials (with its vault device and wrap).
#[utoipa::path(
    delete,
    path = "/webauthn/credential",
    request_body = DeleteCredentialBody,
    responses(
        (status = 204, description = "deleted (also when there was no such credential)"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn delete_credential(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<DeleteCredentialBody>,
) -> impl IntoResponse {
    match db::delete_webauthn_credential(&state.db, &body.cred_id, &user.user_id).await {
        Ok(_) => {
            // a device of the vault goes with its credential
            state.sse_broadcaster.send(&user.user_id, crate::sse::SseMessage::VaultChanged);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            eprintln!("[pgp] delete_credential: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Synced encrypted private-key vault ───────────────────────────────────────
//
// The server stores the vault as opaque records: AES-GCM ciphertexts the browser encrypts and
// decrypts with keys derived from the passkeys' PRF output, and the devices' public key material.
// Private-key material is never decrypted here.

/// Deletes the whole vault: its manifest, the encrypted private keys, and the user's WebAuthn
/// credentials with their devices and wraps.
#[utoipa::path(
    delete,
    path = "/pgp/vault",
    responses(
        (status = 204, description = "deleted"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn reset_vault(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response(),
    };
    let result = async {
        // Deleting WebAuthn credentials cascades through device bundles and wraps.
        sqlx::query("DELETE FROM webauthn_credentials WHERE user_id = ?")
            .bind(&user.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pgp_private_keys WHERE user_id = ?")
            .bind(&user.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pgp_vaults WHERE user_id = ?")
            .bind(&user.user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }
    .await;
    match result {
        Ok(_) => {
            state.sse_broadcaster.send(&user.user_id, crate::sse::SseMessage::VaultChanged);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            eprintln!("[pgp] reset vault: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// An opaque AES-GCM ciphertext encrypted in the browser (the vault manifest); the server cannot
/// decrypt it.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct EncryptedBlob {
    /// the AES-GCM nonce, base64url
    pub iv: String,
    /// base64url
    pub ciphertext: String,
}

/// An OpenPGP private key of the vault, encrypted in the browser with the vault key (opaque to
/// the server). The frontend's `VaultKeyRecord`.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct PrivateKeyBlob {
    /// the key's fingerprint (hex)
    pub fingerprint: String,
    /// the AES-GCM nonce, base64url
    pub iv: String,
    /// the encrypted armored private key, base64url
    pub ciphertext: String,
}

/// A device's public key material and its own private key encrypted with its passkey's PRF output
/// (opaque to the server).
#[derive(Deserialize, Serialize, ToSchema)]
pub struct DeviceBundle {
    /// the device's WebAuthn credential ID
    pub cred_id: String,
    /// the device's public ECDH key, a JWK
    #[schema(value_type = Object)]
    pub public_jwk: serde_json::Value,
    /// the device's private JWK, encrypted with its PRF-derived key
    pub encrypted_private_jwk: String,
    pub private_iv: String,
    pub prf_salt: String,
    pub hkdf_salt: String,
    pub pairing_nonce: String,
}

/// The vault key wrapped for one device (ECDH with an ephemeral key, then AES-GCM); opaque to the
/// server.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct VaultWrap {
    /// the device's WebAuthn credential ID
    pub cred_id: String,
    /// the ephemeral public ECDH key, a JWK (null if the stored one could not be read)
    #[schema(value_type = Object)]
    pub ephemeral_public_jwk: serde_json::Value,
    pub hkdf_salt: String,
    pub iv: String,
    pub ciphertext: String,
}

#[derive(Deserialize, ToSchema)]
pub struct InitializeVaultBody {
    pub manifest: EncryptedBlob,
    pub key: PrivateKeyBlob,
    pub device: DeviceBundle,
    pub wrap: VaultWrap,
}

#[derive(Deserialize, ToSchema)]
pub struct PendingDeviceBody {
    pub device: DeviceBundle,
}

/// A pending device made active, with the vault key wrapped for it.
#[derive(Deserialize, ToSchema)]
pub struct ApproveDeviceBody {
    pub cred_id: String,
    pub wrap: VaultWrap,
}

/// A change of the vault, applied only if the vault is still at `expected_revision`. The
/// frontend's `VaultUpdate`.
#[derive(Deserialize, ToSchema)]
pub struct UpdateVaultBody {
    pub expected_revision: i64,
    /// the new encrypted manifest
    pub manifest: EncryptedBlob,
    /// keys added or replaced
    #[serde(default)]
    pub upsert_keys: Vec<PrivateKeyBlob>,
    /// fingerprints of keys to delete
    #[serde(default)]
    pub delete_keys: Vec<String>,
    pub approve_device: Option<ApproveDeviceBody>,
    /// a credential ID whose credential, device and wrap are deleted
    pub remove_device: Option<String>,
    /// when given, replaces all wraps
    pub replace_wraps: Option<Vec<VaultWrap>>,
}

/// A device of the vault, as stored: its bundle, status and wrap.
#[derive(Serialize, ToSchema)]
pub struct DeviceRecord {
    pub cred_id: String,
    #[schema(required = true)]
    pub device_label: Option<String>,
    /// `pending` (waiting for approval by an active device) or `active`
    pub status: String,
    /// the device's public ECDH key, a JWK (null if the stored one could not be read)
    #[schema(value_type = Object)]
    pub public_jwk: serde_json::Value,
    pub encrypted_private_jwk: String,
    pub private_iv: String,
    pub prf_salt: String,
    pub hkdf_salt: String,
    pub pairing_nonce: String,
    /// the vault key wrapped for this device; null for a pending one
    #[schema(required = true)]
    pub wrap: Option<VaultWrap>,
}

/// The synced vault: opaque encrypted records and the devices' public material.
#[derive(Serialize, ToSchema)]
pub struct VaultData {
    /// incremented by each update
    pub revision: i64,
    pub manifest: EncryptedBlob,
    /// ordered by fingerprint
    pub keys: Vec<PrivateKeyBlob>,
    /// in the order they were added
    pub devices: Vec<DeviceRecord>,
}

/// The vault's revision after an update.
#[derive(Serialize, ToSchema)]
pub struct VaultRevision {
    pub revision: i64,
}

async fn owns_credential(state: &AppState, user_id: &str, cred_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM webauthn_credentials WHERE user_id = ? AND cred_id = ?",
    )
    .bind(user_id)
    .bind(cred_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0)
        > 0
}

/// The user's vault, or 204 when none is initialized.
#[utoipa::path(
    get,
    path = "/pgp/vault",
    responses(
        (status = 200, body = VaultData),
        (status = 204, description = "no vault initialized"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn get_vault(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    let vault = match sqlx::query(
        "SELECT revision, manifest_iv, encrypted_manifest FROM pgp_vaults WHERE user_id = ?",
    )
    .bind(&user.user_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[pgp] get vault: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let keys = match sqlx::query(
        "SELECT fingerprint, iv, ciphertext FROM pgp_private_keys WHERE user_id = ? ORDER BY fingerprint",
    )
    .bind(&user.user_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|r| PrivateKeyBlob {
                fingerprint: r.get::<String, _>(0),
                iv: r.get::<String, _>(1),
                ciphertext: r.get::<String, _>(2),
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("[pgp] get vault keys: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    let devices = match sqlx::query(
        "SELECT d.cred_id, w.device_label, d.status, d.public_jwk, d.encrypted_private_jwk,
                d.private_iv, d.prf_salt, d.hkdf_salt, d.pairing_nonce,
                v.ephemeral_public_jwk, v.hkdf_salt, v.iv, v.ciphertext
         FROM pgp_devices d
         JOIN webauthn_credentials w ON w.cred_id = d.cred_id
         LEFT JOIN pgp_vault_wraps v ON v.user_id = d.user_id AND v.cred_id = d.cred_id
         WHERE d.user_id = ? ORDER BY d.created_at",
    )
    .bind(&user.user_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|r| {
                let envelope = r.get::<Option<String>, _>(9).map(|jwk| VaultWrap {
                    cred_id: r.get::<String, _>(0),
                    ephemeral_public_jwk: serde_json::from_str::<serde_json::Value>(&jwk)
                        .unwrap_or(serde_json::Value::Null),
                    hkdf_salt: r.get::<Option<String>, _>(10).unwrap_or_default(),
                    iv: r.get::<Option<String>, _>(11).unwrap_or_default(),
                    ciphertext: r.get::<Option<String>, _>(12).unwrap_or_default(),
                });
                DeviceRecord {
                    cred_id: r.get::<String, _>(0),
                    device_label: r.get::<Option<String>, _>(1),
                    status: r.get::<String, _>(2),
                    public_jwk: serde_json::from_str::<serde_json::Value>(&r.get::<String, _>(3))
                        .unwrap_or(serde_json::Value::Null),
                    encrypted_private_jwk: r.get::<String, _>(4),
                    private_iv: r.get::<String, _>(5),
                    prf_salt: r.get::<String, _>(6),
                    hkdf_salt: r.get::<String, _>(7),
                    pairing_nonce: r.get::<String, _>(8),
                    wrap: envelope,
                }
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("[pgp] get vault devices: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    Json(VaultData {
        revision: vault.get::<i64, _>(0),
        manifest: EncryptedBlob {
            iv: vault.get::<String, _>(1),
            ciphertext: vault.get::<String, _>(2),
        },
        keys,
        devices,
    })
    .into_response()
}

/// Creates the vault (revision 1) with its first key and first device (active, with its wrap);
/// the device's credential must be the user's.
#[utoipa::path(
    post,
    path = "/pgp/vault/initialize",
    request_body = InitializeVaultBody,
    responses(
        (status = 201, description = "created"),
        (status = 400, description = "plain text: invalid device credential"),
        (status = 409, description = "plain text: vault already exists"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn initialize_vault(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<InitializeVaultBody>,
) -> impl IntoResponse {
    if body.wrap.cred_id != body.device.cred_id
        || !owns_credential(&state, &user.user_id, &body.device.cred_id).await
    {
        return (StatusCode::BAD_REQUEST, "invalid device credential").into_response();
    }
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response(),
    };
    let now = chrono::Utc::now().timestamp();
    let result = async {
        sqlx::query(
            "INSERT INTO pgp_vaults(user_id, revision, manifest_iv, encrypted_manifest, created_at, updated_at)
             VALUES(?, 1, ?, ?, ?, ?)",
        )
        .bind(&user.user_id).bind(&body.manifest.iv).bind(&body.manifest.ciphertext).bind(now).bind(now)
        .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO pgp_private_keys(user_id, fingerprint, iv, ciphertext) VALUES(?, ?, ?, ?)",
        )
        .bind(&user.user_id).bind(&body.key.fingerprint).bind(&body.key.iv).bind(&body.key.ciphertext)
        .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO pgp_devices(cred_id, user_id, status, public_jwk, encrypted_private_jwk, private_iv, prf_salt, hkdf_salt, pairing_nonce, created_at)
             VALUES(?, ?, 'active', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&body.device.cred_id).bind(&user.user_id).bind(body.device.public_jwk.to_string())
        .bind(&body.device.encrypted_private_jwk).bind(&body.device.private_iv).bind(&body.device.prf_salt)
        .bind(&body.device.hkdf_salt).bind(&body.device.pairing_nonce).bind(now)
        .execute(&mut *tx).await?;
        insert_wrap(&mut tx, &user.user_id, &body.wrap).await?;
        sqlx::query("UPDATE webauthn_credentials SET has_prf = 1 WHERE cred_id = ? AND user_id = ?")
            .bind(&body.device.cred_id).bind(&user.user_id).execute(&mut *tx).await?;
        tx.commit().await
    }
    .await;
    match result {
        Ok(_) => {
            state.sse_broadcaster.send(&user.user_id, crate::sse::SseMessage::VaultChanged);
            StatusCode::CREATED.into_response()
        }
        Err(e) => {
            eprintln!("[pgp] initialize vault: {e}");
            (StatusCode::CONFLICT, "vault already exists").into_response()
        }
    }
}

async fn insert_wrap(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    user_id: &str,
    wrap: &VaultWrap,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pgp_vault_wraps(user_id, cred_id, ephemeral_public_jwk, hkdf_salt, iv, ciphertext)
         VALUES(?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, cred_id) DO UPDATE SET ephemeral_public_jwk = excluded.ephemeral_public_jwk,
           hkdf_salt = excluded.hkdf_salt, iv = excluded.iv, ciphertext = excluded.ciphertext",
    )
    .bind(user_id).bind(&wrap.cred_id).bind(wrap.ephemeral_public_jwk.to_string())
    .bind(&wrap.hkdf_salt).bind(&wrap.iv).bind(&wrap.ciphertext)
    .execute(&mut **tx).await?;
    Ok(())
}

/// Adds a device waiting for approval by an active device (`approve_device` of `PUT /pgp/vault`).
#[utoipa::path(
    post,
    path = "/pgp/devices/pending",
    request_body = PendingDeviceBody,
    responses(
        (status = 201, description = "added"),
        (status = 400, description = "plain text: invalid device credential"),
        (status = 409, description = "plain text: vault not initialized or device already registered")
    )
)]
pub async fn create_pending_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<PendingDeviceBody>,
) -> impl IntoResponse {
    if !owns_credential(&state, &user.user_id, &body.device.cred_id).await {
        return (StatusCode::BAD_REQUEST, "invalid device credential").into_response();
    }
    let has_vault =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pgp_vaults WHERE user_id = ?")
            .bind(&user.user_id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0)
            > 0;
    if !has_vault {
        return (StatusCode::CONFLICT, "vault not initialized").into_response();
    }
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query(
        "INSERT INTO pgp_devices(cred_id, user_id, status, public_jwk, encrypted_private_jwk, private_iv, prf_salt, hkdf_salt, pairing_nonce, created_at)
         VALUES(?, ?, 'pending', ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&body.device.cred_id).bind(&user.user_id).bind(body.device.public_jwk.to_string())
    .bind(&body.device.encrypted_private_jwk).bind(&body.device.private_iv).bind(&body.device.prf_salt)
    .bind(&body.device.hkdf_salt).bind(&body.device.pairing_nonce).bind(now)
    .execute(&state.db).await;
    match result {
        Ok(_) => {
            let _ = sqlx::query(
                "UPDATE webauthn_credentials SET has_prf = 1 WHERE cred_id = ? AND user_id = ?",
            )
            .bind(&body.device.cred_id)
            .bind(&user.user_id)
            .execute(&state.db)
            .await;
            state.sse_broadcaster.send(&user.user_id, crate::sse::SseMessage::VaultChanged);
            StatusCode::CREATED.into_response()
        }
        Err(e) => {
            eprintln!("[pgp] pending device: {e}");
            (StatusCode::CONFLICT, "device already registered").into_response()
        }
    }
}

/// Applies a change to the vault in one transaction if it is still at `expected_revision`.
#[utoipa::path(
    put,
    path = "/pgp/vault",
    request_body = UpdateVaultBody,
    responses(
        (status = 200, body = VaultRevision),
        (status = 400, description = "plain text: invalid vault update (e.g. no such pending device)"),
        (status = 409, description = "plain text: vault revision conflict"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn update_vault(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<UpdateVaultBody>,
) -> impl IntoResponse {
    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response(),
    };
    let now = chrono::Utc::now().timestamp();
    let updated = match sqlx::query(
        "UPDATE pgp_vaults SET revision = revision + 1, manifest_iv = ?, encrypted_manifest = ?, updated_at = ?
         WHERE user_id = ? AND revision = ?",
    )
    .bind(&body.manifest.iv).bind(&body.manifest.ciphertext).bind(now).bind(&user.user_id).bind(body.expected_revision)
    .execute(&mut *tx).await
    {
        Ok(r) if r.rows_affected() == 1 => true,
        Ok(_) => false,
        Err(e) => {
            eprintln!("[pgp] update vault: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    if !updated {
        return (StatusCode::CONFLICT, "vault revision conflict").into_response();
    }

    let result = async {
        for key in &body.upsert_keys {
            sqlx::query(
                "INSERT INTO pgp_private_keys(user_id, fingerprint, iv, ciphertext) VALUES(?, ?, ?, ?)
                 ON CONFLICT(user_id, fingerprint) DO UPDATE SET iv = excluded.iv, ciphertext = excluded.ciphertext",
            )
            .bind(&user.user_id).bind(&key.fingerprint).bind(&key.iv).bind(&key.ciphertext)
            .execute(&mut *tx).await?;
        }
        for fingerprint in &body.delete_keys {
            sqlx::query("DELETE FROM pgp_private_keys WHERE user_id = ? AND fingerprint = ?")
                .bind(&user.user_id).bind(fingerprint).execute(&mut *tx).await?;
        }
        if let Some(approval) = &body.approve_device {
            if approval.cred_id != approval.wrap.cred_id {
                return Err(sqlx::Error::Protocol("wrap credential mismatch".into()));
            }
            let approved = sqlx::query("UPDATE pgp_devices SET status = 'active' WHERE user_id = ? AND cred_id = ? AND status = 'pending'")
                .bind(&user.user_id).bind(&approval.cred_id).execute(&mut *tx).await?;
            if approved.rows_affected() != 1 {
                return Err(sqlx::Error::Protocol("pending device not found".into()));
            }
            insert_wrap(&mut tx, &user.user_id, &approval.wrap).await?;
        }
        if let Some(cred_id) = &body.remove_device {
            sqlx::query("DELETE FROM webauthn_credentials WHERE user_id = ? AND cred_id = ?")
                .bind(&user.user_id).bind(cred_id).execute(&mut *tx).await?;
        }
        if let Some(wraps) = &body.replace_wraps {
            sqlx::query("DELETE FROM pgp_vault_wraps WHERE user_id = ?")
                .bind(&user.user_id).execute(&mut *tx).await?;
            for wrap in wraps {
                insert_wrap(&mut tx, &user.user_id, wrap).await?;
            }
        }
        tx.commit().await
    }
    .await;
    match result {
        Ok(_) => {
            state.sse_broadcaster.send(&user.user_id, crate::sse::SseMessage::VaultChanged);
            Json(VaultRevision {
                revision: body.expected_revision + 1,
            })
            .into_response()
        }
        Err(e) => {
            eprintln!("[pgp] apply vault update: {e}");
            (StatusCode::BAD_REQUEST, "invalid vault update").into_response()
        }
    }
}

// ── Trusted PGP public keys ───────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PublicKeysQuery {
    /// only the keys of this address
    pub email: Option<String>,
}

/// A stored OpenPGP public key: the user's own or a trusted sender's.
#[derive(Serialize, ToSchema)]
pub struct StoredPublicKey {
    /// the key's fingerprint (hex)
    pub fingerprint: String,
    pub email: String,
    /// the ASCII-armored public key
    pub armored: String,
    /// whether it is the public half of one of the user's own keys
    pub is_own: bool,
    /// where it came from, e.g. `imported` or `autocrypt`
    pub source: String,
    /// e.g. `trusted`
    pub trust: String,
}

/// The stored public keys, optionally of one address.
#[utoipa::path(
    get,
    path = "/pgp/keys",
    params(PublicKeysQuery),
    responses(
        (status = 200, body = Vec<StoredPublicKey>),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn list_public_keys(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PublicKeysQuery>,
) -> impl IntoResponse {
    match db::get_pgp_public_keys(&state.db, &user.user_id, q.email.as_deref()).await {
        Ok(rows) => {
            let list: Vec<StoredPublicKey> = rows
                .into_iter()
                .map(|row| StoredPublicKey {
                    fingerprint: row.fingerprint,
                    email: row.email,
                    armored: row.armored,
                    is_own: row.is_own,
                    source: row.source,
                    trust: row.trust,
                })
                .collect();
            Json(list).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] list public keys: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct SavePublicKeyBody {
    pub fingerprint: String,
    pub email: String,
    /// an ASCII-armored public key block
    pub armored: String,
    /// default false
    pub is_own: Option<bool>,
    /// default `imported`
    pub source: Option<String>,
}

/// Adds or replaces a public key (stored as trusted); answers the stored key.
#[utoipa::path(
    post,
    path = "/pgp/key",
    request_body = SavePublicKeyBody,
    responses(
        (status = 200, body = StoredPublicKey, description = "saved"),
        (status = 400, description = "plain text: invalid public key"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn save_public_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<SavePublicKeyBody>,
) -> impl IntoResponse {
    if body.fingerprint.trim().is_empty()
        || !body
            .armored
            .contains("-----BEGIN PGP PUBLIC KEY BLOCK-----")
    {
        return (StatusCode::BAD_REQUEST, "invalid public key").into_response();
    }

    let source = body.source.unwrap_or_else(|| "imported".to_string());
    let is_own = body.is_own.unwrap_or(false);
    if let Err(e) = db::upsert_pgp_public_key(
        &state.db,
        &user.user_id,
        &body.fingerprint,
        &body.email,
        &body.armored,
        is_own,
        &source,
        "trusted",
    )
    .await
    {
        eprintln!("[pgp] save public key: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    Json(StoredPublicKey {
        fingerprint: body.fingerprint,
        email: body.email,
        armored: body.armored,
        is_own,
        source,
        trust: "trusted".to_string(),
    })
    .into_response()
}

#[derive(Deserialize, ToSchema)]
pub struct DeletePublicKeyBody {
    pub fingerprint: String,
}

/// Deletes a stored public key.
#[utoipa::path(
    delete,
    path = "/pgp/key",
    request_body = DeletePublicKeyBody,
    responses(
        (status = 204, description = "deleted (also when there was no such key)"),
        (status = 500, description = "plain text: database error")
    )
)]
pub async fn delete_public_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<DeletePublicKeyBody>,
) -> impl IntoResponse {
    match db::delete_pgp_public_key(&state.db, &user.user_id, &body.fingerprint).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[pgp] delete public key: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Raw email ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RawEmailQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

/// A cached message as it came from the server (for PGP/MIME decryption and signature checks).
#[utoipa::path(
    get,
    path = "/email/raw",
    params(RawEmailQuery),
    responses(
        (status = 200, content_type = "message/rfc822", body = String, description = "the raw message (RFC 5322)"),
        (status = 404, description = "plain text: account not found or message not cached"),
        (status = 500, description = "plain text: database, read or decompress error")
    )
)]
pub async fn raw_email(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<RawEmailQuery>,
) -> impl IntoResponse {
    match email_db::get_account(
        &state.db,
        &q.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[pgp] raw_email account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    let path = eml_path(&state.data_dir, &q.account_id, &q.mailbox, q.uid);
    let compressed = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::NOT_FOUND, "message not cached").into_response();
        }
        Err(e) => {
            eprintln!("[pgp] raw_email read: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "read error").into_response();
        }
    };

    let raw = match zstd::decode_all(compressed.as_slice()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[pgp] raw_email decompress: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "decompress error").into_response();
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "message/rfc822")
        .body(Body::from(raw))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// This module's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(register_start))
        .routes(routes!(register_finish))
        .routes(routes!(authenticate_start))
        .routes(routes!(authenticate_finish))
        .routes(routes!(list_credentials))
        .routes(routes!(delete_credential))
        .routes(routes!(get_vault, update_vault, reset_vault))
        .routes(routes!(initialize_vault))
        .routes(routes!(create_pending_device))
        .routes(routes!(list_public_keys))
        .routes(routes!(save_public_key, delete_public_key))
        .routes(routes!(raw_email))
}

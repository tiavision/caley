use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use tower_sessions::Session;
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

// ── Register start ────────────────────────────────────────────────────────────

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

#[derive(Deserialize)]
pub struct RegisterFinishBody {
    pub credential: serde_json::Value,
    pub device_label: Option<String>,
    pub has_prf: Option<bool>,
}

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

    match state.webauthn.finish_passkey_registration(&reg_cred, &reg_state) {
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
            Json(json!({ "cred_id": cred_id })).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] finish_passkey_registration: {e:?}");
            (StatusCode::BAD_REQUEST, "registration failed").into_response()
        }
    }
}

// ── Authenticate start ────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuthStartBody {
    pub prf_salt: String,
    pub credential_id: Option<String>,
}

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
        .filter(|r| body.credential_id.as_ref().map_or(true, |id| id == &r.cred_id))
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

    match state.webauthn.finish_passkey_authentication(&auth_cred, &auth_state) {
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
            Json(json!({ "cred_id": matched_id })).into_response()
        }
        Err(e) => {
            eprintln!("[pgp] finish_passkey_authentication: {e:?}");
            (StatusCode::UNAUTHORIZED, "authentication failed").into_response()
        }
    }
}

// ── List credentials ─────────────────────────────────────────────────────────

pub async fn list_credentials(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match db::get_webauthn_credentials(&state.db, &user.user_id).await {
        Ok(rows) => {
            let list: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "cred_id": r.cred_id,
                        "device_label": r.device_label,
                        "has_prf": r.has_prf,
                    })
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

#[derive(Deserialize)]
pub struct DeleteCredentialBody {
    pub cred_id: String,
}

pub async fn delete_credential(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<DeleteCredentialBody>,
) -> impl IntoResponse {
    match db::delete_webauthn_credential(&state.db, &body.cred_id, &user.user_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[pgp] delete_credential: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Synced encrypted private-key vault ───────────────────────────────────────

#[derive(Deserialize, Serialize)]
pub struct EncryptedBlob {
    pub iv: String,
    pub ciphertext: String,
}

#[derive(Deserialize, Serialize)]
pub struct PrivateKeyBlob {
    pub fingerprint: String,
    pub iv: String,
    pub ciphertext: String,
}

#[derive(Deserialize, Serialize)]
pub struct DeviceBundle {
    pub cred_id: String,
    pub public_jwk: serde_json::Value,
    pub encrypted_private_jwk: String,
    pub private_iv: String,
    pub prf_salt: String,
    pub hkdf_salt: String,
    pub pairing_nonce: String,
}

#[derive(Deserialize, Serialize)]
pub struct VaultWrap {
    pub cred_id: String,
    pub ephemeral_public_jwk: serde_json::Value,
    pub hkdf_salt: String,
    pub iv: String,
    pub ciphertext: String,
}

#[derive(Deserialize)]
pub struct InitializeVaultBody {
    pub manifest: EncryptedBlob,
    pub key: PrivateKeyBlob,
    pub device: DeviceBundle,
    pub wrap: VaultWrap,
}

#[derive(Deserialize)]
pub struct PendingDeviceBody {
    pub device: DeviceBundle,
}

#[derive(Deserialize)]
pub struct ApproveDeviceBody {
    pub cred_id: String,
    pub wrap: VaultWrap,
}

#[derive(Deserialize)]
pub struct UpdateVaultBody {
    pub expected_revision: i64,
    pub manifest: EncryptedBlob,
    #[serde(default)]
    pub upsert_keys: Vec<PrivateKeyBlob>,
    #[serde(default)]
    pub delete_keys: Vec<String>,
    pub approve_device: Option<ApproveDeviceBody>,
    pub remove_device: Option<String>,
    pub replace_wraps: Option<Vec<VaultWrap>>,
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
            .map(|r| json!({ "fingerprint": r.get::<String, _>(0), "iv": r.get::<String, _>(1), "ciphertext": r.get::<String, _>(2) }))
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
                let envelope = r.get::<Option<String>, _>(9).map(|jwk| json!({
                    "cred_id": r.get::<String, _>(0),
                    "ephemeral_public_jwk": serde_json::from_str::<serde_json::Value>(&jwk).unwrap_or(json!(null)),
                    "hkdf_salt": r.get::<Option<String>, _>(10).unwrap_or_default(),
                    "iv": r.get::<Option<String>, _>(11).unwrap_or_default(),
                    "ciphertext": r.get::<Option<String>, _>(12).unwrap_or_default(),
                }));
                json!({
                    "cred_id": r.get::<String, _>(0),
                    "device_label": r.get::<Option<String>, _>(1),
                    "status": r.get::<String, _>(2),
                    "public_jwk": serde_json::from_str::<serde_json::Value>(&r.get::<String, _>(3)).unwrap_or(json!(null)),
                    "encrypted_private_jwk": r.get::<String, _>(4),
                    "private_iv": r.get::<String, _>(5),
                    "prf_salt": r.get::<String, _>(6),
                    "hkdf_salt": r.get::<String, _>(7),
                    "pairing_nonce": r.get::<String, _>(8),
                    "wrap": envelope,
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            eprintln!("[pgp] get vault devices: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    Json(json!({
        "revision": vault.get::<i64, _>(0),
        "manifest": { "iv": vault.get::<String, _>(1), "ciphertext": vault.get::<String, _>(2) },
        "keys": keys,
        "devices": devices,
    }))
    .into_response()
}

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
        Ok(_) => StatusCode::CREATED.into_response(),
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

pub async fn create_pending_device(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<PendingDeviceBody>,
) -> impl IntoResponse {
    if !owns_credential(&state, &user.user_id, &body.device.cred_id).await {
        return (StatusCode::BAD_REQUEST, "invalid device credential").into_response();
    }
    let has_vault = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pgp_vaults WHERE user_id = ?")
        .bind(&user.user_id).fetch_one(&state.db).await.unwrap_or(0) > 0;
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
            let _ = sqlx::query("UPDATE webauthn_credentials SET has_prf = 1 WHERE cred_id = ? AND user_id = ?")
                .bind(&body.device.cred_id).bind(&user.user_id).execute(&state.db).await;
            StatusCode::CREATED.into_response()
        }
        Err(e) => {
            eprintln!("[pgp] pending device: {e}");
            (StatusCode::CONFLICT, "device already registered").into_response()
        }
    }
}

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
            sqlx::query("UPDATE pgp_devices SET status = 'active' WHERE user_id = ? AND cred_id = ? AND status = 'pending'")
                .bind(&user.user_id).bind(&approval.cred_id).execute(&mut *tx).await?;
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
        Ok(_) => Json(json!({ "revision": body.expected_revision + 1 })).into_response(),
        Err(e) => {
            eprintln!("[pgp] apply vault update: {e}");
            (StatusCode::BAD_REQUEST, "invalid vault update").into_response()
        }
    }
}

// ── Trusted PGP public keys ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PublicKeysQuery {
    pub email: Option<String>,
}

pub async fn list_public_keys(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<PublicKeysQuery>,
) -> impl IntoResponse {
    match db::get_pgp_public_keys(&state.db, &user.user_id, q.email.as_deref()).await {
        Ok(rows) => {
            let list: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|row| {
                    json!({
                        "fingerprint": row.fingerprint,
                        "email": row.email,
                        "armored": row.armored,
                        "is_own": row.is_own,
                        "source": row.source,
                        "trust": row.trust,
                    })
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

#[derive(Deserialize)]
pub struct SavePublicKeyBody {
    pub fingerprint: String,
    pub email: String,
    pub armored: String,
    pub is_own: Option<bool>,
    pub source: Option<String>,
}

pub async fn save_public_key(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<SavePublicKeyBody>,
) -> impl IntoResponse {
    if body.fingerprint.trim().is_empty()
        || !body.armored.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----")
    {
        return (StatusCode::BAD_REQUEST, "invalid public key").into_response();
    }

    let source = body.source.as_deref().unwrap_or("imported");
    if let Err(e) = db::upsert_pgp_public_key(
        &state.db,
        &user.user_id,
        &body.fingerprint,
        &body.email,
        &body.armored,
        body.is_own.unwrap_or(false),
        source,
        "trusted",
    )
    .await
    {
        eprintln!("[pgp] save public key: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
pub struct DeletePublicKeyBody {
    pub fingerprint: String,
}

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

#[derive(Deserialize)]
pub struct RawEmailQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

pub async fn raw_email(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<RawEmailQuery>,
) -> impl IntoResponse {
    match email_db::get_account(&state.db, &q.account_id, &user.user_id, &state.encryption_key).await {
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

    let raw = {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut dec = GzDecoder::new(compressed.as_slice());
        let mut out = Vec::new();
        if let Err(e) = dec.read_to_end(&mut out) {
            eprintln!("[pgp] raw_email decompress: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "decompress error").into_response();
        }
        out
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "message/rfc822")
        .body(Body::from(raw))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

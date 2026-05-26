use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::json;
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

fn inject_prf_registration(challenge: &mut serde_json::Value) {
    if let Some(pk) = challenge.get_mut("publicKey") {
        if let Some(obj) = pk.as_object_mut() {
            obj.insert("extensions".to_owned(), json!({ "prf": {} }));
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
            inject_prf_registration(&mut challenge);
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
            StatusCode::OK.into_response()
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

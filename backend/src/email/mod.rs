use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::email_db;
use crate::email_sync::EmailSyncTrigger;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

pub(crate) mod attachments;
pub(crate) mod messages;
pub(crate) mod send;

pub use attachments::{
    add_image_blacklist_origin, add_image_origin, download_attachment, export_account,
    list_image_blacklist, list_image_origins, navigate, proxy_image, remove_image_blacklist_origin,
    remove_image_origin,
};
pub use messages::{
    delete_message, empty_trash, get_message, get_thread, list_messages, move_message, search,
    suggest_addrs, update_flags,
};
pub use send::{autodiscover_handler, send_raw_email};

// ── Accounts ──────────────────────────────────────────────────────────────────

pub async fn list_accounts(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    match email_db::list_accounts(&state.db, &user.user_id, &state.encryption_key).await {
        Ok(accounts) => {
            let list: Vec<_> = accounts
                .iter()
                .map(|a| {
                    json!({
                        "account_id": a.account_id,
                        "display_name": a.display_name,
                        "host": a.host,
                        "port": a.port,
                        "username": a.username,
                        "use_tls": a.use_tls,
                        "smtp_host": a.smtp_host,
                        "smtp_port": a.smtp_port,
                        "sort_order": a.sort_order,
                    })
                })
                .collect();
            Json(json!({ "accounts": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_accounts error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct AccountBody {
    pub account_id: Option<String>,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub use_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub sender_name: Option<String>,
}

pub async fn save_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<AccountBody>,
) -> impl IntoResponse {
    let is_new = body.account_id.is_none();
    let account_id = body
        .account_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let use_tls = body.use_tls.unwrap_or(true);

    let password = match body.password.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => {
            match email_db::get_account(
                &state.db,
                &account_id,
                &user.user_id,
                &state.encryption_key,
            )
            .await
            {
                Ok(Some(existing)) => existing.password,
                Ok(None) => {
                    return (StatusCode::BAD_REQUEST, "password required for new account")
                        .into_response();
                }
                Err(e) => {
                    eprintln!("[email] save_account lookup error: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
                }
            }
        }
    };

    if let Err(e) = email_db::upsert_account(
        &state.db,
        &user.user_id,
        &account_id,
        &body.display_name,
        &body.host,
        body.port,
        &body.username,
        &password,
        use_tls,
        body.smtp_host.as_deref(),
        body.smtp_port,
        &state.encryption_key,
    )
    .await
    {
        eprintln!("[email] save_account error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    if is_new {
        if let Some(name) = body.sender_name.filter(|n| !n.is_empty()) {
            if let Err(e) = email_db::upsert_identity(
                &state.db,
                &Uuid::new_v4().to_string(),
                &account_id,
                &user.user_id,
                &name,
                &body.username,
                true,
            )
            .await
            {
                eprintln!("[email] save_account: failed to create default identity: {e}");
            }
        }
    }

    state
        .sse_broadcaster
        .send(&user.user_id, SseMessage::AccountsChanged);
    Json(json!({ "account_id": account_id })).into_response()
}

#[derive(Deserialize)]
pub struct AccountIdQuery {
    pub account_id: String,
}

pub async fn delete_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<AccountIdQuery>,
) -> impl IntoResponse {
    // Delete all .eml files for this account
    let messages_dir = state.data_dir.join("messages").join(&q.account_id);
    let _ = tokio::fs::remove_dir_all(&messages_dir).await;

    state.email_sync_manager.stop_idle_for(&q.account_id);
    state.email_sync_manager.stop_task_for(&q.account_id);

    if let Err(e) = email_db::delete_account(&state.db, &q.account_id, &user.user_id).await {
        eprintln!("[email] delete_account error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    state
        .sse_broadcaster
        .send(&user.user_id, SseMessage::AccountsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ── Identities ───────────────────────────────────────────────────────────────

pub async fn list_identities(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::list_identities(&state.db, &user.user_id).await {
        Ok(rows) => {
            let list: Vec<_> = rows
                .iter()
                .map(|i| {
                    json!({
                        "identity_id": i.identity_id,
                        "account_id":  i.account_id,
                        "name":        i.name,
                        "email":       i.email,
                        "is_default":  i.is_default,
                        "sort_order":  i.sort_order,
                    })
                })
                .collect();
            Json(json!({ "identities": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_identities error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct IdentityBody {
    pub identity_id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub email: String,
    pub is_default: Option<bool>,
}

pub async fn save_identity(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<IdentityBody>,
) -> impl IntoResponse {
    // Verify the account belongs to this user
    match email_db::get_account(
        &state.db,
        &body.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] save_identity account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    let identity_id = body
        .identity_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let is_default = body.is_default.unwrap_or(false);

    if let Err(e) = email_db::upsert_identity(
        &state.db,
        &identity_id,
        &body.account_id,
        &user.user_id,
        &body.name,
        &body.email,
        is_default,
    )
    .await
    {
        eprintln!("[email] save_identity error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    Json(json!({ "identity_id": identity_id })).into_response()
}

#[derive(Deserialize)]
pub struct IdentityIdQuery {
    pub identity_id: String,
}

pub async fn delete_identity(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<IdentityIdQuery>,
) -> impl IntoResponse {
    if let Err(e) = email_db::delete_identity(&state.db, &q.identity_id, &user.user_id).await {
        eprintln!("[email] delete_identity error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

// ── Mailboxes ─────────────────────────────────────────────────────────────────

pub async fn list_mailboxes(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<AccountIdQuery>,
) -> impl IntoResponse {
    // Verify the account belongs to this user
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
            eprintln!("[email] list_mailboxes account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    match email_db::list_mailboxes(&state.db, &q.account_id).await {
        Ok(mbs) => {
            let list: Vec<_> = mbs
                .iter()
                .map(|m| {
                    json!({
                        "account_id": m.account_id,
                        "name": m.name,
                        "delimiter": m.delimiter,
                        "subscribed": m.subscribed,
                    })
                })
                .collect();
            Json(json!({ "mailboxes": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_mailboxes error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Sync trigger ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MailboxSyncQuery {
    pub account_id: String,
    pub mailbox: String,
}

pub async fn trigger_mailbox_sync(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<MailboxSyncQuery>,
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
            eprintln!("[email] trigger_mailbox_sync account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    state
        .email_sync_manager
        .trigger(&q.account_id, EmailSyncTrigger::Mailbox(q.mailbox));
    StatusCode::OK.into_response()
}

// ── Virtual Folders ───────────────────────────────────────────────────────────

pub async fn list_virtual_folders(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::list_virtual_folders(&state.db, &user.user_id).await {
        Ok(folders) => {
            let list: Vec<_> = folders
                .iter()
                .map(|f| {
                    json!({
                        "folder_id": f.folder_id,
                        "display_name": f.display_name,
                        "sort_order": f.sort_order,
                        "only_flagged": f.only_flagged,
                        "only_attachments": f.only_attachments,
                        "rules": f.rules.iter().map(|r| json!({
                            "account_id": r.account_id,
                            "mailbox_pattern": r.mailbox_pattern,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Json(json!({ "folders": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_virtual_folders error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

pub async fn get_unread_counts(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    let mb = email_db::get_unread_counts(&state.db, &user.user_id)
        .await
        .unwrap_or_default();
    let folders = email_db::get_unread_counts_virtual(&state.db, &user.user_id)
        .await
        .unwrap_or_default();

    let mut mailboxes_map: serde_json::Map<String, serde_json::Value> = Default::default();
    for (acc, mbox, cnt) in mb {
        mailboxes_map
            .entry(acc)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(mbox, json!(cnt));
    }
    let folders_map: serde_json::Map<String, serde_json::Value> = folders
        .into_iter()
        .map(|(id, cnt)| (id, json!(cnt)))
        .collect();

    Json(json!({ "mailboxes": mailboxes_map, "folders": folders_map })).into_response()
}

#[derive(Deserialize)]
pub struct VirtualFolderRuleBody {
    pub account_id: Option<String>,
    pub mailbox_pattern: Option<String>,
}

#[derive(Deserialize)]
pub struct VirtualFolderBody {
    pub display_name: String,
    pub rules: Vec<VirtualFolderRuleBody>,
    pub only_flagged: Option<bool>,
    pub only_attachments: Option<bool>,
}

pub async fn create_virtual_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<VirtualFolderBody>,
) -> impl IntoResponse {
    let folder_id = Uuid::new_v4().to_string();
    let rules: Vec<(Option<&str>, Option<&str>)> = body
        .rules
        .iter()
        .map(|r| (r.account_id.as_deref(), r.mailbox_pattern.as_deref()))
        .collect();

    if let Err(e) = email_db::create_virtual_folder(
        &state.db,
        &folder_id,
        &user.user_id,
        &body.display_name,
        &rules,
        body.only_flagged.unwrap_or(false),
        body.only_attachments.unwrap_or(false),
    )
    .await
    {
        eprintln!("[email] create_virtual_folder error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    Json(json!({ "folder_id": folder_id })).into_response()
}

#[derive(Deserialize)]
pub struct FolderIdQuery {
    pub folder_id: String,
}

pub async fn update_virtual_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<FolderIdQuery>,
    Json(body): Json<VirtualFolderBody>,
) -> impl IntoResponse {
    let rules: Vec<(Option<&str>, Option<&str>)> = body
        .rules
        .iter()
        .map(|r| (r.account_id.as_deref(), r.mailbox_pattern.as_deref()))
        .collect();

    match email_db::update_virtual_folder(
        &state.db,
        &q.folder_id,
        &user.user_id,
        &body.display_name,
        &rules,
        body.only_flagged.unwrap_or(false),
        body.only_attachments.unwrap_or(false),
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.to_string() == "not found" => {
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
        Err(e) => {
            eprintln!("[email] update_virtual_folder error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

pub async fn delete_virtual_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<FolderIdQuery>,
) -> impl IntoResponse {
    if let Err(e) = email_db::delete_virtual_folder(&state.db, &q.folder_id, &user.user_id).await {
        eprintln!("[email] delete_virtual_folder error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::email_db;
use crate::imap_client;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

// ── Accounts ──────────────────────────────────────────────────────────────────

pub async fn list_accounts(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::list_accounts(&state.db, &user.user_id, &state.encryption_key).await {
        Ok(accounts) => {
            let list: Vec<_> = accounts
                .iter()
                .map(|a| json!({
                    "account_id": a.account_id,
                    "display_name": a.display_name,
                    "host": a.host,
                    "port": a.port,
                    "username": a.username,
                    "use_tls": a.use_tls,
                    "smtp_host": a.smtp_host,
                    "smtp_port": a.smtp_port,
                    "sort_order": a.sort_order,
                }))
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
    pub password: String,
    pub use_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
}

pub async fn save_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<AccountBody>,
) -> impl IntoResponse {
    let account_id = body.account_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let use_tls = body.use_tls.unwrap_or(true);

    // Validate by attempting to connect
    if let Err(e) = imap_client::connect(&body.host, body.port, &body.username, &body.password).await {
        return (StatusCode::BAD_REQUEST, format!("IMAP connection failed: {e}")).into_response();
    }

    if let Err(e) = email_db::upsert_account(
        &state.db, &user.user_id, &account_id,
        &body.display_name, &body.host, body.port,
        &body.username, &body.password, use_tls,
        body.smtp_host.as_deref(), body.smtp_port,
        &state.encryption_key,
    ).await {
        eprintln!("[email] save_account error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    // Stop any existing IDLE watcher so it reconnects with updated credentials
    state.email_sync_manager.stop_idle_for(&account_id);
    // Kick off sync (and IDLE) for this account
    state.email_sync_manager.ensure_task_for(&account_id);
    state.sse_broadcaster.send(&user.user_id, SseMessage::AccountsChanged);

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
    if let Err(e) = email_db::delete_account(&state.db, &q.account_id, &user.user_id).await {
        eprintln!("[email] delete_account error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    state.email_sync_manager.stop_idle_for(&q.account_id);
    state.email_sync_manager.stop_task_for(&q.account_id);
    state.sse_broadcaster.send(&user.user_id, SseMessage::AccountsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ── Mailboxes ─────────────────────────────────────────────────────────────────

pub async fn list_mailboxes(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<AccountIdQuery>,
) -> impl IntoResponse {
    // Verify the account belongs to this user
    match email_db::get_account(&state.db, &q.account_id, &user.user_id, &state.encryption_key).await {
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
                .map(|m| json!({
                    "account_id": m.account_id,
                    "name": m.name,
                    "delimiter": m.delimiter,
                    "subscribed": m.subscribed,
                }))
                .collect();
            Json(json!({ "mailboxes": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_mailboxes error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
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
                .map(|f| json!({
                    "folder_id": f.folder_id,
                    "display_name": f.display_name,
                    "sort_order": f.sort_order,
                    "rules": f.rules.iter().map(|r| json!({
                        "account_id": r.account_id,
                        "mailbox_pattern": r.mailbox_pattern,
                    })).collect::<Vec<_>>(),
                }))
                .collect();
            Json(json!({ "folders": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_virtual_folders error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
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
        &state.db, &folder_id, &user.user_id, &body.display_name, &rules,
    ).await {
        eprintln!("[email] create_virtual_folder error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }
    Json(json!({ "folder_id": folder_id })).into_response()
}

#[derive(Deserialize)]
pub struct FolderIdQuery {
    pub folder_id: String,
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

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListMessagesQuery {
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub folder_id: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListMessagesQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let result = if let Some(folder_id) = q.folder_id {
        email_db::list_messages_virtual(&state.db, &user.user_id, &folder_id, limit, offset).await
    } else {
        let account_id = match q.account_id {
            Some(id) => id,
            None => return (StatusCode::BAD_REQUEST, "account_id or folder_id required").into_response(),
        };
        let mailbox = match q.mailbox {
            Some(mb) => mb,
            None => return (StatusCode::BAD_REQUEST, "mailbox required").into_response(),
        };
        email_db::list_messages(&state.db, &account_id, &mailbox, limit, offset).await
    };

    match result {
        Ok((messages, total)) => {
            let list: Vec<_> = messages.iter().map(message_to_json).collect();
            Json(json!({ "messages": list, "total": total })).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_messages error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct GetMessageQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

pub async fn get_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<GetMessageQuery>,
) -> impl IntoResponse {
    // Verify account belongs to user
    match email_db::get_account(&state.db, &q.account_id, &user.user_id, &state.encryption_key).await {
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] get_message account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(account)) => {
            // Fetch body from IMAP if not cached
            let cached = email_db::get_message(&state.db, &q.account_id, &q.mailbox, q.uid)
                .await
                .unwrap_or(None);

            let needs_body = cached.as_ref().map_or(true, |m| m.body_text.is_none());
            if needs_body {
                match imap_client::connect(&account.host, account.port as u16, &account.username, &account.password).await {
                    Ok(mut session) => {
                        if imap_client::select_mailbox(&mut session, &q.mailbox).await.is_ok() {
                            match imap_client::fetch_body(&mut session, q.uid).await {
                                Ok((body_text, _has_att)) => {
                                    let body_idx = state.search_mode.normalize_text(&body_text);
                                    let _ = email_db::update_message_body(
                                        &state.db, &q.account_id, &q.mailbox, q.uid, &body_text, &body_idx,
                                    ).await;
                                }
                                Err(e) => eprintln!("[email] fetch_body error: {e}"),
                            }
                        }
                        let _ = session.logout().await;
                    }
                    Err(e) => eprintln!("[email] connect for body fetch error: {e}"),
                }
            }
        }
    }

    match email_db::get_message(&state.db, &q.account_id, &q.mailbox, q.uid).await {
        Ok(Some(msg)) => Json(message_to_json(&msg)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "message not found").into_response(),
        Err(e) => {
            eprintln!("[email] get_message error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct FlagsBody {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    pub flags: Vec<String>,
    pub set: bool,
}

pub async fn update_flags(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<FlagsBody>,
) -> impl IntoResponse {
    let account = match email_db::get_account(&state.db, &body.account_id, &user.user_id, &state.encryption_key).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] update_flags account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    // Apply on server
    match imap_client::connect(&account.host, account.port as u16, &account.username, &account.password).await {
        Ok(mut session) => {
            if let Ok(_) = imap_client::select_mailbox(&mut session, &body.mailbox).await {
                let flag_strs: Vec<&str> = body.flags.iter().map(|s| s.as_str()).collect();
                if let Err(e) = imap_client::set_flags(&mut session, body.uid, &flag_strs, body.set).await {
                    eprintln!("[email] set_flags error: {e}");
                }
            }
            let _ = session.logout().await;
        }
        Err(e) => eprintln!("[email] connect for flags error: {e}"),
    }

    // Update cached flags
    let mut current_flags: Vec<String> = email_db::get_message(&state.db, &body.account_id, &body.mailbox, body.uid)
        .await
        .ok()
        .flatten()
        .and_then(|m| m.flags)
        .and_then(|f| serde_json::from_str(&f).ok())
        .unwrap_or_default();

    if body.set {
        for f in &body.flags {
            if !current_flags.contains(f) {
                current_flags.push(f.clone());
            }
        }
    } else {
        current_flags.retain(|f| !body.flags.contains(f));
    }

    let flags_json = serde_json::to_string(&current_flags).unwrap_or_default();
    let _ = email_db::update_message_flags(&state.db, &body.account_id, &body.mailbox, body.uid, &flags_json).await;

    state.sse_broadcaster.send(&user.user_id, SseMessage::EmailUpdated {
        account_id: body.account_id.clone(),
        uid: body.uid,
        mailbox: body.mailbox.clone(),
        flags: current_flags,
    });

    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
pub struct DeleteMessageQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

pub async fn delete_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<DeleteMessageQuery>,
) -> impl IntoResponse {
    let account = match email_db::get_account(&state.db, &q.account_id, &user.user_id, &state.encryption_key).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] delete_message account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    match imap_client::connect(&account.host, account.port as u16, &account.username, &account.password).await {
        Ok(mut session) => {
            if let Ok(_) = imap_client::select_mailbox(&mut session, &q.mailbox).await {
                if let Err(e) = imap_client::delete_on_server(&mut session, q.uid).await {
                    eprintln!("[email] delete_on_server error: {e}");
                }
            }
            let _ = session.logout().await;
        }
        Err(e) => eprintln!("[email] connect for delete error: {e}"),
    }

    let _ = email_db::delete_message(&state.db, &q.account_id, &q.mailbox, q.uid).await;

    state.sse_broadcaster.send(&user.user_id, SseMessage::EmailRemoved {
        account_id: q.account_id,
        uid: q.uid,
        mailbox: q.mailbox,
    });

    StatusCode::NO_CONTENT.into_response()
}

// ── Send ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SendBody {
    pub account_id: String,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
}

pub async fn send_email(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<SendBody>,
) -> impl IntoResponse {
    let account = match email_db::get_account(&state.db, &body.account_id, &user.user_id, &state.encryption_key).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] send_email account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let smtp_host = account.smtp_host.as_deref().unwrap_or(&account.host);
    let smtp_port = account.smtp_port.unwrap_or(587) as u16;
    let from = format!("{} <{}>", user.display_name.as_deref().unwrap_or(&account.username), account.username);

    if let Err(e) = imap_client::send_email(
        smtp_host, smtp_port,
        &account.username, &account.password,
        &from, &body.to, &body.subject, &body.body,
        body.in_reply_to.as_deref(),
    ).await {
        eprintln!("[email] send_email error: {e}");
        return (StatusCode::BAD_GATEWAY, format!("Failed to send: {e}")).into_response();
    }

    StatusCode::OK.into_response()
}

// ── Search ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

pub async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    if q.q.trim().is_empty() {
        return Json(json!({ "messages": [] })).into_response();
    }
    let fts_query = state.search_mode.build_fts_query(&q.q);
    let limit = q.limit.unwrap_or(50).min(200);

    match email_db::search_messages(&state.db, &user.user_id, &fts_query, limit).await {
        Ok(messages) => {
            let list: Vec<_> = messages.iter().map(message_to_json).collect();
            Json(json!({ "messages": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] search error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "search error").into_response()
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn message_to_json(m: &email_db::MessageRow) -> serde_json::Value {
    let to: Vec<String> = m.to_addrs.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let cc: Vec<String> = m.cc_addrs.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let flags: Vec<String> = m.flags.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    json!({
        "uid": m.uid,
        "account_id": m.account_id,
        "mailbox": m.mailbox,
        "message_id": m.message_id,
        "thread_id": m.thread_id,
        "from": m.from_addr,
        "to": to,
        "cc": cc,
        "subject": m.subject,
        "sent_at": m.sent_at,
        "flags": flags,
        "has_attachments": m.has_attachments,
        "body_text": m.body_text,
        "is_read": flags.iter().any(|f| f == "\\Seen"),
        "is_flagged": flags.iter().any(|f| f == "\\Flagged"),
    })
}

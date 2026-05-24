use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Json, Response},
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
    // Delete attachment files before the DB transaction removes the records
    if let Ok(paths) = email_db::get_attachment_paths_for_account(&state.db, &q.account_id).await {
        for path in paths {
            let _ = std::fs::remove_file(&path);
        }
    }

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
        Ok(Some(_)) => {}
    }

    match email_db::get_message(&state.db, &q.account_id, &q.mailbox, q.uid).await {
        Ok(Some(msg)) => {
            let attachments = email_db::list_attachments(&state.db, &q.account_id, &q.mailbox, q.uid)
                .await
                .unwrap_or_default();
            Json(message_to_json_with_attachments(&msg, &attachments)).into_response()
        }
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

    // Delete attachment files from disk before removing DB rows
    if let Ok(attachments) = email_db::delete_attachments_for_message(&state.db, &q.account_id, &q.mailbox, q.uid).await {
        for att in attachments {
            let _ = std::fs::remove_file(&att.fs_path);
        }
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

pub async fn send_email(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let mut account_id = String::new();
    let mut to_raw = String::new();
    let mut cc_raw = String::new();
    let mut bcc_raw = String::new();
    let mut subject = String::new();
    let mut body = String::new();
    let mut in_reply_to: Option<String> = None;
    let mut attachments: Vec<(String, String, Vec<u8>)> = vec![];

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_owned();
                if name == "attachment" {
                    let filename = field.file_name().unwrap_or("attachment").to_owned();
                    let ct = field.content_type()
                        .map(|s| s.to_owned())
                        .unwrap_or_else(|| "application/octet-stream".to_owned());
                    match field.bytes().await {
                        Ok(b) if !b.is_empty() => attachments.push((filename, ct, b.to_vec())),
                        _ => {}
                    }
                } else {
                    let text = match field.text().await {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    match name.as_str() {
                        "account_id" => account_id = text,
                        "to" => to_raw = text,
                        "cc" => cc_raw = text,
                        "bcc" => bcc_raw = text,
                        "subject" => subject = text,
                        "body" => body = text,
                        "in_reply_to" if !text.is_empty() => in_reply_to = Some(text),
                        _ => {}
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                eprintln!("[email] multipart error: {e}");
                return (StatusCode::BAD_REQUEST, "invalid multipart data").into_response();
            }
        }
    }

    let split_addrs = |s: &str| -> Vec<String> {
        s.split(',').map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()).collect()
    };
    let to = split_addrs(&to_raw);
    let cc = split_addrs(&cc_raw);
    let bcc = split_addrs(&bcc_raw);

    if account_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing account_id").into_response();
    }

    let account = match email_db::get_account(&state.db, &account_id, &user.user_id, &state.encryption_key).await {
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
        &from, &to, &cc, &bcc, &subject, &body,
        in_reply_to.as_deref(),
        attachments,
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
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub folder_id: Option<String>,
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

    let result = if let Some(folder_id) = q.folder_id.as_deref() {
        email_db::search_messages_virtual(&state.db, &user.user_id, folder_id, &fts_query, limit).await
    } else if let (Some(account_id), Some(mailbox)) = (q.account_id.as_deref(), q.mailbox.as_deref()) {
        email_db::search_messages_mailbox(&state.db, &user.user_id, account_id, mailbox, &fts_query, limit).await
    } else {
        email_db::search_messages(&state.db, &user.user_id, &fts_query, limit).await
    };

    match result {
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
    message_to_json_with_attachments(m, &[])
}

fn message_to_json_with_attachments(
    m: &email_db::MessageRow,
    attachments: &[email_db::AttachmentRow],
) -> serde_json::Value {
    let to: Vec<String> = m.to_addrs.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let cc: Vec<String> = m.cc_addrs.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let flags: Vec<String> = m.flags.as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let att_list: Vec<serde_json::Value> = attachments.iter().map(|a| {
        json!({
            "filename": a.filename,
            "content_type": a.content_type,
            "size": a.size,
        })
    }).collect();

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
        "body_html": m.body_html,
        "is_read": flags.iter().any(|f| f == "\\Seen"),
        "is_flagged": flags.iter().any(|f| f == "\\Flagged"),
        "attachments": att_list,
    })
}

// ── Image proxy ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ProxyImageQuery {
    pub url: Option<String>,
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub uid: Option<u32>,
    pub cid: Option<String>,
}

pub async fn proxy_image(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ProxyImageQuery>,
) -> Response {
    // CID-based inline attachment lookup
    if let (Some(cid), Some(account_id), Some(mailbox), Some(uid)) = (&q.cid, &q.account_id, &q.mailbox, q.uid) {
        // Verify the account belongs to the current user before serving its data.
        match email_db::get_account(&state.db, account_id, &user.user_id, &state.encryption_key).await {
            Ok(Some(_)) => {}
            _ => return transparent_pixel(),
        }
        let att = match email_db::get_attachment_by_cid(&state.db, account_id, mailbox, uid, cid).await {
            Ok(Some(a)) => a,
            _ => return transparent_pixel(),
        };

        let bytes = match tokio::fs::read(&att.fs_path).await {
            Ok(b) => b,
            Err(_) => return transparent_pixel(),
        };

        // Only serve inline attachments as images. The content type comes straight
        // from the (untrusted) MIME part; serving e.g. text/html same-origin would
        // be a stored-XSS vector if the resource URL were opened directly.
        let ct = att.content_type.unwrap_or_else(|| "application/octet-stream".to_string());
        let ct_lower = ct.to_ascii_lowercase();
        if !ct_lower.starts_with("image/") || ct_lower.contains("svg") {
            return transparent_pixel();
        }
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, ct),
                (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
                (header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
            ],
            bytes,
        )
            .into_response();
    }

    // External URL proxy (original behaviour)
    let Some(url) = q.url else {
        return (StatusCode::BAD_REQUEST, "invalid url").into_response();
    };

    let parsed = match url::Url::parse(&url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => return (StatusCode::BAD_REQUEST, "invalid url").into_response(),
    };

    if !is_ssrf_safe(&parsed).await {
        return transparent_pixel();
    }

    let Some(host) = parsed.host_str() else {
        return transparent_pixel();
    };
    let origin = format!("{}://{}", parsed.scheme(), host);

    match email_db::is_origin_whitelisted(&state.db, &user.user_id, &origin).await {
        Ok(true) => {}
        _ => return transparent_pixel(),
    }

    // Do not follow redirects: a whitelisted origin could otherwise 302 to an
    // internal address (e.g. 127.0.0.1 / link-local metadata) that the pre-fetch
    // is_ssrf_safe() check never sees.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(_) => return transparent_pixel(),
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return transparent_pixel(),
    };

    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if !ct.starts_with("image/") || ct.contains("svg") {
        return transparent_pixel();
    }

    let ct_owned = ct.clone();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(_) => return transparent_pixel(),
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ct_owned),
            (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response()
}

// ── Attachment download ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DownloadAttachmentQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    pub filename: String,
}

pub async fn download_attachment(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<DownloadAttachmentQuery>,
) -> Response {
    // Verify account belongs to user
    match email_db::get_account(&state.db, &q.account_id, &user.user_id, &state.encryption_key).await {
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] download_attachment account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    let att = match email_db::get_attachment_by_filename(&state.db, &q.account_id, &q.mailbox, q.uid, &q.filename).await {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "attachment not found").into_response(),
        Err(e) => {
            eprintln!("[email] download_attachment lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let bytes = match tokio::fs::read(&att.fs_path).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[email] download_attachment read error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "file read error").into_response();
        }
    };

    let ct = att.content_type.unwrap_or_else(|| "application/octet-stream".to_string());
    // Strip control chars / quotes so the (user-supplied) filename can't break out
    // of the header value or inject additional headers.
    let safe_name: String = q
        .filename
        .chars()
        .map(|c| if c.is_control() || c == '"' || c == '\\' { '_' } else { c })
        .collect();
    let disposition = format!("attachment; filename=\"{safe_name}\"");

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ct),
            (header::HeaderName::from_static("content-disposition"), disposition),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response()
}

async fn is_ssrf_safe(parsed: &url::Url) -> bool {
    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::net::lookup_host(format!("{host}:{port}")).await {
        Ok(mut addrs) => addrs.all(|a| !is_private_ip(a.ip())),
        Err(_) => false,
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_private_v4(v4),
        std::net::IpAddr::V6(v6) => {
            // IPv4-mapped addresses (::ffff:a.b.c.d) connect to the embedded IPv4
            // host, so apply the v4 rules — otherwise ::ffff:127.0.0.1 bypasses.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_v4(v4);
            }
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

fn is_private_v4(v4: std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_unspecified()
}

fn transparent_pixel() -> Response {
    // 1×1 transparent GIF
    const GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80,
        0x00, 0x00, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x21, 0xf9, 0x04,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ];
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/gif"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-content-type-options"), "nosniff"),
        ],
        GIF,
    )
        .into_response()
}

// ── Navigate (link warning page) ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct NavigateQuery {
    pub to: String,
}

pub async fn navigate(Query(q): Query<NavigateQuery>) -> Response {
    let url = &q.to;
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return (StatusCode::BAD_REQUEST, "invalid url").into_response(),
    }

    let esc = html_escape(url);
    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>External Link</title>
<style>
  body {{ font-family: sans-serif; max-width: 560px; margin: 3rem auto; padding: 0 1.5rem; color: #222; }}
  h2 {{ font-size: 1.1rem; margin-bottom: 1rem; }}
  .url {{ word-break: break-all; background: #f5f5f5; padding: .6rem .8rem; border-radius: 4px; margin: 1rem 0; font-size: .9rem; }}
  .actions {{ display: flex; gap: .75rem; margin-top: 1.5rem; }}
  .continue {{ background: #2a6496; color: #fff; padding: .55rem 1.1rem; border-radius: 4px; text-decoration: none; font-size: .9rem; }}
  .cancel {{ padding: .55rem 1.1rem; border-radius: 4px; text-decoration: none; font-size: .9rem; border: 1px solid #999; color: inherit; cursor: pointer; background: none; }}
</style>
</head>
<body>
<h2>You are about to visit an external site</h2>
<p>Destination:</p>
<div class="url">{esc}</div>
<div class="actions">
  <a class="continue" href="{esc}">Continue</a>
  <button class="cancel" onclick="window.close()">Cancel</button>
</div>
</body>
</html>"##
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ── Image origin whitelist API ────────────────────────────────────────────────

pub async fn list_image_origins(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::get_image_whitelist(&state.db, &user.user_id).await {
        Ok(origins) => Json(json!({ "origins": origins })).into_response(),
        Err(e) => {
            eprintln!("[email] list_image_origins error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct ImageOriginBody {
    pub origin: String,
}

pub async fn add_image_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ImageOriginBody>,
) -> impl IntoResponse {
    // Validate that the origin is a well-formed http(s) origin
    match url::Url::parse(&body.origin) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => {}
        _ => return (StatusCode::BAD_REQUEST, "invalid origin").into_response(),
    }
    // Normalise to scheme://host (strip path/query)
    let origin = {
        let u = url::Url::parse(&body.origin).unwrap();
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""))
    };

    match email_db::add_image_origin(&state.db, &user.user_id, &origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] add_image_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct ImageOriginQuery {
    pub origin: String,
}

pub async fn remove_image_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ImageOriginQuery>,
) -> impl IntoResponse {
    match email_db::remove_image_origin(&state.db, &user.user_id, &q.origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] remove_image_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

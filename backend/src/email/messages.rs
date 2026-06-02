use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use serde_json::json;

use crate::email_db;
use crate::email_sync::eml_path;
use crate::imap_client;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListMessagesQuery {
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub folder_id: Option<String>,
    pub limit: Option<i64>,
    pub cursor_sent_at: Option<i64>,
    pub cursor_rowid: Option<i64>,
    pub direction: Option<String>,
    pub offset: Option<i64>,
}

fn parse_direction(s: Option<&str>) -> email_db::PageDirection {
    match s {
        Some("backward") => email_db::PageDirection::Backward,
        Some("last") => email_db::PageDirection::Last,
        _ => email_db::PageDirection::Forward,
    }
}

fn parse_cursor(sent_at: Option<i64>, rowid: Option<i64>) -> Option<email_db::PageCursor> {
    rowid.map(|r| email_db::PageCursor { sent_at, rowid: r })
}

pub async fn list_messages(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ListMessagesQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    let cursor = parse_cursor(q.cursor_sent_at, q.cursor_rowid);
    let direction = parse_direction(q.direction.as_deref());
    let offset = q.offset;

    let result = if let Some(folder_id) = q.folder_id {
        email_db::list_messages_virtual(
            &state.db,
            &user.user_id,
            &folder_id,
            limit,
            cursor,
            direction,
            offset,
        )
        .await
    } else {
        let account_id = match q.account_id {
            Some(id) => id,
            None => {
                return (StatusCode::BAD_REQUEST, "account_id or folder_id required")
                    .into_response();
            }
        };
        let mailbox = match q.mailbox {
            Some(mb) => mb,
            None => return (StatusCode::BAD_REQUEST, "mailbox required").into_response(),
        };
        email_db::list_messages(
            &state.db,
            &account_id,
            &mailbox,
            limit,
            cursor,
            direction,
            offset,
        )
        .await
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
            eprintln!("[email] get_message account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    match email_db::get_message(&state.db, &q.account_id, &q.mailbox, q.uid).await {
        Ok(Some(msg)) => {
            let attachments =
                email_db::list_attachments(&state.db, &q.account_id, &q.mailbox, q.uid)
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

pub async fn get_thread(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<GetMessageQuery>,
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
            eprintln!("[email] get_thread account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    match email_db::list_thread_messages(&state.db, &q.account_id, &q.mailbox, q.uid).await {
        Ok(messages) => {
            let messages = dedupe_thread_messages(messages, &q.account_id, &q.mailbox, q.uid);
            let list: Vec<_> = messages.iter().map(thread_message_to_json).collect();
            Json(json!({ "messages": list })).into_response()
        }
        Err(e) => {
            eprintln!("[email] get_thread error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

fn dedupe_thread_messages(
    mut messages: Vec<email_db::MessageRow>,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Vec<email_db::MessageRow> {
    messages.sort_by_key(|m| (m.sent_at.is_none(), m.sent_at.unwrap_or_default(), m.rowid));
    let mut result = Vec::new();
    let mut logical_indices: HashMap<String, usize> = HashMap::new();
    for message in messages {
        let is_open_message = message.account_id == account_id
            && message.mailbox == mailbox
            && message.uid == uid as i64;
        if let Some(message_id) = message.message_id.clone() {
            if let Some(&index) = logical_indices.get(&message_id) {
                if is_open_message {
                    result[index] = message;
                }
                continue;
            }
            logical_indices.insert(message_id, result.len());
        }
        result.push(message);
    }
    result.sort_by_key(|m| (m.sent_at.is_none(), m.sent_at.unwrap_or_default(), m.rowid));
    result
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
    let account = match email_db::get_account(
        &state.db,
        &body.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] update_flags account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    // Apply on server (skipped in demo mode)
    if !state.demo_mode {
        match imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await
        {
            Ok(mut session) => {
                if let Ok(_) = imap_client::select_mailbox(&mut session, &body.mailbox).await {
                    let flag_strs: Vec<&str> = body.flags.iter().map(|s| s.as_str()).collect();
                    if let Err(e) =
                        imap_client::set_flags(&mut session, body.uid, &flag_strs, body.set).await
                    {
                        eprintln!("[email] set_flags error: {e}");
                    }
                }
                let _ = session.logout().await;
            }
            Err(e) => eprintln!("[email] connect for flags error: {e}"),
        }
    }

    // Update cached flags
    let mut current_flags: Vec<String> =
        email_db::get_message(&state.db, &body.account_id, &body.mailbox, body.uid)
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
    let _ = email_db::update_message_flags(
        &state.db,
        &body.account_id,
        &body.mailbox,
        body.uid,
        &flags_json,
    )
    .await;

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::EmailUpdated {
            account_id: body.account_id.clone(),
            uid: body.uid,
            mailbox: body.mailbox.clone(),
            flags: current_flags,
        },
    );

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
    let account = match email_db::get_account(
        &state.db,
        &q.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] delete_message account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let mailbox_leaf = q
        .mailbox
        .rsplit(|c| c == '/' || c == '.')
        .next()
        .unwrap_or(&q.mailbox)
        .to_ascii_lowercase();
    let is_trash = mailbox_leaf.contains("trash")
        || mailbox_leaf.contains("deleted")
        || mailbox_leaf.contains("bin");

    let trash_mailbox = if is_trash {
        None
    } else {
        email_db::find_trash_mailbox(&state.db, &q.account_id)
            .await
            .unwrap_or(None)
    };

    if !state.demo_mode {
        match imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await
        {
            Ok(mut session) => {
                if let Ok(_) = imap_client::select_mailbox(&mut session, &q.mailbox).await {
                    if let Some(ref trash) = trash_mailbox {
                        if let Err(e) =
                            imap_client::move_message_on_server(&mut session, q.uid, trash).await
                        {
                            eprintln!("[email] move_to_trash error: {e}");
                        }
                    } else if let Err(e) = imap_client::delete_on_server(&mut session, q.uid).await
                    {
                        eprintln!("[email] delete_on_server error: {e}");
                    }
                }
                let _ = session.logout().await;
            }
            Err(e) => eprintln!("[email] connect for delete error: {e}"),
        }
    }

    // Delete .eml and DB rows
    let _ =
        tokio::fs::remove_file(eml_path(&state.data_dir, &q.account_id, &q.mailbox, q.uid)).await;
    let _ =
        email_db::delete_attachments_for_message(&state.db, &q.account_id, &q.mailbox, q.uid).await;
    let _ = email_db::delete_message(&state.db, &q.account_id, &q.mailbox, q.uid).await;

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::EmailRemoved {
            account_id: q.account_id,
            uid: q.uid,
            mailbox: q.mailbox,
        },
    );

    StatusCode::NO_CONTENT.into_response()
}

// ── Move ──────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MoveMessageBody {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    pub dest_mailbox: String,
}

pub async fn move_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<MoveMessageBody>,
) -> impl IntoResponse {
    let account = match email_db::get_account(
        &state.db,
        &body.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] move_message account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    if !state.demo_mode {
        match imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await
        {
            Ok(mut session) => {
                if let Ok(_) = imap_client::select_mailbox(&mut session, &body.mailbox).await {
                    if let Err(e) = imap_client::move_message_on_server(
                        &mut session,
                        body.uid,
                        &body.dest_mailbox,
                    )
                    .await
                    {
                        eprintln!("[email] move_message_on_server error: {e}");
                    }
                }
                let _ = session.logout().await;
            }
            Err(e) => eprintln!("[email] connect for move error: {e}"),
        }
    }

    let _ = tokio::fs::remove_file(eml_path(
        &state.data_dir,
        &body.account_id,
        &body.mailbox,
        body.uid,
    ))
    .await;
    let _ = email_db::delete_attachments_for_message(
        &state.db,
        &body.account_id,
        &body.mailbox,
        body.uid,
    )
    .await;
    let _ = email_db::delete_message(&state.db, &body.account_id, &body.mailbox, body.uid).await;

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::EmailRemoved {
            account_id: body.account_id.clone(),
            uid: body.uid,
            mailbox: body.mailbox.clone(),
        },
    );
    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::EmailsRefetch {
            account_id: body.account_id,
            mailbox: body.dest_mailbox,
        },
    );

    StatusCode::NO_CONTENT.into_response()
}

// ── Search ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub folder_id: Option<String>,
    pub addrs: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub cursor_sent_at: Option<i64>,
    pub cursor_rowid: Option<i64>,
    pub direction: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct SuggestAddrsQuery {
    q: Option<String>,
}

pub async fn suggest_addrs(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<SuggestAddrsQuery>,
) -> Response {
    let query = q.q.unwrap_or_default();
    if query.trim().is_empty() {
        return Json(json!({ "addrs": [] })).into_response();
    }
    match email_db::suggest_addrs(&state.db, &user.user_id, &query, 5).await {
        Ok(addrs) => Json(json!({ "addrs": addrs })).into_response(),
        Err(_) => Json(json!({ "addrs": [] })).into_response(),
    }
}

pub async fn search(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<SearchQuery>,
) -> impl IntoResponse {
    let raw_q = q.q.as_deref().unwrap_or("").trim().to_owned();
    let addrs: Vec<String> = q
        .addrs
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let filters = email_db::SearchFilters {
        addrs,
        date_from: parse_date_start(q.date_from.as_deref()),
        date_to: parse_date_end(q.date_to.as_deref()),
    };

    if raw_q.is_empty() && filters.is_empty() {
        return Json(json!({ "messages": [] })).into_response();
    }

    let fts_query = if raw_q.is_empty() {
        String::new()
    } else {
        state.search_mode.build_fts_query(&raw_q)
    };
    let cursor = parse_cursor(q.cursor_sent_at, q.cursor_rowid);
    let direction = parse_direction(q.direction.as_deref());
    let limit = q.limit.unwrap_or(51).min(200);

    let result = if let Some(folder_id) = q.folder_id.as_deref() {
        email_db::search_messages_virtual(
            &state.db,
            &user.user_id,
            folder_id,
            &fts_query,
            &filters,
            limit,
            cursor,
            direction,
        )
        .await
    } else if let (Some(account_id), Some(mailbox)) =
        (q.account_id.as_deref(), q.mailbox.as_deref())
    {
        email_db::search_messages_mailbox(
            &state.db,
            &user.user_id,
            account_id,
            mailbox,
            &fts_query,
            &filters,
            limit,
            cursor,
            direction,
        )
        .await
    } else {
        email_db::search_messages(
            &state.db,
            &user.user_id,
            &fts_query,
            &filters,
            limit,
            cursor,
            direction,
        )
        .await
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

fn parse_date_start(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
}

fn parse_date_end(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(super) fn message_to_json(m: &email_db::MessageRow) -> serde_json::Value {
    message_to_json_with_attachments(m, &[])
}

pub(super) fn message_to_json_with_attachments(
    m: &email_db::MessageRow,
    attachments: &[email_db::AttachmentRow],
) -> serde_json::Value {
    let to: Vec<String> = m
        .to_addrs
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let cc: Vec<String> = m
        .cc_addrs
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let flags: Vec<String> = m
        .flags
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let att_list: Vec<serde_json::Value> = attachments
        .iter()
        .map(|a| {
            json!({
                "filename": a.filename,
                "content_type": a.content_type,
                "size": a.size,
            })
        })
        .collect();

    json!({
        "uid": m.uid,
        "account_id": m.account_id,
        "mailbox": m.mailbox,
        "rowid": m.rowid,
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
        "is_encrypted": m.is_encrypted,
        "is_signed": m.is_signed,
        "attachments": att_list,
    })
}

pub(super) fn thread_message_to_json(m: &email_db::MessageRow) -> serde_json::Value {
    let flags: Vec<String> = m
        .flags
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    json!({
        "uid": m.uid,
        "account_id": m.account_id,
        "mailbox": m.mailbox,
        "message_id": m.message_id,
        "parent_message_id": m.thread_id,
        "from": m.from_addr,
        "subject": m.subject,
        "sent_at": m.sent_at,
        "is_read": flags.iter().any(|f| f == "\\Seen"),
    })
}

#[cfg(test)]
mod tests {
    use super::{dedupe_thread_messages};
    use crate::email_db::MessageRow;

    fn thread_row(mailbox: &str, uid: i64, message_id: &str, sent_at: i64) -> MessageRow {
        MessageRow {
            uid,
            account_id: "account".to_owned(),
            mailbox: mailbox.to_owned(),
            message_id: Some(message_id.to_owned()),
            thread_id: None,
            from_addr: None,
            to_addrs: None,
            cc_addrs: None,
            subject: None,
            sent_at: Some(sent_at),
            flags: None,
            has_attachments: false,
            body_text: None,
            rowid: uid,
            body_html: None,
            size: None,
            is_encrypted: false,
            is_signed: false,
        }
    }

    #[test]
    fn thread_messages_deduplicate_mailbox_copies_and_keep_open_locator() {
        let messages = vec![
            thread_row("Archive", 1, "same", 1),
            thread_row("INBOX", 2, "same", 1),
            thread_row("INBOX", 3, "reply", 2),
        ];
        let result = dedupe_thread_messages(messages, "account", "INBOX", 2);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].mailbox, "INBOX");
        assert_eq!(result[0].uid, 2);
    }
}

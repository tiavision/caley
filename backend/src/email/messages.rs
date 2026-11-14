use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use chrono::{Datelike, Duration, Utc};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::api::ApiError;
use crate::email_db;
use crate::email_sync::eml_path;
use crate::imap_client;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

// ── Messages ──────────────────────────────────────────────────────────────────

/// A page of a mailbox (`account_id` + `mailbox`) or of a virtual folder (`folder_id`), newest first.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListMessagesQuery {
    /// the account of `mailbox` (required without `folder_id`)
    pub account_id: Option<String>,
    /// the IMAP mailbox name (required without `folder_id`)
    pub mailbox: Option<String>,
    /// a virtual folder instead of a mailbox
    pub folder_id: Option<String>,
    /// page size, default 50, at most 200
    pub limit: Option<i64>,
    /// keyset cursor: `sent_at` of the row the page starts after (or before, `backward`)
    pub cursor_sent_at: Option<i64>,
    /// keyset cursor: `rowid` of that row (no cursor without it)
    pub cursor_rowid: Option<i64>,
    /// `forward` (default: older than the cursor), `backward` (newer) or `last` (the oldest page)
    pub direction: Option<String>,
    /// rows to skip instead of a cursor
    pub offset: Option<i64>,
}

/// A page of messages (list rows: no body, no attachments).
#[derive(serde::Serialize, ToSchema)]
pub struct MessagesResponse {
    pub messages: Vec<Message>,
    /// the number of messages in the mailbox or folder
    pub total: i64,
}

/// A message, as the list, search and the message endpoint send it.
#[derive(serde::Serialize, ToSchema)]
pub struct Message {
    pub uid: i64,
    pub account_id: String,
    pub mailbox: String,
    /// the cache row; with `sent_at` the list's keyset cursor
    pub rowid: i64,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    /// the From header
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub reply_to: Vec<String>,
    pub subject: Option<String>,
    /// Unix seconds
    pub sent_at: Option<i64>,
    /// IMAP flags (`\Seen`, `\Flagged`, ...)
    pub flags: Vec<String>,
    pub has_attachments: bool,
    /// whether the body was cached (GET /email/message only; false in lists)
    pub body_cached: bool,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub autocrypt_headers: Vec<String>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub is_encrypted: bool,
    pub is_signed: bool,
    /// GET /email/message only; empty in lists
    pub attachments: Vec<Attachment>,
}

/// An attachment of a message (GET /email/attachment downloads it).
#[derive(serde::Serialize, ToSchema)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: Option<String>,
    /// bytes
    pub size: Option<i64>,
    /// the Content-ID of an inline part (`cid:` images)
    pub content_id: Option<String>,
}

/// A message of a conversation.
#[derive(serde::Serialize, ToSchema)]
pub struct ThreadNode {
    pub uid: i64,
    pub account_id: String,
    pub mailbox: String,
    pub message_id: Option<String>,
    /// the message this one replies to (its Message-ID)
    pub parent_message_id: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    /// Unix seconds
    pub sent_at: Option<i64>,
    pub is_read: bool,
}

/// The conversation of a message, oldest first, one entry per Message-ID.
#[derive(serde::Serialize, ToSchema)]
pub struct ThreadResponse {
    pub messages: Vec<ThreadNode>,
}

/// Search results (a page; no `total`).
#[derive(serde::Serialize, ToSchema)]
pub struct SearchResponse {
    pub messages: Vec<Message>,
}

/// Addresses seen in the user's mail that contain the query.
#[derive(serde::Serialize, ToSchema)]
pub struct AddrSuggestions {
    pub addrs: Vec<String>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct TrashEmptied {
    /// the number of messages deleted
    pub deleted: usize,
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

/// A page of a mailbox or virtual folder.
#[utoipa::path(
    get,
    path = "/email/messages",
    params(ListMessagesQuery),
    responses(
        (status = 200, body = MessagesResponse),
        (status = 400, description = "neither `folder_id` nor `account_id` and `mailbox` (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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
            let messages = messages.iter().map(message_to_json).collect();
            Json(MessagesResponse { messages, total }).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_messages error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Where a message lives.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GetMessageQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

/// A message with its cached body and attachments.
#[utoipa::path(
    get,
    path = "/email/message",
    params(GetMessageQuery),
    responses(
        (status = 200, body = Message),
        (status = 404, description = "no such account or message (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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
        Ok(Some(mut msg)) => {
            if msg.reply_to_addrs.is_none() {
                msg.reply_to_addrs = backfill_reply_to(&state, &q).await;
            }
            let attachments =
                email_db::list_attachments(&state.db, &q.account_id, &q.mailbox, q.uid)
                    .await
                    .unwrap_or_default();
            let render_body =
                email_db::get_message_render_body(&state.db, &q.account_id, &q.mailbox, q.uid)
                    .await
                    .unwrap_or_default();
            Json(message_to_json_with_attachments(
                &msg,
                &attachments,
                render_body.as_ref(),
            ))
            .into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "message not found").into_response(),
        Err(e) => {
            eprintln!("[email] get_message error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Parses Reply-To from the cached .eml for rows synced before the column existed.
async fn backfill_reply_to(state: &AppState, q: &GetMessageQuery) -> Option<String> {
    let raw = load_raw_eml(&state.data_dir, &q.account_id, &q.mailbox, q.uid).await?;
    let addrs = serde_json::to_string(&crate::imap_client::parse_reply_to(&raw)).ok()?;
    if let Err(e) =
        email_db::set_message_reply_to(&state.db, &q.account_id, &q.mailbox, q.uid, &addrs).await
    {
        eprintln!("[email] reply_to backfill error: {e}");
    }
    Some(addrs)
}

async fn load_raw_eml(
    data_dir: &std::path::Path,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> Option<Vec<u8>> {
    let path = eml_path(data_dir, account_id, mailbox, uid);
    let compressed = tokio::fs::read(&path).await.ok()?;
    let raw = zstd::decode_all(compressed.as_slice())
        .map_err(|e| {
            eprintln!("[email] raw_eml decompress error for {account_id}/{mailbox}/{uid}: {e}")
        })
        .ok()?;
    Some(raw)
}

/// A MIME message as bytes (only described: the handler sends the bytes themselves).
#[derive(ToSchema)]
#[allow(dead_code)]
#[schema(value_type = String, format = Binary)]
struct RawMessage(Vec<u8>);

/// The message's MIME source (for PGP processing).
#[utoipa::path(
    get,
    path = "/email/message/raw",
    params(GetMessageQuery),
    responses(
        (status = 200, body = inline(RawMessage), content_type = "message/rfc822", description = "the cached .eml"),
        (status = 404, description = "no such account or no cached source (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
pub async fn get_raw_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<GetMessageQuery>,
) -> Response {
    match email_db::get_account(
        &state.db,
        &q.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] get_raw_message account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    }

    match load_raw_eml(&state.data_dir, &q.account_id, &q.mailbox, q.uid).await {
        Some(raw) => (
            [
                (header::CONTENT_TYPE, "message/rfc822"),
                (header::CACHE_CONTROL, "private, no-store"),
                (
                    header::HeaderName::from_static("x-content-type-options"),
                    "nosniff",
                ),
            ],
            raw,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "message source not found").into_response(),
    }
}

/// The conversation of a message.
#[utoipa::path(
    get,
    path = "/email/thread",
    params(GetMessageQuery),
    responses(
        (status = 200, body = ThreadResponse),
        (status = 404, description = "no such account (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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
            let messages = messages.iter().map(thread_message_to_json).collect();
            Json(ThreadResponse { messages }).into_response()
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

/// Sets or clears IMAP flags on a message.
#[derive(Deserialize, ToSchema)]
pub struct FlagsBody {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    /// e.g. `\Seen`, `\Flagged`
    pub flags: Vec<String>,
    /// true adds the flags, false removes them
    pub set: bool,
}

/// Sets or clears flags on the server and in the cache; `email_updated` reports the new flags.
#[utoipa::path(
    patch,
    path = "/email/message/flags",
    request_body = FlagsBody,
    responses(
        (status = 200, description = "applied (empty body)"),
        (status = 404, description = "no such account (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteMessageQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

/// Moves a message to the trash (deletes it outright in the trash); `email_removed` reports it.
#[utoipa::path(
    delete,
    path = "/email/message",
    params(DeleteMessageQuery),
    responses(
        (status = 204, description = "deleted"),
        (status = 404, description = "no such account (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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

    let is_trash = is_trash_mailbox_name(&q.mailbox);

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

/// Deletes the messages of a trash mailbox put there more than `days` ago.
#[derive(Deserialize, ToSchema)]
pub struct EmptyTrashBody {
    pub account_id: String,
    /// a trash mailbox
    pub mailbox: String,
    /// 1 to 3650
    pub days: i64,
}

/// Empties a trash mailbox of old messages (IMAP SAVEDATE); `emails_refetch` reports it.
#[utoipa::path(
    post,
    path = "/email/trash/empty",
    request_body = EmptyTrashBody,
    responses(
        (status = 200, body = TrashEmptied),
        (status = 400, description = "`days` out of range or not a trash mailbox (plain text)"),
        (status = 404, description = "no such account (plain text)"),
        (status = 412, body = ApiError, description = "`savedate_unsupported`: the server cannot tell when messages were trashed"),
        (status = 500, description = "database error (plain text)"),
        (status = 502, description = "the IMAP server failed (plain text)")
    )
)]
pub async fn empty_trash(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<EmptyTrashBody>,
) -> impl IntoResponse {
    if body.days <= 0 || body.days > 3650 {
        return (StatusCode::BAD_REQUEST, "days must be between 1 and 3650").into_response();
    }
    if !is_trash_mailbox_name(&body.mailbox) {
        return (StatusCode::BAD_REQUEST, "mailbox is not trash").into_response();
    }

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
            eprintln!("[email] empty_trash account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let deleted_uids = if state.demo_mode {
        let cutoff_ts = Utc::now().timestamp() - body.days * 24 * 60 * 60;
        match email_db::get_cached_uids_sent_before(
            &state.db,
            &body.account_id,
            &body.mailbox,
            cutoff_ts,
        )
        .await
        {
            Ok(uids) => uids,
            Err(e) => {
                eprintln!("[email] empty_trash demo cache query: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        }
    } else {
        let mut session = match imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await
        {
            Ok(session) => session,
            Err(e) => {
                eprintln!("[email] empty_trash connect: {e}");
                return (StatusCode::BAD_GATEWAY, "imap connection failed").into_response();
            }
        };

        let result = async {
            imap_client::select_mailbox(&mut session, &body.mailbox).await?;
            if !imap_client::selected_mailbox_supports_savedate(&mut session).await? {
                return Ok::<_, anyhow::Error>(None);
            }
            let cutoff_date = imap_savedbefore_date(body.days);
            let uids = imap_client::search_saved_before(&mut session, &cutoff_date).await?;
            imap_client::delete_on_server_batch(&mut session, &uids).await?;
            Ok(Some(uids))
        }
        .await;
        let _ = session.logout().await;

        match result {
            Ok(Some(uids)) => uids,
            Ok(None) => {
                return (
                    StatusCode::PRECONDITION_FAILED,
                    Json(ApiError::new("savedate_unsupported")),
                )
                    .into_response();
            }
            Err(e) => {
                eprintln!("[email] empty_trash imap: {e}");
                return (StatusCode::BAD_GATEWAY, "imap delete failed").into_response();
            }
        }
    };

    if let Err(e) =
        delete_cached_messages_and_files(&state, &body.account_id, &body.mailbox, &deleted_uids)
            .await
    {
        eprintln!("[email] empty_trash cache cleanup: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
    }

    state.sse_broadcaster.send(
        &user.user_id,
        SseMessage::EmailsRefetch {
            account_id: body.account_id,
            mailbox: body.mailbox,
        },
    );

    Json(TrashEmptied {
        deleted: deleted_uids.len(),
    })
    .into_response()
}

fn is_trash_mailbox_name(mailbox: &str) -> bool {
    let mailbox_leaf = mailbox
        .rsplit(|c| c == '/' || c == '.')
        .next()
        .unwrap_or(mailbox)
        .to_ascii_lowercase();
    mailbox_leaf.contains("trash")
        || mailbox_leaf.contains("deleted")
        || mailbox_leaf.contains("bin")
}

fn imap_savedbefore_date(days: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let cutoff = Utc::now().date_naive() - Duration::days(days);
    format!(
        "{}-{}-{}",
        cutoff.day(),
        MONTHS[cutoff.month0() as usize],
        cutoff.year()
    )
}

async fn delete_cached_messages_and_files(
    state: &AppState,
    account_id: &str,
    mailbox: &str,
    uids: &[u32],
) -> anyhow::Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    for uid in uids {
        let _ = tokio::fs::remove_file(eml_path(&state.data_dir, account_id, mailbox, *uid)).await;
    }
    for chunk in uids.chunks(500) {
        email_db::delete_attachments_by_uids(&state.db, account_id, mailbox, chunk).await?;
        email_db::delete_messages_by_uids(&state.db, account_id, mailbox, chunk).await?;
    }
    Ok(())
}

// ── Move ──────────────────────────────────────────────────────────────────────

/// Moves a message to another mailbox of its account.
#[derive(Deserialize, ToSchema)]
pub struct MoveMessageBody {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    pub dest_mailbox: String,
}

/// Moves a message; `email_removed` and `emails_refetch` (of the destination) report it.
#[utoipa::path(
    post,
    path = "/email/message/move",
    request_body = MoveMessageBody,
    responses(
        (status = 204, description = "moved"),
        (status = 404, description = "no such account (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
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

/// A full-text search with filters, in one mailbox, one virtual folder or all mail.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// the search text (may be empty when a filter is set)
    pub q: Option<String>,
    /// with `mailbox`: search that mailbox
    pub account_id: Option<String>,
    /// with `account_id`: search that mailbox
    pub mailbox: Option<String>,
    /// search a virtual folder (takes precedence over `account_id`/`mailbox`)
    pub folder_id: Option<String>,
    /// comma-separated addresses the message must be from or to
    pub addrs: Option<String>,
    /// `YYYY-MM-DD`: sent on or after
    pub date_from: Option<String>,
    /// `YYYY-MM-DD`: sent on or before
    pub date_to: Option<String>,
    /// only unread messages
    pub unread_only: Option<bool>,
    /// keyset cursor: `sent_at` of the row the page starts after
    pub cursor_sent_at: Option<i64>,
    /// keyset cursor: `rowid` of that row
    pub cursor_rowid: Option<i64>,
    /// `forward` (default) or `backward`
    pub direction: Option<String>,
    /// page size, default 51, at most 200
    pub limit: Option<i64>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SuggestAddrsQuery {
    /// part of an address
    q: Option<String>,
}

/// Up to five addresses from the user's mail that contain `q` (none for an empty `q` or on error).
#[utoipa::path(
    get,
    path = "/email/suggest-addrs",
    params(SuggestAddrsQuery),
    responses((status = 200, body = AddrSuggestions))
)]
pub async fn suggest_addrs(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<SuggestAddrsQuery>,
) -> Response {
    let query = q.q.unwrap_or_default();
    if query.trim().is_empty() {
        return Json(AddrSuggestions { addrs: Vec::new() }).into_response();
    }
    let addrs = email_db::suggest_addrs(&state.db, &user.user_id, &query, 5)
        .await
        .unwrap_or_default();
    Json(AddrSuggestions { addrs }).into_response()
}

/// Searches messages; no text and no filter is an empty result.
#[utoipa::path(
    get,
    path = "/email/search",
    params(SearchQuery),
    responses(
        (status = 200, body = SearchResponse),
        (status = 500, description = "search error (plain text)")
    )
)]
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
        unread_only: q.unread_only.unwrap_or(false),
    };

    if raw_q.is_empty() && filters.is_empty() {
        return Json(SearchResponse {
            messages: Vec::new(),
        })
        .into_response();
    }

    let fts_query = if raw_q.is_empty() {
        String::new()
    } else {
        state.search_mode.build_fts_query(&raw_q)
    };
    let cursor = parse_cursor(q.cursor_sent_at, q.cursor_rowid);
    let direction = parse_direction(q.direction.as_deref());
    let limit = q.limit.unwrap_or(51).min(200);

    let scope = if let Some(folder_id) = q.folder_id.as_deref() {
        email_db::SearchScope::Virtual { folder_id }
    } else if let (Some(account_id), Some(mailbox)) =
        (q.account_id.as_deref(), q.mailbox.as_deref())
    {
        email_db::SearchScope::Mailbox {
            account_id,
            mailbox,
        }
    } else {
        email_db::SearchScope::All
    };
    let result = email_db::search_messages(
        &state.db,
        &user.user_id,
        scope,
        &fts_query,
        &filters,
        limit,
        cursor,
        direction,
    )
    .await;

    match result {
        Ok(messages) => {
            let messages = messages.iter().map(message_to_json).collect();
            Json(SearchResponse { messages }).into_response()
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

fn json_strings(s: Option<&str>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

pub(super) fn message_to_json(m: &email_db::MessageRow) -> Message {
    message_to_json_with_attachments(m, &[], None)
}

pub(super) fn message_to_json_with_attachments(
    m: &email_db::MessageRow,
    attachments: &[email_db::AttachmentRow],
    render_body: Option<&email_db::MessageBodyRow>,
) -> Message {
    let flags = json_strings(m.flags.as_deref());
    Message {
        uid: m.uid,
        account_id: m.account_id.clone(),
        mailbox: m.mailbox.clone(),
        rowid: m.rowid,
        message_id: m.message_id.clone(),
        thread_id: m.thread_id.clone(),
        from: m.from_addr.clone(),
        to: json_strings(m.to_addrs.as_deref()),
        cc: json_strings(m.cc_addrs.as_deref()),
        reply_to: json_strings(m.reply_to_addrs.as_deref()),
        subject: m.subject.clone(),
        sent_at: m.sent_at,
        has_attachments: m.has_attachments,
        body_cached: render_body.is_some(),
        body_text: render_body.and_then(|body| body.body_text.clone()),
        body_html: render_body.and_then(|body| body.body_html.clone()),
        autocrypt_headers: render_body
            .map(|body| body.autocrypt_headers.clone())
            .unwrap_or_default(),
        is_read: flags.iter().any(|f| f == "\\Seen"),
        is_flagged: flags.iter().any(|f| f == "\\Flagged"),
        is_encrypted: m.is_encrypted,
        is_signed: render_body.map(|body| body.is_signed).unwrap_or(false),
        attachments: attachments
            .iter()
            .map(|a| Attachment {
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                size: a.size,
                content_id: a.content_id.clone(),
            })
            .collect(),
        flags,
    }
}

pub(super) fn thread_message_to_json(m: &email_db::MessageRow) -> ThreadNode {
    let flags = json_strings(m.flags.as_deref());
    ThreadNode {
        uid: m.uid,
        account_id: m.account_id.clone(),
        mailbox: m.mailbox.clone(),
        message_id: m.message_id.clone(),
        parent_message_id: m.thread_id.clone(),
        from: m.from_addr.clone(),
        subject: m.subject.clone(),
        sent_at: m.sent_at,
        is_read: flags.iter().any(|f| f == "\\Seen"),
    }
}

/// This file's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_messages))
        .routes(routes!(get_message, delete_message))
        .routes(routes!(get_raw_message))
        .routes(routes!(get_thread))
        .routes(routes!(update_flags))
        .routes(routes!(move_message))
        .routes(routes!(empty_trash))
        .routes(routes!(search))
        .routes(routes!(suggest_addrs))
}

#[cfg(test)]
mod tests {
    use super::{dedupe_thread_messages, message_to_json_with_attachments};
    use crate::email_db::{MessageBodyRow, MessageRow};

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
            rowid: uid,
            size: None,
            is_encrypted: false,
            reply_to_addrs: None,
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

    #[test]
    fn message_detail_contains_cached_body_but_never_raw_mime() {
        let message = thread_row("INBOX", 1, "message", 1);
        let body = MessageBodyRow {
            body_text: Some("Hello".to_owned()),
            body_html: Some("<p>Hello</p>".to_owned()),
            autocrypt_headers: Vec::new(),
            is_signed: false,
        };

        let json =
            serde_json::to_value(message_to_json_with_attachments(&message, &[], Some(&body)))
                .unwrap();

        assert_eq!(json["body_cached"], true);
        assert_eq!(json["body_text"], "Hello");
        assert!(json.get("raw_eml").is_none());
    }
}

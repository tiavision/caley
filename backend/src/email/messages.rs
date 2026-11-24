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
use crate::email_db::{self, jmap_db};
use crate::email_sync::eml_path;
use crate::imap_client;
use crate::jmap;
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
    #[schema(required)]
    pub message_id: Option<String>,
    #[schema(required)]
    pub thread_id: Option<String>,
    /// the From header
    #[schema(required)]
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub reply_to: Vec<String>,
    #[schema(required)]
    pub subject: Option<String>,
    /// Unix seconds
    #[schema(required)]
    pub sent_at: Option<i64>,
    /// IMAP flags (`\Seen`, `\Flagged`, ...)
    pub flags: Vec<String>,
    pub has_attachments: bool,
    /// whether the body was cached (GET /email/message only; false in lists). The reader reads the body from the
    /// message's source (`/email/message/raw`).
    pub body_cached: bool,
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
    pub filename: String,
    pub content_type: String,
    /// bytes
    pub size: i64,
    /// the Content-ID of an inline part (`cid:` images)
    #[schema(required)]
    pub content_id: Option<String>,
}

/// A message of a conversation.
#[derive(serde::Serialize, ToSchema)]
pub struct ThreadNode {
    pub uid: i64,
    pub account_id: String,
    pub mailbox: String,
    #[schema(required)]
    pub message_id: Option<String>,
    /// the message this one replies to (its Message-ID)
    #[schema(required)]
    pub parent_message_id: Option<String>,
    #[schema(required)]
    pub from: Option<String>,
    #[schema(required)]
    pub subject: Option<String>,
    /// Unix seconds
    #[schema(required)]
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
    /// whether there are more results past this page, in the direction asked for
    pub has_more: bool,
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

/// Which message's source, and whether its large attachments are left out.
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RawMessageQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    /// leave out the bodies of attachments larger than this many bytes (encoded): a cut message (sinter-mail's
    /// "Large attachments: cut messages"), the cuts in the `Mail-Holes` header. Inline images count from 512 KiB
    /// (or this, if larger).
    #[serde(default)]
    pub omit: Option<usize>,
}

/// The message's MIME source, as the reader reads it (sinter-mail's `mail.mime`). With `omit`, a cut message: the
/// bodies of large attachments left out, where they were in `Mail-Holes` (JSON, `[{ start, end, size }]`, offsets of
/// the whole message, `size` decoded). Without it the whole message, which answers a `Range` (one range of bytes,
/// `If-Range` with its `ETag`), so a cut body is fetched when it is opened. Both carry the whole message's `ETag`.
#[utoipa::path(
    get,
    path = "/email/message/raw",
    params(RawMessageQuery),
    responses(
        (status = 200, body = crate::api::Binary, content_type = "message/rfc822", description = "the cached .eml, or with `omit` the cut message",
         headers(("ETag" = String, description = "the whole message's"), ("Mail-Holes" = String, description = "with `omit`: the cuts (JSON)"))),
        (status = 206, body = crate::api::Binary, content_type = "application/octet-stream", description = "the range of the whole message a `Range` asked for"),
        (status = 404, description = "no such account or no cached source (plain text)"),
        (status = 416, description = "a range outside the message (plain text)"),
        (status = 500, description = "database error (plain text)")
    )
)]
pub async fn get_raw_message(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<RawMessageQuery>,
    headers: axum::http::HeaderMap,
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

    let Some(raw) = load_raw_eml(&state.data_dir, &q.account_id, &q.mailbox, q.uid).await else {
        return (StatusCode::NOT_FOUND, "message source not found").into_response();
    };
    raw_response(raw, q.omit, &headers)
}

/// The response of `get_raw_message` for the whole message `raw`.
fn raw_response(raw: Vec<u8>, omit: Option<usize>, request: &axum::http::HeaderMap) -> Response {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&raw);
    let etag = format!(
        "\"{}\"",
        digest[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let common = [
        (header::CACHE_CONTROL, "private, no-store".to_owned()),
        (
            header::HeaderName::from_static("x-content-type-options"),
            "nosniff".to_owned(),
        ),
        (header::ETAG, etag.clone()),
    ];

    if let Some(over) = omit {
        let options = super::cut::CutOptions {
            over,
            related_over: over.max(512 * 1024),
            ..Default::default()
        };
        let (cut, holes) = super::cut::cut(&raw, &options);
        let holes = serde_json::to_string(&holes).unwrap_or_else(|_| "[]".to_owned());
        return (
            common,
            [
                (header::CONTENT_TYPE, "message/rfc822".to_owned()),
                (header::HeaderName::from_static("mail-holes"), holes),
            ],
            cut,
        )
            .into_response();
    }

    let range = request.get(header::RANGE).and_then(|v| v.to_str().ok());
    let current = request
        .get(header::IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v == etag);
    if let (Some(range), true) = (range, current) {
        return match byte_range(range, raw.len()) {
            Some((from, to)) => (
                StatusCode::PARTIAL_CONTENT,
                common,
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
                    (
                        header::CONTENT_RANGE,
                        format!("bytes {from}-{to}/{}", raw.len()),
                    ),
                ],
                raw[from..=to].to_vec(),
            )
                .into_response(),
            None => (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{}", raw.len()))],
                "range not satisfiable",
            )
                .into_response(),
        };
    }
    (
        common,
        [
            (header::CONTENT_TYPE, "message/rfc822".to_owned()),
            (header::ACCEPT_RANGES, "bytes".to_owned()),
        ],
        raw,
    )
        .into_response()
}

/// One range of a `Range: bytes=…` header (`a-b`, `a-`, `-n`) within `len` bytes: its first and last byte.
fn byte_range(value: &str, len: usize) -> Option<(usize, usize)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (from, to) = if a.is_empty() {
        let n: usize = b.parse().ok()?;
        (len.checked_sub(n.min(len)).filter(|_| n > 0)?, len - 1)
    } else {
        let from: usize = a.parse().ok()?;
        let to = if b.is_empty() {
            len - 1
        } else {
            b.parse::<usize>().ok()?.min(len - 1)
        };
        (from, to)
    };
    (from <= to && from < len).then_some((from, to))
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

// ── Flags, moves, deletions ───────────────────────────────────────────────────
//
// One message (the reader, a dragged row) or many (the list's bulk menu): both run `apply`, which makes the
// changes of one account's messages on its IMAP server over one connection, then in the cache, and reports each
// with `email_updated` / `email_removed` (and `emails_refetch` for a move's destination). A server error is
// logged, not reported: the cache changes either way and the next sync corrects it.

/// A message: its account, mailbox and UID.
#[derive(Clone, Deserialize, serde::Serialize, ToSchema)]
pub struct MessageRef {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
}

/// What happens to one message.
enum Change {
    Flags {
        flags: Vec<String>,
        set: bool,
    },
    Move {
        dest: String,
    },
    /// to the account's trash; deleted outright in a trash mailbox or without one
    Delete,
}

/// The messages' accounts, each looked up once, with the indices of its messages. The messages of an account that
/// does not exist (or could not be read) go to `failed`.
async fn accounts_of(
    state: &AppState,
    user_id: &str,
    refs: &[&MessageRef],
    failed: &mut Vec<MessageRef>,
) -> Vec<(email_db::ImapAccountRow, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, r) in refs.iter().enumerate() {
        match groups.iter_mut().find(|(id, _)| *id == r.account_id) {
            Some((_, idx)) => idx.push(i),
            None => groups.push((r.account_id.clone(), vec![i])),
        }
    }
    let mut out = Vec::new();
    for (account_id, idx) in groups {
        match email_db::get_account(&state.db, &account_id, user_id, &state.encryption_key).await {
            Ok(Some(account)) => out.push((account, idx)),
            other => {
                if let Err(e) = other {
                    eprintln!("[email] account lookup: {e}");
                }
                failed.extend(idx.iter().map(|&i| refs[i].clone()));
            }
        }
    }
    out
}

/// Makes the changes of one account's messages: on the server, in the cache, and in the server events.
async fn apply(
    state: &AppState,
    user_id: &str,
    account: &email_db::ImapAccountRow,
    changes: Vec<(MessageRef, Change)>,
) {
    // A deletion outside the trash is a move to it.
    let in_trash = |mailbox: &str, trash: &Option<String>| {
        if account.is_jmap() {
            trash.as_deref() == Some(mailbox)
        } else {
            is_trash_mailbox_name(mailbox)
        }
    };
    let trash = if account.is_jmap() {
        jmap_db::jmap_mailbox_by_role(&state.db, &account.account_id, "trash")
            .await
            .unwrap_or(None)
            .map(|(_, name)| name)
    } else if changes
        .iter()
        .any(|(m, c)| matches!(c, Change::Delete) && !is_trash_mailbox_name(&m.mailbox))
    {
        email_db::find_trash_mailbox(&state.db, &account.account_id)
            .await
            .unwrap_or(None)
    } else {
        None
    };
    let changes: Vec<(MessageRef, Change)> = changes
        .into_iter()
        .map(|(m, c)| match (c, &trash) {
            (Change::Delete, Some(t)) if !in_trash(&m.mailbox, &trash) => {
                (m, Change::Move { dest: t.clone() })
            }
            (c, _) => (m, c),
        })
        .collect();

    if state.demo_mode {
        // the cache alone
    } else if account.is_jmap() {
        apply_on_jmap(state, user_id, account, &changes).await;
    } else {
        match imap_client::connect(
            &account.host,
            account.port as u16,
            &account.username,
            &account.password,
        )
        .await
        {
            Ok(mut session) => {
                let mut selected: Option<&str> = None;
                for (m, change) in &changes {
                    if selected != Some(m.mailbox.as_str()) {
                        if let Err(e) = imap_client::select_mailbox(&mut session, &m.mailbox).await
                        {
                            eprintln!("[email] select {}: {e}", m.mailbox);
                            selected = None;
                            continue;
                        }
                        selected = Some(m.mailbox.as_str());
                    }
                    let result = match change {
                        Change::Flags { flags, set } => {
                            let flags: Vec<&str> = flags.iter().map(|s| s.as_str()).collect();
                            imap_client::set_flags(&mut session, m.uid, &flags, *set).await
                        }
                        Change::Move { dest } => {
                            imap_client::move_message_on_server(&mut session, m.uid, dest).await
                        }
                        Change::Delete => imap_client::delete_on_server(&mut session, m.uid).await,
                    };
                    if let Err(e) = result {
                        eprintln!("[email] change of {} {}: {e}", m.mailbox, m.uid);
                    }
                }
                let _ = session.logout().await;
            }
            Err(e) => eprintln!("[email] connect for a message change: {e}"),
        }
    }

    let mut refetch: Vec<String> = Vec::new();
    for (m, change) in changes {
        match change {
            Change::Flags { flags, set } => cache_flags(state, user_id, m, &flags, set).await,
            Change::Move { dest } => {
                cache_remove(state, user_id, m).await;
                if !refetch.contains(&dest) {
                    refetch.push(dest);
                }
            }
            Change::Delete => cache_remove(state, user_id, m).await,
        }
    }
    for mailbox in refetch {
        state.sse_broadcaster.send(
            user_id,
            SseMessage::EmailsRefetch {
                account_id: account.account_id.clone(),
                mailbox,
            },
        );
    }
}

/// Makes the changes on a JMAP server (Email/set); failures are logged, as IMAP's are. The server's push then syncs
/// the account, which gives a moved message its UID in the destination.
async fn apply_on_jmap(
    state: &AppState,
    user_id: &str,
    account: &email_db::ImapAccountRow,
    changes: &[(MessageRef, Change)],
) {
    let client = match state.email_sync_manager.jmap_client(account, user_id).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[email] connect for a message change: {e}");
            return;
        }
    };
    let ops: Vec<(&str, u32, jmap::ops::Change)> = changes
        .iter()
        .map(|(m, c)| {
            let op = match c {
                Change::Flags { flags, set } => jmap::ops::Change::Flags { flags, set: *set },
                Change::Move { dest } => jmap::ops::Change::Move { dest },
                Change::Delete => jmap::ops::Change::Delete,
            };
            (m.mailbox.as_str(), m.uid, op)
        })
        .collect();
    match jmap::ops::apply_changes(&client, &state.db, &account.account_id, &ops).await {
        Ok(failed) => {
            for ((mailbox, uid), reason) in failed {
                eprintln!("[email] change of {mailbox} {uid}: {reason}");
            }
        }
        Err(e) => eprintln!("[email] JMAP change: {e}"),
    }
    // the moved messages' UIDs in their destinations come with the account's next sync
    if changes
        .iter()
        .any(|(_, c)| matches!(c, Change::Move { .. }))
    {
        state.email_sync_manager.trigger(
            &account.account_id,
            crate::email_sync::EmailSyncTrigger::Full,
        );
    }
}

/// Adds or removes flags of a cached message; `email_updated` reports the new flags.
async fn cache_flags(state: &AppState, user_id: &str, m: MessageRef, flags: &[String], set: bool) {
    let mut current: Vec<String> =
        email_db::get_message(&state.db, &m.account_id, &m.mailbox, m.uid)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.flags)
            .and_then(|f| serde_json::from_str(&f).ok())
            .unwrap_or_default();
    if set {
        for f in flags {
            if !current.contains(f) {
                current.push(f.clone());
            }
        }
    } else {
        current.retain(|f| !flags.contains(f));
    }
    let flags_json = serde_json::to_string(&current).unwrap_or_default();
    let _ =
        email_db::update_message_flags(&state.db, &m.account_id, &m.mailbox, m.uid, &flags_json)
            .await;
    state.sse_broadcaster.send(
        user_id,
        SseMessage::EmailUpdated {
            account_id: m.account_id,
            uid: m.uid,
            mailbox: m.mailbox,
            flags: current,
        },
    );
}

/// Drops a message from the cache (its .eml, attachments and row); `email_removed` reports it.
async fn cache_remove(state: &AppState, user_id: &str, m: MessageRef) {
    let _ =
        tokio::fs::remove_file(eml_path(&state.data_dir, &m.account_id, &m.mailbox, m.uid)).await;
    let _ =
        email_db::delete_attachments_for_message(&state.db, &m.account_id, &m.mailbox, m.uid).await;
    let _ = email_db::delete_message(&state.db, &m.account_id, &m.mailbox, m.uid).await;
    state.sse_broadcaster.send(
        user_id,
        SseMessage::EmailRemoved {
            account_id: m.account_id,
            uid: m.uid,
            mailbox: m.mailbox,
        },
    );
}

/// The account of a single-message request: 404 when there is none.
async fn account_or_error(
    state: &AppState,
    user_id: &str,
    account_id: &str,
) -> Result<email_db::ImapAccountRow, Response> {
    match email_db::get_account(&state.db, account_id, user_id, &state.encryption_key).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err((StatusCode::NOT_FOUND, "account not found").into_response()),
        Err(e) => {
            eprintln!("[email] account lookup: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response())
        }
    }
}

/// Runs one change per message, grouped by account; the messages whose account is unknown failed.
async fn apply_bulk(
    state: &AppState,
    user_id: &str,
    items: Vec<(MessageRef, Change)>,
) -> BulkResult {
    let mut failed = Vec::new();
    let refs: Vec<&MessageRef> = items.iter().map(|(m, _)| m).collect();
    let groups = accounts_of(state, user_id, &refs, &mut failed).await;
    let mut items: Vec<Option<(MessageRef, Change)>> = items.into_iter().map(Some).collect();
    for (account, idx) in groups {
        let changes = idx.iter().filter_map(|&i| items[i].take()).collect();
        apply(state, user_id, &account, changes).await;
    }
    BulkResult { failed }
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
    let account = match account_or_error(&state, &user.user_id, &body.account_id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let m = MessageRef {
        account_id: body.account_id,
        mailbox: body.mailbox,
        uid: body.uid,
    };
    let change = Change::Flags {
        flags: body.flags,
        set: body.set,
    };
    apply(&state, &user.user_id, &account, vec![(m, change)]).await;
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
    let account = match account_or_error(&state, &user.user_id, &q.account_id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let m = MessageRef {
        account_id: q.account_id,
        mailbox: q.mailbox,
        uid: q.uid,
    };
    apply(&state, &user.user_id, &account, vec![(m, Change::Delete)]).await;
    StatusCode::NO_CONTENT.into_response()
}

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
    let account = match account_or_error(&state, &user.user_id, &body.account_id).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let (m, change) = move_change(body);
    apply(&state, &user.user_id, &account, vec![(m, change)]).await;
    StatusCode::NO_CONTENT.into_response()
}

fn move_change(body: MoveMessageBody) -> (MessageRef, Change) {
    let m = MessageRef {
        account_id: body.account_id,
        mailbox: body.mailbox,
        uid: body.uid,
    };
    (
        m,
        Change::Move {
            dest: body.dest_mailbox,
        },
    )
}

// ── Bulk ──────────────────────────────────────────────────────────────────────

/// What a bulk change could not do: the messages whose account does not exist (the others were changed).
#[derive(serde::Serialize, ToSchema)]
pub struct BulkResult {
    pub failed: Vec<MessageRef>,
}

/// Sets or clears flags on several messages.
#[derive(Deserialize, ToSchema)]
pub struct BulkFlagsBody {
    pub messages: Vec<MessageRef>,
    /// e.g. `\Seen`, `\Flagged`
    pub flags: Vec<String>,
    /// true adds the flags, false removes them
    pub set: bool,
}

/// Sets or clears flags of several messages (each as PATCH /email/message/flags); `email_updated` reports each.
#[utoipa::path(
    patch,
    path = "/email/messages/flags",
    request_body = BulkFlagsBody,
    responses((status = 200, body = BulkResult))
)]
pub async fn bulk_flags(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<BulkFlagsBody>,
) -> impl IntoResponse {
    let items = body
        .messages
        .into_iter()
        .map(|m| {
            let change = Change::Flags {
                flags: body.flags.clone(),
                set: body.set,
            };
            (m, change)
        })
        .collect();
    Json(apply_bulk(&state, &user.user_id, items).await)
}

/// Moves several messages, each to its `dest_mailbox`.
#[derive(Deserialize, ToSchema)]
pub struct BulkMoveBody {
    pub messages: Vec<MoveMessageBody>,
}

/// Moves several messages (each as POST /email/message/move); `email_removed` and `emails_refetch` report them.
#[utoipa::path(
    post,
    path = "/email/messages/move",
    request_body = BulkMoveBody,
    responses((status = 200, body = BulkResult))
)]
pub async fn bulk_move(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<BulkMoveBody>,
) -> impl IntoResponse {
    let items = body.messages.into_iter().map(move_change).collect();
    Json(apply_bulk(&state, &user.user_id, items).await)
}

/// Several messages.
#[derive(Deserialize, ToSchema)]
pub struct BulkMessagesBody {
    pub messages: Vec<MessageRef>,
}

/// Deletes several messages (each as DELETE /email/message: to the trash, outright in the trash); `email_removed`
/// reports each.
#[utoipa::path(
    post,
    path = "/email/messages/delete",
    request_body = BulkMessagesBody,
    responses((status = 200, body = BulkResult))
)]
pub async fn bulk_delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<BulkMessagesBody>,
) -> impl IntoResponse {
    let items = body
        .messages
        .into_iter()
        .map(|m| (m, Change::Delete))
        .collect();
    Json(apply_bulk(&state, &user.user_id, items).await)
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

    if account.is_jmap() && !state.demo_mode {
        // JMAP has no date a message was put in a mailbox (IMAP's SAVEDATE); receivedAt would empty a trash of
        // old mail trashed today
        return (
            StatusCode::PRECONDITION_FAILED,
            Json(ApiError::new("savedate_unsupported")),
        )
            .into_response();
    }

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
            has_more: false,
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
    let limit = q.limit.unwrap_or(51).clamp(1, 200);
    let backward = matches!(
        direction,
        email_db::PageDirection::Backward | email_db::PageDirection::Last
    );

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
        // one more than the page: whether there are more results
        limit + 1,
        cursor,
        direction,
    )
    .await;

    match result {
        Ok(mut messages) => {
            let has_more = messages.len() as i64 > limit;
            if has_more {
                // the extra one is the farthest from the cursor: last going forward, first going backward
                if backward {
                    messages.remove(0);
                } else {
                    messages.pop();
                }
            }
            let messages = messages.iter().map(message_to_json).collect();
            Json(SearchResponse { messages, has_more }).into_response()
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
        is_read: flags.iter().any(|f| f == "\\Seen"),
        is_flagged: flags.iter().any(|f| f == "\\Flagged"),
        is_encrypted: m.is_encrypted,
        is_signed: render_body.map(|body| body.is_signed).unwrap_or(false),
        attachments: attachments
            .iter()
            .map(|a| Attachment {
                // The sync always stores the three (`untitled` for a part without a name); the columns allow NULL.
                filename: a.filename.clone().unwrap_or_else(|| "untitled".to_owned()),
                content_type: a
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_owned()),
                size: a.size.unwrap_or(0),
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
        .routes(routes!(bulk_flags))
        .routes(routes!(bulk_move))
        .routes(routes!(bulk_delete))
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
    fn message_detail_says_the_body_is_cached_but_carries_neither_it_nor_raw_mime() {
        let message = thread_row("INBOX", 1, "message", 1);
        let body = MessageBodyRow { is_signed: false };

        let json =
            serde_json::to_value(message_to_json_with_attachments(&message, &[], Some(&body)))
                .unwrap();

        assert_eq!(json["body_cached"], true);
        assert!(json.get("body_text").is_none() && json.get("body_html").is_none());
        assert!(json.get("raw_eml").is_none());
    }
}

#[cfg(test)]
mod raw_tests {
    use super::*;

    #[test]
    fn byte_ranges() {
        assert_eq!(byte_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(byte_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=95-", 100), Some((95, 99)));
        assert_eq!(byte_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(byte_range("bytes=100-", 100), None);
        assert_eq!(byte_range("bytes=5-1", 100), None);
        assert_eq!(byte_range("bytes=0-1,5-6", 100), None);
        assert_eq!(byte_range("items=0-1", 100), None);
    }

    #[tokio::test]
    async fn the_whole_message_cut_and_by_range() {
        let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHi\r\n\
                    --b\r\nContent-Type: application/pdf\r\n\r\n0123456789abcdef\r\n--b--\r\n".to_vec();
        let none = axum::http::HeaderMap::new();
        let cut = raw_response(raw.clone(), Some(8), &none);
        assert_eq!(cut.status(), StatusCode::OK);
        let holes: serde_json::Value =
            serde_json::from_str(cut.headers()["mail-holes"].to_str().unwrap()).unwrap();
        let (start, end) = (
            holes[0]["start"].as_u64().unwrap() as usize,
            holes[0]["end"].as_u64().unwrap() as usize,
        );
        assert_eq!(&raw[start..end], b"0123456789abcdef");
        let etag = cut.headers()[header::ETAG].clone();

        let mut ranged = axum::http::HeaderMap::new();
        ranged.insert(
            header::RANGE,
            format!("bytes={start}-{}", end - 1).parse().unwrap(),
        );
        ranged.insert(header::IF_RANGE, etag);
        let part = raw_response(raw.clone(), None, &ranged);
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        let body = axum::body::to_bytes(part.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"0123456789abcdef");

        ranged.insert(header::IF_RANGE, "\"other\"".parse().unwrap());
        assert_eq!(raw_response(raw, None, &ranged).status(), StatusCode::OK); // changed: the whole message
    }
}

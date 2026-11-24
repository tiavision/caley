use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::email_db;
use crate::email_sync::EmailSyncTrigger;
use crate::oidc::{AppState, CurrentUser};
use crate::sse::SseMessage;

pub(crate) mod cut;
pub(crate) mod attachments;
pub(crate) mod messages;
pub(crate) mod send;

// ── Accounts ──────────────────────────────────────────────────────────────────

/// An IMAP account (its password is never sent).
#[derive(Serialize, ToSchema)]
pub struct Account {
    pub account_id: String,
    pub display_name: String,
    /// IMAP server
    pub host: String,
    pub port: i64,
    pub username: String,
    pub use_tls: bool,
    /// SMTP server; `null`: the IMAP host
    #[schema(required)]
    pub smtp_host: Option<String>,
    /// `null`: 587
    #[schema(required)]
    pub smtp_port: Option<i64>,
    pub sort_order: i64,
}

#[derive(Serialize, ToSchema)]
pub struct AccountsResponse {
    pub accounts: Vec<Account>,
}

#[derive(Serialize, ToSchema)]
pub struct AccountSaved {
    pub account_id: String,
}

/// The IMAP accounts.
#[utoipa::path(
    get,
    path = "/email/accounts",
    responses((status = 200, body = AccountsResponse), (status = 500, description = "database error"))
)]
pub async fn list_accounts(State(state): State<AppState>, user: CurrentUser) -> impl IntoResponse {
    match email_db::list_accounts(&state.db, &user.user_id, &state.encryption_key).await {
        Ok(accounts) => {
            let accounts = accounts
                .into_iter()
                .map(|a| Account {
                    account_id: a.account_id,
                    display_name: a.display_name,
                    host: a.host,
                    port: a.port,
                    username: a.username,
                    use_tls: a.use_tls,
                    smtp_host: a.smtp_host,
                    smtp_port: a.smtp_port,
                    sort_order: a.sort_order,
                })
                .collect();
            Json(AccountsResponse { accounts }).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_accounts error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// An IMAP account to add (no `account_id`) or update.
#[derive(Deserialize, ToSchema)]
pub struct AccountBody {
    pub account_id: Option<String>,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    /// empty or missing: keep the stored one (required for a new account)
    pub password: Option<String>,
    /// default true
    pub use_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    /// a new account's default identity gets this name (none without it)
    pub sender_name: Option<String>,
}

/// Adds (no `account_id`) or updates an IMAP account; PUT is the same.
#[utoipa::path(
    method(post, put),
    path = "/email/account",
    request_body = AccountBody,
    responses(
        (status = 200, body = AccountSaved),
        (status = 400, description = "password required for new account (plain text)"),
        (status = 500, description = "database error")
    )
)]
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
    Json(AccountSaved { account_id }).into_response()
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AccountIdQuery {
    pub account_id: String,
}

/// Deletes an IMAP account with its cached mail.
#[utoipa::path(
    delete,
    path = "/email/account",
    params(AccountIdQuery),
    responses((status = 204, description = "deleted"), (status = 500, description = "database error"))
)]
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

/// A sender identity (name and address) of an account.
#[derive(Serialize, ToSchema)]
pub struct Identity {
    pub identity_id: String,
    pub account_id: String,
    pub name: String,
    pub email: String,
    pub is_default: bool,
    pub sort_order: i64,
}

#[derive(Serialize, ToSchema)]
pub struct IdentitiesResponse {
    pub identities: Vec<Identity>,
}

#[derive(Serialize, ToSchema)]
pub struct IdentitySaved {
    pub identity_id: String,
}

/// The sender identities of all accounts.
#[utoipa::path(
    get,
    path = "/email/identities",
    responses((status = 200, body = IdentitiesResponse), (status = 500, description = "database error"))
)]
pub async fn list_identities(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::list_identities(&state.db, &user.user_id).await {
        Ok(rows) => {
            let identities = rows
                .into_iter()
                .map(|i| Identity {
                    identity_id: i.identity_id,
                    account_id: i.account_id,
                    name: i.name,
                    email: i.email,
                    is_default: i.is_default,
                    sort_order: i.sort_order,
                })
                .collect();
            Json(IdentitiesResponse { identities }).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_identities error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// A sender identity to add (no `identity_id`) or update.
#[derive(Deserialize, ToSchema)]
pub struct IdentityBody {
    pub identity_id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub email: String,
    /// default false
    pub is_default: Option<bool>,
}

/// Adds (no `identity_id`) or updates a sender identity; PUT is the same.
#[utoipa::path(
    method(post, put),
    path = "/email/identity",
    request_body = IdentityBody,
    responses(
        (status = 200, body = IdentitySaved),
        (status = 404, description = "account not found (plain text)"),
        (status = 500, description = "database error")
    )
)]
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

    Json(IdentitySaved { identity_id }).into_response()
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IdentityIdQuery {
    pub identity_id: String,
}

#[utoipa::path(
    delete,
    path = "/email/identity",
    params(IdentityIdQuery),
    responses((status = 204, description = "deleted"), (status = 500, description = "database error"))
)]
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

/// An IMAP mailbox of an account.
#[derive(Serialize, ToSchema)]
pub struct Mailbox {
    pub account_id: String,
    /// the full IMAP name
    pub name: String,
    /// the hierarchy delimiter (`/`, `.`), `null` for a flat namespace
    #[schema(required)]
    pub delimiter: Option<String>,
    pub subscribed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct MailboxesResponse {
    pub mailboxes: Vec<Mailbox>,
}

/// The cached mailboxes of an account.
#[utoipa::path(
    get,
    path = "/email/mailboxes",
    params(AccountIdQuery),
    responses(
        (status = 200, body = MailboxesResponse),
        (status = 404, description = "account not found (plain text)"),
        (status = 500, description = "database error")
    )
)]
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
            let mailboxes = mbs
                .into_iter()
                .map(|m| Mailbox {
                    account_id: m.account_id,
                    name: m.name,
                    delimiter: m.delimiter,
                    subscribed: m.subscribed,
                })
                .collect();
            Json(MailboxesResponse { mailboxes }).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_mailboxes error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Sync trigger ─────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MailboxSyncQuery {
    pub account_id: String,
    pub mailbox: String,
}

/// Starts a sync of one mailbox; its progress arrives on the event stream.
#[utoipa::path(
    post,
    path = "/email/sync",
    params(MailboxSyncQuery),
    responses(
        (status = 200, description = "sync started (empty body)"),
        (status = 404, description = "account not found (plain text)"),
        (status = 500, description = "database error")
    )
)]
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

/// Which mailboxes a virtual folder takes: `null` matches any account or mailbox.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct VirtualFolderRule {
    #[schema(required)]
    pub account_id: Option<String>,
    #[schema(required)]
    pub mailbox_pattern: Option<String>,
}

/// A virtual folder: the messages of the mailboxes its rules match.
#[derive(Serialize, ToSchema)]
pub struct VirtualFolder {
    pub folder_id: String,
    pub display_name: String,
    pub sort_order: i64,
    pub only_flagged: bool,
    pub only_attachments: bool,
    pub rules: Vec<VirtualFolderRule>,
}

#[derive(Serialize, ToSchema)]
pub struct VirtualFoldersResponse {
    pub folders: Vec<VirtualFolder>,
}

/// The id of the folder created or updated.
#[derive(Serialize, ToSchema)]
pub struct VirtualFolderSaved {
    pub folder_id: String,
}

/// The unread messages per account and mailbox, and per virtual folder.
#[derive(Serialize, ToSchema)]
pub struct UnreadCounts {
    /// account id → mailbox name → unread count
    pub mailboxes: BTreeMap<String, BTreeMap<String, i64>>,
    /// folder id → unread count
    pub folders: BTreeMap<String, i64>,
}

#[utoipa::path(
    get,
    path = "/email/folders",
    responses((status = 200, body = VirtualFoldersResponse), (status = 500, description = "database error"))
)]
pub async fn list_virtual_folders(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::list_virtual_folders(&state.db, &user.user_id).await {
        Ok(folders) => {
            let folders = folders
                .into_iter()
                .map(|f| VirtualFolder {
                    folder_id: f.folder_id,
                    display_name: f.display_name,
                    sort_order: f.sort_order,
                    only_flagged: f.only_flagged,
                    only_attachments: f.only_attachments,
                    rules: f
                        .rules
                        .into_iter()
                        .map(|r| VirtualFolderRule {
                            account_id: r.account_id,
                            mailbox_pattern: r.mailbox_pattern,
                        })
                        .collect(),
                })
                .collect();
            Json(VirtualFoldersResponse { folders }).into_response()
        }
        Err(e) => {
            eprintln!("[email] list_virtual_folders error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Unread counts; a database error gives empty maps.
#[utoipa::path(get, path = "/email/unread-counts", responses((status = 200, body = UnreadCounts)))]
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

    let mut mailboxes: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for (acc, mbox, cnt) in mb {
        mailboxes.entry(acc).or_default().insert(mbox, cnt);
    }
    let folders = folders.into_iter().collect();

    Json(UnreadCounts { mailboxes, folders }).into_response()
}

/// A virtual folder to create (no `folder_id`) or update.
#[derive(Deserialize, ToSchema)]
pub struct VirtualFolderBody {
    /// the folder to update; absent or `null`: a new folder
    pub folder_id: Option<String>,
    pub display_name: String,
    pub rules: Vec<VirtualFolderRule>,
    /// default false
    pub only_flagged: Option<bool>,
    /// default false
    pub only_attachments: Option<bool>,
}

/// Creates a virtual folder, or updates the one `folder_id` names.
#[utoipa::path(
    post,
    path = "/email/folder",
    request_body = VirtualFolderBody,
    responses(
        (status = 200, body = VirtualFolderSaved),
        (status = 404, description = "no such folder (plain text)"),
        (status = 500, description = "database error")
    )
)]
pub async fn save_virtual_folder(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<VirtualFolderBody>,
) -> impl IntoResponse {
    let rules: Vec<(Option<&str>, Option<&str>)> = body
        .rules
        .iter()
        .map(|r| (r.account_id.as_deref(), r.mailbox_pattern.as_deref()))
        .collect();
    let only_flagged = body.only_flagged.unwrap_or(false);
    let only_attachments = body.only_attachments.unwrap_or(false);

    let (folder_id, result) = match body.folder_id {
        Some(folder_id) => {
            let updated = email_db::update_virtual_folder(
                &state.db,
                &folder_id,
                &user.user_id,
                &body.display_name,
                &rules,
                only_flagged,
                only_attachments,
            )
            .await;
            (folder_id, updated)
        }
        None => {
            let folder_id = Uuid::new_v4().to_string();
            let created = email_db::create_virtual_folder(
                &state.db,
                &folder_id,
                &user.user_id,
                &body.display_name,
                &rules,
                only_flagged,
                only_attachments,
            )
            .await;
            (folder_id, created)
        }
    };
    match result {
        Ok(()) => Json(VirtualFolderSaved { folder_id }).into_response(),
        Err(e) if e.to_string() == "not found" => {
            (StatusCode::NOT_FOUND, "not found").into_response()
        }
        Err(e) => {
            eprintln!("[email] save_virtual_folder error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FolderIdQuery {
    pub folder_id: String,
}

#[utoipa::path(
    delete,
    path = "/email/folder",
    params(FolderIdQuery),
    responses((status = 204, description = "deleted"), (status = 500, description = "database error"))
)]
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

/// The mail routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_accounts))
        .routes(routes!(save_account, delete_account))
        .routes(routes!(list_mailboxes))
        .routes(routes!(trigger_mailbox_sync))
        .routes(routes!(get_unread_counts))
        .routes(routes!(list_virtual_folders))
        .routes(routes!(save_virtual_folder, delete_virtual_folder))
        .routes(routes!(list_identities))
        .routes(routes!(save_identity, delete_identity))
        .merge(messages::routes())
        .merge(attachments::routes())
        .merge(send::routes())
}

/// The public mail routes, relative to `/api` (api.rs).
pub fn public_routes() -> OpenApiRouter<AppState> {
    attachments::public_routes()
}

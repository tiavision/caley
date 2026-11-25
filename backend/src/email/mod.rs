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

pub(crate) mod attachments;
pub(crate) mod cut;
pub(crate) mod messages;
pub(crate) mod send;

// ── Accounts ──────────────────────────────────────────────────────────────────

/// The user's mail account `account_id`: the one way a handler turns an id from a request into an account, whose
/// [`email_db::ImapAccountRow::id`] the account's data functions require. 404 when it is not the user's (or does
/// not exist: the two are not told apart).
pub(crate) async fn owned_account(
    state: &AppState,
    user_id: &str,
    account_id: &str,
) -> Result<email_db::ImapAccountRow, axum::response::Response> {
    match email_db::get_account(&state.db, account_id, user_id, &state.encryption_key).await {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err((StatusCode::NOT_FOUND, "account not found").into_response()),
        Err(e) => {
            eprintln!("[email] account lookup: {e}");
            Err((StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response())
        }
    }
}

/// A failed save whose error is [`email_db::NotOwned`] (an id of another user's) is a 404, as a lookup of it is.
pub(crate) fn save_error(context: &str, e: anyhow::Error) -> axum::response::Response {
    if e.is::<email_db::NotOwned>() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    eprintln!("[email] {context} error: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
}

/// A mail account, IMAP or JMAP (its password is never sent).
#[derive(Serialize, ToSchema)]
pub struct Account {
    pub account_id: String,
    pub display_name: String,
    /// `imap` or `jmap`
    #[schema(value_type = Protocol)]
    pub protocol: String,
    /// a JMAP account's session URL; `null` for IMAP
    #[schema(required)]
    pub jmap_url: Option<String>,
    /// how a JMAP account signs in: `password`, `bearer` (an API token), `oidc` (the user's OIDC sign-in) or
    /// `oauth` (the server's own sign-in); `password` for IMAP
    #[schema(value_type = AccountAuth)]
    pub auth: String,
    /// `oauth`: the OAuth client the server has for Caley (its secret is never sent)
    #[schema(required)]
    pub oauth_client_id: Option<String>,
    /// IMAP server (JMAP: the host of its URL)
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

/// A mail account's protocol.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Imap,
    Jmap,
}

/// How a JMAP account signs in.
#[derive(Serialize, Deserialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AccountAuth {
    /// HTTP Basic with the username and password
    Password,
    /// the password is an API token, sent as a bearer token
    Bearer,
    /// the user's own OIDC sign-in: its access token, renewed with its refresh token
    Oidc,
    /// the server's own sign-in (OAuth authorization code with PKCE), with the OAuth client the server has for
    /// Caley; its tokens belong to the account
    Oauth,
}

impl AccountAuth {
    fn as_str(self) -> &'static str {
        match self {
            AccountAuth::Password => "password",
            AccountAuth::Bearer => "bearer",
            AccountAuth::Oidc => "oidc",
            AccountAuth::Oauth => "oauth",
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct AccountsResponse {
    pub accounts: Vec<Account>,
}

#[derive(Serialize, ToSchema)]
pub struct AccountSaved {
    pub account_id: String,
    /// an account that signs in at the server (`oauth`) and is to be authorized there: not saved yet, the page goes
    /// to this link (Caley's, which redirects to the server's sign-in; the account is saved when it comes back)
    #[schema(required)]
    pub authorize: Option<String>,
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
                    protocol: a.protocol,
                    jmap_url: a.jmap_url,
                    auth: a.auth,
                    oauth_client_id: a.oauth_client_id,
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

/// A mail account to add (no `account_id`) or update.
#[derive(Deserialize, ToSchema)]
pub struct AccountBody {
    pub account_id: Option<String>,
    pub display_name: String,
    /// default `imap`
    pub protocol: Option<Protocol>,
    /// JMAP: the session URL, or the server's host (`https://host/.well-known/jmap`)
    pub jmap_url: Option<String>,
    /// JMAP: how it signs in, default `password`
    pub auth: Option<AccountAuth>,
    /// IMAP server (ignored for JMAP: taken from its URL)
    pub host: String,
    pub port: u16,
    pub username: String,
    /// empty or missing: keep the stored one (required for a new account, unless it signs in with OIDC)
    pub password: Option<String>,
    /// default true
    pub use_tls: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    /// a new account's default identity gets this name (none without it)
    pub sender_name: Option<String>,
    /// `oauth`: the OAuth client the server has for Caley
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    /// its secret; empty or missing: keep the stored one (none for a public client)
    #[serde(default)]
    pub oauth_client_secret: Option<String>,
    /// `oauth`: sign in at the server again (a new account, or one on another server or client, always does)
    #[serde(default)]
    pub authorize: Option<bool>,
}

/// Adds (no `account_id`) or updates a mail account; PUT is the same. A JMAP account is saved only once its server
/// has answered its session with the account's sign-in; one that signs in at the server (`oauth`) and is to be
/// authorized is saved when the server's sign-in comes back (`authorize`).
#[utoipa::path(
    method(post, put),
    path = "/email/account",
    request_body = AccountBody,
    responses(
        (status = 200, body = AccountSaved),
        (status = 400, description = "password required for new account, or the JMAP server refused the sign-in or \
            could not be reached (plain text)"),
        (status = 500, description = "database error")
    )
)]
pub async fn save_account(
    State(state): State<AppState>,
    user: CurrentUser,
    session: tower_sessions::Session,
    crate::public_origin::RequestOrigin(origin): crate::public_origin::RequestOrigin,
    Json(body): Json<AccountBody>,
) -> impl IntoResponse {
    let is_new = body.account_id.is_none();
    let account_id = body
        .account_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut use_tls = body.use_tls.unwrap_or(true);
    let protocol = body.protocol.unwrap_or(Protocol::Imap);
    let auth = match protocol {
        Protocol::Imap => AccountAuth::Password,
        Protocol::Jmap => body.auth.unwrap_or(AccountAuth::Password),
    };

    // An account_id names an account of the user's to update: never one to create (the ids are the server's) nor
    // another user's.
    let existing = if is_new {
        None
    } else {
        match owned_account(&state, &user.user_id, &account_id).await {
            Ok(existing) => Some(existing),
            Err(response) => return response,
        }
    };
    if auth == AccountAuth::Oauth {
        return save_oauth_account(
            &state,
            &user.user_id,
            &session,
            &origin,
            account_id,
            existing,
            body,
        )
        .await;
    }
    let password = match body.password.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None if auth == AccountAuth::Oidc => String::new(),
        None => match &existing {
            Some(existing) => existing.password.clone(),
            None => {
                return (StatusCode::BAD_REQUEST, "password required for new account")
                    .into_response();
            }
        },
    };

    let (mut host, mut port, mut username) = (body.host, body.port, body.username);
    let mut jmap_url = None;
    if protocol == Protocol::Jmap {
        let checked = match check_jmap(
            &state,
            &user.user_id,
            body.jmap_url.as_deref().unwrap_or_default(),
            auth,
            &username,
            &password,
        )
        .await
        {
            Ok(checked) => checked,
            Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
        };
        host = checked.url.host_str().unwrap_or_default().to_owned();
        port = checked.url.port_or_known_default().unwrap_or(443);
        use_tls = checked.url.scheme() == "https";
        if username.is_empty() {
            username = checked.username.unwrap_or_default();
        }
        jmap_url = Some(checked.url.to_string());
    }

    // another protocol or JMAP server: what the cache holds is of the old one (an IMAP server's UIDVALIDITY tells
    // the sync itself). A sync of the old server still running finishes first, and none starts until the new one is
    // saved (it would read the old account again).
    let mut _no_sync = None;
    if let Some(existing) = &existing {
        let protocol_name = if protocol == Protocol::Jmap {
            "jmap"
        } else {
            "imap"
        };
        let moved = existing.protocol != protocol_name
            || (protocol == Protocol::Jmap && existing.jmap_url != jmap_url);
        if moved {
            _no_sync = Some(state.email_sync_manager.jmap_lock(&account_id).await);
            state.email_sync_manager.stop_idle_for(&account_id);
            let _ = tokio::fs::remove_dir_all(crate::email_sync::account_dir(
                &state.data_dir,
                existing.id(),
            ))
            .await;
            if let Err(e) = email_db::reset_account_cache(&state.db, existing.id()).await {
                eprintln!("[email] save_account cache reset error: {e}");
            }
        }
    }

    if let Err(e) = email_db::upsert_account(
        &state.db,
        &user.user_id,
        &account_id,
        &body.display_name,
        &host,
        port,
        &username,
        &password,
        use_tls,
        body.smtp_host
            .as_deref()
            .filter(|_| protocol == Protocol::Imap),
        body.smtp_port.filter(|_| protocol == Protocol::Imap),
        &email_db::AccountProtocol {
            protocol: match protocol {
                Protocol::Imap => "imap",
                Protocol::Jmap => "jmap",
            },
            jmap_url: jmap_url.as_deref(),
            auth: auth.as_str(),
            oauth_client_id: None,
            oauth_client_secret: None,
        },
        &state.encryption_key,
    )
    .await
    {
        return save_error("save_account", e);
    }

    if is_new {
        if let (Some(name), Ok(account)) = (
            body.sender_name.filter(|n| !n.is_empty()),
            owned_account(&state, &user.user_id, &account_id).await,
        ) {
            if let Err(e) = email_db::upsert_identity(
                &state.db,
                &Uuid::new_v4().to_string(),
                account.id(),
                &user.user_id,
                &name,
                &username,
                true,
            )
            .await
            {
                eprintln!("[email] save_account: failed to create default identity: {e}");
            }
        }
    }

    if !is_new {
        // new credentials or a new server: the next sync connects with them
        state.email_sync_manager.stop_idle_for(&account_id);
    }
    state.email_sync_manager.ensure_task_for(&account_id);

    state
        .sse_broadcaster
        .send(&user.user_id, SseMessage::AccountsChanged);
    Json(AccountSaved {
        account_id,
        authorize: None,
    })
    .into_response()
}

/// Saves a JMAP account that signs in at the server (`oauth`). A new one, one on another server or client (or
/// with another sign-in before), one without tokens, or one asked to (`authorize`) is to be authorized: kept in the
/// session and answered with the link that starts it (jmap/oauth.rs). Otherwise its other fields are saved and its
/// tokens kept.
async fn save_oauth_account(
    state: &AppState,
    user_id: &str,
    session: &tower_sessions::Session,
    origin: &str,
    account_id: String,
    existing: Option<email_db::ImapAccountRow>,
    body: AccountBody,
) -> axum::response::Response {
    use crate::jmap::oauth;
    let url = match crate::jmap::session_url(body.jmap_url.as_deref().unwrap_or_default()) {
        Ok(url) => url,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    let client_id = body
        .oauth_client_id
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .or_else(|| existing.as_ref().and_then(|e| e.oauth_client_id.clone()));
    let Some(client_id) = client_id else {
        return (
            StatusCode::BAD_REQUEST,
            "the server's OAuth client id is required",
        )
            .into_response();
    };
    let same_client = existing
        .as_ref()
        .is_some_and(|e| e.oauth_client_id.as_deref() == Some(client_id.as_str()));
    let client_secret = body
        .oauth_client_secret
        .filter(|s| !s.is_empty())
        .or_else(|| {
            existing
                .as_ref()
                .filter(|_| same_client)
                .and_then(|e| e.oauth_client_secret.clone())
        });
    let username = if body.username.is_empty() {
        existing
            .as_ref()
            .map(|e| e.username.clone())
            .unwrap_or_default()
    } else {
        body.username
    };

    let authorize = match &existing {
        None => true,
        Some(e) => {
            body.authorize == Some(true)
                || e.auth != "oauth"
                || e.jmap_url.as_deref() != Some(url.as_str())
                || !same_client
                || !oauth::has_tokens(&state.db, &account_id).await
        }
    };
    if authorize && !state.demo_mode {
        let endpoints = match oauth::discover(&state.email_sync_manager.http, &url).await {
            Ok(e) => e,
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        };
        let client_secret = match client_secret
            .map(|s| crate::crypto::encrypt(&s, &state.encryption_key))
            .transpose()
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[email] save_account encrypt: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        };
        let pending = oauth::Pending {
            account_id: account_id.clone(),
            is_new: existing.is_none(),
            display_name: body.display_name,
            sender_name: body.sender_name,
            jmap_url: url.to_string(),
            username,
            client_id,
            client_secret,
            endpoints,
            redirect_uri: format!("{origin}/auth/jmap"),
            verifier: String::new(),
            created_at: 0,
        };
        return match oauth::begin(session, pending).await {
            Ok(link) => Json(AccountSaved {
                account_id,
                authorize: Some(link),
            })
            .into_response(),
            Err(e) => {
                eprintln!("[email] save_account session: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "session error").into_response()
            }
        };
    }

    if let Err(e) = email_db::upsert_account(
        &state.db,
        user_id,
        &account_id,
        &body.display_name,
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or(443),
        &username,
        "",
        url.scheme() == "https",
        None,
        None,
        &email_db::AccountProtocol {
            protocol: "jmap",
            jmap_url: Some(url.as_str()),
            auth: "oauth",
            oauth_client_id: Some(&client_id),
            oauth_client_secret: client_secret.as_deref(),
        },
        &state.encryption_key,
    )
    .await
    {
        return save_error("save_account", e);
    }
    state
        .sse_broadcaster
        .send(user_id, SseMessage::AccountsChanged);
    Json(AccountSaved {
        account_id,
        authorize: None,
    })
    .into_response()
}

/// A JMAP server that answered its session.
struct CheckedJmap {
    url: url::Url,
    /// the session's username
    username: Option<String>,
}

/// Fetches a JMAP account's session with its sign-in: the error text for the account dialog otherwise.
async fn check_jmap(
    state: &AppState,
    user_id: &str,
    input: &str,
    auth: AccountAuth,
    username: &str,
    password: &str,
) -> Result<CheckedJmap, String> {
    let url = crate::jmap::session_url(input).map_err(|e| e.to_string())?;
    if state.demo_mode {
        return Ok(CheckedJmap {
            url,
            username: None,
        });
    }
    let auth = match auth {
        AccountAuth::Password => crate::jmap::Auth::Basic {
            username: username.to_owned(),
            password: password.to_owned(),
        },
        AccountAuth::Bearer => crate::jmap::Auth::Bearer(password.to_owned()),
        AccountAuth::Oidc => match state.oidc_tokens.access_token(user_id).await {
            Ok(token) => crate::jmap::Auth::Bearer(token),
            Err(e) => return Err(e.to_string()),
        },
        // checked when its sign-in at the server comes back (jmap/oauth.rs)
        AccountAuth::Oauth => return Err("an OAuth account is saved through its sign-in".into()),
    };
    match crate::jmap::Client::connect(&state.email_sync_manager.http, url.as_str(), auth).await {
        Ok(client) => Ok(CheckedJmap {
            url,
            username: client.session.username.clone(),
        }),
        Err(e) if crate::jmap::is_unauthorized(&e) => Err(e.to_string()),
        Err(e) => Err(format!("the JMAP server could not be used: {e}")),
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AccountIdQuery {
    pub account_id: String,
}

/// Deletes a mail account with its cached mail.
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
    // nothing of the account is touched before it is known to be the user's
    let account = match owned_account(&state, &user.user_id, &q.account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };
    // Delete all .eml files for this account
    let _ = tokio::fs::remove_dir_all(crate::email_sync::account_dir(
        &state.data_dir,
        account.id(),
    ))
    .await;

    state.email_sync_manager.stop_idle_for(&q.account_id);
    state.email_sync_manager.stop_task_for(&q.account_id);

    if let Err(e) = email_db::delete_account(&state.db, account.id()).await {
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
    let account = match owned_account(&state, &user.user_id, &body.account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };

    // An identity_id names an identity of the user's to update, never one to create.
    let identity_id = match body.identity_id {
        Some(id) => match email_db::get_identity(&state.db, &id, &user.user_id).await {
            Ok(Some(_)) => id,
            Ok(None) => return (StatusCode::NOT_FOUND, "identity not found").into_response(),
            Err(e) => return save_error("save_identity lookup", e),
        },
        None => Uuid::new_v4().to_string(),
    };
    let is_default = body.is_default.unwrap_or(false);

    if let Err(e) = email_db::upsert_identity(
        &state.db,
        &identity_id,
        account.id(),
        &user.user_id,
        &body.name,
        &body.email,
        is_default,
    )
    .await
    {
        return save_error("save_identity", e);
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
    let account = match owned_account(&state, &user.user_id, &q.account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };

    match email_db::list_mailboxes(&state.db, account.id()).await {
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
    if let Err(response) = owned_account(&state, &user.user_id, &q.account_id).await {
        return response;
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
    // a rule names one of the user's accounts (or none: all of them)
    for account_id in body.rules.iter().filter_map(|r| r.account_id.as_deref()) {
        if !account_id.is_empty() {
            if let Err(response) = owned_account(&state, &user.user_id, account_id).await {
                return response;
            }
        }
    }

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

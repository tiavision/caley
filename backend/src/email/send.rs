use std::collections::HashSet;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::Deserialize;
use serde_json::json;

use crate::email_autodiscover;
use crate::email_db;
use crate::email_sync::EmailSyncTrigger;
use crate::imap_client;
use crate::oidc::{AppState, CurrentUser};

// ── Send ──────────────────────────────────────────────────────────────────────

pub async fn send_email(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let mut account_id = String::new();
    let mut identity_id = String::new();
    let mut to_raw = String::new();
    let mut cc_raw = String::new();
    let mut bcc_raw = String::new();
    let mut subject = String::new();
    let mut body = String::new();
    let mut in_reply_to: Option<String> = None;
    let mut references: Option<String> = None;
    let mut attachments: Vec<(String, String, Vec<u8>)> = vec![];

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_owned();
                if name == "attachment" {
                    let filename = field.file_name().unwrap_or("attachment").to_owned();
                    let ct = field
                        .content_type()
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
                        "identity_id" => identity_id = text,
                        "to" => to_raw = text,
                        "cc" => cc_raw = text,
                        "bcc" => bcc_raw = text,
                        "subject" => subject = text,
                        "body" => body = text,
                        "in_reply_to" if !text.is_empty() => in_reply_to = Some(text),
                        "references" if !text.is_empty() => references = Some(text),
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
        s.split(',')
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect()
    };
    let to = split_addrs(&to_raw);
    let cc = split_addrs(&cc_raw);
    let bcc = split_addrs(&bcc_raw);

    // Resolve identity → account + from header
    let (account, from) = if !identity_id.is_empty() {
        let identity = match email_db::get_identity(&state.db, &identity_id, &user.user_id).await {
            Ok(Some(i)) => i,
            Ok(None) => return (StatusCode::NOT_FOUND, "identity not found").into_response(),
            Err(e) => {
                eprintln!("[email] send_email identity lookup: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        };
        let acc = match email_db::get_account(
            &state.db,
            &identity.account_id,
            &user.user_id,
            &state.encryption_key,
        )
        .await
        {
            Ok(Some(a)) => a,
            Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
            Err(e) => {
                eprintln!("[email] send_email account lookup: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        };
        let from = format!("{} <{}>", identity.name, identity.email);
        (acc, from)
    } else {
        // Fallback: legacy account_id path
        if account_id.is_empty() {
            return (StatusCode::BAD_REQUEST, "missing identity_id or account_id").into_response();
        }
        let acc = match email_db::get_account(
            &state.db,
            &account_id,
            &user.user_id,
            &state.encryption_key,
        )
        .await
        {
            Ok(Some(a)) => a,
            Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
            Err(e) => {
                eprintln!("[email] send_email account lookup: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
            }
        };
        let from = format!(
            "{} <{}>",
            user.display_name.as_deref().unwrap_or(&acc.username),
            acc.username
        );
        (acc, from)
    };

    if state.demo_mode {
        eprintln!("[demo] send_email suppressed: to={to:?} subject={subject:?}");
        return StatusCode::OK.into_response();
    }

    let smtp_host = account.smtp_host.as_deref().unwrap_or(&account.host);
    let smtp_port = account.smtp_port.unwrap_or(587) as u16;

    let raw_message = match imap_client::send_email(
        smtp_host,
        smtp_port,
        &account.username,
        &account.password,
        &from,
        &to,
        &cc,
        &bcc,
        &subject,
        &body,
        in_reply_to.as_deref(),
        references.as_deref(),
        attachments,
    )
    .await
    {
        Ok(raw_message) => raw_message,
        Err(e) => {
            eprintln!("[email] send_email error: {e}");
            return (StatusCode::BAD_GATEWAY, format!("Failed to send: {e}")).into_response();
        }
    };

    match imap_client::append_to_sent(
        &account.host,
        account.port as u16,
        &account.username,
        &account.password,
        &raw_message,
    )
    .await
    {
        Ok(Some(mailbox)) => {
            state
                .email_sync_manager
                .trigger(&account.account_id, EmailSyncTrigger::Mailbox(mailbox));
        }
        Ok(None) => eprintln!(
            "[email] sent mailbox not found for account {}",
            account.account_id
        ),
        Err(e) => eprintln!("[email] failed to append sent message: {e}"),
    }

    StatusCode::OK.into_response()
}

// ── Raw send (client-built signed/encrypted MIME) ────────────────────────────

pub async fn send_raw_email(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let mut identity_id = String::new();
    let mut envelope_to: Vec<String> = vec![];
    let mut envelope_cc: Vec<String> = vec![];
    let mut envelope_bcc: Vec<String> = vec![];
    let mut raw_mime: Vec<u8> = vec![];

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_owned();
                if name == "raw_mime" {
                    match field.bytes().await {
                        Ok(bytes) => raw_mime = bytes.to_vec(),
                        Err(_) => {
                            return (StatusCode::BAD_REQUEST, "invalid raw mime").into_response();
                        }
                    }
                    continue;
                }
                let text = match field.text().await {
                    Ok(text) => text,
                    Err(_) => {
                        return (StatusCode::BAD_REQUEST, "invalid multipart data").into_response();
                    }
                };
                match name.as_str() {
                    "identity_id" => identity_id = text,
                    "envelope_to" => match serde_json::from_str(&text) {
                        Ok(addresses) => envelope_to = addresses,
                        Err(_) => {
                            return (StatusCode::BAD_REQUEST, "invalid envelope recipients")
                                .into_response();
                        }
                    },
                    "envelope_cc" => match serde_json::from_str(&text) {
                        Ok(addresses) => envelope_cc = addresses,
                        Err(_) => {
                            return (StatusCode::BAD_REQUEST, "invalid envelope recipients")
                                .into_response();
                        }
                    },
                    "envelope_bcc" => match serde_json::from_str(&text) {
                        Ok(addresses) => envelope_bcc = addresses,
                        Err(_) => {
                            return (StatusCode::BAD_REQUEST, "invalid envelope recipients")
                                .into_response();
                        }
                    },
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid multipart data").into_response(),
        }
    }

    if identity_id.is_empty() || raw_mime.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing identity_id or raw_mime").into_response();
    }

    let identity = match email_db::get_identity(&state.db, &identity_id, &user.user_id).await {
        Ok(Some(identity)) => identity,
        Ok(None) => return (StatusCode::NOT_FOUND, "identity not found").into_response(),
        Err(e) => {
            eprintln!("[email] send_raw identity lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };
    let account = match email_db::get_account(
        &state.db,
        &identity.account_id,
        &user.user_id,
        &state.encryption_key,
    )
    .await
    {
        Ok(Some(account)) => account,
        Ok(None) => return (StatusCode::NOT_FOUND, "account not found").into_response(),
        Err(e) => {
            eprintln!("[email] send_raw account lookup: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let parsed = match mail_parser::MessageParser::default().parse(&raw_mime) {
        Some(parsed) => parsed,
        None => return (StatusCode::BAD_REQUEST, "invalid raw mime").into_response(),
    };
    if parsed.bcc().is_some() {
        return (
            StatusCode::BAD_REQUEST,
            "Bcc header is not permitted in raw MIME",
        )
            .into_response();
    }
    let from_addresses = parsed.from().map(raw_address_emails).unwrap_or_default();
    if from_addresses.len() != 1 || !from_addresses[0].eq_ignore_ascii_case(&identity.email) {
        return (
            StatusCode::BAD_REQUEST,
            "From does not match selected identity",
        )
            .into_response();
    }

    let mut seen = HashSet::new();
    let recipients: Vec<String> = envelope_to
        .into_iter()
        .chain(envelope_cc)
        .chain(envelope_bcc)
        .filter_map(|address| {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                return None;
            }
            let lower = trimmed.to_ascii_lowercase();
            seen.insert(lower).then(|| trimmed.to_owned())
        })
        .collect();
    if recipients.is_empty() {
        return (StatusCode::BAD_REQUEST, "no recipients").into_response();
    }
    if recipients.len() > 100 {
        return (StatusCode::BAD_REQUEST, "too many recipients").into_response();
    }
    if recipients
        .iter()
        .any(|recipient| recipient.parse::<lettre::Address>().is_err())
    {
        return (StatusCode::BAD_REQUEST, "invalid recipient address").into_response();
    }

    if state.demo_mode {
        eprintln!("[demo] send_raw_email suppressed for identity {}", identity.email);
        return StatusCode::OK.into_response();
    }

    let smtp_host = account.smtp_host.as_deref().unwrap_or(&account.host);
    let smtp_port = account.smtp_port.unwrap_or(587) as u16;
    if let Err(e) = imap_client::send_raw_email(
        smtp_host,
        smtp_port,
        &account.username,
        &account.password,
        &identity.email,
        &recipients,
        &raw_mime,
    )
    .await
    {
        eprintln!("[email] send_raw error: {e}");
        return (StatusCode::BAD_GATEWAY, format!("Failed to send: {e}")).into_response();
    }

    match imap_client::append_to_sent(
        &account.host,
        account.port as u16,
        &account.username,
        &account.password,
        &raw_mime,
    )
    .await
    {
        Ok(Some(mailbox)) => {
            state
                .email_sync_manager
                .trigger(&account.account_id, EmailSyncTrigger::Mailbox(mailbox));
        }
        Ok(None) => eprintln!(
            "[email] sent mailbox not found for account {}",
            account.account_id
        ),
        Err(e) => eprintln!("[email] failed to append sent message: {e}"),
    }

    StatusCode::OK.into_response()
}

fn raw_address_emails(address: &mail_parser::Address<'_>) -> Vec<String> {
    let email = |addr: &mail_parser::Addr<'_>| {
        addr.address
            .as_ref()
            .map(|address| address.trim().to_owned())
            .filter(|address| !address.is_empty())
    };
    match address {
        mail_parser::Address::List(addresses) => addresses.iter().filter_map(email).collect(),
        mail_parser::Address::Group(groups) => groups
            .iter()
            .flat_map(|group| group.addresses.iter().filter_map(email))
            .collect(),
    }
}

// ── Autodiscover ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AutodiscoverQuery {
    pub email: String,
}

pub async fn autodiscover_handler(
    State(state): State<AppState>,
    _user: CurrentUser,
    Query(params): Query<AutodiscoverQuery>,
) -> impl IntoResponse {
    if state.demo_mode {
        let local = params.email.split('@').next().unwrap_or(&params.email);
        return Json(json!({
            "found": true,
            "provider_name": "Demo Mail",
            "imap_host": "imap.demo.example",
            "imap_port": 993,
            "imap_use_tls": true,
            "imap_username": local,
            "smtp_host": "smtp.demo.example",
            "smtp_port": 587,
        }))
        .into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(crate::version::USER_AGENT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return Json(json!({ "found": false })).into_response(),
    };

    match email_autodiscover::autodiscover(&params.email, &client).await {
        Some(r) => Json(json!({
            "found": true,
            "provider_name": r.provider_name,
            "imap_host": r.imap_host,
            "imap_port": r.imap_port,
            "imap_use_tls": r.imap_use_tls,
            "imap_username": r.imap_username,
            "smtp_host": r.smtp_host,
            "smtp_port": r.smtp_port,
        }))
        .into_response(),
        None => Json(json!({ "found": false })).into_response(),
    }
}

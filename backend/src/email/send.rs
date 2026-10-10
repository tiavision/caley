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

// ── Raw send ─────────────────────────────────────────────────────────────────

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

    let smtp_from = match normalize_envelope_address(&identity.email) {
        Ok(Some(address)) => address,
        Ok(None) | Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "selected identity address requires SMTPUTF8",
            )
                .into_response();
        }
    };

    let parsed = match mail_parser::MessageParser::default().parse(&raw_mime) {
        Some(parsed) => parsed,
        None => return (StatusCode::BAD_REQUEST, "invalid raw mime").into_response(),
    };
    if !raw_mime.is_ascii() {
        return (
            StatusCode::BAD_REQUEST,
            "raw MIME contains non-ASCII bytes; use MIME transfer encoding",
        )
            .into_response();
    }
    if parsed.bcc().is_some() {
        return (
            StatusCode::BAD_REQUEST,
            "Bcc header is not permitted in raw MIME",
        )
            .into_response();
    }
    let from_addresses = parsed.from().map(raw_address_emails).unwrap_or_default();
    let normalized_from = from_addresses
        .first()
        .and_then(|address| normalize_envelope_address(address).ok().flatten());
    if from_addresses.len() != 1
        || !normalized_from
            .as_deref()
            .is_some_and(|from| from.eq_ignore_ascii_case(&smtp_from))
    {
        return (
            StatusCode::BAD_REQUEST,
            "From does not match selected identity",
        )
            .into_response();
    }

    let mut seen = HashSet::new();
    let mut recipients = Vec::new();
    for address in envelope_to
        .into_iter()
        .chain(envelope_cc)
        .chain(envelope_bcc)
    {
        let Some(normalized) = (match normalize_envelope_address(&address) {
            Ok(address) => address,
            Err(NormalizeAddressError::SmtpUtf8Required) => {
                return (
                    StatusCode::BAD_REQUEST,
                    "recipient address requires SMTPUTF8",
                )
                    .into_response();
            }
            Err(NormalizeAddressError::Invalid) => {
                return (StatusCode::BAD_REQUEST, "invalid recipient address").into_response();
            }
        }) else {
            continue;
        };
        let lower = normalized.to_ascii_lowercase();
        if seen.insert(lower) {
            recipients.push(normalized);
        }
    }
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
        eprintln!(
            "[demo] send_raw_email suppressed for identity {}",
            identity.email
        );
        return StatusCode::OK.into_response();
    }

    let smtp_host = account.smtp_host.as_deref().unwrap_or(&account.host);
    let smtp_port = account.smtp_port.unwrap_or(587) as u16;
    if let Err(e) = imap_client::send_raw_email(
        smtp_host,
        smtp_port,
        &account.username,
        &account.password,
        &smtp_from,
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

#[derive(Debug, PartialEq, Eq)]
enum NormalizeAddressError {
    Invalid,
    SmtpUtf8Required,
}

fn normalize_envelope_address(value: &str) -> Result<Option<String>, NormalizeAddressError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let address = trimmed
        .find('<')
        .and_then(|start| {
            trimmed[start + 1..]
                .find('>')
                .map(|end| &trimmed[start + 1..start + 1 + end])
        })
        .unwrap_or(trimmed)
        .trim();
    let (local, domain) = address
        .rsplit_once('@')
        .ok_or(NormalizeAddressError::Invalid)?;
    if local.is_empty() || domain.is_empty() {
        return Err(NormalizeAddressError::Invalid);
    }
    if !local.is_ascii() {
        return Err(NormalizeAddressError::SmtpUtf8Required);
    }
    let ascii_domain = idna::domain_to_ascii(domain).map_err(|_| NormalizeAddressError::Invalid)?;
    if ascii_domain.is_empty() || !ascii_domain.is_ascii() {
        return Err(NormalizeAddressError::Invalid);
    }
    let normalized = format!("{local}@{ascii_domain}");
    normalized
        .parse::<lettre::Address>()
        .map_err(|_| NormalizeAddressError::Invalid)?;
    Ok(Some(normalized))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ascii_address() {
        assert_eq!(
            normalize_envelope_address("User@example.com"),
            Ok(Some("User@example.com".to_owned()))
        );
    }

    #[test]
    fn normalize_idn_domain_to_punycode() {
        assert_eq!(
            normalize_envelope_address("user@bücher.example"),
            Ok(Some("user@xn--bcher-kva.example".to_owned()))
        );
    }

    #[test]
    fn normalize_address_with_display_name() {
        assert_eq!(
            normalize_envelope_address("Example User <user@example.com>"),
            Ok(Some("user@example.com".to_owned()))
        );
    }

    #[test]
    fn reject_non_ascii_local_part() {
        assert_eq!(
            normalize_envelope_address("müller@example.com"),
            Err(NormalizeAddressError::SmtpUtf8Required)
        );
    }

    #[test]
    fn reject_invalid_address() {
        assert_eq!(
            normalize_envelope_address("not an address"),
            Err(NormalizeAddressError::Invalid)
        );
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
        .use_preconfigured_tls(crate::imap_client::tls_config())
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

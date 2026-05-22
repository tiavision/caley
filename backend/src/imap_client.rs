use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_imap::Client;
use futures_util::StreamExt;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::{rustls, TlsConnector};

use crate::email_db::MessageHeader;
use crate::search::SearchMode;

pub type ImapSession = async_imap::Session<tokio_rustls::client::TlsStream<TcpStream>>;

fn tls_connector() -> Result<TlsConnector> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

pub async fn connect(host: &str, port: u16, username: &str, password: &str) -> Result<ImapSession> {
    let connector = tls_connector()?;
    let tcp = TcpStream::connect((host, port)).await?;
    let server_name: ServerName<'static> = ServerName::try_from(host.to_owned())
        .map_err(|_| anyhow!("invalid IMAP hostname: {host}"))?;
    let tls_stream = connector.connect(server_name, tcp).await?;
    let client = Client::new(tls_stream);
    let session = client
        .login(username, password)
        .await
        .map_err(|(e, _)| anyhow!("IMAP login failed: {e}"))?;
    Ok(session)
}

/// Returns (name, delimiter, subscribed) for every mailbox on the server.
pub async fn list_mailboxes(session: &mut ImapSession) -> Result<Vec<(String, Option<String>, bool)>> {
    let mut stream = session.list(None, Some("*")).await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let name_info = item?;
        let name = name_info.name().to_owned();
        let delim = name_info.delimiter().map(|d| d.to_owned());
        let subscribed = !name_info.attributes().iter().any(|a| {
            matches!(a, async_imap::types::NameAttribute::NoSelect)
        });
        out.push((name, delim, subscribed));
    }
    Ok(out)
}

/// Selects a mailbox and returns its UIDVALIDITY.
pub async fn select_mailbox(session: &mut ImapSession, mailbox: &str) -> Result<(u32, u32)> {
    let mb = session.select(mailbox).await?;
    let uidvalidity = mb.uid_validity.unwrap_or(0);
    let exists = mb.exists;
    Ok((uidvalidity, exists))
}

/// Returns all UIDs in the currently-selected mailbox.
pub async fn fetch_all_uids(session: &mut ImapSession) -> Result<Vec<u32>> {
    let uid_set = session.uid_search("ALL").await?;
    let mut v: Vec<u32> = uid_set.into_iter().collect();
    v.sort_unstable();
    Ok(v)
}

/// Fetches headers for a list of UIDs. Returns parsed MessageHeaders.
pub async fn fetch_headers(
    session: &mut ImapSession,
    uids: &[u32],
    search_mode: &SearchMode,
) -> Result<Vec<MessageHeader>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let uid_set = uids_to_set(uids);
    let fetch_items =
        "(FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID REFERENCES IN-REPLY-TO)])";

    let mut stream = session.uid_fetch(&uid_set, fetch_items).await?;
    let mut headers = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item?;
        let uid = match fetch.uid {
            Some(u) => u,
            None => continue,
        };

        let flags = fetch
            .flags()
            .map(|f| match f {
                async_imap::types::Flag::Seen => "\\Seen".to_owned(),
                async_imap::types::Flag::Answered => "\\Answered".to_owned(),
                async_imap::types::Flag::Flagged => "\\Flagged".to_owned(),
                async_imap::types::Flag::Deleted => "\\Deleted".to_owned(),
                async_imap::types::Flag::Draft => "\\Draft".to_owned(),
                async_imap::types::Flag::Recent => "\\Recent".to_owned(),
                async_imap::types::Flag::Custom(c) => c.to_string(),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let flags_json = serde_json::to_string(&flags).unwrap_or_default();

        let (message_id, thread_id, from_addr, to_addrs, cc_addrs, subject, sent_at) =
            if let Some(header_bytes) = fetch.header() {
                parse_header_bytes(header_bytes)
            } else {
                Default::default()
            };

        let subject_idx = subject.as_deref().map(|s| search_mode.normalize_text(s));
        let from_idx = from_addr.as_deref().map(|s| search_mode.normalize_text(s));

        headers.push(MessageHeader {
            uid: uid as i64,
            message_id,
            thread_id,
            from_addr,
            to_addrs: to_addrs.map(|v| serde_json::to_string(&v).unwrap_or_default()),
            cc_addrs: cc_addrs.map(|v| serde_json::to_string(&v).unwrap_or_default()),
            subject,
            sent_at,
            flags: Some(flags_json),
            has_attachments: false,
            // body and idx fields not used in header struct; stored separately
        });
        let _ = subject_idx;
        let _ = from_idx;
    }
    Ok(headers)
}

/// Fetches the full raw body for a single UID. Returns (body_text, has_attachments).
pub async fn fetch_body(session: &mut ImapSession, uid: u32) -> Result<(String, bool)> {
    let mut stream = session.uid_fetch(uid.to_string(), "(BODY.PEEK[])").await?;
    if let Some(item) = stream.next().await {
        let fetch = item?;
        if let Some(body_bytes) = fetch.body() {
            return parse_body_bytes(body_bytes);
        }
    }
    Ok((String::new(), false))
}

/// Stores the \Deleted flag and expunges on the server.
pub async fn delete_on_server(session: &mut ImapSession, uid: u32) -> Result<()> {
    let uid_str = uid.to_string();
    {
        // Scope the stream so the borrow on session ends before expunge
        let mut stream = session.uid_store(&uid_str, "+FLAGS (\\Deleted)").await?;
        while let Some(r) = stream.next().await { let _ = r; }
    }
    session.expunge().await?;
    Ok(())
}

/// Sets or clears flags on the server.
pub async fn set_flags(session: &mut ImapSession, uid: u32, flags: &[&str], set: bool) -> Result<()> {
    let uid_str = uid.to_string();
    let flag_list = flags.join(" ");
    let cmd = if set {
        format!("+FLAGS ({flag_list})")
    } else {
        format!("-FLAGS ({flag_list})")
    };
    let mut stream = session.uid_store(&uid_str, &cmd).await?;
    while let Some(r) = stream.next().await {
        let _ = r;
    }
    Ok(())
}

// ── Header parsing ────────────────────────────────────────────────────────────

fn parse_header_bytes(
    bytes: &[u8],
) -> (
    Option<String>,           // message_id
    Option<String>,           // thread_id (from In-Reply-To / References)
    Option<String>,           // from_addr
    Option<Vec<String>>,      // to_addrs
    Option<Vec<String>>,      // cc_addrs
    Option<String>,           // subject
    Option<i64>,              // sent_at
) {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => return Default::default(),
    };

    let message_id = msg.message_id().map(str::to_owned);

    // Build thread_id from In-Reply-To (TextList or Text header value)
    let thread_id = hv_first_text(msg.in_reply_to())
        .or_else(|| hv_first_text(msg.references()));

    let from_addr = msg.from().map(|a| {
        let addrs = address_to_strings(a);
        addrs.into_iter().next().unwrap_or_default()
    });
    let to_addrs = msg.to().map(address_to_strings);
    let cc_addrs = msg.cc().map(address_to_strings);
    let subject = msg.subject().map(str::to_owned);
    let sent_at = msg.date().map(|dt| dt.to_timestamp());

    (message_id, thread_id, from_addr, to_addrs, cc_addrs, subject, sent_at)
}

fn hv_first_text(hv: &mail_parser::HeaderValue<'_>) -> Option<String> {
    match hv {
        mail_parser::HeaderValue::Text(t) => {
            t.split_whitespace().next().map(str::to_owned)
        }
        mail_parser::HeaderValue::TextList(list) => {
            list.first().and_then(|t| t.split_whitespace().next().map(str::to_owned))
        }
        _ => None,
    }
}

fn address_to_strings(addr: &mail_parser::Address<'_>) -> Vec<String> {
    match addr {
        mail_parser::Address::List(addrs) => addrs.iter().map(format_addr).collect(),
        mail_parser::Address::Group(groups) => {
            groups.iter().flat_map(|g| g.addresses.iter().map(format_addr)).collect()
        }
    }
}

fn format_addr(a: &mail_parser::Addr<'_>) -> String {
    match (&a.name, &a.address) {
        (Some(name), Some(addr)) => format!("{name} <{addr}>"),
        (None, Some(addr)) => addr.to_string(),
        (Some(name), None) => name.to_string(),
        _ => String::new(),
    }
}

fn parse_body_bytes(bytes: &[u8]) -> Result<(String, bool)> {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => return Ok((String::from_utf8_lossy(bytes).into_owned(), false)),
    };

    let has_attachments = msg.attachments().count() > 0;

    // Prefer plain text; fall back to stripping HTML
    let body_text = if let Some(text) = msg.body_text(0) {
        text.as_ref().to_owned()
    } else if let Some(html) = msg.body_html(0) {
        strip_html(html.as_ref())
    } else {
        String::new()
    };

    Ok((body_text, has_attachments))
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    // Collapse runs of whitespace
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn uids_to_set(uids: &[u32]) -> String {
    if uids.is_empty() {
        return String::new();
    }
    // Build compact RFC 3501 sequence set with ranges
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    let mut parts = Vec::new();
    let mut start = sorted[0];
    let mut end = sorted[0];
    for &uid in &sorted[1..] {
        if uid == end + 1 {
            end = uid;
        } else {
            if start == end {
                parts.push(start.to_string());
            } else {
                parts.push(format!("{start}:{end}"));
            }
            start = uid;
            end = uid;
        }
    }
    if start == end {
        parts.push(start.to_string());
    } else {
        parts.push(format!("{start}:{end}"));
    }
    parts.join(",")
}

/// Send an email via SMTP.
pub async fn send_email(
    smtp_host: &str,
    smtp_port: u16,
    username: &str,
    password: &str,
    from: &str,
    to: &[String],
    subject: &str,
    body: &str,
    in_reply_to: Option<&str>,
) -> Result<()> {
    use lettre::{
        message::{header::ContentType, SinglePart},
        AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
        transport::smtp::authentication::Credentials,
    };

    let mut builder = Message::builder()
        .from(from.parse().map_err(|e| anyhow!("invalid from address: {e}"))?)
        .subject(subject);

    for addr in to {
        builder = builder.to(addr.parse().map_err(|e| anyhow!("invalid to address: {e}"))?);
    }
    if let Some(reply_to) = in_reply_to {
        builder = builder.in_reply_to(reply_to.to_owned());
    }

    let email = builder
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(body.to_owned()),
        )
        .map_err(|e| anyhow!("failed to build email: {e}"))?;

    let creds = Credentials::new(username.to_owned(), password.to_owned());
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
        .map_err(|e| anyhow!("SMTP relay error: {e}"))?
        .port(smtp_port)
        .credentials(creds)
        .build();

    mailer.send(email).await.map_err(|e| anyhow!("SMTP send failed: {e}"))?;
    Ok(())
}

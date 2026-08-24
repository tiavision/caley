use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_imap::Client;
use futures_util::StreamExt;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, rustls};

use crate::email_db::MessageHeader;

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
pub async fn list_mailboxes(
    session: &mut ImapSession,
) -> Result<Vec<(String, Option<String>, bool)>> {
    let mut stream = session.list(None, Some("*")).await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let name_info = item?;
        let name = name_info.name().to_owned();
        let delim = name_info.delimiter().map(|d| d.to_owned());
        let subscribed = !name_info
            .attributes()
            .iter()
            .any(|a| matches!(a, async_imap::types::NameAttribute::NoSelect));
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

/// Returns true when the selected mailbox advertises save-date storage.
pub async fn selected_mailbox_supports_savedate(session: &mut ImapSession) -> Result<bool> {
    if !session.capabilities().await?.has_str("SAVEDATE") {
        return Ok(false);
    }
    let all_uids = session.uid_search("ALL").await?;
    if all_uids.is_empty() {
        return Ok(true);
    }
    Ok(!session.uid_search("SAVEDATESUPPORTED").await?.is_empty())
}

/// Finds UIDs whose RFC 8514 save date is before the given IMAP date.
pub async fn search_saved_before(session: &mut ImapSession, imap_date: &str) -> Result<Vec<u32>> {
    let uid_set = session
        .uid_search(format!("SAVEDBEFORE {imap_date}"))
        .await?;
    let mut v: Vec<u32> = uid_set.into_iter().collect();
    v.sort_unstable();
    Ok(v)
}

fn flag_name(flag: async_imap::types::Flag<'_>) -> String {
    match flag {
        async_imap::types::Flag::Seen => "\\Seen".to_owned(),
        async_imap::types::Flag::Answered => "\\Answered".to_owned(),
        async_imap::types::Flag::Flagged => "\\Flagged".to_owned(),
        async_imap::types::Flag::Deleted => "\\Deleted".to_owned(),
        async_imap::types::Flag::Draft => "\\Draft".to_owned(),
        async_imap::types::Flag::Recent => "\\Recent".to_owned(),
        async_imap::types::Flag::Custom(c) => c.to_string(),
        _ => String::new(),
    }
}

fn fetch_flags(fetch: &async_imap::types::Fetch) -> Vec<String> {
    fetch
        .flags()
        .map(flag_name)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Fetches only IMAP flags for a list of UIDs in the currently-selected mailbox.
pub async fn fetch_message_flags(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<(u32, Vec<String>)>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let uid_set = uids_to_set(uids);
    let mut stream = session.uid_fetch(&uid_set, "(FLAGS)").await?;
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item?;
        if let Some(uid) = fetch.uid {
            out.push((uid, fetch_flags(&fetch)));
        }
    }
    Ok(out)
}

/// Fetches headers for a list of UIDs. Returns parsed MessageHeaders.
pub async fn fetch_headers(session: &mut ImapSession, uids: &[u32]) -> Result<Vec<MessageHeader>> {
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let uid_set = uids_to_set(uids);
    let fetch_items = "(FLAGS RFC822.SIZE BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID REFERENCES IN-REPLY-TO)])";

    let mut stream = session.uid_fetch(&uid_set, fetch_items).await?;
    let mut headers = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item?;
        let uid = match fetch.uid {
            Some(u) => u,
            None => continue,
        };

        let flags = fetch_flags(&fetch);
        let flags_json = serde_json::to_string(&flags).unwrap_or_default();

        let (
            message_id,
            thread_id,
            from_addr,
            to_addrs,
            cc_addrs,
            subject,
            sent_at,
            from_emails,
            to_emails,
            cc_emails,
        ) = if let Some(header_bytes) = fetch.header() {
            parse_header_bytes(header_bytes)
        } else {
            Default::default()
        };

        let size = fetch.size.map(|s| s as i64);

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
            size,
            from_emails,
            to_emails,
            cc_emails,
        });
    }
    Ok(headers)
}

/// Parsed attachment extracted from a MIME message.
pub struct AttachmentPart {
    pub filename: Option<String>,
    pub content_type: String,
    pub content_id: Option<String>,
    pub size: i64,
    pub part_index: usize,
}

pub struct MessageRenderBody {
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub search_text: String,
    pub autocrypt_headers: Vec<String>,
    pub is_signed: bool,
}

/// Fetches full raw bodies for a list of UIDs in one IMAP command.
/// Detect PGP-encrypted content in raw message bytes.
pub fn detect_pgp_encrypted(raw: &[u8]) -> bool {
    let mime = String::from_utf8_lossy(raw).to_ascii_lowercase();
    (mime.contains("multipart/encrypted") && mime.contains("application/pgp-encrypted"))
        || raw.windows(27).any(|w| w == b"-----BEGIN PGP MESSAGE-----")
}

pub fn detect_pgp_signed(raw: &[u8]) -> bool {
    let mime = String::from_utf8_lossy(raw).to_ascii_lowercase();
    (mime.contains("multipart/signed") && mime.contains("application/pgp-signature"))
        || raw
            .windows(b"-----BEGIN PGP SIGNED MESSAGE-----".len())
            .any(|w| w == b"-----BEGIN PGP SIGNED MESSAGE-----")
}

/// Returns raw bytes plus the small render body and attachment index for each UID.
pub async fn fetch_bodies(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<(u32, Vec<u8>, MessageRenderBody, bool, Vec<AttachmentPart>)>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let uid_set = uids_to_set(uids);
    let mut stream = session.uid_fetch(&uid_set, "(BODY.PEEK[])").await?;
    let mut results = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item?;
        let uid = match fetch.uid {
            Some(u) => u,
            None => continue,
        };
        if let Some(body_bytes) = fetch.body() {
            let raw = body_bytes.to_vec();
            let (render_body, has_attach, attachments) = parse_body_bytes(body_bytes)?;
            results.push((uid, raw, render_body, has_attach, attachments));
        }
    }
    Ok(results)
}

/// Moves a message to `dest_mailbox` using MOVE (RFC 6851) when the server
/// supports it, otherwise falls back to COPY + \Deleted + EXPUNGE.
pub async fn move_message_on_server(
    session: &mut ImapSession,
    uid: u32,
    dest_mailbox: &str,
) -> Result<()> {
    let uid_str = uid.to_string();
    let has_move = session.capabilities().await?.has_str("MOVE");
    if has_move {
        session.uid_mv(&uid_str, dest_mailbox).await?;
    } else {
        session.uid_copy(&uid_str, dest_mailbox).await?;
        {
            let mut stream = session.uid_store(&uid_str, "+FLAGS (\\Deleted)").await?;
            while let Some(r) = stream.next().await {
                let _ = r;
            }
        }
        let uidplus = session.capabilities().await?.has_str("UIDPLUS");
        if uidplus {
            let mut stream = Box::pin(session.uid_expunge(&uid_str).await?);
            while let Some(r) = stream.next().await {
                let _ = r;
            }
        } else {
            let mut stream = Box::pin(session.expunge().await?);
            while let Some(r) = stream.next().await {
                let _ = r;
            }
        }
    }
    Ok(())
}

/// Stores the \Deleted flag and expunges on the server.
pub async fn delete_on_server(session: &mut ImapSession, uid: u32) -> Result<()> {
    let uid_str = uid.to_string();
    {
        // Scope the stream so the borrow on session ends before expunge
        let mut stream = session.uid_store(&uid_str, "+FLAGS (\\Deleted)").await?;
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    }
    // Expunge only the targeted UID when the server supports UIDPLUS, so other
    // messages flagged \Deleted aren't purged as collateral; otherwise fall back
    // to a mailbox-wide EXPUNGE. Both return a stream of untagged responses that
    // must be polled to completion (and aren't Unpin, so box-pin them) or the
    // command isn't driven and its responses corrupt the next one.
    let uidplus = session.capabilities().await?.has_str("UIDPLUS");
    if uidplus {
        let mut stream = Box::pin(session.uid_expunge(&uid_str).await?);
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    } else {
        let mut stream = Box::pin(session.expunge().await?);
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    }
    Ok(())
}

/// Stores the \Deleted flag for a UID set and expunges it on the server.
pub async fn delete_on_server_batch(session: &mut ImapSession, uids: &[u32]) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let uid_set = uids_to_set(uids);
    {
        let mut stream = session.uid_store(&uid_set, "+FLAGS (\\Deleted)").await?;
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    }
    let uidplus = session.capabilities().await?.has_str("UIDPLUS");
    if uidplus {
        let mut stream = Box::pin(session.uid_expunge(&uid_set).await?);
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    } else {
        let mut stream = Box::pin(session.expunge().await?);
        while let Some(r) = stream.next().await {
            let _ = r;
        }
    }
    Ok(())
}

/// Sets or clears flags on the server.
pub async fn set_flags(
    session: &mut ImapSession,
    uid: u32,
    flags: &[&str],
    set: bool,
) -> Result<()> {
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

/// Appends a transmitted RFC 5322 message to the account's existing sent mailbox.
/// Returns the mailbox name when a suitable destination was found.
pub async fn append_to_sent(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    raw_message: &[u8],
) -> Result<Option<String>> {
    let mut session = connect(host, port, username, password).await?;
    let sent_mailbox = find_sent_mailbox(&mut session).await?;
    if let Some(mailbox) = &sent_mailbox {
        append_message(&mut session, mailbox, raw_message).await?;
    }
    let _ = session.logout().await;
    Ok(sent_mailbox)
}

async fn append_message(
    session: &mut ImapSession,
    mailbox: &str,
    raw_message: &[u8],
) -> Result<()> {
    use async_imap::imap_proto::{Response, Status};
    use tokio::io::AsyncWriteExt;

    let mailbox_arg = quote_imap_string(mailbox)?;
    let request_id = session
        .run_command(format!(
            "APPEND {mailbox_arg} (\\Seen) {{{}}}",
            raw_message.len()
        ))
        .await?;

    loop {
        let response = session.read_response().await.ok_or_else(|| {
            anyhow!("IMAP connection closed while appending to mailbox {mailbox:?}")
        })??;
        match response.parsed() {
            Response::Continue { .. } => break,
            Response::Done {
                tag,
                status,
                code,
                information,
            } if tag == &request_id => {
                return Err(anyhow!(
                    "IMAP APPEND to mailbox {mailbox:?} rejected before upload: {status:?} {code:?} {}",
                    information.as_deref().unwrap_or("")
                ));
            }
            _ => {}
        }
    }

    session.as_mut().write_all(raw_message).await?;
    session.as_mut().write_all(b"\r\n").await?;
    session.as_mut().flush().await?;

    loop {
        let response = session.read_response().await.ok_or_else(|| {
            anyhow!("IMAP connection closed while completing APPEND to mailbox {mailbox:?}")
        })??;
        if let Response::Done {
            tag,
            status,
            code,
            information,
        } = response.parsed()
        {
            if tag != &request_id {
                continue;
            }
            return match status {
                Status::Ok => Ok(()),
                _ => Err(anyhow!(
                    "IMAP APPEND to mailbox {mailbox:?} rejected after upload: {status:?} {code:?} {}",
                    information.as_deref().unwrap_or("")
                )),
            };
        }
    }
}

fn quote_imap_string(value: &str) -> Result<String> {
    if value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return Err(anyhow!("invalid IMAP mailbox name"));
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

async fn find_sent_mailbox(session: &mut ImapSession) -> Result<Option<String>> {
    let mut stream = session.list(None, Some("*")).await?;
    let mut fallback = None;
    while let Some(item) = stream.next().await {
        let info = item?;
        let name = info.name().to_owned();
        if info
            .attributes()
            .iter()
            .any(|attribute| matches!(attribute, async_imap::types::NameAttribute::Sent))
        {
            return Ok(Some(name));
        }

        let leaf = name
            .rsplit(|c| c == '/' || c == '.')
            .next()
            .unwrap_or(&name)
            .to_ascii_lowercase();
        if fallback.is_none()
            && matches!(
                leaf.as_str(),
                "sent" | "sent mail" | "sent messages" | "gesendet"
            )
        {
            fallback = Some(name);
        }
    }
    Ok(fallback)
}

// ── Header parsing ────────────────────────────────────────────────────────────

fn parse_header_bytes(
    bytes: &[u8],
) -> (
    Option<String>,      // message_id
    Option<String>,      // thread_id (from In-Reply-To / References)
    Option<String>,      // from_addr
    Option<Vec<String>>, // to_addrs
    Option<Vec<String>>, // cc_addrs
    Option<String>,      // subject
    Option<i64>,         // sent_at
    Vec<String>,         // from_emails (lowercased, name stripped)
    Vec<String>,         // to_emails
    Vec<String>,         // cc_emails
) {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => return Default::default(),
    };

    let message_id = msg.message_id().map(str::to_owned);

    // Store the direct reply parent when available, otherwise the nearest
    // referenced ancestor for messages without In-Reply-To.
    let thread_id = hv_first_text(msg.in_reply_to()).or_else(|| hv_last_text(msg.references()));

    let from_addr = msg.from().map(|a| {
        let addrs = address_to_strings(a);
        addrs.into_iter().next().unwrap_or_default()
    });
    let to_addrs = msg.to().map(address_to_strings);
    let cc_addrs = msg.cc().map(address_to_strings);
    let from_emails = msg.from().map(address_to_emails).unwrap_or_default();
    let to_emails = msg.to().map(address_to_emails).unwrap_or_default();
    let cc_emails = msg.cc().map(address_to_emails).unwrap_or_default();
    let subject = msg.subject().map(str::to_owned);
    let sent_at = msg.date().map(|dt| dt.to_timestamp());

    (
        message_id,
        thread_id,
        from_addr,
        to_addrs,
        cc_addrs,
        subject,
        sent_at,
        from_emails,
        to_emails,
        cc_emails,
    )
}

fn hv_first_text(hv: &mail_parser::HeaderValue<'_>) -> Option<String> {
    match hv {
        mail_parser::HeaderValue::Text(t) => t.split_whitespace().next().map(str::to_owned),
        mail_parser::HeaderValue::TextList(list) => list
            .first()
            .and_then(|t| t.split_whitespace().next().map(str::to_owned)),
        _ => None,
    }
}

fn hv_last_text(hv: &mail_parser::HeaderValue<'_>) -> Option<String> {
    match hv {
        mail_parser::HeaderValue::Text(t) => t.split_whitespace().last().map(str::to_owned),
        mail_parser::HeaderValue::TextList(list) => list
            .last()
            .and_then(|t| t.split_whitespace().last().map(str::to_owned)),
        _ => None,
    }
}

fn address_to_strings(addr: &mail_parser::Address<'_>) -> Vec<String> {
    match addr {
        mail_parser::Address::List(addrs) => addrs.iter().map(format_addr).collect(),
        mail_parser::Address::Group(groups) => groups
            .iter()
            .flat_map(|g| g.addresses.iter().map(format_addr))
            .collect(),
    }
}

/// Extracts the bare email addresses (lowercased, display name dropped) from an
/// address header, for the normalized `message_addresses` lookup table.
fn address_to_emails(addr: &mail_parser::Address<'_>) -> Vec<String> {
    let emails = |a: &mail_parser::Addr<'_>| {
        a.address
            .as_ref()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
    };
    match addr {
        mail_parser::Address::List(addrs) => addrs.iter().filter_map(emails).collect(),
        mail_parser::Address::Group(groups) => groups
            .iter()
            .flat_map(|g| g.addresses.iter().filter_map(emails))
            .collect(),
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

fn parse_body_bytes(bytes: &[u8]) -> Result<(MessageRenderBody, bool, Vec<AttachmentPart>)> {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            return Ok((
                MessageRenderBody {
                    body_text: Some(text.clone()),
                    body_html: None,
                    search_text: text,
                    autocrypt_headers: extract_header_values(bytes, "autocrypt"),
                    is_signed: detect_pgp_signed(bytes),
                },
                false,
                Vec::new(),
            ));
        }
    };

    let body_text = msg.body_text(0).map(|text| text.as_ref().to_owned());
    let body_html = msg.body_html(0).map(|html| html.as_ref().to_owned());
    let search_text = body_text
        .clone()
        .unwrap_or_else(|| body_html.as_deref().map(strip_tags).unwrap_or_default());

    let attachments = extract_attachments(&msg);
    let has_attachments = !attachments.is_empty();

    Ok((
        MessageRenderBody {
            body_text,
            body_html,
            search_text,
            autocrypt_headers: extract_header_values(bytes, "autocrypt"),
            is_signed: detect_pgp_signed(bytes),
        },
        has_attachments,
        attachments,
    ))
}

fn extract_header_values(bytes: &[u8], name: &str) -> Vec<String> {
    let raw = String::from_utf8_lossy(bytes);
    let header_end = raw
        .find("\r\n\r\n")
        .or_else(|| raw.find("\n\n"))
        .unwrap_or(raw.len());
    let mut unfolded: Vec<String> = Vec::new();
    for line in raw[..header_end].lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(previous) = unfolded.last_mut() {
                previous.push(' ');
                previous.push_str(line.trim());
            }
        } else {
            unfolded.push(line.trim_end_matches('\r').to_owned());
        }
    }
    let prefix = format!("{}:", name.to_ascii_lowercase());
    unfolded
        .into_iter()
        .filter_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .starts_with(&prefix)
                .then(|| line[prefix.len()..].trim().to_owned())
        })
        .collect()
}

fn extract_attachments(msg: &mail_parser::Message<'_>) -> Vec<AttachmentPart> {
    use mail_parser::MimeHeaders;
    use std::collections::HashSet;

    let mut attachments = Vec::new();
    let mut used_filenames = HashSet::new();
    for (idx, part) in msg.parts.iter().enumerate() {
        let is_attachment = part
            .content_disposition()
            .map_or(false, |cd| cd.ctype().to_ascii_lowercase() == "attachment");
        let has_filename = part.attachment_name().is_some();
        let content_type = part
            .content_type()
            .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let is_calendar = is_calendar_content_type(&content_type);
        // Treat as inline image if: explicitly Content-Disposition: inline + image/*,
        // OR has a Content-ID + image/* (multipart/related parts built by lettre omit
        // Content-Disposition but still need to be findable by CID for proxy-image).
        let is_inline_image = part
            .content_type()
            .map_or(false, |ct| ct.ctype().starts_with("image/"))
            && (part
                .content_disposition()
                .map_or(false, |cd| cd.ctype().to_ascii_lowercase() == "inline")
                || part.content_id().is_some());

        if !is_attachment && !has_filename && !is_inline_image && !is_calendar {
            continue;
        }

        let filename = part.attachment_name().map(|s| s.to_owned()).or_else(|| {
            if is_calendar {
                return Some(unique_fallback_filename(
                    &mut used_filenames,
                    "noname",
                    "ics",
                ));
            }
            part.content_id().map(|cid| {
                let safe = cid
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .replace(['/', '\\', ':', ' '], "_");
                format!("{safe}.bin")
            })
        });

        if let Some(filename) = filename.as_deref() {
            used_filenames.insert(filename.to_owned());
        }

        let content_id = part
            .content_id()
            .map(|cid| cid.trim_start_matches('<').trim_end_matches('>').to_owned());

        let size = match &part.body {
            mail_parser::PartType::Binary(b) => b.len(),
            mail_parser::PartType::InlineBinary(b) => b.len(),
            mail_parser::PartType::Text(t) => t.len(),
            mail_parser::PartType::Html(h) => h.len(),
            _ => 0,
        } as i64;

        attachments.push(AttachmentPart {
            filename,
            content_type,
            content_id,
            size,
            part_index: idx,
        });
    }
    attachments
}

fn is_calendar_content_type(content_type: &str) -> bool {
    matches!(
        content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "text/calendar" | "application/ics" | "application/icalendar"
    )
}

fn unique_fallback_filename(
    used_filenames: &mut std::collections::HashSet<String>,
    stem: &str,
    extension: &str,
) -> String {
    let first = format!("{stem}.{extension}");
    if used_filenames.insert(first.clone()) {
        return first;
    }

    for n in 2.. {
        let candidate = format!("{stem}-{n}.{extension}");
        if used_filenames.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded filename suffix search must return")
}

fn strip_tags(html: &str) -> String {
    const BLOCK: &[&str] = &[
        "p",
        "div",
        "br",
        "li",
        "tr",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "blockquote",
        "hr",
    ];

    let mut result = String::with_capacity(html.len());
    let mut tag_buf = String::new();
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag_buf
                    .split_ascii_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('/')
                    .to_ascii_lowercase();
                if BLOCK.contains(&name.as_str()) {
                    result.push('\n');
                }
            }
            c if in_tag => tag_buf.push(c),
            c => result.push(c),
        }
    }

    // Collapse 3+ consecutive newlines to 2
    let mut collapsed = String::with_capacity(result.len());
    let mut newline_count = 0u8;
    for c in result.chars() {
        if c == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                collapsed.push('\n');
            }
        } else {
            newline_count = 0;
            collapsed.push(c);
        }
    }

    collapsed
        .trim()
        .to_owned()
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
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

/// Relay an already-built RFC 5322 message without reconstructing its MIME headers.
pub async fn send_raw_email(
    smtp_host: &str,
    smtp_port: u16,
    username: &str,
    password: &str,
    from: &str,
    recipients: &[String],
    raw_mime: &[u8],
) -> Result<()> {
    use lettre::{
        AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
        address::{Address, Envelope},
        transport::smtp::authentication::Credentials,
    };

    let sender: Address = from
        .parse()
        .map_err(|e| anyhow!("invalid from address: {e}"))?;
    let recipient_addresses = recipients
        .iter()
        .map(|recipient| {
            recipient
                .parse::<Address>()
                .map_err(|e| anyhow!("invalid recipient address: {e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let envelope = Envelope::new(Some(sender), recipient_addresses)
        .map_err(|e| anyhow!("invalid SMTP envelope: {e}"))?;

    let creds = Credentials::new(username.to_owned(), password.to_owned());
    let builder = if smtp_port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
    }
    .map_err(|e| anyhow!("SMTP relay error: {e}"))?;
    let mailer = builder.port(smtp_port).credentials(creds).build();

    mailer
        .send_raw(&envelope, raw_mime)
        .await
        .map_err(|e| anyhow!("SMTP send failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_cp1252_alias_encoded_word_subject() {
        // Real-world senders commonly label windows-1252 as "Cp1252" (or
        // "cp1252") rather than the canonical "windows-1252" charset name.
        let raw = b"Subject: =?Cp1252?Q?Zahnarztpraxis_Dr._Julia_Pennig_Terminbest=E4tigung?=\r\n\r\nbody";
        let (_, _, _, _, _, subject, ..) = parse_header_bytes(raw);
        assert_eq!(
            subject.as_deref(),
            Some("Zahnarztpraxis Dr. Julia Pennig Terminbestätigung")
        );
    }

    #[test]
    fn test_reply_parent_prefers_in_reply_to() {
        let raw = b"Message-ID: <child@example.test>\r\nIn-Reply-To: <parent@example.test>\r\nReferences: <root@example.test> <other@example.test>\r\n\r\nbody";
        let (_, parent, ..) = parse_header_bytes(raw);
        assert_eq!(parent.as_deref(), Some("parent@example.test"));
    }

    #[test]
    fn test_reply_parent_uses_nearest_reference() {
        let raw = b"Message-ID: <child@example.test>\r\nReferences: <root@example.test> <parent@example.test>\r\n\r\nbody";
        let (_, parent, ..) = parse_header_bytes(raw);
        assert_eq!(parent.as_deref(), Some("parent@example.test"));
    }

    #[test]
    fn extracts_unnamed_calendar_part_as_ics_attachment() {
        let raw = b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n--b\r\nContent-Type: text/calendar; method=REQUEST; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nBEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n--b--\r\n";

        let (_, has_attachments, attachments) = parse_body_bytes(raw).unwrap();

        assert!(has_attachments);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename.as_deref(), Some("noname.ics"));
        assert_eq!(attachments[0].content_type, "text/calendar");
    }

    #[test]
    fn keeps_named_calendar_attachment_filename() {
        let raw = b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n--b\r\nContent-Type: text/calendar; name=\"invite.ics\"\r\nContent-Disposition: attachment; filename=\"invite.ics\"\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b--\r\n";

        let (_, has_attachments, attachments) = parse_body_bytes(raw).unwrap();

        assert!(has_attachments);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename.as_deref(), Some("invite.ics"));
    }

    #[test]
    fn gives_multiple_unnamed_calendar_parts_unique_filenames() {
        let raw = b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n--b\r\nContent-Type: text/calendar\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b\r\nContent-Type: application/ics\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b--\r\n";

        let (_, has_attachments, attachments) = parse_body_bytes(raw).unwrap();

        assert!(has_attachments);
        let filenames: Vec<_> = attachments
            .iter()
            .filter_map(|att| att.filename.as_deref())
            .collect();
        assert_eq!(filenames, vec!["noname.ics", "noname-2.ics"]);
    }

    #[test]
    fn does_not_expose_plain_text_body_as_attachment() {
        let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n";

        let (_, has_attachments, attachments) = parse_body_bytes(raw).unwrap();

        assert!(!has_attachments);
        assert!(attachments.is_empty());
    }

    #[test]
    fn extracts_render_body_and_unfolded_autocrypt_header() {
        let raw = b"From: Alice <alice@example.test>\r\nAutocrypt: addr=alice@example.test;\r\n keydata=YWJj\r\nContent-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain\r\n\r\nHello\r\n--alt\r\nContent-Type: text/html\r\n\r\n<p>Hello</p>\r\n--alt--\r\n";

        let (body, _, _) = parse_body_bytes(raw).unwrap();

        assert_eq!(body.body_text.as_deref(), Some("Hello"));
        assert_eq!(body.body_html.as_deref(), Some("<p>Hello</p>"));
        assert_eq!(
            body.autocrypt_headers,
            vec!["addr=alice@example.test; keydata=YWJj"]
        );
        assert!(!body.is_signed);
    }

    #[test]
    fn detects_pgp_mime_encrypted_mail() {
        let raw = b"Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"e\"\r\n\r\n--e\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n--e--\r\n";

        assert!(detect_pgp_encrypted(raw));
    }

    #[test]
    fn detects_inline_pgp_encrypted_mail() {
        let raw = b"Content-Type: text/plain\r\n\r\n-----BEGIN PGP MESSAGE-----\r\nx\r\n-----END PGP MESSAGE-----";

        assert!(detect_pgp_encrypted(raw));
    }

    #[test]
    fn does_not_treat_signed_mail_as_encrypted() {
        let raw = b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=\"s\"\r\n\r\n--s\r\nContent-Type: text/plain\r\n\r\nHello\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\nx\r\n-----END PGP SIGNATURE-----\r\n--s--\r\n";

        assert!(!detect_pgp_encrypted(raw));
        assert!(detect_pgp_signed(raw));
    }

    #[test]
    fn does_not_treat_ordinary_mail_as_encrypted() {
        let raw = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n";

        assert!(!detect_pgp_encrypted(raw));
    }
}

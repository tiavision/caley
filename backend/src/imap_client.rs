use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_imap::Client;
use futures_util::StreamExt;
use rustls_pki_types::ServerName;
use tokio::net::TcpStream;
use tokio_rustls::{rustls, TlsConnector};

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

        let (
            message_id, thread_id, from_addr, to_addrs, cc_addrs, subject, sent_at,
            from_emails, to_emails, cc_emails,
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

/// Fetches full raw bodies for a list of UIDs in one IMAP command.
/// Detect PGP-encrypted or PGP-signed content in raw message bytes.
/// Returns (is_encrypted, is_signed).
pub fn detect_pgp(raw: &[u8]) -> (bool, bool) {
    let hdr_end = raw.len().min(4096);
    let hdr = String::from_utf8_lossy(&raw[..hdr_end]).to_ascii_lowercase();
    let is_encrypted = (hdr.contains("multipart/encrypted") && hdr.contains("application/pgp-encrypted"))
        || raw.windows(27).any(|w| w == b"-----BEGIN PGP MESSAGE-----");
    let is_signed = (hdr.contains("multipart/signed") && hdr.contains("application/pgp-signature"))
        || raw.windows(34).any(|w| w == b"-----BEGIN PGP SIGNED MESSAGE-----");
    (is_encrypted, is_signed)
}

/// Returns a vec of (uid, raw_bytes, body_text, raw_html, has_attachments, attachments).
pub async fn fetch_bodies(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<(u32, Vec<u8>, String, Option<String>, bool, Vec<AttachmentPart>)>> {
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
            let (text, html, has_attach, attachments) = parse_body_bytes(body_bytes)?;
            results.push((uid, raw, text, html, has_attach, attachments));
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
            while let Some(r) = stream.next().await { let _ = r; }
        }
        let uidplus = session.capabilities().await?.has_str("UIDPLUS");
        if uidplus {
            let mut stream = Box::pin(session.uid_expunge(&uid_str).await?);
            while let Some(r) = stream.next().await { let _ = r; }
        } else {
            let mut stream = Box::pin(session.expunge().await?);
            while let Some(r) = stream.next().await { let _ = r; }
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
        while let Some(r) = stream.next().await { let _ = r; }
    }
    // Expunge only the targeted UID when the server supports UIDPLUS, so other
    // messages flagged \Deleted aren't purged as collateral; otherwise fall back
    // to a mailbox-wide EXPUNGE. Both return a stream of untagged responses that
    // must be polled to completion (and aren't Unpin, so box-pin them) or the
    // command isn't driven and its responses corrupt the next one.
    let uidplus = session.capabilities().await?.has_str("UIDPLUS");
    if uidplus {
        let mut stream = Box::pin(session.uid_expunge(&uid_str).await?);
        while let Some(r) = stream.next().await { let _ = r; }
    } else {
        let mut stream = Box::pin(session.expunge().await?);
        while let Some(r) = stream.next().await { let _ = r; }
    }
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

async fn append_message(session: &mut ImapSession, mailbox: &str, raw_message: &[u8]) -> Result<()> {
    use async_imap::imap_proto::{Response, Status};
    use tokio::io::AsyncWriteExt;

    let mailbox_arg = quote_imap_string(mailbox)?;
    let request_id = session
        .run_command(format!("APPEND {mailbox_arg} (\\Seen) {{{}}}", raw_message.len()))
        .await?;

    loop {
        let response = session
            .read_response()
            .await
            .ok_or_else(|| anyhow!("IMAP connection closed while appending to mailbox {mailbox:?}"))??;
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
        let response = session
            .read_response()
            .await
            .ok_or_else(|| anyhow!("IMAP connection closed while completing APPEND to mailbox {mailbox:?}"))??;
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
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
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
            && matches!(leaf.as_str(), "sent" | "sent mail" | "sent messages" | "gesendet")
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
    Option<String>,           // message_id
    Option<String>,           // thread_id (from In-Reply-To / References)
    Option<String>,           // from_addr
    Option<Vec<String>>,      // to_addrs
    Option<Vec<String>>,      // cc_addrs
    Option<String>,           // subject
    Option<i64>,              // sent_at
    Vec<String>,              // from_emails (lowercased, name stripped)
    Vec<String>,              // to_emails
    Vec<String>,              // cc_emails
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
    let thread_id = hv_first_text(msg.in_reply_to())
        .or_else(|| hv_last_text(msg.references()));

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
        message_id, thread_id, from_addr, to_addrs, cc_addrs, subject, sent_at,
        from_emails, to_emails, cc_emails,
    )
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

fn hv_last_text(hv: &mail_parser::HeaderValue<'_>) -> Option<String> {
    match hv {
        mail_parser::HeaderValue::Text(t) => {
            t.split_whitespace().last().map(str::to_owned)
        }
        mail_parser::HeaderValue::TextList(list) => {
            list.last().and_then(|t| t.split_whitespace().last().map(str::to_owned))
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

fn parse_body_bytes(bytes: &[u8]) -> Result<(String, Option<String>, bool, Vec<AttachmentPart>)> {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => return Ok((String::from_utf8_lossy(bytes).into_owned(), None, false, Vec::new())),
    };

    let has_attachments = msg.attachments().count() > 0;

    let raw_html = msg.body_html(0).map(|h| h.as_ref().to_owned());
    let body_html = raw_html.as_deref().map(sanitize_email_html);

    let body_text = if let Some(text) = msg.body_text(0) {
        text.as_ref().to_owned()
    } else if let Some(ref html) = raw_html {
        strip_tags(html)
    } else {
        String::new()
    };

    let attachments = extract_attachments(&msg);

    Ok((body_text, body_html, has_attachments, attachments))
}

fn extract_attachments(msg: &mail_parser::Message<'_>) -> Vec<AttachmentPart> {
    use mail_parser::MimeHeaders;
    let mut attachments = Vec::new();
    for (idx, part) in msg.parts.iter().enumerate() {
        let is_attachment = part.content_disposition().map_or(false, |cd| {
            cd.ctype().to_ascii_lowercase() == "attachment"
        });
        let has_filename = part.attachment_name().is_some();
        let is_inline_image = part.content_disposition().map_or(false, |cd| {
            cd.ctype().to_ascii_lowercase() == "inline"
        }) && part.content_type().map_or(false, |ct| {
            ct.ctype().starts_with("image/")
        });

        if !is_attachment && !has_filename && !is_inline_image {
            continue;
        }

        let filename = part
            .attachment_name()
            .map(|s| s.to_owned())
            .or_else(|| {
                part.content_id().map(|cid| {
                    let safe = cid.trim_start_matches('<').trim_end_matches('>').replace(['/', '\\', ':', ' '], "_");
                    format!("{safe}.bin")
                })
            });

        let content_type = part
            .content_type()
            .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let content_id = part.content_id().map(|cid| {
            cid.trim_start_matches('<').trim_end_matches('>').to_owned()
        });

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

/// Rewrite `cid:xxx` URLs in HTML to point at our proxy endpoint.
pub fn rewrite_cid_urls(
    html: &str,
    account_id: &str,
    mailbox: &str,
    uid: u32,
) -> String {
    use lol_html::{element, HtmlRewriter, Settings};

    let mut output = Vec::with_capacity(html.len() + 256);
    {
        let mut rewriter = HtmlRewriter::new(
            Settings {
                element_content_handlers: vec![
                    element!("img", |el| {
                        if let Some(src) = el.get_attribute("src") {
                            if let Some(cid) = src.strip_prefix("cid:") {
                                let encoded_cid: String =
                                    url::form_urlencoded::byte_serialize(cid.as_bytes()).collect();
                                let new_src = format!(
                                    "/api/email/proxy-image?account_id={}&mailbox={}&uid={}&cid={}",
                                    url::form_urlencoded::byte_serialize(account_id.as_bytes()).collect::<String>(),
                                    url::form_urlencoded::byte_serialize(mailbox.as_bytes()).collect::<String>(),
                                    uid,
                                    encoded_cid
                                );
                                el.set_attribute("src", &new_src)?;
                            }
                        }
                        Ok(())
                    }),
                ],
                ..Settings::default()
            },
            |c: &[u8]| output.extend_from_slice(c),
        );
        let _ = rewriter.write(html.as_bytes());
        let _ = rewriter.end();
    }
    String::from_utf8(output).unwrap_or_else(|_| html.to_owned())
}

// ── HTML sanitization for email display ──────────────────────────────────────

pub fn sanitize_email_html(html: &str) -> String {
    use std::borrow::Cow;
    use std::collections::{HashMap, HashSet};

    let allowed_tags: HashSet<&str> = [
        "a", "abbr", "b", "blockquote", "br", "cite", "code", "col", "colgroup",
        "del", "div", "dl", "dt", "dd", "em", "font", "h1", "h2", "h3", "h4",
        "h5", "h6", "hr", "i", "img", "ins", "li", "ol", "p", "pre", "s",
        "small", "span", "strike", "strong", "sub", "sup", "style",
        "table", "tbody", "td", "tfoot", "th", "thead", "tr", "u", "ul",
    ]
    .iter()
    .copied()
    .collect();

    let clean_content: HashSet<&str> = ["script", "head", "noscript", "title"]
        .iter()
        .copied()
        .collect();

    let cell_attrs: HashSet<&str> = [
        "colspan", "rowspan", "align", "valign", "width", "height", "bgcolor",
    ]
    .iter()
    .copied()
    .collect();

    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href", "title"].iter().copied().collect());
    tag_attrs.insert(
        "img",
        ["src", "alt", "width", "height"].iter().copied().collect(),
    );
    tag_attrs.insert("td", cell_attrs.clone());
    tag_attrs.insert("th", cell_attrs);
    tag_attrs.insert(
        "table",
        [
            "width", "cellpadding", "cellspacing", "border", "bgcolor", "align",
        ]
        .iter()
        .copied()
        .collect(),
    );
    tag_attrs.insert(
        "font",
        ["color", "face", "size"].iter().copied().collect(),
    );
    tag_attrs.insert("col", ["span", "width"].iter().copied().collect());
    tag_attrs.insert("colgroup", ["span"].iter().copied().collect());

    // `cid`/`data` must be whitelisted here: ammonia strips URL attributes whose
    // scheme isn't allowed during DOM cleaning, *before* the attribute_filter below
    // runs. Without `cid` the inline-image src is dropped and rewrite_cid_urls has
    // nothing to fix; without `data` the data:image/* src is dropped too. The filter
    // below is what actually constrains where each scheme is permitted.
    let url_schemes: HashSet<&str> =
        ["http", "https", "mailto", "cid", "data"].iter().copied().collect();
    let generic_attrs: HashSet<&str> = ["style", "class", "align", "dir"]
        .iter()
        .copied()
        .collect();

    let clean = ammonia::Builder::new()
        .tags(allowed_tags)
        .clean_content_tags(clean_content)
        .tag_attributes(tag_attrs)
        .generic_attributes(generic_attrs)
        .url_schemes(url_schemes)
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(|element, attribute, value| -> Option<Cow<str>> {
            match (element, attribute) {
                ("img", "src") => {
                    if value.starts_with("http://") || value.starts_with("https://") {
                        let encoded: String =
                            url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
                        Some(Cow::Owned(format!("/api/email/proxy-image?url={encoded}")))
                    } else if let Some(rest) = value.strip_prefix("data:image/") {
                        // Inline data images are fine, except SVG: it can carry script,
                        // matching the proxy_image endpoint which also refuses SVG.
                        if rest.trim_start().to_ascii_lowercase().starts_with("svg") {
                            None
                        } else {
                            Some(Cow::Borrowed(value))
                        }
                    } else if value.starts_with("cid:") {
                        // Preserved as-is; rewrite_cid_urls rewrites these after sanitization.
                        Some(Cow::Borrowed(value))
                    } else {
                        None
                    }
                }
                (_, "style") => {
                    let filtered = strip_css_url_functions(value);
                    if filtered.trim().is_empty() {
                        None
                    } else {
                        Some(Cow::Owned(filtered))
                    }
                }
                _ => {
                    // `data:` is only legitimate on <img src> (handled above). Strip it
                    // from every other attribute (notably <a href>) so a data:text/html
                    // payload can't smuggle markup/script past sanitization. Trim + lower
                    // because Url::parse — and browsers — tolerate leading space and case.
                    if value.trim_start().to_ascii_lowercase().starts_with("data:") {
                        None
                    } else {
                        Some(Cow::Borrowed(value))
                    }
                }
            }
        })
        .clean(html)
        .to_string();

    let with_safe_styles = sanitize_style_elements(&clean);
    rewrite_links(&with_safe_styles)
}

/// Sanitizes CSS content inside `<style>` elements, preserving safe layout rules
/// while blocking all CSS-based attack vectors (url(), @import, expression, etc.).
fn sanitize_style_content(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut rule_start = 0;

    while pos < len {
        if bytes[pos] == b'/' && pos + 1 < len && bytes[pos + 1] == b'*' {
            if let Some(end_offset) = css[pos + 2..].find("*/") {
                pos = pos + 2 + end_offset + 2;
            } else {
                break;
            }
            continue;
        }

        let lower_from = css[pos..].to_ascii_lowercase();
        let lower_trimmed = lower_from.trim_start();

        if lower_trimmed.starts_with("@import")
            || lower_trimmed.starts_with("@font-face")
            || lower_trimmed.starts_with("@keyframes")
            || lower_trimmed.starts_with("@-webkit-keyframes")
            || lower_trimmed.starts_with("@-moz-keyframes")
            || lower_trimmed.starts_with("@-o-keyframes")
            || lower_trimmed.starts_with("@-ms-keyframes")
        {
            pos = skip_at_rule(css, pos);
            rule_start = pos;
            continue;
        }

        if lower_trimmed.starts_with("@media")
            || lower_trimmed.starts_with("@supports")
        {
            pos = skip_at_rule(css, pos);
            rule_start = pos;
            continue;
        }

        if bytes[pos] == b'{' {
            pos += 1;
            let close = find_matching_brace(css, pos);
            let selector = css[rule_start..pos - 1].trim();
            let declarations = &css[pos..close];
            let safe_decls = sanitize_declarations(declarations);
            if !safe_decls.is_empty() {
                result.push_str(selector);
                result.push_str(" { ");
                result.push_str(&safe_decls);
                result.push_str(" } ");
            }
            pos = close + 1;
            rule_start = pos;
            continue;
        }

        pos += utf8_char_width(bytes[pos]);
    }

    if rule_start < len {
        let trailing = css[rule_start..].trim();
        if !trailing.is_empty() && !trailing.starts_with('@') && !trailing.contains('{') {
            let safe = sanitize_declarations(trailing);
            if !safe.is_empty() {
                result.push_str(&safe);
            }
        }
    }

    result.trim_end().to_owned()
}

fn utf8_char_width(b: u8) -> usize {
    match b {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

fn skip_at_rule(css: &str, start: usize) -> usize {
    let mut pos = start;
    let bytes = css.as_bytes();
    let len = bytes.len();
    while pos < len && bytes[pos] != b'{' && bytes[pos] != b';' {
        pos += 1;
    }
    if pos >= len {
        return pos;
    }
    if bytes[pos] == b';' {
        return pos + 1;
    }
    pos += 1;
    let mut depth: usize = 1;
    while pos < len && depth > 0 {
        if bytes[pos] == b'{' {
            depth += 1;
        } else if bytes[pos] == b'}' {
            depth -= 1;
        }
        pos += 1;
    }
    pos
}

fn find_matching_brace(css: &str, after_open: usize) -> usize {
    let mut depth: usize = 1;
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut pos = after_open;
    while pos < len && depth > 0 {
        if bytes[pos] == b'{' {
            depth += 1;
        } else if bytes[pos] == b'}' {
            depth -= 1;
        }
        if depth > 0 {
            pos += 1;
        }
    }
    pos
}

fn sanitize_declarations(declarations: &str) -> String {
    declarations
        .split(';')
        .filter_map(|decl| {
            let trimmed = decl.trim();
            if trimmed.is_empty() {
                return None;
            }

            let lower = trimmed.to_ascii_lowercase();

            if lower.contains("expression(") {
                return None;
            }
            if lower.starts_with("-moz-binding") || lower.starts_with("behavior") {
                return None;
            }
            if lower.starts_with("position:") {
                let value = lower[9..].trim();
                if value.starts_with("fixed") {
                    return None;
                }
            }

            if contains_url_function(&lower) {
                return None;
            }

            Some(trimmed.to_owned())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Rewrites `<style>` element content through `sanitize_style_content()`.
fn sanitize_style_elements(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut search_from = 0;

    while search_from < html.len() {
        let open_start = match html[search_from..].find("<style") {
            Some(i) => search_from + i,
            None => {
                result.push_str(&html[search_from..]);
                break;
            }
        };

        result.push_str(&html[search_from..open_start]);

        let tag_search_from = open_start + 6;
        let open_end = match html[tag_search_from..].find('>') {
            Some(i) => tag_search_from + i + 1,
            None => {
                result.push_str(&html[open_start..]);
                break;
            }
        };

        let close_tag = match html[open_end..].find("</style>") {
            Some(i) => open_end + i,
            None => {
                result.push_str(&html[open_start..]);
                break;
            }
        };

        let css_content = &html[open_end..close_tag];
        let safe_content = sanitize_style_content(css_content);

        result.push_str(&html[open_start..open_end]);
        result.push_str(&safe_content);
        result.push_str("</style>");

        search_from = close_tag + 8;
    }

    result
}

/// Strips CSS declarations containing `url(` (with optional whitespace before `(`).
fn strip_css_url_functions(style: &str) -> String {
    style
        .split(';')
        .filter(|decl| !contains_url_function(decl))
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

fn contains_url_function(css: &str) -> bool {
    let mut start = 0;
    let lower = css.to_ascii_lowercase();
    while let Some(pos) = lower[start..].find("url") {
        let abs = start + pos;
        let after = lower[abs + 3..].trim_start_matches(|c: char| c.is_ascii_whitespace());
        if after.starts_with('(') {
            return true;
        }
        start = abs + 1;
    }
    false
}

/// Post-processes ammonia-sanitized HTML using two lol_html passes:
/// 1. Identify anchors where the visible text looks like a URL that differs from the href (phishing).
/// 2. Add `target="_blank"` to all `<a>` tags; rewrite identified phishing hrefs to the navigate warning page.
fn rewrite_links(html: &str) -> String {
    use lol_html::{element, text, HtmlRewriter, Settings};
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    // Pass 1: collect hrefs whose anchor text spoofs a different host.
    let phishing: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));
    let cur_href: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let cur_text: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    {
        let ph = Rc::clone(&phishing);
        let ch = Rc::clone(&cur_href);
        let ct = Rc::clone(&cur_text);
        let ch_t = Rc::clone(&cur_href);
        let ct_t = Rc::clone(&cur_text);

        let mut scanner = HtmlRewriter::new(
            Settings {
                element_content_handlers: vec![
                    element!("a[href]", move |el| {
                        let href = el.get_attribute("href").unwrap_or_default();
                        *ch.borrow_mut() = Some(href);
                        ct.borrow_mut().clear();

                        let ph2 = Rc::clone(&ph);
                        let ch2 = Rc::clone(&ch);
                        let ct2 = Rc::clone(&ct);
                        if let Some(handlers) = el.end_tag_handlers() {
                            handlers.push(Box::new(move |_end| {
                                let text = ct2.borrow().clone();
                                let href_ref = ch2.borrow();
                                let href = href_ref.as_deref().unwrap_or("");
                                if looks_like_url(text.trim()) && !same_host(text.trim(), href) {
                                    ph2.borrow_mut().insert(href.to_owned());
                                }
                                Ok(())
                            }));
                        }
                        Ok(())
                    }),
                    text!("a", move |t| {
                        if ch_t.borrow().is_some() {
                            ct_t.borrow_mut().push_str(t.as_str());
                        }
                        Ok(())
                    }),
                ],
                ..Settings::default()
            },
            |_: &[u8]| {},
        );
        let _ = scanner.write(html.as_bytes());
        let _ = scanner.end();
    }

    let phishing = phishing.borrow().clone();

    // Pass 2: add target="_blank" and rewrite phishing hrefs.
    let mut output = Vec::with_capacity(html.len() + 256);
    {
        let mut rewriter = HtmlRewriter::new(
            Settings {
                element_content_handlers: vec![
                    element!("a", move |el| {
                        if el.get_attribute("target").is_none() {
                            el.set_attribute("target", "_blank")?;
                        }
                        if let Some(href) = el.get_attribute("href") {
                            if phishing.contains(&href) {
                                let encoded: String =
                                    url::form_urlencoded::byte_serialize(href.as_bytes()).collect();
                                el.set_attribute("href", &format!("/api/email/navigate?to={encoded}"))?;
                            }
                        }
                        Ok(())
                    }),
                ],
                ..Settings::default()
            },
            |c: &[u8]| output.extend_from_slice(c),
        );
        let _ = rewriter.write(html.as_bytes());
        let _ = rewriter.end();
    }

    String::from_utf8(output).unwrap_or_else(|_| html.to_owned())
}

fn strip_tags(html: &str) -> String {
    const BLOCK: &[&str] = &[
        "p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "hr",
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

fn looks_like_url(text: &str) -> bool {
    if text.is_empty() || text.len() > 200 {
        return false;
    }
    let lower = text.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || (!lower.contains(' ') && lower.contains('.') && lower.len() >= 5)
}

fn same_host(text: &str, href: &str) -> bool {
    let host_of = |s: &str| -> String {
        let s = s.trim().to_ascii_lowercase();
        let url_str = if s.starts_with("http://") || s.starts_with("https://") {
            s.clone()
        } else if s.starts_with("//") {
            format!("https:{s}")
        } else {
            format!("https://{s}")
        };
        url::Url::parse(&url_str)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_ascii_lowercase()))
            .unwrap_or_default()
    };

    let text_host = host_of(text);
    let href_host = host_of(href);

    href_host.is_empty() || text_host.is_empty() || text_host == href_host
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
    cc: &[String],
    bcc: &[String],
    subject: &str,
    body: &str,
    in_reply_to: Option<&str>,
    attachments: Vec<(String, String, Vec<u8>)>,
) -> Result<Vec<u8>> {
    use lettre::{
        message::{header::ContentType, Attachment, MultiPart, SinglePart},
        AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
        transport::smtp::authentication::Credentials,
    };

    let mut builder = Message::builder()
        .from(from.parse().map_err(|e| anyhow!("invalid from address: {e}"))?)
        .subject(subject);

    for addr in to {
        builder = builder.to(addr.parse().map_err(|e| anyhow!("invalid to address: {e}"))?);
    }
    for addr in cc {
        builder = builder.cc(addr.parse().map_err(|e| anyhow!("invalid cc address: {e}"))?);
    }
    for addr in bcc {
        builder = builder.bcc(addr.parse().map_err(|e| anyhow!("invalid bcc address: {e}"))?);
    }
    if let Some(reply_to) = in_reply_to {
        builder = builder.in_reply_to(reply_to.to_owned());
    }

    let text_part = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_owned());

    let email = if attachments.is_empty() {
        builder.singlepart(text_part)
    } else {
        let mut mp = MultiPart::mixed().singlepart(text_part);
        for (filename, content_type_str, bytes) in attachments {
            let ct = content_type_str
                .parse::<ContentType>()
                .unwrap_or_else(|_| "application/octet-stream".parse().unwrap());
            mp = mp.singlepart(Attachment::new(filename).body(bytes, ct));
        }
        builder.multipart(mp)
    }
    .map_err(|e| anyhow!("failed to build email: {e}"))?;
    let raw_message = email.formatted();

    let creds = Credentials::new(username.to_owned(), password.to_owned());
    // Port 465 uses implicit TLS (SMTPS); 587/25 use STARTTLS.
    let builder = if smtp_port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(smtp_host)
    }
    .map_err(|e| anyhow!("SMTP relay error: {e}"))?;
    let mailer = builder
        .port(smtp_port)
        .credentials(creds)
        .build();

    mailer.send(email).await.map_err(|e| anyhow!("SMTP send failed: {e}"))?;
    Ok(raw_message)
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
        address::{Address, Envelope},
        transport::smtp::authentication::Credentials,
        AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    };

    let sender: Address = from.parse().map_err(|e| anyhow!("invalid from address: {e}"))?;
    let recipient_addresses = recipients
        .iter()
        .map(|recipient| recipient.parse::<Address>().map_err(|e| anyhow!("invalid recipient address: {e}")))
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
    fn test_title_tag_stripped_with_content() {
        let html = "<title>Zalando</title><p>Hello</p>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("Zalando"), "title content should be removed: {result}");
        assert!(result.contains("<p>Hello</p>"), "paragraph should remain: {result}");
    }

    #[test]
    fn test_title_with_complex_content() {
        let html = "<title>DB - Buchungsbestätigung</title><div>Body text</div>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("Buchungsbestätigung"), "title content should be removed: {result}");
        assert!(result.contains("Body text"), "body content should remain: {result}");
    }

    #[test]
    fn test_style_element_preserved() {
        let html = "<style>.preheader { display:none; }</style><div class=\"preheader\">Hidden</div><p>Visible</p>";
        let result = sanitize_email_html(html);
        assert!(result.contains("<style>"), "style tag should be preserved: {result}");
        assert!(result.contains("display:none"), "display:none rule should be preserved: {result}");
    }

    #[test]
    fn test_style_import_stripped() {
        let html = "<style>@import url('https://evil.com/track.css'); .preheader { display:none; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("@import"), "@import should be removed: {result}");
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
    }

    #[test]
    fn test_style_url_function_stripped() {
        let html = "<style>.bg { background-image: url('https://example.com/track.png'); } .hide { display:none; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("url("), "url() in style element should be removed: {result}");
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
    }

    #[test]
    fn test_style_expression_stripped() {
        let html = "<style>.evil { width: expression(document.cookie); display:none; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("expression"), "expression() should be removed: {result}");
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
    }

    #[test]
    fn test_style_moz_binding_stripped() {
        let html = "<style>.x { -moz-binding: url(bad); color: red; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("-moz-binding"), "-moz-binding should be removed: {result}");
        assert!(result.contains("color: red"), "safe property should be preserved: {result}");
    }

    #[test]
    fn test_style_behavior_stripped() {
        let html = "<style>.x { behavior: url(script.htc); display:none; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("behavior"), "behavior should be removed: {result}");
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
    }

    #[test]
    fn test_style_position_fixed_stripped() {
        let html = "<style>.overlay { position: fixed; top: 0; } .safe { position: absolute; top: 10px; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("position: fixed"), "position:fixed should be removed: {result}");
        assert!(result.contains("position: absolute"), "position:absolute should be preserved: {result}");
    }

    #[test]
    fn test_style_font_face_stripped() {
        let html = "<style>@font-face { font-family: 'Evil'; src: url('https://evil.com/font.woff2'); } .text { font-family: Arial; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("@font-face"), "@font-face should be removed: {result}");
        assert!(result.contains("font-family: Arial"), "safe font-family should be preserved: {result}");
    }

    #[test]
    fn test_style_keyframes_stripped() {
        let html = "<style>@keyframes slide { from { left: 0; } to { left: 100px; } } .box { color: red; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("@keyframes"), "@keyframes should be removed: {result}");
        assert!(result.contains("color: red"), "safe rule should be preserved: {result}");
    }

    #[test]
    fn test_style_media_queries_stripped() {
        let html = "<style>@media screen and (max-width: 600px) { .container { width: 100%; } } .container { width: 960px; }</style>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("@media"), "@media should be removed: {result}");
        assert!(result.contains("width: 960px"), "non-media rule should be preserved: {result}");
    }

    #[test]
    fn test_style_normal_css_preserved() {
        let html = "<style>body { color: #333; font-family: Arial, sans-serif; background-color: #fff; } table { border-collapse: collapse; }</style>";
        let result = sanitize_email_html(html);
        assert!(result.contains("color: #333"), "color should be preserved: {result}");
        assert!(result.contains("font-family:"), "font-family should be preserved: {result}");
        assert!(result.contains("background-color:"), "background-color should be preserved: {result}");
        assert!(result.contains("border-collapse:"), "border-collapse should be preserved: {result}");
    }

    #[test]
    fn test_combined_title_and_style() {
        let html = "<html><head><title>Zalando</title><style>.preheader{display:none;font-size:1px;color:#fff}</style></head><body><div class=\"preheader\">Zalando</div><p>Your order is confirmed</p></body></html>";
        let result = sanitize_email_html(html);
        assert!(!result.contains("<title>"), "title tag should be removed: {result}");
        let title_text_count = result.matches("Zalando").count();
        assert!(title_text_count <= 1, "Zalando should appear at most once in preheader div, found {title_text_count} times: {result}");
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
        assert!(result.contains("Your order is confirmed"), "body text should remain: {result}");
    }

    #[test]
    fn test_sanitize_style_content_preserves_safe_rules() {
        let css = ".hide { display:none; } .text { color:red; font-size:14px; }";
        let result = sanitize_style_content(css);
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
        assert!(result.contains("color:red"), "color should be preserved: {result}");
        assert!(result.contains("font-size:14px"), "font-size should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_import() {
        let css = "@import url('evil.css'); .safe { color:blue; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("@import"), "@import should be removed: {result}");
        assert!(result.contains("color:blue"), "safe rule should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_import_without_braces() {
        let css = "@import 'evil.css'; .safe { color:blue; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("@import"), "@import should be removed: {result}");
        assert!(result.contains("color:blue"), "safe rule should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_font_face() {
        let css = "@font-face { font-family: 'Test'; src: url('font.woff2'); } .text { font-family: 'Test'; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("@font-face"), "@font-face should be removed: {result}");
        assert!(result.contains("font-family: 'Test'"), "safe font-family should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_keyframes() {
        let css = "@keyframes x { from { opacity: 0; } to { opacity: 1; } } .faded { opacity: 0; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("@keyframes"), "@keyframes should be removed: {result}");
        assert!(result.contains("opacity: 0"), "safe opacity rule should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_url_in_declarations() {
        let css = ".bg { background-image: url('https://evil.com/track.png'); color: red; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("url("), "url() should be removed: {result}");
        assert!(result.contains("color: red"), "safe property should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_position_fixed() {
        let css = ".overlay { position:fixed; top:0; } .safe { position:relative; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("position:fixed"), "position:fixed should be removed: {result}");
        assert!(result.contains("position:relative"), "position:relative should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_elements_replaces_content() {
        let html = "<div>before</div><style>.hide{display:none;}</style><div>after</div>";
        let result = sanitize_style_elements(html);
        assert!(result.contains("<style>"), "style tag should remain: {result}");
        assert!(result.contains("display:none"), "safe CSS should remain: {result}");
        assert!(result.contains("before"), "surrounding content should remain: {result}");
        assert!(result.contains("after"), "surrounding content should remain: {result}");
    }

    #[test]
    fn test_sanitize_style_elements_multiple_style_blocks() {
        let html = "<style>.a{color:red;}</style><p>text</p><style>.b{color:blue;}</style>";
        let result = sanitize_style_elements(html);
        assert!(result.contains("color:red"), "first style block should be preserved: {result}");
        assert!(result.contains("color:blue"), "second style block should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_elements_strips_dangerous_content() {
        let html = "<style>@import url('evil.css'); .safe{color:green;}</style>";
        let result = sanitize_style_elements(html);
        assert!(!result.contains("@import"), "@import should be removed: {result}");
        assert!(result.contains("color:green"), "safe rule should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_expression() {
        let css = ".evil { width: expression(document.cookie); color: blue; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("expression"), "expression() should be removed: {result}");
        assert!(result.contains("color: blue"), "safe property should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_behavior() {
        let css = ".evil { behavior: url(script.htc); display: block; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("behavior"), "behavior should be removed: {result}");
        assert!(result.contains("display: block"), "display should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_strips_moz_binding() {
        let css = ".evil { -moz-binding: url(xbl); background: white; }";
        let result = sanitize_style_content(css);
        assert!(!result.contains("-moz-binding"), "-moz-binding should be removed: {result}");
        assert!(result.contains("background: white"), "background should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_preserves_position_absolute() {
        let css = ".safe { position: absolute; top: 10px; }";
        let result = sanitize_style_content(css);
        assert!(result.contains("position: absolute"), "position:absolute should be preserved: {result}");
        assert!(result.contains("top: 10px"), "top should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_declarations_strips_url() {
        let result = sanitize_declarations("background-image: url(evil.png); color: red");
        assert!(!result.contains("url("), "url() should be stripped: {result}");
        assert!(result.contains("color: red"), "safe declaration should remain: {result}");
    }

    #[test]
    fn test_sanitize_declarations_strips_css_url_with_spaces() {
        let result = sanitize_declarations("background: url ( evil.png ); color: blue");
        assert!(!result.contains("url"), "url() with spaces should be stripped: {result}");
        assert!(result.contains("color: blue"), "safe declaration should remain: {result}");
    }

    #[test]
    fn test_style_with_attributes_preserved() {
        let html = "<style type=\"text/css\">.hide { display:none; }</style>";
        let result = sanitize_style_elements(html);
        assert!(result.contains("display:none"), "display:none should be preserved: {result}");
    }

    #[test]
    fn test_empty_style_removed() {
        let html = "<style>@import url('evil.css');</style><p>Safe</p>";
        let result = sanitize_style_elements(html);
        assert!(!result.contains("@import"), "@import should be removed: {result}");
        // The entire style block should be removed since no content remains
    }

    #[test]
    fn test_contains_url_function() {
        assert!(contains_url_function("background: url(foo.png)"));
        assert!(contains_url_function("background: url ( foo.png )"));
        assert!(contains_url_function("list-style-image: url('bullet.png')"));
        assert!(!contains_url_function("color: red"));
        assert!(!contains_url_function("border: solid"));
    }

    #[test]
    fn test_sanitize_style_content_with_multibyte_utf8() {
        let css = ".ExternalClass, .ExternalClass p { line-height:\u{2028}100%; } body { color: résumé; }";
        let result = sanitize_style_content(css);
        assert!(result.contains("line-height"), "line-height should be preserved: {result}");
        assert!(result.contains("color:"), "color should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_style_content_with_unicode_selectors() {
        let css = ".résumé { color: blue; } .über { font-size: 14px; }";
        let result = sanitize_style_content(css);
        assert!(result.contains("color: blue"), "color should be preserved: {result}");
        assert!(result.contains("font-size: 14px"), "font-size should be preserved: {result}");
    }

    #[test]
    fn test_sanitize_email_html_with_utf8_in_style() {
        let html = "<style>.x { font-family:\"Hélvética\"; }</style><p>Test</p>";
        let result = sanitize_email_html(html);
        assert!(result.contains("font-family"), "font-family should be preserved: {result}");
    }

    #[test]
    fn test_cid_src_survives_sanitization() {
        let html = "<img src=\"cid:logo@example.com\" alt=\"logo\">";
        let result = sanitize_email_html(html);
        assert!(
            result.contains("cid:logo@example.com"),
            "cid: src must survive sanitization so rewrite_cid_urls can rewrite it: {result}"
        );
    }

    #[test]
    fn test_cid_src_rewritten_to_proxy() {
        let html = "<img src=\"cid:logo@example.com\">";
        let sanitized = sanitize_email_html(html);
        let rewritten = rewrite_cid_urls(&sanitized, "acct1", "INBOX", 42);
        assert!(
            rewritten.contains("/api/email/proxy-image?account_id=acct1"),
            "cid: src should be rewritten to the proxy endpoint: {rewritten}"
        );
        assert!(
            rewritten.contains("cid=logo%40example.com"),
            "cid value should be url-encoded in the proxy URL: {rewritten}"
        );
        assert!(
            !rewritten.contains("src=\"cid:"),
            "no raw cid: src should remain after rewrite: {rewritten}"
        );
    }

    #[test]
    fn test_data_image_src_preserved() {
        let html = "<img src=\"data:image/png;base64,iVBORw0KGgo=\">";
        let result = sanitize_email_html(html);
        assert!(
            result.contains("data:image/png;base64,iVBORw0KGgo="),
            "data:image/png src should survive sanitization: {result}"
        );
    }

    #[test]
    fn test_data_image_svg_src_dropped() {
        let html = "<img src=\"data:image/svg+xml,<svg onload=alert(1)></svg>\">";
        let result = sanitize_email_html(html);
        assert!(
            !result.contains("data:image/svg"),
            "data:image/svg src must be dropped (script vector): {result}"
        );
    }

    #[test]
    fn test_data_href_dropped() {
        let html = "<a href=\"data:text/html,<script>alert(1)</script>\">click</a>";
        let result = sanitize_email_html(html);
        assert!(
            !result.contains("data:text/html"),
            "data: href must be stripped to prevent XSS: {result}"
        );
        assert!(result.contains("click"), "link text should remain: {result}");
    }

    #[test]
    fn test_data_href_dropped_with_leading_space_and_case() {
        let html = "<a href=\" DATA:text/html,<script>alert(1)</script>\">x</a>";
        let result = sanitize_email_html(html);
        assert!(
            !result.to_ascii_lowercase().contains("data:text/html"),
            "data: href must be stripped regardless of case/leading space: {result}"
        );
    }

    #[test]
    fn test_utf8_char_width() {
        assert_eq!(utf8_char_width(b'a'), 1);
        assert_eq!(utf8_char_width(b'{'), 1);
        assert_eq!(utf8_char_width(0xC3), 2); // é = 0xC3 0xA9
        assert_eq!(utf8_char_width(0xE2), 3); // \u{2028} = 0xE2 0x80 0xA8
        assert_eq!(utf8_char_width(0xF0), 4); // emoji lead byte
        assert_eq!(utf8_char_width(0x80), 1); // continuation byte, fallback to 1
    }
}

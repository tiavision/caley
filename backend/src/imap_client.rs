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

/// Fetches the full raw body for a single UID. Returns (body_text, body_html, has_attachments).
pub async fn fetch_body(session: &mut ImapSession, uid: u32) -> Result<(String, Option<String>, bool)> {
    let mut stream = session.uid_fetch(uid.to_string(), "(BODY.PEEK[])").await?;
    if let Some(item) = stream.next().await {
        let fetch = item?;
        if let Some(body_bytes) = fetch.body() {
            return parse_body_bytes(body_bytes);
        }
    }
    Ok((String::new(), None, false))
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

fn parse_body_bytes(bytes: &[u8]) -> Result<(String, Option<String>, bool)> {
    use mail_parser::MessageParser;
    let parser = MessageParser::default();
    let msg = match parser.parse(bytes) {
        Some(m) => m,
        None => return Ok((String::from_utf8_lossy(bytes).into_owned(), None, false)),
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

    Ok((body_text, body_html, has_attachments))
}

// ── HTML sanitization for email display ──────────────────────────────────────

pub fn sanitize_email_html(html: &str) -> String {
    use std::borrow::Cow;
    use std::collections::{HashMap, HashSet};

    let allowed_tags: HashSet<&str> = [
        "a", "abbr", "b", "blockquote", "br", "cite", "code", "col", "colgroup",
        "del", "div", "dl", "dt", "dd", "em", "font", "h1", "h2", "h3", "h4",
        "h5", "h6", "hr", "i", "img", "ins", "li", "ol", "p", "pre", "s",
        "small", "span", "strike", "strong", "sub", "sup", "table", "tbody",
        "td", "tfoot", "th", "thead", "tr", "u", "ul",
    ]
    .iter()
    .copied()
    .collect();

    let clean_content: HashSet<&str> = ["script", "style", "head", "noscript"]
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

    let url_schemes: HashSet<&str> = ["http", "https", "mailto"].iter().copied().collect();
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
                    } else if value.starts_with("data:image/") {
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
                _ => Some(Cow::Borrowed(value)),
            }
        })
        .clean(html)
        .to_string();

    rewrite_links(&clean)
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

/// Post-processes ammonia-sanitized HTML: adds `target="_blank"` to all `<a>` tags
/// and rewrites hrefs that spoof a URL in the anchor text to the navigate warning endpoint.
fn rewrite_links(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut s = html;

    loop {
        let Some(i) = find_open_a(s) else {
            out.push_str(s);
            break;
        };

        out.push_str(&s[..i]);
        s = &s[i..];

        let Some(j) = find_tag_end(s) else {
            out.push_str(s);
            break;
        };

        let opening = &s[..j + 1];
        s = &s[j + 1..];

        // Find matching </a> (case-insensitive)
        let lower_rest = s.to_ascii_lowercase();
        if let Some(close) = lower_rest.find("</a>") {
            let inner = &s[..close];
            s = &s[close + 4..];

            let new_opening = process_a_tag(opening, inner);
            out.push_str(&new_opening);
            out.push_str(inner);
            out.push_str("</a>");
        } else {
            // No closing tag — emit opening as-is with target="_blank"
            out.push_str(&add_target_blank(opening));
        }
    }

    out
}

fn find_open_a(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let pos = lower[from..].find("<a").map(|p| from + p)?;
        // Ensure it's <a> or <a followed by whitespace (not <abbr etc.)
        let after = s.as_bytes().get(pos + 2).copied();
        if matches!(after, Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'>') | None) {
            return Some(pos);
        }
        from = pos + 2;
    }
}

/// Finds the closing `>` of a tag, respecting quoted attribute values.
fn find_tag_end(s: &str) -> Option<usize> {
    let mut in_q: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (c, in_q) {
            ('"' | '\'', None) => in_q = Some(c),
            (q, Some(open)) if q == open => in_q = None,
            ('>', None) => return Some(i),
            _ => {}
        }
    }
    None
}

fn process_a_tag(opening: &str, inner: &str) -> String {
    let with_target = add_target_blank(opening);

    let text = strip_tags(inner);
    let text = text.trim();

    if let Some(href) = get_href(&with_target) {
        if looks_like_url(text) && !same_host(text, href) {
            let encoded: String =
                url::form_urlencoded::byte_serialize(href.as_bytes()).collect();
            let new_href = format!("/api/email/navigate?to={encoded}");
            return replace_href_value(&with_target, href, &new_href);
        }
    }

    with_target
}

fn add_target_blank(opening: &str) -> String {
    if opening.to_ascii_lowercase().contains("target=") {
        return opening.to_string();
    }
    // Insert before the closing >
    if let Some(pos) = opening.rfind('>') {
        format!("{} target=\"_blank\">", &opening[..pos])
    } else {
        opening.to_string()
    }
}

fn get_href(opening: &str) -> Option<&str> {
    let lower = opening.to_ascii_lowercase();
    let href_pos = lower.find("href=")?;
    let rest = &opening[href_pos + 5..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)?;
        Some(&rest[1..end + 1])
    } else {
        None
    }
}

fn replace_href_value(opening: &str, old_href: &str, new_href: &str) -> String {
    opening.replacen(old_href, new_href, 1)
}

fn strip_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
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

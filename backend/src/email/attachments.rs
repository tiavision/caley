use std::time::Duration;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::email_db;
use crate::email_sync::eml_path;
use crate::oidc::{AppState, CurrentUser};
use crate::version;

fn decompress_eml(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::decode_all(bytes)
}

fn extract_part_bytes(eml_bytes: &[u8], part_index: usize) -> Option<Vec<u8>> {
    use mail_parser::MessageParser;
    let msg = MessageParser::default().parse(eml_bytes)?;
    // The position among the root message's own parts, as `extract_attachments`
    // in `imap_client` numbers them.
    let part = msg.root().parts().nth(part_index)?;
    part_to_bytes(part)
}

fn part_to_bytes(part: mail_parser::MessagePart<'_>) -> Option<Vec<u8>> {
    match part.kind() {
        mail_parser::PartKind::Binary | mail_parser::PartKind::InlineBinary => {
            Some(part.decoded().into_owned())
        }
        mail_parser::PartKind::Text | mail_parser::PartKind::Html => {
            part.text().map(|t| t.into_owned().into_bytes())
        }
        _ => None,
    }
}

/// Scan all MIME parts in the EML for one whose Content-ID matches `cid`
/// (with or without surrounding angle brackets).  Returns the decoded bytes
/// when found.  Used as a fallback when the attachment DB row is missing.
fn find_bytes_by_cid(eml_bytes: &[u8], cid: &str) -> Option<Vec<u8>> {
    use mail_parser::MessageParser;
    let msg = MessageParser::default().parse(eml_bytes)?;
    for part in msg.root().parts() {
        if let Some(part_cid) = part.content_id() {
            let bare = part_cid.trim_start_matches('<').trim_end_matches('>');
            if bare == cid || part_cid == cid {
                return part_to_bytes(part);
            }
        }
    }
    None
}

// ── Image proxy ───────────────────────────────────────────────────────────────

/// Either `url` (a remote image of a whitelisted origin) or `cid` with `account_id`, `mailbox`
/// and `uid` (an inline image of a cached message).
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProxyImageQuery {
    /// the remote image's http(s) URL
    pub url: Option<String>,
    pub account_id: Option<String>,
    pub mailbox: Option<String>,
    pub uid: Option<u32>,
    /// the inline part's Content-ID, with or without angle brackets
    pub cid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowedImage {
    Png,
    Jpeg,
    Gif,
    WebP,
    Avif,
}

impl AllowedImage {
    fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
            Self::Avif => "image/avif",
        }
    }
}

fn detect_allowed_image(data: &[u8]) -> Option<AllowedImage> {
    match infer::get(data).map(|t| t.mime_type()) {
        Some("image/png") => Some(AllowedImage::Png),
        Some("image/jpeg") => Some(AllowedImage::Jpeg),
        Some("image/gif") => Some(AllowedImage::Gif),
        Some("image/webp") => Some(AllowedImage::WebP),
        Some("image/avif") => Some(AllowedImage::Avif),
        _ => None,
    }
}

/// An image of a message: an inline part by Content-ID, or a remote image of a whitelisted origin.
/// Only PNG, JPEG, GIF, WebP and AVIF (sniffed from the bytes) are served; anything else, an
/// origin not whitelisted, an unsafe address or a failed fetch is a 1×1 transparent GIF.
#[utoipa::path(
    get,
    path = "/email/proxy-image",
    params(ProxyImageQuery),
    responses(
        (status = 200, description = "the image (PNG, JPEG, GIF, WebP or AVIF), or a 1×1 transparent GIF", body = Vec<u8>, content_type = "image/*"),
        (status = 400, description = "`url` missing or not http(s) (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn proxy_image(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ProxyImageQuery>,
) -> Response {
    // CID-based inline attachment lookup
    if let (Some(cid), Some(account_id), Some(mailbox), Some(uid)) =
        (&q.cid, &q.account_id, &q.mailbox, q.uid)
    {
        // Verify the account belongs to the current user before serving its data.
        match email_db::get_account(&state.db, account_id, &user.user_id, &state.encryption_key)
            .await
        {
            Ok(Some(_)) => {}
            _ => return transparent_pixel(),
        }
        let eml_raw =
            match tokio::fs::read(eml_path(&state.data_dir, account_id, mailbox, uid)).await {
                Ok(b) => b,
                Err(_) => return transparent_pixel(),
            };
        let eml = decompress_eml(&eml_raw).unwrap_or_default();

        // Try the indexed path first (fast); fall back to scanning all MIME
        // parts by Content-ID for messages cached before the index was built.
        let bytes =
            match email_db::get_attachment_by_cid(&state.db, account_id, mailbox, uid, cid).await {
                Ok(Some(att)) => match extract_part_bytes(&eml, att.part_index as usize) {
                    Some(b) => b,
                    None => return transparent_pixel(),
                },
                _ => match find_bytes_by_cid(&eml, cid) {
                    Some(b) => b,
                    None => return transparent_pixel(),
                },
            };

        // Sniff bytes before serving inline attachments. Declared MIME type is
        // untrusted and could otherwise make active content same-origin.
        let Some(kind) = detect_allowed_image(&bytes) else {
            return transparent_pixel();
        };
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, kind.content_type().to_string()),
                (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
                (
                    header::HeaderName::from_static("x-content-type-options"),
                    "nosniff".to_string(),
                ),
            ],
            bytes,
        )
            .into_response();
    }

    // External URL proxy (original behaviour)
    let Some(url) = q.url else {
        return (StatusCode::BAD_REQUEST, "invalid url").into_response();
    };

    let parsed = match url::Url::parse(&url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => return (StatusCode::BAD_REQUEST, "invalid url").into_response(),
    };

    if !is_ssrf_safe(&parsed).await {
        return transparent_pixel();
    }

    let Some(host) = parsed.host_str() else {
        return transparent_pixel();
    };
    let origin = format!("{}://{}", parsed.scheme(), host);

    match email_db::is_origin_whitelisted(&state.db, &user.user_id, &origin).await {
        Ok(true) => {}
        _ => return transparent_pixel(),
    }

    // Do not follow redirects: a whitelisted origin could otherwise 302 to an
    // internal address (e.g. 127.0.0.1 / link-local metadata) that the pre-fetch
    // is_ssrf_safe() check never sees.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(version::USER_AGENT)
        .use_preconfigured_tls(crate::imap_client::tls_config())
        .redirect(reqwest::redirect::Policy::default())
        .build()
    {
        Ok(c) => c,
        Err(_) => return transparent_pixel(),
    };

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return transparent_pixel(),
    };

    if !resp.status().is_success() {
        return transparent_pixel();
    }

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(_) => {
            return transparent_pixel();
        }
    };

    let kind = match detect_allowed_image(&bytes) {
        Some(k) => k,
        None => return transparent_pixel(),
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, kind.content_type().to_string()),
            (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

// ── Attachment download ───────────────────────────────────────────────────────

fn can_preview_attachment(content_type: &str) -> bool {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    matches!(
        content_type.as_str(),
        "application/pdf"
            | "text/plain"
            | "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
    ) || (content_type.starts_with("audio/") && content_type.len() > "audio/".len())
        || (content_type.starts_with("video/") && content_type.len() > "video/".len())
}

fn attachment_disposition(content_type: &str, filename: &str) -> String {
    let kind = if can_preview_attachment(content_type) {
        "inline"
    } else {
        "attachment"
    };
    format!("{kind}; filename=\"{filename}\"")
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DownloadAttachmentQuery {
    pub account_id: String,
    pub mailbox: String,
    pub uid: u32,
    pub filename: String,
}

/// An attachment of a cached message, with its declared content type. Content-Disposition is
/// `inline` for previewable types (PDF, plain text, raster images, audio, video), else `attachment`.
#[utoipa::path(
    get,
    path = "/email/attachment",
    params(DownloadAttachmentQuery),
    responses(
        (status = 200, description = "the attachment's bytes, as its declared content type (`application/octet-stream` without one)", body = Vec<u8>, content_type = "application/octet-stream"),
        (status = 404, description = "no such account, attachment or part (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database or file error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn download_attachment(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<DownloadAttachmentQuery>,
) -> Response {
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
            eprintln!("[email] download_attachment account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
        Ok(Some(_)) => {}
    }

    let att = match email_db::get_attachment_by_filename(
        &state.db,
        &q.account_id,
        &q.mailbox,
        q.uid,
        &q.filename,
    )
    .await
    {
        Ok(Some(a)) => a,
        Ok(None) => return (StatusCode::NOT_FOUND, "attachment not found").into_response(),
        Err(e) => {
            eprintln!("[email] download_attachment lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let eml =
        match tokio::fs::read(eml_path(&state.data_dir, &q.account_id, &q.mailbox, q.uid)).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[email] download_attachment read error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "file read error").into_response();
            }
        };
    let eml = match decompress_eml(&eml) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[email] download_attachment decompress error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "decompress error").into_response();
        }
    };
    let bytes = match extract_part_bytes(&eml, att.part_index as usize) {
        Some(b) => b,
        None => return (StatusCode::NOT_FOUND, "attachment part not found").into_response(),
    };

    let ct = att
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());
    // Strip control chars / quotes so the (user-supplied) filename can't break out
    // of the header value or inject additional headers.
    let safe_name: String = q
        .filename
        .chars()
        .map(|c| {
            if c.is_control() || c == '"' || c == '\\' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let disposition = attachment_disposition(&ct, &safe_name);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, ct),
            (
                header::HeaderName::from_static("content-disposition"),
                disposition,
            ),
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

// ── Export ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExportQuery {
    pub account_id: String,
}

/// Every cached message of an account as a gzipped tar archive (`<mailbox>/<uid>.eml`), named
/// `<account display name>.tar.gz`; streamed.
#[utoipa::path(
    get,
    path = "/email/export",
    params(ExportQuery),
    responses(
        (status = 200, description = "a .tar.gz of the account's messages", body = Vec<u8>, content_type = "application/gzip"),
        (status = 404, description = "no such account (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn export_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ExportQuery>,
) -> Response {
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
            eprintln!("[email] export_account account lookup error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let rows = match sqlx::query_as::<_, (String, i64)>(
        "SELECT mailbox, uid FROM cached_messages WHERE account_id = ? ORDER BY mailbox, uid",
    )
    .bind(&q.account_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[email] export_account query error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let account_id = q.account_id.clone();
    let data_dir = state.data_dir.clone();
    let display_name = account.display_name.replace(['"', '\\', '\0'], "_");

    let (writer, reader) = tokio::io::duplex(128 * 1024);

    tokio::task::spawn_blocking(move || {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tokio_util::io::SyncIoBridge;

        let sync_writer = SyncIoBridge::new(writer);
        let gz = GzEncoder::new(sync_writer, Compression::default());
        let mut tar = tar::Builder::new(gz);

        for (mailbox, uid) in &rows {
            let path = eml_path(&data_dir, &account_id, mailbox, *uid as u32);
            let entry_name = format!("{}/{}.eml", mailbox, uid);
            if let Ok(compressed) = std::fs::read(&path) {
                if let Ok(bytes) = decompress_eml(&compressed) {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(bytes.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    let _ = tar.append_data(&mut header, &entry_name, bytes.as_slice());
                }
            }
        }
        let _ = tar.finish();
    });

    let stream = tokio_util::io::ReaderStream::new(reader);
    let filename = format!("{display_name}.tar.gz");

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn is_ssrf_safe(parsed: &url::Url) -> bool {
    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
    match tokio::net::lookup_host(format!("{host}:{port}")).await {
        Ok(mut addrs) => addrs.all(|a| !is_private_ip(a.ip())),
        Err(_) => false,
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_private_v4(v4),
        std::net::IpAddr::V6(v6) => {
            // IPv4-mapped addresses (::ffff:a.b.c.d) connect to the embedded IPv4
            // host, so apply the v4 rules — otherwise ::ffff:127.0.0.1 bypasses.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_v4(v4);
            }
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (seg0 & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (seg0 & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

fn is_private_v4(v4: std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_unspecified()
}

fn transparent_pixel() -> Response {
    // 1×1 transparent GIF
    const GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xff, 0xff,
        0xff, 0x00, 0x00, 0x00, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
    ];
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/gif"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff",
            ),
        ],
        GIF,
    )
        .into_response()
}

// ── Navigate (link warning page) ─────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NavigateQuery {
    /// the external http(s) URL
    pub to: String,
}

/// A warning page before an external link of a message: the destination, Continue and Cancel.
#[utoipa::path(
    get,
    path = "/email/navigate",
    params(NavigateQuery),
    responses(
        (status = 200, description = "the warning page", body = String, content_type = "text/html"),
        (status = 400, description = "`to` not an http(s) URL (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn navigate(Query(q): Query<NavigateQuery>) -> Response {
    let url = &q.to;
    match url::Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => {}
        _ => return (StatusCode::BAD_REQUEST, "invalid url").into_response(),
    }

    let esc = html_escape(url);
    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>External Link</title>
<style>
  body {{ font-family: sans-serif; max-width: 560px; margin: 3rem auto; padding: 0 1.5rem; color: #222; }}
  h2 {{ font-size: 1.1rem; margin-bottom: 1rem; }}
  .url {{ word-break: break-all; background: #f5f5f5; padding: .6rem .8rem; border-radius: 4px; margin: 1rem 0; font-size: .9rem; }}
  .actions {{ display: flex; gap: .75rem; margin-top: 1.5rem; }}
  .continue {{ background: #2a6496; color: #fff; padding: .55rem 1.1rem; border-radius: 4px; text-decoration: none; font-size: .9rem; }}
  .cancel {{ padding: .55rem 1.1rem; border-radius: 4px; text-decoration: none; font-size: .9rem; border: 1px solid #999; color: inherit; cursor: pointer; background: none; }}
</style>
</head>
<body>
<h2>You are about to visit an external site</h2>
<p>Destination:</p>
<div class="url">{esc}</div>
<div class="actions">
  <a class="continue" href="{esc}">Continue</a>
  <button class="cancel" onclick="window.close()">Cancel</button>
</div>
</body>
</html>"##
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ── Image origin whitelist API ────────────────────────────────────────────────

/// A list of image origins (`scheme://host`): the allowed ones or the ignored ones.
#[derive(serde::Serialize, ToSchema)]
pub struct ImageOrigins {
    pub origins: Vec<String>,
}

/// The origins whose remote images are loaded.
#[utoipa::path(
    get,
    path = "/email/image-origins",
    responses(
        (status = 200, body = ImageOrigins),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn list_image_origins(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::get_image_whitelist(&state.db, &user.user_id).await {
        Ok(origins) => Json(ImageOrigins { origins }).into_response(),
        Err(e) => {
            eprintln!("[email] list_image_origins error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ImageOriginBody {
    /// an http(s) URL; stored as its `scheme://host`
    pub origin: String,
}

/// Allows the remote images of an origin.
#[utoipa::path(
    post,
    path = "/email/image-origins",
    request_body = ImageOriginBody,
    responses(
        (status = 204, description = "added"),
        (status = 400, description = "not an http(s) origin (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn add_image_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ImageOriginBody>,
) -> impl IntoResponse {
    // Validate that the origin is a well-formed http(s) origin
    match url::Url::parse(&body.origin) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => {}
        _ => return (StatusCode::BAD_REQUEST, "invalid origin").into_response(),
    }
    // Normalise to scheme://host (strip path/query)
    let origin = {
        let u = url::Url::parse(&body.origin).unwrap();
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""))
    };

    match email_db::add_image_origin(&state.db, &user.user_id, &origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] add_image_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImageOriginQuery {
    pub origin: String,
}

#[utoipa::path(
    delete,
    path = "/email/image-origins",
    params(ImageOriginQuery),
    responses(
        (status = 204, description = "removed (also when it was not listed)"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn remove_image_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ImageOriginQuery>,
) -> impl IntoResponse {
    match email_db::remove_image_origin(&state.db, &user.user_id, &q.origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] remove_image_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

// ── Image origin blacklist API ─────────────────────────────────────────────

/// The origins whose remote images are ignored (not offered for loading).
#[utoipa::path(
    get,
    path = "/email/image-blacklist",
    responses(
        (status = 200, body = ImageOrigins),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn list_image_blacklist(
    State(state): State<AppState>,
    user: CurrentUser,
) -> impl IntoResponse {
    match email_db::get_image_blacklist(&state.db, &user.user_id).await {
        Ok(origins) => Json(ImageOrigins { origins }).into_response(),
        Err(e) => {
            eprintln!("[email] list_image_blacklist error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// Ignores the remote images of an origin.
#[utoipa::path(
    post,
    path = "/email/image-blacklist",
    request_body = ImageOriginBody,
    responses(
        (status = 204, description = "added"),
        (status = 400, description = "not an http(s) origin (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn add_image_blacklist_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Json(body): Json<ImageOriginBody>,
) -> impl IntoResponse {
    match url::Url::parse(&body.origin) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host().is_some() => {}
        _ => return (StatusCode::BAD_REQUEST, "invalid origin").into_response(),
    }
    let origin = {
        let u = url::Url::parse(&body.origin).unwrap();
        format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""))
    };

    match email_db::add_image_blacklist(&state.db, &user.user_id, &origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] add_image_blacklist_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/email/image-blacklist",
    params(ImageOriginQuery),
    responses(
        (status = 204, description = "removed (also when it was not listed)"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn remove_image_blacklist_origin(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ImageOriginQuery>,
) -> impl IntoResponse {
    match email_db::remove_image_blacklist(&state.db, &user.user_id, &q.origin).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("[email] remove_image_blacklist_origin error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
        }
    }
}

/// This file's routes that need a signed-in user, relative to `/api` (api.rs).
pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(proxy_image))
        .routes(routes!(download_attachment))
        .routes(routes!(export_account))
        .routes(routes!(
            list_image_origins,
            add_image_origin,
            remove_image_origin
        ))
        .routes(routes!(
            list_image_blacklist,
            add_image_blacklist_origin,
            remove_image_blacklist_origin
        ))
}

/// This file's public routes, relative to `/api` (api.rs).
pub fn public_routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(navigate))
}

#[cfg(test)]
mod tests {
    use super::{
        AllowedImage, attachment_disposition, can_preview_attachment, detect_allowed_image,
        extract_part_bytes,
    };

    #[test]
    fn reads_an_attachment_by_its_root_part_index() {
        let raw = crate::imap_client::tests::FORWARDED_THEN_ATTACHMENT;
        assert_eq!(extract_part_bytes(raw, 3).as_deref(), Some(&b"hello"[..]));
        assert_eq!(
            extract_part_bytes(raw, 1).as_deref(),
            Some(&b"See below"[..])
        );
    }

    #[test]
    fn previewable_attachment_types_are_allowlisted_case_insensitively() {
        for content_type in [
            "application/pdf",
            "APPLICATION/PDF; version=1.7",
            " text/plain; charset=utf-8",
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/avif",
            "audio/mpeg",
            "video/mp4",
        ] {
            assert!(can_preview_attachment(content_type), "{content_type}");
        }
    }

    #[test]
    fn active_and_unknown_attachment_types_are_not_previewable() {
        for content_type in [
            "text/html",
            "image/svg+xml",
            "application/xml",
            "application/javascript",
            "application/octet-stream",
            "audio/",
            "video/",
            "",
        ] {
            assert!(!can_preview_attachment(content_type), "{content_type}");
        }
    }

    #[test]
    fn disposition_uses_preview_policy() {
        assert_eq!(
            attachment_disposition("application/pdf", "file.pdf"),
            "inline; filename=\"file.pdf\""
        );
        assert_eq!(
            attachment_disposition("text/html", "file.html"),
            "attachment; filename=\"file.html\""
        );
    }

    #[test]
    fn image_detection_sniffs_bytes_not_declared_type() {
        let png =
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";
        assert_eq!(detect_allowed_image(png), Some(AllowedImage::Png));

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(detect_allowed_image(svg), None);

        let html = b"<!doctype html><script>alert(1)</script>";
        assert_eq!(detect_allowed_image(html), None);
    }
}

use std::sync::{Arc, LazyLock};
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
use crate::public_net::{PublicOnly, literal_host_is_public};
use crate::raster_image::{AllowedImage, detect_allowed_image};
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

/// An image of a message: an inline part by Content-ID, or a remote image of a whitelisted origin.
/// Only PNG, JPEG, GIF, WebP and AVIF (sniffed from the bytes) are served; anything else, an
/// origin not whitelisted, a non-public address, a redirect, more than 10 MiB or a failed fetch is a
/// 1×1 transparent GIF.
#[utoipa::path(
    get,
    path = "/email/proxy-image",
    params(ProxyImageQuery),
    responses(
        (status = 200, description = "the image (PNG, JPEG, GIF, WebP or AVIF), or a 1×1 transparent GIF", body = crate::api::Binary, content_type = "image/*"),
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
        let account = match email_db::get_account(
            &state.db,
            account_id,
            &user.user_id,
            &state.encryption_key,
        )
        .await
        {
            Ok(Some(account)) => account,
            _ => return transparent_pixel(),
        };
        let eml_raw =
            match tokio::fs::read(eml_path(&state.data_dir, account.id(), mailbox, uid)).await {
                Ok(b) => b,
                Err(_) => return transparent_pixel(),
            };
        let eml = decompress_eml(&eml_raw).unwrap_or_default();

        // Try the indexed path first (fast); fall back to scanning all MIME
        // parts by Content-ID for messages cached before the index was built.
        let bytes =
            match email_db::get_attachment_by_cid(&state.db, account.id(), mailbox, uid, cid).await
            {
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

    // an address in the URL is not resolved (PublicOnly): checked here
    if !literal_host_is_public(&parsed) {
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

    let Some(client) = image_client() else {
        return transparent_pixel();
    };
    let Some((kind, bytes)) = fetch_image(client, parsed).await else {
        return transparent_pixel();
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

/// The most a remote image may be (10 MiB): a larger one is the transparent pixel.
const MAX_REMOTE_IMAGE: usize = 10 << 20;

/// The client remote images are fetched with (one for all requests). Names resolve to public addresses only
/// (`PublicOnly`; also on a later lookup, DNS rebinding), and it does not follow redirects: a whitelisted
/// origin could otherwise 302 to an internal address. A redirect is not a success, so it is the pixel.
fn image_client() -> Option<&'static reqwest::Client> {
    static CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
        image_client_builder()
            .dns_resolver(Arc::new(PublicOnly))
            .build()
            .inspect_err(|e| eprintln!("[email] image client: {e}"))
            .ok()
    });
    CLIENT.as_ref()
}

/// `image_client` but its resolver.
fn image_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(version::USER_AGENT)
        .use_preconfigured_tls(crate::imap_client::tls_config())
        .redirect(reqwest::redirect::Policy::none())
}

/// A remote image: a success, at most `MAX_REMOTE_IMAGE` bytes, that sniffs as an allowed image.
async fn fetch_image(client: &reqwest::Client, url: url::Url) -> Option<(AllowedImage, Vec<u8>)> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = read_capped(resp, MAX_REMOTE_IMAGE).await?;
    Some((detect_allowed_image(&bytes)?, bytes))
}

/// The body, unless it is (or says it is) more than `cap` bytes: reading stops there.
async fn read_capped(mut resp: reqwest::Response, cap: usize) -> Option<Vec<u8>> {
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > cap {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

// ── Attachment download ───────────────────────────────────────────────────────

/// The audio and video types an attachment is shown as, compared with the declared type's essence (before
/// `;`, trimmed, lowercased) exactly. The frontend's `PREVIEW_TYPES` (lib/mailUtils.ts) is this list with
/// the images, `application/pdf` and `text/plain`.
const MEDIA_TYPES: &[&str] = &[
    "audio/aac",
    "audio/flac",
    "audio/mp4",
    "audio/mpeg",
    "audio/ogg",
    "audio/wav",
    "audio/webm",
    "audio/x-wav",
    "video/mp4",
    "video/ogg",
    "video/quicktime",
    "video/webm",
];

/// The policy of an image or text attachment shown in the browser: `sandbox` gives the document an opaque
/// origin and no script, wherever it is opened; it loads nothing but itself (an image document).
const SANDBOX_CSP: &str = "sandbox; default-src 'none'; img-src 'self'; media-src 'self'; \
    style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// `SANDBOX_CSP` for audio and video, the document left its origin (`allow-same-origin`; still no script,
/// form or popup): in an opaque origin Chromium's media document cannot load the file it shows (CORS).
const MEDIA_CSP: &str = "sandbox allow-same-origin; default-src 'none'; img-src 'self'; \
    media-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; \
    frame-ancestors 'none'";

/// How an attachment is served. Its Content-Type is never the declared one, which the sender wrote
/// (mail-parser keeps `audio/mpeg,text/html` whole, and browsers read the last type of such a header).
#[derive(Debug, PartialEq, Eq)]
enum Served {
    /// an image or text, shown in the browser as this type in the sandbox (`SANDBOX_CSP`)
    Sandboxed(&'static str),
    /// audio or video, shown as this type (`MEDIA_CSP`)
    Media(&'static str),
    /// a PDF, shown without the sandbox (Chrome's viewer does not run in one): the app's CSP
    Pdf,
    /// downloaded, as `application/octet-stream`
    Download,
}

/// Images by their bytes (PNG, JPEG, GIF, WebP, AVIF; their sniffed type, whatever was declared), a PDF
/// declared so that starts `%PDF-`, UTF-8 plain text declared so, and `MEDIA_TYPES`; anything else is a download.
fn serve_as(declared: Option<&str>, bytes: &[u8]) -> Served {
    if let Some(image) = detect_allowed_image(bytes) {
        return Served::Sandboxed(image.content_type());
    }
    let essence = declared
        .and_then(|t| t.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "application/pdf" if bytes.starts_with(b"%PDF-") => Served::Pdf,
        // text parts arrive decoded to UTF-8 (part_to_bytes)
        "text/plain" if std::str::from_utf8(bytes).is_ok() => {
            Served::Sandboxed("text/plain; charset=utf-8")
        }
        t => MEDIA_TYPES
            .iter()
            .find(|m| **m == t)
            .map_or(Served::Download, |m| Served::Media(m)),
    }
}

fn attachment_disposition(inline: bool, filename: &str) -> String {
    let kind = if inline { "inline" } else { "attachment" };
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

/// An attachment of a cached message. Shown (`inline`) only as a PNG, JPEG, GIF, WebP or AVIF its bytes
/// are, a PDF, UTF-8 plain text or one of a list of audio and video types, all but PDF sandboxed
/// (`Content-Security-Policy: sandbox …`); anything else is `application/octet-stream`, `attachment`.
#[utoipa::path(
    get,
    path = "/email/attachment",
    params(DownloadAttachmentQuery),
    responses(
        (status = 200, description = "the attachment's bytes: as the image type they are, `application/pdf`, `text/plain; charset=utf-8`, an audio or video type, else `application/octet-stream`", body = crate::api::Binary, content_type = "application/octet-stream"),
        (status = 404, description = "no such account, attachment or part (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database or file error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn download_attachment(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<DownloadAttachmentQuery>,
) -> Response {
    let account = match super::owned_account(&state, &user.user_id, &q.account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };

    let att = match email_db::get_attachment_by_filename(
        &state.db,
        account.id(),
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
        match tokio::fs::read(eml_path(&state.data_dir, account.id(), &q.mailbox, q.uid)).await {
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
    let (content_type, csp) = match serve_as(att.content_type.as_deref(), &bytes) {
        Served::Sandboxed(t) => (Some(t), Some(SANDBOX_CSP)),
        Served::Media(t) => (Some(t), Some(MEDIA_CSP)),
        Served::Pdf => (Some("application/pdf"), None),
        Served::Download => (None, None),
    };

    let mut response = (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                content_type.unwrap_or("application/octet-stream"),
            ),
            (
                header::HeaderName::from_static("x-content-type-options"),
                "nosniff",
            ),
        ],
        [(
            header::CONTENT_DISPOSITION,
            attachment_disposition(content_type.is_some(), &safe_name),
        )],
        bytes,
    )
        .into_response();
    if let Some(csp) = csp {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(csp),
        );
    }
    response
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
        (status = 200, description = "a .tar.gz of the account's messages", body = crate::api::Binary, content_type = "application/gzip"),
        (status = 404, description = "no such account (plain text)", body = String, content_type = "text/plain"),
        (status = 500, description = "database error (plain text)", body = String, content_type = "text/plain")
    )
)]
pub async fn export_account(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(q): Query<ExportQuery>,
) -> Response {
    let account = match super::owned_account(&state, &user.user_id, &q.account_id).await {
        Ok(account) => account,
        Err(response) => return response,
    };

    let rows = match sqlx::query_as::<_, (String, i64)>(
        "SELECT mailbox, uid FROM cached_messages WHERE account_id = ? ORDER BY mailbox, uid",
    )
    .bind(account.id().as_str())
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[email] export_account query error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response();
        }
    };

    let files: Vec<(String, i64, std::path::PathBuf)> = rows
        .into_iter()
        .map(|(mailbox, uid)| {
            let path = eml_path(&state.data_dir, account.id(), &mailbox, uid as u32);
            (mailbox, uid, path)
        })
        .collect();
    let display_name = account.display_name.replace(['"', '\\', '\0'], "_");

    let (writer, reader) = tokio::io::duplex(128 * 1024);

    tokio::task::spawn_blocking(move || {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tokio_util::io::SyncIoBridge;

        let sync_writer = SyncIoBridge::new(writer);
        let gz = GzEncoder::new(sync_writer, Compression::default());
        let mut tar = tar::Builder::new(gz);

        for (mailbox, uid, path) in &files {
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

/// Allows the remote images of an origin (and takes it off the ignore list).
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

/// Ignores the remote images of an origin (and takes it off the allow list).
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_REMOTE_IMAGE, Served, attachment_disposition, extract_part_bytes, fetch_image,
        image_client_builder, read_capped, serve_as,
    };
    use crate::raster_image::tests::PNG;
    use axum::{Router, response::Redirect, routing::get};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A server on 127.0.0.1 with `app`; its address.
    async fn serve(app: Router) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    /// The image client's policy (`image_client` is it with `PublicOnly`, which would not reach 127.0.0.1).
    #[tokio::test]
    async fn the_image_client_does_not_follow_redirects() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        // `/image` redirects to `/internal`, which counts its requests
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        let addr = serve(
            Router::new()
                .route("/image", get(|| async { Redirect::temporary("/internal") }))
                .route(
                    "/internal",
                    get(move || async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        "secret"
                    }),
                ),
        )
        .await;

        let client = image_client_builder().build().unwrap();
        let url = url::Url::parse(&format!("http://{addr}/image")).unwrap();
        let response = client.get(url.clone()).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
        assert!(fetch_image(&client, url).await.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    /// The image client itself reaches no name that resolves to this machine.
    #[tokio::test]
    async fn the_image_client_reaches_no_internal_address() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        let addr = serve(Router::new().route(
            "/a.png",
            get(move || async move {
                counted.fetch_add(1, Ordering::SeqCst);
                PNG
            }),
        ))
        .await;
        let client = super::image_client().unwrap();
        let url = url::Url::parse(&format!("http://localhost:{}/a.png", addr.port())).unwrap();
        assert!(fetch_image(client, url).await.is_none());
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    /// Larger than the cap, by Content-Length or streamed without one: nothing; at the cap, the image.
    #[tokio::test]
    async fn remote_images_are_capped() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        fn image(len: usize) -> Vec<u8> {
            let mut body = PNG.to_vec();
            body.resize(len, 0);
            body
        }
        let addr = serve(
            Router::new()
                .route("/big", get(|| async { image(MAX_REMOTE_IMAGE + 1) }))
                .route("/fits", get(|| async { image(MAX_REMOTE_IMAGE) }))
                .route(
                    "/streamed",
                    get(|| async {
                        // chunked, no Content-Length: 11 chunks of 1 MiB
                        let chunks = (0..11).map(|i| {
                            let chunk = if i == 0 {
                                image(1 << 20)
                            } else {
                                vec![0; 1 << 20]
                            };
                            Ok::<_, std::io::Error>(bytes::Bytes::from(chunk))
                        });
                        axum::body::Body::from_stream(futures_util::stream::iter(chunks))
                    }),
                ),
        )
        .await;
        let client = image_client_builder().build().unwrap();
        let get = |path: &str| {
            let url = format!("http://{addr}{path}");
            let client = client.clone();
            async move { client.get(url).send().await.unwrap() }
        };

        let big = get("/big").await;
        assert_eq!(big.content_length(), Some(MAX_REMOTE_IMAGE as u64 + 1));
        assert!(read_capped(big, MAX_REMOTE_IMAGE).await.is_none());
        let streamed = get("/streamed").await;
        assert_eq!(streamed.content_length(), None);
        assert!(read_capped(streamed, MAX_REMOTE_IMAGE).await.is_none());
        let fits = read_capped(get("/fits").await, MAX_REMOTE_IMAGE).await;
        assert_eq!(fits.map(|b| b.len()), Some(MAX_REMOTE_IMAGE));

        let url = |path: &str| url::Url::parse(&format!("http://{addr}{path}")).unwrap();
        assert!(fetch_image(&client, url("/streamed")).await.is_none());
        assert!(fetch_image(&client, url("/fits")).await.is_some());
    }

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
    fn attachments_shown_are_allowlisted_exactly() {
        let pdf = b"%PDF-1.7\n";
        for (declared, bytes, served) in [
            ("application/pdf", &pdf[..], Served::Pdf),
            ("APPLICATION/PDF; version=1.7", pdf, Served::Pdf),
            (
                " text/plain; charset=iso-8859-1",
                b"hello",
                Served::Sandboxed("text/plain; charset=utf-8"),
            ),
            ("audio/mpeg", b"ID3", Served::Media("audio/mpeg")),
            ("Video/MP4", b"\0\0\0", Served::Media("video/mp4")),
            ("audio/x-wav", b"RIFF", Served::Media("audio/x-wav")),
        ] {
            assert_eq!(serve_as(Some(declared), bytes), served, "{declared}");
        }
    }

    #[test]
    fn anything_else_is_a_download() {
        let html = b"<!doctype html><script>alert(1)</script>";
        for declared in [
            // browsers read the last type of a list: HTML
            "audio/mpeg,text/html",
            "video/mp4 , text/html",
            "text/plain,text/html",
            "audio/x\ttext/html",
            "audio/mpeg text/html",
            "application/pdf,text/html",
            "text/html",
            "application/xhtml+xml",
            "image/svg+xml",
            "application/xml",
            "text/xml",
            "application/javascript",
            "text/javascript",
            "application/x-javascript",
            "text/css",
            "application/json",
            "application/octet-stream",
            "audio/",
            "video/",
            "audio/x-unknown",
            "",
            // declared an image, the bytes are HTML
            "image/png",
            "image/jpeg",
        ] {
            assert_eq!(
                serve_as(Some(declared), html),
                Served::Download,
                "{declared:?}"
            );
        }
        assert_eq!(serve_as(None, html), Served::Download);
        // a PDF that is not one
        assert_eq!(serve_as(Some("application/pdf"), html), Served::Download);
        // plain text that is not UTF-8
        assert_eq!(serve_as(Some("text/plain"), b"caf\xe9"), Served::Download);
    }

    #[test]
    fn images_are_served_as_their_bytes_are() {
        for declared in [
            Some("text/html"),
            Some("application/octet-stream"),
            Some("image/gif"),
            Some("audio/mpeg,text/html"),
            None,
        ] {
            assert_eq!(
                serve_as(declared, PNG),
                Served::Sandboxed("image/png"),
                "{declared:?}"
            );
        }
    }

    #[test]
    fn disposition() {
        assert_eq!(
            attachment_disposition(true, "file.pdf"),
            "inline; filename=\"file.pdf\""
        );
        assert_eq!(
            attachment_disposition(false, "file.html"),
            "attachment; filename=\"file.html\""
        );
    }

    /// The handler: what a mail declares is not what is served.
    #[tokio::test]
    async fn download_attachment_serves_by_policy() {
        use crate::email_db::{self, AccountProtocol};
        use crate::jmap::api_tests::{USER, app_state, user};
        use axum::extract::{Query, State};
        use axum::http::header;
        use base64::Engine;

        let state = app_state().await;
        let account = "attachments-test";
        email_db::upsert_account(
            &state.db,
            USER,
            account,
            "Mail",
            "127.0.0.1",
            1,
            "user",
            "pw",
            false,
            None,
            None,
            &AccountProtocol {
                protocol: "imap",
                jmap_url: None,
                auth: "password",
                oauth_client_id: None,
                oauth_client_secret: None,
            },
            &state.encryption_key,
        )
        .await
        .unwrap();
        let png = base64::engine::general_purpose::STANDARD.encode(PNG);
        // part 1 the body, the attachments from 2 on
        let parts: [(&str, &str, &str, &str); 7] = [
            (
                "trick.mp3",
                "audio/mpeg,text/html",
                "7bit",
                "<script>alert(1)</script>",
            ),
            (
                "notes.txt",
                "text/plain; charset=iso-8859-1",
                "quoted-printable",
                "caf=E9",
            ),
            ("doc.pdf", "application/pdf", "7bit", "%PDF-1.4\r\n%%EOF"),
            ("pic.bin", "application/octet-stream", "base64", &png),
            (
                "fake.png",
                "image/png",
                "7bit",
                "<!doctype html><script>alert(1)</script>",
            ),
            ("app.js", "application/javascript", "7bit", "alert(1)"),
            ("song.mp3", "audio/mpeg", "base64", "SUQz"),
        ];
        let mut raw = String::from(
            "From: a@example.com\r\nSubject: s\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nbody\r\n",
        );
        for (name, ct, cte, body) in parts {
            raw.push_str(&format!(
                "--b\r\nContent-Type: {ct}\r\nContent-Transfer-Encoding: {cte}\r\n\
                 Content-Disposition: attachment; filename={name}\r\n\r\n{body}\r\n"
            ));
        }
        raw.push_str("--b--\r\n");
        let path = crate::email_sync::eml_path(
            &state.data_dir,
            email_db::MailAccountId::internal(account),
            "INBOX",
            1,
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, zstd::encode_all(raw.as_bytes(), 3).unwrap()).unwrap();
        for (i, (name, ct, ..)) in parts.iter().enumerate() {
            // stored as extract_attachments stores it: type/subtype, the rest of the subtype kept
            let stored = ct.split(';').next().unwrap();
            email_db::insert_attachment(
                &state.db,
                account,
                "INBOX",
                1,
                name,
                stored,
                1,
                i as i64 + 2,
                None,
            )
            .await
            .unwrap();
        }

        let sandbox = Some(super::SANDBOX_CSP);
        for (name, content_type, disposition, csp, body) in [
            (
                "trick.mp3",
                "application/octet-stream",
                "attachment",
                None,
                None,
            ),
            (
                "notes.txt",
                "text/plain; charset=utf-8",
                "inline",
                sandbox,
                Some("café".as_bytes()),
            ),
            ("doc.pdf", "application/pdf", "inline", None, None),
            ("pic.bin", "image/png", "inline", sandbox, Some(PNG)),
            (
                "fake.png",
                "application/octet-stream",
                "attachment",
                None,
                None,
            ),
            (
                "app.js",
                "application/octet-stream",
                "attachment",
                None,
                None,
            ),
            (
                "song.mp3",
                "audio/mpeg",
                "inline",
                Some(super::MEDIA_CSP),
                Some(&b"ID3"[..]),
            ),
        ] {
            let response = super::download_attachment(
                State(state.clone()),
                user(),
                Query(super::DownloadAttachmentQuery {
                    account_id: account.into(),
                    mailbox: "INBOX".into(),
                    uid: 1,
                    filename: name.into(),
                }),
            )
            .await;
            assert_eq!(response.status(), 200, "{name}");
            let h = response.headers();
            assert_eq!(h[header::CONTENT_TYPE], content_type, "{name}");
            assert_eq!(
                h[header::CONTENT_DISPOSITION],
                format!("{disposition}; filename=\"{name}\""),
                "{name}"
            );
            assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{name}");
            assert_eq!(
                h.get(header::CONTENT_SECURITY_POLICY)
                    .map(|v| v.to_str().unwrap()),
                csp,
                "{name}"
            );
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            if let Some(body) = body {
                assert_eq!(&bytes[..], body, "{name}");
            }
        }
    }
}

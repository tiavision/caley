//! The frontend build, served with cache headers that let a browser keep what cannot change and
//! revalidate everything else, so a deploy reaches the next page load at once.
//!
//! - Hashed files under `/assets/` (`name-<8 char hash>.ext`, Vite's and sinter's file names): a new
//!   build gives them new names, so they are cached for a year and never revalidated.
//! - Everything else (HTML pages, `favicon.ico`, `site.webmanifest`, `icons/…`): `no-cache`, i.e.
//!   kept but revalidated on every use (ETag / Last-Modified, answered with 304 when unchanged), so
//!   a new page arrives with the first request after a deploy and points at the new asset names.

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::services::ServeDir;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

/// The static file service for `dir`, with cache headers.
pub fn service(dir: &str) -> Router {
    Router::new()
        .fallback_service(ServeDir::new(dir))
        .layer(middleware::from_fn(cache_headers))
}

async fn cache_headers(req: Request, next: Next) -> Response {
    let hashed = is_hashed_asset(req.uri().path());
    let mut resp = next.run(req).await;
    let status = resp.status();
    // a missing file is not cached as such: the next request asks again
    let value = if hashed && (status.is_success() || status == axum::http::StatusCode::NOT_MODIFIED)
    {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    resp
}

/// Whether `path` is a content-hashed build file: directly under `/assets/`, named
/// `<name>-<hash>.<ext>` with an 8 character hash of `[A-Za-z0-9_-]` (the hash itself may hold `-`).
fn is_hashed_asset(path: &str) -> bool {
    let Some(file) = path.strip_prefix("/assets/") else {
        return false;
    };
    if file.contains('/') {
        return false;
    }
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let bytes = stem.as_bytes();
    if bytes.len() < 10 || bytes[bytes.len() - 9] != b'-' {
        return false;
    }
    bytes[bytes.len() - 8..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    #[test]
    fn hashed_assets() {
        for p in [
            "/assets/calendar-BmPLGx7H.js",
            "/assets/temper-DPEFizoY.css",
            "/assets/sinter-i18n-l-DgYWuJ.js",
            "/assets/calendar-early-AY05xxFr.js",
            "/assets/calendar-de-PdLpg0yl.js",
            "/assets/mail-view-early-n11KQjSK.js",
            "/assets/sinter.helpers-CcXc4zlG.js",
            "/assets/inter-latin-400-normal-C38fXH4l.woff2",
        ] {
            assert!(is_hashed_asset(p), "{p}");
        }
        for p in [
            "/favicon.ico",
            "/site.webmanifest",
            "/icons/favicon-32x32.png",
            "/caley-logo.png",
            "/calendar/",
            "/calendar/index.html",
            "/assets/logo.png",
            "/assets/a-short.js",
            "/assets/sub/x-BmPLGx7H.js",
            "/assets/x-BmPLGx7H.",
            "/assets/x-BmPL.x7H.js",
        ] {
            assert!(!is_hashed_asset(p), "{p}");
        }
    }

    #[tokio::test]
    async fn headers_by_kind() {
        let dir = std::env::temp_dir().join(format!("caley-static-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::create_dir_all(dir.join("calendar")).unwrap();
        std::fs::write(dir.join("assets/app-BmPLGx7H.js"), "x").unwrap();
        std::fs::write(dir.join("calendar/index.html"), "<p>").unwrap();
        std::fs::write(dir.join("favicon.ico"), "i").unwrap();
        let app = service(dir.to_str().unwrap());
        let get = |p: &str| {
            let app = app.clone();
            let p = p.to_string();
            async move {
                let r = app
                    .oneshot(Request::get(p).body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                (
                    r.status().as_u16(),
                    r.headers()
                        .get(header::CACHE_CONTROL)
                        .map(|v| v.to_str().unwrap().to_string()),
                )
            }
        };
        assert_eq!(
            get("/assets/app-BmPLGx7H.js").await,
            (200, Some(IMMUTABLE.into()))
        );
        assert_eq!(get("/calendar/").await, (200, Some(REVALIDATE.into())));
        assert_eq!(get("/favicon.ico").await, (200, Some(REVALIDATE.into())));
        // a hashed name that is not there (an old chunk after a deploy) is not cached
        assert_eq!(
            get("/assets/app-OLDHASH1.js").await,
            (404, Some(REVALIDATE.into()))
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

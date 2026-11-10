//! Validators for the API's JSON: every successful JSON GET gets a weak `ETag` (a hash of its body)
//! and `Cache-Control: private, no-cache`, and a request whose `If-None-Match` names it is answered
//! `304 Not Modified` without the body.
//!
//! `no-cache` makes the browser revalidate a reload (an `<sn-sync>` refetch, `refresh()`) with
//! `If-None-Match`; sinter sees the same `ETag` as on the data it has and neither parses nor
//! re-renders it. The handler still runs (the hash is of its output), so this saves transfer and
//! browser work, not server work.

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};

/// Bodies above this are passed on as they are (attachments are not JSON, but be safe).
const MAX_BYTES: usize = 8 << 20;

pub async fn etag(req: Request, next: Next) -> Response {
    if req.method() != Method::GET {
        return next.run(req).await;
    }
    let if_none_match = req.headers().get(header::IF_NONE_MATCH).cloned();
    let res = next.run(req).await;
    if res.status() != StatusCode::OK
        || !is_json(res.headers())
        || res.headers().contains_key(header::ETAG)
        || res.headers().contains_key(header::CACHE_CONTROL)
    {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let bytes: Bytes = match http_body_util::Limited::new(body, MAX_BYTES)
        .collect()
        .await
    {
        Ok(b) => b.to_bytes(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let tag = tag_of(&bytes);
    parts.headers.insert(header::ETAG, tag.clone());
    parts.headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if if_none_match.is_some_and(|v| matches(&v, &tag)) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, Body::from(bytes))
}

fn is_json(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| {
            t.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
}

/// `W/"<first 16 bytes of the body's SHA-256, hex>"`: weak, since compression changes the bytes sent.
fn tag_of(body: &[u8]) -> HeaderValue {
    let hash = Sha256::digest(body);
    let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    HeaderValue::from_str(&format!("W/\"{hex}\"")).expect("hex is a valid header value")
}

/// `If-None-Match` against our tag, by weak comparison (the `W/` prefix does not count); `*` matches.
fn matches(if_none_match: &HeaderValue, tag: &HeaderValue) -> bool {
    let Ok(list) = if_none_match.to_str() else {
        return false;
    };
    let ours = tag.to_str().unwrap_or("").trim_start_matches("W/");
    list.split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.trim_start_matches("W/") == ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, middleware, routing::get};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route(
                "/json",
                get(|| async { Json(serde_json::json!({ "a": 1 })) }),
            )
            .route("/text", get(|| async { "plain" }))
            .layer(middleware::from_fn(etag))
    }

    async fn send(path: &str, inm: Option<&str>) -> Response {
        let mut req = Request::get(path).body(Body::empty()).unwrap();
        if let Some(v) = inm {
            req.headers_mut()
                .insert(header::IF_NONE_MATCH, v.parse().unwrap());
        }
        app().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn json_gets_a_tag_and_304() {
        let r = send("/json", None).await;
        assert_eq!(r.status(), StatusCode::OK);
        let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
        assert!(tag.starts_with("W/\""));
        assert_eq!(r.headers()[header::CACHE_CONTROL], "private, no-cache");
        let body = r.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], br#"{"a":1}"#);

        let r = send("/json", Some(&tag)).await;
        assert_eq!(r.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(r.headers()[header::ETAG].to_str().unwrap(), tag);
        assert!(r.into_body().collect().await.unwrap().to_bytes().is_empty());

        // the strong form of the same tag (a proxy may drop W/) and a list
        let strong = tag.trim_start_matches("W/").to_string();
        assert_eq!(
            send("/json", Some(&format!("\"x\", {strong}")))
                .await
                .status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            send("/json", Some("W/\"other\"")).await.status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn other_responses_untouched() {
        let r = send("/text", None).await;
        assert!(!r.headers().contains_key(header::ETAG));
        assert!(!r.headers().contains_key(header::CACHE_CONTROL));
    }
}

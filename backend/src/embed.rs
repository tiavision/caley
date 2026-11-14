//! The pages' API data in the HTML (sinter-axum): the frontend is built with `sinter({ embed: true })`,
//! which leaves an empty `<script type="application/json" data-sn="…">` in each page's head for every
//! resource whose request the page's URL decides, listed in `dist/sinter-embed.json`. The layer answers
//! those pages: it calls the API router in-process with the page request's cookie (so the API sees the
//! same session) and writes each response into its placeholder while the head is already on its way.
//!
//! The layer sits inside `RequireAuthLayer`: a signed-out request for an app page is redirected to the
//! login page before anything is fetched. The pages are read when the server starts, so a new frontend
//! build needs a restart. Without a `sinter-embed.json` (a build without `embed: true`, or none yet) the
//! pages are served as files and the browser requests their data, as before.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use bytes::Bytes;
use http_body_util::BodyExt;
use sinter_axum::{Embed, EmbedLayer, SlotRequest};
use std::path::Path;
use tower::ServiceExt;

/// The largest API response put into a page.
const MAX_BYTES: usize = 2 << 20;

/// The layer for the build in `static_dir`, its data from `api` (the API routes, with the session layer).
/// `None` when the build has no embedded-data placeholders.
pub fn layer(static_dir: &str, api: Router) -> anyhow::Result<Option<EmbedLayer>> {
    if !Path::new(static_dir).join("sinter-embed.json").exists() {
        eprintln!(
            "[embed] {static_dir}/sinter-embed.json not found: pages are served without their data"
        );
        return Ok(None);
    }
    // Each page is built once per language (`localePrefix`: `de/index.html`), so the resolvers are
    // registered for each language's file; a build without languages has the files at the root.
    let dirs: Vec<String> = match locales(static_dir) {
        Some(list) => list.into_iter().map(|l| format!("{l}/")).collect(),
        None => vec![String::new()],
    };
    let mut embed = Embed::from_dir(static_dir)
        .api(api.clone())
        // Cookie for the session; Accept-Language as the browser would send it (no Authorization: Caley
        // has no token auth).
        .forward([header::COOKIE, header::ACCEPT_LANGUAGE])
        .max_bytes(MAX_BYTES);
    for dir in &dirs {
        let user_api = api.clone();
        let search_api = api.clone();
        embed = embed
            // The login page asks who is signed in: 401 is "nobody" (src/sinter.config.ts makes it `null` in
            // the browser too), not a failed page.
            .resolve(format!("{dir}index.html:user"), move |req| {
                let api = user_api.clone();
                async move {
                    match get(&api, "/api/user", &req.headers).await? {
                        (StatusCode::OK, body) => Some(body),
                        (StatusCode::UNAUTHORIZED, _) => Some(Bytes::from_static(b"null")),
                        _ => None,
                    }
                }
            })
            // The search in the URL: its `src` reads `urlSearch($url)` (frontend src/lib/events.ts), which
            // the build cannot describe, so the same request is put together here.
            .resolve(format!("{dir}search/index.html:results"), move |req| {
                let api = search_api.clone();
                async move {
                    let url = search_url(&req)?;
                    match get(&api, &url, &req.headers).await? {
                        (StatusCode::OK, body) => Some(body),
                        _ => None,
                    }
                }
            });
    }
    Ok(Some(embed.build()?))
}

/// The languages of a `localePrefix` build (`sinter-embed.json`'s `locales.list`), `None` without.
fn locales(static_dir: &str) -> Option<Vec<String>> {
    let manifest = std::fs::read(Path::new(static_dir).join("sinter-embed.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    serde_json::from_value(manifest.get("locales")?.get("list")?.clone()).ok()
}

/// `/api/search?…` for the page URL's search, as `urlSearch()` builds it; None without words.
fn search_url(req: &SlotRequest) -> Option<String> {
    let p = &req.params;
    let q = p.get("q").map(|s| s.trim()).unwrap_or("");
    if q.is_empty() {
        return None;
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.append_pair("q", q);
    for key in ["start", "end"] {
        if let Some(v) = p.get(key).filter(|v| !v.is_empty()) {
            query.append_pair(key, v);
        }
    }
    // `cals`: absent = the calendars shown in the calendar (`active=1`), `all` = every calendar, else
    // the chosen calendars' keys, one per line.
    match p.get("cals").filter(|v| !v.is_empty()).map(String::as_str) {
        None => {
            query.append_pair("active", "1");
        }
        Some("all") => {}
        Some(cals) => {
            for cal in cals.split('\n') {
                query.append_pair("cal", cal);
            }
        }
    }
    Some(format!("/api/search?{}", query.finish()))
}

/// GET `url` from the API router with the page request's cookie: the status and a JSON body.
async fn get(api: &Router, url: &str, page: &HeaderMap) -> Option<(StatusCode, Bytes)> {
    let mut req = Request::get(url).body(Body::empty()).ok()?;
    for name in [header::COOKIE, header::ACCEPT_LANGUAGE] {
        for v in page.get_all(&name) {
            req.headers_mut().append(name.clone(), v.clone());
        }
    }
    req.headers_mut()
        .insert(header::ACCEPT, "application/json".parse().ok()?);
    let res = api.clone().oneshot(req).await.ok()?;
    let status = res.status();
    let body = http_body_util::Limited::new(res.into_body(), MAX_BYTES)
        .collect()
        .await
        .ok()?
        .to_bytes();
    Some((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn slot(params: &[(&str, &str)]) -> SlotRequest {
        SlotRequest {
            page: "search/index.html".into(),
            resource: "results".into(),
            url: None,
            params: Arc::new(
                params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect::<HashMap<_, _>>(),
            ),
            headers: Arc::new(HeaderMap::new()),
        }
    }

    #[test]
    fn search_requests() {
        assert_eq!(search_url(&slot(&[])), None);
        assert_eq!(search_url(&slot(&[("q", "  ")])), None);
        assert_eq!(
            search_url(&slot(&[("q", " dentist ")])).as_deref(),
            Some("/api/search?q=dentist&active=1")
        );
        assert_eq!(
            search_url(&slot(&[
                ("q", "a b"),
                ("start", "2026-01-01"),
                ("end", ""),
                ("cals", "all")
            ]))
            .as_deref(),
            Some("/api/search?q=a+b&start=2026-01-01")
        );
        assert_eq!(
            search_url(&slot(&[("q", "x"), ("cals", "acc|/c1/\nacc|/c2/")])).as_deref(),
            Some("/api/search?q=x&cal=acc%7C%2Fc1%2F&cal=acc%7C%2Fc2%2F")
        );
    }
}

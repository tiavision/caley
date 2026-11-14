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
use sinter_axum::{Embed, EmbedLayer};
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
            });
        // The message open in the mail page's URL: <mail-reader-data> loads its thread through a helper
        // (urlRef()), which the build cannot describe, so those two are the browser's to load.
        for resource in ["thread", "threadEvents"] {
            embed = embed.skip(format!("{dir}mail/index.html:{resource}"));
        }
    }
    Ok(Some(embed.build()?))
}

/// The languages of a `localePrefix` build (`sinter-embed.json`'s `locales.list`), `None` without.
fn locales(static_dir: &str) -> Option<Vec<String>> {
    let manifest = std::fs::read(Path::new(static_dir).join("sinter-embed.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).ok()?;
    serde_json::from_value(manifest.get("locales")?.get("list")?.clone()).ok()
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

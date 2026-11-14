//! Where `/` sends someone who is signed in: decided by the server, so the login page is never painted
//! on the way (a redirect has no body).
//!
//! - `next`: the page a signed-out visitor wanted (`/?next=/mail/?uid=3`, or the page request that
//!   `RequireAuthLayer` sent to the IdP). Kept in the session until the sign-in, used once.
//! - `last_page`: the app page the signed-in user loaded last (its path, without the query). Recorded
//!   by `RequireAuthLayer` for every page load except prefetches.
//! - otherwise the calendar.
//!
//! The pages are built once per language under its prefix (`/de/mail/`, sinter's `localePrefix`), so every
//! path here may start with one; an unprefixed one is sent on to the visitor's language by the embed layer
//! (sinter-axum), and the calendar fallback is unprefixed for that.

use axum::http::{HeaderMap, Method};
use tower_sessions::Session;

const NEXT_KEY: &str = "login_next";
const LAST_PAGE_KEY: &str = "last_page";
const FALLBACK: &str = "/calendar/";

/// The app's pages (each answers at its directory URL).
const PAGES: &[&str] = &[
    "/calendar/",
    "/mail/",
    "/contacts/",
    "/search/",
    "/settings/",
];

/// `path` without its language prefix (`/de/mail/` is `/mail/`, `/de/` is `/`): a first segment of two
/// lowercase letters, optionally with a region (`pt-BR`), followed by a `/`.
pub fn without_locale(path: &str) -> &str {
    let Some(rest) = path.strip_prefix('/') else {
        return path;
    };
    let Some(end) = rest.find('/') else {
        return path;
    };
    let (lang, region) = rest[..end].split_once('-').unwrap_or((&rest[..end], ""));
    let is_lang = lang.len() == 2 && lang.bytes().all(|b| b.is_ascii_lowercase());
    let is_region = region.is_empty() || region.bytes().all(|b| b.is_ascii_alphabetic());
    if is_lang && is_region {
        &rest[end..]
    } else {
        path
    }
}

/// `path`'s language prefix (`/de` of `/de/mail/`), or ''.
pub fn locale_prefix(path: &str) -> &str {
    &path[..path.len() - without_locale(path).len()]
}

/// The login page: `/`, or its copy in a language (`/de/`).
pub fn is_login_page(path: &str) -> bool {
    without_locale(path) == "/"
}

/// A URL on this site under one of the app's pages, safe to redirect to (no other origin, no `//host`).
pub fn valid_next(next: &str) -> bool {
    let next_path = without_locale(next);
    !next.contains(['\\', '\r', '\n'])
        && PAGES.iter().any(|p| next_path.starts_with(p))
        && !next.split(['?', '#']).next().unwrap_or("").contains("/.")
}

/// The page to remember as the last one: an app page itself, not the standalone message view (a
/// window of its own, without the rail).
fn is_landing_page(path: &str) -> bool {
    PAGES.contains(&without_locale(path))
}

/// A real page load (not a speculation-rules or link prefetch, which only maybe becomes one).
fn is_page_load(method: &Method, headers: &HeaderMap) -> bool {
    method == Method::GET
        && !headers
            .get("sec-purpose")
            .or_else(|| headers.get("purpose"))
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("prefetch"))
}

/// Signed in: remember `path` if it is an app page being loaded.
pub async fn record(session: &Session, method: &Method, headers: &HeaderMap, path: &str) {
    if is_landing_page(path)
        && is_page_load(method, headers)
        && session
            .get::<String>(LAST_PAGE_KEY)
            .await
            .ok()
            .flatten()
            .as_deref()
            != Some(path)
    {
        session.insert(LAST_PAGE_KEY, path).await.ok();
    }
}

/// Signed out: keep the page wanted until the sign-in (ignored unless it is one of the app's).
pub async fn remember_next(session: &Session, next: &str) {
    if valid_next(next) {
        session.insert(NEXT_KEY, next).await.ok();
    }
}

/// Signed in: where to go now (the `next` kept is used up). `lang` is the prefix of the page asking
/// (`/de` for the login page at `/de/`, else ''), for the calendar fallback.
pub async fn take(session: &Session, lang: &str) -> String {
    if let Some(next) = session
        .remove::<String>(NEXT_KEY)
        .await
        .ok()
        .flatten()
        .filter(|n| valid_next(n))
    {
        return next;
    }
    session
        .get::<String>(LAST_PAGE_KEY)
        .await
        .ok()
        .flatten()
        .filter(|p| is_landing_page(p))
        .unwrap_or_else(|| format!("{lang}{FALLBACK}"))
}

/// `next` from a query string (`/?next=…`).
pub fn next_param(query: Option<&str>) -> Option<String> {
    url::form_urlencoded::parse(query?.as_bytes())
        .find(|(k, _)| k == "next")
        .map(|(_, v)| v.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_must_be_an_app_page() {
        assert!(valid_next("/mail/?account_id=a&uid=3"));
        assert!(valid_next("/mail/view/?uid=3"));
        assert!(valid_next("/calendar/#today"));
        assert!(!valid_next("/"));
        assert!(!valid_next("//evil.example/mail/"));
        assert!(!valid_next("https://evil.example/mail/"));
        assert!(!valid_next("/mail/../logout"));
        assert!(!valid_next("/mail/\\evil"));
        assert!(!valid_next("/assets/x.js"));
        assert!(valid_next("/de/mail/?uid=3"));
        assert!(!valid_next("/de/"));
        assert!(!valid_next("/de//evil.example/mail/"));
    }

    #[test]
    fn language_prefixes() {
        assert_eq!(without_locale("/de/mail/"), "/mail/");
        assert_eq!(without_locale("/pt-BR/mail/"), "/mail/");
        assert_eq!(without_locale("/de/"), "/");
        assert_eq!(without_locale("/mail/"), "/mail/");
        assert_eq!(without_locale("/de"), "/de");
        assert_eq!(without_locale("/api/user"), "/api/user");
        assert!(is_login_page("/") && is_login_page("/en/") && !is_login_page("/mail/"));
    }

    #[test]
    fn only_pages_are_remembered() {
        assert!(is_landing_page("/contacts/"));
        assert!(is_landing_page("/de/contacts/"));
        assert!(!is_landing_page("/de/mail/view/"));
        assert!(!is_landing_page("/mail/view/"));
        assert!(!is_landing_page("/api/user"));
    }

    #[test]
    fn prefetches_are_not_page_loads() {
        let mut h = HeaderMap::new();
        assert!(is_page_load(&Method::GET, &h));
        h.insert("sec-purpose", "prefetch;prerender".parse().unwrap());
        assert!(!is_page_load(&Method::GET, &h));
        assert!(!is_page_load(&Method::HEAD, &HeaderMap::new()));
    }

    #[test]
    fn next_from_query() {
        assert_eq!(
            next_param(Some("next=%2Fmail%2F%3Fuid%3D3")).as_deref(),
            Some("/mail/?uid=3")
        );
        assert_eq!(next_param(Some("a=1")), None);
        assert_eq!(next_param(None), None);
    }
}

//! The CalDAV/CardDAV account's credentials go only to its own server. The server is the account URL's
//! origin; its `.well-known` discovery may move it once, over https, to another origin (or upgrade it to
//! https on the same host). Every request, every redirect and every href from a response is checked
//! against those origins: a redirect elsewhere is an error before anything is sent there, an href
//! elsewhere is skipped.

use anyhow::bail;
use url::Url;

/// Whether `to` is `from` reached over https: the same host, `from` http and `to` https, and both on
/// their scheme's default port or both on the same explicit port (`http://h` → `https://h`,
/// `http://h:8080` → `https://h:8080`; not `http://h:8080` → `https://h`).
pub(crate) fn is_https_upgrade(from: &Url, to: &Url) -> bool {
    from.scheme() == "http"
        && to.scheme() == "https"
        && from.host().is_some()
        && from.host() == to.host()
        && from.port() == to.port()
}

/// Whether `url` is on `server`'s origin, or on its https upgrade (`is_https_upgrade`).
pub(crate) fn same_server(server: &Url, url: &Url) -> bool {
    url.origin() == server.origin() || is_https_upgrade(server, url)
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("?").to_owned()
}

/// Where a redirect from `current` to `location` (absolute or relative) leads, when that is on one of
/// the `allowed` servers (`same_server`) and does not leave https; else an error naming the host.
pub(crate) fn redirect_target(
    current: &Url,
    location: &str,
    allowed: &[Url],
    what: &str,
) -> anyhow::Result<Url> {
    let Ok(next) = current.join(location) else {
        bail!("{what} redirected to an invalid location");
    };
    if current.scheme() == "https" && next.scheme() != "https" {
        bail!(
            "{what} redirected from https to {} on {}: not followed",
            next.scheme(),
            host_of(&next)
        );
    }
    if !allowed.iter().any(|server| same_server(server, &next)) {
        bail!(
            "{what} redirected to {}, which is not the account's server: not followed",
            host_of(&next)
        );
    }
    Ok(next)
}

/// The server a `.well-known` redirect from `well_known` to `location` makes the account's: the same
/// origin, its https upgrade, or one other origin over https (the one cross-origin hop discovery may
/// make); `None` for anything else (discovery then starts at the account URL).
pub(crate) fn well_known_target(well_known: &Url, location: &str) -> Option<Url> {
    let next = well_known.join(location).ok()?;
    if same_server(well_known, &next) {
        if well_known.scheme() == "https" && next.scheme() != "https" {
            return None;
        }
        return Some(next);
    }
    if next.scheme() == "https" && next.host().is_some() {
        return Some(next);
    }
    eprintln!(
        "caldav: .well-known redirected to {}://{} without https: ignored",
        next.scheme(),
        host_of(&next)
    );
    None
}

/// An href from a response to a request to `base`, resolved (`resolve_url`) when it is on `base`'s
/// server (`same_server`); `None`, logged, for one elsewhere, which is never requested.
pub(crate) fn resolve_href(base: &str, href: &str) -> Option<String> {
    let resolved = super::parsing::resolve_url(base, href);
    match (Url::parse(base), Url::parse(&resolved)) {
        (Ok(server), Ok(url)) if same_server(&server, &url) => Some(resolved),
        (Ok(_), Ok(url)) => {
            eprintln!(
                "caldav: skipping href on {}, which is not the account's server",
                host_of(&url)
            );
            None
        }
        // a base that is no URL (the demo's paths): the href as before, nothing is requested
        (Err(_), _) => Some(resolved),
        (Ok(_), Err(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn https_upgrade_is_the_same_host_and_port() {
        assert!(is_https_upgrade(
            &u("http://dav.example"),
            &u("https://dav.example/x")
        ));
        assert!(is_https_upgrade(
            &u("http://dav.example:80/"),
            &u("https://dav.example:443/")
        ));
        assert!(is_https_upgrade(
            &u("http://dav.example:8080"),
            &u("https://dav.example:8080")
        ));
        assert!(!is_https_upgrade(
            &u("http://dav.example:8080"),
            &u("https://dav.example")
        ));
        assert!(!is_https_upgrade(
            &u("http://dav.example"),
            &u("https://dav.example:8443")
        ));
        assert!(!is_https_upgrade(
            &u("http://dav.example"),
            &u("https://other.example")
        ));
        assert!(!is_https_upgrade(
            &u("https://dav.example"),
            &u("http://dav.example")
        ));
        assert!(!is_https_upgrade(
            &u("http://dav.example"),
            &u("http://dav.example")
        ));
    }

    #[test]
    fn redirects_stay_on_the_server() {
        let current = u("https://dav.example/cal/");
        let allowed = [u("https://dav.example/")];
        // relative and absolute on the same origin
        assert_eq!(
            redirect_target(&current, "/dav/cal/", &allowed, "PROPFIND")
                .unwrap()
                .as_str(),
            "https://dav.example/dav/cal/"
        );
        assert_eq!(
            redirect_target(&current, "sub/", &allowed, "PROPFIND")
                .unwrap()
                .as_str(),
            "https://dav.example/cal/sub/"
        );
        // another host, another port
        let e = redirect_target(&current, "https://evil.example/x", &allowed, "PUT").unwrap_err();
        assert!(e.to_string().contains("evil.example"), "{e}");
        assert!(redirect_target(&current, "https://dav.example:8443/x", &allowed, "PUT").is_err());
        // never from https to http, not even on the same host
        let e = redirect_target(&current, "http://dav.example/x", &allowed, "PUT").unwrap_err();
        assert!(e.to_string().contains("https"), "{e}");
        // http to https on the same host is followed
        let current = u("http://dav.example/cal/");
        let allowed = [u("http://dav.example/")];
        assert_eq!(
            redirect_target(&current, "https://dav.example/cal/", &allowed, "GET")
                .unwrap()
                .as_str(),
            "https://dav.example/cal/"
        );
        assert!(
            redirect_target(&current, "https://dav.example:8443/cal/", &allowed, "GET").is_err()
        );
    }

    #[test]
    fn well_known_may_move_once_over_https() {
        let wk = u("https://example.com/.well-known/caldav");
        assert_eq!(
            well_known_target(&wk, "/dav/").unwrap().as_str(),
            "https://example.com/dav/"
        );
        assert_eq!(
            well_known_target(&wk, "https://dav.provider.example/dav/")
                .unwrap()
                .as_str(),
            "https://dav.provider.example/dav/"
        );
        assert!(well_known_target(&wk, "http://dav.provider.example/dav/").is_none());
        assert!(well_known_target(&wk, "http://example.com/dav/").is_none());
        let wk = u("http://example.com/.well-known/caldav");
        assert_eq!(
            well_known_target(&wk, "https://example.com/dav/")
                .unwrap()
                .as_str(),
            "https://example.com/dav/"
        );
        assert!(well_known_target(&wk, "http://other.example/dav/").is_none());
    }

    #[test]
    fn hrefs_elsewhere_are_skipped() {
        let base = "https://dav.example/cal/";
        assert_eq!(
            resolve_href(base, "/cal/a.ics").as_deref(),
            Some("https://dav.example/cal/a.ics")
        );
        assert_eq!(
            resolve_href(base, "https://dav.example/cal/b.ics").as_deref(),
            Some("https://dav.example/cal/b.ics")
        );
        assert_eq!(resolve_href(base, "https://evil.example/cal/a.ics"), None);
        assert_eq!(resolve_href(base, "http://dav.example/cal/a.ics"), None);
        assert_eq!(
            resolve_href(base, "https://dav.example:444/cal/a.ics"),
            None
        );
    }
}

//! The origin (`scheme://host[:port]`) a request was made to, for the URLs the app hands to the
//! browser and the IdP (the OIDC redirect and post-logout URLs). One instance can be reached both
//! through a reverse proxy (`https://calendar.example.com`) and directly (`http://10.0.0.5:3000`),
//! and each sign-in comes back to the URL it started from.
//!
//! - From a peer in `TRUSTED_PROXIES`: `Forwarded` (`proto=`, `host=`), else `X-Forwarded-Proto` /
//!   `X-Forwarded-Host`, else the `Host` header with `http`.
//! - From any other peer: `http://` + `Host` (the app does not terminate TLS, so a direct
//!   connection is plain HTTP); forwarding headers are ignored.
//! - Without a usable host (or an invalid one): `APP_BASE_URL`.
//!
//! A forged `Host` only changes the URLs in the forger's own responses, and the IdP accepts only
//! the redirect URIs registered for the client.

use crate::oidc::AppState;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::{HeaderMap, request::Parts};
use ipnet::IpNet;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

pub struct OriginConfig {
    /// `APP_BASE_URL` without a trailing slash.
    fallback: String,
    trusted_proxies: Vec<IpNet>,
}

impl OriginConfig {
    pub fn new(app_base_url: &str, trusted_proxies: &[String]) -> anyhow::Result<Self> {
        let trusted_proxies = trusted_proxies
            .iter()
            .flat_map(|s| s.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<IpNet>()
                    .or_else(|_| s.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| anyhow::anyhow!("Invalid TRUSTED_PROXIES entry: {s}"))
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self {
            fallback: app_base_url.trim_end_matches('/').to_owned(),
            trusted_proxies,
        })
    }

    fn is_trusted(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| {
            let ip = ip.to_canonical();
            self.trusted_proxies.iter().any(|net| net.contains(&ip))
        })
    }

    /// The origin of a request with these headers, URI authority (HTTP/2 has no `Host`) and peer.
    pub fn resolve(
        &self,
        headers: &HeaderMap,
        authority: Option<&str>,
        peer: Option<IpAddr>,
    ) -> String {
        let host_header = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .or(authority);
        let (proto, host) = if self.is_trusted(peer) {
            let fwd = forwarded(headers);
            let proto = fwd
                .as_ref()
                .and_then(|f| f.0.clone())
                .or_else(|| first(headers, "x-forwarded-proto"));
            let host = fwd
                .and_then(|f| f.1)
                .or_else(|| first(headers, "x-forwarded-host"))
                .or_else(|| host_header.map(str::to_owned));
            (proto.unwrap_or_else(|| "http".to_owned()), host)
        } else {
            ("http".to_owned(), host_header.map(str::to_owned))
        };
        host.and_then(|host| origin(&proto, &host))
            .unwrap_or_else(|| self.fallback.clone())
    }
}

/// The first value of a comma-separated header (the client-facing hop).
fn first(headers: &HeaderMap, name: &str) -> Option<String> {
    let v = headers.get(name)?.to_str().ok()?.split(',').next()?.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

/// `proto` and `host` of the first element of an RFC 7239 `Forwarded` header.
fn forwarded(headers: &HeaderMap) -> Option<(Option<String>, Option<String>)> {
    let element = headers.get("forwarded")?.to_str().ok()?.split(',').next()?;
    let (mut proto, mut host) = (None, None);
    for pair in element.split(';') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim().to_ascii_lowercase().as_str() {
            "proto" => proto = Some(value),
            "host" => host = Some(value),
            _ => {}
        }
    }
    Some((proto, host))
}

/// `scheme://host[:port]` when `proto` is http(s) and `host` is a bare host with an optional port.
fn origin(proto: &str, host: &str) -> Option<String> {
    let proto = proto.to_ascii_lowercase();
    if proto != "http" && proto != "https" {
        return None;
    }
    if host.contains(['/', '\\', '@', '?', '#']) || host.chars().any(char::is_whitespace) {
        return None;
    }
    let url = url::Url::parse(&format!("{proto}://{host}")).ok()?;
    url.host_str()?;
    Some(url.origin().ascii_serialization())
}

/// Extractor: the origin of the current request (see the module docs).
pub struct RequestOrigin(pub String);

impl FromRequestParts<AppState> for RequestOrigin {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Infallible> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let authority = parts.uri.authority().map(|a| a.as_str());
        Ok(Self(state.origin.resolve(&parts.headers, authority, peer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OriginConfig {
        OriginConfig::new(
            "https://calendar.example.com/",
            &["10.0.0.1, 172.16.0.0/12".to_owned()],
        )
        .unwrap()
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    const PROXY: Option<IpAddr> = Some(IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1)));
    const CLIENT: Option<IpAddr> = Some(IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 9)));

    #[test]
    fn direct_request_uses_host_over_http() {
        let h = headers(&[("host", "10.0.0.5:3000"), ("x-forwarded-proto", "https")]);
        assert_eq!(cfg().resolve(&h, None, CLIENT), "http://10.0.0.5:3000");
    }

    #[test]
    fn trusted_proxy_uses_forwarded_headers() {
        let h = headers(&[
            ("host", "caley:3000"),
            ("x-forwarded-proto", "https, http"),
            ("x-forwarded-host", "calendar.example.com"),
        ]);
        assert_eq!(
            cfg().resolve(&h, None, PROXY),
            "https://calendar.example.com"
        );
        let h = headers(&[("host", "caley:3000"), ("x-forwarded-proto", "https")]);
        assert_eq!(cfg().resolve(&h, None, PROXY), "https://caley:3000");
        let peer = Some("172.20.0.3".parse().unwrap());
        let h = headers(&[(
            "forwarded",
            "for=1.2.3.4;proto=https;host=\"cal.example.org:8443\"",
        )]);
        assert_eq!(
            cfg().resolve(&h, None, peer),
            "https://cal.example.org:8443"
        );
    }

    #[test]
    fn ipv4_mapped_peer_matches() {
        let peer = Some("::ffff:10.0.0.1".parse().unwrap());
        let h = headers(&[("host", "x"), ("x-forwarded-proto", "https")]);
        assert_eq!(cfg().resolve(&h, None, peer), "https://x");
    }

    #[test]
    fn default_port_dropped_and_invalid_falls_back() {
        let h = headers(&[("host", "Example.com:80")]);
        assert_eq!(cfg().resolve(&h, None, CLIENT), "http://example.com");
        for bad in ["evil.com/path", "user@evil.com", ""] {
            let h = headers(&[("host", bad)]);
            assert_eq!(
                cfg().resolve(&h, None, CLIENT),
                "https://calendar.example.com"
            );
        }
        let h = headers(&[("host", "x"), ("x-forwarded-proto", "javascript")]);
        assert_eq!(
            cfg().resolve(&h, None, PROXY),
            "https://calendar.example.com"
        );
        assert_eq!(
            cfg().resolve(&HeaderMap::new(), None, CLIENT),
            "https://calendar.example.com"
        );
        assert_eq!(
            cfg().resolve(&HeaderMap::new(), Some("h2.local:3000"), CLIENT),
            "http://h2.local:3000"
        );
    }

    #[test]
    fn invalid_proxy_entry_is_an_error() {
        assert!(OriginConfig::new("http://x", &["nope".to_owned()]).is_err());
    }
}

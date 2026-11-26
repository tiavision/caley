//! Outgoing requests to URLs that users or mail name (push endpoints, remote images): only to public
//! addresses. A client built with `PublicOnly` resolves names to their public addresses only, so a name
//! that resolves to an internal one (also on a later lookup: DNS rebinding) gets no connection; an address
//! written in the URL is not resolved, so callers check it with `literal_host_is_public`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Resolves names to their public addresses only (see the module).
pub(crate) struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|addr| is_public(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host}: no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// `host` as an address, if it is one (an IPv6 one in brackets, as URLs write it).
pub(crate) fn literal_ip(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// Whether a client with `PublicOnly` may be sent to `url`: a name (the resolver checks it), or an address
/// that is public. No host: no.
pub(crate) fn literal_host_is_public(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(_)) => true,
        Some(url::Host::Ipv4(v4)) => is_public(IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => is_public(IpAddr::V6(v6)),
        None => false,
    }
}

/// Whether `ip` is on the public internet: not loopback, private (RFC 1918, IPv6 unique local), link-local,
/// shared (100.64/10), unspecified ("this network", 0/8), broadcast, multicast, IETF protocol assignments
/// (192.0.0/24), benchmarking, documentation or reserved (240/4). IPv6 that carries an IPv4 address
/// (v4-mapped, v4-compatible, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) counts as that address; Teredo
/// (`2001::/32`) and local-use NAT64 (`64:ff9b:1::/48`), whose IPv4 cannot be checked simply, never.
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            let low32 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
            // ::a.b.c.d and ::ffff:a.b.c.d (also ::1 → 0.0.0.1 and :: → 0.0.0.0, neither public)
            if let Some(v4) = v6.to_ipv4() {
                return is_public_v4(v4);
            }
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_public_v4(low32);
            }
            if s[0] == 0x2002 {
                let embedded =
                    Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
                return is_public_v4(embedded);
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                || (s[0] == 0x2001 && s[1] == 0)
                || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1))
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    !(v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses() {
        for ip in [
            "93.184.215.14",
            "2606:4700::6810:84e5",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.178.20",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "192.0.0.8",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "255.255.255.255",
            "240.0.0.1",
            "224.0.0.1",
            "198.18.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "::169.254.169.254",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::808:808",
            "2002:7f00:1::",
            "2002:a00:1::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:db8::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn literal_hosts_are_checked_names_left_to_the_resolver() {
        for url in [
            "http://127.0.0.1/a.png",
            "http://2130706433/a.png",
            "http://0x7f.1/a.png",
            "http://10.0.0.5/a.png",
            "http://100.64.1.1/a.png",
            "http://192.168.1.1:8080/a.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/a.png",
            "http://[::ffff:127.0.0.1]/a.png",
            "http://[64:ff9b::a00:1]/a.png",
            "http://[fd00::1]/a.png",
        ] {
            assert!(
                !literal_host_is_public(&url::Url::parse(url).unwrap()),
                "{url}"
            );
        }
        for url in [
            "https://93.184.215.14/a.png",
            "https://[2606:4700::1]/a.png",
            "https://images.example.com/a.png",
        ] {
            assert!(
                literal_host_is_public(&url::Url::parse(url).unwrap()),
                "{url}"
            );
        }
    }

    /// A name that resolves only to loopback gets no address.
    #[tokio::test]
    async fn the_resolver_yields_no_internal_address() {
        use reqwest::dns::Resolve;
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(PublicOnly.resolve(name).await.is_err());
    }
}

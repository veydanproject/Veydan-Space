// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Which addresses may be asked.
//!
//! A link in a message is written by someone else. It must not make this
//! device talk to itself, to the router or to anything else that is only
//! reachable from the inside.

use crate::refuse;
use messenger_core::Result;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const MAX_URL_LEN: usize = 2048;

/// An `https` address that names a host on the open internet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    url: String,
    host: String,
}

fn v4_is_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || o[0] == 0
        // Shared address space of carriers, 100.64.0.0/10.
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        // Protocol assignments, 192.0.0.0/24.
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        // Benchmarking, 198.18.0.0/15.
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        // Reserved, 240.0.0.0/4.
        || o[0] >= 240)
}

fn v6_is_public(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return v4_is_public(v4);
    }
    let s = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique local, fc00::/7.
        || (s[0] & 0xfe00) == 0xfc00
        // Link local, fe80::/10.
        || (s[0] & 0xffc0) == 0xfe80
        // Documentation, 2001:db8::/32.
        || (s[0] == 0x2001 && s[1] == 0x0db8)
        // IPv4 inside: 64:ff9b::/96 and the old compatible form.
        || (s[0] == 0x0064 && s[1] == 0xff9b)
        || s[..6].iter().all(|x| *x == 0))
}

/// Not loopback, not private, not link-local, not reserved.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_is_public(v4),
        IpAddr::V6(v6) => v6_is_public(v6),
    }
}

impl Target {
    pub fn parse(url: &str) -> Result<Self> {
        let s = url.trim();
        if s.len() > MAX_URL_LEN || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(refuse("preview_bad_url"));
        }
        let rest = match s.get(..8) {
            Some(p) if p.eq_ignore_ascii_case("https://") => &s[8..],
            _ => return Err(refuse("preview_not_https")),
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        // `https://good.example@evil.example/` goes to the second one.
        if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
            return Err(refuse("preview_bad_url"));
        }
        let host = if let Some(v6) = authority.strip_prefix('[') {
            let (inside, after) = v6.split_once(']').ok_or_else(|| refuse("preview_bad_url"))?;
            port_of(after)?;
            let ip: Ipv6Addr = inside.parse().map_err(|_| refuse("preview_bad_url"))?;
            if !is_public(IpAddr::V6(ip)) {
                return Err(refuse("preview_private_host"));
            }
            inside.to_ascii_lowercase()
        } else {
            let (name, after) = match authority.split_once(':') {
                Some((n, p)) => (n, format!(":{p}")),
                None => (authority, String::new()),
            };
            port_of(&after)?;
            host_of(name)?
        };
        Ok(Self { url: s.to_string(), host })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Lowercase, without port.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// An address a page names (`/a.png`, `//cdn.example/a.png`, a full one), as a target.
    pub fn join(&self, reference: &str) -> Result<Self> {
        let r = reference.trim();
        if r.is_empty() {
            return Err(refuse("preview_bad_url"));
        }
        if r.contains("://") {
            return Self::parse(r);
        }
        // `data:…`, `javascript:…`: a scheme, and not ours.
        if r.split(['/', '?', '#']).next().unwrap_or_default().contains(':') {
            return Err(refuse("preview_bad_url"));
        }
        if let Some(rest) = r.strip_prefix("//") {
            return Self::parse(&format!("https://{rest}"));
        }
        let origin_end = 8 + self.url[8..].find(['/', '?', '#']).unwrap_or(self.url.len() - 8);
        let origin = &self.url[..origin_end];
        if r.starts_with('/') {
            return Self::parse(&format!("{origin}{r}"));
        }
        let path = self.url[origin_end..].split(['?', '#']).next().unwrap_or_default();
        let dir = match path.rfind('/') {
            Some(i) => &path[..=i],
            None => "/",
        };
        Self::parse(&format!("{origin}{dir}{r}"))
    }
}

fn port_of(after_host: &str) -> Result<()> {
    match after_host.strip_prefix(':') {
        None if after_host.is_empty() => Ok(()),
        Some(p) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n != 0) => Ok(()),
        _ => Err(refuse("preview_bad_url")),
    }
}

fn host_of(name: &str) -> Result<String> {
    let host = name.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 {
        return Err(refuse("preview_bad_url"));
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return if is_public(IpAddr::V4(ip)) { Ok(host) } else { Err(refuse("preview_private_host")) };
    }
    let labels_ok = host.split('.').all(|l| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    if !labels_ok {
        return Err(refuse("preview_bad_url"));
    }
    // Numbers written another way (`0x7f.1`, `2130706433`) are addresses to some resolvers.
    let last = host.rsplit('.').next().unwrap_or_default();
    if last.bytes().all(|b| b.is_ascii_digit()) || last.starts_with("0x") {
        return Err(refuse("preview_bad_url"));
    }
    // Names that never leave the machine or the local network.
    if !host.contains('.') || ["localhost", "local", "internal", "lan", "home", "onion"].contains(&last) {
        return Err(refuse("preview_private_host"));
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::MessengerError;

    fn code(url: &str) -> String {
        match Target::parse(url) {
            Err(MessengerError::Invalid(c)) => c,
            other => panic!("{url}: {other:?}"),
        }
    }

    #[test]
    fn open_addresses() {
        for (url, host) in [
            ("https://example.com", "example.com"),
            ("HTTPS://Example.COM/Path?q=1#x", "example.com"),
            ("https://sub.example.co.uk:8443/a", "sub.example.co.uk"),
            ("https://example.com./", "example.com"),
            ("https://xn--e1afmkfd.xn--p1ai/", "xn--e1afmkfd.xn--p1ai"),
            ("https://93.184.216.34/", "93.184.216.34"),
            ("https://[2606:2800:220:1:248:1893:25c8:1946]:443/", "2606:2800:220:1:248:1893:25c8:1946"),
        ] {
            assert_eq!(Target::parse(url).unwrap().host(), host, "{url}");
        }
    }

    #[test]
    fn refusals() {
        for url in ["http://example.com", "ftp://example.com", "veydan://group/x", "javascript:alert(1)", "example.com", ""] {
            assert_eq!(code(url), "preview_not_https", "{url}");
        }
        for url in [
            "https://localhost/",
            "https://printer/",
            "https://router.lan/",
            "https://nas.local/",
            "https://db.internal/",
            "https://abc.onion/",
            "https://127.0.0.1/",
            "https://127.1.2.3:8080/",
            "https://10.0.0.1/",
            "https://172.16.5.4/",
            "https://192.168.1.1/",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.0.1/",
            "https://0.0.0.0/",
            "https://255.255.255.255/",
            "https://[::1]/",
            "https://[::]/",
            "https://[fe80::1]/",
            "https://[fd00::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[::ffff:10.0.0.1]/",
            "https://[64:ff9b::7f00:1]/",
        ] {
            assert_eq!(code(url), "preview_private_host", "{url}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        for url in [
            "https://",
            "https:///path",
            "https://good.example@evil.example/",
            "https://user:pw@example.com/",
            "https://example.com\\@evil.example/",
            "https://exa mple.com/",
            "https://example.com/a\nb",
            "https://example.com:0/",
            "https://example.com:99999/",
            "https://example.com:8a/",
            "https://-bad.example/",
            "https://bad_.example/",
            "https://2130706433/",
            "https://0x7f.0.0.1/",
            "https://127.1/",
            "https://[::1/",
            "https://[zz]/",
            long.as_str(),
        ] {
            assert_eq!(code(url), "preview_bad_url", "{url}");
        }
    }

    #[test]
    fn addresses_a_page_names() {
        let page = Target::parse("https://example.com/news/today.html?x=1#top").unwrap();
        for (reference, url) in [
            ("/img/a.png", "https://example.com/img/a.png"),
            ("a.png", "https://example.com/news/a.png"),
            ("//cdn.example.net/a.png", "https://cdn.example.net/a.png"),
            ("https://other.example/a.png", "https://other.example/a.png"),
        ] {
            assert_eq!(page.join(reference).unwrap().url(), url, "{reference}");
        }
        let bare = Target::parse("https://example.com").unwrap();
        assert_eq!(bare.join("a.png").unwrap().url(), "https://example.com/a.png");
        assert_eq!(bare.join("/a.png").unwrap().url(), "https://example.com/a.png");
        assert!(page.join("http://example.com/a.png").is_err());
        assert!(page.join("//127.0.0.1/a.png").is_err());
        assert!(page.join("data://x").is_err());
        assert!(page.join("data:image/png;base64,AAAA").is_err());
        assert!(page.join("javascript:alert(1)").is_err());
        assert!(page.join(" ").is_err());
    }

    #[test]
    fn public_and_not() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.0.1", "172.31.255.255", "169.254.0.1", "100.127.0.1", "198.18.0.1", "224.0.0.1", "240.0.0.1", "::1", "fc00::1", "fe80::1", "ff02::1", "2001:db8::1", "::ffff:192.168.0.1", "::7f00:1"] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }
}

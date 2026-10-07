// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Links to the user's other profiles: Telegram, GitHub, Mastodon and so on.
//!
//! A link is stored and sent as a platform id and a handle (`SocialLink`,
//! `{"p":"github","h":"octocat"}`); the address is built here from the
//! platform's template, never taken from the wire. Every handle is checked
//! against the platform's own charset and length, all of it ASCII and none of
//! it `/ ? # @ % : \`, so a handle cannot bend the address to another host or
//! path. The one exception is the platform `other`, whose handle is a whole
//! https address, checked on its own.
//!
//! `normalize_link` is for what the user types: a handle, `@handle` or a
//! pasted profile address of that platform. `view` and `clean_list` are for
//! what was received: they accept only the stored form and drop the rest.

use crate::phone;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Links in one profile or card.
pub const MAX_SOCIALS: usize = 16;
/// The address of `other`, in bytes.
pub const MAX_OTHER_URL_BYTES: usize = 512;
/// Longer input is refused before it is looked at.
const MAX_INPUT_BYTES: usize = 1024;

/// Wire and storage form: a platform id and a handle.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct SocialLink {
    pub p: String,
    pub h: String,
}

/// A checked link as the UI shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SocialView {
    pub platform: String,
    /// Name of the platform, e.g. "GitHub".
    pub name: String,
    /// The handle as people write it there, e.g. "@name" or "u/name".
    pub handle: String,
    /// Profile address; empty when the platform has none for this handle
    /// (a Discord username).
    pub url: String,
}

/// A platform the user can pick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SocialPlatform {
    pub id: String,
    pub name: String,
    /// What to type, e.g. "@username" or "https://…".
    pub hint: String,
}

/// Charset and length of a plain handle. ASCII letters and digits are always
/// allowed, `extra` adds more; a char of `edges` may be neither first, last,
/// nor doubled (so no `.`, `..` or `-x-` tricks).
#[derive(Clone, Copy)]
struct Rule {
    min: usize,
    max: usize,
    extra: &'static str,
    edges: &'static str,
    first_alpha: bool,
    lower: bool,
}

#[derive(Clone, Copy)]
enum Check {
    Rule(Rule),
    /// Digits of a phone number, without the `+`.
    Phone,
    /// A username, or a numeric user id that has an address.
    Discord,
    /// `user@instance`; the address is on the instance.
    Mastodon,
    /// A domain handle or a `did:plc:`.
    Bluesky,
    /// A whole https address.
    Other,
}

struct Platform {
    id: &'static str,
    name: &'static str,
    hint: &'static str,
    /// The first one builds the address; all are accepted when pasted.
    hosts: &'static [&'static str],
    /// Path templates with `{h}` last; the first one builds the address.
    paths: &'static [&'static str],
    /// Put before the handle when shown.
    prefix: &'static str,
    check: Check,
    /// Paths of the platform's own pages that fit the charset but are no
    /// profile, e.g. instagram.com/p/… or facebook.com/profile.php?id=….
    reserved: &'static [&'static str],
}

const fn rule(min: usize, max: usize, extra: &'static str, edges: &'static str) -> Check {
    Check::Rule(Rule { min, max, extra, edges, first_alpha: false, lower: false })
}

const PLATFORMS: &[Platform] = &[
    Platform {
        id: "telegram",
        name: "Telegram",
        hint: "@username",
        hosts: &["t.me", "telegram.me"],
        paths: &["/{h}"],
        prefix: "@",
        check: Check::Rule(Rule { min: 5, max: 32, extra: "_", edges: "", first_alpha: true, lower: false }),
        reserved: &["joinchat", "addstickers", "addemoji", "share", "proxy", "socks"],
    },
    Platform {
        id: "instagram",
        name: "Instagram",
        hint: "@username",
        hosts: &["instagram.com"],
        paths: &["/{h}"],
        prefix: "@",
        check: rule(1, 30, "._", "."),
        reserved: &["p", "reel", "reels", "tv", "explore", "stories", "accounts", "direct"],
    },
    Platform {
        id: "tiktok",
        name: "TikTok",
        hint: "@username",
        hosts: &["tiktok.com"],
        paths: &["/@{h}"],
        prefix: "@",
        check: rule(2, 24, "._", "."),
        reserved: &[],
    },
    Platform {
        id: "x",
        name: "X",
        hint: "@username",
        hosts: &["x.com", "twitter.com"],
        paths: &["/{h}"],
        prefix: "@",
        check: rule(1, 15, "_", ""),
        reserved: &["home", "i", "search", "explore", "intent", "share", "messages", "settings", "notifications"],
    },
    Platform {
        id: "youtube",
        name: "YouTube",
        hint: "@handle",
        hosts: &["youtube.com"],
        paths: &["/@{h}"],
        prefix: "@",
        check: rule(3, 30, "._-", "."),
        reserved: &[],
    },
    Platform {
        id: "vk",
        name: "VK",
        hint: "username",
        hosts: &["vk.com", "vk.ru"],
        paths: &["/{h}"],
        prefix: "",
        check: rule(1, 32, "._", "."),
        reserved: &["feed", "im", "search", "away.php", "login"],
    },
    Platform {
        id: "facebook",
        name: "Facebook",
        hint: "username",
        hosts: &["facebook.com", "fb.com"],
        paths: &["/{h}"],
        prefix: "",
        check: rule(5, 50, ".", "."),
        reserved: &["profile.php", "groups", "pages", "watch", "events", "marketplace", "sharer.php", "share", "login"],
    },
    Platform {
        id: "linkedin",
        name: "LinkedIn",
        hint: "username",
        hosts: &["linkedin.com"],
        paths: &["/in/{h}"],
        prefix: "",
        check: rule(3, 100, "-", "-"),
        reserved: &[],
    },
    Platform {
        id: "github",
        name: "GitHub",
        hint: "username",
        hosts: &["github.com"],
        paths: &["/{h}"],
        prefix: "",
        check: rule(1, 39, "-", "-"),
        reserved: &["orgs", "settings", "topics", "marketplace", "explore", "sponsors", "login", "notifications"],
    },
    Platform {
        id: "whatsapp",
        name: "WhatsApp",
        hint: "+1 555 123 4567",
        hosts: &["wa.me"],
        paths: &["/{h}"],
        prefix: "+",
        check: Check::Phone,
        reserved: &[],
    },
    Platform {
        id: "discord",
        name: "Discord",
        hint: "username",
        hosts: &["discord.com", "discordapp.com"],
        paths: &["/users/{h}"],
        prefix: "",
        check: Check::Discord,
        reserved: &[],
    },
    Platform {
        id: "twitch",
        name: "Twitch",
        hint: "username",
        hosts: &["twitch.tv"],
        paths: &["/{h}"],
        prefix: "",
        check: rule(3, 25, "_", ""),
        reserved: &["directory", "videos", "settings", "search", "downloads"],
    },
    Platform {
        id: "mastodon",
        name: "Mastodon",
        hint: "@user@mastodon.social",
        hosts: &[],
        paths: &["/@{h}"],
        prefix: "@",
        check: Check::Mastodon,
        reserved: &[],
    },
    Platform {
        id: "bluesky",
        name: "Bluesky",
        hint: "@name.bsky.social",
        hosts: &["bsky.app"],
        paths: &["/profile/{h}"],
        prefix: "@",
        check: Check::Bluesky,
        reserved: &[],
    },
    Platform {
        id: "threads",
        name: "Threads",
        hint: "@username",
        hosts: &["threads.net", "threads.com"],
        paths: &["/@{h}"],
        prefix: "@",
        check: rule(1, 30, "._", "."),
        reserved: &[],
    },
    Platform {
        id: "reddit",
        name: "Reddit",
        hint: "u/username",
        hosts: &["reddit.com"],
        paths: &["/user/{h}", "/u/{h}"],
        prefix: "u/",
        check: rule(3, 20, "_-", ""),
        reserved: &[],
    },
    Platform {
        id: "other",
        name: "Link",
        hint: "https://…",
        hosts: &[],
        paths: &[],
        prefix: "",
        check: Check::Other,
        reserved: &[],
    },
];

/// Subdomains of a platform's host that lead to the same profile.
const SUBDOMAINS: [&str; 5] = ["www.", "m.", "mobile.", "old.", "new."];
/// Hosts that also serve profiles on a two-letter country subdomain, such
/// as ru.linkedin.com.
const COUNTRY_HOSTS: [&str; 1] = ["linkedin.com"];

/// `host` without a two-letter country label, when the rest is one of
/// `COUNTRY_HOSTS`.
fn strip_country(host: &str) -> &str {
    match host.split_once('.') {
        Some((cc, rest))
            if cc.len() == 2 && cc.bytes().all(|b| b.is_ascii_lowercase()) && COUNTRY_HOSTS.contains(&rest) =>
        {
            rest
        }
        _ => host,
    }
}

fn platform(id: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.id == id)
}

/// Every platform, in the order of the picker.
pub fn platforms() -> Vec<SocialPlatform> {
    PLATFORMS
        .iter()
        .map(|p| SocialPlatform { id: p.id.into(), name: p.name.into(), hint: p.hint.into() })
        .collect()
}

/// What the user typed for `platform`, in the stored form. Errors
/// `"social_unknown_platform"`, `"social_bad_handle"`.
pub fn normalize_link(platform_id: &str, input: &str) -> Result<SocialLink, &'static str> {
    let p = platform(platform_id).ok_or("social_unknown_platform")?;
    let s = input.trim();
    if s.is_empty() || s.len() > MAX_INPUT_BYTES {
        return Err("social_bad_handle");
    }
    let h = match p.check {
        Check::Other => check_other(s, true),
        _ => handle_from_input(p, s).and_then(|h| canonical(p, &h)),
    };
    h.map(|h| SocialLink { p: p.id.into(), h }).ok_or("social_bad_handle")
}

/// A received link as shown, or None when it is not valid.
pub fn view(link: &SocialLink) -> Option<SocialView> {
    let p = platform(&link.p)?;
    if link.h.len() > MAX_INPUT_BYTES {
        return None;
    }
    let h = canonical(p, &link.h)?;
    Some(SocialView {
        platform: p.id.into(),
        name: p.name.into(),
        handle: display(p, &h),
        url: url(p, &h),
    })
}

/// The valid links of a received list in the stored form, each once, at most
/// `MAX_SOCIALS`.
pub fn clean_list(list: &[SocialLink]) -> Vec<SocialLink> {
    let mut out: Vec<SocialLink> = Vec::new();
    let mut seen: Vec<(&'static str, String)> = Vec::new();
    for link in list {
        if out.len() >= MAX_SOCIALS {
            break;
        }
        let Some(p) = platform(&link.p) else { continue };
        if link.h.len() > MAX_INPUT_BYTES {
            continue;
        }
        let Some(h) = canonical(p, &link.h) else { continue };
        let key = match p.check {
            Check::Other => h.clone(),
            _ => h.to_ascii_lowercase(),
        };
        if seen.iter().any(|(id, k)| *id == p.id && *k == key) {
            continue;
        }
        seen.push((p.id, key));
        out.push(SocialLink { p: p.id.into(), h });
    }
    out
}

/// `clean_list` of a JSON value from the wire: elements that are not
/// `{"p": string, "h": string}` are skipped instead of failing the list.
pub fn list_from_json(value: &serde_json::Value) -> Vec<SocialLink> {
    let Some(items) = value.as_array() else { return Vec::new() };
    let links: Vec<SocialLink> = items
        .iter()
        .take(MAX_SOCIALS * 4)
        .filter_map(|v| {
            let p = v.get("p")?.as_str()?;
            let h = v.get("h")?.as_str()?;
            Some(SocialLink { p: p.into(), h: h.into() })
        })
        .collect();
    clean_list(&links)
}

/// The handle part of typed input, before the platform's check.
fn handle_from_input(p: &Platform, s: &str) -> Option<String> {
    let s = if p.id == "reddit" { strip_prefix_ci(s, "/u/").or(strip_prefix_ci(s, "u/")).unwrap_or(s) } else { s };
    if strip_scheme(s).is_some() || s.contains('/') {
        return handle_from_url(p, s);
    }
    match p.check {
        Check::Phone => phone::normalize_phone(s).ok().map(|n| n[1..].to_string()),
        Check::Bluesky => {
            let h = s.strip_prefix('@').unwrap_or(s).to_ascii_lowercase();
            if h.contains('.') || h.starts_with("did:") {
                Some(h)
            } else {
                Some(format!("{h}.bsky.social"))
            }
        }
        _ => Some(s.strip_prefix('@').unwrap_or(s).to_string()),
    }
}

/// The handle in a pasted profile address of `p`.
fn handle_from_url(p: &Platform, s: &str) -> Option<String> {
    let (host, path) = split_url(s)?;
    if let Check::Mastodon = p.check {
        let user = match_path(&path, p.paths)?;
        let host = valid_host(&host)?;
        return Some(format!("{user}@{host}"));
    }
    let mut host = host.as_str();
    for sub in SUBDOMAINS {
        if let Some(rest) = host.strip_prefix(sub) {
            host = rest;
            break;
        }
    }
    let host = strip_country(host);
    if !p.hosts.contains(&host) {
        return None;
    }
    match_path(&path, p.paths).map(str::to_string)
}

/// Lowercased host and the path without query, fragment and trailing
/// slashes; None for userinfo, a port, or anything but printable ASCII.
fn split_url(s: &str) -> Option<(String, String)> {
    let rest = strip_scheme(s).unwrap_or(s);
    if !rest.bytes().all(|b| b.is_ascii_graphic()) || rest.contains('\\') {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains(['@', ':']) {
        return None;
    }
    let host = authority.strip_suffix('.').unwrap_or(authority).to_ascii_lowercase();
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = tail[..path_end].trim_end_matches('/');
    Some((host, path.to_string()))
}

/// The `{h}` of the first template the path fits; it is one segment.
fn match_path<'a>(path: &'a str, templates: &[&str]) -> Option<&'a str> {
    templates.iter().find_map(|t| {
        let pre = t.strip_suffix("{h}")?;
        let rest = strip_prefix_ci(path, pre)?;
        (!rest.is_empty() && !rest.contains('/')).then_some(rest)
    })
}

fn strip_scheme(s: &str) -> Option<&str> {
    strip_prefix_ci(s, "https://").or_else(|| strip_prefix_ci(s, "http://"))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// The stored form of a handle, or None. Takes only the stored form: no `@`,
/// no address, no phone formatting.
fn canonical(p: &Platform, h: &str) -> Option<String> {
    match p.check {
        Check::Rule(r) => check_rule(&r, h).filter(|h| !p.reserved.iter().any(|w| w.eq_ignore_ascii_case(h))),
        Check::Phone => phone::valid_digits(h).then(|| h.to_string()),
        Check::Discord => {
            let h = h.to_ascii_lowercase();
            if is_discord_id(&h) {
                return Some(h);
            }
            check_rule(&DISCORD_NAME, &h)
        }
        Check::Mastodon => {
            let (user, host) = h.split_once('@')?;
            let user = check_rule(&MASTODON_USER, user)?;
            let host = valid_host(host)?;
            Some(format!("{user}@{host}"))
        }
        Check::Bluesky => {
            let h = h.to_ascii_lowercase();
            if let Some(id) = h.strip_prefix("did:plc:") {
                let ok = id.len() == 24 && id.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'));
                return ok.then_some(h);
            }
            valid_host(&h)
        }
        Check::Other => check_other(h, false),
    }
}

fn check_rule(r: &Rule, h: &str) -> Option<String> {
    let b = h.as_bytes();
    if b.len() < r.min || b.len() > r.max {
        return None;
    }
    if !b.iter().all(|&c| c.is_ascii_alphanumeric() || (c.is_ascii() && r.extra.contains(c as char))) {
        return None;
    }
    if r.first_alpha && !b[0].is_ascii_alphabetic() {
        return None;
    }
    let edge = |c: u8| r.edges.as_bytes().contains(&c);
    if edge(b[0]) || edge(b[b.len() - 1]) || b.windows(2).any(|w| edge(w[0]) && edge(w[1])) {
        return None;
    }
    Some(if r.lower { h.to_ascii_lowercase() } else { h.to_string() })
}

/// A Discord username; lowercased.
const DISCORD_NAME: Rule = Rule { min: 2, max: 32, extra: "._", edges: ".", first_alpha: false, lower: true };
/// The user part of `user@instance`.
const MASTODON_USER: Rule = Rule { min: 1, max: 30, extra: "_.-", edges: ".-", first_alpha: false, lower: false };

/// A Discord user id (a snowflake); only these have an address.
fn is_discord_id(h: &str) -> bool {
    (17..=20).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_digit())
}

/// A DNS name with a dot, lowercased: labels of `a-z 0-9 -`, no `-` at their
/// edges, the last one no number (so no IP address).
fn valid_host(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return None;
    }
    for l in &labels {
        let b = l.as_bytes();
        if b.is_empty() || b.len() > 63 || b[0] == b'-' || b[b.len() - 1] == b'-' {
            return None;
        }
        if !b.iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
            return None;
        }
    }
    if is_ipv4_number(labels[labels.len() - 1]) {
        return None;
    }
    Some(host)
}

/// A label a browser reads as a number, which makes the whole host an IPv4
/// address (WHATWG URL): decimal or octal digits, or `0x` and hex digits, so
/// 127.0x1 and 0x7f.0x1 are 127.0.0.1. Takes a lowercased label.
fn is_ipv4_number(label: &str) -> bool {
    match label.strip_prefix("0x") {
        Some(hex) => hex.bytes().all(|c| c.is_ascii_hexdigit()),
        None => label.bytes().all(|c| c.is_ascii_digit()),
    }
}

/// Characters an address of `other` may not carry anywhere: whitespace,
/// controls, invisible ones, and what is never left unescaped.
fn bad_url_char(c: char) -> bool {
    c.is_control()
        || c.is_whitespace()
        || matches!(c, '\\' | '<' | '>' | '"' | '`' | '{' | '}' | '|' | '^')
        || is_format_char(c)
        || is_blank_char(c)
}

/// The Unicode category Cf (format characters, Unicode 16): all invisible.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

/// Characters of other categories that show as nothing: the combining
/// grapheme joiner, the Khmer inherent vowels and the Hangul fillers.
fn is_blank_char(c: char) -> bool {
    matches!(c, '\u{034f}' | '\u{115f}' | '\u{1160}' | '\u{17b4}' | '\u{17b5}' | '\u{3164}' | '\u{ffa0}')
}

/// Whether typed input names a scheme of its own: `name:` right before
/// `//`, in front of the first `/`, `?` or `#`. A `://` later on is part of
/// the path or query.
fn has_scheme(s: &str) -> bool {
    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    s[..end].ends_with(':') && s[end..].starts_with("//")
}

/// A whole https address with a host and no userinfo, at most
/// `MAX_OTHER_URL_BYTES`; host lowercased. `lenient` lets typed input leave
/// out `https://`.
fn check_other(s: &str, lenient: bool) -> Option<String> {
    if s.len() > MAX_INPUT_BYTES || s.chars().any(bad_url_char) {
        return None;
    }
    let rest = match strip_prefix_ci(s, "https://") {
        Some(rest) => rest,
        None if lenient && !has_scheme(s) && strip_prefix_ci(s, "http:").is_none() => s,
        None => return None,
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    let host = valid_host(host)?;
    let port = match port {
        None => String::new(),
        Some(p) if (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()) => {
            let n: u32 = p.parse().ok()?;
            if n == 0 || n > 65535 {
                return None;
            }
            format!(":{p}")
        }
        Some(_) => return None,
    };
    let out = format!("https://{host}{port}{tail}");
    (out.len() <= MAX_OTHER_URL_BYTES).then_some(out)
}

fn url(p: &Platform, h: &str) -> String {
    match p.check {
        Check::Other => h.to_string(),
        Check::Mastodon => match h.split_once('@') {
            Some((user, host)) => format!("https://{host}/@{user}"),
            None => String::new(),
        },
        Check::Discord if !is_discord_id(h) => String::new(),
        _ => format!("https://{}{}", p.hosts[0], p.paths[0].replace("{h}", h)),
    }
}

fn display(p: &Platform, h: &str) -> String {
    match p.check {
        Check::Other => h.strip_prefix("https://").unwrap_or(h).to_string(),
        _ => format!("{}{h}", p.prefix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(p: &str, s: &str) -> Result<String, &'static str> {
        normalize_link(p, s).map(|l| l.h)
    }

    fn link(p: &str, h: &str) -> SocialLink {
        SocialLink { p: p.into(), h: h.into() }
    }

    fn url_of(p: &str, h: &str) -> String {
        view(&link(p, h)).expect("valid").url
    }

    /// Handles that try to leave the path or the host, for every platform.
    const HOSTILE: &[&str] = &[
        "../evil",
        "..",
        ".",
        "a/b",
        "a?x",
        "a#b",
        "a@evil.com",
        "a%2fb",
        "a%40evil.com",
        "a:b",
        "a\\b",
        "a b",
        "a\tb",
        "a\nb",
        "a\u{0}b",
        "javascript:alert(1)",
        "//evil.com",
        "аdmin_user",      // Cyrillic а
        "adm\u{0131}n_user", // dotless i
        "ａdmin_user",     // fullwidth
        "admin\u{200b}user",
        "admin\u{202e}resu",
        "ádmin_user",
    ];

    #[test]
    fn registry_is_complete_and_unique() {
        let ids: Vec<String> = platforms().into_iter().map(|p| p.id).collect();
        let want = [
            "telegram", "instagram", "tiktok", "x", "youtube", "vk", "facebook", "linkedin", "github", "whatsapp",
            "discord", "twitch", "mastodon", "bluesky", "threads", "reddit", "other",
        ];
        assert_eq!(ids, want);
        for p in PLATFORMS {
            assert!(!p.name.is_empty() && !p.hint.is_empty());
            for t in p.paths {
                assert!(t.starts_with('/') && t.ends_with("{h}") && t.matches("{h}").count() == 1, "{}", p.id);
            }
            if !matches!(p.check, Check::Other | Check::Mastodon) {
                assert!(!p.hosts.is_empty() && !p.paths.is_empty(), "{}", p.id);
            }
        }
    }

    #[test]
    fn unknown_platform() {
        assert_eq!(normalize_link("myspace", "tom"), Err("social_unknown_platform"));
        assert_eq!(normalize_link("", "tom"), Err("social_unknown_platform"));
        assert_eq!(normalize_link("GitHub", "tom"), Err("social_unknown_platform"));
        assert_eq!(view(&link("myspace", "tom")), None);
    }

    #[test]
    fn hostile_handles_are_refused_everywhere() {
        for p in PLATFORMS {
            for h in HOSTILE {
                // user@instance is what a Mastodon handle is.
                if p.id == "mastodon" && *h == "a@evil.com" {
                    continue;
                }
                assert_eq!(normalize_link(p.id, h), Err("social_bad_handle"), "{} {h:?}", p.id);
                assert_eq!(normalize_link(p.id, &format!("@{h}")), Err("social_bad_handle"), "{} @{h:?}", p.id);
                assert_eq!(view(&link(p.id, h)), None, "{} {h:?}", p.id);
            }
        }
    }

    #[test]
    fn reserved_paths_are_no_profiles() {
        assert_eq!(norm("facebook", "https://facebook.com/profile.php?id=4"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "https://instagram.com/p/"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "https://www.instagram.com/explore/"), Err("social_bad_handle"));
        assert_eq!(norm("x", "https://x.com/home"), Err("social_bad_handle"));
        assert_eq!(norm("x", "I"), Err("social_bad_handle"));
        assert_eq!(norm("github", "https://github.com/Settings"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me/joinchat"), Err("social_bad_handle"));
        assert_eq!(norm("twitch", "directory"), Err("social_bad_handle"));
        assert_eq!(norm("vk", "feed"), Err("social_bad_handle"));
        assert_eq!(view(&link("instagram", "explore")), None);
        for p in PLATFORMS {
            for w in p.reserved {
                assert_eq!(view(&link(p.id, w)), None, "{} {w}", p.id);
            }
        }
        // A word reserved on one platform is a name on another.
        assert_eq!(norm("tiktok", "explore").unwrap(), "explore");
    }

    #[test]
    fn empty_and_huge_input() {
        for p in PLATFORMS {
            assert_eq!(normalize_link(p.id, ""), Err("social_bad_handle"));
            assert_eq!(normalize_link(p.id, "   "), Err("social_bad_handle"));
            assert_eq!(normalize_link(p.id, "@"), Err("social_bad_handle"));
            assert_eq!(normalize_link(p.id, &"a".repeat(5000)), Err("social_bad_handle"));
            assert_eq!(view(&link(p.id, "")), None);
            assert_eq!(view(&link(p.id, &"a".repeat(5000))), None);
        }
    }

    #[test]
    fn urls_never_leave_the_platform_host() {
        let samples = [
            ("telegram", "durov"),
            ("instagram", "a.b_c"),
            ("tiktok", "a.b"),
            ("x", "jack"),
            ("youtube", "a-b.c_d"),
            ("vk", "id1"),
            ("facebook", "zuck.1"),
            ("linkedin", "john-doe-1a2b"),
            ("github", "octo-cat"),
            ("whatsapp", "15551234567"),
            ("discord", "80351110224678912"),
            ("twitch", "ninja"),
            ("bluesky", "jay.bsky.team"),
            ("threads", "zuck"),
            ("reddit", "spez"),
        ];
        for (p, h) in samples {
            let u = url_of(p, h);
            let plat = platform(p).unwrap();
            let host = &u["https://".len()..u["https://".len()..].find('/').unwrap() + "https://".len()];
            assert_eq!(host, plat.hosts[0], "{p}");
            assert!(u.ends_with(h), "{p} {u}");
            assert!(!u.contains("..") && !u.contains('?') && !u.contains('#') && !u.contains('%'), "{u}");
        }
    }

    #[test]
    fn telegram() {
        assert_eq!(norm("telegram", "durov").unwrap(), "durov");
        assert_eq!(norm("telegram", "@durov").unwrap(), "durov");
        assert_eq!(norm("telegram", "https://t.me/durov").unwrap(), "durov");
        assert_eq!(norm("telegram", "http://t.me/durov/").unwrap(), "durov");
        assert_eq!(norm("telegram", "t.me/durov").unwrap(), "durov");
        assert_eq!(norm("telegram", "https://telegram.me/durov?start=1").unwrap(), "durov");
        assert_eq!(norm("telegram", "HTTPS://WWW.T.ME/durov#x").unwrap(), "durov");
        assert_eq!(norm("telegram", "Some_Name_1").unwrap(), "Some_Name_1");
        assert_eq!(norm("telegram", "abcd"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "1durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "_durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", &"a".repeat(33)), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me/durov/123"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://evil.com/durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me.evil.com/durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://evilt.me/durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://user@t.me/durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me:8443/durov"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me/"), Err("social_bad_handle"));
        assert_eq!(norm("telegram", "https://t.me"), Err("social_bad_handle"));
        let v = view(&link("telegram", "durov")).unwrap();
        assert_eq!(
            v,
            SocialView {
                platform: "telegram".into(),
                name: "Telegram".into(),
                handle: "@durov".into(),
                url: "https://t.me/durov".into()
            }
        );
    }

    #[test]
    fn instagram() {
        assert_eq!(norm("instagram", "@nat.geo").unwrap(), "nat.geo");
        assert_eq!(norm("instagram", "https://www.instagram.com/natgeo/?hl=en").unwrap(), "natgeo");
        assert_eq!(norm("instagram", "instagram.com/natgeo").unwrap(), "natgeo");
        assert_eq!(norm("instagram", "a").unwrap(), "a");
        assert_eq!(norm("instagram", "_a_").unwrap(), "_a_");
        assert_eq!(norm("instagram", ".natgeo"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "natgeo."), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "nat..geo"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "nat-geo"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", &"a".repeat(31)), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "https://instagram.com/p/Cxyz"), Err("social_bad_handle"));
        assert_eq!(norm("instagram", "https://instagram.com.evil.com/natgeo"), Err("social_bad_handle"));
        assert_eq!(url_of("instagram", "natgeo"), "https://instagram.com/natgeo");
        assert_eq!(view(&link("instagram", "natgeo")).unwrap().handle, "@natgeo");
    }

    #[test]
    fn tiktok() {
        assert_eq!(norm("tiktok", "@khaby.lame").unwrap(), "khaby.lame");
        assert_eq!(norm("tiktok", "https://www.tiktok.com/@khaby.lame?lang=en").unwrap(), "khaby.lame");
        assert_eq!(norm("tiktok", "https://m.tiktok.com/@khaby").unwrap(), "khaby");
        assert_eq!(norm("tiktok", "https://www.tiktok.com/khaby"), Err("social_bad_handle"));
        assert_eq!(norm("tiktok", "https://www.tiktok.com/@khaby/video/123"), Err("social_bad_handle"));
        assert_eq!(norm("tiktok", "a"), Err("social_bad_handle"));
        assert_eq!(norm("tiktok", &"a".repeat(25)), Err("social_bad_handle"));
        assert_eq!(url_of("tiktok", "khaby.lame"), "https://tiktok.com/@khaby.lame");
        assert_eq!(view(&link("tiktok", "khaby")).unwrap().handle, "@khaby");
    }

    #[test]
    fn x() {
        assert_eq!(norm("x", "@jack").unwrap(), "jack");
        assert_eq!(norm("x", "https://x.com/jack").unwrap(), "jack");
        assert_eq!(norm("x", "https://twitter.com/jack?s=20").unwrap(), "jack");
        assert_eq!(norm("x", "https://mobile.twitter.com/jack").unwrap(), "jack");
        assert_eq!(norm("x", "j").unwrap(), "j");
        assert_eq!(norm("x", "a_very_long_name").unwrap_err(), "social_bad_handle");
        assert_eq!(norm("x", "ja.ck"), Err("social_bad_handle"));
        assert_eq!(norm("x", "https://x.com/jack/status/20"), Err("social_bad_handle"));
        assert_eq!(url_of("x", "jack"), "https://x.com/jack");
    }

    #[test]
    fn youtube() {
        assert_eq!(norm("youtube", "@MrBeast").unwrap(), "MrBeast");
        assert_eq!(norm("youtube", "https://www.youtube.com/@MrBeast").unwrap(), "MrBeast");
        assert_eq!(norm("youtube", "https://m.youtube.com/@a-b.c_d/").unwrap(), "a-b.c_d");
        assert_eq!(norm("youtube", "https://youtube.com/c/MrBeast"), Err("social_bad_handle"));
        assert_eq!(norm("youtube", "https://youtube.com/watch?v=x"), Err("social_bad_handle"));
        assert_eq!(norm("youtube", "ab"), Err("social_bad_handle"));
        assert_eq!(norm("youtube", "a..b"), Err("social_bad_handle"));
        assert_eq!(url_of("youtube", "MrBeast"), "https://youtube.com/@MrBeast");
        assert_eq!(view(&link("youtube", "MrBeast")).unwrap().handle, "@MrBeast");
    }

    #[test]
    fn vk() {
        assert_eq!(norm("vk", "durov").unwrap(), "durov");
        assert_eq!(norm("vk", "id1").unwrap(), "id1");
        assert_eq!(norm("vk", "https://vk.com/durov").unwrap(), "durov");
        assert_eq!(norm("vk", "https://m.vk.com/durov").unwrap(), "durov");
        assert_eq!(norm("vk", "https://vk.ru/id1").unwrap(), "id1");
        assert_eq!(norm("vk", "du-rov"), Err("social_bad_handle"));
        assert_eq!(url_of("vk", "durov"), "https://vk.com/durov");
        assert_eq!(view(&link("vk", "durov")).unwrap().handle, "durov");
    }

    #[test]
    fn facebook() {
        assert_eq!(norm("facebook", "zuck.1").unwrap(), "zuck.1");
        assert_eq!(norm("facebook", "https://www.facebook.com/zuckerberg").unwrap(), "zuckerberg");
        assert_eq!(norm("facebook", "https://fb.com/zuckerberg").unwrap(), "zuckerberg");
        assert_eq!(norm("facebook", "zuck"), Err("social_bad_handle"));
        assert_eq!(norm("facebook", "zuck_berg"), Err("social_bad_handle"));
        assert_eq!(url_of("facebook", "zuckerberg"), "https://facebook.com/zuckerberg");
    }

    #[test]
    fn linkedin() {
        assert_eq!(norm("linkedin", "john-doe-1a2b").unwrap(), "john-doe-1a2b");
        assert_eq!(norm("linkedin", "https://www.linkedin.com/in/john-doe/").unwrap(), "john-doe");
        assert_eq!(norm("linkedin", "https://linkedin.com/IN/john").unwrap(), "john");
        assert_eq!(norm("linkedin", "https://linkedin.com/company/acme"), Err("social_bad_handle"));
        assert_eq!(norm("linkedin", "https://linkedin.com/john"), Err("social_bad_handle"));
        assert_eq!(norm("linkedin", "-john"), Err("social_bad_handle"));
        assert_eq!(norm("linkedin", "jo"), Err("social_bad_handle"));
        assert_eq!(url_of("linkedin", "john-doe"), "https://linkedin.com/in/john-doe");
        // Country subdomains lead to the same profile.
        assert_eq!(norm("linkedin", "https://ru.linkedin.com/in/john-doe").unwrap(), "john-doe");
        assert_eq!(norm("linkedin", "https://uk.linkedin.com/in/john-doe/").unwrap(), "john-doe");
        assert_eq!(norm("linkedin", "DE.linkedin.com/in/john").unwrap(), "john");
        for bad in [
            "https://rus.linkedin.com/in/john",
            "https://r.linkedin.com/in/john",
            "https://r1.linkedin.com/in/john",
            "https://ru.ru.linkedin.com/in/john",
            "https://ru.evil.com/in/john",
            "https://ru.linkedin.com.evil.com/in/john",
        ] {
            assert_eq!(norm("linkedin", bad), Err("social_bad_handle"), "{bad}");
        }
        // Only LinkedIn uses them.
        assert_eq!(norm("github", "https://ru.github.com/octocat"), Err("social_bad_handle"));
        assert_eq!(norm("x", "https://ru.x.com/jack"), Err("social_bad_handle"));
    }

    #[test]
    fn github() {
        assert_eq!(norm("github", "octocat").unwrap(), "octocat");
        assert_eq!(norm("github", "@octo-cat").unwrap(), "octo-cat");
        assert_eq!(norm("github", "https://github.com/octocat").unwrap(), "octocat");
        assert_eq!(norm("github", "github.com/octocat/").unwrap(), "octocat");
        assert_eq!(norm("github", "https://github.com/octocat/repo"), Err("social_bad_handle"));
        assert_eq!(norm("github", "octo--cat"), Err("social_bad_handle"));
        assert_eq!(norm("github", "-octocat"), Err("social_bad_handle"));
        assert_eq!(norm("github", "octocat-"), Err("social_bad_handle"));
        assert_eq!(norm("github", "octo_cat"), Err("social_bad_handle"));
        assert_eq!(norm("github", &"a".repeat(40)), Err("social_bad_handle"));
        assert_eq!(norm("github", "https://gist.github.com/octocat"), Err("social_bad_handle"));
        assert_eq!(url_of("github", "octocat"), "https://github.com/octocat");
        assert_eq!(view(&link("github", "octocat")).unwrap().handle, "octocat");
    }

    #[test]
    fn whatsapp() {
        assert_eq!(norm("whatsapp", "+1 (555) 123-4567").unwrap(), "15551234567");
        assert_eq!(norm("whatsapp", "+79991234567").unwrap(), "79991234567");
        assert_eq!(norm("whatsapp", "https://wa.me/79991234567").unwrap(), "79991234567");
        assert_eq!(norm("whatsapp", "wa.me/79991234567?text=hi").unwrap(), "79991234567");
        assert_eq!(norm("whatsapp", "https://wa.me/+79991234567"), Err("social_bad_handle"));
        assert_eq!(norm("whatsapp", "+123"), Err("social_bad_handle"));
        assert_eq!(norm("whatsapp", "@john"), Err("social_bad_handle"));
        assert_eq!(norm("whatsapp", "john"), Err("social_bad_handle"));
        let v = view(&link("whatsapp", "79991234567")).unwrap();
        assert_eq!(v.url, "https://wa.me/79991234567");
        assert_eq!(v.handle, "+79991234567");
        assert_eq!(view(&link("whatsapp", "+79991234567")), None);
        assert_eq!(view(&link("whatsapp", "0123456789")), None);
    }

    #[test]
    fn discord() {
        assert_eq!(norm("discord", "Some.User_1").unwrap(), "some.user_1");
        assert_eq!(norm("discord", "@someuser").unwrap(), "someuser");
        assert_eq!(norm("discord", "80351110224678912").unwrap(), "80351110224678912");
        assert_eq!(norm("discord", "https://discord.com/users/80351110224678912").unwrap(), "80351110224678912");
        assert_eq!(norm("discord", "https://discordapp.com/users/80351110224678912").unwrap(), "80351110224678912");
        assert_eq!(norm("discord", "https://discord.gg/abc"), Err("social_bad_handle"));
        assert_eq!(norm("discord", "user#1234"), Err("social_bad_handle"));
        assert_eq!(norm("discord", "a"), Err("social_bad_handle"));
        assert_eq!(norm("discord", "some..user"), Err("social_bad_handle"));
        assert_eq!(norm("discord", "some-user"), Err("social_bad_handle"));
        let v = view(&link("discord", "someuser")).unwrap();
        assert_eq!(v.url, "");
        assert_eq!(v.handle, "someuser");
        assert_eq!(url_of("discord", "80351110224678912"), "https://discord.com/users/80351110224678912");
    }

    #[test]
    fn twitch() {
        assert_eq!(norm("twitch", "ninja").unwrap(), "ninja");
        assert_eq!(norm("twitch", "https://www.twitch.tv/ninja").unwrap(), "ninja");
        assert_eq!(norm("twitch", "https://m.twitch.tv/ninja/").unwrap(), "ninja");
        assert_eq!(norm("twitch", "https://twitch.tv/videos/1"), Err("social_bad_handle"));
        assert_eq!(norm("twitch", "ni"), Err("social_bad_handle"));
        assert_eq!(norm("twitch", "nin.ja"), Err("social_bad_handle"));
        assert_eq!(url_of("twitch", "ninja"), "https://twitch.tv/ninja");
    }

    #[test]
    fn mastodon() {
        assert_eq!(norm("mastodon", "@Gargron@Mastodon.Social").unwrap(), "Gargron@mastodon.social");
        assert_eq!(norm("mastodon", "gargron@mastodon.social").unwrap(), "gargron@mastodon.social");
        assert_eq!(norm("mastodon", "https://mastodon.social/@Gargron").unwrap(), "Gargron@mastodon.social");
        assert_eq!(norm("mastodon", "fosstodon.org/@a.b-c/").unwrap(), "a.b-c@fosstodon.org");
        assert_eq!(norm("mastodon", "gargron"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "@gargron"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@localhost"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@127.0.0.1"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@[::1]"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@evil.com:8080"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@evil.com/x"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "a@b@evil.com"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "gargron@-evil.com"), Err("social_bad_handle"));
        // Hosts a browser reads as an IPv4 address.
        for host in ["127.0x1", "127.0.0.0x1", "0x7f.0x1", "10.0.0.0x1", "169.254.169.0xfe", "1.0X", "a.0177"] {
            assert_eq!(norm("mastodon", &format!("gargron@{host}")), Err("social_bad_handle"), "{host}");
            assert_eq!(norm("mastodon", &format!("https://{host}/@gargron")), Err("social_bad_handle"), "{host}");
            assert_eq!(view(&link("mastodon", &format!("gargron@{host}"))), None, "{host}");
        }
        assert_eq!(view(&link("mastodon", "a@169.254.169.0xfe")), None);
        // A name that only starts like a hex number is a name.
        assert_eq!(norm("mastodon", "gargron@0x.social").unwrap(), "gargron@0x.social");
        assert_eq!(norm("mastodon", "gargron@m.0xg").unwrap(), "gargron@m.0xg");
        assert_eq!(norm("mastodon", "gargron@xn--mstdn-n1a.social").unwrap(), "gargron@xn--mstdn-n1a.social");
        assert_eq!(norm("mastodon", "gargron@mаstodon.social"), Err("social_bad_handle")); // Cyrillic а
        assert_eq!(norm("mastodon", "https://user@mastodon.social/@gargron"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "https://mastodon.social/gargron"), Err("social_bad_handle"));
        assert_eq!(norm("mastodon", "https://mastodon.social/@gargron/1234"), Err("social_bad_handle"));
        let v = view(&link("mastodon", "Gargron@mastodon.social")).unwrap();
        assert_eq!(v.url, "https://mastodon.social/@Gargron");
        assert_eq!(v.handle, "@Gargron@mastodon.social");
        assert_eq!(view(&link("mastodon", "@gargron@mastodon.social")), None);
    }

    #[test]
    fn bluesky() {
        assert_eq!(norm("bluesky", "@Jay.bsky.team").unwrap(), "jay.bsky.team");
        assert_eq!(norm("bluesky", "alice").unwrap(), "alice.bsky.social");
        assert_eq!(norm("bluesky", "alice.example.com").unwrap(), "alice.example.com");
        assert_eq!(norm("bluesky", "https://bsky.app/profile/jay.bsky.team").unwrap(), "jay.bsky.team");
        let did = "did:plc:z72i7hdynmk6r22z27h6tvur";
        assert_eq!(norm("bluesky", did).unwrap(), did);
        assert_eq!(norm("bluesky", &format!("https://bsky.app/profile/{did}")).unwrap(), did);
        assert_eq!(norm("bluesky", "did:plc:short"), Err("social_bad_handle"));
        assert_eq!(norm("bluesky", "did:web:evil.com"), Err("social_bad_handle"));
        assert_eq!(norm("bluesky", "alice_b"), Err("social_bad_handle"));
        assert_eq!(norm("bluesky", "1.2.3.4"), Err("social_bad_handle"));
        assert_eq!(norm("bluesky", "https://bsky.app/profile/jay.bsky.team/post/1"), Err("social_bad_handle"));
        let v = view(&link("bluesky", "jay.bsky.team")).unwrap();
        assert_eq!(v.url, "https://bsky.app/profile/jay.bsky.team");
        assert_eq!(v.handle, "@jay.bsky.team");
        assert_eq!(view(&link("bluesky", "alice")), None);
    }

    #[test]
    fn threads() {
        assert_eq!(norm("threads", "@zuck").unwrap(), "zuck");
        assert_eq!(norm("threads", "https://www.threads.net/@zuck").unwrap(), "zuck");
        assert_eq!(norm("threads", "https://www.threads.com/@zuck?hl=en").unwrap(), "zuck");
        assert_eq!(norm("threads", "https://www.threads.net/zuck"), Err("social_bad_handle"));
        assert_eq!(url_of("threads", "zuck"), "https://threads.net/@zuck");
        assert_eq!(view(&link("threads", "zuck")).unwrap().handle, "@zuck");
    }

    #[test]
    fn reddit() {
        assert_eq!(norm("reddit", "spez").unwrap(), "spez");
        assert_eq!(norm("reddit", "u/spez").unwrap(), "spez");
        assert_eq!(norm("reddit", "/u/spez").unwrap(), "spez");
        assert_eq!(norm("reddit", "U/spez").unwrap(), "spez");
        assert_eq!(norm("reddit", "https://www.reddit.com/user/spez/").unwrap(), "spez");
        assert_eq!(norm("reddit", "https://old.reddit.com/u/spez").unwrap(), "spez");
        assert_eq!(norm("reddit", "https://reddit.com/r/rust"), Err("social_bad_handle"));
        assert_eq!(norm("reddit", "u/sp"), Err("social_bad_handle"));
        assert_eq!(norm("reddit", "u/spez/x"), Err("social_bad_handle"));
        assert_eq!(norm("reddit", "u/../evil"), Err("social_bad_handle"));
        let v = view(&link("reddit", "spez")).unwrap();
        assert_eq!(v.url, "https://reddit.com/user/spez");
        assert_eq!(v.handle, "u/spez");
    }

    #[test]
    fn other() {
        assert_eq!(norm("other", "https://example.com").unwrap(), "https://example.com");
        assert_eq!(norm("other", "HTTPS://Example.COM/Path?q=1#Top").unwrap(), "https://example.com/Path?q=1#Top");
        assert_eq!(norm("other", "example.com/me").unwrap(), "https://example.com/me");
        assert_eq!(norm("other", "https://example.com:8443/x").unwrap(), "https://example.com:8443/x");
        assert_eq!(norm("other", "https://ru.wikipedia.org/wiki/Тест").unwrap(), "https://ru.wikipedia.org/wiki/Тест");
        for bad in [
            "http://example.com",
            "ftp://example.com",
            "javascript:alert(1)",
            "data:text/html,x",
            "https://",
            "https:///path",
            "https://user@example.com",
            "https://user:pass@example.com/",
            "https://example.com@evil.com",
            "https://localhost/",
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://exa mple.com",
            "https://example.com/a b",
            "https://example.com/\u{202e}fdp.exe",
            "https://example.com/a\u{200b}",
            "https://example.com\\@evil.com",
            "https://exаmple.com", // Cyrillic а
            "https://example.com:0/",
            "https://example.com:99999/",
            "https://example.com:x/",
            "https://example.com/<script>",
            "https://-example.com",
            "https://example..com",
            "https://example.com./",
            "https://192.168.1.0x1/",
            "https://127.0x1/",
            "https://127.0.0.0x1/",
            "https://0x7f.0x1:8080/admin",
            "https://10.0.0.0x1/",
            "https://169.254.169.0XFE/",
            "https://10.0.0.010/",
        ] {
            assert_eq!(norm("other", bad), Err("social_bad_handle"), "{bad:?}");
            assert_eq!(view(&link("other", bad)), None, "{bad:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_OTHER_URL_BYTES));
        assert_eq!(norm("other", &long), Err("social_bad_handle"));
        let fits = format!("https://example.com/{}", "a".repeat(MAX_OTHER_URL_BYTES - 20));
        assert_eq!(fits.len(), MAX_OTHER_URL_BYTES);
        assert_eq!(norm("other", &fits).unwrap(), fits);

        // Typed without a scheme: the same IP hosts are refused.
        assert_eq!(norm("other", "0x7f.0x1:8080/admin"), Err("social_bad_handle"));
        assert_eq!(norm("other", "127.0.0.0x1"), Err("social_bad_handle"));

        // Invisible characters, whatever their kind, anywhere in the address.
        for c in [
            '\u{00ad}', '\u{034f}', '\u{0600}', '\u{061c}', '\u{06dd}', '\u{070f}', '\u{08e2}', '\u{115f}',
            '\u{1160}', '\u{17b4}', '\u{17b5}', '\u{180e}', '\u{200b}', '\u{200d}', '\u{2060}', '\u{2064}',
            '\u{206a}', '\u{206f}', '\u{3164}', '\u{feff}', '\u{ffa0}', '\u{fff9}', '\u{110bd}', '\u{1d173}',
            '\u{e0001}', '\u{e0041}',
        ] {
            let u = format!("https://example.com/{c}x");
            assert_eq!(norm("other", &u), Err("social_bad_handle"), "{:04x}", c as u32);
            assert_eq!(view(&link("other", &u)), None, "{:04x}", c as u32);
        }

        // A typed address may carry another address in its query.
        assert_eq!(norm("other", "example.com/?r=https://x.com").unwrap(), "https://example.com/?r=https://x.com");
        assert_eq!(norm("other", "example.com#https://x.com").unwrap(), "https://example.com#https://x.com");
        assert_eq!(norm("other", "example.com:8443/a?u=ftp://x").unwrap(), "https://example.com:8443/a?u=ftp://x");
        // But no other scheme in front.
        for bad in [
            "ftp://example.com/?r=https://x.com",
            "wss://example.com",
            "HTTP://example.com/?a=b",
            "x-y://example.com",
        ] {
            assert_eq!(norm("other", bad), Err("social_bad_handle"), "{bad}");
        }

        // Received data must be a whole https address: no scheme added.
        assert_eq!(view(&link("other", "example.com/me")), None);
        let v = view(&link("other", "https://example.com/me")).unwrap();
        assert_eq!(v.url, "https://example.com/me");
        assert_eq!(v.handle, "example.com/me");
        assert_eq!(v.name, "Link");
    }

    #[test]
    fn foreign_hosts_are_refused_for_every_platform() {
        for p in PLATFORMS {
            if matches!(p.check, Check::Mastodon | Check::Other) {
                continue;
            }
            for t in p.paths {
                let path = t.replace("{h}", "validname1");
                for host in ["evil.com", "notgithub.com", "t.me.evil.com", "evil.com/t.me"] {
                    let u = format!("https://{host}{path}");
                    assert_eq!(normalize_link(p.id, &u), Err("social_bad_handle"), "{} {u}", p.id);
                }
                for host in p.hosts {
                    let u = format!("https://{host}.evil.com{path}");
                    assert_eq!(normalize_link(p.id, &u), Err("social_bad_handle"), "{} {u}", p.id);
                    let u = format!("https://evil.com@{host}{path}");
                    assert_eq!(normalize_link(p.id, &u), Err("social_bad_handle"), "{} {u}", p.id);
                }
            }
        }
    }

    #[test]
    fn normalized_is_a_fixed_point_and_views() {
        let typed = [
            ("telegram", "@durov"),
            ("instagram", "https://instagram.com/natgeo"),
            ("tiktok", "@khaby.lame"),
            ("x", "https://twitter.com/jack"),
            ("youtube", "@MrBeast"),
            ("vk", "id1"),
            ("facebook", "zuckerberg"),
            ("linkedin", "https://linkedin.com/in/john-doe"),
            ("github", "octocat"),
            ("whatsapp", "+1 555 123 4567"),
            ("discord", "Some.User"),
            ("twitch", "ninja"),
            ("mastodon", "@Gargron@Mastodon.Social"),
            ("bluesky", "alice"),
            ("threads", "@zuck"),
            ("reddit", "u/spez"),
            ("other", "Example.com/me"),
        ];
        for (p, s) in typed {
            let l = normalize_link(p, s).unwrap();
            assert_eq!(normalize_link(p, &l.h).unwrap(), l, "{p}");
            assert!(view(&l).is_some(), "{p}");
            assert_eq!(clean_list(std::slice::from_ref(&l)), vec![l.clone()], "{p}");
        }
    }

    #[test]
    fn clean_list_filters_dedups_and_caps() {
        let list = vec![
            link("github", "octocat"),
            link("github", "OctoCat"),
            link("telegram", "durov"),
            link("myspace", "tom"),
            link("github", "../evil"),
            link("x", "octocat"),
            link("other", "https://example.com/A"),
            link("other", "https://example.com/a"),
            link("other", "HTTPS://EXAMPLE.COM/a"),
            link("discord", "SomeUser"),
            link("discord", "someuser"),
        ];
        let got = clean_list(&list);
        assert_eq!(
            got,
            vec![
                link("github", "octocat"),
                link("telegram", "durov"),
                link("x", "octocat"),
                link("other", "https://example.com/A"),
                link("other", "https://example.com/a"),
                link("discord", "someuser"),
            ]
        );

        let many: Vec<SocialLink> = (0..40).map(|i| link("github", &format!("user{i}"))).collect();
        let got = clean_list(&many);
        assert_eq!(got.len(), MAX_SOCIALS);
        assert_eq!(got[0].h, "user0");
        assert_eq!(got[MAX_SOCIALS - 1].h, format!("user{}", MAX_SOCIALS - 1));
        assert!(clean_list(&[]).is_empty());
    }

    #[test]
    fn list_from_json_skips_malformed_elements() {
        let v = serde_json::json!([
            {"p": "github", "h": "octocat"},
            {"p": "github"},
            {"p": 1, "h": "x"},
            "github:octocat",
            null,
            {"p": "telegram", "h": "durov", "extra": true},
            {"p": "x", "h": "a?b"}
        ]);
        assert_eq!(list_from_json(&v), vec![link("github", "octocat"), link("telegram", "durov")]);
        assert!(list_from_json(&serde_json::json!({"p": "github", "h": "x"})).is_empty());
        assert!(list_from_json(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn wire_form() {
        let l = link("github", "octocat");
        let s = serde_json::to_string(&l).unwrap();
        assert_eq!(s, r#"{"p":"github","h":"octocat"}"#);
        assert_eq!(serde_json::from_str::<SocialLink>(&s).unwrap(), l);
        let v = serde_json::to_value(view(&l).unwrap()).unwrap();
        assert_eq!(v["platform"], "github");
        assert_eq!(v["url"], "https://github.com/octocat");
        let p = serde_json::to_value(&platforms()[0]).unwrap();
        assert_eq!(p, serde_json::json!({"id": "telegram", "name": "Telegram", "hint": "@username"}));
    }
}

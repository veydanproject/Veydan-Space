// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The grammar: `veydan://<type>/<id>?<key>=<value>&…`.
//!
//! - the type is 1..=32 small latin letters;
//! - the identifier is 1..=128 characters of `A-Z a-z 0-9 - _ . ~`;
//! - a value is percent-encoded UTF-8; a list is values joined by commas;
//! - a key may appear once; a pair without `=` says nothing and is skipped;
//! - keys a type does not know are skipped by that type (newer clients
//!   may add them).

use crate::error::LinkError;
use crate::kind::LinkType;

pub const SCHEME: &str = "veydan://";
pub const MAX_LINK_LEN: usize = 2048;
const MAX_TYPE_LEN: usize = 32;
const MAX_ID_LEN: usize = 128;

fn unreserved(b: u8) -> bool {
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
}

pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A link taken apart. Values are kept as written; they are decoded when read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uri {
    link_type: LinkType,
    id: String,
    params: Vec<(String, String)>,
}

impl Uri {
    pub fn parse(link: &str) -> Result<Self, LinkError> {
        let s = link.trim();
        if s.len() > MAX_LINK_LEN {
            return Err(LinkError::TooLong);
        }
        let rest = s.strip_prefix(SCHEME).ok_or(LinkError::Scheme)?;
        if rest.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(LinkError::Characters);
        }
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rest, ""),
        };
        let (word, id) = path.split_once('/').ok_or(LinkError::Type)?;
        // A type is a word of small latin letters; two words are joined
        // by a hyphen (`call-node`), never at an end or doubled.
        if word.is_empty()
            || word.len() > MAX_TYPE_LEN
            || !word.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            || word.starts_with('-')
            || word.ends_with('-')
            || word.contains("--")
        {
            return Err(LinkError::Type);
        }
        if id.is_empty() || id.len() > MAX_ID_LEN || !id.bytes().all(unreserved) {
            return Err(LinkError::Id);
        }
        let mut params: Vec<(String, String)> = Vec::new();
        for pair in query.split('&') {
            let Some((k, v)) = pair.split_once('=') else { continue };
            if k.is_empty() || params.iter().any(|(known, _)| known == k) {
                return Err(LinkError::Param);
            }
            params.push((k.to_string(), v.to_string()));
        }
        Ok(Self { link_type: LinkType::of(word), id: id.to_string(), params })
    }

    pub fn link_type(&self) -> &LinkType {
        &self.link_type
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The value as written, for values that are never encoded (keys, numbers).
    pub fn raw(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// `Ok(None)`: the key is absent. `Err`: it is there and badly encoded.
    pub fn text(&self, key: &str) -> Result<Option<String>, LinkError> {
        match self.raw(key) {
            None => Ok(None),
            Some(v) => decode(v).map(Some).ok_or(LinkError::Param),
        }
    }

    /// Values of a list; an absent key is an empty list.
    pub fn list(&self, key: &str) -> Result<Vec<String>, LinkError> {
        match self.raw(key) {
            None => Ok(Vec::new()),
            Some(v) => v.split(',').filter(|x| !x.is_empty()).map(|x| decode(x).ok_or(LinkError::Param)).collect(),
        }
    }
}

/// Puts a link together; what it writes, `Uri::parse` reads.
pub struct UriBuilder {
    out: String,
    first: bool,
}

impl UriBuilder {
    /// `id` must be of the characters an identifier may have.
    pub fn new(link_type: &LinkType, id: &str) -> Self {
        debug_assert!(!id.is_empty() && id.bytes().all(unreserved), "an identifier is written as it is");
        Self { out: format!("{SCHEME}{}/{id}", link_type.as_str()), first: true }
    }

    fn key(&mut self, key: &str) {
        self.out.push(if self.first { '?' } else { '&' });
        self.first = false;
        self.out.push_str(key);
        self.out.push('=');
    }

    pub fn param(mut self, key: &str, value: &str) -> Self {
        self.key(key);
        self.out.push_str(&encode(value));
        self
    }

    /// Nothing is written for an empty list.
    pub fn list<'a>(mut self, key: &str, values: impl IntoIterator<Item = &'a str>) -> Self {
        let items: Vec<String> = values.into_iter().map(encode).collect();
        if !items.is_empty() {
            self.key(key);
            self.out.push_str(&items.join(","));
        }
        self
    }

    pub fn build(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_link_apart() {
        let u = Uri::parse("  veydan://group/abc-1?t=public&n=%D0%9A%D0%BB%D1%83%D0%B1%20%26%20co&m=a,b%2Cc&flag&future=1 ").unwrap();
        assert_eq!(u.link_type(), &LinkType::Group);
        assert_eq!(u.id(), "abc-1");
        assert_eq!(u.raw("t"), Some("public"));
        assert_eq!(u.text("n").unwrap().as_deref(), Some("Клуб & co"));
        assert_eq!(u.list("m").unwrap(), vec!["a", "b,c"]);
        assert_eq!(u.raw("flag"), None, "a pair without a value says nothing");
        assert_eq!(u.text("absent").unwrap(), None);
        assert!(u.list("absent").unwrap().is_empty());

        let bare = Uri::parse("veydan://contact/x").unwrap();
        assert_eq!(bare.link_type(), &LinkType::Contact);
        assert_eq!(bare.raw("n"), None);
    }

    #[test]
    fn a_type_of_tomorrow_is_still_a_link() {
        let u = Uri::parse("veydan://channel/42?x=1").unwrap();
        assert_eq!(u.link_type(), &LinkType::Unknown("channel".into()));
        assert_eq!(u.link_type().as_str(), "channel");
    }

    #[test]
    fn refusals() {
        let long = format!("veydan://group/a?n={}", "x".repeat(MAX_LINK_LEN));
        for (text, why) in [
            ("https://example.com", LinkError::Scheme),
            ("VEYDAN://group/a", LinkError::Scheme),
            ("veydan:group/a", LinkError::Scheme),
            ("javascript:alert(1)", LinkError::Scheme),
            ("", LinkError::Scheme),
            (long.as_str(), LinkError::TooLong),
            ("veydan://group/a b", LinkError::Characters),
            ("veydan://group/a?n=x\ny", LinkError::Characters),
            ("veydan://group/a?n=x\u{0}", LinkError::Characters),
            ("veydan://group", LinkError::Type),
            ("veydan:///a", LinkError::Type),
            ("veydan://Group/a", LinkError::Type),
            ("veydan://gr0up/a", LinkError::Type),
            ("veydan://group/", LinkError::Id),
            ("veydan://group/a/b", LinkError::Id),
            ("veydan://group/a%20b", LinkError::Id),
            ("veydan://group/a?t=1&t=2", LinkError::Param),
            ("veydan://group/a?=1", LinkError::Param),
        ] {
            assert_eq!(Uri::parse(text), Err(why), "{text:?}");
        }
        let id = "a".repeat(MAX_ID_LEN + 1);
        assert_eq!(Uri::parse(&format!("veydan://group/{id}")), Err(LinkError::Id));
    }

    #[test]
    fn a_bad_value_is_refused_when_read() {
        let u = Uri::parse("veydan://group/a?n=%ZZ&k=%E0%A4&m=ok,%4").unwrap();
        assert_eq!(u.text("n"), Err(LinkError::Param));
        assert_eq!(u.text("k"), Err(LinkError::Param), "not UTF-8");
        assert_eq!(u.list("m"), Err(LinkError::Param));
    }

    #[test]
    fn what_is_written_is_read() {
        let name = "Клуб & друзья / 2026 = ? # %";
        let text = UriBuilder::new(&LinkType::Group, "abc")
            .param("n", name)
            .list("m", ["one", "t,w&o"])
            .list("none", [])
            .param("e", "3")
            .build();
        assert!(text.starts_with("veydan://group/abc?n="));
        assert!(!text.contains(' ') && !text.contains("none"));
        let u = Uri::parse(&text).unwrap();
        assert_eq!(u.text("n").unwrap().as_deref(), Some(name));
        assert_eq!(u.list("m").unwrap(), vec!["one", "t,w&o"]);
        assert_eq!(u.raw("e"), Some("3"));

        assert_eq!(UriBuilder::new(&LinkType::Contact, "x").build(), "veydan://contact/x");
    }
}

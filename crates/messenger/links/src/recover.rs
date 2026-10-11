// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The link a person who lost everything sends to a trusted person
//! (internal/cloud-recovery.md §8.5): `veydan://recover/<npub>?c=<code>`.
//!
//! The id is the key of the account to recover, 64 hex or `npub1…`; `c` is
//! the check code of the request, 12 Crockford characters in groups of
//! four (`AB7K-2QZD-9MXP`), the first 60 bits of the sha256 of the
//! temporary key of the request. The code is not a secret: the trusted
//! person's app computes the same code from what the server says of the
//! request and refuses to confirm when the two differ.

use crate::contact::pubkey_of_npub;
use crate::error::LinkError;
use crate::kind::LinkType;
use crate::uri::{Uri, UriBuilder};
use messenger_core::PubKey;

/// The characters of a code, hyphens aside.
const CODE_CHARS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverLink {
    /// The account to recover.
    pub npub: PubKey,
    /// The check code as it is shown: upper case, hyphens between the groups.
    pub code: String,
}

/// The code in the form it is shown: upper case, the groups of four joined
/// by hyphens; `None` for anything that is not 12 Crockford characters.
pub fn normalize_code(typed: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_CHARS + 2);
    let mut n = 0;
    for ch in typed.chars() {
        if ch.is_whitespace() || ch == '-' {
            continue;
        }
        let up = ch.to_ascii_uppercase();
        if !(up.is_ascii_digit() || up.is_ascii_uppercase()) {
            return None;
        }
        if n > 0 && n % 4 == 0 {
            out.push('-');
        }
        out.push(up);
        n += 1;
    }
    (n == CODE_CHARS).then_some(out)
}

impl RecoverLink {
    pub fn encode(&self) -> String {
        UriBuilder::new(&LinkType::Recover, self.npub.as_hex()).param("c", &self.code).build()
    }

    pub fn parse(link: &str) -> Result<Self, LinkError> {
        Self::from_uri(&Uri::parse(link)?)
    }

    pub fn from_uri(uri: &Uri) -> Result<Self, LinkError> {
        if uri.link_type() != &LinkType::Recover {
            return Err(LinkError::Type);
        }
        let npub = PubKey::parse(uri.id()).or_else(|| pubkey_of_npub(uri.id())).ok_or(LinkError::Id)?;
        let code = uri.text("c")?.ok_or(LinkError::Missing)?;
        let code = normalize_code(&code).ok_or(LinkError::Param)?;
        Ok(Self { npub, code })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPUB: &str = "3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f";

    #[test]
    fn a_link_is_written_and_read() {
        let link = RecoverLink { npub: PubKey::parse(NPUB).unwrap(), code: "AB7K-2QZD-9MXP".into() };
        let text = link.encode();
        assert_eq!(text, format!("veydan://recover/{NPUB}?c=AB7K-2QZD-9MXP"));
        assert_eq!(RecoverLink::parse(&text).unwrap(), link);
        // The code in any case and spacing; the account as npub1….
        let typed = format!("veydan://recover/{NPUB}?c=ab7k2qzd%209mxp");
        assert_eq!(RecoverLink::parse(&typed).unwrap().code, "AB7K-2QZD-9MXP");
        let bech = crate::contact::npub_of(&link.npub).unwrap();
        assert_eq!(RecoverLink::parse(&format!("veydan://recover/{bech}?c=AB7K-2QZD-9MXP")).unwrap().npub, link.npub);
    }

    #[test]
    fn what_is_not_a_recover_link_is_refused() {
        assert_eq!(RecoverLink::parse(&format!("veydan://contact/{NPUB}?c=AB7K-2QZD-9MXP")), Err(LinkError::Type));
        assert_eq!(RecoverLink::parse("veydan://recover/nobody?c=AB7K-2QZD-9MXP"), Err(LinkError::Id));
        assert_eq!(RecoverLink::parse(&format!("veydan://recover/{NPUB}")), Err(LinkError::Missing));
        assert_eq!(RecoverLink::parse(&format!("veydan://recover/{NPUB}?c=AB7K")), Err(LinkError::Param));
        assert_eq!(RecoverLink::parse(&format!("veydan://recover/{NPUB}?c=AB7K-2QZD-9MX%2B")), Err(LinkError::Param));
        assert_eq!(normalize_code(" ab7k 2qzd 9mxp "), Some("AB7K-2QZD-9MXP".into()));
        assert_eq!(normalize_code("AB7K-2QZD"), None);
    }
}

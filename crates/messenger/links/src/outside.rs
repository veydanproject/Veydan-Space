// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A link the system hands to the app: a `veydan://…` clicked in a
//! browser, a mail or another app (internal/messenger-spec.md, «Ссылка из
//! системы»). Anybody can put such a link on a page, so it opens a card and
//! never does anything by itself; and some types are not opened from
//! outside at all.

use crate::error::LinkError;
use crate::uri::{MAX_LINK_LEN, SCHEME};

/// Types opened only from the app's own scanner or a paste, never from a
/// link the system hands over: a sign-in to the web cabinet (`web`: any
/// site would put its own sign-in up for approval), a recovery through the
/// trusted people (`recover`) and the linking of a device (`link`).
pub const SCANNER_ONLY: &[&str] = &["web", "recover", "link"];

/// The link to read, when the system may hand it over: `veydan://` only
/// (no bare `npub1…`), at most `MAX_LINK_LEN` bytes, and of no type of
/// `SCANNER_ONLY`. The rest of the grammar is `Uri::parse`'s.
pub fn from_outside(text: &str) -> Result<&str, LinkError> {
    let s = text.trim();
    if s.len() > MAX_LINK_LEN {
        return Err(LinkError::TooLong);
    }
    let rest = s.strip_prefix(SCHEME).ok_or(LinkError::Scheme)?;
    let word = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if SCANNER_ONLY.contains(&word) {
        return Err(LinkError::ScannerOnly);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NPUB: &str = "npub1sg6plzptd64u62a878hep2kev88swjh3tw00gjsfl8f237lmu63q0uf63m";

    #[test]
    fn the_types_of_cards_are_let_through() {
        for link in [
            format!("veydan://contact/{NPUB}"),
            "veydan://group/abc?k=1".to_string(),
            "veydan://channel/abc?s=1".to_string(),
            "veydan://server/abc".to_string(),
            "veydan://vlink/abc".to_string(),
            "veydan://call-node/abc".to_string(),
            // A type this version does not know is read as unknown, not refused.
            "veydan://future/abc".to_string(),
        ] {
            assert_eq!(from_outside(&link), Ok(link.as_str()), "{link}");
        }
    }

    #[test]
    fn a_sign_in_a_recovery_and_a_device_link_are_refused() {
        for link in [
            "veydan://web/0123456789abcdef",
            "veydan://recover/abc?c=AB7K-2QZD-9MXP",
            "veydan://link/abcd/abandon-ability",
            "veydan://web?x=1",
            "veydan://web",
            " veydan://link/x ",
        ] {
            assert_eq!(from_outside(link), Err(LinkError::ScannerOnly), "{link}");
        }
        assert_eq!(LinkError::ScannerOnly.code(), "link_scanner_only");
    }

    #[test]
    fn a_word_that_only_begins_like_a_refused_type_is_not_one() {
        // Read further by `Uri::parse`: `webx` is unknown, `Web` is no type at all.
        assert!(from_outside("veydan://webx/a").is_ok());
        assert!(from_outside("veydan://Web/a").is_ok());
    }

    #[test]
    fn only_a_link_of_the_scheme_comes_from_outside() {
        assert_eq!(from_outside(NPUB), Err(LinkError::Scheme));
        assert_eq!(from_outside(&format!("nostr:{NPUB}")), Err(LinkError::Scheme));
        assert_eq!(from_outside("https://veydan.net/"), Err(LinkError::Scheme));
        assert_eq!(from_outside(""), Err(LinkError::Scheme));
    }

    #[test]
    fn a_link_longer_than_a_link_may_be_is_refused_before_it_is_read() {
        let at_most = format!("veydan://group/a?n={}", "x".repeat(MAX_LINK_LEN - "veydan://group/a?n=".len()));
        assert_eq!(at_most.len(), MAX_LINK_LEN);
        assert!(from_outside(&at_most).is_ok());
        let longer = format!("{at_most}x");
        assert_eq!(from_outside(&longer), Err(LinkError::TooLong));
        // Refused for its length even when its type is one of the refused.
        assert_eq!(from_outside(&format!("veydan://web/{}", "a".repeat(MAX_LINK_LEN))), Err(LinkError::TooLong));
    }
}

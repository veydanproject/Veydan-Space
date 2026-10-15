// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the snapshot of my relations lives on the relays
//! (internal/messenger-wire.md §3, "Снимок отношений"). Here, not in the
//! DM crate, because ingress classifies the events and must not depend on
//! the crate that applies them.

/// Kind of the snapshot: application data (NIP-78), addressable, so a
/// relay keeps one event per key and `d`: the newest list, nothing older.
pub const KIND_RELATIONS: u16 = 30078;

/// The `d` of the header. The pages after it are `<header>.<n>`.
pub const RELATIONS_D: &str = "veydan.relations.v1";

/// A page carries this many peers: NIP-44 v2 takes 65535 bytes of plain
/// text at most, and an entry is under a hundred.
pub const RELATIONS_PEERS_PER_PAGE: usize = 500;

/// The header and the pages after it are this many at most; what does
/// not fit is left out and the header says `complete: false`.
pub const RELATIONS_MAX_PAGES: usize = 16;

/// The `d` of page `n`: the header for 0, `veydan.relations.v1.<n>`
/// after it.
pub fn relations_d(page: usize) -> String {
    if page == 0 {
        RELATIONS_D.to_string()
    } else {
        format!("{RELATIONS_D}.{page}")
    }
}

/// Which page a `d` names: 0 for the header, 1… for a page; `None` for
/// a `d` that is not of the snapshot (a link code lives under the same
/// kind).
pub fn relations_page(d: &str) -> Option<usize> {
    if d == RELATIONS_D {
        return Some(0);
    }
    let n = d.strip_prefix(RELATIONS_D)?.strip_prefix('.')?;
    if n.is_empty() || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = n.parse().ok()?;
    (n < RELATIONS_MAX_PAGES).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pages_are_named_after_the_header_and_bounded() {
        assert_eq!(relations_d(0), "veydan.relations.v1");
        assert_eq!(relations_d(3), "veydan.relations.v1.3");
        for n in 0..RELATIONS_MAX_PAGES {
            assert_eq!(relations_page(&relations_d(n)), Some(n));
        }
        assert_eq!(relations_page("veydan.relations.v1.16"), None, "past the last page");
        assert_eq!(relations_page("veydan.relations.v1.0"), None, "the header is not page zero");
        assert_eq!(relations_page("veydan.relations.v1.01"), None);
        assert_eq!(relations_page("veydan.relations.v1."), None);
        assert_eq!(relations_page("veydan.relations.v2"), None);
        assert_eq!(relations_page("veydan"), None);
        assert_eq!(relations_page(&"ab".repeat(32)), None, "a link code");
    }
}

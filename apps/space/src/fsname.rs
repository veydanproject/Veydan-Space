// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! One name of a path, as every desktop can write it.

use std::path::Path;

/// Exactly one normal component on this system. `:` (a drive, `C:x`, or a
/// stream, `a:b`), `\`, the characters Windows forbids, control characters, a
/// trailing dot or space and the device names (`CON`, `nul.txt`, `COM1`) are
/// refused on every system, so a file is written alike everywhere.
pub(crate) fn valid_component(c: &str) -> bool {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    let stem = c.split('.').next().unwrap_or("").trim_end_matches(' ');
    let b = stem.as_bytes();
    let device = RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r))
        || (b.len() == 4
            && (b[..3].eq_ignore_ascii_case(b"COM") || b[..3].eq_ignore_ascii_case(b"LPT"))
            && matches!(b[3], b'1'..=b'9'));
    let mut parts = Path::new(c).components();
    !c.is_empty()
        && c != "."
        && c != ".."
        && !c.ends_with(['.', ' '])
        && !c.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        && !c.chars().any(char::is_control)
        && !device
        && matches!(
            (parts.next(), parts.next()),
            (Some(std::path::Component::Normal(_)), None)
        )
}

#[cfg(test)]
mod tests {
    use super::valid_component;

    #[test]
    fn names_windows_cannot_write_are_refused() {
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "C:x",
            "a:b",
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "a\u{1}b",
            "a\0b",
            "name.",
            "name ",
            "CON",
            "con",
            "NUL.txt",
            "Aux.tar.gz",
            "COM1",
            "lpt9.log",
            "PRN ",
        ] {
            assert!(!valid_component(bad), "{bad:?} must be refused");
        }
        for good in [
            "a", "file.txt", ".hidden", "COM0", "COM10", "CONSOLE", "a b", "ä.txt",
        ] {
            assert!(valid_component(good), "{good:?} must pass");
        }
    }
}

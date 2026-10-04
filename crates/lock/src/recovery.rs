// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Recovery code: 24 Crockford base32 chars (120 bits), shown as 6 groups of 4.

use rand::RngExt;

const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const CODE_LEN: usize = 24;
const GROUP: usize = 4;

/// Fresh random code in display form, e.g. `A1B2-C3D4-...`.
pub fn generate() -> String {
    let mut rng = rand::rng();
    let raw: String = (0..CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format_groups(&raw)
}

/// Canonical form for the KDF: uppercase, no separators, look-alikes folded.
/// Returns None when the result is not exactly 24 alphabet chars.
pub fn normalize(input: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for ch in input.chars() {
        if ch.is_whitespace() || ch == '-' {
            continue;
        }
        let ch = match ch.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'U' => 'V',
            c => c,
        };
        if !ALPHABET.contains(&(ch as u8)) {
            return None;
        }
        out.push(ch);
    }
    (out.len() == CODE_LEN).then_some(out)
}

fn format_groups(raw: &str) -> String {
    raw.as_bytes()
        .chunks(GROUP)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_normalizes_to_itself() {
        let code = generate();
        assert_eq!(code.len(), CODE_LEN + CODE_LEN / GROUP - 1);
        assert_eq!(normalize(&code).unwrap(), code.replace('-', ""));
    }

    #[test]
    fn normalize_folds_lookalikes() {
        let typed = "abcd efgh jkmn pqrs tvwx yz0o";
        assert_eq!(normalize(typed).unwrap(), "ABCDEFGHJKMNPQRSTVWXYZ00");
        assert!(normalize("too-short").is_none());
    }
}

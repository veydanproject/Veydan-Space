// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a person types to recover the lock, in the two forms the key row
//! knows (platform-spec 8.1, cloud-recovery.md §3):
//!
//! - version 1: a code of 24 Crockford base32 chars (120 bits), shown as
//!   6 groups of 4; the canonical form is the input of the KDF;
//! - version 2: the Recovery Key, 16 random bytes shown as 12 BIP-39 words;
//!   the input of the KDF is the hex of `HKDF-SHA256(entropy, salt
//!   "veydan-recovery", info "veydan-recovery/lock-wrap/1")`, the same
//!   derivation `crates/recovery` makes (`RecoveryKey::lock_wrap_input`),
//!   fixed by the golden vector of that crate (`tests/recovery_golden.rs`).

use bip39::{Language, Mnemonic};
use hkdf::Hkdf;
use rand::Rng;
#[cfg(test)]
use rand::RngExt;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The length of a code of version 1.
pub const CODE_LEN: usize = 24;
#[cfg(test)]
const GROUP: usize = 4;

/// The second wrap is made from a 24-char code.
pub const VERSION_CODE: u8 = 1;
/// The second wrap is made from the Recovery Key of 12 words.
pub const VERSION_WORDS: u8 = 2;

/// The entropy of a Recovery Key, 128 bits.
pub const RK_LEN: usize = 16;
/// How many words show the entropy.
pub const RK_WORDS: usize = 12;
/// The salt of every derivation from the Recovery Key (cloud-recovery.md §3).
pub const HKDF_SALT: &[u8] = b"veydan-recovery";
/// The label of the input of the second wrap of the lock.
pub const LOCK_WRAP_INFO: &[u8] = b"veydan-recovery/lock-wrap/1";

/// A recovery input in canonical form, with the version of wrap it opens.
pub struct Normalized {
    /// What goes into `derive_kek`.
    pub canonical: Zeroizing<String>,
    /// [`VERSION_CODE`] or [`VERSION_WORDS`].
    pub version: u8,
}

/// Fresh random code of version 1 in display form, e.g. `A1B2-C3D4-...`:
/// what 4.0.7 and 5.x before C3 issued; the tests make such rows.
#[cfg(test)]
pub fn generate() -> String {
    let mut rng = rand::rng();
    let raw: String = (0..CODE_LEN)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format_groups(&raw)
}

/// A fresh Recovery Key: its entropy and its 12 words, one space between.
pub fn generate_words() -> (Zeroizing<[u8; RK_LEN]>, Zeroizing<String>) {
    let mut entropy = Zeroizing::new([0u8; RK_LEN]);
    rand::rng().fill_bytes(entropy.as_mut_slice());
    let words = words_of(&entropy);
    (entropy, words)
}

/// The 12 words of an entropy, lower case, one space between them.
pub fn words_of(entropy: &[u8; RK_LEN]) -> Zeroizing<String> {
    // 16 bytes is a length BIP-39 allows: the constructor cannot fail on it.
    match Mnemonic::from_entropy_in(Language::English, entropy) {
        Ok(m) => Zeroizing::new(m.words().collect::<Vec<_>>().join(" ")),
        Err(_) => Zeroizing::new(String::new()),
    }
}

/// The input of the KDF for a wrap of version 2: the hex of the
/// derivation, 64 lower-case characters.
pub fn lock_wrap_input(entropy: &[u8; RK_LEN]) -> Zeroizing<String> {
    let mut out = Zeroizing::new([0u8; 32]);
    // 32 bytes is far below what HKDF-SHA256 allows: expand cannot fail.
    if Hkdf::<Sha256>::new(Some(HKDF_SALT), entropy)
        .expand(LOCK_WRAP_INFO, out.as_mut_slice())
        .is_err()
    {
        out.zeroize();
    }
    let mut hex = String::with_capacity(64);
    for b in out.iter() {
        hex.push_str(&format!("{b:02x}"));
    }
    Zeroizing::new(hex)
}

/// Canonical form of a code of version 1 for the KDF: uppercase, no
/// separators, look-alikes folded. None when the result is not exactly 24
/// alphabet chars.
pub fn normalize_code(input: &str) -> Option<String> {
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

/// The entropy of 12 words as a person typed them: any case, any
/// whitespace, the BIP-39 checksum checked. None for anything else.
pub fn entropy_of_words(input: &str) -> Option<Zeroizing<[u8; RK_LEN]>> {
    let words: Vec<String> = input
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    if words.len() != RK_WORDS {
        return None;
    }
    let text = Zeroizing::new(words.join(" "));
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &text).ok()?;
    let (mut entropy, len) = mnemonic.to_entropy_array();
    if len != RK_LEN {
        entropy.zeroize();
        return None;
    }
    let mut out = Zeroizing::new([0u8; RK_LEN]);
    out.copy_from_slice(&entropy[..RK_LEN]);
    entropy.zeroize();
    Some(out)
}

/// A code or 12 words, as the screen of the lock takes both in one field:
/// the version is told by the input, the KDF input is canonical. None
/// when it is neither.
pub fn normalize(input: &str) -> Option<Normalized> {
    if let Some(entropy) = entropy_of_words(input) {
        return Some(Normalized {
            canonical: lock_wrap_input(&entropy),
            version: VERSION_WORDS,
        });
    }
    normalize_code(input).map(|canonical| Normalized {
        canonical: Zeroizing::new(canonical),
        version: VERSION_CODE,
    })
}

#[cfg(test)]
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

    /// The golden vector of crates/recovery (`golden/recovery-v1.json`):
    /// the same entropy gives the same words and the same wrap input here.
    const GOLDEN_RK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    const GOLDEN_WORDS: &str =
        "abandon math mimic master filter design carbon crystal rookie group knife young";
    const GOLDEN_LOCK_WRAP: &str =
        "2efd25fdd80bf56d6e97ace82cb4b035e7b533a5485d8f72002ee0a80b73862c";

    #[test]
    fn generate_normalizes_to_itself() {
        let code = generate();
        assert_eq!(code.len(), CODE_LEN + CODE_LEN / GROUP - 1);
        assert_eq!(normalize_code(&code).unwrap(), code.replace('-', ""));
        let n = normalize(&code).unwrap();
        assert_eq!((n.canonical.as_str(), n.version), (code.replace('-', "").as_str(), 1));
    }

    #[test]
    fn normalize_folds_lookalikes() {
        let typed = "abcd efgh jkmn pqrs tvwx yz0o";
        assert_eq!(normalize_code(typed).unwrap(), "ABCDEFGHJKMNPQRSTVWXYZ00");
        assert!(normalize("too-short").is_none());
    }

    #[test]
    fn the_words_and_the_wrap_input_match_the_golden_vector_of_recovery() {
        assert_eq!(words_of(&GOLDEN_RK).as_str(), GOLDEN_WORDS);
        assert_eq!(lock_wrap_input(&GOLDEN_RK).as_str(), GOLDEN_LOCK_WRAP);
        let typed = "  Abandon MATH mimic master\nfilter design carbon crystal rookie group knife young ";
        assert_eq!(entropy_of_words(typed).unwrap().as_slice(), &GOLDEN_RK);
        let n = normalize(typed).unwrap();
        assert_eq!((n.canonical.as_str(), n.version), (GOLDEN_LOCK_WRAP, 2));
    }

    #[test]
    fn words_with_a_wrong_checksum_or_count_are_refused() {
        let swapped = "abandon math mimic master filter design carbon crystal rookie group young knife";
        assert!(entropy_of_words(swapped).is_none());
        assert!(normalize(swapped).is_none());
        assert!(normalize("abandon math mimic").is_none());
        assert!(normalize(&format!("{GOLDEN_WORDS} abandon")).is_none());
    }

    #[test]
    fn a_fresh_key_reads_back_from_its_words() {
        let (entropy, words) = generate_words();
        assert_eq!(words.split(' ').count(), RK_WORDS);
        assert_eq!(entropy_of_words(&words).unwrap().as_slice(), entropy.as_slice());
        let (other, _) = generate_words();
        assert_ne!(entropy.as_slice(), other.as_slice());
    }
}

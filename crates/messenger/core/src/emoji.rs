// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What may be sent as a reaction. Not a full emoji table, which would age
//! with every Unicode release: a reaction is one short picture, so the test
//! is shape only. No letters, no spaces, nothing invisible that could
//! carry text or turn the line around; the joiners and modifiers that real
//! emoji are built of pass. Both sides check it: the sender before it goes,
//! the receiver before it is kept.

/// The most bytes of one reaction. A family or a flag with its joiners
/// fits; a sentence does not.
pub const MAX_REACTION_BYTES: usize = 32;
/// The most Unicode scalars of one reaction.
pub const MAX_REACTION_CHARS: usize = 8;

const VS16: char = '\u{fe0f}';
const KEYCAP: char = '\u{20e3}';

/// Whether `s` may be a reaction: not empty, at most
/// `MAX_REACTION_BYTES` bytes and `MAX_REACTION_CHARS` scalars, no ASCII,
/// no letters or digits of any script, no whitespace, no control or
/// invisible formatting characters. The zero width joiner, variation
/// selectors, skin tones and the tag characters of subdivision flags pass. The one ASCII allowed is the base of a keycap
/// (`1️⃣`, `#️⃣`): a digit, `#` or `*` followed by the keycap mark.
pub fn is_reaction(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_REACTION_BYTES || s.chars().count() > MAX_REACTION_CHARS {
        return false;
    }
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().all(|(i, &c)| {
        if c.is_ascii() {
            return keycap_base(&chars[i..]);
        }
        !c.is_whitespace() && !c.is_control() && !invisible(c) && !letter(c)
    })
}

/// A letter or a digit of any script: a few of them make a word. The
/// emoji that Unicode counts as letters (ℹ, Ⓜ, 🅰 and its kin) pass.
fn letter(c: char) -> bool {
    (c.is_alphabetic() || c.is_numeric()) && !matches!(c, '\u{2139}' | '\u{24c2}' | '\u{1f170}'..='\u{1f189}')
}

/// `rest` starts with a keycap: `[0-9#*]`, an optional VS16, U+20E3.
fn keycap_base(rest: &[char]) -> bool {
    let base = matches!(rest.first(), Some(c) if c.is_ascii_digit() || *c == '#' || *c == '*');
    base && match rest.get(1) {
        Some(&KEYCAP) => true,
        Some(&VS16) => rest.get(2) == Some(&KEYCAP),
        _ => false,
    }
}

/// Format characters that show nothing yet hide text or reorder it. The
/// zero width joiner (U+200D) is not among them: emoji are joined by it.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00ad}'                      // soft hyphen
        | '\u{061c}'                    // Arabic letter mark
        | '\u{180e}'                    // Mongolian vowel separator
        | '\u{200b}' | '\u{200c}'       // zero width space, non-joiner
        | '\u{200e}' | '\u{200f}'       // directional marks
        | '\u{2028}' | '\u{2029}'       // line and paragraph separators
        | '\u{202a}'..='\u{202e}'       // directional embeddings, overrides
        | '\u{2060}'..='\u{2064}'       // word joiner, invisible operators
        | '\u{2066}'..='\u{206f}'       // directional isolates, deprecated
        | '\u{feff}'                    // byte order mark
        | '\u{fff9}'..='\u{fffb}'       // interlinear annotation
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reactions_by_shape() {
        let cases: &[(&str, bool, &str)] = &[
            ("👍", true, "plain"),
            ("❤️", true, "with VS16"),
            ("❤", true, "without VS16"),
            ("👨‍👩‍👧", true, "joined by ZWJ"),
            ("🏳️‍🌈", true, "flag with VS16 and ZWJ"),
            ("👍🏽", true, "skin tone"),
            ("🇷🇺", true, "regional indicators"),
            ("1️⃣", true, "keycap"),
            ("#⃣", true, "keycap without VS16"),
            ("🏴\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}", true, "subdivision flag with tags"),
            ("©️", true, "non-ASCII symbol"),
            ("😂😂", true, "two pictures, still short"),
            ("", false, "empty"),
            ("a", false, "a letter"),
            ("+", false, "ASCII sign"),
            ("1", false, "a digit without the keycap mark"),
            ("a\u{20e3}", false, "a letter is no keycap"),
            ("1\u{fe0f}", false, "VS16 without the keycap mark"),
            ("👍 ", false, "trailing space"),
            ("👍\n", false, "newline"),
            ("👍\u{a0}", false, "no-break space"),
            ("👍\u{3000}", false, "ideographic space"),
            ("👍\u{7f}", false, "ASCII control"),
            ("👍\u{85}", false, "C1 control"),
            ("👍\u{202e}", false, "right-to-left override"),
            ("👍\u{200b}", false, "zero width space"),
            ("👍\u{feff}", false, "byte order mark"),
            ("Я", false, "a letter of another script"),
            ("Да", false, "a word of another script"),
            ("٣", false, "a digit of another script"),
            ("ℹ️", true, "an emoji Unicode counts as a letter"),
            ("Ⓜ️", true, "another one"),
            ("🅰️", true, "and another"),
        ];
        for (s, want, why) in cases {
            assert_eq!(is_reaction(s), *want, "{why}: {s:?}");
        }
    }

    #[test]
    fn limits_of_length() {
        let forty = "👍".repeat(10);
        assert_eq!(forty.len(), 40);
        assert!(!is_reaction(&forty), "40 bytes");
        let eight = "👍".repeat(8);
        assert_eq!(eight.len(), MAX_REACTION_BYTES);
        assert!(is_reaction(&eight), "32 bytes, 8 scalars");
        assert!(!is_reaction(&"©".repeat(9)), "9 scalars in 18 bytes");
        assert!(is_reaction(&"©".repeat(8)));
        // The longest Unicode emoji, a kiss with two skin tones, is 10 scalars
        // and 35 bytes: past the limit. Every shorter one of emoji 18.0 passes.
        assert!(!is_reaction("\u{1f469}\u{1f3fb}\u{200d}\u{2764}\u{fe0f}\u{200d}\u{1f48b}\u{200d}\u{1f468}\u{1f3fc}"));
        assert!(is_reaction("\u{1f469}\u{1f3fb}\u{200d}\u{2764}\u{fe0f}\u{200d}\u{1f468}\u{1f3fc}"), "a couple with skin tones: 8");
    }
}

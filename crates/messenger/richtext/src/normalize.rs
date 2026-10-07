// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The one form of a text.
//!
//! Only the first `MAX_INPUT_BYTES` of an input are read, so a huge input
//! costs no more than a long bio. Every line break becomes `\n` and a tab
//! a space. Controls, bidi marks and overrides, the byte order mark and
//! interlinear annotations are dropped: they can turn a text around or
//! hide part of it. At most three combining marks stay in a row, of any
//! script, enough for any language and too few to pile a mark over the
//! lines around it. Line ends are trimmed, at most two empty lines stay in
//! a row, lines past `MAX_LINES` are joined onto the last one by spaces,
//! the whole is trimmed and then cut to `MAX_CHARS`.
//!
//! Applying it twice gives what applying it once gave.

use crate::marks::is_mark;
use crate::{ERR_TOO_LONG, MAX_CHARS, MAX_INPUT_BYTES, MAX_LINES};

/// Combining marks that may stay in a row.
pub(crate) const MAX_MARKS: usize = 3;

pub(crate) fn normalize(input: &str) -> String {
    let mut s = full(head(input));
    if let Some((cut, _)) = s.char_indices().nth(MAX_CHARS) {
        s.truncate(cut);
        let keep = s.trim_end().len();
        s.truncate(keep);
    }
    s
}

/// An input longer than `MAX_INPUT_BYTES` is refused unread: no bio that
/// fits needs as much.
pub(crate) fn check(input: &str) -> Result<String, &'static str> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(ERR_TOO_LONG);
    }
    let s = full(input);
    if s.chars().count() > MAX_CHARS {
        return Err(ERR_TOO_LONG);
    }
    Ok(s)
}

/// The part of an input that is read, cut on a character boundary.
fn head(input: &str) -> &str {
    if input.len() <= MAX_INPUT_BYTES {
        return input;
    }
    let mut end = MAX_INPUT_BYTES;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    &input[..end]
}

/// Everything but the cut to `MAX_CHARS`.
fn full(input: &str) -> String {
    lines(&filter(input))
}

/// Marks that pile up over a letter: the blocks of combining diacritics,
/// whole, and every other nonspacing or enclosing mark.
pub(crate) fn is_combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
        || is_mark(c)
}

/// Joiners and variation selectors are kept and neither count as a mark
/// nor end a row of them, so they cannot be used to stack more.
pub(crate) fn is_transparent(c: char) -> bool {
    matches!(c, '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
}

fn is_dropped(c: char) -> bool {
    (c.is_control() && c != '\n')
        || matches!(c,
            '\u{061C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}')
}

/// Counts combining marks in a row; says whether the next one may stay.
#[derive(Default)]
pub(crate) struct MarkRun(usize);

impl MarkRun {
    pub(crate) fn keep(&mut self, c: char) -> bool {
        if is_combining(c) {
            if self.0 >= MAX_MARKS {
                return false;
            }
            self.0 += 1;
        } else if !is_transparent(c) {
            self.0 = 0;
        }
        true
    }
}

fn filter(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut run = MarkRun::default();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                '\n'
            }
            '\u{2028}' | '\u{2029}' => '\n',
            '\t' => ' ',
            c => c,
        };
        if is_dropped(c) || !run.keep(c) {
            continue;
        }
        out.push(c);
    }
    out
}

fn lines(s: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut empties = 0;
    for line in s.split('\n') {
        let line = line.trim_end();
        let line = if kept.is_empty() { line.trim_start() } else { line };
        if line.is_empty() {
            if kept.is_empty() {
                continue;
            }
            empties += 1;
            if empties > 2 {
                continue;
            }
        } else {
            empties = 0;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.is_empty()) {
        kept.pop();
    }
    if kept.len() <= MAX_LINES {
        return kept.join("\n");
    }
    let mut out = kept[..MAX_LINES - 1].join("\n");
    out.push('\n');
    let tail: Vec<&str> = kept[MAX_LINES - 1..].iter().copied().filter(|l| !l.is_empty()).collect();
    out.push_str(&tail.join(" "));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_breaks_and_tabs() {
        assert_eq!(normalize("a\r\nb\rc\u{2028}d\u{2029}e"), "a\nb\nc\nd\ne");
        assert_eq!(normalize("a\r\r\nb"), "a\n\nb");
        assert_eq!(normalize("a\tb"), "a b");
        assert_eq!(normalize("a\t\n\tb"), "a\n b");
    }

    #[test]
    fn controls_and_bidi_dropped() {
        assert_eq!(normalize("a\u{0}b\u{7}c\u{1B}[31md\u{7F}e\u{85}f\u{9F}g"), "abc[31mdefg");
        for c in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
            '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{FEFF}', '\u{FFF9}', '\u{FFFA}', '\u{FFFB}',
        ] {
            assert_eq!(normalize(&format!("x{c}y")), "xy", "{:04X}", c as u32);
        }
        // An override that would show "gpj.exe" as "exe.jpg".
        assert_eq!(normalize("photo\u{202E}gpj.exe"), "photogpj.exe");
        // Joiners, variation selectors and other format characters stay.
        assert_eq!(normalize("👩\u{200D}💻 ❤\u{FE0F} a\u{200B}b"), "👩\u{200D}💻 ❤\u{FE0F} a\u{200B}b");
    }

    #[test]
    fn zalgo_capped() {
        let z = format!("e{}", "\u{0301}".repeat(50));
        assert_eq!(normalize(&z), format!("e{}", "\u{0301}".repeat(3)));
        // Every block counts, and they count together.
        let mixed = "a\u{0300}\u{1AB0}\u{1DC0}\u{20D0}\u{FE20}b";
        assert_eq!(normalize(mixed), "a\u{0300}\u{1AB0}\u{1DC0}b");
        // A joiner or a selector does not start a new row.
        let hidden = "e\u{0301}\u{0301}\u{0301}\u{200D}\u{0301}\u{FE0F}\u{0301}";
        assert_eq!(normalize(hidden), "e\u{0301}\u{0301}\u{0301}\u{200D}\u{FE0F}");
        // A dropped control does not either.
        let ctl = "e\u{0301}\u{0301}\u{0}\u{0301}\u{0301}";
        assert_eq!(normalize(ctl), "e\u{0301}\u{0301}\u{0301}");
        // Any other character does.
        assert_eq!(normalize("é\u{0301}\u{0301}\u{0301}e\u{0301}"), "é\u{0301}\u{0301}\u{0301}e\u{0301}");
        // Ordinary text with marks is untouched.
        assert_eq!(normalize("Tiếng Việt"), "Tiếng Việt");
        assert_eq!(normalize("e\u{0302}\u{0301}"), "e\u{0302}\u{0301}");
    }

    #[test]
    fn line_ends_and_whole_trimmed() {
        assert_eq!(normalize("  a  \n b \u{A0}\n\n  "), "a\n b");
        assert_eq!(normalize("\n\n \n x"), "x");
        assert_eq!(normalize("   "), "");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("\u{202E}\u{0}\n\t"), "");
    }

    #[test]
    fn blank_lines_capped() {
        assert_eq!(normalize("a\n\n\n\n\n\nb"), "a\n\n\nb");
        assert_eq!(normalize("a\n \n\t\n  \n\nb"), "a\n\n\nb");
        assert_eq!(normalize("a\n\nb\n\n\nc"), "a\n\nb\n\n\nc");
    }

    #[test]
    fn lines_capped() {
        let many: Vec<String> = (1..=60).map(|i| format!("l{i}")).collect();
        let n = normalize(&many.join("\n"));
        let lines: Vec<&str> = n.split('\n').collect();
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[MAX_LINES - 2], "l39");
        let tail: Vec<String> = (40..=60).map(|i| format!("l{i}")).collect();
        assert_eq!(lines[MAX_LINES - 1], tail.join(" "));
        // Empty lines of the rest leave no double spaces.
        let mut gappy: Vec<String> = (1..=39).map(|i| format!("l{i}")).collect();
        gappy.extend(["x".into(), "".into(), "".into(), "y".into(), "".into(), "z".into()]);
        let n = normalize(&gappy.join("\n"));
        assert!(n.ends_with("\nl39\nx y z"), "{n}");
        // Exactly MAX_LINES lines are left alone.
        let exact: Vec<String> = (1..=MAX_LINES).map(|i| format!("l{i}")).collect();
        assert_eq!(normalize(&exact.join("\n")), exact.join("\n"));
        // Blank lines are collapsed before lines are counted.
        let spaced = (1..=30).map(|i| format!("l{i}")).collect::<Vec<_>>().join("\n\n\n\n\n");
        assert_eq!(normalize(&spaced).split('\n').count(), MAX_LINES);
    }

    #[test]
    fn chars_capped() {
        let long = "я".repeat(MAX_CHARS + 500);
        let n = normalize(&long);
        assert_eq!(n.chars().count(), MAX_CHARS);
        // A cut that ends on a space or a line break is trimmed.
        let s = format!("{}  \n\n b", "a".repeat(MAX_CHARS - 2));
        assert_eq!(normalize(&s), "a".repeat(MAX_CHARS - 2));
        // Counted after normalising: dropped characters do not count.
        let padded = format!("{}{}", "a".repeat(MAX_CHARS), "\u{0}".repeat(5000));
        assert_eq!(normalize(&padded).chars().count(), MAX_CHARS);
    }

    #[test]
    fn check_refuses_too_long() {
        assert_eq!(check(&"a".repeat(MAX_CHARS)), Ok("a".repeat(MAX_CHARS)));
        assert_eq!(check(&"a".repeat(MAX_CHARS + 1)), Err("bio_too_long"));
        // Whitespace and controls that normalising removes do not count.
        let s = format!("  {}\u{200E}\u{0}  \n\n\n\n", "b".repeat(MAX_CHARS));
        assert_eq!(check(&s), Ok("b".repeat(MAX_CHARS)));
        assert_eq!(check(" a\r\n b "), Ok("a\n b".into()));
    }

    #[test]
    fn idempotent() {
        let mut cases: Vec<String> = vec![
            "".into(),
            "  hi \r\n\r\n\r\n\r\n there \t".into(),
            format!("e{}\u{200D}{}", "\u{0301}".repeat(5), "\u{0301}".repeat(5)),
            format!("{}\n\n\n{}", "x".repeat(1999), "y"),
            format!("{} {}", "a".repeat(MAX_CHARS - 1), "\u{0301}".repeat(9)),
            (0..100).map(|i| if i % 3 == 0 { " \n".to_string() } else { format!("w{i}") }).collect(),
            (0..100).map(|i| format!("  {i}\t \u{2028}")).collect(),
            "\u{0301}\u{0301}\u{0301}\u{0301} start".into(),
        ];
        cases.push(format!("{}\n\u{0301}\u{0301}", "q\n".repeat(60)));
        for c in cases {
            let once = normalize(&c);
            assert_eq!(normalize(&once), once, "{c:?}");
            assert_eq!(check(&once), Ok(once.clone()));
        }
    }

    #[test]
    fn marks_of_every_script_capped() {
        // Nonspacing and enclosing marks beyond the blocks of diacritics
        // pile up just the same: Cyrillic, Hebrew, Arabic, Syriac, Thai,
        // Lao, Tibetan, musical, Cyrillic extended.
        for m in [
            '\u{0483}', '\u{0489}', '\u{0591}', '\u{05BF}', '\u{05C7}', '\u{0610}', '\u{064B}', '\u{0670}', '\u{06D6}',
            '\u{0711}', '\u{0730}', '\u{0E31}', '\u{0E47}', '\u{0EB1}', '\u{0F71}', '\u{0F90}', '\u{1D167}',
            '\u{1D17B}', '\u{1D1AA}', '\u{2DE0}', '\u{A670}', '\u{A674}', '\u{A69E}',
        ] {
            let z = format!("a{}b", m.to_string().repeat(50));
            assert_eq!(normalize(&z), format!("a{}b", m.to_string().repeat(3)), "{:04X}", m as u32);
        }
        // The enclosing millions sign, alone and mixed with Arabic tashkil.
        assert_eq!(normalize(&format!("a{}", "\u{0489}".repeat(1999))), "a\u{0489}\u{0489}\u{0489}");
        let mixed = format!("a{}", "\u{0489}\u{064B}".repeat(500));
        assert_eq!(normalize(&mixed), "a\u{0489}\u{064B}\u{0489}");
        assert_eq!(crate::strip(&mixed), "a\u{0489}\u{064B}\u{0489}");
        // They count together with the diacritics.
        assert_eq!(normalize("a\u{0301}\u{0489}\u{064B}\u{0E48}b"), "a\u{0301}\u{0489}\u{064B}b");
        // The old blocks count whole, assigned or not.
        assert_eq!(normalize("a\u{20FF}\u{20FF}\u{20FF}\u{20FF}"), "a\u{20FF}\u{20FF}\u{20FF}");
        // Real text keeps its marks.
        for real in ["שָׁלוֹם", "בְּרֵאשִׁית", "مُحَمَّدٌ", "ٱلرَّحْمَٰنِ", "ภาษาไทย น้ำ ที่", "ພາສາລາວ", "བོད་ཡིག", "हिन्दी", "Привет"] {
            assert_eq!(normalize(real), real, "{real}");
        }
        // Spacing letters and punctuation of those scripts are not marks.
        assert!(!is_combining('\u{05BE}') && !is_combining('\u{05C0}') && !is_combining('\u{05C3}'));
        assert!(!is_combining('\u{0E32}') && !is_combining('a') && !is_combining('\u{200D}'));
    }

    #[test]
    fn variation_selectors_are_transparent() {
        // Any variation selector is kept and neither counts nor ends a row.
        for vs in ['\u{FE00}', '\u{FE0E}', '\u{FE0F}', '\u{E0100}', '\u{E01EF}'] {
            let s = format!("e\u{0301}\u{0301}{vs}\u{0301}\u{0301}");
            assert_eq!(normalize(&s), format!("e\u{0301}\u{0301}{vs}\u{0301}"), "{:04X}", vs as u32);
        }
        assert_eq!(normalize("葛\u{E0100}"), "葛\u{E0100}");
    }

    #[test]
    fn input_read_up_to_a_cap() {
        // What lies past MAX_INPUT_BYTES is never read: a bio cannot be
        // pushed there by padding that normalising would remove.
        let padded = format!("{}hi", " ".repeat(MAX_INPUT_BYTES));
        assert_eq!(normalize(&padded), "");
        assert_eq!(check(&padded), Err(ERR_TOO_LONG));
        let just = format!("{}hi", " ".repeat(MAX_INPUT_BYTES - 2));
        assert_eq!(normalize(&just), "hi");
        assert_eq!(check(&just), Ok("hi".into()));
        // The cut falls on a character boundary.
        let wide = format!("{}{}", " ".repeat(MAX_INPUT_BYTES - 1), "я".repeat(10));
        assert_eq!(normalize(&wide), "");
        let wide = format!("{}{}", " ".repeat(MAX_INPUT_BYTES - 2), "я".repeat(10));
        assert_eq!(normalize(&wide), "я");
        // A huge input costs what the cap costs.
        for huge in ["a\n".repeat(10_000_000), "\u{0}".repeat(20_000_000), " ".repeat(20_000_000)] {
            let started = std::time::Instant::now();
            let n = normalize(&huge);
            let _ = check(&huge);
            let _ = crate::parse(&huge);
            let _ = crate::strip(&huge);
            let took = started.elapsed();
            assert!(took < std::time::Duration::from_millis(300), "took {took:?}");
            assert_eq!(normalize(&n), n);
        }
    }
}

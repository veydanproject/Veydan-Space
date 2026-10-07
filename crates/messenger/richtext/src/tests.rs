// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Tests of the whole: markup, links, the one form of what `strip` gives,
//! and hostile inputs.

use crate::*;
use std::time::{Duration, Instant};

fn st() -> Style {
    Style::default()
}

fn t(text: &str, style: Style) -> Span {
    Span::Text { text: text.into(), style }
}

fn p(text: &str) -> Span {
    t(text, st())
}

fn l(url: &str, style: Style) -> Span {
    Span::Link { url: url.into(), text: url.into(), style }
}

fn bold() -> Style {
    Style { bold: true, ..st() }
}

fn italic() -> Style {
    Style { italic: true, ..st() }
}

fn strike() -> Style {
    Style { strike: true, ..st() }
}

fn code() -> Style {
    Style { code: true, ..st() }
}

fn color(c: Color) -> Style {
    Style { color: Some(c), ..st() }
}

/// What every parse result must satisfy, whatever the input.
fn assert_sound(input: &str) {
    for (spans, markup) in [(parse(input), true), (plain(input), false)] {
        let text = text_of(&spans);
        assert_eq!(normalize(&text), text, "text of spans not normal: {input:?} (markup {markup})");
        assert!(text.chars().count() <= MAX_CHARS);
        let mut prev: Option<&Span> = None;
        for span in &spans {
            match span {
                Span::Text { text, style } => {
                    assert!(!text.is_empty(), "empty text span: {input:?}");
                    assert!(!text.contains('\n'), "line break in a text span: {input:?}");
                    if let Some(Span::Text { style: ps, .. }) = prev {
                        assert_ne!(ps, style, "unmerged neighbours: {input:?}");
                    }
                    if !markup {
                        assert_eq!(style, &Style::default());
                    }
                }
                Span::Link { url, text, .. } => {
                    assert_eq!(url, text);
                    assert!(url.starts_with("https://"));
                    assert!(url.len() <= MAX_LINK_BYTES);
                    assert!(crate::link::is_valid(url), "{url}");
                }
                Span::Break => {}
            }
            prev = Some(span);
        }
        assert!(!matches!(spans.first(), Some(Span::Break)));
        assert!(!matches!(spans.last(), Some(Span::Break)));
    }
    assert_eq!(strip(input), text_of(&parse(input)));
    assert_eq!(strip(&normalize(input)), strip(input));
    assert_eq!(parse(&normalize(input)), parse(input));
}

#[test]
fn every_mark() {
    assert_eq!(parse("**b**"), vec![t("b", bold())]);
    assert_eq!(parse("*i*"), vec![t("i", italic())]);
    assert_eq!(parse("~~s~~"), vec![t("s", strike())]);
    assert_eq!(parse("`c`"), vec![t("c", code())]);
    for c in Color::ALL {
        assert_eq!(parse(&format!("{{{}}}x{{/}}", c.name())), vec![t("x", color(c))]);
    }
    assert_eq!(
        parse("a **b** c *d* e ~~f~~ g `h` {blue}i{/} j"),
        vec![
            p("a "),
            t("b", bold()),
            p(" c "),
            t("d", italic()),
            p(" e "),
            t("f", strike()),
            p(" g "),
            t("h", code()),
            p(" "),
            t("i", color(Color::Blue)),
            p(" j"),
        ]
    );
}

#[test]
fn nesting() {
    let bi = Style { bold: true, italic: true, ..st() };
    assert_eq!(parse("***x***"), vec![t("x", bi.clone())]);
    assert_eq!(parse("**a *b* c**"), vec![t("a ", bold()), t("b", bi.clone()), t(" c", bold())]);
    assert_eq!(parse("*a **b** c*"), vec![t("a ", italic()), t("b", bi), t(" c", italic())]);
    let red_bold = Style { bold: true, color: Some(Color::Red), ..st() };
    assert_eq!(parse("{red}a **b**{/}"), vec![t("a ", color(Color::Red)), t("b", red_bold.clone())]);
    assert_eq!(parse("**{red}b{/}**"), vec![t("b", red_bold)]);
    let all = Style { bold: true, italic: true, strike: true, code: false, color: Some(Color::Teal) };
    assert_eq!(parse("{teal}**~~*x*~~**{/}"), vec![t("x", all)]);
    // Code takes the style around it.
    let bold_code = Style { bold: true, code: true, ..st() };
    assert_eq!(parse("**a `c` b**"), vec![t("a ", bold()), t("c", bold_code), t(" b", bold())]);
    // Styles span lines.
    assert_eq!(parse("**a\nb**"), vec![t("a", bold()), Span::Break, t("b", bold())]);
    assert_eq!(parse("{red}a\n\nb{/}"), vec![t("a", color(Color::Red)), Span::Break, Span::Break, t("b", color(Color::Red))]);
}

#[test]
fn color_in_color_is_text() {
    assert_eq!(parse("{red}a {blue}b{/} c"), vec![t("a {blue}b", color(Color::Red)), p(" c")]);
    assert_eq!(parse("{red}a{/}{blue}b{/}"), vec![t("a", color(Color::Red)), t("b", color(Color::Blue))]);
    assert_eq!(parse("{red}{red}x{/}{/}"), vec![t("{red}x", color(Color::Red)), p("{/}")]);
}

#[test]
fn unknown_or_odd_colors_are_text() {
    for s in ["{magenta}x{/}", "{Red}x{/}", "{RED}x{/}", "{ red}x{/}", "{red }x{/}", "{}x{/}", "{/}", "{red", "{red}"] {
        assert_eq!(parse(s), vec![p(s)], "{s}");
    }
    assert_eq!(parse("{/}{red}x{/}"), vec![p("{/}"), t("x", color(Color::Red))]);
}

#[test]
fn code_is_literal() {
    assert_eq!(parse("`**x** {red}y{/} \\* https://a.com`"), vec![t("**x** {red}y{/} \\* https://a.com", code())]);
    // Code does not span lines; its backtick then is text.
    assert_eq!(parse("`a\nb`"), vec![p("`a"), Span::Break, p("b`")]);
    // Empty code is text.
    assert_eq!(parse("``"), vec![p("``")]);
    assert_eq!(parse("``a`"), vec![p("`"), t("a", code())]);
    assert_eq!(parse("`a` `b`"), vec![t("a", code()), p(" "), t("b", code())]);
    assert_eq!(parse("a`b"), vec![p("a`b")]);
    // Spaces inside code stay, except at the end of a line.
    assert_eq!(parse("x ` a ` y"), vec![p("x "), t(" a ", code()), p(" y")]);
}

#[test]
fn unclosed_marks_are_text() {
    for s in ["**a", "*a", "~~a", "a**", "a*", "a~~", "**a*", "{red}a", "a{/}", "`a", "~a~", "~~ a~~", "** a**", "* a *"] {
        assert_eq!(parse(s), vec![p(s)], "{s}");
    }
    assert_eq!(parse("**a *b**"), vec![t("a *b", bold())]);
    assert_eq!(parse("*a **b*"), vec![t("a **b", italic())]);
    assert_eq!(parse("**a ~~b**"), vec![t("a ~~b", bold())]);
    assert_eq!(parse("{red}**a{/}"), vec![t("**a", color(Color::Red))]);
    assert_eq!(parse("*a*b*"), vec![t("a", italic()), p("b*")]);
    assert_eq!(parse("**a**b**"), vec![t("a", bold()), p("b**")]);
}

#[test]
fn spaces_around_marks() {
    assert_eq!(parse("2 * 3 * 4"), vec![p("2 * 3 * 4")]);
    assert_eq!(parse("* one\n* two"), vec![p("* one"), Span::Break, p("* two")]);
    assert_eq!(parse("a ** b ** c"), vec![p("a ** b ** c")]);
    assert_eq!(parse("2*3*4"), vec![p("2"), t("3", italic()), p("4")]);
    assert_eq!(parse("(**x**)"), vec![p("("), t("x", bold()), p(")")]);
    assert_eq!(parse("****"), vec![p("****")]);
    assert_eq!(parse("~~~~"), vec![p("~~~~")]);
    assert_eq!(parse("~"), vec![p("~")]);
    assert_eq!(parse("~~~x~~~"), vec![t("~x", strike()), p("~")]);
}

#[test]
fn escapes() {
    assert_eq!(parse("\\*a\\*"), vec![p("*a*")]);
    assert_eq!(parse("\\*\\*a\\*\\*"), vec![p("**a**")]);
    assert_eq!(parse("\\~~a~~"), vec![p("~~a~~")]);
    assert_eq!(parse("\\~\\~a~~"), vec![p("~~a~~")]);
    assert_eq!(parse("~~a\\~~"), vec![p("~~a~~")]);
    assert_eq!(parse("\\`a`"), vec![p("`a`")]);
    assert_eq!(parse("\\{red}a{/}"), vec![p("{red}a{/}")]);
    assert_eq!(parse("{red}a\\{/}"), vec![p("{red}a{/}")]);
    assert_eq!(parse("\\\\"), vec![p("\\")]);
    assert_eq!(parse("\\\\*a*"), vec![p("\\"), t("a", italic())]);
    // A backslash before anything else is itself.
    assert_eq!(parse("a\\b \\n C:\\x"), vec![p("a\\b \\n C:\\x")]);
    assert_eq!(parse("a\\"), vec![p("a\\")]);
    assert_eq!(strip("\\*not italic\\*"), "*not italic*");
}

#[test]
fn no_other_markup() {
    for s in [
        "# heading",
        "- list",
        "1. list",
        "> quote",
        "![img](https://a.com/x.png)",
        "<b>html</b>",
        "<script>alert(1)</script>",
        "__under__",
        "_it_",
        "||spoiler||",
    ] {
        assert_eq!(strip(s), s, "{s}");
        assert!(parse(s).iter().all(|sp| !matches!(sp, Span::Text { style, .. } if *style != Style::default())), "{s}");
    }
    assert_eq!(
        parse("[text](https://a.com)"),
        vec![p("[text]("), l("https://a.com", st()), p(")")]
    );
}

#[test]
fn autolinks() {
    assert_eq!(parse("see https://a.com/x."), vec![p("see "), l("https://a.com/x", st()), p(".")]);
    assert_eq!(parse("https://a.com"), vec![l("https://a.com", st())]);
    assert_eq!(
        parse("(https://en.wikipedia.org/wiki/Rust_(language))"),
        vec![p("("), l("https://en.wikipedia.org/wiki/Rust_(language)", st()), p(")")]
    );
    assert_eq!(parse("**https://a.com**"), vec![l("https://a.com", bold())]);
    assert_eq!(parse("{red}https://a.com{/}"), vec![l("https://a.com", color(Color::Red))]);
    assert_eq!(parse("~~https://a.com~~"), vec![l("https://a.com", strike())]);
    assert_eq!(parse("https://a.com/~user/x"), vec![l("https://a.com/~user/x", st())]);
    assert_eq!(parse("https://a.com/a*b*c"), vec![l("https://a.com/a*b*c", st())]);
    assert_eq!(parse("a https://x.io\nhttps://y.io"), vec![p("a "), l("https://x.io", st()), Span::Break, l("https://y.io", st())]);
    assert_eq!(parse("<https://a.com>"), vec![p("<"), l("https://a.com", st()), p(">")]);
    assert_eq!(parse("\"https://a.com\""), vec![p("\""), l("https://a.com", st()), p("\"")]);
}

#[test]
fn autolinks_https_only() {
    for s in ["http://a.com", "ftp://a.com", "javascript:alert(1)", "data:text/html,x", "veydan://contact/x", "HTTPS://A.COM", "www.a.com"] {
        assert_eq!(parse(s), vec![p(s)], "{s}");
        assert_eq!(plain(s), vec![p(s)], "{s}");
    }
    // Part of a word.
    assert_eq!(parse("xhttps://a.com"), vec![p("xhttps://a.com")]);
    assert_eq!(parse("1https://a.com"), vec![p("1https://a.com")]);
    assert_eq!(parse("_https://a.com"), vec![p("_https://a.com")]);
    assert_eq!(parse("https://"), vec![p("https://")]);
    assert_eq!(parse("https:// a.com"), vec![p("https:// a.com")]);
}

#[test]
fn autolinks_refuse_userinfo() {
    for s in ["https://user@evil.com", "https://user:pw@evil.com/x", "https://good.com@evil.com/"] {
        assert_eq!(parse(s), vec![p(s)], "{s}");
        assert_eq!(plain(s), vec![p(s)], "{s}");
    }
    // A refused address is text whole: nothing inside it becomes a link or a mark.
    let s = "https://u@evil.com/https://good.com/**x**";
    assert_eq!(parse(s), vec![p(s)]);
    assert_eq!(strip(s), s);
    // `@` after the host is fine.
    assert_eq!(parse("https://a.com/@me"), vec![l("https://a.com/@me", st())]);
}

#[test]
fn autolinks_capped() {
    // The longest that fits a bio: MAX_CHARS bytes of ASCII.
    let longest = format!("https://a.com/{}", "x".repeat(MAX_CHARS - 14));
    assert_eq!(parse(&longest), vec![l(&longest, st())]);
    // Over the cap by bytes, still under MAX_CHARS by characters.
    let over = format!("https://a.com/{}", "я".repeat(1100));
    assert!(over.len() > MAX_LINK_BYTES && over.chars().count() < MAX_CHARS);
    assert_eq!(parse(&over), vec![p(&over)]);
    assert_eq!(plain(&over), vec![p(&over)]);
}

#[test]
fn plain_finds_links_only() {
    assert_eq!(
        plain("**a** `b` {red}c{/} \\* https://a.com."),
        vec![p("**a** `b` {red}c{/} \\* "), l("https://a.com", st()), p(".")]
    );
    assert_eq!(plain(" a\r\n\r\n\r\n\r\nb "), vec![p("a"), Span::Break, Span::Break, Span::Break, p("b")]);
    assert_eq!(plain(""), vec![]);
}

#[test]
fn strip_is_the_text() {
    assert_eq!(strip("**Hi**, I'm *Ann*. {pink}~~cats~~{/} `rust` https://a.com!"), "Hi, I'm Ann. cats rust https://a.com!");
    assert_eq!(strip("line\n\n**two**"), "line\n\ntwo");
    assert_eq!(strip(""), "");
}

#[test]
fn strip_stays_normal() {
    // Taking marks out leaves no space at a line end, no blank start, no
    // extra empty lines and no pile of combining marks.
    assert_eq!(strip("a {red}{/}"), "a");
    assert_eq!(strip("{red} a{/}"), "a");
    assert_eq!(strip("a **  **"), "a **  **");
    assert_eq!(strip("a {red} {/}\nb"), "a\nb");
    assert_eq!(strip("{red}{/}\n\nx"), "x");
    assert_eq!(strip("x\n\n{red}{/}\n\ny"), "x\n\n\ny");
    assert_eq!(strip("x\n{red}{/}\n{red}{/}\n{red}{/}\n{red}{/}\ny"), "x\n\n\ny");
    let piled = "e\u{301}\u{301}{red}\u{301}\u{301}{/}";
    assert_eq!(strip(piled), "e\u{301}\u{301}\u{301}");
    assert_eq!(parse(piled), vec![p("e\u{301}\u{301}"), t("\u{301}", color(Color::Red))]);
    for s in [piled, "a {red}{/}", "x\n{red}{/}\n{red}{/}\n{red}{/}\n{red}{/}\ny", "{red}{/}"] {
        assert_sound(s);
    }
}

#[test]
fn input_is_normalised_first() {
    assert_eq!(parse("  **a**\u{202E}  \r\n"), vec![t("a", bold())]);
    assert_eq!(parse("*\u{200F}a*"), vec![t("a", italic())]);
    let long = format!("**{}**", "a".repeat(MAX_CHARS));
    // Cut to MAX_CHARS first, so the closing mark is gone.
    assert_eq!(strip(&long).chars().count(), MAX_CHARS);
    assert!(strip(&long).starts_with("**a"));
}

#[test]
fn serde_shape() {
    let spans = parse("**a** https://a.com\n{red}b{/}");
    let json = serde_json::to_value(&spans).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {"kind": "text", "text": "a", "style": {"bold": true, "italic": false, "strike": false, "code": false, "color": null}},
            {"kind": "text", "text": " ", "style": {"bold": false, "italic": false, "strike": false, "code": false, "color": null}},
            {"kind": "link", "url": "https://a.com", "text": "https://a.com", "style": {"bold": false, "italic": false, "strike": false, "code": false, "color": null}},
            {"kind": "break"},
            {"kind": "text", "text": "b", "style": {"bold": false, "italic": false, "strike": false, "code": false, "color": "red"}},
        ])
    );
    let back: Vec<Span> = serde_json::from_value(json).unwrap();
    assert_eq!(back, spans);
}

#[test]
fn colors_named() {
    for c in Color::ALL {
        assert_eq!(Color::from_name(c.name()), Some(c));
        assert_eq!(serde_json::to_value(c).unwrap(), serde_json::json!(c.name()));
    }
    assert_eq!(Color::from_name("grey"), None);
}

#[test]
fn check_and_parse_agree() {
    let ok = check(" **hi**\r\n").unwrap();
    assert_eq!(ok, "**hi**");
    assert_eq!(parse(&ok), parse(" **hi**\r\n"));
    assert_eq!(check(&"*".repeat(MAX_CHARS + 1)), Err(ERR_TOO_LONG));
}

fn hostile() -> Vec<String> {
    let n = 100_000;
    vec![
        "*".repeat(n),
        "**".repeat(n / 2) + "a",
        "*a".repeat(n / 2),
        "a*".repeat(n / 2),
        "~".repeat(n),
        "~~a".repeat(n / 3),
        "{".repeat(n),
        "}".repeat(n),
        "{red}".repeat(n / 5),
        "{red}x{/}".repeat(n / 9),
        "{/}".repeat(n / 3),
        "{redd".repeat(n / 5),
        "\\".repeat(n),
        "\\*".repeat(n / 2),
        "`".repeat(n),
        "`a".repeat(n / 2),
        "`\n".repeat(n / 2),
        "**{red}~~*".repeat(n / 10),
        "{red}**~~*a".repeat(n / 10),
        "*~".repeat(n / 2) + "a",
        "https://".repeat(n / 8),
        "https://a.com/".to_string() + &"(".repeat(n),
        "https://a.com/".to_string() + &")".repeat(n),
        "https://a.com/".to_string() + &".".repeat(n),
        "https://a.com".repeat(n / 13),
        "https://u@a.com ".repeat(n / 16),
        "e".to_string() + &"\u{301}".repeat(n),
        "\u{202E}".repeat(n),
        "\n".repeat(n),
        " \n".repeat(n / 2),
        "a\n".repeat(n / 2),
        "\r".repeat(n),
        "\t".repeat(n) + "x",
        "\u{0}".repeat(n),
    ]
}

#[test]
fn hostile_inputs_are_fast_and_sound() {
    for input in hostile() {
        let started = Instant::now();
        let _ = normalize(&input);
        let _ = check(&input);
        let spans = parse(&input);
        let _ = plain(&input);
        let _ = strip(&input);
        // The parser itself, on the whole input, with no cut before it.
        let raw = crate::parse::parse(&input, true);
        let _ = crate::parse::parse(&input, false);
        let took = started.elapsed();
        assert!(took < Duration::from_secs(3), "{:?}… took {took:?}", input.chars().take(20).collect::<String>());
        assert!(text_of(&spans).chars().count() <= MAX_CHARS);
        assert!(!raw.is_empty() || input.trim().is_empty() || normalize(&input).is_empty());
        assert_sound(&input);
    }
}

#[test]
fn deep_nesting_is_bounded() {
    let deep = "{red}**~~*".repeat(100) + "x" + &"*~~**{/}".repeat(100);
    let spans = parse(&deep);
    // Each kind is open at most once: a second opener closes the first or is
    // text, so nothing nests deeper than the four kinds.
    assert!(strip(&deep).contains('x'));
    assert!(spans.iter().any(|s| matches!(s, Span::Text { style, .. } if style.color == Some(Color::Red))));
    assert_sound(&deep);
}

/// A small deterministic generator: random texts from markup pieces.
fn fuzz_inputs(count: usize) -> Vec<String> {
    const PIECES: &[&str] = &[
        "*", "**", "***", "~", "~~", "`", "\\", "{", "}", "{red}", "{blue}", "{/}", "{nope}", "a", "b", "word",
        " ", "  ", "\n", "\n\n\n", "\r\n", "\t", "\u{301}", "\u{200D}", "\u{202E}", "\u{0}", "https://", "https://a.com",
        "https://a.com/x).", "https://u@b.com", "(", ")", ".", "я", "👩\u{200D}💻", "\u{A0}", "\u{2028}",
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let len = (next() % 40) as usize;
            (0..len).map(|_| PIECES[(next() % PIECES.len() as u64) as usize]).collect()
        })
        .collect()
}

#[test]
fn fuzzed_inputs_are_sound() {
    for input in fuzz_inputs(5000) {
        assert_sound(&input);
        let once = normalize(&input);
        assert_eq!(normalize(&once), once, "{input:?}");
    }
}

#[test]
fn autolinks_keep_their_own_tilde_and_star() {
    // A `~` or `*` at the end of an address is part of it while no mark is
    // open that it could close, as in the messages' tokenizer.
    for s in ["https://a.com/~", "https://www.cs.cmu.edu/~", "https://host/file.txt~", "https://a.com/*", "https://a.com/x**", "https://a.com/x~~"] {
        assert_eq!(parse(s), vec![l(s, st())], "{s}");
        assert_eq!(plain(s), vec![l(s, st())], "{s}");
        assert_eq!(strip(s), s);
    }
    assert_eq!(parse("see https://a.com/~."), vec![p("see "), l("https://a.com/~", st()), p(".")]);
    assert_eq!(plain("~~https://a.com/~~"), vec![p("~~"), l("https://a.com/~~", st())]);
    // A run closes the marks that are open and no more.
    assert_eq!(parse("~~https://a.com/~~~"), vec![l("https://a.com/~", strike())]);
    assert_eq!(parse("~~x https://a.com/~ y~~"), vec![t("x ", strike()), l("https://a.com/~", strike()), t(" y", strike())]);
    assert_eq!(parse("**https://a.com/*"), vec![p("**"), l("https://a.com/*", st())]);
    assert_eq!(parse("*https://a.com/**"), vec![l("https://a.com/*", italic())]);
    let bi = Style { bold: true, italic: true, ..st() };
    assert_eq!(parse("***https://a.com***"), vec![l("https://a.com", bi)]);
    assert_eq!(parse("**https://a.com**."), vec![l("https://a.com", bold()), p(".")]);
    assert_eq!(parse("**https://a.com.**"), vec![l("https://a.com", bold()), t(".", bold())]);
    assert_eq!(parse("*https://a.com*!"), vec![l("https://a.com", italic()), p("!")]);
    for s in ["https://a.com/~", "~~https://a.com/~~~", "**https://a.com/*", "*https://a.com/**", "~~x https://a.com/~ y~~"] {
        assert_sound(s);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Tests of the Markdown of messages: what each mark becomes, what is
//! never markup (HTML, pictures, links that lie), the limits, and the
//! text without the marks.

use crate::md::*;
use std::time::{Duration, Instant};

fn t(text: &str) -> MdInline {
    MdInline::Text { text: text.into(), style: MdStyle::default() }
}

fn styled(text: &str, bold: bool, italic: bool, strike: bool) -> MdInline {
    MdInline::Text { text: text.into(), style: MdStyle { bold, italic, strike } }
}

fn para(inlines: Vec<MdInline>) -> MdBlock {
    MdBlock::Para { inlines }
}

fn one(input: &str) -> Vec<MdInline> {
    match parse(input).unwrap().as_slice() {
        [MdBlock::Para { inlines }] => inlines.clone(),
        other => panic!("one paragraph expected: {other:?}"),
    }
}

#[test]
fn marks_of_a_line() {
    assert_eq!(
        one("a **b** *c* ~~d~~ `e`"),
        vec![
            t("a "),
            styled("b", true, false, false),
            t(" "),
            styled("c", false, true, false),
            t(" "),
            styled("d", false, false, true),
            t(" "),
            MdInline::Code { text: "e".into() },
        ]
    );
    assert_eq!(one("***both***"), vec![styled("both", true, true, false)]);
    assert_eq!(one(r"\*not\*"), vec![t("*not*")], "an escape is the character");
}

#[test]
fn a_new_line_is_a_new_line() {
    assert_eq!(one("one\ntwo"), vec![t("one"), MdInline::Break, t("two")]);
}

#[test]
fn blocks() {
    let blocks = parse("# Title\n\n#### deep\n\n> quoted\n\n- a\n- [x] b\n\n3. c\n\n---\n\n```Rust extra\nfn main() {}\n```").unwrap();
    assert_eq!(
        blocks,
        vec![
            MdBlock::Heading { level: 1, inlines: vec![t("Title")] },
            MdBlock::Heading { level: MAX_HEADING, inlines: vec![t("deep")] },
            MdBlock::Quote { blocks: vec![para(vec![t("quoted")])] },
            MdBlock::List {
                ordered: false,
                start: 1,
                items: vec![
                    MdItem { task: None, blocks: vec![para(vec![t("a")])] },
                    MdItem { task: Some(true), blocks: vec![para(vec![t("b")])] },
                ],
            },
            MdBlock::List { ordered: true, start: 3, items: vec![MdItem { task: None, blocks: vec![para(vec![t("c")])] }] },
            MdBlock::Rule,
            MdBlock::Code { lang: Some("rust".into()), text: "fn main() {}".into() },
        ]
    );
}

#[test]
fn code_is_what_it_is() {
    let blocks = parse("```\n**not bold** <b>x</b>\n\n[a](https://x.example)\n```").unwrap();
    assert_eq!(blocks, vec![MdBlock::Code { lang: None, text: "**not bold** <b>x</b>\n\n[a](https://x.example)".into() }]);
    assert_eq!(parse("```<script>\nx\n```").unwrap(), vec![MdBlock::Code { lang: None, text: "x".into() }], "a strange language is no language");
    assert_eq!(parse("    indented\n").unwrap(), vec![MdBlock::Code { lang: None, text: "indented".into() }]);
}

#[test]
fn html_is_text() {
    assert_eq!(one("a <img src=x onerror=alert(1)> b"), vec![t("a <img src=x onerror=alert(1)> b")]);
    assert_eq!(
        parse("<div onclick=\"x\">\nhi\n</div>").unwrap(),
        vec![para(vec![t("<div onclick=\"x\">"), MdInline::Break, t("hi"), MdInline::Break, t("</div>")])]
    );
    assert_eq!(one("&lt;b&gt;"), vec![t("<b>")], "an entity is the character, still text");
}

#[test]
fn links_carry_their_host() {
    assert_eq!(
        one("[the bank](https://Bank.example:8443/login?x=1)"),
        vec![MdInline::Link { url: "https://Bank.example:8443/login?x=1".into(), host: Some("bank.example".into()), children: vec![t("the bank")] }]
    );
    assert_eq!(
        one("[group](veydan://group/abc)"),
        vec![MdInline::Link { url: "veydan://group/abc".into(), host: None, children: vec![t("group")] }]
    );
    assert_eq!(
        one("<https://a.example>"),
        vec![MdInline::Link { url: "https://a.example".into(), host: Some("a.example".into()), children: vec![t("https://a.example")] }]
    );
    assert_eq!(
        one("[v6](http://[::1]:80/x)"),
        vec![MdInline::Link { url: "http://[::1]:80/x".into(), host: Some("[::1]".into()), children: vec![t("v6")] }]
    );
}

#[test]
fn links_that_lie_are_text() {
    // The words, then the address as code: never a link again, not even to the UI's finder of bare links.
    let lie = |words: &str, dest: &str| vec![t(&format!("{words} (")), MdInline::Code { text: dest.into() }, t(")")];
    assert_eq!(one("[x](javascript:alert(1))"), lie("x", "javascript:alert(1)"));
    assert_eq!(one("[Bank](https://bank.example@evil.example/)"), lie("Bank", "https://bank.example@evil.example/"));
    assert_eq!(one("[x](/relative)"), lie("x", "/relative"));
    assert_eq!(one("[x](https://аpple.example)"), lie("x", "https://аpple.example"), "a host in letters of another script");
    assert_eq!(one("[x](https://a.example/\u{202E}gpj.exe)"), lie("x", "https://a.example/\u{202E}gpj.exe"));
    assert_eq!(one("[x](https://good.example%2eevil.example)"), lie("x", "https://good.example%2eevil.example"), "a dot the browser makes");
    assert_eq!(one("[x](https://a.example:80x/)"), lie("x", "https://a.example:80x/"));
    assert_eq!(one("[x](data:text/html,hi)"), lie("x", "data:text/html,hi"));
    assert_eq!(one("<a@b.example>"), vec![MdInline::Code { text: "a@b.example".into() }]);
    assert_eq!(one("<https://bank.example@evil.example>"), vec![MdInline::Code { text: "https://bank.example@evil.example".into() }]);
}

#[test]
fn a_scheme_is_opened_in_lowercase() {
    assert_eq!(
        one("[x](HTTPS://A.example/Path)"),
        vec![MdInline::Link { url: "https://A.example/Path".into(), host: Some("a.example".into()), children: vec![t("x")] }]
    );
    assert_eq!(
        one("[g](VEYDAN://group/abc)"),
        vec![MdInline::Link { url: "veydan://group/abc".into(), host: None, children: vec![t("g")] }]
    );
}

#[test]
fn a_wide_table_is_read_without_tables() {
    // 300 columns × 20 rows could hold 6000 cells: read as lines of text.
    let input = format!("{}|\n{}|\n{}", "| a ".repeat(300), "|-".repeat(300), "| x |\n".repeat(20));
    let blocks = parse(&input).unwrap();
    assert!(blocks.iter().all(|b| !matches!(b, MdBlock::Table { .. })));
    assert!(parse("| a | b |\n|---|---|\n| 1 | 2 |").unwrap().iter().any(|b| matches!(b, MdBlock::Table { .. })));
}

#[test]
fn links_are_found_however_they_are_spelled() {
    let has = |s: &str| s.contains("://");
    assert!(holds_link("[hi](https:&#47;&#47;evil.example)", |_| false), "an entity in the address of a mark");
    assert!(holds_link("https:&#47;&#47;evil.example", has), "an entity spells the address");
    assert!(holds_link("[hi](https://a.example)", |_| false), "a mark is a link");
    assert!(!holds_link("**hi** `https://in.code`", |s| s.contains("://") && !s.contains("in.code")));
    assert!(!holds_link("just *words*", has));
}

#[test]
fn pictures_are_never_loaded() {
    assert_eq!(
        one("![cat](https://img.example/cat.png)"),
        vec![MdInline::Link { url: "https://img.example/cat.png".into(), host: Some("img.example".into()), children: vec![t("cat")] }]
    );
    // A picture inside a link: the link, with the picture's words.
    assert_eq!(
        one("[![cat](https://img.example/c.png)](https://a.example)"),
        vec![MdInline::Link { url: "https://a.example".into(), host: Some("a.example".into()), children: vec![t("cat")] }]
    );
}

#[test]
fn tables() {
    let blocks = parse("| a | **b** |\n|---|---|\n| 1 | `2` |\n").unwrap();
    assert_eq!(
        blocks,
        vec![MdBlock::Table {
            head: vec![vec![t("a")], vec![styled("b", true, false, false)]],
            rows: vec![vec![vec![t("1")], vec![MdInline::Code { text: "2".into() }]]],
        }]
    );
}

#[test]
fn depth_is_bounded() {
    let deep = format!("{} end", ">".repeat(5000));
    let blocks = parse(&deep).unwrap();
    let mut depth = 0;
    let mut level = &blocks;
    while let [MdBlock::Quote { blocks }] = level.as_slice() {
        depth += 1;
        level = blocks;
    }
    assert_eq!(depth, MAX_DEPTH);
    assert_eq!(strip(&blocks), "end");

    let list: String = (0..200).map(|i| format!("{}- item {i}\n", "  ".repeat(i))).collect();
    let blocks = parse(&list).unwrap();
    let text = strip(&blocks);
    assert!(text.contains("item 0") && text.contains("item 199"), "nothing past the depth is lost");
}

#[test]
fn too_much_is_plain() {
    let many = "*a* ".repeat(MAX_NODES);
    assert_eq!(parse(&many), None);
    assert_eq!(plain(&many), many.trim());
    assert_eq!(parse(&"a".repeat(MAX_INPUT_BYTES + 1)), None);
}

#[test]
fn hostile_inputs_are_quick() {
    let inputs = [
        "[".repeat(30_000),
        "*".repeat(30_000),
        "*a ".repeat(10_000),
        "`".repeat(30_000),
        "> - ".repeat(8_000),
        "| a ".repeat(8_000) + "|\n" + &"|-".repeat(8_000),
        "[a](".repeat(8_000),
        "<".repeat(30_000),
    ];
    for input in &inputs {
        let started = Instant::now();
        let _ = parse(input);
        assert!(started.elapsed() < Duration::from_secs(1), "slow on {:?}…", &input[..12]);
    }
}

#[test]
fn strip_gives_the_words() {
    assert_eq!(plain("# Hi\n\n**bold** and [a link](https://x.example)\n\n- one\n- two\n\n```\ncode\n```"), "Hi\nbold and a link\none\ntwo\ncode");
    assert_eq!(plain("| a | b |\n|---|---|\n| 1 | 2 |"), "a b\n1 2");
    assert_eq!(plain("   "), "");
}

#[test]
fn json_shape() {
    let blocks = parse("**a** [b](https://c.example)").unwrap();
    assert_eq!(
        serde_json::to_string(&blocks).unwrap(),
        r#"[{"kind":"para","inlines":[{"kind":"text","text":"a","style":{"bold":true,"italic":false,"strike":false}},{"kind":"text","text":" "},{"kind":"link","url":"https://c.example","host":"c.example","children":[{"kind":"text","text":"b"}]}]}]"#
    );
}

#[test]
fn a_wide_table_of_short_rows_is_plain() {
    // The parser fills every short row up to the head: 2000 columns × 4000 rows of one cell.
    let input = format!("{}|\n{}|\n{}", "| a ".repeat(2000), "|-".repeat(2000), "|x|\n".repeat(4000));
    assert!(input.len() < 32 * 1024);
    let started = Instant::now();
    assert_eq!(parse(&input), None);
    assert!(started.elapsed() < Duration::from_secs(1));
}

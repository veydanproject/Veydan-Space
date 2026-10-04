// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a page says about itself: `<title>` and the `og:` / `twitter:`
//! entries of its head. Nothing of the page is run or kept; what is read
//! becomes plain text.

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 400;
const MAX_IMAGE_REF_LEN: usize = 2048;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    /// As the page wrote it: may be relative.
    pub image: Option<String>,
}

/// `&amp;` and the like; an entity that is not known stays as written.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.char_indices().take(12).find(|(_, c)| *c == ';').map(|(j, _)| j);
        let decoded = end.and_then(|j| {
            let name = &tail[1..j];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name.strip_prefix('#').and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(h) => u32::from_str_radix(h, 16).ok(),
                    None => n.parse().ok(),
                })
                .and_then(char::from_u32),
            };
            ch.map(|c| (c, j))
        });
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &tail[j + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One line of plain text, no longer than `max`.
fn clean(s: &str, max: usize) -> Option<String> {
    let text: String = unescape(s).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    Some(if text.chars().count() > max { text.chars().take(max - 1).chain(['…']).collect() } else { text })
}

/// Attributes of one tag: `name="value"`, `name='value'`, `name=value`.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        if start == i {
            break;
        }
        let name = tag[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let from = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                value = tag[from..i].to_string();
                i += 1;
            } else {
                let from = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                value = tag[from..i].to_string();
            }
        }
        out.push((name, value));
    }
    out
}

/// Where a tag that starts at `from` ends, quotes respected.
fn tag_end(html: &str, from: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in html[from..].char_indices() {
        match (quote, c) {
            (None, '"') | (None, '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(from + i),
            _ => {}
        }
    }
    None
}

fn starts_with_tag(html: &str, at: usize, name: &str) -> bool {
    let rest = &html.as_bytes()[at + 1..];
    rest.len() > name.len()
        && rest[..name.len()].eq_ignore_ascii_case(name.as_bytes())
        && (rest[name.len()].is_ascii_whitespace() || rest[name.len()] == b'>' || rest[name.len()] == b'/')
}

pub fn read(html: &str) -> PageMeta {
    let mut og_title = None;
    let mut og_description = None;
    let mut description = None;
    let mut title = None;
    let mut site_name = None;
    let mut image = None;

    let mut at = 0;
    while let Some(i) = html[at..].find('<') {
        let open = at + i;
        if html[open..].starts_with("<!--") {
            match html[open..].find("-->") {
                Some(e) => at = open + e + 3,
                None => break,
            }
            continue;
        }
        let Some(close) = tag_end(html, open + 1) else { break };
        at = close + 1;
        if starts_with_tag(html, open, "meta") {
            let attrs = attributes(&html[open + 5..close]);
            let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
            let key = get("property").or_else(|| get("name")).unwrap_or_default().to_ascii_lowercase();
            let Some(content) = get("content") else { continue };
            match key.as_str() {
                "og:title" | "twitter:title" if og_title.is_none() => og_title = clean(content, MAX_TITLE_CHARS),
                "og:description" | "twitter:description" if og_description.is_none() => og_description = clean(content, MAX_DESCRIPTION_CHARS),
                "description" if description.is_none() => description = clean(content, MAX_DESCRIPTION_CHARS),
                "og:site_name" if site_name.is_none() => site_name = clean(content, MAX_TITLE_CHARS),
                "og:image" | "og:image:url" | "og:image:secure_url" | "twitter:image" if image.is_none() => {
                    let r = unescape(content.trim());
                    if !r.is_empty() && r.len() <= MAX_IMAGE_REF_LEN && !r.chars().any(|c| c.is_control() || c.is_whitespace()) {
                        image = Some(r);
                    }
                }
                _ => {}
            }
        } else if starts_with_tag(html, open, "title") && title.is_none() {
            let lower_end = html.as_bytes()[at..].windows(7).position(|w| w.eq_ignore_ascii_case(b"</title"));
            if let Some(e) = lower_end {
                title = clean(&html[at..at + e], MAX_TITLE_CHARS);
                at += e;
            }
        } else if starts_with_tag(html, open, "body") {
            // What a page says about itself is in its head.
            break;
        } else if starts_with_tag(html, open, "script") || starts_with_tag(html, open, "style") {
            let name: &[u8] = if starts_with_tag(html, open, "script") { b"</script" } else { b"</style" };
            match html.as_bytes()[at..].windows(name.len()).position(|w| w.eq_ignore_ascii_case(name)) {
                Some(e) => at += e,
                None => break,
            }
        }
    }

    PageMeta { title: og_title.or(title), description: og_description.or(description), site_name, image }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usual_page() {
        let html = r#"<!DOCTYPE html>
<html lang="en"><head>
  <meta charset="utf-8">
  <title>Plain &amp; simple — Example</title>
  <!-- <meta property="og:title" content="from a comment"> -->
  <meta name="description" content="Plain description">
  <meta property="og:title" content="Rust 2.0 &quot;released&quot; &#8212; news &#x2764;" />
  <meta content='Two
     lines   of text' property='og:description'>
  <META PROPERTY="OG:IMAGE" CONTENT="/img/cover.png?a=1&amp;b=2">
  <meta property=og:site_name content=Example>
  <meta property="og:title" content="the second one loses">
  <script>var s = "<meta property='og:image' content='https://evil.example/x.png'>";</script>
</head><body><meta property="og:image" content="https://body.example/x.png"><title>in body</title></body></html>"#;
        assert_eq!(
            read(html),
            PageMeta {
                title: Some("Rust 2.0 \"released\" — news ❤".into()),
                description: Some("Two lines of text".into()),
                site_name: Some("Example".into()),
                image: Some("/img/cover.png?a=1&b=2".into()),
            }
        );
    }

    #[test]
    fn a_page_with_a_title_only() {
        let m = read("<html><head><TITLE>\n  Только заголовок \n</TITLE></head></html>");
        assert_eq!(m.title.as_deref(), Some("Только заголовок"));
        assert!(m.description.is_none() && m.image.is_none());
        assert_eq!(read("<meta name=description content='d'><meta name='twitter:image' content='a.png'>").description.as_deref(), Some("d"));
    }

    #[test]
    fn what_is_read_is_plain_text() {
        let m = read(r#"<meta property="og:title" content="&lt;script&gt;alert(1)&lt;/script&gt; a > b"><meta property="og:image" content="java script:x">"#);
        assert_eq!(m.title.as_deref(), Some("<script>alert(1)</script> a > b"), "text, to be shown as text");
        assert!(m.image.is_none());
        let long = format!(r#"<meta property="og:title" content="{}">"#, "я".repeat(500));
        let t = read(&long).title.unwrap();
        assert_eq!(t.chars().count(), MAX_TITLE_CHARS);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn broken_pages_do_not_break_the_reader() {
        for html in ["", "<", "<meta", "<meta property=\"og:title\" content=\"never closed", "<title>never closed", "<!-- never closed", "<script>never closed", "&&&;&#;&#x;&#99999999;", "<meta content>", "<<<>>>", "<meta property='og:title' content=''>"] {
            let m = read(html);
            assert!(m.title.is_none() && m.image.is_none(), "{html}");
        }
        assert_eq!(unescape("a &amp b &unknown; &#65;&#x42;"), "a &amp b &unknown; AB");
    }
}

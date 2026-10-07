// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a bare address must look like to be a link.
//!
//! Only `https://` at the start of a word, up to the next space or a
//! character no address contains. Punctuation at its end belongs to the
//! sentence, as in the messages' tokenizer (`content/tokenize.ts`). So do
//! the `*` and `~` that close a mark open before the address, and only
//! those: a `~` of its own, as in `https://host/~`, stays. An address with a user name and
//! password before the host is refused: it shows one host and opens
//! another. So is one longer than `MAX_LINK_BYTES`.

pub const MAX_LINK_BYTES: usize = 2048;

pub(crate) const SCHEME: [char; 8] = ['h', 't', 't', 'p', 's', ':', '/', '/'];

/// Ends a candidate address.
pub(crate) fn is_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '{' | '}' | '\\')
}

/// A character before `https://` that makes it part of a word.
pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Marks open where an address starts: a run of `*` or `~` at its end may
/// close them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Open {
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) strike: bool,
}

/// The length of a candidate without the punctuation at its end. A `)`
/// is kept while the address opened as many brackets as it closes. Of a
/// run of `*` or `~`, only what closes the marks in `open` is cut, once.
pub(crate) fn trim_tail(s: &[char], open: Open) -> usize {
    let opens = s.iter().filter(|&&c| c == '(').count();
    let mut closes = s.iter().filter(|&&c| c == ')').count();
    let mut stars = open.bold || open.italic;
    let mut tildes = open.strike;
    let mut end = s.len();
    while end > SCHEME.len() {
        match s[end - 1] {
            '.' | ',' | ';' | ':' | '!' | '?' | ']' | '}' | '\'' | '"' | '»' | '…' => end -= 1,
            ')' if closes > opens => {
                closes -= 1;
                end -= 1;
            }
            '*' if stars => {
                let run = run_at(s, end, '*');
                let cut = [(open.bold && open.italic, 3), (open.bold, 2), (open.italic, 1)]
                    .into_iter()
                    .find(|&(can, n)| can && n <= run)
                    .map_or(0, |(_, n)| n);
                if cut == 0 {
                    break;
                }
                end -= cut;
                stars = false;
            }
            '~' if tildes && run_at(s, end, '~') >= 2 => {
                end -= 2;
                tildes = false;
            }
            _ => break,
        }
    }
    end
}

/// How many `c` end `s[..end]` after the scheme.
fn run_at(s: &[char], end: usize, c: char) -> usize {
    s[SCHEME.len()..end].iter().rev().take_while(|&&x| x == c).count()
}

/// A trimmed candidate that may be shown as a link.
pub(crate) fn is_valid(url: &str) -> bool {
    if url.len() > MAX_LINK_BYTES {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let (host_ok, port) = if let Some(v6) = authority.strip_prefix('[') {
        let Some(close) = v6.find(']') else {
            return false;
        };
        let host = &v6[..close];
        let after = &v6[close + 1..];
        let port = match after {
            "" => None,
            p => match p.strip_prefix(':') {
                Some(p) => Some(p),
                None => return false,
            },
        };
        (!host.is_empty() && host.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.'), port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        let ok = host.chars().any(char::is_alphanumeric)
            && host.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.');
        (ok, port)
    };
    let port_ok = match port {
        None => true,
        Some(p) => !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok(),
    };
    host_ok && port_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trimmed_in(s: &str, open: Open) -> String {
        let c: Vec<char> = s.chars().collect();
        c[..trim_tail(&c, open)].iter().collect()
    }

    fn trimmed(s: &str) -> String {
        trimmed_in(s, Open::default())
    }

    #[test]
    fn marks_trimmed_only_when_open() {
        let none = Open::default();
        let bold = Open { bold: true, ..none };
        let italic = Open { italic: true, ..none };
        let both = Open { bold: true, italic: true, ..none };
        let strike = Open { strike: true, ..none };
        let all = Open { bold: true, italic: true, strike: true };
        for s in ["https://a.com/~", "https://a.com/x~~", "https://a.com/*", "https://a.com/x**"] {
            assert_eq!(trimmed_in(s, none), s);
        }
        assert_eq!(trimmed("https://a.com/~."), "https://a.com/~");
        assert_eq!(trimmed_in("https://a.com/x**", bold), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x*", bold), "https://a.com/x*");
        assert_eq!(trimmed_in("https://a.com/x***", bold), "https://a.com/x*");
        assert_eq!(trimmed_in("https://a.com/x**", italic), "https://a.com/x*");
        assert_eq!(trimmed_in("https://a.com/x***", both), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x**", both), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x*", both), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/~", strike), "https://a.com/~");
        assert_eq!(trimmed_in("https://a.com/x~~", strike), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x~~~", strike), "https://a.com/x~");
        // Punctuation on either side of the run, and a run cut once.
        assert_eq!(trimmed_in("https://a.com/x.**!", bold), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x**.**", bold), "https://a.com/x**");
        assert_eq!(trimmed_in("https://a.com/x~~**", all), "https://a.com/x");
        assert_eq!(trimmed_in("https://a.com/x~~**", bold), "https://a.com/x~~");
        // Never into the scheme.
        assert_eq!(trimmed_in("https://**", bold), "https://");
        assert_eq!(trimmed_in("https://~~", strike), "https://");
    }

    #[test]
    fn tail_trimmed() {
        assert_eq!(trimmed("https://a.com/x."), "https://a.com/x");
        assert_eq!(trimmed("https://a.com/x?!.,;:"), "https://a.com/x");
        assert_eq!(trimmed("https://a.com/x]}'\"»…"), "https://a.com/x");
        assert_eq!(trimmed("https://a.com/x)"), "https://a.com/x");
        assert_eq!(trimmed("https://a.com/x)."), "https://a.com/x");
        assert_eq!(trimmed("https://en.wikipedia.org/wiki/Rust_(language)"), "https://en.wikipedia.org/wiki/Rust_(language)");
        assert_eq!(trimmed("https://en.wikipedia.org/wiki/Rust_(language))."), "https://en.wikipedia.org/wiki/Rust_(language)");
        assert_eq!(trimmed("https://a.com/(x))))"), "https://a.com/(x)");
        assert_eq!(trimmed("https://"), "https://");
        assert_eq!(trimmed("https://..."), "https://");
        assert_eq!(trimmed("https://a.com/~user/"), "https://a.com/~user/");
    }

    #[test]
    fn validity() {
        for ok in [
            "https://a.com",
            "https://example.com/path?q=1#frag",
            "https://sub.example.co.uk:8443/x",
            "https://пример.рф/путь",
            "https://[::1]/x",
            "https://[2001:db8::1]:443",
            "https://localhost",
            "https://a.com/x@y",
            "https://a.com?u=a@b",
        ] {
            assert!(is_valid(ok), "{ok}");
        }
        for bad in [
            "https://",
            "https:///x",
            "https://user@evil.com",
            "https://user:pass@evil.com/x",
            "https://good.com@evil.com",
            "https://a.com:",
            "https://a.com:99999",
            "https://a.com:12ab",
            "https://a_b.com",
            "https://a%2e.com",
            "https://...",
            "https://[]/",
            "https://[::1",
            "https://[::1]x",
            "https://[zz]/",
            "http://a.com",
            "ftp://a.com",
            "HTTPS://a.com",
        ] {
            assert!(!is_valid(bad), "{bad}");
        }
        let at_cap = format!("https://a.com/{}", "x".repeat(MAX_LINK_BYTES - 14));
        assert_eq!(at_cap.len(), MAX_LINK_BYTES);
        assert!(is_valid(&at_cap));
        assert!(!is_valid(&format!("{at_cap}x")));
    }
}

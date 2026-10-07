// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Marks and links into spans.
//!
//! One pass over the characters makes tokens: text, code, links, line
//! breaks and marks. Open marks wait on a stack; a mark that closes one
//! deeper in the stack turns the marks above it back into text, and marks
//! still open at the end are text too. A kind is open at most once, so the
//! stack never holds more than four marks and every character is looked
//! at a bounded number of times.
//!
//! The rules:
//! - `**`, `*` and `~~` open only before a non-space and close only after
//!   one, so `2 * 3 * 4` and a list of `* items` stay text. They may span
//!   lines.
//! - `{name}` opens a color of the palette and `{/}` closes it; a color
//!   inside a color, an unknown name and a `{/}` with nothing open are text.
//! - `` `code` `` stays on one line and everything inside is text, marks
//!   and addresses included.
//! - `\` before one of `\ * ~ ` { }` makes that character text.
//! - `https://…` at the start of a word is a link in any style; inside it
//!   no mark is read.
//!
//! The text the spans show is then brought to the one form again, since
//! taking marks out can leave a space at a line end or an empty line: so
//! `strip` is always its own normal form and what the spans show.

use crate::link;
use crate::normalize::MarkRun;
use crate::{Color, Span, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Bold,
    Italic,
    Strike,
    Color,
}

enum Tok {
    Text(String),
    Code(String),
    Link(String),
    Break,
    Mark { kind: Kind, open: bool, color: Option<Color>, lit: String, matched: bool },
}

/// `input` is already normalised. `markup` false finds links and line
/// breaks only.
pub(crate) fn parse(input: &str, markup: bool) -> Vec<Span> {
    let chars: Vec<char> = input.chars().collect();
    let mut scanner = Scanner::new(&chars, markup);
    scanner.scan();
    tidy(render(scanner.toks))
}

const NONE: usize = usize::MAX;

struct Scanner<'a> {
    c: &'a [char],
    markup: bool,
    toks: Vec<Tok>,
    stack: Vec<(Kind, usize)>,
    /// For each position, the next backtick on the same line, or `NONE`.
    next_tick: Vec<usize>,
}

fn is_markup(c: char) -> bool {
    matches!(c, '\\' | '*' | '~' | '`' | '{' | '}')
}

impl<'a> Scanner<'a> {
    fn new(c: &'a [char], markup: bool) -> Self {
        let mut next_tick = Vec::new();
        if markup {
            next_tick = vec![NONE; c.len()];
            let mut next = NONE;
            for i in (0..c.len()).rev() {
                match c[i] {
                    '\n' => next = NONE,
                    '`' => next = i,
                    _ => {}
                }
                next_tick[i] = next;
            }
        }
        Scanner { c, markup, toks: Vec::new(), stack: Vec::new(), next_tick }
    }

    fn scan(&mut self) {
        let c = self.c;
        let n = c.len();
        let mut i = 0;
        while i < n {
            let ch = c[i];
            if ch == '\n' {
                self.toks.push(Tok::Break);
                i += 1;
                continue;
            }
            if ch == 'h' {
                if let Some(len) = self.address(i) {
                    i += len;
                    continue;
                }
            }
            if self.markup {
                match ch {
                    '\\' if i + 1 < n && is_markup(c[i + 1]) => {
                        self.push_char(c[i + 1]);
                        i += 2;
                        continue;
                    }
                    '`' => {
                        let close = if i + 1 < n { self.next_tick[i + 1] } else { NONE };
                        if close != NONE && close > i + 1 {
                            self.toks.push(Tok::Code(c[i + 1..close].iter().collect()));
                            i = close + 1;
                            continue;
                        }
                    }
                    '*' | '~' => {
                        let mut end = i;
                        while end < n && c[end] == ch {
                            end += 1;
                        }
                        let can_close = i > 0 && !c[i - 1].is_whitespace();
                        let can_open = end < n && !c[end].is_whitespace();
                        if ch == '*' {
                            self.stars(end - i, can_open, can_close);
                        } else {
                            self.tildes(end - i, can_open, can_close);
                        }
                        i = end;
                        continue;
                    }
                    '{' => {
                        if let Some(len) = self.color_tag(i) {
                            i += len;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            self.push_char(ch);
            i += 1;
        }
    }

    /// A bare address at `i`: a link, or text when it is not a valid one.
    /// Either way it is taken whole, so nothing inside it is read as a mark
    /// or as another address.
    fn address(&mut self, i: usize) -> Option<usize> {
        let c = self.c;
        if !c[i..].starts_with(&link::SCHEME) || (i > 0 && link::is_word(c[i - 1])) {
            return None;
        }
        let mut end = i + link::SCHEME.len();
        while end < c.len() && !link::is_stop(c[end]) {
            end += 1;
        }
        let open = link::Open { bold: self.has(Kind::Bold), italic: self.has(Kind::Italic), strike: self.has(Kind::Strike) };
        let len = link::trim_tail(&c[i..end], open);
        let text: String = c[i..i + len].iter().collect();
        if link::is_valid(&text) {
            self.toks.push(Tok::Link(text));
        } else {
            self.push_str(&text);
        }
        Some(len)
    }

    fn stars(&mut self, mut len: usize, can_open: bool, can_close: bool) {
        if can_close {
            while len > 0 {
                match self.top() {
                    // `**a *b**`: the pair of stars closes the bold, not the italic.
                    Some(Kind::Italic) if len == 2 && self.has(Kind::Bold) => {
                        self.close(Kind::Bold, "**");
                        len -= 2;
                    }
                    Some(Kind::Italic) => {
                        self.close(Kind::Italic, "*");
                        len -= 1;
                    }
                    Some(Kind::Bold) if len >= 2 => {
                        self.close(Kind::Bold, "**");
                        len -= 2;
                    }
                    _ if len >= 2 && self.has(Kind::Bold) => {
                        self.close(Kind::Bold, "**");
                        len -= 2;
                    }
                    _ if self.has(Kind::Italic) => {
                        self.close(Kind::Italic, "*");
                        len -= 1;
                    }
                    _ => break,
                }
            }
        }
        if can_open {
            if len >= 2 && !self.has(Kind::Bold) {
                self.open(Kind::Bold, "**".into(), None);
                len -= 2;
            }
            if len >= 1 && !self.has(Kind::Italic) {
                self.open(Kind::Italic, "*".into(), None);
                len -= 1;
            }
        }
        if len > 0 {
            self.push_str(&"*".repeat(len));
        }
    }

    fn tildes(&mut self, mut len: usize, can_open: bool, can_close: bool) {
        if can_close && len >= 2 && self.has(Kind::Strike) {
            self.close(Kind::Strike, "~~");
            len -= 2;
        }
        if can_open && len >= 2 && !self.has(Kind::Strike) {
            self.open(Kind::Strike, "~~".into(), None);
            len -= 2;
        }
        if len > 0 {
            self.push_str(&"~".repeat(len));
        }
    }

    /// `{name}` or `{/}` at `i` when it opens or closes a color.
    fn color_tag(&mut self, i: usize) -> Option<usize> {
        let c = self.c;
        if c.get(i + 1) == Some(&'/') && c.get(i + 2) == Some(&'}') {
            if !self.has(Kind::Color) {
                return None;
            }
            self.close(Kind::Color, "{/}");
            return Some(3);
        }
        let mut end = i + 1;
        while end < c.len() && end - i <= 7 && c[end].is_ascii_lowercase() {
            end += 1;
        }
        if c.get(end) != Some(&'}') || self.has(Kind::Color) {
            return None;
        }
        let name: String = c[i + 1..end].iter().collect();
        let color = Color::from_name(&name)?;
        self.open(Kind::Color, format!("{{{name}}}"), Some(color));
        Some(end - i + 1)
    }

    fn top(&self) -> Option<Kind> {
        self.stack.last().map(|(k, _)| *k)
    }

    fn has(&self, kind: Kind) -> bool {
        self.stack.iter().any(|(k, _)| *k == kind)
    }

    fn open(&mut self, kind: Kind, lit: String, color: Option<Color>) {
        self.stack.push((kind, self.toks.len()));
        self.toks.push(Tok::Mark { kind, open: true, color, lit, matched: false });
    }

    /// Closes `kind`, which is open; marks opened after it stay text.
    fn close(&mut self, kind: Kind, lit: &str) {
        while let Some((k, at)) = self.stack.pop() {
            if k != kind {
                continue;
            }
            if let Tok::Mark { matched, .. } = &mut self.toks[at] {
                *matched = true;
            }
            self.toks.push(Tok::Mark { kind, open: false, color: None, lit: lit.into(), matched: true });
            return;
        }
    }

    fn push_char(&mut self, ch: char) {
        if let Some(Tok::Text(s)) = self.toks.last_mut() {
            s.push(ch);
        } else {
            self.toks.push(Tok::Text(ch.to_string()));
        }
    }

    fn push_str(&mut self, text: &str) {
        if let Some(Tok::Text(s)) = self.toks.last_mut() {
            s.push_str(text);
        } else {
            self.toks.push(Tok::Text(text.into()));
        }
    }
}

enum Piece {
    Text(String, Style),
    Link(String, Style),
    Break,
}

fn push_text(out: &mut Vec<Piece>, text: &str, style: &Style) {
    if text.is_empty() {
        return;
    }
    if let Some(Piece::Text(s, st)) = out.last_mut() {
        if st == style {
            s.push_str(text);
            return;
        }
    }
    out.push(Piece::Text(text.into(), style.clone()));
}

fn render(toks: Vec<Tok>) -> Vec<Piece> {
    let mut style = Style::default();
    let mut out = Vec::new();
    for tok in toks {
        match tok {
            Tok::Text(s) => push_text(&mut out, &s, &style),
            Tok::Code(s) => push_text(&mut out, &s, &Style { code: true, ..style.clone() }),
            Tok::Link(url) => out.push(Piece::Link(url, style.clone())),
            Tok::Break => out.push(Piece::Break),
            Tok::Mark { kind, open, color, matched: true, .. } => match kind {
                Kind::Bold => style.bold = open,
                Kind::Italic => style.italic = open,
                Kind::Strike => style.strike = open,
                Kind::Color => style.color = if open { color } else { None },
            },
            Tok::Mark { lit, .. } => push_text(&mut out, &lit, &style),
        }
    }
    out
}

/// The one form of the text the pieces show: at most three combining
/// marks in a row, no space at a line end, at most two empty lines in a
/// row, nothing blank at the start or the end. Links are kept whole.
fn tidy(pieces: Vec<Piece>) -> Vec<Span> {
    let mut run = MarkRun::default();
    let mut out: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match piece {
            Piece::Break => {
                run = MarkRun::default();
                trim_end(&mut out);
                let breaks = out.iter().rev().take(3).take_while(|p| matches!(p, Piece::Break)).count();
                if out.is_empty() || breaks >= 3 {
                    continue;
                }
                out.push(Piece::Break);
            }
            Piece::Link(url, style) => {
                for ch in url.chars() {
                    run.keep(ch);
                }
                out.push(Piece::Link(url, style));
            }
            Piece::Text(text, style) => {
                let mut kept: String = text.chars().filter(|&ch| run.keep(ch)).collect();
                if out.is_empty() {
                    kept = kept.trim_start().to_string();
                }
                if kept.is_empty() {
                    continue;
                }
                push_text(&mut out, &kept, &style);
            }
        }
    }
    trim_end(&mut out);
    while matches!(out.last(), Some(Piece::Break)) {
        out.pop();
        trim_end(&mut out);
    }
    out.into_iter()
        .map(|p| match p {
            Piece::Text(text, style) => Span::Text { text, style },
            Piece::Link(url, style) => Span::Link { text: url.clone(), url, style },
            Piece::Break => Span::Break,
        })
        .collect()
}

/// Drops whitespace at the end of the last line built so far.
fn trim_end(out: &mut Vec<Piece>) {
    while let Some(Piece::Text(s, _)) = out.last_mut() {
        let keep = s.trim_end().len();
        if keep > 0 {
            s.truncate(keep);
            return;
        }
        out.pop();
    }
}

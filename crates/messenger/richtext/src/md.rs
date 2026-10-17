// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messages written in Markdown.
//!
//! A message the author marked as Markdown (`"fmt":"md"` in its envelope)
//! is parsed here into blocks and inlines that the UI renders as elements
//! and text nodes, never as HTML. The parser is CommonMark with tables,
//! strikethrough and task lists (`pulldown-cmark`); only its events are
//! read, its HTML writer is not even built.
//!
//! What the author cannot do with it:
//! - HTML, inline or as a block, is the text it is.
//! - A picture is never loaded: `![alt](url)` is a link to it, its text
//!   the alt.
//! - A link leads only to `http(s)://` without a user name before the host
//!   (it would show one host and open another) or to `veydan://`; it
//!   carries the host it opens, which the UI shows beside its text. Any
//!   other destination is shown as text in brackets after the words.
//! - Quotes and lists deeper than `MAX_DEPTH` go on at that depth; a text
//!   of more than `MAX_NODES` pieces, or longer than `MAX_INPUT_BYTES`, is
//!   not parsed at all and is shown as plain text.
//!
//! `strip` gives the text without the marks: what the list of chats, a
//! push and a quote show of such a message.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::link::MAX_LINK_BYTES;

/// Longer texts are not parsed (a message holds at most 32 KiB).
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// Quotes and lists inside each other.
pub const MAX_DEPTH: usize = 6;
/// Blocks and inlines of one text.
pub const MAX_NODES: usize = 4096;
/// The name of a language over a block of code.
pub const MAX_LANG_CHARS: usize = 24;
/// Levels of headings; deeper ones are shown as the last.
pub const MAX_HEADING: u8 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MdStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

impl MdStyle {
    /// Plain words: left out of the JSON, which holds a style per piece.
    pub fn is_plain(&self) -> bool {
        !self.bold && !self.italic && !self.strike
    }
}

/// A piece of a line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MdInline {
    /// `style` is left out for plain words.
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "MdStyle::is_plain")]
        style: MdStyle,
    },
    /// Shown as it is, in a fixed-width font.
    Code { text: String },
    /// `host`: what `http(s)://` opens, shown beside the words; `null` for
    /// `veydan://`. `children` never hold a link.
    Link { url: String, host: Option<String>, children: Vec<MdInline> },
    Break,
}

/// A block of a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MdBlock {
    Para { inlines: Vec<MdInline> },
    /// `level` 1..=`MAX_HEADING`.
    Heading { level: u8, inlines: Vec<MdInline> },
    /// `lang`: the first word after the fence, lowercase, when it looks
    /// like the name of a language.
    Code { lang: Option<String>, text: String },
    Quote { blocks: Vec<MdBlock> },
    List { ordered: bool, start: u32, items: Vec<MdItem> },
    /// Rows of cells; a cell is a line.
    Table { head: Vec<Vec<MdInline>>, rows: Vec<Vec<Vec<MdInline>>> },
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MdItem {
    /// A task list: done or not.
    pub task: Option<bool>,
    pub blocks: Vec<MdBlock>,
}

/// The blocks of a text, or `None` when it is too long or too many pieces
/// to be shown as Markdown: it is then shown as plain text.
pub fn parse(input: &str) -> Option<Vec<MdBlock>> {
    if input.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    if table_cells(input) <= MAX_NODES {
        options |= Options::ENABLE_TABLES;
    }
    let mut b = Builder::new();
    for event in Parser::new_ext(input, options) {
        b.event(event);
        if b.nodes > MAX_NODES {
            return None;
        }
    }
    b.finish()
}

/// The text of a Markdown message without its marks, lines joined by `\n`;
/// the text itself when it is not parsed.
pub fn plain(input: &str) -> String {
    match parse(input) {
        Some(blocks) => strip(&blocks),
        None => input.trim().to_string(),
    }
}

/// The text the blocks show, lines joined by `\n`.
pub fn strip(blocks: &[MdBlock]) -> String {
    let mut lines = Vec::new();
    strip_blocks(blocks, &mut lines);
    lines.join("\n").trim().to_string()
}

fn strip_blocks(blocks: &[MdBlock], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            MdBlock::Para { inlines } | MdBlock::Heading { inlines, .. } => out.push(inline_text(inlines)),
            MdBlock::Code { text, .. } => out.push(text.clone()),
            MdBlock::Quote { blocks } => strip_blocks(blocks, out),
            MdBlock::List { items, .. } => items.iter().for_each(|i| strip_blocks(&i.blocks, out)),
            MdBlock::Table { head, rows } => {
                for row in std::iter::once(head).chain(rows) {
                    out.push(row.iter().map(|c| inline_text(c)).collect::<Vec<_>>().join(" "));
                }
            }
            MdBlock::Rule => {}
        }
    }
}

fn inline_text(inlines: &[MdInline]) -> String {
    let mut s = String::new();
    for i in inlines {
        match i {
            MdInline::Text { text, .. } | MdInline::Code { text } => s.push_str(text),
            MdInline::Link { children, .. } => s.push_str(&inline_text(children)),
            MdInline::Break => s.push('\n'),
        }
    }
    s
}

/// Where a link may lead: the destination (its scheme in lowercase, as the
/// app opens it) and the host it opens.
fn target(url: &str) -> Option<(String, Option<String>)> {
    if url.is_empty() || url.len() > MAX_LINK_BYTES || url.chars().any(hidden) {
        return None;
    }
    let (scheme, rest) = url.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "veydan" if rest.starts_with("//") && rest.len() > 2 => Some((format!("{scheme}:{rest}"), None)),
        "https" | "http" => {
            let after = rest.strip_prefix("//")?;
            let authority = &after[..after.find(['/', '?', '#']).unwrap_or(after.len())];
            // `https://bank.example@evil.example/` opens the second.
            if authority.contains(['@', '\\']) {
                return None;
            }
            let (host, port) = match authority.strip_prefix('[') {
                Some(v6) => {
                    let end = v6.find(']')?;
                    let ok = end > 0 && v6[..end].chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
                    (ok.then(|| &authority[..end + 2])?, &v6[end + 1..])
                }
                None => match authority.split_once(':') {
                    Some((h, _)) => (h, &authority[h.len()..]),
                    None => (authority, ""),
                },
            };
            // The host shown is the host opened: letters, digits, dots and
            // dashes of ASCII only (no other script that draws a lookalike,
            // no `%2e` the browser turns into a dot), and a port of digits.
            let plain = host.starts_with('[') || host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            let port_ok = port.is_empty() || (port.len() > 1 && port[1..].chars().all(|c| c.is_ascii_digit()) && port.starts_with(':'));
            if host.is_empty() || !plain || !port_ok {
                return None;
            }
            Some((format!("{scheme}:{rest}"), Some(host.to_ascii_lowercase())))
        }
        _ => None,
    }
}

/// The cells the tables of a text could hold. The parser fills every row
/// of a table up to the cells of its head while it reads the text, before
/// anything here can count them: a text whose tables could hold more
/// than `MAX_NODES` is read without tables. Counted over every line after
/// a row of dashes up to a blank line: never less than the parser makes.
fn table_cells(input: &str) -> usize {
    let (mut total, mut cols) = (0usize, 0usize);
    for line in input.lines() {
        let t = line.trim_start_matches(|c: char| c == '>' || c.is_whitespace()).trim_end();
        if t.is_empty() {
            cols = 0;
        } else if cols > 0 {
            total = total.saturating_add(cols);
        } else if t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
            cols = t.split('|').filter(|s| !s.trim().is_empty()).count().max(1);
            total = total.saturating_add(cols.saturating_mul(2));
        }
    }
    total
}

/// Characters a destination must not hide: controls, spaces, and the ones
/// that change the order or the look of what is around them.
fn hidden(c: char) -> bool {
    c.is_control()
        || c.is_whitespace()
        || matches!(c, '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

fn lang_of(info: &str) -> Option<String> {
    let word = info.split_whitespace().next()?;
    let ok = word.chars().count() <= MAX_LANG_CHARS
        && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '#' | '.' | '_'));
    ok.then(|| word.to_ascii_lowercase())
}

fn heading(level: HeadingLevel) -> u8 {
    (level as u8).clamp(1, MAX_HEADING)
}

enum Frame {
    Root(Vec<MdBlock>),
    Quote(Vec<MdBlock>),
    List { ordered: bool, start: u32, items: Vec<MdItem> },
    Item(MdItem),
    Table { head: Vec<Vec<MdInline>>, rows: Vec<Vec<Vec<MdInline>>>, row: Vec<Vec<MdInline>> },
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Para,
    Heading(u8),
    Cell,
}

struct OpenLink {
    target: Option<(String, Option<String>)>,
    dest: String,
    children: Vec<MdInline>,
}

struct Builder {
    frames: Vec<Frame>,
    /// Whether each open quote, list, item and table made a frame (one
    /// past `MAX_DEPTH` does not).
    opened: Vec<bool>,
    line: Option<(Line, Vec<MdInline>)>,
    code: Option<(Option<String>, String)>,
    links: Vec<OpenLink>,
    bold: u32,
    italic: u32,
    strike: u32,
    nodes: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            frames: vec![Frame::Root(Vec::new())],
            opened: Vec::new(),
            line: None,
            code: None,
            links: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            nodes: 0,
        }
    }

    fn finish(mut self) -> Option<Vec<MdBlock>> {
        self.close_line();
        while self.frames.len() > 1 {
            self.close_frame();
        }
        match self.frames.pop() {
            Some(Frame::Root(blocks)) => Some(blocks),
            _ => None,
        }
    }

    fn depth(&self) -> usize {
        self.frames.iter().filter(|f| matches!(f, Frame::Quote(_) | Frame::List { .. })).count()
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(s) => match &mut self.code {
                Some((_, text)) => text.push_str(&s),
                None => self.text(&s),
            },
            Event::Code(s) => self.push_inline(MdInline::Code { text: s.to_string() }),
            Event::Html(s) | Event::InlineHtml(s) => self.lines(&s),
            Event::SoftBreak | Event::HardBreak => self.push_inline(MdInline::Break),
            Event::Rule => {
                self.close_line();
                self.push_block(MdBlock::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Frame::Item(item)) = self.frames.last_mut() {
                    item.task = Some(done);
                }
            }
            // Not enabled: math and footnotes are their text.
            Event::InlineMath(s) | Event::DisplayMath(s) | Event::FootnoteReference(s) => self.text(&s),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.open_line(Line::Para),
            Tag::Heading { level, .. } => self.open_line(Line::Heading(heading(level))),
            Tag::BlockQuote(_) => self.open_frame(|| Frame::Quote(Vec::new()), true),
            Tag::List(start) => {
                let ordered = start.is_some();
                let start = start.unwrap_or(1).min(u64::from(u32::MAX)) as u32;
                self.open_frame(|| Frame::List { ordered, start, items: Vec::new() }, true);
            }
            Tag::Item => {
                // An item outside a list (its list was too deep) is a block of its own.
                let in_list = matches!(self.frames.last(), Some(Frame::List { .. }));
                self.close_line();
                self.opened.push(in_list);
                if in_list {
                    self.frames.push(Frame::Item(MdItem { task: None, blocks: Vec::new() }));
                }
            }
            Tag::CodeBlock(kind) => {
                self.close_line();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => lang_of(&info),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((lang, String::new()));
            }
            Tag::Table(_) => self.open_frame(|| Frame::Table { head: Vec::new(), rows: Vec::new(), row: Vec::new() }, false),
            Tag::TableHead | Tag::TableRow => {
                if let Some(Frame::Table { row, .. }) = self.frames.last_mut() {
                    row.clear();
                }
            }
            Tag::TableCell => self.line = Some((Line::Cell, Vec::new())),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.links.push(OpenLink { target: target(&dest_url), dest: dest_url.to_string(), children: Vec::new() })
            }
            // Not enabled.
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock => self.close_line(),
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::Table => {
                self.close_line();
                if self.opened.pop() == Some(true) {
                    self.close_frame();
                }
            }
            TagEnd::CodeBlock => {
                if let Some((lang, mut text)) = self.code.take() {
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.push_block(MdBlock::Code { lang, text });
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(Frame::Table { head, rows, row }) = self.frames.last_mut() {
                    let cells = std::mem::take(row);
                    if matches!(tag, TagEnd::TableHead) {
                        *head = cells;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            TagEnd::TableCell => {
                let cell = match self.line.take() {
                    Some((Line::Cell, inlines)) => inlines,
                    other => {
                        self.line = other;
                        return;
                    }
                };
                // Every cell counts, empty ones too: a short row is filled up to the head.
                self.nodes += 1;
                if let Some(Frame::Table { row, .. }) = self.frames.last_mut() {
                    row.push(cell);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {
                let Some(link) = self.links.pop() else { return };
                match link.target {
                    Some((url, host)) => self.push_inline(MdInline::Link { url, host, children: link.children }),
                    None => {
                        // Moved out of the link: they were counted inside it.
                        self.nodes = self.nodes.saturating_sub(link.children.len());
                        // The address is code, never a link again (not even to the
                        // UI's finder of bare links); `<a@b.example>`: the words are
                        // the address.
                        let words = inline_text(&link.children);
                        let same = words == link.dest || link.dest.strip_prefix("mailto:") == Some(words.as_str());
                        if same {
                            self.push_inline(MdInline::Code { text: words });
                        } else {
                            for child in link.children {
                                self.push_inline(child);
                            }
                            if !link.dest.is_empty() {
                                self.text(" (");
                                self.push_inline(MdInline::Code { text: link.dest });
                                self.text(")");
                            }
                        }
                    }
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    /// A quote, list or table; past `MAX_DEPTH` a quote or list goes on in
    /// the frame it is in.
    fn open_frame(&mut self, make: impl FnOnce() -> Frame, nests: bool) {
        self.close_line();
        let open = !nests || self.depth() < MAX_DEPTH;
        self.opened.push(open);
        if open {
            self.frames.push(make());
        }
    }

    fn close_frame(&mut self) {
        let block = match self.frames.pop() {
            Some(Frame::Quote(blocks)) => MdBlock::Quote { blocks },
            Some(Frame::List { ordered, start, items }) => MdBlock::List { ordered, start, items },
            Some(Frame::Item(item)) => {
                if let Some(Frame::List { items, .. }) = self.frames.last_mut() {
                    items.push(item);
                    self.nodes += 1;
                } else {
                    item.blocks.into_iter().for_each(|b| self.push_block(b));
                }
                return;
            }
            Some(Frame::Table { head, rows, .. }) => MdBlock::Table { head, rows },
            Some(root @ Frame::Root(_)) => {
                self.frames.push(root);
                return;
            }
            None => return,
        };
        self.push_block(block);
    }

    fn push_block(&mut self, block: MdBlock) {
        self.nodes += 1;
        match self.frames.last_mut() {
            Some(Frame::Root(blocks) | Frame::Quote(blocks) | Frame::Item(MdItem { blocks, .. })) => blocks.push(block),
            Some(Frame::List { items, .. }) => items.push(MdItem { task: None, blocks: vec![block] }),
            // Nothing but rows of cells stands in a table.
            Some(Frame::Table { .. }) | None => {}
        }
    }

    fn open_line(&mut self, kind: Line) {
        self.close_line();
        self.line = Some((kind, Vec::new()));
    }

    fn close_line(&mut self) {
        match self.line.take() {
            Some((Line::Cell, inlines)) => self.line = Some((Line::Cell, inlines)),
            Some((kind, mut inlines)) => {
                while matches!(inlines.last(), Some(MdInline::Break)) {
                    inlines.pop();
                }
                if inlines.is_empty() {
                    return;
                }
                self.push_block(match kind {
                    Line::Heading(level) => MdBlock::Heading { level, inlines },
                    _ => MdBlock::Para { inlines },
                });
            }
            None => {}
        }
    }

    /// Where an inline goes: the open link, else the open line (a list's
    /// tight item has text without a paragraph: it opens one).
    fn inlines(&mut self) -> &mut Vec<MdInline> {
        if let Some(link) = self.links.last_mut() {
            return &mut link.children;
        }
        &mut self.line.get_or_insert_with(|| (Line::Para, Vec::new())).1
    }

    fn push_inline(&mut self, inline: MdInline) {
        let in_link = !self.links.is_empty();
        let to = self.inlines();
        let added = match inline {
            // A link inside a link (a picture that is a link) is its words.
            MdInline::Link { children, .. } if in_link => {
                // Counted already, inside the inner link.
                to.extend(children);
                0
            }
            MdInline::Text { text, style } => match to.last_mut() {
                Some(MdInline::Text { text: last, style: s }) if *s == style => {
                    last.push_str(&text);
                    0
                }
                _ => {
                    to.push(MdInline::Text { text, style });
                    1
                }
            },
            other => {
                to.push(other);
                1
            }
        };
        self.nodes += added;
    }

    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let style = MdStyle { bold: self.bold > 0, italic: self.italic > 0, strike: self.strike > 0 };
        self.push_inline(MdInline::Text { text: s.to_string(), style });
    }

    /// HTML as the text it is, line by line.
    fn lines(&mut self, s: &str) {
        let mut first = true;
        for line in s.split('\n') {
            if !first {
                self.push_inline(MdInline::Break);
            }
            first = false;
            self.text(line);
        }
    }
}

/// Whether a text, read as Markdown, holds a link: one of its marks, or an
/// address its words spell once the marks and entities are read
/// (`https:&#47;&#47;…`). `has_link` is the reader's test of plain words.
pub fn holds_link(input: &str, has_link: impl Fn(&str) -> bool) -> bool {
    fn inlines(list: &[MdInline]) -> bool {
        list.iter().any(|i| matches!(i, MdInline::Link { .. }))
    }
    fn blocks(list: &[MdBlock]) -> bool {
        list.iter().any(|b| match b {
            MdBlock::Para { inlines: i } | MdBlock::Heading { inlines: i, .. } => inlines(i),
            MdBlock::Quote { blocks: b } => blocks(b),
            MdBlock::List { items, .. } => items.iter().any(|it| blocks(&it.blocks)),
            MdBlock::Table { head, rows } => head.iter().chain(rows.iter().flatten()).any(|c| inlines(c)),
            MdBlock::Code { .. } | MdBlock::Rule => false,
        })
    }
    has_link(input) || parse(input).is_some_and(|b| blocks(&b)) || has_link(&plain(input))
}

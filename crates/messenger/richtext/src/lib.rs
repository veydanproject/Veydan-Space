// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The small markup of a profile's bio.
//!
//! A bio is plain text with a few marks: `**bold**`, `*italic*`,
//! `~~strike~~`, `` `code` `` and colors of a fixed palette,
//! `{red}text{/}`. A bare `https://` address becomes a link. Nothing else
//! is markup: headings, lists, quotes, images, HTML and `[text](url)` stay
//! the text they are. A mark that is not closed is text too, and `\` makes
//! the next mark character text.
//!
//! Every text is brought to one form first (`normalize`): line breaks,
//! controls, bidi overrides, piles of combining marks, blank lines and
//! length, so what one device saves another reads the same. Then it is
//! parsed into spans that the UI renders as text nodes and anchors, never
//! as HTML. `strip` gives the text without the marks: the `about` other
//! Nostr clients see, always exactly the text of the spans and already in
//! the one form. `plain` reads an `about` written by another client: text
//! and links only.
//!
//! - `normalize`: the one form of a text.
//! - `parse`: marks and links into spans.
//! - `link`: what a bare address must look like to be a link.
//! - `marks`: the marks that pile up over a letter.
//!
//! Everything here is pure, and takes time linear in the input up to
//! `MAX_INPUT_BYTES`, the most of it that is read.

mod link;
mod marks;
mod normalize;
mod parse;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use link::MAX_LINK_BYTES;

/// Unicode scalar values of a normalised text.
pub const MAX_CHARS: usize = 2000;
/// Lines of a normalised text; the rest is joined onto the last one.
pub const MAX_LINES: usize = 40;
/// Bytes of an input that are read; the rest is never looked at. Far more
/// than any text of `MAX_CHARS` with its spaces, so only padding meant to
/// cost the reader time is lost.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// The refusal of `check`.
pub const ERR_TOO_LONG: &str = "bio_too_long";

/// The palette. The UI maps each to a token readable in both themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
    Gray,
}

impl Color {
    pub const ALL: [Color; 9] = [
        Color::Red,
        Color::Orange,
        Color::Yellow,
        Color::Green,
        Color::Teal,
        Color::Blue,
        Color::Purple,
        Color::Pink,
        Color::Gray,
    ];

    /// The name written in the markup: `{red}`.
    pub fn name(self) -> &'static str {
        match self {
            Color::Red => "red",
            Color::Orange => "orange",
            Color::Yellow => "yellow",
            Color::Green => "green",
            Color::Teal => "teal",
            Color::Blue => "blue",
            Color::Purple => "purple",
            Color::Pink => "pink",
            Color::Gray => "gray",
        }
    }

    /// Lowercase names only; anything else is not a color.
    pub fn from_name(name: &str) -> Option<Color> {
        Color::ALL.into_iter().find(|c| c.name() == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub color: Option<Color>,
}

/// A piece of a parsed text. Text never contains a line break: lines are
/// separated by `Break`. A link's text is its address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Span {
    Text { text: String, style: Style },
    Link { url: String, text: String, style: Style },
    Break,
}

/// The one form of a text, cut to `MAX_CHARS`. Idempotent.
pub fn normalize(input: &str) -> String {
    normalize::normalize(input)
}

/// The one form of a text to be saved, or `ERR_TOO_LONG` when it is
/// longer than `MAX_CHARS` (or its input longer than `MAX_INPUT_BYTES`):
/// a bio is never cut silently on save.
pub fn check(input: &str) -> Result<String, &'static str> {
    normalize::check(input)
}

/// The markup of a bio into spans.
pub fn parse(input: &str) -> Vec<Span> {
    parse::parse(&normalize(input), true)
}

/// A text without markup (an `about` written by another client): only
/// line breaks and links are found in it.
pub fn plain(input: &str) -> Vec<Span> {
    parse::parse(&normalize(input), false)
}

/// The text of `parse(input)` without the marks, lines joined by `\n`:
/// what other clients get as `about`. Its own normal form.
pub fn strip(input: &str) -> String {
    text_of(&parse(input))
}

/// The text the spans show, lines joined by `\n`.
pub fn text_of(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Text { text, .. } | Span::Link { text, .. } => out.push_str(text),
            Span::Break => out.push('\n'),
        }
    }
    out
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! 3-way merge shared by history merge and vault sync. Line endings are
//! normalized to LF first so CRLF/LF differences never count as changes.
//! Conflicts are returned as structured blocks; the UI never parses markers.

use diffy::{ConflictStyle, MergeOptions};
use serde::Serialize;

/// Long markers so real text lines like `=======` cannot be mistaken for one.
const MARKER_LEN: usize = 32;
/// Prefix hiding user lines that look like a marker while diffy runs.
const ESCAPE: char = '\u{2060}';

#[derive(Debug, Serialize, Clone)]
pub struct MergeBlock {
    /// "normal" | "conflict"
    pub kind: String,
    /// Text of a normal block
    pub text: String,
    /// Conflict sides
    pub ours: String,
    pub theirs: String,
}

impl MergeBlock {
    fn normal(text: String) -> Self {
        Self {
            kind: "normal".into(),
            text,
            ours: String::new(),
            theirs: String::new(),
        }
    }

    fn conflict(ours: String, theirs: String) -> Self {
        Self {
            kind: "conflict".into(),
            text: String::new(),
            ours,
            theirs,
        }
    }

    fn is_conflict(&self) -> bool {
        self.kind == "conflict"
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct MergeResult {
    /// Clean merged text; with conflicts it still carries markers, use `blocks` instead
    pub content: String,
    pub has_conflicts: bool,
    pub blocks: Vec<MergeBlock>,
}

impl MergeResult {
    /// For a conflict the caller already decided on (e.g. no common ancestor):
    /// a clean merge of two different texts still needs a choice.
    pub fn or_whole_texts(self, ours: &str, theirs: &str) -> Self {
        let (o, t) = (prepare(ours), prepare(theirs));
        if self.has_conflicts || o == t {
            return self;
        }
        Self {
            content: self.content,
            has_conflicts: true,
            blocks: vec![MergeBlock::conflict(unescape(&o), unescape(&t))],
        }
    }
}

pub fn normalize_eol(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn marker(ch: char) -> String {
    std::iter::repeat_n(ch, MARKER_LEN).collect()
}

fn looks_like_marker(line: &str) -> bool {
    ['<', '=', '>']
        .iter()
        .any(|&c| line.starts_with(&marker(c)))
}

/// LF endings, a final newline, and user lines that mimic a marker escaped.
fn prepare(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    for line in normalize_eol(s).split_inclusive('\n') {
        if looks_like_marker(line) {
            out.push(ESCAPE);
        }
        out.push_str(line);
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn unescape(s: &str) -> String {
    s.split_inclusive('\n')
        .map(|l| l.strip_prefix(ESCAPE).unwrap_or(l))
        .collect()
}

pub fn merge3(ancestor: &str, ours: &str, theirs: &str) -> MergeResult {
    let (a, o, t) = (prepare(ancestor), prepare(ours), prepare(theirs));
    let mut opts = MergeOptions::new();
    opts.set_conflict_style(ConflictStyle::Merge)
        .set_conflict_marker_length(MARKER_LEN);
    match opts.merge(&a, &o, &t) {
        Ok(content) => {
            let content = unescape(&content);
            MergeResult {
                blocks: vec![MergeBlock::normal(content.clone())],
                content,
                has_conflicts: false,
            }
        }
        Err(marked) => {
            let mut blocks: Vec<MergeBlock> = parse_blocks(&marked)
                .into_iter()
                .map(|b| MergeBlock {
                    text: unescape(&b.text),
                    ours: unescape(&b.ours),
                    theirs: unescape(&b.theirs),
                    ..b
                })
                .collect();
            // Never hand the UI "conflict, but nothing to choose": fall back to whole texts.
            if !blocks.iter().any(MergeBlock::is_conflict) {
                blocks = vec![MergeBlock::conflict(unescape(&o), unescape(&t))];
            }
            MergeResult {
                blocks,
                content: unescape(&marked),
                has_conflicts: true,
            }
        }
    }
}

enum Marker {
    Start,
    Sep,
    End,
}

/// Parser position: outside a conflict, in the ours side, or in the theirs side.
#[derive(Default)]
struct Parser {
    blocks: Vec<MergeBlock>,
    normal: String,
    ours: Option<String>,
    theirs: Option<String>,
}

impl Parser {
    /// Earliest marker in the line that is valid for the current state.
    fn find_marker(&self, line: &str) -> Option<(usize, Marker)> {
        let mut found: Option<(usize, Marker)> = None;
        let candidates = [
            (Marker::Start, '<', self.ours.is_none()),
            (
                Marker::Sep,
                '=',
                self.ours.is_some() && self.theirs.is_none(),
            ),
            (Marker::End, '>', self.theirs.is_some()),
        ];
        for (kind, ch, valid) in candidates {
            if !valid {
                continue;
            }
            if let Some(pos) = line.find(&marker(ch)) {
                // An escaped user line is text, not a marker.
                if line[..pos].ends_with(ESCAPE) {
                    continue;
                }
                if found.as_ref().map(|(p, _)| pos < *p).unwrap_or(true) {
                    found = Some((pos, kind));
                }
            }
        }
        found
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match (&mut self.ours, &mut self.theirs) {
            (_, Some(t)) => t.push_str(text),
            (Some(o), None) => o.push_str(text),
            _ => self.normal.push_str(text),
        }
    }

    fn flush_normal(&mut self) {
        if !self.normal.is_empty() {
            self.blocks
                .push(MergeBlock::normal(std::mem::take(&mut self.normal)));
        }
    }

    fn close_conflict(&mut self) {
        if let Some(ours) = self.ours.take() {
            self.blocks.push(MergeBlock::conflict(
                ours,
                self.theirs.take().unwrap_or_default(),
            ));
        }
    }

    fn apply(&mut self, m: Marker) {
        match m {
            Marker::Start => {
                self.flush_normal();
                self.ours = Some(String::new());
            }
            Marker::Sep => self.theirs = Some(String::new()),
            Marker::End => self.close_conflict(),
        }
    }
}

/// Split diffy's marked output into normal and conflict blocks.
/// Block texts keep their trailing newline so joining them restores the file.
/// A marker glued to the end of a text line still counts; an unclosed
/// conflict at EOF is kept, not dropped.
fn parse_blocks(marked: &str) -> Vec<MergeBlock> {
    let mut p = Parser::default();
    for line in marked.split_inclusive('\n') {
        match p.find_marker(line) {
            Some((pos, m)) => {
                // Text before the marker belongs to the current side; a glued
                // marker means the line had no newline, so give it one.
                let before = &line[..pos];
                if !before.is_empty() {
                    p.push_text(before);
                    if !before.ends_with('\n') {
                        p.push_text("\n");
                    }
                }
                p.apply(m);
            }
            None => p.push_text(line),
        }
    }
    p.close_conflict();
    p.flush_normal();
    p.blocks
}

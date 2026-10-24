// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The slash commands a bot says it answers, `veydan_commands` of its
//! kind 0: `[{"name":"ping","about":"Answers pong"}]`. The app offers them
//! as a «/» menu in a direct chat with the bot.
//!
//! The list is untrusted on both ways: what a bot asks to publish and what
//! a kind 0 says go through `clean_list`, which keeps the first
//! `MAX_COMMANDS` entries that pass and drops the rest. A name is 1 to
//! `MAX_NAME` characters of `a-z 0-9 _` (lowercased, a leading `/`
//! dropped), the form `/name` a bot reads; a name said twice keeps its
//! first line. A line is trimmed, its runs of white space and control
//! characters made one space, and is at most `MAX_ABOUT` characters, else
//! the entry is dropped.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Commands in one profile: a menu, not a manual.
pub const MAX_COMMANDS: usize = 64;
/// Characters of a name, without its `/`.
pub const MAX_NAME: usize = 32;
/// Characters of the line about a command.
pub const MAX_ABOUT: usize = 120;

/// A command a bot answers: `/name`, and a line about it.
#[derive(TS, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct BotCommand {
    /// Without the `/`: `ping` for `/ping`.
    pub name: String,
    /// What it does, one line; may be empty.
    #[serde(default)]
    pub about: String,
}

/// The name as a bot reads it, or `None` when it is no name.
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    let name = name.strip_prefix('/').unwrap_or(name).to_ascii_lowercase();
    let ok = (1..=MAX_NAME).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    ok.then_some(name)
}

/// The text on one line: its runs of white space and control characters
/// made one space, trimmed.
fn one_line(about: &str) -> String {
    about.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The line on one line, or `None` when it is too long.
fn clean_about(about: &str) -> Option<String> {
    let line = one_line(about);
    (line.chars().count() <= MAX_ABOUT).then_some(line)
}

/// A description made a line that passes: on one line, cut at
/// `MAX_ABOUT` characters (a bot's own description may be longer; the
/// menu shows a line).
pub fn fit_about(description: &str) -> String {
    let line = one_line(description);
    match line.char_indices().nth(MAX_ABOUT) {
        Some((at, _)) => line[..at].trim_end().to_string(),
        None => line,
    }
}

/// The commands that pass, in their order, at most `MAX_COMMANDS`.
pub fn clean_list(list: &[BotCommand]) -> Vec<BotCommand> {
    let mut out: Vec<BotCommand> = Vec::new();
    for c in list {
        if out.len() >= MAX_COMMANDS {
            break;
        }
        // Longer than any line kept: not looked at.
        if c.name.len() > MAX_NAME * 4 || c.about.len() > MAX_ABOUT * 8 {
            continue;
        }
        let (Some(name), Some(about)) = (clean_name(&c.name), clean_about(&c.about)) else { continue };
        if out.iter().any(|o| o.name == name) {
            continue;
        }
        out.push(BotCommand { name, about });
    }
    out
}

/// `clean_list` of `veydan_commands` from the wire: anything but an array
/// is no list, an element without a string `name` is dropped, an `about`
/// that is no string is an empty line.
pub fn list_from_json(value: &Value) -> Vec<BotCommand> {
    let Some(items) = value.as_array() else { return Vec::new() };
    let list: Vec<BotCommand> = items
        .iter()
        .take(MAX_COMMANDS * 4)
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?;
            let about = v.get("about").and_then(Value::as_str).unwrap_or_default();
            Some(BotCommand { name: name.into(), about: about.into() })
        })
        .collect();
    clean_list(&list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(name: &str, about: &str) -> BotCommand {
        BotCommand { name: name.into(), about: about.into() }
    }

    #[test]
    fn names_are_the_form_a_bot_reads_and_lines_one_line() {
        let list = clean_list(&[
            cmd("/Ping", "  Answers\n pong\t now "),
            cmd("echo_2", ""),
            cmd("ping", "a second line for ping"),
            cmd("pi-ng", "x"),
            cmd("", "x"),
            cmd("пинг", "x"),
            cmd(&"a".repeat(MAX_NAME + 1), "x"),
            cmd("long", &"й".repeat(MAX_ABOUT + 1)),
            cmd("edge", &"й".repeat(MAX_ABOUT)),
            cmd("bell", "ring\u{7}the bell"),
        ]);
        assert_eq!(
            list,
            vec![cmd("ping", "Answers pong now"), cmd("echo_2", ""), cmd("edge", &"й".repeat(MAX_ABOUT)), cmd("bell", "ring the bell")]
        );
    }

    #[test]
    fn a_long_description_is_cut_to_a_line_that_passes() {
        assert_eq!(fit_about(" one\ntwo "), "one two");
        let cut = fit_about(&format!("{} tail", "й".repeat(MAX_ABOUT - 1)));
        assert_eq!(cut, "й".repeat(MAX_ABOUT - 1), "cut at the limit, the space before it trimmed");
        assert_eq!(clean_list(&[cmd("x", &cut)]).len(), 1);
    }

    #[test]
    fn a_list_keeps_its_first_commands() {
        let many: Vec<BotCommand> = (0..MAX_COMMANDS + 10).map(|i| cmd(&format!("c{i}"), "")).collect();
        let list = clean_list(&many);
        assert_eq!(list.len(), MAX_COMMANDS);
        assert_eq!(list[0].name, "c0");
        assert_eq!(list[MAX_COMMANDS - 1].name, format!("c{}", MAX_COMMANDS - 1));
    }

    #[test]
    fn hostile_json_is_read_safely() {
        assert!(list_from_json(&json!("ping")).is_empty());
        assert!(list_from_json(&json!({ "name": "ping" })).is_empty());
        assert_eq!(
            list_from_json(&json!([
                { "name": "ping", "about": "Answers pong" },
                { "name": "help", "about": 5 },
                { "name": 7, "about": "x" },
                { "about": "no name" },
                "status",
                null,
            ])),
            vec![cmd("ping", "Answers pong"), cmd("help", "")]
        );
    }
}

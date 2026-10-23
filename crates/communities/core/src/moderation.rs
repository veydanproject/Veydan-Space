// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/moderation.rs at 1f2ef31d, then changed for channels.

//! Moderation of a channel on a device: whether the marks that came are
//! in force by the journal now, who may make one, what the log of
//! moderation says, and which messages the rule of pre-moderation holds.
//!
//! Only the rules live here (stage 1.1). The marks are kept through
//! `MarkStore` (the crate's own database implements it); the queue and
//! the messages it resolves are the content of the feed, which the
//! service brings in 1.3 (communities-stage1.md §7).

use crate::log::OpLog;
use crate::marks::{active, MarkRow};
use crate::op::OpBody;
use crate::state::{rules_at, ChannelState, Reader};
use async_trait::async_trait;
use messenger_core::traits::UiEvent;
use messenger_core::{PubKey, Result};

/// A mark was applied, or an operation of moderation took effect:
/// `{channel, action, by, target}`. A signal only: the page reads the
/// channel again.
pub const UI_EVENT_CHANNEL_MODERATION: &str = "channel.moderation";
/// What waits for a moderator changed in number: `{channel, pending}`.
pub const UI_EVENT_CHANNEL_QUEUE: &str = "channel.queue";

/// One line of the queue: a message that waits, the version an approval
/// has to name, and how many more of the same author wait behind it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueLine {
    pub id: String,
    pub author: String,
    pub created_at: i64,
    pub version: String,
    pub more_from_author: u32,
}

/// One line of the log of moderation: a mark, or an operation of the
/// journal that is one of moderation (ban, mute, trust, pins, the rules
/// of posting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub at: i64,
    pub by: String,
    /// `hide` | `unhide` | `approve` | `reject` | `ban` | `unban` | `mute`
    /// | `unmute` | `trust` | `untrust` | `remove` | `pin` | `unpin` |
    /// `settings`
    pub action: String,
    /// The message, the post or the person; empty for settings.
    pub target: String,
    /// JSON with what else the line says (a reason, a span, a version,
    /// whether a mark is in force, the settings set).
    pub detail: String,
}

/// Where the marks of moderation are kept: the crate's database, or a
/// fake in a test.
#[async_trait]
pub trait MarkStore: Send + Sync {
    /// Keep a mark; `false` when this row (note and target) is here already.
    async fn insert_mark(&self, mark: &MarkRow) -> Result<bool>;
    /// Every mark of a channel, newest first.
    async fn marks_of(&self, community_id: &str) -> Result<Vec<MarkRow>>;
    /// The marks about one message.
    async fn marks_about(&self, community_id: &str, target_id: &str) -> Result<Vec<MarkRow>>;
    /// The marks about a span of an author's messages that `created_at`
    /// falls into.
    async fn marks_spanning(&self, community_id: &str, author: &str, created_at: i64) -> Result<Vec<MarkRow>>;
    /// Rewrite whether marks are in force: `(note_id, target_id, active)`.
    async fn set_marks_active(&self, community_id: &str, changes: &[(String, String, bool)]) -> Result<()>;
}

/// The word to the page of an operation of the journal that is
/// moderation, the same as of a mark (`channel.moderation`); none for any
/// other operation.
pub fn op_event(community_id: &str, op: &crate::op::Op) -> Option<UiEvent> {
    let (action, who) = match &op.body {
        OpBody::Ban { who, .. } => ("ban", who.as_hex()),
        OpBody::Unban { who, .. } => ("unban", who.as_hex()),
        OpBody::Remove { who } => ("remove", who.as_hex()),
        OpBody::SetMuted { who, muted, .. } => (if *muted { "mute" } else { "unmute" }, who.as_hex()),
        OpBody::SetTrusted { who, trusted } => (if *trusted { "trust" } else { "untrust" }, who.as_hex()),
        OpBody::Pin { post_id, pinned } => (if *pinned { "pin" } else { "unpin" }, post_id.as_str()),
        OpBody::SetLevels { who, .. } => ("levels", who.as_hex()),
        OpBody::Settings { post_mode, premod, schedule, comments, .. }
            if post_mode.is_some() || premod.is_some() || schedule.is_some() || comments.is_some() =>
        {
            ("settings", "")
        }
        _ => return None,
    };
    Some(moderation_event(community_id, action, op.author.as_hex(), who))
}

pub fn moderation_event(community_id: &str, action: &str, by: &str, target: &str) -> UiEvent {
    UiEvent {
        name: UI_EVENT_CHANNEL_MODERATION.into(),
        payload: serde_json::json!({ "channel": community_id, "action": action, "by": by, "target": target }),
    }
}

pub fn queue_event(community_id: &str, pending: i64) -> UiEvent {
    UiEvent { name: UI_EVENT_CHANNEL_QUEUE.into(), payload: serde_json::json!({ "channel": community_id, "pending": pending }) }
}

/// May `me` make a mark of `kind` about a message of `target_author`
/// (under the post of `thread_author` when it is a comment)? The same
/// rule as the force of a mark (`marks::active`): one I could not make
/// would be out of force on every device.
pub fn may_mark(state: &ChannelState, me: &PubKey, kind: crate::marks::Kind, target_author: Option<&str>, thread_author: Option<&str>) -> bool {
    let probe = MarkRow {
        note_id: String::new(),
        community_id: state.community_id.clone(),
        kind: kind.as_str().into(),
        target_id: if target_author.is_some() { "probe".into() } else { String::new() },
        author: None,
        from_at: None,
        to_at: None,
        version: None,
        reason: String::new(),
        by: me.as_hex().into(),
        at: 0,
        active: false,
    };
    active(&probe, state, target_author, thread_author)
}

/// Of a mark: the author of the message it names and of that message's
/// post (`refresh_marks`).
pub type MarkAuthors<'a> = dyn Fn(&MarkRow) -> (Option<String>, Option<String>) + Sync + 'a;

/// Read again whether each mark of a channel is in force by the journal
/// now, and keep what changed. `authors` gives, of a mark, the author of
/// the message it names and the author of that message's post (none for
/// a span or a message not here yet). Returns the marks that changed:
/// their messages are resolved again by the service.
pub async fn refresh_marks(
    store: &dyn MarkStore,
    state: &ChannelState,
    community_id: &str,
    authors: &MarkAuthors<'_>,
) -> Result<Vec<MarkRow>> {
    let mut changed = Vec::new();
    let mut changes = Vec::new();
    for m in store.marks_of(community_id).await? {
        let (target, thread) = authors(&m);
        let now_active = active(&m, state, target.as_deref(), thread.as_deref());
        if now_active != m.active {
            changes.push((m.note_id.clone(), m.target_id.clone(), now_active));
            changed.push(MarkRow { active: now_active, ..m });
        }
    }
    if !changes.is_empty() {
        store.set_marks_active(community_id, &changes).await?;
    }
    Ok(changed)
}

/// The moment from which the rule of pre-moderation can hold a message
/// that came before it: the time of the operation that turned it on.
/// Messages before it no rule of this journal holds.
pub fn premod_turned_on_at(log: &OpLog) -> Option<i64> {
    log.state().premod_since_at
}

/// Whether a message that was never held is held by the journal as it
/// stands: its author is held now, and was when the message was dated
/// (`rules_at`). A message that outran the operation putting its author
/// under the rule (a relay hands the newest first) was judged without it.
pub fn outran_the_rule(log: &OpLog, author: &PubKey, reader: Option<&Reader>, created_at: i64) -> bool {
    log.state().subject_to_premod(author, reader) && rules_at(log, author, reader, created_at).subject_to_premod()
}

/// What was decided in the channel, newest first: every mark (in force
/// or not, the detail says), and the operations of the journal that are
/// moderation.
pub fn log_lines(log: &OpLog, marks: &[MarkRow]) -> Result<Vec<LogLine>> {
    let mut lines: Vec<LogLine> = Vec::new();
    for op in log.ordered().filter(|o| log.rejection(&o.id()).is_none()) {
        let (action, target, detail) = match &op.body {
            OpBody::Ban { who, reason } => ("ban", who.as_hex().to_string(), serde_json::json!({ "reason": reason })),
            OpBody::Unban { who, .. } => ("unban", who.as_hex().to_string(), serde_json::json!({})),
            OpBody::Remove { who } => ("remove", who.as_hex().to_string(), serde_json::json!({})),
            OpBody::SetMuted { who, muted: true, until } => ("mute", who.as_hex().to_string(), serde_json::json!({ "until": until })),
            OpBody::SetMuted { who, muted: false, .. } => ("unmute", who.as_hex().to_string(), serde_json::json!({})),
            OpBody::SetTrusted { who, trusted } => (if *trusted { "trust" } else { "untrust" }, who.as_hex().to_string(), serde_json::json!({})),
            OpBody::Pin { post_id, pinned } => (if *pinned { "pin" } else { "unpin" }, post_id.clone(), serde_json::json!({})),
            OpBody::SetLevels { who, levels } => ("levels", who.as_hex().to_string(), serde_json::json!({ "levels": levels })),
            OpBody::Settings { post_mode, premod, schedule, comments, .. }
                if post_mode.is_some() || premod.is_some() || schedule.is_some() || comments.is_some() =>
            {
                let mut d = serde_json::Map::new();
                if let Some(p) = post_mode {
                    d.insert("post_mode".into(), serde_json::to_value(p)?);
                }
                if let Some(p) = premod {
                    d.insert("premod".into(), serde_json::to_value(p)?);
                }
                if let Some(s) = schedule {
                    d.insert("schedule".into(), if s.is_empty() { serde_json::Value::Null } else { serde_json::to_value(s)? });
                }
                if let Some(c) = comments {
                    d.insert("comments".into(), serde_json::to_value(c)?);
                }
                ("settings", String::new(), serde_json::Value::Object(d))
            }
            _ => continue,
        };
        lines.push(LogLine { at: op.created_at, by: op.author.as_hex().to_string(), action: action.into(), target, detail: detail.to_string() });
    }
    for m in marks {
        let mut d = serde_json::json!({ "active": m.active, "note": m.note_id });
        if !m.reason.is_empty() {
            d["reason"] = serde_json::Value::String(m.reason.clone());
        }
        if let (Some(from), Some(to)) = (m.from_at, m.to_at) {
            d["from"] = serde_json::json!(from);
            d["to"] = serde_json::json!(to);
        }
        if let Some(v) = &m.version {
            d["version"] = serde_json::Value::String(v.clone());
        }
        let target = if m.target_id.is_empty() { m.author.clone().unwrap_or_default() } else { m.target_id.clone() };
        lines.push(LogLine { at: m.at, by: m.by.clone(), action: m.kind.clone(), target, detail: d.to_string() });
    }
    lines.sort_by(|a, b| b.at.cmp(&a.at).then(a.by.cmp(&b.by)).then(a.action.cmp(&b.action)).then(a.target.cmp(&b.target)));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::Insert;
    use crate::marks::{Kind, KIND_HIDE};
    use crate::op::tests::Settings;
    use crate::op::Premod;
    use crate::roles::Role;
    use crate::state::tests::{create, pk};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake(Mutex<Vec<MarkRow>>);

    #[async_trait]
    impl MarkStore for Fake {
        async fn insert_mark(&self, mark: &MarkRow) -> Result<bool> {
            let mut all = self.0.lock().unwrap();
            if all.iter().any(|m| m.note_id == mark.note_id && m.target_id == mark.target_id) {
                return Ok(false);
            }
            all.push(mark.clone());
            Ok(true)
        }
        async fn marks_of(&self, _: &str) -> Result<Vec<MarkRow>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn marks_about(&self, _: &str, target_id: &str) -> Result<Vec<MarkRow>> {
            Ok(self.0.lock().unwrap().iter().filter(|m| m.target_id == target_id).cloned().collect())
        }
        async fn marks_spanning(&self, _: &str, author: &str, at: i64) -> Result<Vec<MarkRow>> {
            Ok(self.0.lock().unwrap().iter().filter(|m| m.author.as_deref() == Some(author) && m.from_at <= Some(at) && m.to_at >= Some(at)).cloned().collect())
        }
        async fn set_marks_active(&self, _: &str, changes: &[(String, String, bool)]) -> Result<()> {
            for m in self.0.lock().unwrap().iter_mut() {
                if let Some((_, _, a)) = changes.iter().find(|(n, t, _)| *n == m.note_id && *t == m.target_id) {
                    m.active = *a;
                }
            }
            Ok(())
        }
    }

    fn journal() -> OpLog {
        let o = pk("0");
        let mut log = OpLog::create(create(&o)).unwrap();
        let op = log.next(&o, 120, OpBody::SetRole { who: pk("b"), role: Role::Moderator });
        assert_eq!(log.insert(op), Insert::Applied);
        log
    }

    #[tokio::test]
    async fn marks_follow_the_journal() {
        let mut log = journal();
        let (o, m, u) = (pk("0"), pk("b"), pk("c"));
        let store = Fake::default();
        let row = MarkRow {
            note_id: "n".into(),
            community_id: log.community_id().into(),
            kind: KIND_HIDE.into(),
            target_id: "t".into(),
            author: None,
            from_at: None,
            to_at: None,
            version: None,
            reason: String::new(),
            by: m.as_hex().into(),
            at: 300,
            active: false,
        };
        assert!(store.insert_mark(&row).await.unwrap());
        assert!(!store.insert_mark(&row).await.unwrap());
        let authors = |_: &MarkRow| (Some(u.as_hex().to_string()), None);
        let changed = refresh_marks(&store, log.state(), log.community_id(), &authors).await.unwrap();
        assert_eq!(changed.len(), 1);
        assert!(store.marks_about("", "t").await.unwrap()[0].active);
        assert!(refresh_marks(&store, log.state(), log.community_id(), &authors).await.unwrap().is_empty(), "nothing changed");
        // Demoted: out of force.
        let op = log.next(&o, 400, OpBody::SetRole { who: m.clone(), role: Role::Member });
        assert_eq!(log.insert(op), Insert::Applied);
        assert_eq!(refresh_marks(&store, log.state(), log.community_id(), &authors).await.unwrap().len(), 1);
        assert!(!store.marks_about("", "t").await.unwrap()[0].active);
        assert!(!may_mark(log.state(), &m, Kind::Hide, Some(u.as_hex()), None));
        assert!(may_mark(log.state(), &o, Kind::Hide, Some(u.as_hex()), None));
        assert!(may_mark(log.state(), &u, Kind::Hide, Some(pk("d").as_hex()), Some(u.as_hex())), "under my post");

        let lines = log_lines(&log, &store.marks_of("").await.unwrap()).unwrap();
        assert_eq!(lines.len(), 1, "a role is no moderation; the mark is");
        assert_eq!(lines[0].action, "hide");
    }

    #[test]
    fn the_log_and_the_words_to_the_page() {
        let mut log = journal();
        let (o, m, u) = (pk("0"), pk("b"), pk("c"));
        for (at, author, body) in [
            (200, &m, OpBody::Ban { who: u.clone(), reason: Some("spam".into()) }),
            (210, &m, OpBody::Unban { who: u.clone(), reason: None }),
            (220, &m, OpBody::SetMuted { who: u.clone(), muted: true, until: None }),
            (230, &m, OpBody::Pin { post_id: "ab".repeat(32), pinned: true }),
            (240, &o, Settings { premod: Some(Premod::All), ..Settings::default() }.body()),
            (250, &o, Settings { name: Some("x".into()), ..Settings::default() }.body()),
        ] {
            let op = log.next(author, at, body);
            assert!(op_event(log.community_id(), &op).is_some() == (at != 250), "{at}");
            assert_eq!(log.insert(op), Insert::Applied);
        }
        let lines = log_lines(&log, &[]).unwrap();
        assert_eq!(lines.iter().map(|l| l.action.as_str()).collect::<Vec<_>>(), vec!["settings", "pin", "mute", "unban", "ban"]);
        assert_eq!(premod_turned_on_at(&log), Some(240));
        // A reader who joined after the rule, writing after it: held, by
        // the journal now and by the rules then.
        assert!(outran_the_rule(&log, &pk("d"), Some(&Reader { since: 100 }), 245));
        assert!(!outran_the_rule(&log, &m, None, 245), "a moderator never");
        assert_eq!(queue_event("c", 3).payload["pending"], 3);
    }
}

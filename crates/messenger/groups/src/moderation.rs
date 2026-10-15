// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Moderation of a group on a device: the marks that arrive and the
//! marks I make (`marks`), what they do to the messages in the store, the
//! queue of what waits for a moderator, and the log of what was decided.
//!
//! A mark is applied to a message whenever the two meet: when the mark
//! comes and the message is here, when the message comes and the mark
//! waited for it, and again whenever the journal changes (a mark is in
//! force by the roles as they stand now). Nothing depends on the order
//! of arrival: history, a catch-up and a new device all end the same.

use crate::log::OpLog;
use crate::marks::{active, resolve, Kind, ModNote, Review, SKEW_SECS};
use crate::op::OpBody;
use crate::roles::Role;
use crate::service::*;
use crate::state::GroupState;
use crate::wire::{self, InnerMessage};
use messenger_core::traits::UiEvent;
use messenger_core::{Envelope, MessengerError, PubKey, Result};
use messenger_store::group_marks::{self as marks_repo};
use messenger_store::groups::{self as repo};
use messenger_store::messages::{self as msgs, MessageRow, MOD_HIDDEN, MOD_SHOWN, REVIEW_APPROVED, REVIEW_OK, REVIEW_REJECTED};
use messenger_store::chats;
use nostr::key::Keys;
use std::collections::HashSet;

/// A mark was applied, or an operation of moderation took effect:
/// `{group, action, by, target}`. A signal only: the page reads the
/// chat again.
pub const UI_EVENT_GROUP_MODERATION: &str = "group.moderation";
/// What waits for a moderator changed in number: `{group, pending}`.
pub const UI_EVENT_GROUP_QUEUE: &str = "group.queue";

/// One line of the queue: a message that waits, the version an approval
/// has to name, and how many more of the same author wait behind it.
#[derive(Clone, Debug)]
pub struct QueueLine {
    pub row: MessageRow,
    pub version: String,
    pub more_from_author: u32,
}

/// One line of the log of moderation: a mark, or an operation of the
/// journal that is one of moderation (ban, mute, trust, the rules of
/// posting).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub at: i64,
    pub by: String,
    /// `hide` | `unhide` | `approve` | `reject` | `ban` | `unban` | `mute`
    /// | `unmute` | `trust` | `untrust` | `settings`
    pub action: String,
    /// The message, or the person; empty for settings.
    pub target: String,
    /// JSON with what else the line says (a reason, a span, a version,
    /// whether a mark is in force, the settings set).
    pub detail: String,
}

fn not_permitted() -> MessengerError {
    MessengerError::Invalid("group_not_permitted".into())
}

/// The word to the page of an operation of the journal that is
/// moderation (a ban, a mute, trust, the rules of posting), the same as
/// of a mark (`group.moderation`); none for any other operation. The
/// rules are told of too: who may write changed for everyone, and the
/// page reads the group again on this word, which a renaming does not
/// need (the row of the group carries the name, not `can_post`).
pub(crate) fn op_event(group_id: &str, op: &crate::op::Op) -> Option<UiEvent> {
    let (action, who) = match &op.body {
        OpBody::Ban { who } => ("ban", who.as_hex()),
        OpBody::Unban { who } => ("unban", who.as_hex()),
        OpBody::SetMuted { who, muted: true, .. } => ("mute", who.as_hex()),
        OpBody::SetMuted { who, muted: false, .. } => ("unmute", who.as_hex()),
        OpBody::SetTrusted { who, trusted: true } => ("trust", who.as_hex()),
        OpBody::SetTrusted { who, trusted: false } => ("untrust", who.as_hex()),
        OpBody::EditSettings { post_mode, post_schedule, premod, .. }
            if post_mode.is_some() || post_schedule.is_some() || premod.is_some() =>
        {
            ("settings", group_id)
        }
        _ => return None,
    };
    Some(GroupService::moderation_event(group_id, action, op.author.as_hex(), who))
}

impl GroupService {
    pub(crate) fn moderation_event(group_id: &str, action: &str, by: &str, target: &str) -> UiEvent {
        UiEvent {
            name: UI_EVENT_GROUP_MODERATION.into(),
            payload: serde_json::json!({ "group": group_id, "action": action, "by": by, "target": target }),
        }
    }

    pub(crate) async fn queue_event(&self, group_id: &str) -> Result<UiEvent> {
        let pending = msgs::count_pending(&self.store, &repo::group_chat_id(group_id)).await?;
        Ok(UiEvent { name: UI_EVENT_GROUP_QUEUE.into(), payload: serde_json::json!({ "group": group_id, "pending": pending }) })
    }

    /// The state of a group by the journal here, for whoever decides by
    /// the roles and the rules now (the facade); `group_unknown` without
    /// a journal.
    pub async fn state_of(&self, group_id: &str) -> Result<GroupState> {
        Ok(self.need_log(group_id).await?.state().clone())
    }

    /// Whether `who` moderates the group now: a moderator or above by the
    /// journal here. Who may see what waits, and act on it.
    pub async fn is_moderator(&self, group_id: &str, who: &PubKey) -> Result<bool> {
        Ok(self.log(group_id).await?.is_some_and(|l| l.state().role_of(who).is_some_and(|r| r >= Role::Moderator)))
    }

    // ─── Marks that arrive ──────────────────────────────────────────────────

    /// A note of moderation came in the group: kept whatever its author's
    /// standing (the journal decides its force, now and later), unless it
    /// is dated ahead of my clock past the window or malformed.
    pub(crate) async fn on_mark(&self, group_id: &str, log: &OpLog, m: &InnerMessage, envelope: &Envelope, out: &mut Outcome) -> Result<()> {
        let note = match ModNote::parse(envelope) {
            Ok(Some(n)) => n,
            Ok(None) => return Ok(()),
            Err(what) => {
                out.notes.push(format!("group: a note of moderation dropped: bad {what}"));
                return Ok(());
            }
        };
        if m.created_at > self.now() + SKEW_SECS {
            out.notes.push("group: a note of moderation dropped: dated ahead".into());
            return Ok(());
        }
        self.apply_note(group_id, log, &note, m.id.as_hex(), &m.author, m.created_at, out).await
    }

    /// Keep the marks of a note and apply them to the messages they are
    /// about, those that are here. A mark about a message by id that is
    /// dated before the message is not kept: its author did not know the
    /// message when they wrote it. The same note again changes nothing.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn apply_note(
        &self,
        group_id: &str,
        log: &OpLog,
        note: &ModNote,
        note_id: &str,
        by: &PubKey,
        at: i64,
        out: &mut Outcome,
    ) -> Result<()> {
        let state = log.state();
        let chat_id = repo::group_chat_id(group_id);
        let mut affected: Vec<MessageRow> = Vec::new();
        // The page hears of the note once, by its first target: it reads
        // the chat again whatever the count (a note names up to 64).
        let mut told = false;
        for mut row in note.rows(group_id, note_id, by, at) {
            let target = if row.target_id.is_empty() { None } else { msgs::get(&self.store, &row.target_id).await? };
            if let Some(t) = &target {
                if t.chat_id != chat_id || t.is_hidden || t.content_type == msgs::CT_SYSTEM || at < t.created_at {
                    continue;
                }
            }
            row.active = active(&row, state, target.as_ref().map(|t| t.sender_pubkey.as_str()));
            if !marks_repo::insert_if_new(&self.store, &row).await? {
                continue;
            }
            match (&target, row.author.as_deref(), row.from_at, row.to_at) {
                (Some(t), _, _, _) => affected.push(t.clone()),
                (None, Some(author), Some(from), Some(to)) => affected.extend(msgs::of_author_between(&self.store, &chat_id, author, from, to).await?),
                _ => {}
            }
            if !told {
                let about = if row.target_id.is_empty() { row.author.clone().unwrap_or_default() } else { row.target_id.clone() };
                out.events.push(Self::moderation_event(group_id, &row.kind, &row.by, &about));
                told = true;
            }
        }
        self.resolve_rows(group_id, state, affected, true, out).await
    }

    /// Write what the marks say of these rows into them; with `tell`, the
    /// page hears once of the chat when a row of it changed (not for a
    /// row it is about to hear of anyway).
    pub(crate) async fn resolve_rows(&self, group_id: &str, state: &GroupState, rows: Vec<MessageRow>, tell: bool, out: &mut Outcome) -> Result<()> {
        let chat_id = repo::group_chat_id(group_id);
        let mut seen = HashSet::new();
        let mut changed: Vec<String> = Vec::new();
        for row in rows {
            if !seen.insert(row.id.clone()) || row.chat_id != chat_id {
                continue;
            }
            if self.resolve_row(group_id, state, &row).await? {
                changed.push(row.id);
            }
        }
        if let Some(first) = changed.first() {
            if tell {
                out.events.push(UiEvent {
                    name: messenger_dm::UI_EVENT_DM_UPDATED.into(),
                    payload: serde_json::json!({ "chat_id": chat_id, "message_id": first, "changed": changed.len() }),
                });
            }
            chats::recompute_last(&self.store, &chat_id).await?;
            out.events.push(self.queue_event(group_id).await?);
        }
        Ok(())
    }

    /// What the marks about one row add up to (`marks::resolve`), by the
    /// journal as it stands (`state`), written into it; `true` when
    /// something changed. A row that was never held (`review = ok`) takes
    /// no approval and no rejection; one held that nobody decided waits
    /// while the journal holds its author and is shown when it does not.
    pub(crate) async fn resolve_row(&self, group_id: &str, state: &GroupState, row: &MessageRow) -> Result<bool> {
        let mut marks = marks_repo::by_target(&self.store, group_id, &row.id).await?;
        marks.extend(marks_repo::by_author_range(&self.store, group_id, &row.sender_pubkey, row.created_at).await?);
        let version = msgs::version_of(&self.store, row).await?;
        let subject = Self::subject_now(state, row);
        let r = resolve(&marks, row.created_at, &version, row.review != REVIEW_OK, subject);
        let mut changed = msgs::set_mod_state(&self.store, &row.id, if r.hidden { MOD_HIDDEN } else { MOD_SHOWN }).await?;
        changed |= msgs::set_review(&self.store, &row.id, r.review.as_str()).await?;
        Ok(changed)
    }

    /// Whether the journal holds the author of this row for a moderator
    /// now (`GroupState::subject_to_premod`).
    fn subject_now(state: &GroupState, row: &MessageRow) -> bool {
        PubKey::parse(&row.sender_pubkey).is_some_and(|a| state.subject_to_premod(&a))
    }

    /// The rows never held for a moderator that the journal, as it stands,
    /// holds: a message that outran the operation putting its author under
    /// the rule (a relay hands the newest first; a journal catches up late)
    /// was judged by a journal without it. Each is judged again as it was
    /// when it came (`rules_at` by its date), against the journal of now.
    /// Only rows dated from the operation that turned the rule on are
    /// read: an earlier one no rule of this journal holds, whatever came
    /// after. The journal changes seldom; the rows since the rule are the
    /// price of judging by it alone.
    async fn outrun_rows(&self, log: &OpLog, chat_id: &str) -> Result<Vec<MessageRow>> {
        let state = log.state();
        let Some(since) = state.premod_since else { return Ok(vec![]) };
        let Some(turned_on) = log.ordered().filter(|o| log.rejection(&o.id()).is_none()).nth(since as usize) else { return Ok(vec![]) };
        let rows = msgs::ok_rows_since(&self.store, chat_id, turned_on.created_at).await?;
        Ok(rows
            .into_iter()
            .filter(|r| {
                PubKey::parse(&r.sender_pubkey)
                    .is_some_and(|a| state.subject_to_premod(&a) && crate::inbound::rules_at(log, &a, r.created_at).subject_to_premod())
            })
            .collect())
    }

    /// A message just came: the marks that waited for it are applied.
    /// Those that name it by id have their force read again, now that
    /// its author is known.
    pub(crate) async fn marks_on_insert(&self, group_id: &str, state: &GroupState, row: &MessageRow, out: &mut Outcome) -> Result<()> {
        let by_id = marks_repo::by_target(&self.store, group_id, &row.id).await?;
        let changes: Vec<(String, String, bool)> = by_id
            .iter()
            .map(|m| (m.note_id.clone(), m.target_id.clone(), active(m, state, Some(&row.sender_pubkey))))
            .filter(|(n, t, a)| by_id.iter().any(|m| &m.note_id == n && &m.target_id == t && m.active != *a))
            .collect();
        marks_repo::set_active_bulk(&self.store, group_id, &changes).await?;
        self.resolve_rows(group_id, state, vec![row.clone()], false, out).await
    }

    /// The journal changed: the force of every mark of the group is read
    /// again, and the messages of those that changed are resolved again;
    /// so is every row held for a moderator whose author the journal no
    /// longer holds (pre-moderation off, the author trusted or raised),
    /// every row released so whose author it holds again, and every row
    /// never held that the journal holds now (`outrun_rows`: the message
    /// came before the operation that put its author under the rule). One
    /// pass over all of them; groups change seldom, marks are few, and of
    /// the rows that wait only those that change hands are touched.
    pub(crate) async fn recompute_marks(&self, group_id: &str, log: &OpLog, out: &mut Outcome) -> Result<()> {
        let state = log.state();
        let chat_id = repo::group_chat_id(group_id);
        let mut affected: Vec<MessageRow> = msgs::held_rows(&self.store, &chat_id)
            .await?
            .into_iter()
            .filter(|r| (r.review == msgs::REVIEW_PENDING) != Self::subject_now(state, r))
            .collect();
        for row in self.outrun_rows(log, &chat_id).await? {
            msgs::set_review(&self.store, &row.id, msgs::REVIEW_PENDING).await?;
            if let Some(fresh) = msgs::get(&self.store, &row.id).await? {
                affected.push(fresh);
            }
        }
        let marks = marks_repo::by_group(&self.store, group_id).await?;
        let mut changes = Vec::new();
        for m in &marks {
            let target = if m.target_id.is_empty() { None } else { msgs::get(&self.store, &m.target_id).await? };
            let now_active = active(m, state, target.as_ref().map(|t| t.sender_pubkey.as_str()));
            if now_active == m.active {
                continue;
            }
            changes.push((m.note_id.clone(), m.target_id.clone(), now_active));
            match (target, m.author.as_deref(), m.from_at, m.to_at) {
                (Some(t), _, _, _) => affected.push(t),
                (None, Some(author), Some(from), Some(to)) => affected.extend(msgs::of_author_between(&self.store, &chat_id, author, from, to).await?),
                _ => {}
            }
        }
        if !changes.is_empty() {
            marks_repo::set_active_bulk(&self.store, group_id, &changes).await?;
        }
        if affected.is_empty() {
            return Ok(());
        }
        self.resolve_rows(group_id, state, affected, true, out).await
    }

    /// `recompute_marks` when the journal is not the one the marks were
    /// last read against: its length and the version of its state say so.
    /// Read once per group per start, then on every change.
    pub(crate) async fn marks_follow_log(&self, group_id: &str, log: &OpLog, out: &mut Outcome) -> Result<()> {
        let fingerprint = (log.len() as u64, log.state().version);
        {
            let mut seen = self.marks_seen.lock().await;
            if seen.get(group_id) == Some(&fingerprint) {
                return Ok(());
            }
            seen.insert(group_id.to_string(), fingerprint);
        }
        self.recompute_marks(group_id, log, out).await
    }

    /// An edit of a message by its author: one who is held for moderation
    /// (the message waited, or the author waits by the rules at the time
    /// of the edit) makes the message wait again, under its new version.
    pub(crate) async fn after_edit(&self, group_id: &str, log: &OpLog, row: &MessageRow, at: i64) -> Result<()> {
        let held = row.review != REVIEW_OK
            || PubKey::parse(&row.sender_pubkey).is_some_and(|a| crate::inbound::rules_at(log, &a, at).subject_to_premod());
        if !held {
            return Ok(());
        }
        msgs::set_review(&self.store, &row.id, Review::Pending.as_str()).await?;
        if let Some(fresh) = msgs::get(&self.store, &row.id).await? {
            self.resolve_row(group_id, log.state(), &fresh).await?;
        }
        Ok(())
    }

    // ─── Marks I make ───────────────────────────────────────────────────────

    /// Make a note of moderation, apply it here at once and seal it for
    /// the group. Refused when I am below a moderator, or when a message
    /// named is of someone at my role or above (`group_not_permitted`).
    /// Nothing is sent for what is in the asked state already: a hide of
    /// what is hidden, an approval of what is approved; a note whose
    /// every target is done is no note at all (an empty outcome).
    pub async fn moderate(&self, keys: &Keys, group_id: &str, note: ModNote) -> Result<Outcome> {
        self.moderate_as(keys, group_id, note, false).await
    }

    /// `moderate`; with `again`, a target already in the asked state by
    /// somebody else's mark is named all the same (the inheritance of a
    /// demoted moderator's marks), only one my own newest mark covers is
    /// left out.
    async fn moderate_as(&self, keys: &Keys, group_id: &str, note: ModNote, again: bool) -> Result<Outcome> {
        let me = me_of(keys);
        let row = repo::get(&self.store, group_id).await?.ok_or_else(|| MessengerError::Invalid("group_unknown".into()))?;
        if row.membership != MEMBERSHIP_JOINED {
            return Err(MessengerError::Invalid("group_not_member".into()));
        }
        let log = self.need_log(group_id).await?;
        let s = log.state();
        let my_role = s.role_of(&me).filter(|r| *r >= Role::Moderator).ok_or_else(not_permitted)?;
        let chat_id = repo::group_chat_id(group_id);
        let mut at = self.now();
        let kind = note.kind();
        let below_me = |author: &str| PubKey::parse(author).and_then(|p| s.role_of(&p)).unwrap_or(Role::Member) < my_role;
        let note = match note {
            ModNote::Hide { targets, reason } => {
                if targets.is_empty() {
                    return Err(MessengerError::Invalid("targets".into()));
                }
                let mut keep = Vec::new();
                for id in targets {
                    let t = self.target_of(&chat_id, &id).await?;
                    if !below_me(&t.sender_pubkey) {
                        return Err(not_permitted());
                    }
                    at = at.max(t.created_at);
                    let done = if again { self.mine_already(group_id, &me, &id, kind, None).await? } else { t.mod_state == MOD_HIDDEN };
                    if !done {
                        keep.push(id);
                    }
                }
                if keep.is_empty() {
                    return Ok(Outcome::default());
                }
                ModNote::Hide { targets: keep, reason }
            }
            ModNote::Unhide { targets } => {
                if targets.is_empty() {
                    return Err(MessengerError::Invalid("targets".into()));
                }
                let mut keep = Vec::new();
                for id in targets {
                    let t = self.target_of(&chat_id, &id).await?;
                    if !below_me(&t.sender_pubkey) {
                        return Err(not_permitted());
                    }
                    at = at.max(t.created_at);
                    let done = if again { self.mine_already(group_id, &me, &id, kind, None).await? } else { t.mod_state != MOD_HIDDEN };
                    if !done {
                        keep.push(id);
                    }
                }
                if keep.is_empty() {
                    return Ok(Outcome::default());
                }
                ModNote::Unhide { targets: keep }
            }
            ModNote::HideAuthor { author, from, to, reason } => {
                if !below_me(author.as_hex()) {
                    return Err(not_permitted());
                }
                if from > to {
                    return Err(MessengerError::Invalid("range".into()));
                }
                // A span of mine in force that covers this one says it
                // already: the same «spam» a second time sends nothing.
                let covered = marks_repo::by_author_of_marks(&self.store, group_id, me.as_hex()).await?.into_iter().any(|m| {
                    m.active
                        && m.kind == Kind::Hide.as_str()
                        && m.author.as_deref() == Some(author.as_hex())
                        && m.from_at.is_some_and(|f| f <= from)
                        && m.to_at.is_some_and(|t| t >= to)
                });
                if covered {
                    return Ok(Outcome::default());
                }
                ModNote::HideAuthor { author, from, to, reason }
            }
            ModNote::Approve { target, version } => {
                let t = self.target_of(&chat_id, &target).await?;
                if t.review == REVIEW_OK {
                    return Err(MessengerError::Invalid("group_not_pending".into()));
                }
                if msgs::version_of(&self.store, &t).await? != version {
                    return Err(MessengerError::Invalid("group_version_stale".into()));
                }
                at = at.max(t.created_at);
                let done = if again { self.mine_already(group_id, &me, &target, kind, Some(&version)).await? } else { t.review == REVIEW_APPROVED };
                if done {
                    return Ok(Outcome::default());
                }
                ModNote::Approve { target, version }
            }
            ModNote::Reject { target } => {
                let t = self.target_of(&chat_id, &target).await?;
                if t.review == REVIEW_OK {
                    return Err(MessengerError::Invalid("group_not_pending".into()));
                }
                at = at.max(t.created_at);
                let done = if again { self.mine_already(group_id, &me, &target, kind, None).await? } else { t.review == REVIEW_REJECTED };
                if done {
                    return Ok(Outcome::default());
                }
                ModNote::Reject { target }
            }
        };
        // My later word about a message is dated after my earlier one. A
        // message dated ahead of my clock dates every mark about it to its
        // own second (`at` is never before the message), and of two marks
        // dated alike the stricter stands: my unhide would never undo my
        // hide until my clock caught up.
        for id in note.targets() {
            let mine = marks_repo::by_target(&self.store, group_id, id).await?.into_iter().filter(|m| m.by == me.as_hex()).map(|m| m.at).max();
            if let Some(newest) = mine {
                at = at.max(newest + 1);
            }
        }
        let key = self.sealing_key(group_id, &log).await?;
        let grace = self.grace_key(group_id, s).await?;
        let signed = wire::sign_message(keys, group_id, &note.envelope().encode(), at, None)?;
        let sealed = wire::seal_note(group_id, &key, grace.as_ref(), &signed, keys)?;
        let mut out = Outcome { publish: vec![Self::scoped(group_id, sealed)], ..Default::default() };
        self.apply_note(group_id, &log, &note, &signed.id.to_hex(), &me, at, &mut out).await?;
        Ok(out)
    }

    /// The message a note names, in this chat: a message of the chat, not
    /// a line of the system.
    async fn target_of(&self, chat_id: &str, id: &str) -> Result<MessageRow> {
        msgs::get(&self.store, id)
            .await?
            .filter(|r| r.chat_id == chat_id && !r.is_hidden && r.content_type != msgs::CT_SYSTEM)
            .ok_or_else(|| MessengerError::Invalid("unknown message".into()))
    }

    /// Whether my own newest mark in force about the message is this
    /// already (of this `kind`, and of this `version` when one is named).
    async fn mine_already(&self, group_id: &str, me: &PubKey, id: &str, kind: Kind, version: Option<&str>) -> Result<bool> {
        let marks = marks_repo::by_target(&self.store, group_id, id).await?;
        let newest = marks.iter().filter(|m| m.by == me.as_hex() && m.active).max_by_key(|m| (m.at, std::cmp::Reverse(m.note_id.clone())));
        Ok(newest.is_some_and(|m| m.kind == kind.as_str() && version.is_none_or(|v| m.version.as_deref() == Some(v))))
    }

    /// Say again, as me, what another moderator decided: their newest
    /// mark about each message, and each of their spans. For the admin
    /// who demotes a moderator and keeps their decisions. Returns how
    /// many notes went out; a mark that cannot be said again (the message
    /// edited since the approval, its author above me) is passed by.
    pub async fn inherit_marks(&self, keys: &Keys, group_id: &str, from: &PubKey) -> Result<(u32, Outcome)> {
        self.need_log(group_id).await?;
        let marks = marks_repo::by_author_of_marks(&self.store, group_id, from.as_hex()).await?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Outcome::default();
        let mut sent = 0;
        for m in marks {
            if !m.target_id.is_empty() && !seen.insert(m.target_id.clone()) {
                continue;
            }
            let Some(kind) = Kind::parse(&m.kind) else { continue };
            let note = match (kind, m.target_id.is_empty()) {
                (Kind::Hide, false) => ModNote::Hide { targets: vec![m.target_id.clone()], reason: m.reason.clone() },
                (Kind::Hide, true) => match (m.author.as_deref().and_then(PubKey::parse), m.from_at, m.to_at) {
                    (Some(author), Some(from), Some(to)) => ModNote::HideAuthor { author, from, to, reason: m.reason.clone() },
                    _ => continue,
                },
                (Kind::Unhide, _) => ModNote::Unhide { targets: vec![m.target_id.clone()] },
                (Kind::Approve, _) => ModNote::Approve { target: m.target_id.clone(), version: m.version.clone().unwrap_or_default() },
                (Kind::Reject, _) => ModNote::Reject { target: m.target_id.clone() },
            };
            match self.moderate_as(keys, group_id, note, true).await {
                Ok(o) => {
                    if !o.publish.is_empty() {
                        sent += 1;
                    }
                    out.merge(o);
                }
                Err(e) => out.notes.push(format!("group: a mark was not inherited: {e}")),
            }
        }
        Ok((sent, out))
    }

    // ─── The queue and the log ──────────────────────────────────────────────

    /// What waits for a moderator, one line per author (their oldest),
    /// newest first, strictly older than `before`; and how many rows wait
    /// in all under the filter. For moderators and above.
    pub async fn queue(&self, group_id: &str, me: &PubKey, filter: &msgs::QueueFilter, before: Option<i64>, limit: i64) -> Result<(Vec<QueueLine>, i64)> {
        if !self.is_moderator(group_id, me).await? {
            return Err(not_permitted());
        }
        let chat_id = repo::group_chat_id(group_id);
        let (lines, total) = msgs::queue(&self.store, &chat_id, filter, before, limit.clamp(1, 500)).await?;
        let mut out = Vec::with_capacity(lines.len());
        for l in lines {
            let version = msgs::version_of(&self.store, &l.row).await?;
            out.push(QueueLine { row: l.row, version, more_from_author: l.more_from_author.max(0) as u32 });
        }
        Ok((out, total))
    }

    /// What was decided in the group, newest first: every mark (in force
    /// or not, the detail says), and the operations of the journal that
    /// are moderation: bans, mutes, trust, the rules of posting. For
    /// moderators and above.
    pub async fn mod_log(&self, group_id: &str, me: &PubKey) -> Result<Vec<LogLine>> {
        if !self.is_moderator(group_id, me).await? {
            return Err(not_permitted());
        }
        let log = self.need_log(group_id).await?;
        let mut lines: Vec<LogLine> = Vec::new();
        for op in log.ordered().filter(|o| log.rejection(&o.id()).is_none()) {
            let (action, target, detail) = match &op.body {
                OpBody::Ban { who } => ("ban", who.as_hex().to_string(), serde_json::json!({})),
                OpBody::Unban { who } => ("unban", who.as_hex().to_string(), serde_json::json!({})),
                OpBody::SetMuted { who, muted: true, until } => ("mute", who.as_hex().to_string(), serde_json::json!({ "until": until })),
                OpBody::SetMuted { who, muted: false, .. } => ("unmute", who.as_hex().to_string(), serde_json::json!({})),
                OpBody::SetTrusted { who, trusted } => (if *trusted { "trust" } else { "untrust" }, who.as_hex().to_string(), serde_json::json!({})),
                OpBody::EditSettings { post_mode, post_schedule, premod, .. } if post_mode.is_some() || post_schedule.is_some() || premod.is_some() => {
                    let mut d = serde_json::Map::new();
                    if let Some(p) = post_mode {
                        d.insert("post_mode".into(), serde_json::to_value(p)?);
                    }
                    if let Some(p) = premod {
                        d.insert("premod".into(), serde_json::to_value(p)?);
                    }
                    if let Some(s) = post_schedule {
                        d.insert("schedule".into(), if s.is_empty() { serde_json::Value::Null } else { serde_json::to_value(s)? });
                    }
                    ("settings", String::new(), serde_json::Value::Object(d))
                }
                _ => continue,
            };
            lines.push(LogLine { at: op.created_at, by: op.author.as_hex().to_string(), action: action.into(), target, detail: detail.to_string() });
        }
        for m in marks_repo::by_group(&self.store, group_id).await? {
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
            lines.push(LogLine { at: m.at, by: m.by, action: m.kind, target, detail: d.to_string() });
        }
        lines.sort_by(|a, b| b.at.cmp(&a.at).then(a.by.cmp(&b.by)).then(a.action.cmp(&b.action)).then(a.target.cmp(&b.target)));
        Ok(lines)
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Moderation of a channel as the page asks for it, over the rows of the
//! feed (internal/communities-stage1.md §6.1, `channel_moderation_*`): the
//! acts (`cmod.*` notes sealed into the feed, or operations of the
//! journal), the queue of what pre-moderation holds, the log, the
//! inheritance of a demoted moderator's marks, the requests and the
//! invitations. The rules are `moderation` and `marks` (1.1); the words
//! of the acts follow the groups' facade (`ModAction`), so that the page
//! of groups serves channels with another `api`.

use crate::db::{self, channels::{InviteRow, RequestRow}};
use crate::marks::{self, Kind, MarkRow, ModNote, REASON_SPAM};
use crate::moderation::{self, LogLine, MarkStore, QueueLine};
use crate::op::OpBody;
use crate::roles::{permits, Action, Role};
use crate::service::{ChannelService, Outcome, MAX_INVITES_PER_CHANNEL, MAX_INVITES_PER_PERSON, MAX_OPEN_INVITES, UI_EVENT_CHANNEL_INVITE};
use crate::state::ChannelState;
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

fn not_permitted() -> MessengerError {
    invalid("not permitted")
}

/// One act of moderation; tagged by `t` as the groups' facade is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ModAction {
    Hide { targets: Vec<String> },
    HideAuthor { author: String, from: i64, to: i64 },
    Unhide { targets: Vec<String> },
    /// `version` empty: the version held now.
    Approve { target: String, #[serde(default)] version: String },
    Reject { target: String },
    /// Ban the author when I may, and hide what they wrote up to now
    /// (`target`: the message the decision was taken on).
    Spam { author: String, #[serde(default)] target: Option<String> },
    /// `until` absent: for good.
    Mute { who: String, #[serde(default)] until: Option<i64> },
    Unmute { who: String },
    Trust { who: String, trusted: bool },
}

/// The invitations of a channel against the limits of groups (wire §7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteStatus {
    /// Open invitations of mine, of every channel.
    pub open: usize,
    pub max_open: usize,
    /// Open invitations of this channel.
    pub of_channel: usize,
    pub max_per_channel: usize,
    pub max_per_person: usize,
}

fn key(s: &str) -> Result<PubKey> {
    PubKey::parse(s.trim()).ok_or_else(|| invalid("key"))
}

impl ChannelService {
    fn is_moderator(state: &ChannelState, me: &PubKey) -> bool {
        !state.is_banned(me) && state.role_of(me).is_some_and(Role::is_moderator)
    }

    /// The author of a message here and of its post (of a comment).
    async fn authors_of(&self, id: &str) -> Result<(String, Option<String>)> {
        let row = db::content::get(&self.env.db, id).await?.ok_or_else(|| invalid("unknown message"))?;
        let thread = if row.is_post() { None } else { db::content::get(&self.env.db, &row.post_id).await?.map(|p| p.author) };
        Ok((row.author, thread))
    }

    /// One act of moderation: a note (hide, unhide, approve, reject), an
    /// operation of the journal (mute, unmute, trust), or both (spam).
    /// Each target is checked by the rule a mark is judged by on every
    /// device (`moderation::may_mark`); one already in the state asked for
    /// is left out, and nothing is sent when nothing is left.
    pub async fn moderate(&self, keys: &Keys, community_id: &str, action: ModAction) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let state = self.state(community_id).await?;
        let now = self.now();
        let note = match action {
            ModAction::Hide { targets } => {
                let mut keep = Vec::new();
                for id in targets {
                    let (author, thread) = self.authors_of(&id).await?;
                    if !moderation::may_mark(&state, &me, Kind::Hide, Some(&author), thread.as_deref()) {
                        return Err(not_permitted());
                    }
                    let hidden = db::content::get(&self.env.db, &id).await?.is_some_and(|r| r.mod_state == db::content::MOD_HIDDEN);
                    if !hidden {
                        keep.push(id);
                    }
                }
                if keep.is_empty() {
                    return Ok(Outcome::default());
                }
                ModNote::Hide { targets: keep, reason: String::new() }
            }
            ModAction::Unhide { targets } => {
                let mut keep = Vec::new();
                for id in targets {
                    let (author, thread) = self.authors_of(&id).await?;
                    if !moderation::may_mark(&state, &me, Kind::Unhide, Some(&author), thread.as_deref()) {
                        return Err(not_permitted());
                    }
                    let hidden = db::content::get(&self.env.db, &id).await?.is_some_and(|r| r.mod_state == db::content::MOD_HIDDEN);
                    if hidden {
                        keep.push(id);
                    }
                }
                if keep.is_empty() {
                    return Ok(Outcome::default());
                }
                ModNote::Unhide { targets: keep }
            }
            ModAction::HideAuthor { author, from, to } => {
                let author = key(&author)?;
                if !moderation::may_mark(&state, &me, Kind::Hide, None, None) || state.role_of(&author).is_some_and(|r| r >= state.role_of(&me).unwrap_or(Role::Member)) {
                    return Err(not_permitted());
                }
                ModNote::HideAuthor { author, from, to, reason: String::new() }
            }
            ModAction::Approve { target, version } => {
                let row = db::content::get(&self.env.db, &target).await?.ok_or_else(|| invalid("unknown message"))?;
                let (author, thread) = self.authors_of(&target).await?;
                if !moderation::may_mark(&state, &me, Kind::Approve, Some(&author), thread.as_deref()) {
                    return Err(not_permitted());
                }
                if row.review != marks::REVIEW_PENDING {
                    return Ok(Outcome::default());
                }
                let version = if version.is_empty() { row.version } else { version };
                ModNote::Approve { target, version }
            }
            ModAction::Reject { target } => {
                let row = db::content::get(&self.env.db, &target).await?.ok_or_else(|| invalid("unknown message"))?;
                let (author, thread) = self.authors_of(&target).await?;
                if !moderation::may_mark(&state, &me, Kind::Reject, Some(&author), thread.as_deref()) {
                    return Err(not_permitted());
                }
                if row.review == marks::REVIEW_REJECTED {
                    return Ok(Outcome::default());
                }
                ModNote::Reject { target }
            }
            ModAction::Spam { author, target } => {
                let author = key(&author)?;
                let mine = state.role_of(&me).filter(|r| r.is_moderator()).ok_or_else(not_permitted)?;
                let mut out = Outcome::default();
                // The ban is given by whoever may; a moderator below the
                // author hides only.
                if !state.is_banned(&author) && permits(mine, Action::Ban(state.role_of(&author).unwrap_or(Role::Member))) {
                    out.merge(self.act(keys, community_id, OpBody::Ban { who: author.clone(), reason: Some(REASON_SPAM.into()) }).await?);
                }
                let mut to = now;
                if let Some(t) = target {
                    if let Some(m) = db::content::get(&self.env.db, &t).await? {
                        to = to.max(m.created_at);
                    }
                }
                let note = ModNote::HideAuthor { author, from: 0, to, reason: REASON_SPAM.into() };
                let (_, o) = self.send_mod(keys, community_id, &note).await?;
                out.merge(o);
                return Ok(out);
            }
            ModAction::Mute { who, until } => {
                let who = key(&who)?;
                if state.staff_of(&who).is_some_and(|m| m.muted_at(now) && m.muted_until == until) {
                    return Ok(Outcome::default());
                }
                return self.act(keys, community_id, OpBody::SetMuted { who, muted: true, until }).await;
            }
            ModAction::Unmute { who } => {
                let who = key(&who)?;
                if !state.staff_of(&who).is_some_and(|m| m.muted_at(now)) {
                    return Ok(Outcome::default());
                }
                return self.act(keys, community_id, OpBody::SetMuted { who, muted: false, until: None }).await;
            }
            ModAction::Trust { who, trusted } => {
                let who = key(&who)?;
                if state.staff_of(&who).is_some_and(|m| m.trusted) == trusted {
                    return Ok(Outcome::default());
                }
                return self.act(keys, community_id, OpBody::SetTrusted { who, trusted }).await;
            }
        };
        let (_, mut out) = self.send_mod(keys, community_id, &note).await?;
        out.events.push(moderation::moderation_event(community_id, note.kind().as_str(), me.as_hex(), note.targets().first().map_or("", String::as_str)));
        out.events.push(moderation::queue_event(community_id, db::content::pending(&self.env.db, community_id).await?.len() as i64));
        Ok(out)
    }

    /// What pre-moderation holds, one line per author (their oldest, with
    /// how many more of theirs wait), newest first and strictly older
    /// than `before`; and how many wait in all. With `author` (hex), every
    /// message of that author alone, one line each (with how many of
    /// theirs wait after it), and how many of theirs wait: what a
    /// moderator decides at once from the author's card. For moderators.
    pub async fn queue(&self, keys: &Keys, community_id: &str, author: Option<&str>, before: Option<i64>, limit: usize) -> Result<(Vec<QueueLine>, usize)> {
        let state = self.state(community_id).await?;
        if !Self::is_moderator(&state, &Self::me(keys)?) {
            return Err(not_permitted());
        }
        let mut rows = db::content::pending(&self.env.db, community_id).await?;
        if let Some(author) = author {
            rows.retain(|r| r.author == author);
            let total = rows.len();
            // Oldest first from the database: what waits after each is the rest.
            let mut lines: Vec<QueueLine> = rows
                .into_iter()
                .enumerate()
                .map(|(i, r)| QueueLine { id: r.id, author: r.author, created_at: r.created_at, version: r.version, more_from_author: u32::try_from(total - 1 - i).unwrap_or(u32::MAX) })
                .filter(|l| before.is_none_or(|b| l.created_at < b))
                .collect();
            lines.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
            lines.truncate(limit.clamp(1, 500));
            return Ok((lines, total));
        }
        let total = rows.len();
        let mut by_author: BTreeMap<String, (QueueLine, u32)> = BTreeMap::new();
        for r in rows {
            match by_author.get_mut(&r.author) {
                Some((_, more)) => *more += 1,
                None => {
                    let line = QueueLine { id: r.id, author: r.author.clone(), created_at: r.created_at, version: r.version, more_from_author: 0 };
                    by_author.insert(r.author, (line, 0));
                }
            }
        }
        let mut lines: Vec<QueueLine> = by_author.into_values().map(|(l, more)| QueueLine { more_from_author: more, ..l }).filter(|l| before.is_none_or(|b| l.created_at < b)).collect();
        lines.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        lines.truncate(limit.clamp(1, 500));
        Ok((lines, total))
    }

    /// What was decided in the channel, newest first, strictly older than
    /// `before`: the marks and the operations of moderation of the
    /// journal. A full page takes every further line of the second of its
    /// last one along, so that the next page passes none by. For
    /// moderators.
    pub async fn mod_log(&self, keys: &Keys, community_id: &str, before: Option<i64>, limit: usize) -> Result<Vec<LogLine>> {
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        if !Self::is_moderator(log.state(), &Self::me(keys)?) {
            return Err(not_permitted());
        }
        let marks = self.env.db.marks_of(community_id).await?;
        let limit = limit.clamp(1, 500);
        let mut out: Vec<LogLine> = Vec::new();
        for l in moderation::log_lines(&log, &marks)?.into_iter().filter(|l| before.is_none_or(|b| l.at < b)) {
            if out.len() >= limit && out.last().is_some_and(|last| last.at != l.at) {
                break;
            }
            out.push(l);
        }
        Ok(out)
    }

    /// Says again, as me, what another moderator decided (`from`): for
    /// the admin who demotes one and keeps their decisions. How many
    /// notes went out.
    pub async fn inherit_marks(&self, keys: &Keys, community_id: &str, from: &PubKey) -> Result<(u32, Outcome)> {
        let marks: Vec<MarkRow> = db::marks::by_author_of_marks(&self.env.db, community_id, from.as_hex()).await?;
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Outcome::default();
        let mut sent = 0;
        for m in marks {
            if !m.target_id.is_empty() && !seen.insert(m.target_id.clone()) {
                continue;
            }
            let Some(kind) = Kind::parse(&m.kind) else { continue };
            let action = match (kind, m.target_id.is_empty()) {
                (Kind::Hide, false) => ModAction::Hide { targets: vec![m.target_id.clone()] },
                (Kind::Hide, true) => match (m.author.clone(), m.from_at, m.to_at) {
                    (Some(author), Some(from), Some(to)) => ModAction::HideAuthor { author, from, to },
                    _ => continue,
                },
                (Kind::Unhide, _) => ModAction::Unhide { targets: vec![m.target_id.clone()] },
                (Kind::Approve, _) => ModAction::Approve { target: m.target_id.clone(), version: m.version.clone().unwrap_or_default() },
                (Kind::Reject, _) => ModAction::Reject { target: m.target_id.clone() },
            };
            match self.moderate(keys, community_id, action).await {
                Ok(o) => {
                    if !o.events.is_empty() {
                        sent += 1;
                    }
                    out.merge(o);
                }
                Err(e) => out.note(format!("a mark was not inherited: {e}")),
            }
        }
        Ok((sent, out))
    }

    /// Whether marks are in force is a function of the journal: read again
    /// after it changed, and the messages whose marks changed resolved
    /// again.
    pub(crate) async fn refresh_marks(&self, log: &crate::log::OpLog) -> Result<()> {
        let cid = log.community_id().to_string();
        let marks = self.env.db.marks_of(&cid).await?;
        if marks.is_empty() {
            return Ok(());
        }
        let mut authors: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
        for m in marks.iter().filter(|m| !m.target_id.is_empty()) {
            if authors.contains_key(&m.target_id) {
                continue;
            }
            let pair = match self.authors_of(&m.target_id).await {
                Ok((a, t)) => (Some(a), t),
                Err(_) => (None, None),
            };
            authors.insert(m.target_id.clone(), pair);
        }
        let lookup = |m: &MarkRow| authors.get(&m.target_id).cloned().unwrap_or((None, None));
        for m in moderation::refresh_marks(&self.env.db, log.state(), &cid, &lookup).await? {
            if let Some(row) = db::content::get(&self.env.db, &m.target_id).await? {
                self.resolve(log, &row).await?;
            }
        }
        Ok(())
    }

    // ─── Requests ───────────────────────────────────────────────────────────

    /// The requests waiting in a channel. For moderators.
    pub async fn requests(&self, keys: &Keys, community_id: &str) -> Result<Vec<RequestRow>> {
        let state = self.state(community_id).await?;
        if !Self::is_moderator(&state, &Self::me(keys)?) {
            return Err(not_permitted());
        }
        db::channels::requests(&self.env.db, community_id, "in", "pending").await
    }

    pub async fn requests_count(&self, community_id: &str) -> Result<usize> {
        Ok(db::channels::requests(&self.env.db, community_id, "in", "pending").await?.len())
    }

    /// A request answered: an admission to the keeper and a copy to the
    /// person (`channel.admit`), or `channel.rejected`.
    pub async fn request_act(&self, keys: &Keys, community_id: &str, requester: &PubKey, approve: bool) -> Result<Outcome> {
        if approve {
            self.approve_request(keys, community_id, requester).await
        } else {
            self.reject_request(keys, community_id, requester).await
        }
    }

    // ─── Invitations ────────────────────────────────────────────────────────

    /// An invitation of mine to `who` (wire §7; the limits of groups).
    pub async fn invite_create(&self, keys: &Keys, community_id: &str, who: &PubKey) -> Result<(String, Outcome)> {
        self.invite(keys, community_id, who).await
    }

    /// The invitations I made in a channel, newest first.
    pub async fn invite_list(&self, community_id: &str) -> Result<Vec<InviteRow>> {
        db::channels::expire_invites(&self.env.db, self.now()).await?;
        let mut all: Vec<InviteRow> = db::channels::invites(&self.env.db, "out", &["sent", "accepted", "done", "declined", "expired", "cancelled"])
            .await?
            .into_iter()
            .filter(|i| i.community_id == community_id)
            .collect();
        all.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        Ok(all)
    }

    /// Takes back an invitation not answered yet: its answer is not taken
    /// from then on.
    pub async fn invite_revoke(&self, invite_id: &str) -> Result<Outcome> {
        let row = db::channels::invite(&self.env.db, invite_id).await?.filter(|i| i.direction == "out").ok_or_else(|| invalid("unknown invitation"))?;
        if row.status == "sent" {
            db::channels::set_invite_status(&self.env.db, invite_id, "cancelled").await?;
        }
        let mut out = Outcome::default();
        out.event(UI_EVENT_CHANNEL_INVITE, serde_json::json!({ "community_id": row.community_id, "invite_id": invite_id }));
        Ok(out)
    }

    /// The invitations open now against their limits.
    pub async fn invite_status(&self, community_id: &str) -> Result<InviteStatus> {
        db::channels::expire_invites(&self.env.db, self.now()).await?;
        let open = db::channels::invites(&self.env.db, "out", &["sent", "accepted"]).await?;
        Ok(InviteStatus {
            open: open.len(),
            max_open: MAX_OPEN_INVITES,
            of_channel: open.iter().filter(|i| i.community_id == community_id).count(),
            max_per_channel: MAX_INVITES_PER_CHANNEL,
            max_per_person: MAX_INVITES_PER_PERSON,
        })
    }
}

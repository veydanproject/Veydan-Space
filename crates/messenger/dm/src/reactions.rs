// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Reactions (docs/messenger-wire.md §3, "Реакции"). A person puts up to
//! three emoji on a message, each once; a message carries at most three
//! different ones. A second tap takes mine back. In a direct chat a
//! reaction is a note to the peer with a copy for my devices; in a group it
//! is a quiet sealed note to the members. Nothing here wakes anybody.
//!
//! Each (message, author, emoji) is kept on its own and the later
//! `created_at` wins, so notes may come in any order and twice. A reaction
//! is kept by the chat it came in, never by what the note says, so nobody
//! can put one under a message of a chat they are not in.
//!
//! The limits are checked in full where a reaction is made. Where it comes
//! it is kept whatever stands already: what has arrived so far depends on
//! the order, and the order must not decide what everyone sees. Of one
//! author's emoji only the earliest three count, read from the rows
//! (`messenger_store::reactions::MAX_PER_AUTHOR`); whether a message has
//! three different ones is left to the one who reacts.
//!
//! A reaction is kept even before its message comes, and so even when it
//! names one that never does; such rows are swept a week after they came
//! (`prune_reactions_if_due`), and a deleted chat takes its own along.

use crate::relationship::{outbound_permission, DenyReason, OutboundPermission};
use crate::service::{updated, DmService};
use crate::view::MessageView;
use crate::wrap::wrap_note;
use messenger_core::emoji::is_reaction;
use messenger_core::{Effect, Envelope, EventId, MessengerError, Outbound, PubKey, Result};
use messenger_store::messages as repo;
use messenger_store::reactions::{self, ReactionRow};
use messenger_store::settings;
use nostr::key::Keys;
use std::collections::HashSet;

/// Emoji one person may have on one message.
pub const MAX_MINE_PER_MESSAGE: usize = reactions::MAX_PER_AUTHOR as usize;
/// Different emoji one message may carry; checked where a reaction is made.
pub const MAX_DISTINCT_PER_MESSAGE: usize = 3;
/// A reaction whose message is not in its chat this long after it came is
/// forgotten: a stranger's notes naming ids that never come do not pile up.
pub const ORPHAN_KEEP_SECS: i64 = 7 * 86_400;
/// How often that is looked at.
pub const PRUNE_EVERY_SECS: i64 = 86_400;
/// When reactions were last swept here (unix seconds).
pub const KEY_REACTIONS_PRUNED_AT: &str = "reactions.pruned_at";

/// Why a reaction was not made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a reaction: text, too long, or invisible characters.
    Invalid,
    /// Three of mine on the message already, or three different and this
    /// one would be a fourth.
    Limit,
}

impl Refusal {
    /// The code the host translates.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::Invalid => "reaction_invalid",
            Refusal::Limit => "reaction_limit",
        }
    }
}

impl From<Refusal> for MessengerError {
    fn from(r: Refusal) -> Self {
        MessengerError::Invalid(r.code().into())
    }
}

/// What a tap on `emoji` does, given what stands on the message
/// (`active`: rows not taken back, of every author): `false` takes mine
/// back, `true` puts it. Taking back is always allowed.
pub fn decide(active: &[ReactionRow], me: &str, emoji: &str) -> std::result::Result<bool, Refusal> {
    if active.iter().any(|r| r.author == me && r.emoji == emoji && !r.removed) {
        return Ok(false);
    }
    if !is_reaction(emoji) {
        return Err(Refusal::Invalid);
    }
    let mine = active.iter().filter(|r| r.author == me && !r.removed).count();
    if mine >= MAX_MINE_PER_MESSAGE {
        return Err(Refusal::Limit);
    }
    let distinct: HashSet<&str> = active.iter().filter(|r| !r.removed).map(|r| r.emoji.as_str()).collect();
    if !distinct.contains(emoji) && distinct.len() >= MAX_DISTINCT_PER_MESSAGE {
        return Err(Refusal::Limit);
    }
    Ok(true)
}

/// A reaction made in a direct chat, applied here and ready to go.
#[derive(Clone, Debug)]
pub struct PreparedReaction {
    pub chat_id: String,
    /// The message with the reaction as it stands now.
    pub message: MessageView,
    /// The note for the peer.
    pub to_peer: Outbound,
    /// The copy for my other devices.
    pub to_self: Option<Outbound>,
}

impl DmService {
    /// `author` put (`set`) or took back `emoji` on `target`, said at `at`,
    /// in `chat_id`: the chat of whoever sent it (`dm:<peer>`, `group:<id>`),
    /// never one named by the note. Shared by direct chats and groups, and
    /// by what is made here and what comes. What makes no sense is dropped
    /// without a word. Kept even when the message is not here yet: it may
    /// come later, and is shown with it then. Kept over the author's limit
    /// too: what counts is read from the rows, whatever came first.
    pub async fn apply_reaction(
        &self,
        chat_id: &str,
        author: &PubKey,
        target: &str,
        emoji: &str,
        set: bool,
        at: i64,
    ) -> Result<Vec<Effect>> {
        let Some(target) = EventId::parse(target) else { return Ok(vec![]) };
        let target = target.as_hex();
        if !is_reaction(emoji) {
            return Ok(vec![]);
        }
        let row = ReactionRow {
            message_id: target.to_string(),
            chat_id: chat_id.to_string(),
            author: author.as_hex().to_string(),
            emoji: emoji.to_string(),
            created_at: at,
            removed: !set,
        };
        if !reactions::put(&self.store, &row, self.clock.now().secs()).await? {
            return Ok(vec![]);
        }
        match repo::get(&self.store, target).await? {
            Some(m) if m.chat_id == chat_id && !m.is_hidden && m.deleted_at.is_none() => {
                Ok(vec![Effect::Emit(updated(chat_id, target))])
            }
            _ => Ok(vec![]),
        }
    }

    /// Tap `emoji` on `message_id` in `chat_id` as `me`: decide, stamp and
    /// apply it here, count the use. Returns whether it was put and its
    /// time. The caller has checked that the message may be reacted to and
    /// builds what goes out with this very time, so that its echo changes
    /// nothing.
    pub async fn react_locally(&self, chat_id: &str, me: &PubKey, message_id: &str, emoji: &str) -> Result<(bool, i64)> {
        let active = reactions::active(&self.store, message_id, chat_id).await?;
        let set = decide(&active, me.as_hex(), emoji)?;
        // Later than any earlier word of mine on it, or a quick second tap
        // in the same second would lose to the first.
        let now = self.clock.now().secs();
        let at = match reactions::get(&self.store, message_id, me.as_hex(), emoji).await? {
            Some(before) => now.max(before.created_at + 1),
            None => now,
        };
        self.apply_reaction(chat_id, me, message_id, emoji, set, at).await?;
        if set {
            self.emoji_used(emoji, now).await?;
        }
        Ok((set, at))
    }

    /// Put `emoji` on a message of a direct chat, or take mine back if it
    /// is there. The message must be here, shown and not deleted, in a chat
    /// with someone I send notes to.
    pub async fn prepare_reaction(&self, keys: &Keys, message_id: &str, emoji: &str) -> Result<PreparedReaction> {
        let row = repo::get(&self.store, message_id)
            .await?
            .filter(|r| !r.is_hidden && r.content_type != repo::CT_SYSTEM)
            .ok_or_else(|| MessengerError::Invalid("unknown message".into()))?;
        if row.deleted_at.is_some() {
            return Err(MessengerError::Invalid("message is deleted".into()));
        }
        let peer = row
            .chat_id
            .strip_prefix("dm:")
            .and_then(PubKey::parse)
            .ok_or_else(|| MessengerError::Invalid("not a direct chat".into()))?;
        let me = PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| MessengerError::Crypto("bad key".into()))?;
        if peer == me {
            return Err(MessengerError::Invalid("not a direct chat".into()));
        }
        if !self.notes_allowed(&peer).await? {
            // Why, in the words of a message refused; a chat not yet agreed
            // on both sides is waiting, whatever a request could do.
            let r = self.load_relation(&peer).await?;
            let code = match outbound_permission(&r, 1, true) {
                OutboundPermission::Deny(reason) => reason.as_str(),
                _ => DenyReason::WaitingApproval.as_str(),
            };
            return Err(MessengerError::Invalid(code.into()));
        }
        let (set, at) = self.react_locally(&row.chat_id, &me, &row.id, emoji).await?;
        let w = wrap_note(keys, &peer, &Envelope::reaction(&row.id, emoji, set).encode(), at, true, None)?;
        let to_peer = Outbound::PublishToInbox { recipient: peer.clone(), event: w.to_peer, hint_relays: self.hints(&peer).await? };
        let to_self = w.to_self.map(|event| Outbound::PublishOwn { event });
        let message = self.message(&row.id).await?.ok_or_else(|| MessengerError::Storage("message vanished".into()))?;
        Ok(PreparedReaction { chat_id: row.chat_id, message, to_peer, to_self })
    }

    /// Forget the reactions whose message has not come within
    /// `ORPHAN_KEEP_SECS` of them, once a day at most. How many went.
    pub async fn prune_reactions_if_due(&self, now: i64) -> Result<u64> {
        let last = settings::get(&self.store, KEY_REACTIONS_PRUNED_AT).await?.and_then(|s| s.parse::<i64>().ok());
        if last.is_some_and(|at| at > now - PRUNE_EVERY_SECS) {
            return Ok(0);
        }
        let gone = reactions::prune_orphans(&self.store, now - ORPHAN_KEEP_SECS).await?;
        settings::set(&self.store, KEY_REACTIONS_PRUNED_AT, &now.to_string()).await?;
        Ok(gone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(author: &str, emoji: &str) -> ReactionRow {
        ReactionRow {
            message_id: "ab".repeat(32),
            chat_id: "dm:x".into(),
            author: author.into(),
            emoji: emoji.into(),
            created_at: 1,
            removed: false,
        }
    }

    #[test]
    fn a_tap_puts_or_takes_back() {
        assert_eq!(decide(&[], "me", "👍"), Ok(true));
        assert_eq!(decide(&[row("me", "👍")], "me", "👍"), Ok(false));
        assert_eq!(decide(&[row("you", "👍")], "me", "👍"), Ok(true), "the same emoji of another is joined");
    }

    #[test]
    fn what_is_no_reaction_is_refused_but_mine_goes_back_whatever_it_is() {
        for bad in ["", "a", "ok", "👍 ", "Да", "\u{200b}"] {
            assert_eq!(decide(&[], "me", bad), Err(Refusal::Invalid), "{bad:?}");
        }
        assert_eq!(decide(&[row("me", "ok")], "me", "ok"), Ok(false), "taking back is always allowed");
    }

    #[test]
    fn limits_of_a_message() {
        let three_mine = [row("me", "👍"), row("me", "❤️"), row("me", "😂")];
        assert_eq!(decide(&three_mine, "me", "🔥"), Err(Refusal::Limit), "a fourth of mine");
        assert_eq!(decide(&three_mine, "me", "😂"), Ok(false), "one of mine goes back");

        let three_kinds = [row("a", "👍"), row("b", "❤️"), row("c", "😂")];
        assert_eq!(decide(&three_kinds, "me", "🔥"), Err(Refusal::Limit), "a fourth kind");
        assert_eq!(decide(&three_kinds, "me", "❤️"), Ok(true), "one of the three is joined");

        let mut taken_back = row("a", "😮");
        taken_back.removed = true;
        assert_eq!(decide(&[row("a", "👍"), row("b", "❤️"), taken_back], "me", "🔥"), Ok(true), "what was taken back does not count");
    }

    #[test]
    fn refusals_reach_the_host_as_codes() {
        assert_eq!(MessengerError::from(Refusal::Limit).to_string(), MessengerError::Invalid("reaction_limit".into()).to_string());
        assert!(matches!(MessengerError::from(Refusal::Invalid), MessengerError::Invalid(c) if c == "reaction_invalid"));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Reactions and the emoji I use most, as the host asks for them. A
//! reaction is made and applied by the DM or the group service and leaves
//! through the outbox here. The map of emoji goes to my other devices from
//! the session's receipt loop, at most every ten minutes, and when the
//! session closes.

use crate::MessengerRuntime;
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::{Clock, Envelope, Outbound, Result};
use messenger_dm::wrap::wrap_own;
use messenger_dm::{DmService, MessageView, UI_EVENT_DM_UPDATED};
use messenger_ingress::Outbox;
use nostr::key::Keys;

/// How long the outbox tries a reaction to the peer. The copy for my own
/// devices is tried until it leaves.
pub const REACTION_TTL_SECS: i64 = 7 * 86_400;

/// Queue the map of emoji for my other devices if it is owed: at most every
/// ten minutes, or whenever `closing`. `true` when one was queued; one that
/// could not be is owed again.
pub(crate) async fn send_emoji_snapshot(dm: &DmService, outbox: &Outbox, keys: &Keys, now: i64, closing: bool) -> Result<bool> {
    let due = if closing { dm.emoji_snapshot_now(now).await? } else { dm.emoji_snapshot_if_due(now).await? };
    let Some(note) = due else { return Ok(false) };
    if let Err(e) = tell_own(outbox, keys, &note, now).await {
        dm.emoji_snapshot_again().await?;
        return Err(e);
    }
    Ok(true)
}

async fn tell_own(outbox: &Outbox, keys: &Keys, note: &Envelope, now: i64) -> Result<()> {
    let event = wrap_own(keys, &note.encode(), now)?;
    outbox.enqueue(Outbound::PublishOwn { event }).await?;
    Ok(())
}

impl MessengerRuntime {
    /// Put `emoji` on a message (a direct chat or a group), or take mine
    /// back if it is there. Refusals: `reaction_limit`, `reaction_invalid`,
    /// and those of the chat (`dm_blocked`, `group_muted`, …).
    pub async fn dm_react(&self, message_id: &str, emoji: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let (chat_id, fallback) = if self.is_group_message(message_id).await? {
            let (chat_id, out) = self.groups().prepare_reaction(&keys, message_id, emoji).await?;
            self.outbox.enqueue(out).await?;
            (chat_id, None)
        } else {
            let p = self.dm.prepare_reaction(&keys, message_id, emoji).await?;
            self.outbox.enqueue_for(p.to_peer, REACTION_TTL_SECS).await?;
            if let Some(own) = p.to_self {
                self.outbox.enqueue(own).await?;
            }
            (p.chat_id, Some(p.message))
        };
        self.outbox.kick();
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": chat_id, "message_id": message_id }),
        });
        match self.dm.message(message_id).await? {
            Some(view) => Ok(view),
            None => fallback.ok_or_else(|| messenger_core::MessengerError::Storage("message vanished".into())),
        }
    }

    /// I picked `emoji` in the composer: it counts among the ones I use most.
    pub async fn emoji_used(&self, emoji: &str) -> Result<()> {
        self.dm.emoji_used(emoji, SystemClock.now().secs()).await
    }

    /// The `n` emoji I use most, on any of my devices; the most used first.
    pub async fn emoji_top(&self, n: usize) -> Result<Vec<String>> {
        self.dm.emoji_top(n).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipts::tests::with_a_message_of;
    use messenger_core::envelope::{KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR, T_OWN_EMOJI, T_REACTION};
    use messenger_dm::ReactionView;
    use nostr::nips::nip59::UnwrappedGift;
    use nostr::prelude::Event;

    /// What the outbox holds, opened: the kind and content of each rumor,
    /// whether it is the peer's, its deadline, and whether it wakes nobody.
    struct Queued {
        to_peer: bool,
        kind: u16,
        note: Envelope,
        expires_at: Option<i64>,
        created_at: i64,
        silent: bool,
    }

    async fn queued(rt: &MessengerRuntime, me: &Keys, peer: &Keys) -> Vec<Queued> {
        let rows = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap();
        let mut out = Vec::new();
        for r in &rows {
            let (to_peer, event, opener) = match r.outbound().unwrap() {
                Outbound::PublishToInbox { event, .. } => (true, event, peer),
                Outbound::PublishOwn { event } => (false, event, me),
                _ => continue,
            };
            let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
            let silent = ev.tags.iter().any(|t| t.as_slice() == ["silent", "1"]);
            // Not every event of mine is a wrap (the list of my inbox relays).
            let Ok(opened) = UnwrappedGift::from_gift_wrap(opener, &ev) else { continue };
            let rumor = opened.rumor;
            out.push(Queued {
                to_peer,
                kind: rumor.kind.as_u16(),
                note: Envelope::parse(&rumor.content).unwrap(),
                expires_at: r.expires_at,
                created_at: r.created_at,
                silent,
            });
        }
        out
    }

    /// The maps of emoji in the outbox.
    async fn maps(rt: &MessengerRuntime, me: &Keys, peer: &Keys) -> Vec<Queued> {
        queued(rt, me, peer).await.into_iter().filter(|q| q.note.t == T_OWN_EMOJI).collect()
    }

    #[tokio::test]
    async fn dm_react_queues_a_note_for_the_peer_and_a_copy_for_me() {
        let peer = Keys::generate();
        let (_dir, rt, _) = with_a_message_of(&peer).await;
        let me = rt.identity().load_keys().await.unwrap();
        let id = "22".repeat(32);

        let view = rt.dm_react(&id, "👍").await.unwrap();
        assert_eq!(view.reactions, vec![ReactionView { emoji: "👍".into(), count: 1, mine: true }]);
        let notes: Vec<Queued> = queued(&rt, &me, &peer).await.into_iter().filter(|q| q.note.t == T_REACTION).collect();
        assert_eq!(notes.len(), 2, "one for the peer, one for my devices");
        for q in &notes {
            assert_eq!(q.kind, KIND_PEER_NOTE_RUMOR);
            assert_eq!(q.note, Envelope::reaction(&id, "👍", true));
            assert!(q.silent, "a reaction wakes nobody");
        }
        let peer_copy = notes.iter().find(|q| q.to_peer).unwrap();
        assert_eq!(peer_copy.expires_at, Some(peer_copy.created_at + REACTION_TTL_SECS));
        assert_eq!(notes.iter().find(|q| !q.to_peer).unwrap().expires_at, None, "my copy is tried until it leaves");

        // Again: taken back.
        let view = rt.dm_react(&id, "👍").await.unwrap();
        assert!(view.reactions.is_empty());
        let code = |e: messenger_core::MessengerError| match e {
            messenger_core::MessengerError::Invalid(c) => c,
            other => other.to_string(),
        };
        assert_eq!(code(rt.dm_react(&id, "abc").await.unwrap_err()), "reaction_invalid");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn the_map_of_emoji_goes_to_my_devices_once_in_ten_minutes() {
        let peer = Keys::generate();
        let (_dir, rt, _) = with_a_message_of(&peer).await;
        let me = rt.identity().load_keys().await.unwrap();
        rt.emoji_used("🎉").await.unwrap();
        assert_eq!(rt.emoji_top(6).await.unwrap(), vec!["🎉"]);
        rt.flush_receipts().await.unwrap();
        let sent = maps(&rt, &me, &peer).await;
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].to_peer && sent[0].silent && sent[0].expires_at.is_none());
        assert_eq!(sent[0].kind, KIND_OWN_RUMOR);
        assert_eq!(sent[0].note.fields["usage"]["🎉"][0], 1);

        rt.emoji_used("🎉").await.unwrap();
        rt.flush_receipts().await.unwrap();
        assert_eq!(maps(&rt, &me, &peer).await.len(), 1, "not twice within ten minutes");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_closing_session_sends_the_map_it_owes() {
        let peer = Keys::generate();
        let (_dir, rt, _) = with_a_message_of(&peer).await;
        let me = rt.identity().load_keys().await.unwrap();
        rt.emoji_used("🎉").await.unwrap();
        rt.flush_receipts().await.unwrap();
        rt.emoji_used("🙏").await.unwrap();
        rt.stop_session().await;
        let maps = maps(&rt, &me, &peer).await;
        assert_eq!(maps.len(), 2, "the second one went at the close");
        assert!(maps.iter().any(|q| q.note.fields["usage"].get("🙏").is_some()));
        rt.shutdown().await;
    }
}

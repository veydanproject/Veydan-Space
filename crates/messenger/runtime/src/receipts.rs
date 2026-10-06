// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Receipts this device owes: which messages of the peers came, and up to
//! where a chat was read here. What is owed is taken from the store once
//! (`DmService::take_due_*`) and goes out as notes, one per peer and kind;
//! the session flushes them every few seconds, so a burst of messages costs
//! one note. A late receipt is worth nothing: it is given up after a day.

use crate::MessengerRuntime;
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, Envelope, Outbound, PubKey, Result};
use messenger_dm::wrap::wrap_note;
use messenger_dm::DmService;
use messenger_groups::GroupService;
use messenger_ingress::Outbox;
use nostr::key::Keys;
use std::time::Duration;

/// How often the session sends what is owed.
pub const RECEIPT_FLUSH_SECS: u64 = 2;
/// How long the outbox tries a receipt.
pub const RECEIPT_TTL_SECS: i64 = 86_400;
/// How long a relay keeps one (NIP-40 on the outer wrap).
pub const RECEIPT_EXPIRATION_SECS: i64 = 7 * 86_400;

pub(crate) const RECEIPT_FLUSH: Duration = Duration::from_secs(RECEIPT_FLUSH_SECS);

/// Queue every receipt that is owed now. Group read receipts are kept by
/// the group, not by a deadline: they are queued like any group event.
/// What was taken and could not be queued is owed again, so the next tick
/// tries it; one that fails does not hold up the others. The map of the
/// emoji I use rides the same tick to my other devices when it is due
/// (`crate::reactions`), and once a day the reactions to messages that
/// never came are swept.
pub(crate) async fn flush(dm: &DmService, groups: &GroupService, outbox: &Outbox, keys: &Keys) -> Result<()> {
    let now = SystemClock.now().secs();
    let me = keys.public_key().to_hex();
    if let Err(e) = dm.prune_reactions_if_due(now).await {
        eprintln!("messenger reactions: {e}");
    }
    let mut queued = match crate::reactions::send_emoji_snapshot(dm, outbox, keys, now, false).await {
        Ok(sent) => sent,
        Err(e) => {
            eprintln!("messenger emoji: {e}");
            false
        }
    };
    for (_, peer, ids) in dm.take_due_delivered(now).await? {
        let Some(peer) = PubKey::parse(&peer).filter(|p| p.as_hex() != me) else { continue };
        let note = Envelope::receipt_delivered(&ids);
        match note_to(dm, outbox, keys, &peer, &note, now).await {
            Ok(()) => queued = true,
            Err(e) => {
                eprintln!("messenger receipts: delivered to {}: {e}", peer.as_hex());
                if let Err(e) = dm.owe_delivered_again(&ids).await {
                    eprintln!("messenger receipts: delivered to {}, lost: {e}", peer.as_hex());
                }
            }
        }
    }
    for (chat_id, at) in dm.take_due_read().await? {
        let sent = if let Some(peer) = chat_id.strip_prefix("dm:").and_then(PubKey::parse) {
            if peer.as_hex() == me {
                continue;
            }
            note_to(dm, outbox, keys, &peer, &Envelope::receipt_read(at), now).await
        } else if let Some(group) = chat_id.strip_prefix("group:") {
            match groups.prepare_read_receipt(keys, group, at).await {
                Ok(out) => outbox.enqueue(out).await.map(|_| ()),
                // Left, or no key yet: there is nobody to tell.
                Err(e) => {
                    eprintln!("messenger receipts: group {group}: {e}");
                    continue;
                }
            }
        } else {
            continue;
        };
        match sent {
            Ok(()) => queued = true,
            Err(e) => {
                eprintln!("messenger receipts: read of {chat_id}: {e}");
                if let Err(e) = dm.owe_read_again(&chat_id).await {
                    eprintln!("messenger receipts: read of {chat_id}, lost: {e}");
                }
            }
        }
    }
    if queued {
        outbox.kick();
    }
    Ok(())
}

async fn note_to(dm: &DmService, outbox: &Outbox, keys: &Keys, peer: &PubKey, note: &Envelope, now: i64) -> Result<()> {
    outbox.enqueue_for(to_peer(dm, keys, peer, note, now).await?, RECEIPT_TTL_SECS).await?;
    Ok(())
}

async fn to_peer(dm: &DmService, keys: &Keys, peer: &PubKey, note: &Envelope, now: i64) -> Result<Outbound> {
    let w = wrap_note(keys, peer, &note.encode(), now, false, Some(now + RECEIPT_EXPIRATION_SECS))?;
    Ok(Outbound::PublishToInbox { recipient: peer.clone(), event: w.to_peer, hint_relays: dm.hints(peer).await? })
}

/// Runs for the life of a session.
pub(crate) async fn receipt_loop(dm: DmService, groups: GroupService, outbox: Outbox, keys: Keys) {
    loop {
        tokio::time::sleep(RECEIPT_FLUSH).await;
        if let Err(e) = flush(&dm, &groups, &outbox, &keys).await {
            eprintln!("messenger receipts: {e}");
        }
    }
}

impl MessengerRuntime {
    /// Send what is owed now instead of at the next tick. Nothing without a
    /// session: what is owed waits for one.
    pub async fn flush_receipts(&self) -> Result<()> {
        let Ok(keys) = self.session_keys().await else { return Ok(()) };
        flush(self.dm(), self.groups(), self.outbox(), &keys).await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use messenger_core::inbound::Envelope as WireEnvelope;
    use messenger_core::{Context, DmInbound, EventId, EventSource, MessengerConfig, RelayUrl, Timestamp};
    use messenger_testkit::MemorySecretStore;
    use nostr::nips::nip59::UnwrappedGift;
    use nostr::prelude::Event;
    use std::sync::Arc;

    /// A runtime with a session and one message of `peer`, as ingress hands
    /// it over; and the time it came.
    pub(crate) async fn with_a_message_of(peer: &Keys) -> (tempfile::TempDir, MessengerRuntime, i64) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        let me = rt.identity().create("pw").await.unwrap().identity.pubkey;
        assert!(rt.refresh_signer().await.unwrap());
        rt.dm().set_gate(false);

        let peer_pk = PubKey::parse(&peer.public_key().to_hex()).unwrap();
        let now = SystemClock.now().secs();
        let msg = DmInbound {
            envelope: WireEnvelope {
                wire_id: EventId::parse(&"11".repeat(32)).unwrap(),
                source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
                wire_created_at: Timestamp(now),
            },
            rumor_id: EventId::parse(&"22".repeat(32)).unwrap(),
            sender: peer_pk,
            recipients: vec![me.clone()],
            created_at: Timestamp(now),
            content: Envelope::text("hi").encode(),
            reply_to: None,
            rumor_kind: 14,
        };
        let ctx = Context { my_pubkey: me, session_started_at: Timestamp(now - 10), clock: Arc::new(SystemClock) };
        rt.dm().apply_inbound(msg, &ctx).await.unwrap();
        (dir, rt, now)
    }

    /// The notes queued for `peer`, in the order of their kind.
    async fn notes_for(rt: &MessengerRuntime, peer: &Keys, now: i64) -> Vec<Envelope> {
        let peer_pk = PubKey::parse(&peer.public_key().to_hex()).unwrap();
        let rows = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap();
        let mut said = Vec::new();
        for r in &rows {
            let Ok(Outbound::PublishToInbox { recipient, event, .. }) = r.outbound() else { continue };
            assert_eq!(recipient, peer_pk);
            assert_eq!(r.expires_at, Some(r.created_at + RECEIPT_TTL_SECS), "a late receipt is given up");
            let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
            let tag = |k: &str| ev.tags.iter().find(|t| t.kind() == k).and_then(|t| t.as_slice().get(1).cloned());
            assert_eq!(tag("silent").as_deref(), Some("1"), "a receipt wakes nobody");
            assert!(tag("expiration").and_then(|s| s.parse::<i64>().ok()).is_some_and(|at| at > now));
            let rumor = UnwrappedGift::from_gift_wrap(peer, &ev).unwrap().rumor;
            assert_eq!(rumor.kind.as_u16(), messenger_core::envelope::KIND_PEER_NOTE_RUMOR);
            said.push(Envelope::parse(&rumor.content).unwrap());
        }
        said.sort_by(|a, b| a.t.cmp(&b.t));
        said
    }

    #[tokio::test]
    async fn a_chat_read_here_leaves_as_notes_with_a_deadline() {
        let peer = Keys::generate();
        let (_dir, rt, now) = with_a_message_of(&peer).await;
        rt.chat_mark_read(&messenger_store::chats::dm_chat_id(&peer.public_key().to_hex())).await.unwrap();

        let said = notes_for(&rt, &peer, now).await;
        assert_eq!(said, vec![Envelope::receipt_delivered(&["22".repeat(32)]), Envelope::receipt_read(now)]);

        rt.flush_receipts().await.unwrap();
        assert_eq!(notes_for(&rt, &peer, now).await, said, "nothing is owed twice");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn what_could_not_be_queued_goes_at_the_next_tick() {
        let peer = Keys::generate();
        let (_dir, rt, now) = with_a_message_of(&peer).await;
        // Where to reach the peer cannot be read for now: no note can be built.
        let pool = rt.store().pool();
        sqlx::query("ALTER TABLE msg_dm_routes RENAME TO msg_dm_routes_away").execute(pool).await.unwrap();
        rt.chat_mark_read(&messenger_store::chats::dm_chat_id(&peer.public_key().to_hex())).await.unwrap();
        assert!(notes_for(&rt, &peer, now).await.is_empty());

        sqlx::query("ALTER TABLE msg_dm_routes_away RENAME TO msg_dm_routes").execute(pool).await.unwrap();
        rt.flush_receipts().await.unwrap();
        assert_eq!(
            notes_for(&rt, &peer, now).await,
            vec![Envelope::receipt_delivered(&["22".repeat(32)]), Envelope::receipt_read(now)],
            "both are owed still, and go now"
        );
        rt.shutdown().await;
    }
}

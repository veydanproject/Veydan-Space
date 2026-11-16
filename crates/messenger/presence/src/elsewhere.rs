// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Which chat another device of mine is reading now, by its focus events
//! ([`crate::focus`]): what this device asks before it shows a message
//! (ROO-1275, D2). A chat read live there is shown here by nothing.
//!
//! The runtime watches my focus key and the handler ([`FocusHandler`])
//! writes down the newest event of it that another device made
//! (`focus.elsewhere`). Which events are this device's own is told by
//! their ids, written before each goes ([`sending`]), not by their time:
//! a device that is in focus itself sends every half minute, and an event
//! of the other device a second older than its own last one is no less
//! live (ROO-1277, П5). An event counts by its own time, once: a copy
//! heard again moves nothing, one dated far ahead of my clock is dropped
//! (ROO-1292). A phone woken by a push has no runtime and reads nothing
//! here (D3 asks the relay instead).

use crate::focus::{self, FocusEvent, FocusKeys, FOCUS_TTL_SECS};
use async_trait::async_trait;
use messenger_core::presence::{KEY_FOCUS_ELSEWHERE, KEY_FOCUS_HEARD, KEY_FOCUS_SENT};
use messenger_core::{Context, Effect, Handler, MetaInbound, Result};
use messenger_store::{settings, Store};
use std::sync::{Arc, RwLock};

/// How many ids of this device's own events are kept: a relay brings back
/// the last one or two; the rest is margin. As many of the other devices'
/// are kept, four minutes of their beats.
pub const SENT_KEPT: usize = 8;

/// How far ahead of my clock an event may be dated and still count: a
/// clock a little fast is common, one further is not taken at its word
/// (an event that counts by its own time would hold the chat that long).
pub const FOCUS_AHEAD_SECS: i64 = 30;

/// What `focus.elsewhere` writes for an event that says "in no chat".
const NO_CHAT: &str = "-";

/// The focus keys of this session, which the runtime puts in when it
/// watches them and the handler opens events with. Empty: no session, and
/// what comes is dropped.
pub type FocusSlot = Arc<RwLock<Option<Arc<FocusKeys>>>>;

/// Whether an event of another device, seen at `seen_at` and good until
/// `until`, says it is in its chat at `now`: before its end, and no older
/// than one event's promise, whatever its end says.
pub fn live(seen_at: i64, until: i64, now: i64) -> bool {
    now < until && now - seen_at < FOCUS_TTL_SECS
}

/// This device sends the event `id`: written down before it goes, so its
/// copy from a relay, in this process or the next, is never taken for
/// another device's.
pub async fn sending(store: &Store, id: &str) -> Result<()> {
    remember(store, KEY_FOCUS_SENT, id).await
}

/// `id` first in the list under `key`, once, the oldest beyond
/// [`SENT_KEPT`] gone.
async fn remember(store: &Store, key: &str, id: &str) -> Result<()> {
    let kept = settings::get(store, key).await?.unwrap_or_default();
    let mut ids: Vec<&str> = vec![id];
    ids.extend(kept.split(' ').filter(|s| !s.is_empty() && *s != id).take(SENT_KEPT - 1));
    settings::set(store, key, &ids.join(" ")).await
}

async fn listed(store: &Store, key: &str, id: &str) -> Result<bool> {
    Ok(settings::get(store, key).await?.is_some_and(|s| s.split(' ').any(|x| x == id)))
}

async fn sent_here(store: &Store, id: &str) -> Result<bool> {
    listed(store, KEY_FOCUS_SENT, id).await
}

/// What was heard last: the chat (`None`: in no chat), `seen_at`, `until`.
async fn read(store: &Store) -> Result<Option<(Option<String>, i64, i64)>> {
    Ok(settings::get(store, KEY_FOCUS_ELSEWHERE).await?.and_then(|s| {
        // The chat first: split from the end, so that nothing in it matters.
        let mut parts = s.rsplitn(3, ' ');
        let until = parts.next()?.parse().ok()?;
        let seen_at = parts.next()?.parse().ok()?;
        let chat = parts.next()?;
        Some(((chat != NO_CHAT).then(|| chat.to_string()), seen_at, until))
    }))
}

/// An event `id` of my focus key came, opened: kept when another device
/// made it, it was not heard before and it is no older than the one kept.
/// `seen_at` and `until` as the handler counts them. `true` when kept.
pub async fn heard(store: &Store, id: &str, chat: Option<&str>, seen_at: i64, until: i64) -> Result<bool> {
    if sent_here(store, id).await? || listed(store, KEY_FOCUS_HEARD, id).await? {
        return Ok(false);
    }
    remember(store, KEY_FOCUS_HEARD, id).await?;
    if let Some((_, kept, _)) = read(store).await? {
        // Of one second, the later to come: a device that moves from one
        // chat to another within a second says so in that order.
        if seen_at < kept {
            return Ok(false);
        }
    }
    let chat = chat.filter(|c| !c.is_empty() && !c.contains(' ')).unwrap_or(NO_CHAT);
    settings::set(store, KEY_FOCUS_ELSEWHERE, &format!("{chat} {seen_at} {until}")).await?;
    Ok(true)
}

/// Whether another device of mine is reading `chat` at `now`, by what was
/// heard last. Reads the store only.
pub async fn reading(store: &Store, chat: &str, now: i64) -> Result<bool> {
    let Some((Some(there), seen_at, until)) = read(store).await? else { return Ok(false) };
    Ok(there == chat && live(seen_at, until, now))
}

/// `MetaInbound::Focus` → `focus.elsewhere`. An event that does not open
/// with my keys (another epoch, a relay that let a stranger through, a
/// content touched) is dropped. Shown to nobody: nothing is emitted.
pub struct FocusHandler {
    store: Store,
    keys: FocusSlot,
}

impl FocusHandler {
    pub fn new(store: Store, keys: FocusSlot) -> Self {
        Self { store, keys }
    }
}

#[async_trait]
impl Handler<MetaInbound> for FocusHandler {
    async fn handle(&self, msg: MetaInbound, ctx: &Context) -> Result<Vec<Effect>> {
        let MetaInbound::Focus { author, id, created_at, expires_at, content } = msg else {
            return Ok(vec![]);
        };
        let Some(keys) = self.keys.read().unwrap().clone() else { return Ok(vec![]) };
        let event = FocusEvent { author, created_at: created_at.secs(), expires_at: expires_at.map(|t| t.secs()), content };
        let Ok(said) = focus::open(&keys, &event) else { return Ok(vec![]) };
        // By its own time, so that a copy of it, whenever it comes, says
        // what it said the first time; dated far ahead, not at all.
        if event.created_at > ctx.clock.now().secs() + FOCUS_AHEAD_SECS {
            return Ok(vec![]);
        }
        let seen_at = event.created_at;
        let until = event.until().min(seen_at + FOCUS_TTL_SECS);
        heard(&self.store, id.as_hex(), said.chat.as_deref(), seen_at, until).await?;
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{Clock, EventId, PubKey, Timestamp};
    use nostr::key::Keys;
    use nostr::prelude::Event;

    #[test]
    fn an_event_of_another_device_counts_while_it_lasts() {
        assert!(live(1_000, 1_080, 1_000));
        assert!(live(1_000, 1_080, 1_079));
        assert!(!live(1_000, 1_080, 1_080), "gone at its end");
        assert!(!live(1_000, 5_000, 1_080), "older than an event promises");
    }

    #[tokio::test]
    async fn this_devices_own_events_are_told_by_their_ids_not_their_time() {
        let store = Store::open_in_memory().await.unwrap();
        assert!(!reading(&store, "dm:a", 1_000).await.unwrap(), "nothing heard");

        sending(&store, "mine-1").await.unwrap();
        assert!(!heard(&store, "mine-1", Some("dm:a"), 1_000, 1_080).await.unwrap(), "my own copy");
        assert!(!reading(&store, "dm:a", 1_010).await.unwrap());

        // The phone said dm:a a second before this device's own last event:
        // live all the same (П5 of ROO-1277).
        sending(&store, "mine-2").await.unwrap();
        assert!(heard(&store, "phone-1", Some("dm:a"), 999, 1_079).await.unwrap());
        assert!(reading(&store, "dm:a", 1_010).await.unwrap());
        assert!(!reading(&store, "dm:b", 1_010).await.unwrap(), "another chat");
        assert!(!reading(&store, "dm:a", 1_079).await.unwrap(), "ended");

        // An older event of the phone does not move it back; the later of
        // one second does.
        assert!(!heard(&store, "phone-0", Some("dm:b"), 990, 1_070).await.unwrap());
        assert!(heard(&store, "phone-2", Some("dm:b"), 999, 1_079).await.unwrap());
        assert!(reading(&store, "dm:b", 1_010).await.unwrap());

        // The phone went to the list: no chat is read there.
        assert!(heard(&store, "phone-3", None, 1_020, 1_100).await.unwrap());
        assert!(!reading(&store, "dm:b", 1_030).await.unwrap());
        assert_eq!(settings::get(&store, KEY_FOCUS_ELSEWHERE).await.unwrap().as_deref(), Some("- 1020 1100"));

        // The oldest of mine go when more are sent.
        for n in 3..3 + SENT_KEPT - 1 {
            sending(&store, &format!("mine-{n}")).await.unwrap();
        }
        assert!(!sent_here(&store, "mine-1").await.unwrap(), "pushed out");
        assert!(sent_here(&store, "mine-2").await.unwrap());
        sending(&store, "mine-2").await.unwrap();
        let kept = settings::get(&store, KEY_FOCUS_SENT).await.unwrap().unwrap();
        assert_eq!(kept.split(' ').count(), SENT_KEPT);
        assert!(kept.starts_with("mine-2 "), "sent again: first, once");
    }

    struct At(i64);
    impl Clock for At {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    fn ctx_at(now: i64) -> Context {
        Context {
            my_pubkey: PubKey::parse(&"11".repeat(32)).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(At(now)),
        }
    }

    fn inbound(wire: &messenger_core::outbound::WireEvent) -> MetaInbound {
        let event: Event = serde_json::from_value(wire.json.clone()).unwrap();
        let expires_at = event
            .tags
            .iter()
            .find(|t| t.kind() == "expiration")
            .and_then(|t| t.as_slice().get(1))
            .and_then(|s| s.parse().ok())
            .map(Timestamp);
        MetaInbound::Focus {
            author: PubKey::parse(&event.pubkey.to_hex()).unwrap(),
            id: EventId::parse(&event.id.to_hex()).unwrap(),
            created_at: Timestamp(event.created_at.as_secs() as i64),
            expires_at,
            content: event.content,
        }
    }

    /// A copy of an event heard again (another relay, a relay that sends it
    /// twice, later) moves nothing: neither its end nor a later word
    /// (ROO-1292).
    #[tokio::test]
    async fn a_copy_heard_again_neither_lasts_longer_nor_covers_a_later_word() {
        let store = Store::open_in_memory().await.unwrap();
        let me = Keys::generate();
        let keys = FocusKeys::derive(&me, 0);
        let h = FocusHandler::new(store.clone(), Arc::new(RwLock::new(Some(Arc::new(FocusKeys::derive(&me, 0))))));
        let now = 1_759_700_000;
        // Dated as far ahead as is taken.
        let x = inbound(&focus::build(&keys, Some("dm:x"), now + FOCUS_AHEAD_SECS).unwrap());
        h.handle(x.clone(), &ctx_at(now)).await.unwrap();
        let end = now + FOCUS_AHEAD_SECS + FOCUS_TTL_SECS;
        assert!(reading(&store, "dm:x", end - 1).await.unwrap());

        // Again near its end, and after it: it ends when it ended.
        h.handle(x.clone(), &ctx_at(end - 10)).await.unwrap();
        assert!(!reading(&store, "dm:x", end).await.unwrap());
        h.handle(x.clone(), &ctx_at(end + 10)).await.unwrap();
        assert!(!reading(&store, "dm:x", end + 11).await.unwrap());

        // The phone moved to Y, then to no chat; X again covers neither.
        let y = inbound(&focus::build(&keys, Some("dm:y"), end + 20).unwrap());
        h.handle(y.clone(), &ctx_at(end + 20)).await.unwrap();
        h.handle(x.clone(), &ctx_at(end + 21)).await.unwrap();
        assert!(reading(&store, "dm:y", end + 22).await.unwrap());
        assert!(!reading(&store, "dm:x", end + 22).await.unwrap());
        let none = inbound(&focus::build(&keys, None, end + 30).unwrap());
        h.handle(none, &ctx_at(end + 30)).await.unwrap();
        for copy in [x, y] {
            h.handle(copy, &ctx_at(end + 31)).await.unwrap();
        }
        assert!(!reading(&store, "dm:x", end + 32).await.unwrap());
        assert!(!reading(&store, "dm:y", end + 32).await.unwrap());

        // Of one second, a copy of the earlier word heard after the later.
        let a = inbound(&focus::build(&keys, Some("dm:a"), end + 40).unwrap());
        let b = inbound(&focus::build(&keys, Some("dm:b"), end + 40).unwrap());
        h.handle(a.clone(), &ctx_at(end + 40)).await.unwrap();
        h.handle(b, &ctx_at(end + 40)).await.unwrap();
        h.handle(a, &ctx_at(end + 41)).await.unwrap();
        assert!(reading(&store, "dm:b", end + 42).await.unwrap());
    }

    #[tokio::test]
    async fn the_handler_opens_with_my_keys_and_keeps_another_devices_word() {
        let store = Store::open_in_memory().await.unwrap();
        let me = Keys::generate();
        let slot: FocusSlot = Arc::default();
        let h = FocusHandler::new(store.clone(), slot.clone());
        let now = 1_759_700_000;
        let keys = FocusKeys::derive(&me, 0);
        let phone = focus::build(&keys, Some("dm:x"), now).unwrap();

        assert!(h.handle(inbound(&phone), &ctx_at(now)).await.unwrap().is_empty());
        assert!(!reading(&store, "dm:x", now + 1).await.unwrap(), "no session: no keys, dropped");

        *slot.write().unwrap() = Some(Arc::new(FocusKeys::derive(&me, 0)));
        assert!(h.handle(inbound(&phone), &ctx_at(now)).await.unwrap().is_empty(), "shown to nobody");
        assert!(reading(&store, "dm:x", now + 1).await.unwrap());
        assert!(!reading(&store, "dm:x", now + FOCUS_TTL_SECS).await.unwrap());

        // This device's own word, sent later, changes nothing of it.
        let own = focus::build(&keys, Some("dm:y"), now + 10).unwrap();
        sending(&store, own.id.as_hex()).await.unwrap();
        h.handle(inbound(&own), &ctx_at(now + 10)).await.unwrap();
        assert!(reading(&store, "dm:x", now + 11).await.unwrap());
        assert!(!reading(&store, "dm:y", now + 11).await.unwrap());

        // A stranger's key, another epoch: not opened.
        let stranger = focus::build(&FocusKeys::derive(&Keys::generate(), 0), Some("dm:z"), now + 20).unwrap();
        let other_epoch = focus::build(&FocusKeys::derive(&me, 1), Some("dm:z"), now + 20).unwrap();
        for e in [stranger, other_epoch] {
            h.handle(inbound(&e), &ctx_at(now + 20)).await.unwrap();
        }
        assert!(!reading(&store, "dm:z", now + 21).await.unwrap());

        // A clock a little fast: by its own time, so it ends a TTL from it.
        let ahead = focus::build(&keys, Some("dm:w"), now + 30 + FOCUS_AHEAD_SECS).unwrap();
        h.handle(inbound(&ahead), &ctx_at(now + 30)).await.unwrap();
        assert!(reading(&store, "dm:w", now + 31).await.unwrap());
        assert!(reading(&store, "dm:w", now + 30 + FOCUS_TTL_SECS).await.unwrap());
        assert!(!reading(&store, "dm:w", now + 30 + FOCUS_AHEAD_SECS + FOCUS_TTL_SECS).await.unwrap());
        // Far ahead: not taken.
        let far = focus::build(&keys, Some("dm:v"), now + 31 + FOCUS_AHEAD_SECS).unwrap();
        h.handle(inbound(&far), &ctx_at(now + 30)).await.unwrap();
        assert!(!reading(&store, "dm:v", now + 31).await.unwrap());
        assert!(reading(&store, "dm:w", now + 31).await.unwrap());

        // Not a focus event: nothing.
        let beat = MetaInbound::Presence { author: PubKey::parse(&"aa".repeat(32)).unwrap(), created_at: Timestamp(now), expires_at: None };
        assert!(h.handle(beat, &ctx_at(now + 40)).await.unwrap().is_empty());
    }
}

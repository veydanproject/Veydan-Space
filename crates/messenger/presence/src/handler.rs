// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Beats that come in. A beat names a presence key; it counts only when a
//! contact told me that key, and only when it is newer than the last one.

use crate::heartbeat::ONLINE_TTL_SECS;
use async_trait::async_trait;
use messenger_core::traits::UiEvent;
use messenger_core::presence::KEY_PRESENCE;
use messenger_core::{Context, Effect, Handler, MetaInbound, Result};
use messenger_store::{presence, settings, Store};

/// `{"peer": "<hex>", "seen_at": <secs>, "online_until": <secs>}`
pub const UI_EVENT_PRESENCE_UPDATED: &str = "presence.updated";

/// `MetaInbound::Presence` → `msg_presence`. Everything else in the meta
/// family is someone else's.
pub struct PresenceHandler {
    store: Store,
}

impl PresenceHandler {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Handler<MetaInbound> for PresenceHandler {
    async fn handle(&self, msg: MetaInbound, ctx: &Context) -> Result<Vec<Effect>> {
        let MetaInbound::Presence { author, created_at, expires_at } = msg else {
            return Ok(vec![]);
        };
        // Presence off: I am shown nobody, whatever a subscription still
        // open for a moment brings.
        if !settings::get_bool(&self.store, KEY_PRESENCE, true).await? {
            return Ok(vec![]);
        }
        // A key nobody told me: a stranger, or a contact that rotated away.
        let Some(peer) = presence::peer_of(&self.store, author.as_hex()).await? else {
            return Ok(vec![]);
        };
        // A beat counts no further than my clock: one dated ahead by a fast
        // clock would hold back the honest beats after it, and no beat
        // promises more than two of ours.
        let seen_at = created_at.secs().min(ctx.clock.now().secs());
        // No expiration: the beat is taken to say what ours say.
        let online_until = expires_at.map_or(seen_at + ONLINE_TTL_SECS, |t| t.secs()).min(seen_at + 2 * ONLINE_TTL_SECS);
        if !presence::seen(&self.store, author.as_hex(), seen_at, online_until).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_PRESENCE_UPDATED.into(),
            payload: serde_json::json!({ "peer": peer, "seen_at": seen_at, "online_until": online_until }),
        })])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::traits::SystemClock;
    use messenger_core::{PubKey, Timestamp};
    use std::sync::Arc;

    fn ctx() -> Context {
        Context {
            my_pubkey: PubKey::parse(&"11".repeat(32)).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(SystemClock),
        }
    }

    fn beat(author: &str, at: i64, expires: Option<i64>) -> MetaInbound {
        MetaInbound::Presence {
            author: PubKey::parse(author).unwrap(),
            created_at: Timestamp(at),
            expires_at: expires.map(Timestamp),
        }
    }

    fn emitted(effects: &[Effect]) -> Vec<serde_json::Value> {
        effects
            .iter()
            .map(|e| match e {
                Effect::Emit(ui) => {
                    assert_eq!(ui.name, UI_EVENT_PRESENCE_UPDATED);
                    ui.payload.clone()
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn a_beat_of_a_known_key_moves_the_contact_forward_only() {
        let store = Store::open_in_memory().await.unwrap();
        let (peer, key) = ("aa".repeat(32), "bb".repeat(32));
        presence::put_key(&store, &peer, Some(&key), 1).await.unwrap();
        let h = PresenceHandler::new(store.clone());

        let out = h.handle(beat(&key, 100, Some(180)), &ctx()).await.unwrap();
        assert_eq!(emitted(&out), vec![serde_json::json!({ "peer": peer, "seen_at": 100, "online_until": 180 })]);
        assert!(h.handle(beat(&key, 100, Some(180)), &ctx()).await.unwrap().is_empty(), "a replay");
        assert!(h.handle(beat(&key, 90, Some(170)), &ctx()).await.unwrap().is_empty(), "an older beat");

        let out = h.handle(beat(&key, 130, None), &ctx()).await.unwrap();
        assert_eq!(
            emitted(&out),
            vec![serde_json::json!({ "peer": peer, "seen_at": 130, "online_until": 130 + ONLINE_TTL_SECS })],
            "no expiration: our own TTL"
        );
        assert_eq!(crate::view::snapshot(&store).await.unwrap()[0].seen_at, 130);
    }

    #[tokio::test]
    async fn with_presence_off_a_beat_shows_nothing() {
        let store = Store::open_in_memory().await.unwrap();
        let (peer, key) = ("aa".repeat(32), "bb".repeat(32));
        presence::put_key(&store, &peer, Some(&key), 1).await.unwrap();
        let h = PresenceHandler::new(store.clone());
        settings::set_bool(&store, KEY_PRESENCE, false).await.unwrap();
        assert!(h.handle(beat(&key, 100, Some(180)), &ctx()).await.unwrap().is_empty());
        assert!(crate::view::snapshot(&store).await.unwrap().is_empty(), "nor is it kept");
        settings::set_bool(&store, KEY_PRESENCE, true).await.unwrap();
        assert_eq!(emitted(&h.handle(beat(&key, 100, Some(180)), &ctx()).await.unwrap()).len(), 1);
    }

    struct At(i64);
    impl messenger_core::Clock for At {
        fn now(&self) -> Timestamp {
            Timestamp(self.0)
        }
    }

    fn ctx_at(now: i64) -> Context {
        Context { clock: Arc::new(At(now)), ..ctx() }
    }

    #[tokio::test]
    async fn a_beat_from_the_future_or_promising_too_much_is_cut_to_size() {
        let store = Store::open_in_memory().await.unwrap();
        let (peer, key) = ("aa".repeat(32), "bb".repeat(32));
        presence::put_key(&store, &peer, Some(&key), 1).await.unwrap();
        let h = PresenceHandler::new(store.clone());
        let now = 1_759_700_000;

        // A fast clock: seen now, not an hour on, so the honest beat after it counts.
        let out = h.handle(beat(&key, now + 3_600, Some(now + 3_680)), &ctx_at(now)).await.unwrap();
        assert_eq!(emitted(&out), vec![serde_json::json!({ "peer": peer, "seen_at": now, "online_until": now + 2 * ONLINE_TTL_SECS })]);
        let out = h.handle(beat(&key, now + 30, Some(now + 110)), &ctx_at(now + 30)).await.unwrap();
        assert_eq!(emitted(&out), vec![serde_json::json!({ "peer": peer, "seen_at": now + 30, "online_until": now + 110 })]);

        // A beat that says "online for a year" is taken for two of ours.
        let out = h.handle(beat(&key, now + 60, Some(now + 365 * 86_400)), &ctx_at(now + 60)).await.unwrap();
        assert_eq!(emitted(&out)[0]["online_until"], now + 60 + 2 * ONLINE_TTL_SECS);
    }

    #[tokio::test]
    async fn a_beat_of_an_unknown_key_and_other_meta_do_nothing() {
        let store = Store::open_in_memory().await.unwrap();
        let h = PresenceHandler::new(store.clone());
        let stranger = "cc".repeat(32);
        assert!(h.handle(beat(&stranger, 100, Some(180)), &ctx()).await.unwrap().is_empty());
        assert!(presence::seen(&store, &stranger, 100, 180).await.unwrap(), "nothing was kept of it");

        let profile = MetaInbound::Profile {
            author: PubKey::parse(&"dd".repeat(32)).unwrap(),
            created_at: Timestamp(1),
            content: "{}".into(),
        };
        assert!(h.handle(profile, &ctx()).await.unwrap().is_empty());
    }
}

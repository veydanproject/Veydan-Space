// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The alarm of a keeper that sleeps on a phone (internal/communities-wire.md
//! §7, 2026-10-22; internal/communities-stage1.md §4).
//!
//! A push server wakes a phone for the wraps addressed to the key its user
//! registered there, as it does for a direct message: nobody calls it. The
//! keeper's own key is registered nowhere, so a note to the keeper
//! (`channel.join`, `rebundle`, `missed`, an admission) wakes nobody. The
//! sender therefore also sends the channel's owner, whose device keeps the
//! channel, a note `channel.wake` the way a note goes to a person: to the
//! owner's inbox relays, where the owner's push server watches. Its wrap
//! carries the mark `["channel", "1"]` on the outside (`pushtags::CHANNEL`
//! of the messenger): the phone shows nothing for it and gives the keeper a
//! turn (the push plugin's `Keeper.kt`); the engine takes the note and does
//! nothing with it. Waking the owner's devices is all the network allows:
//! the owner's other devices wake for nothing, and an owner who switched
//! the pushes of direct messages off is not woken.
//!
//! The wake follows the note to the keeper by a moment, once a channel per
//! `WAKE_EVERY`: a newcomer's first join is sent before the row of the
//! channel is written, which names the owner.

use async_trait::async_trait;
use messenger_core::outbound::WireEvent;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, Envelope, EventId, MessengerError, Outbound, PubKey, RawEvent, Result, Transport};
use nostr::key::Keys;
use nostr::nips::nip59::GiftWrapBuilder;
use nostr::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;
use tokio::sync::mpsc;
use veydan_core::BoxFuture;

/// The mark on the outside of the wrap of a wake, as the messenger's
/// `pushtags::CHANNEL` and the push plugin's `Keeper.kt` name it: the tag
/// of a push this module owns.
pub(crate) const PUSH_TAG: &str = "channel";
/// The type of the note inside.
pub(crate) const T_WAKE: &str = "channel.wake";
/// One wake a channel this often at most: a burst of notes to its keeper
/// wakes it once.
pub(crate) const WAKE_EVERY: u64 = 30;
/// How long the wake waits after the note to the keeper.
const WAKE_AFTER: Duration = Duration::from_secs(1);

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

/// The wake of the keeper of `community_id`, from `from` to the channel's
/// `owner`: rumor 30079 `{"t": "channel.wake", "community_id"}`, sealed by
/// `from`, wrapped to the owner with the mark of the push on the outside.
pub(crate) fn wake_note(from: &Keys, owner: &PubKey, community_id: &str, created_at: i64) -> Result<WireEvent> {
    let to = PublicKey::from_hex(owner.as_hex()).map_err(crypto)?;
    let mut envelope = Envelope::new(T_WAKE);
    envelope.fields.insert("community_id".into(), community_id.into());
    let mut rumor = EventBuilder::new(Kind::from(messenger_core::envelope::KIND_PEER_NOTE_RUMOR), envelope.encode())
        .tag(Tag::public_key(to))
        .custom_created_at(Timestamp::from_secs(u64::try_from(created_at).unwrap_or(0)))
        .finalize_unsigned(from.public_key());
    rumor.ensure_id();
    let mark = Tag::parse([PUSH_TAG, "1"]).map_err(crypto)?;
    let event = GiftWrapBuilder::new(to, rumor).extra_tags([mark]).finalize(from).map_err(crypto)?;
    Ok(WireEvent { id: EventId::parse(&event.id.to_hex()).ok_or_else(|| crypto("event id"))?, json: serde_json::to_value(&event)? })
}

/// Whom a note to a keeper wakes.
pub(crate) struct Target {
    pub(crate) community_id: String,
    pub(crate) owner: PubKey,
    /// Who sends the wake: the identity at work.
    pub(crate) from: Keys,
}

/// The keeper's key to the channel it keeps and its owner; none when the
/// channel is not known here, or is mine (my own keeper is woken by
/// nothing but my own device).
pub(crate) type Resolve = Arc<dyn Fn(PubKey) -> BoxFuture<'static, Option<Target>> + Send + Sync>;

/// The messenger's network as the engine meets it, with the wake added
/// after every note to a keeper. A note to a keeper goes with the relays
/// of its channel as the hint, a note to a person without one (the
/// engine's `send_note`), which is how the two are told apart.
pub(crate) struct WakingNet {
    inner: Arc<dyn Transport>,
    resolve: RwLock<Option<Resolve>>,
    /// When each channel was last woken, unix seconds.
    woken: std::sync::Mutex<HashMap<String, u64>>,
    me: Weak<WakingNet>,
    after: Duration,
}

impl WakingNet {
    pub(crate) fn new(inner: Arc<dyn Transport>) -> Arc<Self> {
        Self::with_delay(inner, WAKE_AFTER)
    }

    fn with_delay(inner: Arc<dyn Transport>, after: Duration) -> Arc<Self> {
        Arc::new_cyclic(|me| Self { inner, resolve: RwLock::new(None), woken: Default::default(), me: me.clone(), after })
    }

    /// The way from a keeper's key to its channel; none when the service
    /// goes, which also breaks the circle of the service holding this.
    pub(crate) fn set_resolve(&self, resolve: Option<Resolve>) {
        *self.resolve.write().unwrap_or_else(|e| e.into_inner()) = resolve;
    }

    /// A channel woken not long ago is not woken again; the time is taken.
    fn due(&self, community_id: &str, now: u64) -> bool {
        let mut woken = self.woken.lock().unwrap_or_else(|e| e.into_inner());
        if woken.get(community_id).is_some_and(|at| now < at + WAKE_EVERY) {
            return false;
        }
        woken.insert(community_id.to_string(), now);
        true
    }

    /// The wake of the keeper `keeper`, a moment later, off the sender's way.
    fn follow(&self, keeper: PubKey) {
        let Some(resolve) = self.resolve.read().unwrap_or_else(|e| e.into_inner()).clone() else { return };
        let Some(me) = self.me.upgrade() else { return };
        tokio::spawn(async move {
            tokio::time::sleep(me.after).await;
            let Some(target) = resolve(keeper).await else { return };
            let now = crate::state::now_secs();
            if !me.due(&target.community_id, now) {
                return;
            }
            let created_at = i64::try_from(now).unwrap_or(i64::MAX);
            let event = match wake_note(&target.from, &target.owner, &target.community_id, created_at) {
                Ok(event) => event,
                Err(e) => {
                    crate::log::clog!("the keeper's wake was not made: {e}");
                    return;
                }
            };
            let out = Outbound::PublishToInbox { recipient: target.owner, event, hint_relays: Vec::new() };
            if let Err(e) = me.inner.send(out).await {
                crate::log::clog!("the keeper's wake did not go out: {e}");
            }
        });
    }
}

#[async_trait]
impl Transport for WakingNet {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        let keeper = match &out {
            Outbound::PublishToInbox { recipient, hint_relays, .. } if !hint_relays.is_empty() => Some(recipient.clone()),
            _ => None,
        };
        let ack = self.inner.send(out).await?;
        if let Some(keeper) = keeper {
            self.follow(keeper);
        }
        Ok(ack)
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        self.inner.events()
    }

    async fn status(&self) -> RelayStatusSnapshot {
        self.inner.status().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::RelayUrl;
    use nostr::nips::nip59::UnwrappedGift;

    /// The messenger's pool: what is sent, kept.
    #[derive(Default)]
    struct Pool(std::sync::Mutex<Vec<Outbound>>);

    #[async_trait]
    impl Transport for Pool {
        async fn send(&self, out: Outbound) -> Result<Ack> {
            self.0.lock().unwrap().push(out);
            Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
        }
        fn events(&self) -> mpsc::Receiver<RawEvent> {
            mpsc::channel(1).1
        }
        async fn status(&self) -> RelayStatusSnapshot {
            RelayStatusSnapshot::default()
        }
    }

    fn pk(keys: &Keys) -> PubKey {
        PubKey::parse(&keys.public_key().to_hex()).unwrap()
    }

    fn note(to: &PubKey, hint: bool) -> Outbound {
        let hint_relays = if hint { vec![RelayUrl::parse("wss://channel.example").unwrap()] } else { vec![] };
        let event = WireEvent { id: EventId::parse(&"ab".repeat(32)).unwrap(), json: serde_json::json!({}) };
        Outbound::PublishToInbox { recipient: to.clone(), event, hint_relays }
    }

    async fn settle() {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A join to the keeper of a channel (the channel's relays as the hint)
    /// is followed by a wake to the channel's owner: to the owner's inbox,
    /// with the mark of the push outside and `channel.wake` inside, from me.
    /// A note to a person wakes nobody, and a burst wakes the keeper once.
    #[tokio::test]
    async fn a_note_to_a_keeper_wakes_the_owner_once_for_a_burst() {
        let pool = Arc::new(Pool::default());
        let net = WakingNet::with_delay(pool.clone(), Duration::ZERO);
        let (me, owner, keeper) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (owner_pk, keeper_pk, from) = (pk(&owner), pk(&keeper), me.clone());
        let resolve: Resolve = Arc::new(move |k: PubKey| {
            let hit = (k == keeper_pk).then(|| Target { community_id: "c1".into(), owner: owner_pk.clone(), from: from.clone() });
            Box::pin(async move { hit })
        });
        net.set_resolve(Some(resolve));

        net.send(note(&pk(&keeper), true)).await.unwrap();
        settle().await;
        let sent = pool.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 2, "the note and its wake");
        let Outbound::PublishToInbox { recipient, event, hint_relays } = &sent[1] else { panic!("a wake to an inbox") };
        assert_eq!(recipient, &pk(&owner));
        assert!(hint_relays.is_empty(), "to the owner's own inbox relays");
        let tags = event.json["tags"].as_array().unwrap();
        assert!(tags.iter().any(|t| t[0] == PUSH_TAG && t[1] == "1"), "the mark of the push outside");
        let wrap: Event = serde_json::from_value(event.json.clone()).unwrap();
        let gift = UnwrappedGift::from_gift_wrap(&owner, &wrap).unwrap();
        assert_eq!(gift.sender, me.public_key());
        let inside = Envelope::parse(&gift.rumor.content).unwrap();
        assert_eq!((inside.t.as_str(), inside.str_field("community_id")), (T_WAKE, Some("c1")));

        // Two more notes to the keeper at once, and one to a person.
        net.send(note(&pk(&keeper), true)).await.unwrap();
        net.send(note(&pk(&keeper), true)).await.unwrap();
        net.send(note(&pk(&Keys::generate()), false)).await.unwrap();
        settle().await;
        assert_eq!(pool.0.lock().unwrap().len(), 5, "no second wake within {WAKE_EVERY} s, none for a person");
    }

    /// A keeper this device does not know (or mine) wakes nobody, and after
    /// the service went nothing is resolved any more.
    #[tokio::test]
    async fn an_unknown_keeper_wakes_nobody() {
        let pool = Arc::new(Pool::default());
        let net = WakingNet::with_delay(pool.clone(), Duration::ZERO);
        net.set_resolve(Some(Arc::new(|_| Box::pin(async { None }))));
        net.send(note(&pk(&Keys::generate()), true)).await.unwrap();
        net.set_resolve(None);
        net.send(note(&pk(&Keys::generate()), true)).await.unwrap();
        settle().await;
        assert_eq!(pool.0.lock().unwrap().len(), 2);
    }
}

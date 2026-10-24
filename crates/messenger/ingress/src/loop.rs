// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `IngressLoop`: consumes a transport's event stream, dedups, classifies,
//! dispatches. One per session; dropping the handle stops it.

use crate::classify::{classify, KIND_GIFT_WRAP, KIND_PRESENCE};
use messenger_core::envelope::{KIND_GROUP_TYPING, KIND_TYPING_WRAP};
use crate::dispatch::{Dispatcher, EffectSink};
use messenger_core::{Context, Inbound, RawEvent};
use messenger_store::{banned, events_raw, servers, Store};
use nostr::key::Keys;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Kinds that are not kept in `msg_events_raw`. A presence beat is
/// replaceable and comes twice a minute per contact: kept, it would pile up
/// forever, and a replay of one does no harm (its handler only moves
/// forward). A «typing» is ephemeral: the relays keep none, and one
/// heard twice shows the same word twice.
pub const TRANSIENT_KINDS: &[u16] = &[KIND_PRESENCE, KIND_TYPING_WRAP, KIND_GROUP_TYPING];

/// Whether an event of `kind` skips the dedup table.
pub fn is_transient(kind: u16) -> bool {
    TRANSIENT_KINDS.contains(&kind)
}

/// Counters for the status screen.
#[derive(Default)]
pub struct IngressStats {
    pub received: AtomicU64,
    pub duplicates: AtomicU64,
    pub dispatched: AtomicU64,
    pub dm: AtomicU64,
    pub ignored: AtomicU64,
}

impl IngressStats {
    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        (
            self.received.load(Ordering::Relaxed),
            self.duplicates.load(Ordering::Relaxed),
            self.dispatched.load(Ordering::Relaxed),
            self.dm.load(Ordering::Relaxed),
            self.ignored.load(Ordering::Relaxed),
        )
    }
}

pub struct IngressLoop {
    handle: JoinHandle<()>,
    pub stats: Arc<IngressStats>,
}

impl IngressLoop {
    /// `keys` opens gift wraps; `None` means DMs are ignored until the host
    /// unlocks (the loop is restarted by the runtime when that happens).
    pub fn spawn(
        mut events: mpsc::Receiver<RawEvent>,
        store: Store,
        keys: Option<Keys>,
        dispatcher: Arc<Dispatcher>,
        ctx: Context,
        sink: Arc<dyn EffectSink>,
    ) -> Self {
        let stats = Arc::new(IngressStats::default());
        let st = stats.clone();
        let handle = tokio::spawn(async move {
            while let Some(raw) = events.recv().await {
                st.received.fetch_add(1, Ordering::Relaxed);
                let kept = !is_transient(raw.kind);
                if kept {
                    match events_raw::contains(&store, &raw.id).await {
                        Ok(false) => {}
                        Ok(true) => {
                            st.duplicates.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        Err(e) => {
                            eprintln!("messenger ingress: dedup store failed: {e}");
                            continue;
                        }
                    }
                }
                // A gift wrap that came while locked was not read: it stays
                // unseen, and the history sync after unlock brings it again.
                let unread = keys.is_none() && raw.kind == KIND_GIFT_WRAP;
                let inbound = classify(&raw, keys.as_ref());
                // An event of a group the network closes (`banned_groups` of
                // the manifest) is neither handled nor kept as seen: when the
                // ban lifts, the next history sync brings it again.
                if let Inbound::Group(g) = &inbound {
                    if banned::group_contains(&store, &g.group_id).await.unwrap_or(false) {
                        st.ignored.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    // A group lives on its server, and only there
                    // (`servers::group_heard_on`): its event by another
                    // relay is not taken, nor kept as seen, so the copy of
                    // the group's own server is taken when it comes.
                    let relay = g.envelope.source.relay_url().map(|u| u.as_str());
                    if !servers::group_heard_on(&store, &g.group_id, relay).await.unwrap_or(true) {
                        eprintln!("messenger ingress: an event of a group by a relay not of the group's server, dropped");
                        st.ignored.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                }
                match &inbound {
                    Inbound::Dm(_) => {
                        st.dm.fetch_add(1, Ordering::Relaxed);
                    }
                    Inbound::Ignored { .. } => {
                        st.ignored.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                st.dispatched.fetch_add(1, Ordering::Relaxed);
                // Seen only once applied: an event whose handler failed is
                // taken again from the next copy. Events come one at a
                // time, so a duplicate still finds the row of the first.
                let applied = dispatcher.dispatch(inbound, &ctx, sink.as_ref()).await;
                if kept && applied && !unread {
                    if let Err(e) = events_raw::insert_if_new(&store, &raw).await {
                        eprintln!("messenger ingress: dedup store failed: {e}");
                    }
                }
            }
        });
        Self { handle, stats }
    }

    pub fn abort(&self) {
        self.handle.abort();
    }
}

impl Drop for IngressLoop {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use messenger_core::traits::{SystemClock, UiEvent};
    use messenger_core::{Ack, EventId, EventSource, Outbound, PubKey, RelayUrl, Result, Timestamp};
    use nostr::nips::nip17::PrivateDirectMessageBuilder;
    use nostr::prelude::*;
    use std::sync::Mutex;
    use std::time::Duration;

    struct Sink(Mutex<Vec<UiEvent>>);
    #[async_trait]
    impl EffectSink for Sink {
        async fn send(&self, _: Outbound) -> Result<Ack> {
            Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
        }
        fn emit(&self, e: UiEvent) {
            self.0.lock().unwrap().push(e);
        }
        fn notify(&self, _: messenger_core::Notice) {}
    }

    fn raw_of(event: &Event) -> RawEvent {
        RawEvent {
            id: EventId::parse(&event.id.to_hex()).unwrap(),
            kind: event.kind.as_u16(),
            pubkey: PubKey::parse(&event.pubkey.to_hex()).unwrap(),
            created_at: Timestamp(event.created_at.as_secs() as i64),
            json: serde_json::to_value(event).unwrap(),
            source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
        }
    }

    #[tokio::test]
    async fn loop_dedups_and_counts() {
        let store = Store::open_in_memory().await.unwrap();
        let alice = Keys::generate();
        let bob = Keys::generate();
        let (tx, rx) = mpsc::channel(8);
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let ctx = Context {
            my_pubkey: PubKey::parse(&bob.public_key().to_hex()).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(SystemClock),
        };
        let lp = IngressLoop::spawn(rx, store, Some(bob.clone()), Arc::new(Dispatcher::new()), ctx, sink.clone());

        let wrap = PrivateDirectMessageBuilder::new(bob.public_key(), "x").finalize(&alice).unwrap();
        let note = EventBuilder::new(Kind::from(1u16), "n").finalize(&alice).unwrap();
        tx.send(raw_of(&wrap)).await.unwrap();
        tx.send(raw_of(&wrap)).await.unwrap(); // duplicate
        tx.send(raw_of(&note)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let (received, dups, dispatched, dm, ignored) = lp.stats.snapshot();
        assert_eq!((received, dups, dispatched, dm, ignored), (3, 1, 2, 1, 1));
        // No DM handler registered → reported as ignored to the sink, plus the kind-1 note.
        assert_eq!(sink.0.lock().unwrap().len(), 2);
        drop(lp);
    }

    #[test]
    fn only_presence_beats_are_transient() {
        assert!(is_transient(KIND_PRESENCE));
        for kind in [0u16, 3, 9, 14, 1059, 10002, 10050, 30078, 30079] {
            assert!(!is_transient(kind), "kind {kind}");
        }
    }

    #[tokio::test]
    async fn presence_beats_are_dispatched_but_never_kept() {
        let store = Store::open_in_memory().await.unwrap();
        let me = Keys::generate();
        let presence = Keys::generate();
        let (tx, rx) = mpsc::channel(8);
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let ctx = Context {
            my_pubkey: PubKey::parse(&me.public_key().to_hex()).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(SystemClock),
        };
        let lp = IngressLoop::spawn(rx, store.clone(), Some(me.clone()), Arc::new(Dispatcher::new()), ctx, sink.clone());

        let beat = EventBuilder::new(Kind::from(KIND_PRESENCE), "")
            .tag(Tag::parse(["d", crate::classify::PRESENCE_D]).unwrap())
            .finalize(&presence)
            .unwrap();
        let note = EventBuilder::new(Kind::from(1u16), "n").finalize(&presence).unwrap();
        tx.send(raw_of(&beat)).await.unwrap();
        tx.send(raw_of(&beat)).await.unwrap(); // a replay goes through
        tx.send(raw_of(&note)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let (received, dups, dispatched, _, _) = lp.stats.snapshot();
        assert_eq!((received, dups, dispatched), (3, 0, 3));
        assert_eq!(events_raw::count(&store).await.unwrap(), 1, "only the kind-1 note is kept");
        drop(lp);
    }

    /// A group handler that counts what reached it.
    #[derive(Default)]
    struct Groups(std::sync::atomic::AtomicUsize);

    #[async_trait]
    impl messenger_core::Handler<messenger_core::GroupInbound> for Groups {
        async fn handle(&self, _: messenger_core::GroupInbound, _: &Context) -> Result<Vec<messenger_core::Effect>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        }
    }

    /// An event of a group the network closes is neither handled nor kept
    /// as seen, so the copy that comes after the ban lifts is taken.
    #[tokio::test]
    async fn an_event_of_a_closed_group_is_not_taken_nor_kept() {
        let store = Store::open_in_memory().await.unwrap();
        let me = Keys::generate();
        let handler = Arc::new(Groups::default());
        let dispatcher = Arc::new(Dispatcher::new().with_group(handler.clone()));
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let event = |group: &str| {
            EventBuilder::new(Kind::from(crate::classify::KIND_GROUP_MESSAGE), "sealed")
                .tag(Tag::parse(["h", group]).unwrap())
                .finalize(&Keys::generate())
                .unwrap()
        };
        let (closed, open) = (event("closed"), event("open"));
        banned::group_replace(&store, &["closed".to_string()]).await.unwrap();

        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), Some(me.clone()), dispatcher.clone(), ctx_of(&me), sink.clone());
        tx.send(raw_of(&closed)).await.unwrap();
        tx.send(raw_of(&open)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(handler.0.load(Ordering::SeqCst), 1, "only the open group's");
        assert!(!events_raw::contains(&store, &raw_of(&closed).id).await.unwrap(), "not seen");
        assert!(events_raw::contains(&store, &raw_of(&open).id).await.unwrap());
        drop(lp);

        banned::group_replace(&store, &[]).await.unwrap();
        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), Some(me.clone()), dispatcher, ctx_of(&me), sink);
        tx.send(raw_of(&closed)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(handler.0.load(Ordering::SeqCst), 2, "lifted: taken");
        assert!(events_raw::contains(&store, &raw_of(&closed).id).await.unwrap());
        drop(lp);
    }

    /// A group of a private server is heard from that server's relay
    /// alone: its event by a relay of the home set is neither handled nor
    /// kept as seen, and the same event by the server's relay is taken; a
    /// group of the home set is heard by any relay but a private server's.
    #[tokio::test]
    async fn an_event_of_a_group_is_taken_from_its_server_alone() {
        let store = Store::open_in_memory().await.unwrap();
        let me = Keys::generate();
        let handler = Arc::new(Groups::default());
        let dispatcher = Arc::new(Dispatcher::new().with_group(handler.clone()));
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let server = |url: &str, kind: &str| servers::ServerRow {
            id: String::new(),
            kind: kind.into(),
            name: None,
            relay_url: url.into(),
            relay_key: None,
            media_url: None,
            call_node: None,
            push_mode: servers::PUSH_OFF.into(),
            source: None,
            added_at: 0,
        };
        servers::put(&store, &server("wss://s1.example", servers::KIND_PRIVATE)).await.unwrap();
        servers::put(&store, &server("wss://home.example", servers::KIND_HOME)).await.unwrap();
        let group = |id: &str, relay: &str| messenger_store::groups::GroupRow {
            id: id.into(),
            kind: "private".into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay_url: relay.into(),
            owner: "o".into(),
            membership: "joined".into(),
            my_role: None,
            members: 1,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        };
        messenger_store::groups::upsert(&store, &group("on-s1", "wss://S1.example/")).await.unwrap();
        messenger_store::groups::upsert(&store, &group("at-home", "wss://home.example")).await.unwrap();
        let event = |group: &str| {
            EventBuilder::new(Kind::from(crate::classify::KIND_GROUP_MESSAGE), "sealed")
                .tag(Tag::parse(["h", group]).unwrap())
                .finalize(&Keys::generate())
                .unwrap()
        };
        let by = |event: &Event, url: &str| RawEvent { source: EventSource::Relay { url: RelayUrl::parse(url).unwrap() }, ..raw_of(event) };
        let (private, home) = (event("on-s1"), event("at-home"));

        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), Some(me.clone()), dispatcher, ctx_of(&me), sink);
        tx.send(by(&private, "wss://home.example")).await.unwrap();
        tx.send(by(&home, "wss://s1.example")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(handler.0.load(Ordering::SeqCst), 0, "each by the other's relay");
        assert!(!events_raw::contains(&store, &raw_of(&private).id).await.unwrap(), "not seen");
        assert!(!events_raw::contains(&store, &raw_of(&home).id).await.unwrap(), "not seen");
        tx.send(by(&private, "wss://s1.example")).await.unwrap();
        tx.send(by(&home, "wss://home.example")).await.unwrap();
        tx.send(by(&event("at-home"), "wss://unknown.example")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(handler.0.load(Ordering::SeqCst), 3, "each by its own server; home by a relay of no server");
        assert!(events_raw::contains(&store, &raw_of(&private).id).await.unwrap());
        drop(lp);
    }

    /// A DM handler that fails its first `fail` calls, then takes them.
    struct Flaky {
        calls: std::sync::atomic::AtomicUsize,
        fail: usize,
    }

    #[async_trait]
    impl messenger_core::Handler<messenger_core::DmInbound> for Flaky {
        async fn handle(&self, _: messenger_core::DmInbound, _: &Context) -> Result<Vec<messenger_core::Effect>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) < self.fail {
                return Err(messenger_core::MessengerError::Storage("busy".into()));
            }
            Ok(vec![])
        }
    }

    fn ctx_of(k: &Keys) -> Context {
        Context {
            my_pubkey: PubKey::parse(&k.public_key().to_hex()).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(SystemClock),
        }
    }

    #[tokio::test]
    async fn a_gift_wrap_that_came_while_locked_is_read_after_unlock() {
        let store = Store::open_in_memory().await.unwrap();
        let alice = Keys::generate();
        let bob = Keys::generate();
        let handler = Arc::new(Flaky { calls: Default::default(), fail: 0 });
        let dispatcher = Arc::new(Dispatcher::new().with_dm(handler.clone()));
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let wrap = PrivateDirectMessageBuilder::new(bob.public_key(), "x").finalize(&alice).unwrap();

        // Locked: nothing opens the wrap, and nothing marks it seen.
        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), None, dispatcher.clone(), ctx_of(&bob), sink.clone());
        tx.send(raw_of(&wrap)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(lp.stats.snapshot().4, 1, "ignored without a signer");
        assert_eq!(events_raw::count(&store).await.unwrap(), 0, "not seen");
        drop(lp);

        // Unlocked: the loop is restarted and the next copy is read, once.
        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), Some(bob.clone()), dispatcher, ctx_of(&bob), sink);
        tx.send(raw_of(&wrap)).await.unwrap();
        tx.send(raw_of(&wrap)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (received, dups, _, dm, _) = lp.stats.snapshot();
        assert_eq!((received, dups, dm), (2, 1, 1));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert_eq!(events_raw::count(&store).await.unwrap(), 1);
        drop(lp);
    }

    #[tokio::test]
    async fn an_event_whose_handler_failed_is_taken_again() {
        let store = Store::open_in_memory().await.unwrap();
        let alice = Keys::generate();
        let bob = Keys::generate();
        let handler = Arc::new(Flaky { calls: Default::default(), fail: 1 });
        let dispatcher = Arc::new(Dispatcher::new().with_dm(handler.clone()));
        let sink = Arc::new(Sink(Mutex::new(vec![])));
        let (tx, rx) = mpsc::channel(8);
        let lp = IngressLoop::spawn(rx, store.clone(), Some(bob.clone()), dispatcher, ctx_of(&bob), sink.clone());

        let wrap = PrivateDirectMessageBuilder::new(bob.public_key(), "x").finalize(&alice).unwrap();
        tx.send(raw_of(&wrap)).await.unwrap(); // fails
        tx.send(raw_of(&wrap)).await.unwrap(); // taken
        tx.send(raw_of(&wrap)).await.unwrap(); // a duplicate now
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(handler.calls.load(Ordering::SeqCst), 2);
        assert_eq!(lp.stats.snapshot().1, 1, "one duplicate");
        assert_eq!(events_raw::count(&store).await.unwrap(), 1);
        assert!(sink.0.lock().unwrap().iter().any(|e| e.name == "error"), "the failure is still reported");
        drop(lp);
    }
}

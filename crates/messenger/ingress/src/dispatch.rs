// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Routes `Inbound` to handlers and runs their effects.
//!
//! Ordering rules live here, not in handlers: DMs from one peer are handled
//! strictly one after another (control signals depend on order), different
//! peers run in parallel. Unhandled families are reported to the sink as
//! `ignored` so the UI/debug feed still sees them.

use async_trait::async_trait;
use messenger_core::traits::UiEvent;
use messenger_core::{Ack, Context, DmInbound, Effect, GroupInbound, Handler, Inbound, MetaInbound, Outbound, PubKey, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Where effects go. The runtime implements this over the transport, the
/// outbox and the host's UI channel.
#[async_trait]
pub trait EffectSink: Send + Sync {
    async fn send(&self, out: Outbound) -> Result<Ack>;
    fn emit(&self, event: UiEvent);
    fn notify(&self, notice: messenger_core::Notice);
}

#[derive(Default)]
pub struct Dispatcher {
    dm: Option<Arc<dyn Handler<DmInbound>>>,
    group: Option<Arc<dyn Handler<GroupInbound>>>,
    meta: Option<Arc<dyn Handler<MetaInbound>>>,
    /// One lock per DM peer; taken for the duration of a handler call.
    peer_locks: Mutex<HashMap<PubKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_dm(mut self, h: Arc<dyn Handler<DmInbound>>) -> Self {
        self.dm = Some(h);
        self
    }

    pub fn with_group(mut self, h: Arc<dyn Handler<GroupInbound>>) -> Self {
        self.group = Some(h);
        self
    }

    pub fn with_meta(mut self, h: Arc<dyn Handler<MetaInbound>>) -> Self {
        self.meta = Some(h);
        self
    }

    fn peer_lock(&self, peer: &PubKey) -> Arc<tokio::sync::Mutex<()>> {
        self.peer_locks
            .lock()
            .unwrap()
            .entry(peer.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Handle one inbound and run its effects. Handler errors are reported
    /// through the sink as `error` events and never abort the loop.
    pub async fn dispatch(&self, inbound: Inbound, ctx: &Context, sink: &dyn EffectSink) {
        let family = inbound.family();
        let result = match inbound {
            Inbound::Dm(dm) => match &self.dm {
                Some(h) => {
                    let lock = self.peer_lock(&dm.sender);
                    let _guard = lock.lock().await;
                    Some(h.handle(dm, ctx).await)
                }
                None => None,
            },
            Inbound::Group(g) => match &self.group {
                Some(h) => Some(h.handle(g, ctx).await),
                None => None,
            },
            Inbound::Meta(m) => match &self.meta {
                Some(h) => Some(h.handle(m, ctx).await),
                None => None,
            },
            Inbound::Channel(_) => None,
            Inbound::Ignored { kind, reason } => {
                sink.emit(UiEvent {
                    name: "ignored".into(),
                    payload: serde_json::json!({ "kind": kind, "reason": reason }),
                });
                return;
            }
        };
        match result {
            None => sink.emit(UiEvent {
                name: "ignored".into(),
                payload: serde_json::json!({ "family": family, "reason": "no handler registered" }),
            }),
            Some(Err(e)) => sink.emit(UiEvent {
                name: "error".into(),
                payload: serde_json::json!({ "family": family, "error": e.to_string() }),
            }),
            Some(Ok(effects)) => {
                for effect in effects {
                    match effect {
                        Effect::Send(out) => {
                            if let Err(e) = sink.send(out).await {
                                sink.emit(UiEvent {
                                    name: "error".into(),
                                    payload: serde_json::json!({ "family": family, "error": format!("send: {e}") }),
                                });
                            }
                        }
                        Effect::Emit(ev) => sink.emit(ev),
                        Effect::Notify(notice) => sink.notify(notice),
                    }
                }
            }
        }
    }
}

/// Runs several handlers of one family in order and concatenates their
/// effects. Lets independent crates each own a slice of a family (contacts
/// own profiles, DM owns inbox relay lists) without knowing each other.
pub struct Fanout<I> {
    handlers: Vec<Arc<dyn Handler<I>>>,
}

impl<I> Fanout<I> {
    pub fn new(handlers: Vec<Arc<dyn Handler<I>>>) -> Self {
        Self { handlers }
    }
}

#[async_trait]
impl<I: Clone + Send + Sync + 'static> Handler<I> for Fanout<I> {
    async fn handle(&self, msg: I, ctx: &Context) -> Result<Vec<Effect>> {
        let mut all = Vec::new();
        for h in &self.handlers {
            all.extend(h.handle(msg.clone(), ctx).await?);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::inbound::Envelope;
    use messenger_core::traits::SystemClock;
    use messenger_core::{EventId, EventSource, RelayUrl, Timestamp};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct RecordingSink {
        events: Mutex<Vec<UiEvent>>,
        sent: Mutex<Vec<Outbound>>,
    }

    #[async_trait]
    impl EffectSink for RecordingSink {
        async fn send(&self, out: Outbound) -> Result<Ack> {
            self.sent.lock().unwrap().push(out);
            Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
        }
        fn emit(&self, event: UiEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn notify(&self, _: messenger_core::Notice) {}
    }

    /// Handler that records the order of `content` values and holds each
    /// call for a moment so concurrency is observable.
    struct SlowDm {
        seen: Mutex<Vec<String>>,
        in_flight: AtomicUsize,
        max_in_flight: AtomicUsize,
    }

    #[async_trait]
    impl Handler<DmInbound> for SlowDm {
        async fn handle(&self, msg: DmInbound, _ctx: &Context) -> Result<Vec<Effect>> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.seen.lock().unwrap().push(msg.content.clone());
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(vec![Effect::Emit(UiEvent { name: "dm".into(), payload: serde_json::json!(msg.content) })])
        }
    }

    fn dm(sender: &str, content: &str) -> DmInbound {
        DmInbound {
            envelope: Envelope {
                wire_id: EventId::parse(&"1".repeat(64)).unwrap(),
                source: EventSource::Relay { url: RelayUrl::parse("wss://r.example").unwrap() },
                wire_created_at: Timestamp(1),
            },
            rumor_id: EventId::parse(&"2".repeat(64)).unwrap(),
            sender: PubKey::parse(&sender.repeat(64)).unwrap(),
            recipients: vec![],
            created_at: Timestamp(1),
            content: content.into(),
            reply_to: None,
        }
    }

    fn ctx() -> Context {
        Context {
            my_pubkey: PubKey::parse(&"f".repeat(64)).unwrap(),
            session_started_at: Timestamp(0),
            clock: Arc::new(SystemClock),
        }
    }

    #[tokio::test]
    async fn same_peer_is_serialized_different_peers_run_in_parallel() {
        let handler = Arc::new(SlowDm { seen: Mutex::new(vec![]), in_flight: AtomicUsize::new(0), max_in_flight: AtomicUsize::new(0) });
        let d = Arc::new(Dispatcher::new().with_dm(handler.clone()));
        let sink = Arc::new(RecordingSink { events: Mutex::new(vec![]), sent: Mutex::new(vec![]) });
        let c = ctx();

        // Same peer "a": three messages fired concurrently must complete in order.
        let mut tasks = vec![];
        for i in 0..3 {
            let (d, s, c) = (d.clone(), sink.clone(), c.clone());
            tasks.push(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(i * 5)).await;
                d.dispatch(Inbound::Dm(dm("a", &format!("a{i}"))), &c, s.as_ref()).await;
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(handler.seen.lock().unwrap().clone(), vec!["a0", "a1", "a2"]);
        assert_eq!(handler.max_in_flight.load(Ordering::SeqCst), 1, "peer a never overlaps with itself");

        // Different peers may overlap.
        handler.max_in_flight.store(0, Ordering::SeqCst);
        let (d1, s1, c1) = (d.clone(), sink.clone(), c.clone());
        let (d2, s2, c2) = (d.clone(), sink.clone(), c.clone());
        let t1 = tokio::spawn(async move { d1.dispatch(Inbound::Dm(dm("b", "b")), &c1, s1.as_ref()).await });
        let t2 = tokio::spawn(async move { d2.dispatch(Inbound::Dm(dm("c", "c")), &c2, s2.as_ref()).await });
        t1.await.unwrap();
        t2.await.unwrap();
        assert_eq!(handler.max_in_flight.load(Ordering::SeqCst), 2, "peers b and c overlapped");

        assert_eq!(sink.events.lock().unwrap().iter().filter(|e| e.name == "dm").count(), 5);
    }

    #[tokio::test]
    async fn unhandled_and_ignored_are_reported_not_dropped() {
        let d = Dispatcher::new();
        let sink = RecordingSink { events: Mutex::new(vec![]), sent: Mutex::new(vec![]) };
        d.dispatch(Inbound::ignored(7, "test"), &ctx(), &sink).await;
        d.dispatch(Inbound::Dm(dm("a", "x")), &ctx(), &sink).await;
        let names: Vec<_> = sink.events.lock().unwrap().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["ignored", "ignored"]);
    }
}

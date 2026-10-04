// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! `IngressLoop`: consumes a transport's event stream, dedups, classifies,
//! dispatches. One per session; dropping the handle stops it.

use crate::classify::classify;
use crate::dispatch::{Dispatcher, EffectSink};
use messenger_core::{Context, Inbound, RawEvent};
use messenger_store::{events_raw, Store};
use nostr::key::Keys;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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
                match events_raw::insert_if_new(&store, &raw).await {
                    Ok(true) => {}
                    Ok(false) => {
                        st.duplicates.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    Err(e) => {
                        eprintln!("messenger ingress: dedup store failed: {e}");
                        continue;
                    }
                }
                let inbound = classify(&raw, keys.as_ref());
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
                dispatcher.dispatch(inbound, &ctx, sink.as_ref()).await;
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
}

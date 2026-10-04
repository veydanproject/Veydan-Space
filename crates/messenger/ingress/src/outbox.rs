// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Persistent outbox. `enqueue` stores an `Outbound` and returns a local id;
//! `pump` sends everything that is due and records the outcome. A relay
//! that accepted counts as published; anything else is retried with
//! backoff 5 / 15 / 30 / 60 / 120 s. The stored request holds the signed
//! event, so every retry republishes the same event id.

use messenger_core::{Clock, MessengerError, Outbound, Result, Transport};
use messenger_store::{outbox as repo, Store};
use std::sync::Arc;

const BACKOFF_SECS: [i64; 5] = [5, 15, 30, 60, 120];
/// A `publishing` row older than this is considered abandoned (crash mid-send).
const STALE_PUBLISHING_SECS: i64 = 60;
/// How many outbox items are published at the same time.
const PUMP_CONCURRENCY: usize = 8;

pub fn retry_delay(attempts: i64) -> i64 {
    let i = attempts.clamp(0, BACKOFF_SECS.len() as i64 - 1) as usize;
    BACKOFF_SECS[i]
}

#[derive(Clone)]
pub struct Outbox {
    store: Store,
    clock: Arc<dyn Clock>,
    /// Wakes the background pump when something was queued.
    kick: Arc<tokio::sync::Notify>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpReport {
    pub published: usize,
    pub failed: usize,
}

impl Outbox {
    pub fn new(store: Store, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock, kick: Arc::new(tokio::sync::Notify::new()) }
    }

    /// Ask the background pump to run now. Callers do not wait for relays:
    /// what they queued is already stored.
    pub fn kick(&self) {
        self.kick.notify_one();
    }

    /// Resolves when `kick` was called (at once if it already was).
    pub async fn kicked(&self) {
        self.kick.notified().await;
    }

    /// Persist and return the local id (a uuid-like random hex string).
    pub async fn enqueue(&self, out: Outbound) -> Result<String> {
        let local_id = new_local_id();
        repo::enqueue(&self.store, &local_id, &out, self.clock.now().secs()).await?;
        Ok(local_id)
    }

    /// Send everything due. Never fails on a single item; the report says
    /// how many went through.
    ///
    /// Items go out concurrently (a message, its self-copy and a control
    /// signal do not wait for each other), a few at a time.
    pub async fn pump(&self, transport: &dyn Transport) -> Result<PumpReport> {
        let now = self.clock.now().secs();
        let due = repo::due(&self.store, now, STALE_PUBLISHING_SECS).await?;
        let mut report = PumpReport::default();
        for batch in due.chunks(PUMP_CONCURRENCY) {
            let results = futures_util::future::join_all(batch.iter().map(|row| self.pump_one(transport, row, now))).await;
            for r in results {
                if r? {
                    report.published += 1;
                } else {
                    report.failed += 1;
                }
            }
        }
        Ok(report)
    }

    /// `Ok(true)` published, `Ok(false)` failed and rescheduled.
    async fn pump_one(&self, transport: &dyn Transport, row: &repo::OutboxRow, now: i64) -> Result<bool> {
        let out = match row.outbound() {
            Ok(o) => o,
            Err(e) => {
                repo::mark_failed(&self.store, &row.local_id, &format!("corrupt: {e}"), i64::MAX / 2).await?;
                return Ok(false);
            }
        };
        repo::mark_publishing(&self.store, &row.local_id).await?;
        let outcome = match transport.send(out).await {
            Ok(ack) if ack.is_delivered() || is_non_publish(row) => Ok(()),
            Ok(ack) => Err(MessengerError::Transport(format!("no relay accepted ({} rejected)", ack.rejected_by.len()))),
            Err(e) => Err(e),
        };
        match outcome {
            Ok(()) => {
                repo::mark_published(&self.store, &row.local_id).await?;
                Ok(true)
            }
            Err(e) => {
                // Backoff grows with the failures so far: 1st failure waits 5 s.
                let next = now + retry_delay(row.attempts);
                repo::mark_failed(&self.store, &row.local_id, &e.to_string(), next).await?;
                Ok(false)
            }
        }
    }

    pub async fn retry_now(&self, local_id: &str) -> Result<()> {
        repo::retry_now(&self.store, local_id).await
    }

    pub async fn pending(&self) -> Result<i64> {
        repo::count_pending(&self.store).await
    }
}

/// Subscribe/unsubscribe/sync requests have no relay acks; the transport
/// returning `Ok` is success.
fn is_non_publish(row: &repo::OutboxRow) -> bool {
    !row.outbound_json.contains("\"op\":\"publish_")
}

fn new_local_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}-{:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use messenger_core::outbound::WireEvent;
    use messenger_core::traits::{RelayStatusSnapshot, SystemClock};
    use messenger_core::{Ack, EventId, RawEvent, RelayUrl};
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Transport whose answer is scripted per call.
    struct Scripted {
        answers: Mutex<Vec<Result<Ack>>>,
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl Transport for Scripted {
        async fn send(&self, _out: Outbound) -> Result<Ack> {
            *self.calls.lock().unwrap() += 1;
            self.answers.lock().unwrap().remove(0)
        }
        fn events(&self) -> mpsc::Receiver<RawEvent> {
            mpsc::channel(1).1
        }
        async fn status(&self) -> RelayStatusSnapshot {
            RelayStatusSnapshot::default()
        }
    }

    struct FixedClock(Mutex<i64>);
    impl Clock for FixedClock {
        fn now(&self) -> messenger_core::Timestamp {
            messenger_core::Timestamp(*self.0.lock().unwrap())
        }
    }

    fn publish() -> Outbound {
        Outbound::PublishOwn {
            event: WireEvent { id: EventId::parse(&"a".repeat(64)).unwrap(), json: serde_json::json!({}) },
        }
    }

    fn accepted() -> Result<Ack> {
        Ok(Ack { accepted_by: vec![RelayUrl::parse("wss://ok.example").unwrap()], rejected_by: vec![] })
    }

    fn rejected() -> Result<Ack> {
        Ok(Ack { accepted_by: vec![], rejected_by: vec![(RelayUrl::parse("wss://no.example").unwrap(), "blocked".into())] })
    }

    #[test]
    fn backoff_table() {
        assert_eq!(retry_delay(0), 5);
        assert_eq!(retry_delay(1), 15);
        assert_eq!(retry_delay(4), 120);
        assert_eq!(retry_delay(99), 120);
    }

    #[tokio::test]
    async fn publish_retries_with_backoff_until_a_relay_accepts() {
        let store = Store::open_in_memory().await.unwrap();
        let clock = Arc::new(FixedClock(Mutex::new(1_000)));
        let outbox = Outbox::new(store.clone(), clock.clone());
        let id = outbox.enqueue(publish()).await.unwrap();

        let t = Scripted {
            answers: Mutex::new(vec![Err(MessengerError::Transport("offline".into())), rejected(), accepted()]),
            calls: Mutex::new(0),
        };

        // 1st pump: transport error → failed, retry in 5 s.
        let r = outbox.pump(&t).await.unwrap();
        assert_eq!(r, PumpReport { published: 0, failed: 1 });
        let row = messenger_store::outbox::get(&store, &id).await.unwrap().unwrap();
        assert_eq!(row.state, "failed");
        assert_eq!(row.next_retry_at, 1_005);
        assert_eq!(row.attempts, 1);

        // Not due yet.
        assert_eq!(outbox.pump(&t).await.unwrap(), PumpReport::default());
        assert_eq!(*t.calls.lock().unwrap(), 1);

        // 2nd pump at +5 s: rejected by every relay → failed, retry in 15 s.
        *clock.0.lock().unwrap() = 1_005;
        assert_eq!(outbox.pump(&t).await.unwrap().failed, 1);
        let row = messenger_store::outbox::get(&store, &id).await.unwrap().unwrap();
        assert_eq!(row.next_retry_at, 1_005 + 15);
        assert!(row.last_error.unwrap().contains("rejected"));

        // 3rd pump: accepted → published; same event id was resent each time.
        *clock.0.lock().unwrap() = 1_020;
        assert_eq!(outbox.pump(&t).await.unwrap().published, 1);
        assert_eq!(messenger_store::outbox::get(&store, &id).await.unwrap().unwrap().state, "published");
        assert_eq!(outbox.pending().await.unwrap(), 0);
        assert_eq!(*t.calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn subscriptions_succeed_without_relay_acks() {
        let store = Store::open_in_memory().await.unwrap();
        let outbox = Outbox::new(store, Arc::new(SystemClock));
        outbox.enqueue(Outbound::Unsubscribe { id: messenger_core::SubId("s".into()) }).await.unwrap();
        let t = Scripted { answers: Mutex::new(vec![Ok(Ack { accepted_by: vec![], rejected_by: vec![] })]), calls: Mutex::new(0) };
        assert_eq!(outbox.pump(&t).await.unwrap().published, 1);
    }
}

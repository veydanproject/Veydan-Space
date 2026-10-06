// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Persistent outbox. `enqueue` stores an `Outbound` and returns a local id;
//! `pump` sends everything that is due and records the outcome. A relay
//! that accepted counts as published; anything else is retried with
//! backoff 5 / 15 / 30 / 60 / 120 s. The stored request holds the signed
//! event, so every retry republishes the same event id.
//!
//! A message the user wrote (`enqueue_message`) is tried for an hour, then
//! given up; a relay that refuses it outright gives it up sooner. A network
//! that is down is no refusal: it only runs the hour out. A receipt
//! (`enqueue_for`) is given up the same way after a time of its own.
//! Everything else (`enqueue`) is tried until it leaves.

use messenger_core::{Ack, Clock, Outbound, Result, Transport};
use messenger_store::{outbox as repo, Store};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

const BACKOFF_SECS: [i64; 5] = [5, 15, 30, 60, 120];
/// A `publishing` row older than this is considered abandoned (crash mid-send).
const STALE_PUBLISHING_SECS: i64 = 60;
/// How many outbox items are published at the same time.
const PUMP_CONCURRENCY: usize = 8;
/// How long a message the user wrote is tried.
pub const MESSAGE_TTL_SECS: i64 = 60 * 60;
/// Outright refusals by relays after which a message is given up.
pub const REFUSALS_BEFORE_FAILED: i64 = 3;
/// The answers of NIP-01 that mean "no, and asking again will not help".
/// Rate limits, auth and anything unknown are not among them.
const REFUSALS: [&str; 5] = ["blocked:", "invalid:", "restricted:", "pow:", "mute:"];

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
    /// Nothing is given up for its deadline before this time: the app was
    /// off or the device slept, and the relays are only coming back.
    hold_until: Arc<AtomicI64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PumpReport {
    pub published: usize,
    pub failed: usize,
    /// Given up: past the deadline, or refused.
    pub abandoned: usize,
}

/// What one try came to.
enum Outcome {
    Published,
    /// A relay said no; asking again will not help.
    Refused(String),
    /// No relay was reached, or none answered: the network.
    Unreached(String),
}

impl Outbox {
    pub fn new(store: Store, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock, kick: Arc::new(tokio::sync::Notify::new()), hold_until: Arc::new(AtomicI64::new(0)) }
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

    /// Nothing is given up for its deadline before `until`: called when the
    /// session starts and when the device wakes, so that messages that ran
    /// out meanwhile get a try once the relays are back.
    pub fn hold_expiry(&self, until: i64) {
        self.hold_until.fetch_max(until, Ordering::Relaxed);
    }

    /// Persist and return the local id. Tried until it leaves.
    pub async fn enqueue(&self, out: Outbound) -> Result<String> {
        self.put(out, None).await
    }

    /// A message the user wrote: tried for `MESSAGE_TTL_SECS`, then given up.
    pub async fn enqueue_message(&self, out: Outbound) -> Result<String> {
        self.put(out, Some(MESSAGE_TTL_SECS)).await
    }

    /// Tried for `ttl_secs`, then given up, as a message is: for what is
    /// worth nothing once late (a receipt).
    pub async fn enqueue_for(&self, out: Outbound, ttl_secs: i64) -> Result<String> {
        self.put(out, Some(ttl_secs)).await
    }

    async fn put(&self, out: Outbound, ttl: Option<i64>) -> Result<String> {
        let local_id = new_local_id();
        let now = self.clock.now().secs();
        repo::enqueue(&self.store, &local_id, &out, now, ttl.map(|t| now + t)).await?;
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
                match r? {
                    Some(true) => report.published += 1,
                    Some(false) => report.abandoned += 1,
                    None => report.failed += 1,
                }
            }
        }
        Ok(report)
    }

    /// `Some(true)` published, `Some(false)` given up, `None` failed and
    /// rescheduled.
    async fn pump_one(&self, transport: &dyn Transport, row: &repo::OutboxRow, now: i64) -> Result<Option<bool>> {
        let out = match row.outbound() {
            Ok(o) => o,
            Err(e) => {
                repo::mark_abandoned(&self.store, &row.local_id, &format!("corrupt: {e}")).await?;
                return Ok(Some(false));
            }
        };
        repo::mark_publishing(&self.store, &row.local_id).await?;
        let outcome = match transport.send(out).await {
            Ok(ack) if ack.is_delivered() || is_non_publish(row) => Outcome::Published,
            Ok(ack) => verdict(&ack),
            Err(e) => Outcome::Unreached(e.to_string()),
        };
        // The deadline is looked at after a try: one past it gets its last.
        let expired = row.expires_at.is_some_and(|at| now >= at.max(self.hold_until.load(Ordering::Relaxed)));
        let (why, refused) = match outcome {
            Outcome::Published => {
                repo::mark_published(&self.store, &row.local_id).await?;
                return Ok(Some(true));
            }
            Outcome::Refused(why) => (why, true),
            Outcome::Unreached(why) => (why, false),
        };
        if row.expires_at.is_some() {
            let given_up = if refused && row.rejections + 1 >= REFUSALS_BEFORE_FAILED {
                Some(why.clone())
            } else if expired {
                Some(format!("expired: {why}"))
            } else {
                None
            };
            if let Some(reason) = given_up {
                repo::mark_abandoned(&self.store, &row.local_id, &reason).await?;
                return Ok(Some(false));
            }
        }
        // Backoff grows with the tries so far: the 1st failure waits 5 s.
        let next = now + retry_delay(row.attempts);
        repo::mark_failed(&self.store, &row.local_id, &why, next, refused).await?;
        Ok(None)
    }

    /// The user asked: due now, counters reset, a message gets a fresh hour.
    pub async fn retry_now(&self, local_id: &str) -> Result<()> {
        repo::retry_now(&self.store, local_id, self.clock.now().secs(), MESSAGE_TTL_SECS).await
    }

    pub async fn pending(&self) -> Result<i64> {
        repo::count_pending(&self.store).await
    }
}

/// What the relays' answers say when none accepted.
fn verdict(ack: &Ack) -> Outcome {
    match ack.rejected_by.iter().find(|(_, why)| is_refusal(why)) {
        Some((url, why)) => Outcome::Refused(format!("rejected by {}: {why}", url.as_str())),
        None => Outcome::Unreached(format!("no relay accepted ({} failed)", ack.rejected_by.len())),
    }
}

fn is_refusal(why: &str) -> bool {
    let why = why.to_ascii_lowercase();
    REFUSALS.iter().any(|p| why.contains(p))
}

/// Subscribe/unsubscribe/sync requests have no relay acks; the transport
/// returning `Ok` is success.
fn is_non_publish(row: &repo::OutboxRow) -> bool {
    !row.outbound_json.contains("\"op\":\"publish_")
}

fn new_local_id() -> String {
    use std::sync::atomic::AtomicU64;
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
    use messenger_core::{EventId, MessengerError, RawEvent, RelayUrl};
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    /// Transport whose answer is scripted per call; the last answer repeats.
    struct Scripted {
        answers: Mutex<Vec<Result<Ack>>>,
        calls: Mutex<usize>,
    }

    impl Scripted {
        fn new(answers: Vec<Result<Ack>>) -> Self {
            Self { answers: Mutex::new(answers), calls: Mutex::new(0) }
        }
    }

    #[async_trait]
    impl Transport for Scripted {
        async fn send(&self, _out: Outbound) -> Result<Ack> {
            *self.calls.lock().unwrap() += 1;
            let mut answers = self.answers.lock().unwrap();
            if answers.len() > 1 {
                answers.remove(0)
            } else {
                match &answers[0] {
                    Ok(a) => Ok(a.clone()),
                    Err(e) => Err(MessengerError::Transport(e.to_string())),
                }
            }
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

    fn setup() -> (Arc<FixedClock>, impl std::future::Future<Output = (Store, Outbox)>) {
        let clock = Arc::new(FixedClock(Mutex::new(1_000)));
        let c = clock.clone();
        (clock, async move {
            let store = Store::open_in_memory().await.unwrap();
            let outbox = Outbox::new(store.clone(), c);
            (store, outbox)
        })
    }

    fn publish() -> Outbound {
        Outbound::PublishOwn {
            event: WireEvent { id: EventId::parse(&"a".repeat(64)).unwrap(), json: serde_json::json!({}) },
        }
    }

    fn accepted() -> Result<Ack> {
        Ok(Ack { accepted_by: vec![RelayUrl::parse("wss://ok.example").unwrap()], rejected_by: vec![] })
    }

    fn answered(why: &str) -> Result<Ack> {
        Ok(Ack { accepted_by: vec![], rejected_by: vec![(RelayUrl::parse("wss://no.example").unwrap(), why.into())] })
    }

    fn offline() -> Result<Ack> {
        Err(MessengerError::Transport("no relay connected".into()))
    }

    async fn row(store: &Store, id: &str) -> repo::OutboxRow {
        repo::get(store, id).await.unwrap().unwrap()
    }

    /// Pumps at every second from `from` to `to`, as long as it takes.
    async fn pump_until(outbox: &Outbox, t: &Scripted, clock: &FixedClock, to: i64) {
        loop {
            outbox.pump(t).await.unwrap();
            let now = *clock.0.lock().unwrap();
            if now >= to {
                break;
            }
            *clock.0.lock().unwrap() = (now + 5).min(to);
        }
    }

    #[test]
    fn backoff_table() {
        assert_eq!(retry_delay(0), 5);
        assert_eq!(retry_delay(1), 15);
        assert_eq!(retry_delay(4), 120);
        assert_eq!(retry_delay(99), 120);
    }

    #[test]
    fn refusals_are_told_from_the_network() {
        for why in ["blocked: spam", "rejected by wss://x: invalid: bad signature", "restricted: not allowed", "pow: difficulty 20"] {
            assert!(is_refusal(why), "{why}");
        }
        for why in ["rate-limited: slow down", "auth-required: who are you", "timeout", "not connected", "premature exit", ""] {
            assert!(!is_refusal(why), "{why}");
        }
    }

    #[tokio::test]
    async fn publish_retries_with_backoff_until_a_relay_accepts() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue(publish()).await.unwrap();
        let t = Scripted::new(vec![offline(), answered("blocked: no"), accepted()]);

        // 1st pump: transport error → failed, retry in 5 s.
        assert_eq!(outbox.pump(&t).await.unwrap(), PumpReport { failed: 1, ..Default::default() });
        let r = row(&store, &id).await;
        assert_eq!((r.state.as_str(), r.next_retry_at, r.attempts, r.rejections), ("failed", 1_005, 1, 0));

        // Not due yet.
        assert_eq!(outbox.pump(&t).await.unwrap(), PumpReport::default());
        assert_eq!(*t.calls.lock().unwrap(), 1);

        // 2nd pump at +5 s: refused → failed, retry in 15 s; a row without a
        // deadline is never given up.
        *clock.0.lock().unwrap() = 1_005;
        assert_eq!(outbox.pump(&t).await.unwrap().failed, 1);
        let r = row(&store, &id).await;
        assert_eq!((r.next_retry_at, r.rejections), (1_005 + 15, 1));
        assert!(r.last_error.unwrap().contains("blocked"));

        // 3rd pump: accepted → published; same event id was resent each time.
        *clock.0.lock().unwrap() = 1_020;
        assert_eq!(outbox.pump(&t).await.unwrap().published, 1);
        assert_eq!(row(&store, &id).await.state, "published");
        assert_eq!(outbox.pending().await.unwrap(), 0);
        assert_eq!(*t.calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn a_message_waits_out_its_hour_on_a_network_that_is_down() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_message(publish()).await.unwrap();
        let t = Scripted::new(vec![offline()]);

        pump_until(&outbox, &t, &clock, 1_000 + MESSAGE_TTL_SECS - 1).await;
        let r = row(&store, &id).await;
        assert_eq!((r.state.as_str(), r.rejections), ("failed", 0), "still tried: {r:?}");

        *clock.0.lock().unwrap() = 1_000 + MESSAGE_TTL_SECS + 120;
        assert_eq!(outbox.pump(&t).await.unwrap().abandoned, 1);
        let r = row(&store, &id).await;
        assert_eq!(r.state, repo::STATE_ABANDONED);
        assert!(r.last_error.unwrap().starts_with("expired: "));
        assert_eq!(outbox.pending().await.unwrap(), 0);
        // Given up is not tried again.
        *clock.0.lock().unwrap() += 10_000;
        let calls = *t.calls.lock().unwrap();
        outbox.pump(&t).await.unwrap();
        assert_eq!(*t.calls.lock().unwrap(), calls);
    }

    #[tokio::test]
    async fn refusals_give_a_message_up_before_its_hour() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_message(publish()).await.unwrap();
        let t = Scripted::new(vec![answered("blocked: spam")]);
        for _ in 0..REFUSALS_BEFORE_FAILED - 1 {
            assert_eq!(outbox.pump(&t).await.unwrap().failed, 1);
            *clock.0.lock().unwrap() += 200;
        }
        assert_eq!(outbox.pump(&t).await.unwrap().abandoned, 1);
        let r = row(&store, &id).await;
        assert_eq!(r.state, repo::STATE_ABANDONED);
        assert!(r.last_error.unwrap().contains("blocked: spam"));
        assert!(*clock.0.lock().unwrap() < 1_000 + MESSAGE_TTL_SECS);
    }

    #[tokio::test]
    async fn rate_limits_and_silence_are_not_refusals() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_message(publish()).await.unwrap();
        let t = Scripted::new(vec![
            answered("rate-limited: slow down"),
            Ok(Ack { accepted_by: vec![], rejected_by: vec![] }),
            answered("timeout"),
            answered("rate-limited: slow down"),
        ]);
        for _ in 0..4 {
            outbox.pump(&t).await.unwrap();
            *clock.0.lock().unwrap() += 200;
        }
        let r = row(&store, &id).await;
        assert_eq!((r.state.as_str(), r.rejections), ("failed", 0));
    }

    #[tokio::test]
    async fn what_is_not_a_message_is_never_given_up() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue(publish()).await.unwrap();
        let t = Scripted::new(vec![answered("blocked: no"), answered("blocked: no"), answered("blocked: no"), offline()]);
        pump_until(&outbox, &t, &clock, 1_000 + 3 * MESSAGE_TTL_SECS).await;
        let r = row(&store, &id).await;
        assert_eq!(r.state, "failed", "a control event or a deletion is tried until it leaves");
        assert_eq!(r.rejections, 3);
    }

    #[tokio::test]
    async fn a_wake_holds_the_deadline_back() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_message(publish()).await.unwrap();
        let t = Scripted::new(vec![offline()]);
        // The phone slept through the hour; it wakes, relays come back slowly.
        let woke = 1_000 + 5 * MESSAGE_TTL_SECS;
        *clock.0.lock().unwrap() = woke;
        outbox.hold_expiry(woke + 60);
        assert_eq!(outbox.pump(&t).await.unwrap().failed, 1, "a try, not given up");
        assert_eq!(row(&store, &id).await.state, "failed");
        *clock.0.lock().unwrap() = woke + 60;
        assert_eq!(outbox.pump(&t).await.unwrap().abandoned, 1);
    }

    #[tokio::test]
    async fn a_retry_gives_a_fresh_hour() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_message(publish()).await.unwrap();
        let t = Scripted::new(vec![offline()]);
        *clock.0.lock().unwrap() = 1_000 + MESSAGE_TTL_SECS;
        assert_eq!(outbox.pump(&t).await.unwrap().abandoned, 1);

        let later = 1_000 + 2 * MESSAGE_TTL_SECS;
        *clock.0.lock().unwrap() = later;
        outbox.retry_now(&id).await.unwrap();
        let r = row(&store, &id).await;
        assert_eq!((r.state.as_str(), r.created_at, r.expires_at), ("queued", later, Some(later + MESSAGE_TTL_SECS)));
        assert_eq!(outbox.pump(&t).await.unwrap().failed, 1, "tried again, within its new hour");
    }

    #[tokio::test]
    async fn what_is_late_for_nothing_is_given_up_after_its_own_time() {
        let (clock, made) = setup();
        let (store, outbox) = made.await;
        let id = outbox.enqueue_for(publish(), 86_400).await.unwrap();
        let r = row(&store, &id).await;
        assert_eq!((r.state.as_str(), r.created_at, r.expires_at), ("queued", 1_000, Some(1_000 + 86_400)));

        let t = Scripted::new(vec![offline()]);
        *clock.0.lock().unwrap() = 1_000 + MESSAGE_TTL_SECS + 120;
        assert_eq!(outbox.pump(&t).await.unwrap().failed, 1, "a message's hour is not its time");
        *clock.0.lock().unwrap() = 1_000 + 86_400;
        assert_eq!(outbox.pump(&t).await.unwrap().abandoned, 1);
    }

    #[tokio::test]
    async fn subscriptions_succeed_without_relay_acks() {
        let store = Store::open_in_memory().await.unwrap();
        let outbox = Outbox::new(store, Arc::new(SystemClock));
        outbox.enqueue(Outbound::Unsubscribe { id: messenger_core::SubId("s".into()) }).await.unwrap();
        let t = Scripted::new(vec![Ok(Ack { accepted_by: vec![], rejected_by: vec![] })]);
        assert_eq!(outbox.pump(&t).await.unwrap().published, 1);
    }
}

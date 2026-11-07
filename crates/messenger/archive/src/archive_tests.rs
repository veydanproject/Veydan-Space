// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The archive over a real (in-memory) messenger store: what is kept, what
//! is due, how files are built and merged, the queue and the replay.

use crate::segment;
use crate::*;
use messenger_core::inbound::Envelope;
use messenger_core::{Clock, DmInbound, EventId, EventSource, PubKey, RawEvent, Timestamp};
use messenger_store::identity::{self, IdentityRow};
use messenger_store::Store;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 2026-11-10 00:00 UTC.
const NOV_10: i64 = 1_794_268_800;
/// 2026-11-01 00:00 UTC.
const NOV_1: i64 = 1_793_491_200;

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

fn owner() -> PubKey {
    PubKey::parse(&hex(8)).unwrap()
}

/// An archive whose store belongs to `owner()`.
async fn archive() -> (Archive, Arc<TestClock>) {
    let store = Store::open_in_memory().await.unwrap();
    let row = IdentityRow { id: "primary".into(), npub: "npub1test".into(), pubkey_hex: hex(8), nsec_ref: "nsec".into(), created_at: 1 };
    identity::insert(&store, &row).await.unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(NOV_10)));
    (Archive::new(store, clock.clone()), clock)
}

fn hex(n: u64) -> String {
    format!("{n:064x}")
}

/// A personal message `n` of the home chat, at `at`.
fn dm(n: u64, at: i64) -> ArchivedEvent {
    let inbound = DmInbound {
        envelope: Envelope {
            wire_id: EventId::parse(&hex(n)).unwrap(),
            source: EventSource::Archive,
            wire_created_at: Timestamp(at - 3600),
        },
        rumor_id: EventId::parse(&hex(1000 + n)).unwrap(),
        sender: PubKey::parse(&hex(7)).unwrap(),
        recipients: vec![PubKey::parse(&hex(8)).unwrap()],
        created_at: Timestamp(at),
        content: format!(r#"{{"v":1,"t":"text","text":"message {n}"}}"#),
        reply_to: None,
        rumor_kind: 14,
        srv: None,
    };
    ArchivedEvent::dm(inbound, Timestamp(at + 1))
}

/// A note of my own devices (`KIND_OWN_RUMOR`) with the given content.
fn own_note(n: u64, at: i64, content: &str) -> ArchivedEvent {
    let mut event = dm(n, at);
    let Body::Dm { inbound } = &mut event.body else { unreachable!() };
    inbound.rumor_kind = messenger_core::envelope::KIND_OWN_RUMOR;
    inbound.content = content.to_string();
    event
}

fn group(n: u64, at: i64, id: &str) -> ArchivedEvent {
    let raw = RawEvent {
        id: EventId::parse(&hex(n)).unwrap(),
        kind: 9,
        pubkey: PubKey::parse(&hex(7)).unwrap(),
        created_at: Timestamp(at),
        json: serde_json::json!({ "content": "sealed", "kind": 9 }),
        source: EventSource::Archive,
    };
    ArchivedEvent::wire(raw, ArchiveScope::Group { id: id.into() }, Timestamp(at))
}

fn seg(scope: ArchiveScope, year: i32, month: u8) -> SegmentId {
    SegmentId { owner: owner(), scope, month: Month { year, month } }
}

fn nov(scope: ArchiveScope) -> SegmentId {
    seg(scope, 2026, 11)
}

fn ids(bytes: &[u8]) -> Vec<String> {
    segment::read(bytes).unwrap().events.iter().map(|e| e.id.as_hex().to_string()).collect()
}

fn meta(hash: &str) -> SegmentMeta {
    SegmentMeta { owner: hex(8), scope: String::new(), month: String::new(), count: 1, first_at: 0, last_at: 0, size: 100, hash: hash.into() }
}

/// The replayer of the test: what it was handed; `NotLoggedIn` from the
/// call `logged_out_at` on, when set; a group's events refused while
/// `group_unknown`.
#[derive(Default)]
struct FakeReplayer {
    seen: Mutex<Vec<String>>,
    calls: AtomicUsize,
    logged_out_at: Option<usize>,
    group_unknown: bool,
}

#[async_trait::async_trait]
impl Replayer for FakeReplayer {
    async fn replay(&self, event: ArchivedEvent) -> messenger_core::Result<Replayed> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.logged_out_at.is_some_and(|at| call >= at) {
            return Err(MessengerError::NotLoggedIn);
        }
        if self.group_unknown && matches!(event.scope, ArchiveScope::Group { .. }) {
            return Err(MessengerError::Invalid(REFUSED_GROUP_UNKNOWN.into()));
        }
        self.seen.lock().unwrap().push(event.id.as_hex().to_string());
        Ok(if event.kind == 9 { Replayed::Ignored } else { Replayed::Applied })
    }
}

impl FakeReplayer {
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn record_is_idempotent_and_splits_months_at_the_boundary() {
    let (a, _) = archive().await;
    a.record(dm(1, NOV_1 - 1)).await.unwrap();
    a.record(dm(2, NOV_1)).await.unwrap();
    a.record(dm(2, NOV_1)).await.unwrap();
    let due = a.due(Timestamp(NOV_10), false).await.unwrap();
    let months: Vec<String> = due.iter().map(SegmentId::entity_id).collect();
    let o = hex(8);
    assert_eq!(months, [format!("{o}/dm/2026-11"), format!("{o}/dm/2026-10")]);
    let built = a.build(&nov(ArchiveScope::Dm)).await.unwrap();
    assert_eq!(built.meta.count, 1, "the duplicate is kept once");
    assert_eq!(built.meta.owner, o);
    assert_eq!(segment::read(&built.bytes).unwrap().owner, owner());
}

/// Without an identity nothing is due: the archive has no owner to name.
#[tokio::test]
async fn without_an_identity_nothing_is_due() {
    let store = Store::open_in_memory().await.unwrap();
    let a = Archive::new(store, Arc::new(TestClock(AtomicI64::new(NOV_10))));
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    assert_eq!(a.owner().await.unwrap(), None);
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_open_month_waits_for_the_interval_and_a_closed_one_does_not() {
    let (a, clock) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    a.record(dm(1, NOV_1 + 10)).await.unwrap();
    assert_eq!(a.due(Timestamp(NOV_10), false).await.unwrap(), vec![seg.clone()], "never written: at once");
    let built = a.build(&seg).await.unwrap();
    a.written(&seg, &built.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty(), "clean");

    a.record(dm(2, NOV_1 + 20)).await.unwrap();
    let soon = Timestamp(NOV_10 + FLUSH_INTERVAL_SECS - 1);
    assert!(a.due(soon, false).await.unwrap().is_empty());
    assert_eq!(a.due(soon, true).await.unwrap(), vec![seg.clone()], "exit or background");
    assert_eq!(a.due(Timestamp(NOV_10 + FLUSH_INTERVAL_SECS), false).await.unwrap(), vec![seg.clone()]);

    // A late event of a closed month: written at once.
    let sep = self::seg(ArchiveScope::Dm, 2026, 9);
    a.record(dm(3, 1_789_000_000)).await.unwrap(); // 2026-09-10
    let built = a.build(&sep).await.unwrap();
    a.written(&sep, &built.meta).await.unwrap();
    a.record(dm(4, 1_789_000_100)).await.unwrap();
    assert!(a.due(soon, false).await.unwrap().contains(&sep));

    // Events that came between the build and the upload keep it dirty.
    clock.0.store(NOV_10 + 2000, Ordering::SeqCst);
    let built = a.build(&seg).await.unwrap();
    a.record(dm(5, NOV_1 + 30)).await.unwrap();
    a.written(&seg, &built.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10 + 2000), true).await.unwrap().contains(&seg));

    // A merge that fails is deferred: not due again before the interval.
    a.defer(&seg).await.unwrap();
    assert!(!a.due(Timestamp(NOV_10 + 2000), false).await.unwrap().contains(&seg));
    assert!(a.due(Timestamp(NOV_10 + 2000 + FLUSH_INTERVAL_SECS), false).await.unwrap().contains(&seg));
}

/// An event that came and one that compaction dropped since the build
/// cancel out in a count, not in the file: the segment stays dirty.
#[tokio::test]
async fn written_compares_the_file_not_the_count() {
    let (a, _) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    a.record(dm(2, NOV_1 + 2)).await.unwrap();
    let built = a.build(&seg).await.unwrap();
    // Between the build and the upload: message 1 deleted, message 3 came.
    sqlx::query(
        "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, envelope_json, sender_pubkey, created_at, received_at, deleted_at)
         VALUES (?, 'dm:bb', 'in', 'received', 'text', '{}', 'bb', 1, 1, 9)",
    )
    .bind(hex(1001))
    .execute(a.store.pool())
    .await
    .unwrap();
    a.record(dm(3, NOV_1 + 3)).await.unwrap();
    a.written(&seg, &built.meta).await.unwrap();
    assert_eq!(a.due(Timestamp(NOV_10), true).await.unwrap(), vec![seg.clone()], "two events, as before, but not the same two");
    let again = a.build(&seg).await.unwrap();
    assert_eq!(ids(&again.bytes), [hex(2), hex(3)]);
    a.written(&seg, &again.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_storage_gets_every_segment_again_as_it_was() {
    let (a, _) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    let sep = self::seg(ArchiveScope::Dm, 2026, 9);
    a.record(dm(1, NOV_1 + 10)).await.unwrap();
    a.record(dm(2, 1_789_000_000)).await.unwrap(); // 2026-09-10
    let first = a.build(&seg).await.unwrap();
    a.written(&seg, &first.meta).await.unwrap();
    let old = a.build(&sep).await.unwrap();
    a.written(&sep, &old.meta).await.unwrap();
    // A segment compacted to nothing is written as a tombstone: it holds
    // no event and comes back for no storage.
    let empty = nov(ArchiveScope::Group { id: "g1".into() });
    let none = a.build(&empty).await.unwrap();
    a.written(&empty, &none.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty(), "clean");

    a.forget_written().await.unwrap();
    let mut due: Vec<String> = a.due(Timestamp(NOV_10), false).await.unwrap().iter().map(|s| format!("{}/{}", s.scope, s.month)).collect();
    due.sort();
    assert_eq!(due, ["dm/2026-09", "dm/2026-11"], "every segment with events, at once, the empty one not");
    assert_eq!(a.build(&seg).await.unwrap().bytes, first.bytes, "the files come out as before");
    assert_eq!(a.build(&sep).await.unwrap().bytes, old.bytes);
    // Written again: clean again.
    a.written(&seg, &first.meta).await.unwrap();
    a.written(&sep, &old.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty());
}

/// The mark of a scope is its latest event, but an event dated far ahead
/// (the sender's clock) counts no further than when it was received plus
/// the skew: a catch-up `since` 2099 would never bring anything.
#[tokio::test]
async fn the_marks_of_the_runtime_are_the_high_water_of_the_archive() {
    let (a, _) = archive().await;
    let g = ArchiveScope::Group { id: "g1".into() };
    assert_eq!(ArchiveMarks::high_water(&a, &g).await, None);
    a.record(group(1, NOV_1 + 5, "g1")).await.unwrap();
    assert_eq!(ArchiveMarks::high_water(&a, &g).await, Some(Timestamp(NOV_1 + 5)));
    // Every archive is the hooks the runtime takes.
    let hooks: Arc<dyn ArchiveHooks> = Arc::new(a.clone());
    assert_eq!(hooks.high_water(&g).await, Some(Timestamp(NOV_1 + 5)));
    hooks.record(group(2, NOV_1 + 9, "g1")).await.unwrap();
    assert_eq!(hooks.high_water(&g).await, Some(Timestamp(NOV_1 + 9)));

    // 2099-01-01, received now.
    let mut future = dm(3, 4_070_908_800);
    future.received_at = Timestamp(NOV_10);
    hooks.record(future).await.unwrap();
    assert_eq!(hooks.high_water(&ArchiveScope::Dm).await, Some(Timestamp(NOV_10 + AT_SKEW_SECS)));
}

#[tokio::test]
async fn a_build_reads_back_and_a_grown_month_keeps_its_prefix() {
    let (a, _) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    a.record(dm(2, NOV_1 + 200)).await.unwrap();
    a.record(dm(1, NOV_1 + 100)).await.unwrap();
    let first = a.build(&seg).await.unwrap();
    assert_eq!(ids(&first.bytes), [hex(1), hex(2)], "a first write by (at, id)");
    assert_eq!(first.meta.count, 2);
    assert_eq!((first.meta.first_at, first.meta.last_at), (NOV_1 + 100, NOV_1 + 200));
    assert_eq!(first.meta.size, first.bytes.len() as u64);
    assert_eq!(first.meta.hash, segment::hash(&first.bytes));
    assert_eq!(first.meta.scope, "dm");
    assert_eq!(first.meta.month, "2026-11");
    a.written(&seg, &first.meta).await.unwrap();

    // A late event older than both: appended, not sorted in.
    a.record(dm(3, NOV_1 + 50)).await.unwrap();
    let second = a.build(&seg).await.unwrap();
    assert!(second.bytes.starts_with(&first.bytes));
    assert_eq!(ids(&second.bytes), [hex(1), hex(2), hex(3)]);
    assert_eq!(a.build(&seg).await.unwrap().bytes, second.bytes, "a build is deterministic");
}

#[tokio::test]
async fn merges_replay_what_is_missing_and_upload_only_a_bigger_union() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    a.record(dm(2, NOV_1 + 2)).await.unwrap();
    b.record(dm(1, NOV_1 + 1)).await.unwrap();
    let file_a = a.build(&seg).await.unwrap();
    let file_b = b.build(&seg).await.unwrap();

    // The storage holds more than B: B replays the rest, uploads nothing
    // and is clean.
    let r = FakeReplayer::default();
    let m = b.merge(&seg, &file_a.bytes, &r).await.unwrap();
    assert_eq!(r.seen(), [hex(2)]);
    assert_eq!(m.replay.applied, 1);
    assert!(m.upload.is_none());
    assert!(b.due(Timestamp(NOV_10), true).await.unwrap().is_empty());
    assert_eq!(b.build(&seg).await.unwrap().bytes, file_a.bytes, "the union is the storage's file");
    assert!(b.holds(&seg, &file_a.meta.hash).await.unwrap());

    // A holds more than the storage's file of B: nothing to replay, the
    // upload is that file and A's event after it.
    let r = FakeReplayer::default();
    let m = a.merge(&seg, &file_b.bytes, &r).await.unwrap();
    assert!(r.seen().is_empty());
    let up = m.upload.unwrap();
    assert!(up.bytes.starts_with(&file_b.bytes));
    assert_eq!(ids(&up.bytes), [hex(1), hex(2)]);

    // Disjoint: both.
    let (c, _) = archive().await;
    c.record(dm(3, NOV_1 + 3)).await.unwrap();
    let r = FakeReplayer::default();
    let m = c.merge(&seg, &file_a.bytes, &r).await.unwrap();
    assert_eq!(r.seen(), [hex(1), hex(2)]);
    let up = m.upload.unwrap();
    assert!(up.bytes.starts_with(&file_a.bytes), "the storage's order first");
    assert_eq!(ids(&up.bytes), [hex(1), hex(2), hex(3)]);

    // A file of another segment, or of another owner, is refused.
    let oct = self::seg(ArchiveScope::Dm, 2026, 10);
    assert!(c.merge(&oct, &file_a.bytes, &r).await.is_err());
    let other = SegmentId { owner: PubKey::parse(&hex(9)).unwrap(), ..seg.clone() };
    assert!(c.merge(&other, &file_a.bytes, &r).await.is_err());
}

/// Two devices hold a month, each wrote its own file, and the other's
/// file comes by the restore (not by a merge: the segment was clean).
/// Whoever lost the race finds the storage lacking its event and writes
/// the union with its next pass; the other takes the union and is clean;
/// a third pass of either uploads nothing.
#[tokio::test]
async fn the_loser_of_a_write_race_writes_the_union_and_the_vault_settles() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    b.record(dm(2, NOV_1 + 2)).await.unwrap();
    let file_a = a.build(&seg).await.unwrap();
    a.written(&seg, &file_a.meta).await.unwrap();
    let file_b = b.build(&seg).await.unwrap();
    b.written(&seg, &file_b.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty());

    // B's op won: A's restore reads B's file.
    let r = FakeReplayer::default();
    assert_eq!(a.enqueue(&seg.entity_id(), &file_b.meta).await.unwrap(), Enqueued::Queued);
    let item = a.dequeue(None).await.unwrap().unwrap();
    a.replay_segment(&item, &file_b.bytes, &r).await.unwrap();
    a.done(&item.entity_id, &item.hash).await.unwrap();
    assert_eq!(r.seen(), [hex(2)]);
    assert_eq!(a.due(Timestamp(NOV_10), true).await.unwrap(), vec![seg.clone()], "the storage lacks A's event: due again");
    // A's next pass: the storage's version is held, the union is built
    // from what is held, without the file.
    let m = a.merge_held(&seg, &file_b.meta.hash, &r).await.unwrap().expect("held");
    let union = m.upload.expect("the union goes up");
    assert_eq!(ids(&union.bytes), [hex(2), hex(1)], "the storage's order first");
    a.written(&seg, &union.meta).await.unwrap();
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().is_empty());

    // B takes the union: nothing missing, nothing to write.
    let r = FakeReplayer::default();
    assert_eq!(b.enqueue(&seg.entity_id(), &union.meta).await.unwrap(), Enqueued::Queued);
    let item = b.dequeue(None).await.unwrap().unwrap();
    b.replay_segment(&item, &union.bytes, &r).await.unwrap();
    b.done(&item.entity_id, &item.hash).await.unwrap();
    assert_eq!(r.seen(), [hex(1)]);
    assert!(b.due(Timestamp(NOV_10), true).await.unwrap().is_empty(), "the storage holds all of B");
    assert!(b.merge_held(&seg, &union.meta.hash, &r).await.unwrap().unwrap().upload.is_none());
    assert!(a.merge_held(&seg, &union.meta.hash, &r).await.unwrap().unwrap().upload.is_none(), "a fixed point");
    assert_eq!(b.enqueue(&seg.entity_id(), &union.meta).await.unwrap(), Enqueued::Held { unreplayed: false });
}

#[tokio::test]
async fn a_cut_file_is_merged_and_rewritten_whole() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    for n in 1..=3 {
        a.record(dm(n, NOV_1 + n as i64)).await.unwrap();
    }
    let file = a.build(&seg).await.unwrap().bytes;
    let r = FakeReplayer::default();
    let m = b.merge(&seg, &file[..file.len() - 5], &r).await.unwrap();
    assert!(m.replay.truncated);
    assert_eq!(m.replay.applied, 2);
    assert_eq!(m.upload.map(|u| ids(&u.bytes)), Some(vec![hex(1), hex(2)]), "a whole file again");
}

#[tokio::test]
async fn the_queue_goes_meta_first_then_newest_months_and_the_open_chat_first() {
    let (a, _) = archive().await;
    let o = hex(8);
    let id = |s: &str| format!("{o}/{s}");
    for s in ["dm/2026-10", "dm/2026-11", "group:g1/2026-11", "meta/2026-01", "group:g2/2026-09"] {
        assert_eq!(a.enqueue(&id(s), &meta("h1")).await.unwrap(), Enqueued::Queued, "{s}");
    }
    assert_eq!(a.enqueue(&id("dm/2026-10"), &meta("h1")).await.unwrap(), Enqueued::Known, "the same version again");
    assert_eq!(a.enqueue(&id("dm/2026-10"), &meta("h2")).await.unwrap(), Enqueued::Queued, "a newer version");
    assert!(a.enqueue("nope", &meta("h1")).await.is_err());
    assert!(a.enqueue("dm/2026-10", &meta("h1")).await.is_err(), "no owner");

    let g2 = ArchiveScope::Group { id: "g2".into() };
    assert_eq!(a.dequeue(Some(&g2)).await.unwrap().unwrap().entity_id, id("group:g2/2026-09"), "the open chat first");
    let mut order = Vec::new();
    while let Some(item) = a.dequeue(None).await.unwrap() {
        order.push(item.entity_id.clone());
        a.done(&item.entity_id, &item.hash).await.unwrap();
    }
    assert_eq!(order, [id("meta/2026-01"), id("dm/2026-11"), id("group:g1/2026-11"), id("dm/2026-10"), id("group:g2/2026-09")]);
    let p = a.progress().await.unwrap();
    assert_eq!((p.done, p.total, p.bytes_done, p.bytes_total), (5, 5, 500, 500));

    // A version done is not queued again; a failed one comes back by
    // retry; `done` of another version leaves the row pending.
    assert_eq!(a.enqueue(&id("dm/2026-11"), &meta("h1")).await.unwrap(), Enqueued::Known);
    assert_eq!(a.enqueue(&id("dm/2026-11"), &meta("h3")).await.unwrap(), Enqueued::Queued);
    a.done(&id("dm/2026-11"), "h1").await.unwrap();
    assert_eq!(a.dequeue(None).await.unwrap().unwrap().hash, "h3", "h1 is not h3");
    a.failed(&id("dm/2026-11"), "h3", "403").await.unwrap();
    assert!(a.dequeue(None).await.unwrap().is_none());
    assert_eq!(a.retry_failed().await.unwrap(), 1);
    assert_eq!(a.dequeue(None).await.unwrap().unwrap().hash, "h3");

    // Parked and refused: outside the count, back by their own retries.
    a.park(&id("dm/2026-11"), "h3", REFUSED_GROUP_UNKNOWN).await.unwrap();
    assert_eq!(a.blocked().await.unwrap(), [id("dm/2026-11")]);
    assert_eq!(a.restoring_scopes().await.unwrap(), ["dm"]);
    assert_eq!(a.progress().await.unwrap().total, 4);
    assert_eq!(a.retry_failed().await.unwrap(), 0);
    assert_eq!(a.retry_parked().await.unwrap(), 1);
    a.refuse(&id("dm/2026-11"), "h3", "version 2", "5.2.1").await.unwrap();
    assert_eq!(a.retry_refused("5.2.1").await.unwrap(), 0);
    assert_eq!(a.retry_refused("5.2.2").await.unwrap(), 1);
    a.done(&id("dm/2026-11"), "h3").await.unwrap();
    a.clear_done().await.unwrap();
    assert_eq!(a.progress().await.unwrap().total, 0);
}

#[tokio::test]
async fn a_version_this_device_wrote_is_not_queued() {
    let (a, _) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    let built = a.build(&seg).await.unwrap();
    a.written(&seg, &built.meta).await.unwrap();
    assert_eq!(a.enqueue(&seg.entity_id(), &built.meta).await.unwrap(), Enqueued::Held { unreplayed: false });
    assert!(a.dequeue(None).await.unwrap().is_none());
}

#[tokio::test]
async fn a_replayed_segment_calls_the_replayer_once_per_new_event() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    for n in 1..=3 {
        a.record(dm(n, NOV_1 + n as i64)).await.unwrap();
    }
    a.record(group(9, NOV_1, "g1")).await.unwrap();
    let file = a.build(&seg).await.unwrap();
    b.record(dm(2, NOV_1 + 2)).await.unwrap();
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Queued);
    let item = b.dequeue(None).await.unwrap().unwrap();

    let r = FakeReplayer::default();
    let report = b.replay_segment(&item, &file.bytes, &r).await.unwrap();
    assert_eq!(r.seen(), [hex(1), hex(3)]);
    assert_eq!(report, ReplayReport { applied: 2, duplicate: 1, ignored: 0, failed: 0, truncated: false });
    // The item waits for the caller's word; then the file is held.
    assert_eq!(b.dequeue(None).await.unwrap().unwrap().entity_id, item.entity_id);
    b.done(&item.entity_id, &item.hash).await.unwrap();
    assert!(b.dequeue(None).await.unwrap().is_none());
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: false });

    let again = b.replay_segment(&item, &file.bytes, &r).await.unwrap();
    assert_eq!(again, ReplayReport { applied: 0, duplicate: 3, ignored: 0, failed: 0, truncated: false });
    assert_eq!(r.seen().len(), 2);
    // B holds the month now and could write it, the same file as A's.
    assert_eq!(b.build(&seg).await.unwrap().bytes, file.bytes);

    // A file that is not the item's version, or not its segment, is refused.
    let g = nov(ArchiveScope::Group { id: "g1".into() });
    let gfile = a.build(&g).await.unwrap();
    assert!(b.replay_segment(&item, &gfile.bytes, &r).await.is_err());
    assert_eq!(b.enqueue(&g.entity_id(), &gfile.meta).await.unwrap(), Enqueued::Queued);
    let gitem = b.dequeue(None).await.unwrap().unwrap();
    let report = b.replay_segment(&gitem, &gfile.bytes, &r).await.unwrap();
    assert_eq!(report.ignored, 1, "what the replayer does not take");
}

/// The runtime goes between sessions in the middle of a segment: the
/// item stays pending, the events taken so far are marked, and the next
/// restore replays the rest from what is held, without the file.
#[tokio::test]
async fn an_interrupted_replay_resumes_from_what_is_held_without_the_file() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    for n in 1..=4 {
        a.record(dm(n, NOV_1 + n as i64)).await.unwrap();
    }
    let file = a.build(&seg).await.unwrap();
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Queued);
    let item = b.dequeue(None).await.unwrap().unwrap();

    let r = FakeReplayer { logged_out_at: Some(2), ..Default::default() };
    let err = b.replay_segment(&item, &file.bytes, &r).await.unwrap_err();
    assert!(matches!(err, MessengerError::NotLoggedIn));
    assert_eq!(r.seen(), [hex(1), hex(2)]);
    assert_eq!(b.dequeue(None).await.unwrap().unwrap().hash, item.hash, "still pending");
    // The op comes again (a restart): the version is held, two events await.
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: true });
    assert!(b.holds(&seg, &file.meta.hash).await.unwrap());

    // The next session: the rest, from the rows held.
    let r = FakeReplayer::default();
    let report = b.replay_held_of(&item, &r).await.unwrap().expect("held: no download");
    assert_eq!(r.seen(), [hex(3), hex(4)]);
    assert_eq!(report.applied, 2);
    b.done(&item.entity_id, &item.hash).await.unwrap();
    assert!(b.dequeue(None).await.unwrap().is_none());
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: false });
    // A version not held is not replayed from what is held.
    let other = QueueItem { hash: "nope".into(), ..item };
    assert!(b.replay_held_of(&other, &r).await.unwrap().is_none());
}

/// A group the device has no row for stops the segment where it is, to
/// be parked; a file of a later format is refused as such; a record the
/// handler refuses is left behind and the rest of the month comes.
#[tokio::test]
async fn refusals_are_told_apart() {
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    let g = nov(ArchiveScope::Group { id: "g1".into() });
    a.record(group(1, NOV_1 + 1, "g1")).await.unwrap();
    let file = a.build(&g).await.unwrap();
    b.enqueue(&g.entity_id(), &file.meta).await.unwrap();
    let item = b.dequeue(None).await.unwrap().unwrap();
    let r = FakeReplayer { group_unknown: true, ..Default::default() };
    let err = b.replay_segment(&item, &file.bytes, &r).await.unwrap_err();
    assert!(is_parked_reason(&err), "{err}");
    assert!(!is_format_error(&err));

    let mut v2 = file.bytes.clone();
    v2[4] = 2;
    let err = b.replay_segment(&item, &v2, &r).await.unwrap_err();
    assert!(is_format_error(&err), "{err}");

    struct Refuser;
    #[async_trait::async_trait]
    impl Replayer for Refuser {
        async fn replay(&self, event: ArchivedEvent) -> messenger_core::Result<Replayed> {
            if event.id.as_hex() == hex(12) {
                return Err(MessengerError::Other("the handler refused the event".into()));
            }
            Ok(Replayed::Applied)
        }
    }
    let seg = nov(ArchiveScope::Dm);
    for n in 11..=13 {
        a.record(dm(n, NOV_1 + n as i64)).await.unwrap();
    }
    let file = a.build(&seg).await.unwrap();
    b.enqueue(&seg.entity_id(), &file.meta).await.unwrap();
    let item = b.dequeue(Some(&ArchiveScope::Dm)).await.unwrap().unwrap();
    let report = b.replay_segment(&item, &file.bytes, &Refuser).await.unwrap();
    assert_eq!((report.applied, report.failed), (2, 1));
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: false }, "nothing left behind unmarked");
}

/// A handler that could not run (the store busy, the box locked) stops
/// the replay where it is: what it took is marked, this event and the
/// rest are not, and the next try takes them from what is held. A handler
/// that refused one event leaves it behind (`refusals_are_told_apart`).
#[tokio::test]
async fn a_handler_that_could_not_run_leaves_the_rest_unreplayed() {
    struct Busy(AtomicUsize);
    #[async_trait::async_trait]
    impl Replayer for Busy {
        async fn replay(&self, _: ArchivedEvent) -> messenger_core::Result<Replayed> {
            if self.0.fetch_add(1, Ordering::SeqCst) == 1 {
                return Err(MessengerError::Storage("database is locked".into()));
            }
            Ok(Replayed::Applied)
        }
    }
    let (a, _) = archive().await;
    let (b, _) = archive().await;
    let seg = nov(ArchiveScope::Dm);
    for n in 21..=23 {
        a.record(dm(n, NOV_1 + n as i64)).await.unwrap();
    }
    let file = a.build(&seg).await.unwrap();
    b.enqueue(&seg.entity_id(), &file.meta).await.unwrap();
    let item = b.dequeue(None).await.unwrap().unwrap();

    let err = b.replay_segment(&item, &file.bytes, &Busy(AtomicUsize::new(0))).await.unwrap_err();
    assert!(matches!(err, MessengerError::Storage(_)), "{err}");
    assert!(stops_replay(&err));
    assert!(!stops_replay(&MessengerError::Invalid("bad".into())) && !stops_replay(&MessengerError::Other("no".into())));
    assert!(stops_replay(&MessengerError::SecretsLocked) && stops_replay(&MessengerError::NotLoggedIn));
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: true }, "the second and the third wait");

    // The next try: from what is held, the two that were not taken.
    let fine = FakeReplayer::default();
    let report = b.replay_held_of(&item, &fine).await.unwrap().unwrap();
    assert_eq!((report.applied, report.failed), (2, 0));
    assert_eq!(fine.seen(), [hex(22), hex(23)]);
    assert_eq!(b.enqueue(&seg.entity_id(), &file.meta).await.unwrap(), Enqueued::Held { unreplayed: false });
}

#[tokio::test]
async fn high_water_and_removal_of_a_scope() {
    let (a, _) = archive().await;
    let g = ArchiveScope::Group { id: "g1".into() };
    assert_eq!(a.high_water(&g).await.unwrap(), None);
    a.record(group(1, NOV_1 + 5, "g1")).await.unwrap();
    a.record(group(2, NOV_1 - 5, "g1")).await.unwrap();
    a.record(dm(3, NOV_1 + 50)).await.unwrap();
    assert_eq!(a.high_water(&g).await.unwrap(), Some(Timestamp(NOV_1 + 5)));
    assert_eq!(a.high_water(&ArchiveScope::Dm).await.unwrap(), Some(Timestamp(NOV_1 + 50)));
    let aug = seg(g.clone(), 2026, 8);
    a.enqueue(&aug.entity_id(), &meta("x")).await.unwrap();

    let gone = a.remove_scope(&g).await.unwrap();
    let o = hex(8);
    assert_eq!(gone, [format!("{o}/group:g1/2026-11"), format!("{o}/group:g1/2026-10"), format!("{o}/group:g1/2026-08")]);
    assert_eq!(a.high_water(&g).await.unwrap(), None);
    assert!(a.dequeue(None).await.unwrap().is_none());
    let due: Vec<String> = a.due(Timestamp(NOV_10), true).await.unwrap().iter().map(SegmentId::entity_id).collect();
    assert_eq!(due, [format!("{o}/dm/2026-11")]);
}

/// D17: of the notes of my own devices that are snapshots of a state, the
/// newest per state is the one a file holds; the rest are compacted away
/// like deleted messages, at the build and in the monthly pass.
#[tokio::test]
async fn snapshots_of_my_own_devices_keep_only_the_newest() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    let keys = |cid: &str| format!(r#"{{"v":1,"t":"own.channel_keys","community_id":"{cid}","scopes":[]}}"#);
    a.record(own_note(1, NOV_1 + 1, &keys("aa"))).await.unwrap();
    a.record(own_note(2, NOV_1 + 2, &keys("aa"))).await.unwrap();
    a.record(own_note(3, NOV_1 + 3, &keys("bb"))).await.unwrap();
    a.record(own_note(4, NOV_1 + 4, &keys("aa"))).await.unwrap();
    a.record(own_note(5, NOV_1 + 5, r#"{"v":1,"t":"own.read","chat":"dm:x","at":5}"#)).await.unwrap();
    a.record(own_note(6, NOV_1 + 6, r#"{"v":1,"t":"own.read","chat":"dm:x","at":6}"#)).await.unwrap();
    a.record(own_note(7, NOV_1 + 7, r#"{"v":1,"t":"own.read","chat":"dm:y","at":7}"#)).await.unwrap();
    // History stays whatever came after it: a message, a call record, a hidden message.
    a.record(dm(8, NOV_1 + 8)).await.unwrap();
    a.record(own_note(9, NOV_1 + 9, r#"{"v":1,"t":"own.call","peer":"x"}"#)).await.unwrap();
    a.record(own_note(10, NOV_1 + 10, r#"{"v":1,"t":"own.hide","chat":"dm:x","target":"m"}"#)).await.unwrap();

    let built = a.build(&seg).await.unwrap();
    assert_eq!(ids(&built.bytes), [hex(3), hex(4), hex(6), hex(7), hex(8), hex(9), hex(10)]);
    assert_eq!(built.meta.count, 7);
    assert!(!String::from_utf8_lossy(&built.bytes).contains(&hex(1001)), "the oldest note of aa is gone");
    assert_eq!(a.compact(&seg).await.unwrap(), 0, "nothing left to drop");

    // A newer snapshot recorded later supersedes the one in the file.
    a.record(own_note(11, NOV_1 + 11, &keys("bb"))).await.unwrap();
    assert_eq!(a.compact(&seg).await.unwrap(), 1);
    let again = a.build(&seg).await.unwrap();
    assert_eq!(ids(&again.bytes), [hex(4), hex(6), hex(7), hex(8), hex(9), hex(10), hex(11)]);
}

#[tokio::test]
async fn a_message_deleted_for_everyone_leaves_the_file_and_does_not_come_back() {
    let seg = nov(ArchiveScope::Dm);
    let (a, _) = archive().await;
    a.record(dm(1, NOV_1 + 1)).await.unwrap();
    a.record(dm(2, NOV_1 + 2)).await.unwrap();
    let before = a.build(&seg).await.unwrap();
    a.written(&seg, &before.meta).await.unwrap();
    assert_eq!(a.compact(&seg).await.unwrap(), 0, "nothing deleted yet");

    // The pipeline applied a deletion of message 1 (its rumor id).
    sqlx::query(
        "INSERT INTO msg_messages (id, chat_id, direction, status, content_type, envelope_json, sender_pubkey, created_at, received_at, deleted_at)
         VALUES (?, 'dm:bb', 'in', 'received', 'text', '{}', 'bb', 1, 1, 9)",
    )
    .bind(hex(1001))
    .execute(a.store.pool())
    .await
    .unwrap();
    assert_eq!(a.compact(&seg).await.unwrap(), 1);
    let after = a.build(&seg).await.unwrap();
    assert_eq!(ids(&after.bytes), [hex(2)]);
    assert!(!String::from_utf8_lossy(&after.bytes).contains("message 1"));
    assert!(a.due(Timestamp(NOV_10), true).await.unwrap().contains(&seg), "to be rewritten");

    // The storage still has the old file: the dropped event is not
    // replayed again, and the compacted file is the one to upload.
    let r = FakeReplayer::default();
    let m = a.merge(&seg, &before.bytes, &r).await.unwrap();
    assert!(r.seen().is_empty());
    assert_eq!(m.upload.map(|u| ids(&u.bytes)), Some(vec![hex(2)]));

    // The scope removed and the old file merged again (a group joined
    // anew): the tombstone keeps the deleted message out.
    a.remove_scope(&ArchiveScope::Dm).await.unwrap();
    let m = a.merge(&seg, &before.bytes, &r).await.unwrap();
    assert_eq!(r.seen(), [hex(2)], "message 2 is new here again, message 1 is not");
    assert_eq!(m.upload.map(|u| ids(&u.bytes)), Some(vec![hex(2)]));
}

/// A month over the bounds of the format is not written: the build says
/// so from the sizes the store tells, before the file is made in memory,
/// and the month stays as it is, dirty, for the handler to set aside.
#[tokio::test]
async fn a_month_over_the_bounds_is_not_built() {
    let (archive, _) = archive().await;
    let text = "x".repeat(900 * 1024);
    for n in 0..20u64 {
        let mut event = dm(n + 1, NOV_1 + 100 + n as i64);
        let Body::Dm { inbound } = &mut event.body else { unreachable!() };
        inbound.content = format!(r#"{{"v":1,"t":"text","text":"{text}"}}"#);
        archive.record(event).await.unwrap();
    }
    let seg = SegmentId { owner: owner(), scope: ArchiveScope::Dm, month: Month { year: 2026, month: 11 } };
    let e = archive.build(&seg).await.unwrap_err();
    assert!(is_too_large(&e), "{e}");
    assert!(is_format_error(&e), "refused as a format this build does not write");
    assert_eq!(archive.due(Timestamp(NOV_10), true).await.unwrap(), vec![seg], "still to write, nothing lost");
    assert_eq!(archive.unwritable().await.unwrap(), ["2026-11"], "and named as in no backup");
}

/// The bounds count the header of the file and every bound of the format
/// (ROO-692): records that alone fill [`MAX_SEGMENT_BYTES`] are over it,
/// as are a record too long and too many records.
#[test]
fn the_bounds_of_a_build_count_the_header() {
    use crate::collect::over_bounds;
    use messenger_store::archive::LiveSize;
    let dm = ArchiveScope::Dm;
    let head = segment::header_len(&dm) as i64;
    let max = MAX_SEGMENT_BYTES as i64;
    assert!(over_bounds(&dm, &LiveSize { count: 16, bytes: max, longest: 1024 }).is_some());
    assert!(over_bounds(&dm, &LiveSize { count: 16, bytes: max - head, longest: 1024 }).is_none());
    assert!(over_bounds(&dm, &LiveSize { count: 16, bytes: max - head + 1, longest: 1024 }).is_some());
    assert!(over_bounds(&dm, &LiveSize { count: 1, bytes: 10, longest: MAX_RECORD_BYTES as i64 + 1 }).is_some());
    assert!(over_bounds(&dm, &LiveSize { count: MAX_RECORDS as i64 + 1, bytes: 10, longest: 1 }).is_some());
}

/// A file of the storage refused as over the bounds — its size, a record,
/// the count of records — is a month named as not restored here; one
/// refused for another reason is not.
#[tokio::test]
async fn the_months_refused_as_too_large_are_named() {
    let (a, _) = archive().await;
    let big = SegmentId { owner: owner(), scope: ArchiveScope::Dm, month: Month { year: 2026, month: 10 } };
    let other = SegmentId { owner: owner(), scope: ArchiveScope::Dm, month: Month { year: 2026, month: 9 } };
    for seg in [&big, &other] {
        a.enqueue(&seg.entity_id(), &meta(&seg.month.to_string())).await.unwrap();
    }
    a.refuse(&big.entity_id(), "2026-10", &too_large("a record of 2000000 bytes").to_string(), "b1").await.unwrap();
    a.refuse(&other.entity_id(), "2026-09", &format_error("version 2").to_string(), "b1").await.unwrap();
    assert_eq!(a.refused_too_large().await.unwrap(), ["2026-10"]);
}

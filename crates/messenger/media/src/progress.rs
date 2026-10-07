// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a transfer shows while it runs: its stage, chunks, bytes, speed
//! and time left.
//!
//! The workers only count (`Live`). The meter reads the counts and decides
//! when to tell: at once on a new stage or status, otherwise at most once
//! per 250 ms, and every second while running so that speed and time left
//! keep moving on a slow line. Events are made from memory; the database
//! is not read for them.
//!
//! Bytes and chunks start from where an earlier run of the transfer
//! stopped, as its row says, until this run knows better: once it is past
//! checking, or starts the file over, it tells what it really has.

use messenger_store::media::{ST_CANCELLED, ST_DONE, ST_FAILED, ST_PAUSED, ST_QUEUED, ST_RUNNING, ST_WAITING_RETRY};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;
use ts_rs::TS;

/// At most one event per transfer this often, new stages and statuses aside.
pub const THROTTLE: Duration = Duration::from_millis(250);
/// An event at least this often while running.
pub const HEARTBEAT: Duration = Duration::from_secs(1);
/// Speed is averaged over about this long.
const RATE_WINDOW_SECS: f64 = 3.0;
/// The first speed is told once bytes moved for this long: the first
/// moments of a request only fill buffers, far faster than the line.
const RATE_SEED_SECS: f64 = 2.0;
/// Nothing moved for this long: no speed at all. Meanwhile the speed fades
/// toward zero, and no time left is told once nothing moved for
/// `RATE_WINDOW_SECS`.
const STALL_SECS: f64 = 10.0;
/// Less time left than this is not told: it would only flicker.
const ETA_MIN_SECS: u64 = 5;
/// A chunk in flight counts for at most this share (per mille) of its
/// bytes until the server confirms it.
const IN_FLIGHT_SHARE: u64 = 950;

/// Statuses of a transfer under way.
pub(crate) const UNDER_WAY: [&str; 3] = [ST_QUEUED, ST_RUNNING, ST_WAITING_RETRY];
/// Statuses a run ends with before the file is done.
const ENDED: [&str; 3] = [ST_PAUSED, ST_FAILED, ST_CANCELLED];

/// Where a transfer is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TransferStage {
    /// Waiting for a slot.
    #[default]
    Queued,
    /// The file is made ready before its upload: a photo is made smaller
    /// (the runtime, before the run starts).
    Preparing,
    /// Chunks an earlier attempt kept are checked before anything moves:
    /// looked up on the server (upload) or verified on disk (download).
    Checking,
    Uploading,
    /// The file is stored and the message that carries it goes out
    /// (`MediaService::upload_to_publish`).
    Publishing,
    Downloading,
    /// Chunks are decrypted and joined into the file.
    Assembling,
    /// The hash of the whole file is checked.
    Verifying,
}

impl TransferStage {
    const ALL: [Self; 8] = [
        Self::Queued,
        Self::Preparing,
        Self::Checking,
        Self::Uploading,
        Self::Publishing,
        Self::Downloading,
        Self::Assembling,
        Self::Verifying,
    ];

    fn from_u8(v: u8) -> Self {
        Self::ALL.get(v as usize).copied().unwrap_or_default()
    }

    /// Nothing goes over the network in this stage: it has no speed and no
    /// time left.
    fn moves_no_bytes(self) -> bool {
        matches!(self, Self::Queued | Self::Preparing | Self::Publishing | Self::Assembling | Self::Verifying)
    }
}

/// One event about a transfer, as the host receives it: the payload of
/// the event `transfer.progress`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Progress {
    pub transfer_id: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    /// `up` | `down`
    #[ts(type = "\"up\" | \"down\"")]
    pub direction: String,
    /// queued | running | waiting_retry | paused | done | failed | cancelled
    #[ts(type = "\"queued\" | \"running\" | \"waiting_retry\" | \"paused\" | \"done\" | \"failed\" | \"cancelled\"")]
    pub status: String,
    /// Plaintext bytes: confirmed chunks and part of those in flight. A
    /// run starts where the last one stopped until it knows better (it
    /// goes past checking, or starts the file over); from then on it never
    /// goes back, unless what it counted is no longer stored (a new key,
    /// another server). A run that ends (paused, failed, cancelled) tells
    /// what is stored, as its row does: the requests it dropped count for
    /// nothing, so the next run starts from that very number.
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    /// Always an `err.*` code.
    pub failure_reason: Option<String>,
    pub local_path: Option<String>,
    pub stage: TransferStage,
    /// Chunks finished in the current stage (checking and assembling count
    /// from 0); never goes back within a stage.
    pub chunks_done: u32,
    pub chunks_total: u32,
    #[ts(type = "number")]
    pub chunk_size: u64,
    /// Bytes per second really moved (not chunks found on the server or
    /// on disk), averaged over about three seconds and first told after
    /// two; it fades while nothing moves, and is 0 after ten seconds of it.
    /// Always 0 in a stage that moves nothing over the network (queued,
    /// preparing, publishing, assembling, verifying).
    #[ts(type = "number")]
    pub rate_bps: u64,
    /// Seconds left; none when nothing moved for three seconds, or less
    /// than five remain.
    #[ts(type = "number | null")]
    pub eta_secs: Option<u64>,
    /// When the next automatic attempt starts (unix ms), while waiting for it.
    #[ts(type = "number | null")]
    pub retry_at_ms: Option<i64>,
    /// Automatic retries made in this run.
    pub attempt: u32,
    pub file_name: String,
    pub mime: String,
}

/// Receives progress; the runtime forwards it to the host.
pub trait ProgressSink: Send + Sync {
    fn progress(&self, p: Progress);
}

/// Counts of a running transfer, written by its workers.
///
/// They belong to one run of the transfer and never start again within
/// it: a chunk is counted once, however many automatic retries pass over
/// it. Chunks that needed no transfer (on the server or on disk already)
/// are counted apart, so they never make the speed.
#[derive(Default)]
pub struct Live {
    stage: AtomicU8,
    /// Stages entered since the meter last looked, in order.
    entered: Mutex<Vec<TransferStage>>,
    /// Every chunk of the file: done or not, and how.
    marks: Mutex<Vec<Mark>>,
    /// Chunks marked done.
    marked: AtomicU32,
    /// Chunks decrypted while assembling (that stage counts again from 0).
    assembled: AtomicU32,
    chunks_total: AtomicU32,
    chunk_size: AtomicU64,
    /// Plaintext bytes of the chunks marked done.
    confirmed: AtomicU64,
    /// The part of `confirmed` that was found, not moved.
    found: AtomicU64,
    /// Chunks in flight: bytes sent so far and the plaintext size.
    flights: Mutex<HashMap<u64, (Arc<AtomicU64>, u64)>>,
    next_flight: AtomicU64,
    /// Bumped whenever the counts go down (a new key, chunks no longer
    /// stored): what was told before no longer holds.
    basis: AtomicU32,
    /// The run started the file over: how far an earlier run got is void.
    anew: AtomicBool,
    changed: Notify,
}

/// A chunk done in this run: its plaintext bytes, and whether it was found
/// (already on the server or on disk) rather than moved.
#[derive(Clone, Copy, Default)]
struct Mark {
    bytes: u64,
    found: bool,
    done: bool,
}

impl Live {
    pub fn stage(&self) -> TransferStage {
        TransferStage::from_u8(self.stage.load(Ordering::SeqCst))
    }

    pub fn set_stage(&self, s: TransferStage) {
        let mut entered = self.entered.lock().unwrap_or_else(|e| e.into_inner());
        if self.stage.swap(s as u8, Ordering::SeqCst) != s as u8 {
            entered.push(s);
            drop(entered);
            self.changed.notify_one();
        }
    }

    fn take_entered(&self) -> Vec<TransferStage> {
        std::mem::take(&mut *self.entered.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn marks(&self) -> std::sync::MutexGuard<'_, Vec<Mark>> {
        self.marks.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A pass over `total` chunks begins. A pass of the same file (an
    /// automatic retry) keeps what the run counted; another shape of it
    /// starts the counts again.
    pub fn start(&self, total: u32, chunk_size: u64) {
        let same = self.chunks_total.load(Ordering::SeqCst) == total
            && self.chunk_size.load(Ordering::SeqCst) == chunk_size
            && self.marks().len() == total as usize;
        if !same {
            self.reset(total, chunk_size);
        }
    }

    /// Count again from nothing: the file is sent anew (a new key). What
    /// this run or an earlier one told is void.
    pub fn start_over(&self, total: u32, chunk_size: u64) {
        self.reset(total, chunk_size);
        self.anew.store(true, Ordering::SeqCst);
        self.basis.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }

    /// See the field.
    pub fn basis(&self) -> u32 {
        self.basis.load(Ordering::SeqCst)
    }

    /// See the field.
    pub fn anew(&self) -> bool {
        self.anew.load(Ordering::SeqCst)
    }

    fn reset(&self, total: u32, chunk_size: u64) {
        let mut marks = self.marks();
        *marks = vec![Mark::default(); total as usize];
        self.chunks_total.store(total, Ordering::SeqCst);
        self.chunk_size.store(chunk_size, Ordering::SeqCst);
        self.confirmed.store(0, Ordering::SeqCst);
        self.found.store(0, Ordering::SeqCst);
        self.marked.store(0, Ordering::SeqCst);
        self.assembled.store(0, Ordering::SeqCst);
        drop(marks);
        self.changed.notify_one();
    }

    /// Is chunk `index` done in this run?
    pub fn is_done(&self, index: usize) -> bool {
        self.marks().get(index).is_some_and(|m| m.done)
    }

    fn mark(&self, index: usize, bytes: u64, found: bool) {
        let mut marks = self.marks();
        let Some(m) = marks.get_mut(index) else { return };
        if m.done {
            return;
        }
        *m = Mark { bytes, found, done: true };
        self.confirmed.fetch_add(bytes, Ordering::SeqCst);
        if found {
            self.found.fetch_add(bytes, Ordering::SeqCst);
        }
        self.marked.fetch_add(1, Ordering::SeqCst);
        drop(marks);
        self.changed.notify_one();
    }

    /// Chunk `index` of `bytes` plaintext needs no transfer: it is on the
    /// server or on disk already. Counted once, and never as speed.
    pub fn found(&self, index: usize, bytes: u64) {
        self.mark(index, bytes, true);
    }

    /// Chunk `index` is not done after all (no longer stored): the counts
    /// go down, and what was told before no longer holds.
    pub fn unmark(&self, index: usize) {
        let mut marks = self.marks();
        let Some(m) = marks.get_mut(index) else { return };
        if !m.done {
            return;
        }
        let was = std::mem::take(m);
        self.confirmed.fetch_sub(was.bytes, Ordering::SeqCst);
        if was.found {
            self.found.fetch_sub(was.bytes, Ordering::SeqCst);
        }
        self.marked.fetch_sub(1, Ordering::SeqCst);
        self.basis.fetch_add(1, Ordering::SeqCst);
        drop(marks);
        self.changed.notify_one();
    }

    /// Chunks done in this run.
    pub fn marked(&self) -> u32 {
        self.marked.load(Ordering::SeqCst)
    }

    /// Chunks decrypted so far while assembling.
    pub fn set_assembled(&self, n: u32) {
        self.assembled.store(n, Ordering::SeqCst);
        self.changed.notify_one();
    }

    /// Chunks finished in the current stage; see `chunks_done_in`.
    pub fn chunks_done(&self) -> u32 {
        self.chunks_done_in(self.stage())
    }

    /// Chunks finished in `stage`: assembling and verifying count the
    /// chunks decrypted, the others those done.
    pub fn chunks_done_in(&self, stage: TransferStage) -> u32 {
        match stage {
            TransferStage::Assembling | TransferStage::Verifying => self.assembled.load(Ordering::SeqCst),
            _ => self.marked(),
        }
    }

    /// Plaintext bytes of the chunks done so far.
    pub fn confirmed(&self) -> u64 {
        self.confirmed.load(Ordering::SeqCst)
    }

    /// A chunk of `plain` bytes goes on the wire; count what leaves into
    /// `sent` of the guard. Dropped unconfirmed, it counts for nothing.
    pub fn begin(&self, plain: u64) -> InFlight<'_> {
        let id = self.next_flight.fetch_add(1, Ordering::SeqCst);
        let sent = Arc::new(AtomicU64::new(0));
        self.flights.lock().unwrap_or_else(|e| e.into_inner()).insert(id, (sent.clone(), plain));
        InFlight { live: self, id, plain, sent }
    }

    fn flying(&self) -> u64 {
        self.flights
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|(sent, plain)| sent.load(Ordering::Relaxed).min(plain * IN_FLIGHT_SHARE / 1000))
            .sum()
    }

    /// Confirmed bytes and a share of those in flight, as raw counts (the
    /// meter keeps them from going back).
    pub fn done_bytes(&self) -> u64 {
        self.confirmed() + self.flying()
    }

    /// Bytes this run really moved: `done_bytes` without what was found.
    /// The speed is made of these alone.
    pub fn moved(&self) -> u64 {
        self.confirmed().saturating_sub(self.found.load(Ordering::SeqCst)) + self.flying()
    }
}

/// One chunk on the wire; see [`Live::begin`].
pub struct InFlight<'a> {
    live: &'a Live,
    id: u64,
    plain: u64,
    pub sent: Arc<AtomicU64>,
}

impl InFlight<'_> {
    /// The server has chunk `index` (or it is on disk): its bytes count
    /// whole.
    pub fn confirm(self, index: usize) {
        self.live.flights.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
        self.live.mark(index, self.plain, false);
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.live.flights.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

struct MeterState {
    p: Progress,
    /// The last event: when, and what it said.
    last: Option<(Instant, TransferStage, String, u64, u32)>,
    /// `done_bytes` never goes below what this run told, on one basis of
    /// the counts.
    floor: u64,
    /// The basis of the counts the floor belongs to (`Live::basis`).
    basis: u32,
    /// How far an earlier run got, by its row: told while the run does not
    /// know better yet (queued, checking), and dropped once it does.
    guess: u64,
    /// Nor `chunks_done` within one stage: the stage and its least.
    chunk_floor: (TransferStage, u32),
    rate: f64,
    rated: bool,
    /// Where the speed is measured from: when, and the bytes moved by
    /// then. Before the first speed it marks the start of the first window.
    sample: Option<(Instant, u64)>,
    /// Bytes moved at the last look, and when they last grew.
    moved: u64,
    moved_at: Option<Instant>,
}

/// Turns the counts of one transfer into events; see the module.
pub struct ProgressMeter<'a> {
    sink: &'a dyn ProgressSink,
    live: Arc<Live>,
    /// The last event of every running transfer, for views read meanwhile.
    views: Option<Arc<Mutex<HashMap<String, Progress>>>>,
    state: Mutex<MeterState>,
}

impl<'a> ProgressMeter<'a> {
    /// `base` holds what does not change (ids, names, sizes), the status
    /// to start from and how far an earlier run got: bytes and chunks
    /// start from there until the run knows how far it really is.
    pub fn new(sink: &'a dyn ProgressSink, live: Arc<Live>, base: Progress) -> Self {
        let guess = base.done_bytes;
        let chunk_floor = (base.stage, base.chunks_done);
        let basis = live.basis();
        Self {
            sink,
            live,
            views: None,
            state: Mutex::new(MeterState {
                p: base,
                last: None,
                floor: 0,
                basis,
                guess,
                chunk_floor,
                rate: 0.0,
                rated: false,
                sample: None,
                moved: 0,
                moved_at: None,
            }),
        }
    }

    pub fn with_views(mut self, views: Arc<Mutex<HashMap<String, Progress>>>) -> Self {
        self.views = Some(views);
        self
    }

    pub fn live(&self) -> &Arc<Live> {
        &self.live
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MeterState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new status, told at once. Leaving `running` stops the clock of
    /// the speed; leaving `waiting_retry` forgets its time. A cancelled
    /// transfer keeps nothing, and tells so.
    pub fn status(&self, status: &str, reason: Option<&str>) {
        let now = Instant::now();
        let mut st = self.lock();
        st.p.status = status.to_string();
        st.p.failure_reason = reason.map(str::to_string);
        if status != ST_RUNNING {
            st.rate = 0.0;
            st.rated = false;
            st.sample = None;
            st.moved_at = None;
        }
        if status != ST_WAITING_RETRY {
            st.p.retry_at_ms = None;
        }
        if status == ST_CANCELLED {
            // A cancel removes what was stored: nothing of it is told.
            let live = &self.live;
            live.start_over(live.chunks_total.load(Ordering::SeqCst), live.chunk_size.load(Ordering::SeqCst));
        }
        if ENDED.contains(&status) {
            st.floor = self.live.confirmed();
        }
        // Stages entered meanwhile go out with the new status; the last of
        // them is the present state.
        if !self.flush_stages(&mut st, now) {
            let p = self.snapshot(&mut st, now);
            self.send(&mut st, p, now);
        }
    }

    /// The automatic retry this run is at, and when the next one starts.
    pub fn retry(&self, attempt: u32, retry_at_ms: Option<i64>) {
        let mut st = self.lock();
        st.p.attempt = attempt;
        st.p.retry_at_ms = retry_at_ms;
    }

    /// Finished: every byte counts, and the file is at `local_path`.
    pub fn done(&self, local_path: Option<String>) {
        {
            let mut st = self.lock();
            st.floor = st.p.total_bytes;
            if local_path.is_some() {
                st.p.local_path = local_path;
            }
        }
        self.status(ST_DONE, None);
    }

    /// Tell what is due: every stage entered, then the present state when
    /// the throttle allows or the heartbeat asks.
    pub fn poll(&self) {
        let now = Instant::now();
        let mut st = self.lock();
        self.flush_stages(&mut st, now);
        let p = self.snapshot(&mut st, now);
        let due = match &st.last {
            None => true,
            Some((t, stage, status, done, chunks)) => {
                let since = now.saturating_duration_since(*t);
                *stage != p.stage
                    || *status != p.status
                    || (since >= THROTTLE && (*done != p.done_bytes || *chunks != p.chunks_done))
                    || (since >= HEARTBEAT && p.status == ST_RUNNING)
            }
        };
        if due {
            self.send(&mut st, p, now);
        }
    }

    /// Poll on every change of the counts and four times a second, for
    /// as long as the transfer runs (it is dropped with it).
    pub async fn run(&self) -> std::convert::Infallible {
        let mut tick = tokio::time::interval(THROTTLE);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = self.live.changed.notified() => {}
            }
            self.poll();
        }
    }

    /// One event for every stage entered since the last look, in order.
    /// Whether there was any.
    fn flush_stages(&self, st: &mut MeterState, now: Instant) -> bool {
        let entered = self.live.take_entered();
        for stage in &entered {
            let p = self.snapshot_at(st, now, *stage);
            self.send(st, p, now);
        }
        !entered.is_empty()
    }

    fn snapshot(&self, st: &mut MeterState, now: Instant) -> Progress {
        self.snapshot_at(st, now, self.live.stage())
    }

    fn snapshot_at(&self, st: &mut MeterState, now: Instant, stage: TransferStage) -> Progress {
        let total = st.p.total_bytes;
        let basis = self.live.basis();
        if basis != st.basis {
            // The counts went down for good (a new key, chunks no longer
            // stored): they start again from what is really done.
            st.basis = basis;
            st.floor = 0;
            st.chunk_floor = (stage, 0);
        }
        // Past checking, or starting the file over, the run knows how far
        // it is: the earlier run's word no longer counts.
        let knows = !matches!(stage, TransferStage::Queued | TransferStage::Preparing | TransferStage::Checking);
        if knows || self.live.anew() {
            st.guess = 0;
        }
        let done = st.floor.max(self.live.done_bytes()).min(total);
        st.floor = done;
        let done = done.max(st.guess.min(total));
        if st.chunk_floor.0 != stage {
            st.chunk_floor = (stage, 0);
        }
        let chunks = st.chunk_floor.1.max(self.live.chunks_done_in(stage));
        st.chunk_floor.1 = chunks;
        if stage.moves_no_bytes() {
            // No speed: the last one of the line would linger while the
            // message goes out or the file is joined, and come back after.
            st.rate = 0.0;
            st.rated = false;
            st.sample = None;
            st.moved_at = None;
        } else if st.p.status == ST_RUNNING {
            self.sample_rate(st, now);
        }
        let rate = st.rate.max(0.0).round() as u64;
        // A line that stopped has no time left to tell, though its speed
        // still fades.
        let still = st.moved_at.is_none_or(|t| now.saturating_duration_since(t).as_secs_f64() >= RATE_WINDOW_SECS);
        let eta = match rate {
            _ if still => None,
            0 => None,
            r => Some(total.saturating_sub(done) / r).filter(|s| *s >= ETA_MIN_SECS),
        };
        let mut p = st.p.clone();
        p.done_bytes = done;
        p.stage = stage;
        p.chunks_done = chunks;
        p.chunks_total = self.live.chunks_total.load(Ordering::SeqCst);
        p.chunk_size = self.live.chunk_size.load(Ordering::SeqCst);
        p.rate_bps = if p.status == ST_RUNNING { rate } else { 0 };
        p.eta_secs = if p.status == ST_RUNNING { eta } else { None };
        p
    }

    /// The speed from the bytes really moved, averaged over about three
    /// seconds. The first speed is the average of the first two seconds
    /// after bytes are first seen to move; the bytes of that first look
    /// (buffers filled at once) are not counted. While nothing moves the
    /// speed fades; after `STALL_SECS` of it there is none, and it starts
    /// afresh when bytes move again.
    fn sample_rate(&self, st: &mut MeterState, now: Instant) {
        let moved = self.live.moved();
        let grew = moved > st.moved;
        if grew {
            st.moved_at = Some(now);
        }
        st.moved = moved;
        match st.sample {
            // A request that failed and starts again sends its bytes anew.
            Some((_, b0)) if moved < b0 => st.sample = Some((now, moved)),
            Some((t0, b0)) => {
                let dt = now.saturating_duration_since(t0).as_secs_f64();
                let inst = (moved - b0) as f64 / dt.max(f64::EPSILON);
                if !st.rated && dt >= RATE_SEED_SECS {
                    st.rate = inst;
                    st.rated = true;
                    st.sample = Some((now, moved));
                } else if st.rated && dt >= 0.2 {
                    let a = 1.0 - (-dt / RATE_WINDOW_SECS).exp();
                    st.rate += a * (inst - st.rate);
                    st.sample = Some((now, moved));
                }
            }
            None if grew => st.sample = Some((now, moved)),
            None => {}
        }
        let silent = st.moved_at.is_none_or(|t| now.saturating_duration_since(t).as_secs_f64() >= STALL_SECS);
        if silent {
            st.rate = 0.0;
            st.rated = false;
            st.sample = None;
        }
    }

    fn send(&self, st: &mut MeterState, p: Progress, now: Instant) {
        st.last = Some((now, p.stage, p.status.clone(), p.done_bytes, p.chunks_done));
        // Only a transfer under way is kept: the end of one is in its row.
        if let Some(views) = self.views.as_ref().filter(|_| UNDER_WAY.contains(&p.status.as_str())) {
            views.lock().unwrap_or_else(|e| e.into_inner()).insert(p.transfer_id.clone(), p.clone());
        }
        self.sink.progress(p);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every event with the (test clock) time it came.
    #[derive(Default)]
    pub(crate) struct Timed(pub Mutex<Vec<(Instant, Progress)>>);
    impl ProgressSink for Timed {
        fn progress(&self, p: Progress) {
            self.0.lock().unwrap().push((Instant::now(), p));
        }
    }

    pub(crate) fn base(total: u64) -> Progress {
        Progress {
            transfer_id: "t".into(),
            message_id: None,
            chat_id: None,
            direction: "up".into(),
            status: "queued".into(),
            done_bytes: 0,
            total_bytes: total,
            failure_reason: None,
            local_path: None,
            stage: TransferStage::Queued,
            chunks_done: 0,
            chunks_total: 0,
            chunk_size: 0,
            rate_bps: 0,
            eta_secs: None,
            retry_at_ms: None,
            attempt: 0,
            file_name: "f.bin".into(),
            mime: "application/octet-stream".into(),
        }
    }

    /// Gaps between events that tell nothing new in stage or status.
    pub(crate) fn plain_gaps(events: &[(Instant, Progress)]) -> Vec<Duration> {
        events
            .windows(2)
            .filter(|w| w[0].1.stage == w[1].1.stage && w[0].1.status == w[1].1.status)
            .map(|w| w[1].0 - w[0].0)
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn throttled_with_a_heartbeat_speed_and_time_left() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(1000, 1000);
        let meter = ProgressMeter::new(&sink, live.clone(), base(1_000_000));
        meter.status("running", None);
        live.set_stage(TransferStage::Uploading);

        // 100 kB/s for four seconds in 1 kB chunks, then nothing for eleven.
        let drive = async {
            for i in 0..400 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                live.begin(1000).confirm(i);
            }
            tokio::time::sleep(Duration::from_secs(11)).await;
        };
        tokio::select! {
            _ = drive => {}
            never = meter.run() => match never {},
        }
        let events = sink.0.lock().unwrap().clone();
        let stages: Vec<_> = events.iter().map(|(_, p)| p.stage).collect();
        assert_eq!(&stages[..2], &[TransferStage::Queued, TransferStage::Uploading], "a new stage is told at once");
        for gap in plain_gaps(&events) {
            assert!(gap >= THROTTLE, "at most one event per 250 ms: {gap:?}");
        }
        assert!(events.len() <= 2 + 4 * 4 + 11 + 1, "{} events", events.len());
        assert!(events.windows(2).all(|w| w[0].1.done_bytes <= w[1].1.done_bytes && w[0].1.chunks_done <= w[1].1.chunks_done));

        // While the bytes flowed the speed was about 100 kB/s, with time left.
        let flowing = events.iter().filter(|(t, _)| *t - events[0].0 > Duration::from_secs(3)).find(|(_, p)| p.chunks_done < 400).unwrap();
        assert!((80_000..=120_000).contains(&flowing.1.rate_bps), "{}", flowing.1.rate_bps);
        assert!(flowing.1.eta_secs.is_some_and(|s| (5..=12).contains(&s)), "{:?}", flowing.1.eta_secs);

        // Stalled: a heartbeat every second, the speed fades and never grows.
        let stalled: Vec<_> = events.iter().filter(|(_, p)| p.chunks_done == 400).collect();
        assert!(stalled.len() >= 10, "heartbeats: {}", stalled.len());
        let rates: Vec<u64> = stalled.iter().map(|(_, p)| p.rate_bps).collect();
        assert!(rates.windows(2).all(|w| w[1] <= w[0]), "{rates:?}");
        assert_eq!(stalled.last().unwrap().1.done_bytes, 400_000);
        // Three seconds without a byte: no time left, not one that grows
        // while the speed fades. Ten: no speed either.
        let last_flow = events.iter().rev().find(|(_, p)| p.chunks_done < 400).unwrap().0;
        let still: Vec<_> = events.iter().filter(|(t, _)| *t - last_flow >= Duration::from_millis(3300)).collect();
        assert!(!still.is_empty());
        for (t, p) in still {
            assert_eq!(p.eta_secs, None, "{:?} after the last byte", *t - last_flow);
            if *t - last_flow >= Duration::from_millis(10_300) {
                assert_eq!(p.rate_bps, 0, "{:?} after the last byte", *t - last_flow);
            }
        }
        assert_eq!(events.last().unwrap().1.rate_bps, 0);
    }

    /// A streamed upload counts its bytes as the connection takes them:
    /// at first a burst (buffers filled at once), then steps of a piece as
    /// the line drains. The speed is the line's, neither the burst nor a
    /// step, and coming back after a stall it never jumps above it.
    #[tokio::test(start_paused = true)]
    async fn the_speed_of_a_streamed_upload_is_the_line_not_its_bursts() {
        const PIECE: u64 = 256 * 1024;
        const LINE: f64 = PIECE as f64; // one piece a second
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(1, 1 << 40);
        let meter = ProgressMeter::new(&sink, live.clone(), base(1 << 40));
        meter.status("running", None);
        live.set_stage(TransferStage::Uploading);
        let flight = live.begin(1 << 40);
        let t0 = Instant::now();
        // (from, to) of every stretch the line moves.
        let mut flows = Vec::new();
        let drive = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            flight.sent.fetch_add(4 * PIECE, Ordering::SeqCst);
            for (secs, stall) in [(20, 6), (10, 12), (10, 0)] {
                let from = Instant::now();
                for _ in 0..secs {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    flight.sent.fetch_add(PIECE, Ordering::SeqCst);
                }
                flows.push((from, Instant::now()));
                tokio::time::sleep(Duration::from_secs(stall)).await;
            }
        };
        tokio::select! {
            _ = drive => {}
            never = meter.run() => match never {},
        }
        let events = sink.0.lock().unwrap().clone();
        let at = |t: Instant| (t - t0).as_secs_f64();
        let told: Vec<(f64, u64)> = events.iter().map(|(t, p)| (at(*t), p.rate_bps)).collect();
        // Nothing before two seconds of movement: the burst is no speed.
        assert!(told.iter().filter(|(t, _)| *t < 2.0).all(|(_, r)| *r == 0), "{told:?}");
        for (from, to) in &flows {
            // Settled from three seconds after the line moves again.
            for (t, p) in events.iter().filter(|(t, _)| *t >= *from + Duration::from_secs(3) && *t <= *to) {
                let r = p.rate_bps as f64;
                assert!((LINE / 1.5..=1.5 * LINE).contains(&r), "{r} B/s at {:.2}s: {told:?}", at(*t));
            }
            // Never above the line, before it settled either.
            for (t, p) in events.iter().filter(|(t, _)| *t >= *from && *t <= *to) {
                assert!(p.rate_bps as f64 <= 1.5 * LINE, "{} B/s at {:.2}s: {told:?}", p.rate_bps, at(*t));
            }
        }
        // Six seconds of stall fade the speed; twelve end it.
        let (_, end_first) = flows[0];
        let faded = events.iter().rev().find(|(t, _)| *t > end_first + Duration::from_secs(5) && *t < flows[1].0).unwrap();
        assert!((faded.1.rate_bps as f64) < LINE / 2.0 && faded.1.eta_secs.is_none(), "{:?}", faded.1);
        let (_, end_second) = flows[1];
        let ended = events.iter().rev().find(|(t, _)| *t > end_second + Duration::from_millis(10_300) && *t < flows[2].0).unwrap();
        assert_eq!((ended.1.rate_bps, ended.1.eta_secs), (0, None));
    }

    /// An earlier run's word holds until this one knows better; a new key
    /// voids it at once.
    #[tokio::test(start_paused = true)]
    async fn a_run_that_starts_over_drops_what_the_last_one_got() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(10, 1000);
        let mut b = base(10_000);
        (b.done_bytes, b.chunks_done) = (4_000, 4);
        let meter = ProgressMeter::new(&sink, live.clone(), b);
        meter.status("running", None);
        live.set_stage(TransferStage::Checking);
        meter.poll();
        assert_eq!(sink.0.lock().unwrap().last().unwrap().1.done_bytes, 4_000, "the last run's word while checking");
        // The file changed: a new key, nothing of it is stored.
        live.start_over(10, 1000);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        let last = sink.0.lock().unwrap().last().unwrap().1.clone();
        assert_eq!((last.done_bytes, last.chunks_done), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn bytes_in_flight_count_partly_and_never_go_back() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(1, 10_000);
        let meter = ProgressMeter::new(&sink, live.clone(), base(10_000));
        meter.status("running", None);
        let flight = live.begin(10_000);
        flight.sent.store(10_016, Ordering::SeqCst);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        assert_eq!(sink.0.lock().unwrap().last().unwrap().1.done_bytes, 9_500, "95% until confirmed");
        // The request failed and starts again: what was told stays.
        drop(flight);
        let again = live.begin(10_000);
        again.sent.store(100, Ordering::SeqCst);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        meter.status("waiting_retry", Some("err.network"));
        let last = sink.0.lock().unwrap().last().unwrap().1.clone();
        assert_eq!((last.done_bytes, last.rate_bps, last.eta_secs), (9_500, 0, None));
        again.confirm(0);
        meter.done(Some("/x".into()));
        let last = sink.0.lock().unwrap().last().unwrap().1.clone();
        assert_eq!((last.status.as_str(), last.done_bytes, last.local_path.as_deref()), ("done", 10_000, Some("/x")));
    }

    /// A run that ends tells what is stored: the share of the requests it
    /// dropped goes, so the next run starts from the same number. A cancel
    /// removes it all, and tells nothing.
    #[tokio::test(start_paused = true)]
    async fn a_run_that_ends_tells_only_what_is_stored() {
        for (end, stored, chunks) in [("paused", 10_000, 1), ("failed", 10_000, 1), ("cancelled", 0, 0)] {
            let sink = Timed::default();
            let live = Arc::new(Live::default());
            live.start(2, 10_000);
            let meter = ProgressMeter::new(&sink, live.clone(), base(20_000));
            meter.status("running", None);
            live.begin(10_000).confirm(0);
            let flight = live.begin(10_000);
            flight.sent.store(5_000, Ordering::SeqCst);
            tokio::time::advance(THROTTLE).await;
            meter.poll();
            assert_eq!(sink.0.lock().unwrap().last().unwrap().1.done_bytes, 15_000);
            drop(flight);
            meter.status(end, None);
            let last = sink.0.lock().unwrap().last().unwrap().1.clone();
            assert_eq!((last.done_bytes, last.chunks_done), (stored, chunks), "{end}: the dropped request counts for nothing");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn found_chunks_count_once_and_never_as_speed() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(100, 1000);
        let meter = ProgressMeter::new(&sink, live.clone(), base(100_000));
        meter.status("running", None);
        live.set_stage(TransferStage::Checking);

        // Sixty chunks are there already and are found at once, then forty
        // move at 10 kB/s.
        let drive = async {
            for i in 0..60 {
                live.found(i, 1000);
            }
            live.set_stage(TransferStage::Uploading);
            for i in 60..100 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                live.begin(1000).confirm(i);
            }
        };
        tokio::select! {
            _ = drive => {}
            never = meter.run() => match never {},
        }
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        let events = sink.0.lock().unwrap().clone();
        let most = events.iter().map(|(_, p)| p.rate_bps).max().unwrap();
        assert!((5_000..=15_000).contains(&most), "the speed is what moved: {most}");
        assert_eq!(events.last().unwrap().1.done_bytes, 100_000, "found chunks count as done");

        // A second pass over the same file counts nothing twice.
        live.start(100, 1000);
        for i in 0..100 {
            live.found(i, 1000);
        }
        assert_eq!((live.marked(), live.confirmed(), live.moved()), (100, 100_000, 40_000));
        // Another shape of the file (a new key) counts from nothing.
        live.start(50, 2000);
        assert_eq!((live.marked(), live.confirmed()), (0, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_resumed_run_starts_where_the_last_stopped_and_chunks_never_go_back() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(10, 1000);
        // An earlier run stopped at chunk 4 of 10.
        let mut b = base(10_000);
        (b.done_bytes, b.chunks_done) = (4_000, 4);
        let meter = ProgressMeter::new(&sink, live.clone(), b);
        meter.status("queued", None);
        meter.status("running", None);
        let first: Vec<_> = sink.0.lock().unwrap().iter().map(|(_, p)| (p.done_bytes, p.chunks_done, p.chunks_total)).collect();
        assert_eq!(first, vec![(4_000, 4, 10), (4_000, 4, 10)], "not back to 0 of 0");

        // Checking counts from 0; the bar keeps the last run's word meanwhile.
        live.set_stage(TransferStage::Checking);
        live.found(0, 1000);
        meter.poll();
        live.found(1, 1000);
        tokio::time::advance(HEARTBEAT).await;
        meter.poll();
        let events = sink.0.lock().unwrap().clone();
        let checking: Vec<_> =
            events.iter().filter(|(_, p)| p.stage == TransferStage::Checking).map(|(_, p)| (p.chunks_done, p.done_bytes)).collect();
        assert_eq!(checking, vec![(1, 4_000), (2, 4_000)], "chunks never back within a stage");

        // Past checking the run knows: two chunks are there, not four.
        live.set_stage(TransferStage::Uploading);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        let last = sink.0.lock().unwrap().last().unwrap().1.clone();
        assert_eq!((last.chunks_done, last.done_bytes), (2, 2_000), "the bar and the count agree");
        live.begin(1000).confirm(2);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        assert_eq!(sink.0.lock().unwrap().last().unwrap().1.done_bytes, 3_000);

        // A later pass finds chunk 1 no longer stored (another server):
        // the counts go down with it, once.
        live.unmark(1);
        tokio::time::advance(THROTTLE).await;
        meter.poll();
        let last = sink.0.lock().unwrap().last().unwrap().1.clone();
        assert_eq!((last.chunks_done, last.done_bytes), (2, 2_000));
    }

    /// A steady line to the end: time left is told while five seconds or
    /// more remain, and no longer once fewer do, the speed still there.
    #[tokio::test(start_paused = true)]
    async fn time_left_is_not_told_once_fewer_than_five_seconds_remain() {
        let sink = Timed::default();
        let live = Arc::new(Live::default());
        live.start(1000, 1000);
        let meter = ProgressMeter::new(&sink, live.clone(), base(1_000_000));
        meter.status("running", None);
        live.set_stage(TransferStage::Uploading);
        // 100 kB/s in 1 kB chunks: ten seconds in all.
        let drive = async {
            for i in 0..1000 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                live.begin(1000).confirm(i);
            }
        };
        tokio::select! {
            _ = drive => {}
            never = meter.run() => match never {},
        }
        let events = sink.0.lock().unwrap().clone();
        let flowing: Vec<_> = events.iter().filter(|(_, p)| p.rate_bps > 0 && p.chunks_done < 1000).map(|(_, p)| p).collect();
        let (mut told, mut not_told) = (0, 0);
        for p in flowing {
            let left = (p.total_bytes - p.done_bytes) / p.rate_bps;
            if left >= ETA_MIN_SECS {
                assert_eq!(p.eta_secs, Some(left), "{p:?}");
                told += 1;
            } else {
                assert_eq!(p.eta_secs, None, "{left} s left is not told: {p:?}");
                not_told += 1;
            }
        }
        assert!(told > 0 && not_told > 0, "both sides of five seconds were seen: {told} and {not_told}");
    }

    /// The message goes out (publishing) or the file is joined from disk
    /// (assembling): no speed of the line before, and none after either.
    #[tokio::test(start_paused = true)]
    async fn stages_that_move_nothing_tell_no_speed() {
        for quiet in [TransferStage::Publishing, TransferStage::Assembling, TransferStage::Verifying] {
            let sink = Timed::default();
            let live = Arc::new(Live::default());
            live.start(1000, 1000);
            let meter = ProgressMeter::new(&sink, live.clone(), base(1_000_000));
            meter.status("running", None);
            live.set_stage(TransferStage::Uploading);
            let drive = async {
                for i in 0..400 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    live.begin(1000).confirm(i);
                }
            };
            tokio::select! {
                _ = drive => {}
                never = meter.run() => match never {},
            }
            assert!(sink.0.lock().unwrap().last().unwrap().1.rate_bps > 0, "a speed while the bytes flowed");

            live.set_stage(quiet);
            meter.poll();
            let last = sink.0.lock().unwrap().last().unwrap().1.clone();
            assert_eq!((last.stage, last.rate_bps, last.eta_secs), (quiet, 0, None));
            tokio::time::advance(HEARTBEAT).await;
            meter.poll();
            let last = sink.0.lock().unwrap().last().unwrap().1.clone();
            assert_eq!((last.stage, last.rate_bps, last.eta_secs), (quiet, 0, None), "a heartbeat later");
            // Moving again, the old speed does not come back.
            live.set_stage(TransferStage::Uploading);
            meter.poll();
            let last = sink.0.lock().unwrap().last().unwrap().1.clone();
            assert_eq!((last.stage, last.rate_bps), (TransferStage::Uploading, 0));
        }
    }

    #[test]
    fn stages_serialize_in_snake_case() {
        assert_eq!(serde_json::to_string(&TransferStage::Assembling).unwrap(), "\"assembling\"");
        for s in TransferStage::ALL {
            assert_eq!(TransferStage::from_u8(s as u8), s);
        }
    }
}

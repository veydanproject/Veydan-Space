// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Run, pause and cancel of one transfer, and what its workers share.
//!
//! The word of the user is seen at once by everything that works on the
//! transfer: a pause drops the requests in flight instead of waiting for
//! the chunk to end. Chunks finished before stay finished.

use crate::progress::Live;
use messenger_core::{MessengerError, Result};
use std::sync::{Arc, Mutex};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

pub const RUN: u8 = 0;
pub const PAUSE: u8 = 1;
pub const CANCEL: u8 = 2;
/// The end of the service pauses the run: an interruption, not a pause of
/// the user. It goes on by itself once the service runs again.
pub const INTERRUPT: u8 = 3;

/// Whether `word` pauses a run: the user's pause or an interruption.
pub fn pauses(word: u8) -> bool {
    word == PAUSE || word == INTERRUPT
}

/// Chunks of one transfer in flight at once. A phone has less to spare.
pub const WORKERS: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 2 } else { 4 };
/// Chunk requests of all transfers together, so several big files never
/// open more than this many at once.
pub const CHUNK_SLOTS: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 3 } else { 6 };
/// Chunk slots large transfers never take: a photo sent beside a gigabyte
/// starts at once instead of waiting for its chunks.
pub const SMALL_RESERVED: usize = if cfg!(any(target_os = "android", target_os = "ios")) { 1 } else { 2 };

/// Run, pause or cancel, and "retry now", of one transfer.
#[derive(Clone)]
pub struct Control {
    state: Arc<watch::Sender<u8>>,
    /// Bumped by "retry now": ends the wait before an automatic retry.
    wake: Arc<watch::Sender<u64>>,
    /// The run has decided how it ends: no word reaches it any more.
    closed: Arc<Mutex<bool>>,
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    pub fn new() -> Self {
        Self { state: Arc::new(watch::channel(RUN).0), wake: Arc::new(watch::channel(0).0), closed: Arc::default() }
    }

    /// The very same control (not a copy of its state).
    pub fn same(&self, other: &Control) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    pub fn get(&self) -> u8 {
        *self.state.borrow()
    }

    pub fn set(&self, v: u8) {
        self.state.send_replace(v);
    }

    fn closed(&self) -> std::sync::MutexGuard<'_, bool> {
        self.closed.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The word of the user: whether the run still hears it. Once it is
    /// closed the run has decided how it ends, and nothing is set. A cancel
    /// is final: no later word takes its place. An interruption never takes
    /// the place of a word that came first: a pause the user asked for
    /// stays the user's.
    pub fn ask(&self, v: u8) -> bool {
        let closed = self.closed();
        let now = self.get();
        let kept = now == CANCEL || (v == INTERRUPT && now != RUN);
        if !*closed && !kept {
            self.set(v);
        }
        !*closed
    }

    /// The run decides how it ends: no word reaches it from now on. The
    /// last word asked, which that end must heed.
    pub fn close(&self) -> u8 {
        let mut closed = self.closed();
        *closed = true;
        self.get()
    }

    pub fn is_closed(&self) -> bool {
        *self.closed()
    }

    /// Resolves with `PAUSE`, `INTERRUPT` or `CANCEL` as soon as one is
    /// asked; never while the transfer may run.
    pub async fn stopped(&self) -> u8 {
        let mut rx = self.state.subscribe();
        loop {
            let v = *rx.borrow_and_update();
            if v != RUN {
                return v;
            }
            // The sender lives as long as `self`: `changed` never fails here.
            if rx.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }

    /// End a wait before an automatic retry now.
    pub fn wake(&self) {
        self.wake.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Wakes from now on; `changed()` resolves on the next one.
    pub fn wakes(&self) -> watch::Receiver<u64> {
        let mut rx = self.wake.subscribe();
        rx.mark_unchanged();
        rx
    }
}

/// What a running transfer shares with the service: its control, the
/// chunk slots of all transfers, how many chunks it may move at once, and
/// the counts the progress meter reads.
#[derive(Clone)]
pub struct TransferCtx {
    pub control: Control,
    pub slots: Arc<Semaphore>,
    /// The share of `slots` its lane may hold at once, when it has one: a
    /// large transfer leaves `SMALL_RESERVED` of them to small ones.
    pub lane: Option<Arc<Semaphore>>,
    pub workers: usize,
    pub live: Arc<Live>,
}

/// One chunk request under way: a slot of all transfers, and of its lane.
pub struct Slot {
    _lane: Option<OwnedSemaphorePermit>,
    _shared: OwnedSemaphorePermit,
}

impl TransferCtx {
    /// Alone, with its own slots (tests and tools).
    pub fn standalone(workers: usize) -> Self {
        Self {
            control: Control::new(),
            slots: Arc::new(Semaphore::new(CHUNK_SLOTS)),
            lane: None,
            workers: workers.max(1),
            live: Arc::new(Live::default()),
        }
    }

    /// A slot for one chunk request: first of the lane's share, then of
    /// all transfers, so a large transfer never queues for more of the
    /// shared ones than its share.
    pub async fn slot(&self) -> Result<Slot> {
        let gone = |e: tokio::sync::AcquireError| MessengerError::Other(e.to_string());
        let lane = match &self.lane {
            Some(l) => Some(l.clone().acquire_owned().await.map_err(gone)?),
            None => None,
        };
        let shared = self.slots.clone().acquire_owned().await.map_err(gone)?;
        Ok(Slot { _lane: lane, _shared: shared })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn stop_is_seen_at_once_and_wakes_are_not_stale() {
        let c = Control::new();
        assert!(tokio::time::timeout(Duration::from_millis(20), c.stopped()).await.is_err(), "running: no stop");
        let waiter = tokio::spawn({
            let c = c.clone();
            async move { c.stopped().await }
        });
        tokio::task::yield_now().await;
        c.set(PAUSE);
        assert_eq!(waiter.await.unwrap(), PAUSE);
        c.set(RUN);
        c.set(CANCEL);
        assert_eq!(c.stopped().await, CANCEL);

        c.wake();
        let mut w = c.wakes();
        assert!(tokio::time::timeout(Duration::from_millis(20), w.changed()).await.is_err(), "an earlier wake is not kept");
        c.wake();
        w.changed().await.unwrap();
    }

    #[test]
    fn a_closed_control_hears_nothing_and_tells_the_last_word() {
        let c = Control::new();
        assert!(c.ask(PAUSE));
        assert!(c.ask(CANCEL));
        assert!(c.ask(PAUSE), "still heard");
        assert_eq!(c.get(), CANCEL, "a pause never takes the place of a cancel");
        assert_eq!(c.close(), CANCEL, "the end heeds the cancel");
        assert!(!c.ask(RUN), "a word after the end is not heard");
        assert_eq!((c.get(), c.is_closed()), (CANCEL, true));
    }

    /// The end of the service comes after a pause of the user: the pause
    /// stays the user's. Before any word it interrupts, and a pause or a
    /// cancel of the user still takes its place.
    #[test]
    fn an_interruption_never_takes_the_place_of_the_users_word() {
        let c = Control::new();
        assert!(c.ask(PAUSE));
        assert!(c.ask(INTERRUPT));
        assert_eq!(c.close(), PAUSE);

        let c = Control::new();
        assert!(c.ask(INTERRUPT));
        assert_eq!(c.get(), INTERRUPT);
        assert!(c.ask(PAUSE));
        assert_eq!(c.get(), PAUSE);
        assert!(c.ask(CANCEL));
        assert!(c.ask(INTERRUPT));
        assert_eq!(c.close(), CANCEL);
        assert!(pauses(PAUSE) && pauses(INTERRUPT) && !pauses(CANCEL) && !pauses(RUN));
    }
}

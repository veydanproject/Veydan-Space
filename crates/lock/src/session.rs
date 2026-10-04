// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The open session: when the user was last active, what the session was
//! opened with, how many wrong secrets came in a row.

use crate::vault::LockMeta;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Wrong attempts in a row before each further one waits.
const FAIL_FREE_ATTEMPTS: u32 = 3;
const FAIL_DELAY: Duration = Duration::from_secs(1);

/// Last user activity while unlocked; `None` means locked.
#[derive(Default)]
pub(crate) struct Session {
    last_activity: Mutex<Option<Instant>>,
    failed_attempts: Mutex<u32>,
    /// The lock the open session was verified against; a synced change of
    /// its hash relocks. Also what a key row made in this session names.
    meta: Mutex<Option<LockMeta>>,
}

impl Session {
    /// Open, and active within the last `timeout_min` minutes (0: no timeout).
    pub(crate) fn is_unlocked(&self, timeout_min: u32) -> bool {
        match *self.last_activity.lock().unwrap() {
            None => false,
            Some(_) if timeout_min == 0 => true,
            Some(t) => t.elapsed() < Duration::from_secs(u64::from(timeout_min) * 60),
        }
    }

    /// Open, however long ago the user was last active.
    pub(crate) fn is_open(&self) -> bool {
        self.last_activity.lock().unwrap().is_some()
    }

    pub(crate) fn touch(&self) {
        *self.last_activity.lock().unwrap() = Some(Instant::now());
        *self.failed_attempts.lock().unwrap() = 0;
    }

    /// Unlocked with `meta`; its hash is remembered for the relock check.
    pub(crate) fn open(&self, meta: LockMeta) {
        *self.meta.lock().unwrap() = Some(meta);
        self.touch();
    }

    pub(crate) fn clear(&self) {
        *self.last_activity.lock().unwrap() = None;
        *self.meta.lock().unwrap() = None;
    }

    pub(crate) fn meta(&self) -> Option<LockMeta> {
        self.meta.lock().unwrap().clone()
    }

    pub(crate) fn hash(&self) -> Option<String> {
        self.meta.lock().unwrap().as_ref().map(|m| m.hash.clone())
    }

    /// Slows brute force: after a few wrong secrets each attempt pauses first.
    pub(crate) async fn throttle(&self) {
        let failed = *self.failed_attempts.lock().unwrap();
        if failed >= FAIL_FREE_ATTEMPTS {
            tokio::time::sleep(FAIL_DELAY).await;
        }
    }

    pub(crate) fn record_failure(&self) {
        let mut failed = self.failed_attempts.lock().unwrap();
        *failed = failed.saturating_add(1);
    }

    /// The user was last active `ago` before now.
    #[cfg(test)]
    pub(crate) fn idle_for(&self, ago: Duration) {
        let mut last = self.last_activity.lock().unwrap();
        if last.is_some() {
            *last = Instant::now().checked_sub(ago);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(hash: &str) -> LockMeta {
        LockMeta {
            hash: hash.into(),
            kind: None,
            hint: None,
        }
    }

    #[test]
    fn a_session_ends_after_the_timeout_and_never_without_one() {
        let session = Session::default();
        assert!(!session.is_unlocked(0));
        session.open(meta("h"));
        assert!(session.is_unlocked(5));
        session.idle_for(Duration::from_secs(6 * 60));
        assert!(!session.is_unlocked(5));
        assert!(session.is_unlocked(0));
        assert!(session.is_open());
        session.clear();
        assert!(!session.is_open());
        assert_eq!(session.hash(), None);
    }
}

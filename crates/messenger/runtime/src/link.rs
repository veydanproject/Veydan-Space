// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the app tells the user about its connection: a promise, not a
//! reading. It says "connected" from the start and after a wake, and takes
//! it back only when no relay has been connected for a while: a drop of a
//! few seconds, or the time the relays take to come back, is never shown.
//!
//! - `ok`: connected, or not long without;
//! - `waiting`: no relay for `WAITING_AFTER`: connecting;
//! - `lost`: no relay for `LOST_AFTER`: no connection.
//!
//! Where the truth matters (pushes, the checks of the way to the servers)
//! the count of connected relays is read, never this.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const WAITING_AFTER: Duration = Duration::from_secs(10);
pub const LOST_AFTER: Duration = Duration::from_secs(60);
/// After a start or a wake, messages that ran out of time meanwhile are
/// not given up for this long: the relays are only coming back.
pub const HOLD_EXPIRY_SECS: i64 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    #[default]
    Ok,
    Waiting,
    Lost,
}

/// What one look found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observation {
    pub state: LinkState,
    /// Differs from what was told before.
    pub changed: bool,
    /// A relay is connected now and none was at the last look.
    pub came_up: bool,
}

#[derive(Default)]
struct Inner {
    down_since: Option<Instant>,
    told: LinkState,
    was_up: bool,
}

#[derive(Default)]
pub struct LinkWatch {
    inner: Mutex<Inner>,
}

impl LinkWatch {
    /// `up`: a relay is connected. `expected`: one should be (a session
    /// runs and the relays are not silenced); when none should, the app
    /// shows why it is quiet, and this says nothing against it.
    pub fn observe(&self, now: Instant, up: bool, expected: bool) -> Observation {
        let mut inner = self.inner.lock().expect("link");
        let came_up = up && !inner.was_up;
        inner.was_up = up;
        let state = if up || !expected {
            inner.down_since = None;
            LinkState::Ok
        } else {
            let since = *inner.down_since.get_or_insert(now);
            match now.saturating_duration_since(since) {
                d if d < WAITING_AFTER => LinkState::Ok,
                d if d < LOST_AFTER => LinkState::Waiting,
                _ => LinkState::Lost,
            }
        };
        let changed = state != inner.told;
        inner.told = state;
        Observation { state, changed, came_up }
    }

    /// The last thing told.
    pub fn state(&self) -> LinkState {
        self.inner.lock().expect("link").told
    }

    /// A start or a wake: the time without relays counts from now.
    pub fn reset(&self) {
        self.inner.lock().expect("link").down_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn connected_from_the_start_and_short_drops_unseen() {
        let w = LinkWatch::default();
        let t0 = Instant::now();
        // The app opens with no relay yet: still "connected".
        assert_eq!(w.observe(t0, false, true).state, LinkState::Ok);
        assert_eq!(w.observe(t0 + secs(9), false, true).state, LinkState::Ok);
        let up = w.observe(t0 + secs(9), true, true);
        assert_eq!((up.state, up.changed, up.came_up), (LinkState::Ok, false, true));
        // Drops of five seconds, again and again: never shown.
        for i in 0..10 {
            let at = t0 + secs(10 + i * 10);
            assert_eq!(w.observe(at, false, true).state, LinkState::Ok);
            assert_eq!(w.observe(at + secs(5), false, true).state, LinkState::Ok);
            assert_eq!(w.observe(at + secs(6), true, true).state, LinkState::Ok);
        }
    }

    #[test]
    fn waiting_after_ten_seconds_lost_after_a_minute() {
        let w = LinkWatch::default();
        let t0 = Instant::now();
        w.observe(t0, false, true);
        let waiting = w.observe(t0 + WAITING_AFTER, false, true);
        assert_eq!((waiting.state, waiting.changed), (LinkState::Waiting, true));
        assert!(!w.observe(t0 + secs(30), false, true).changed, "told once");
        let lost = w.observe(t0 + LOST_AFTER, false, true);
        assert_eq!((lost.state, lost.changed), (LinkState::Lost, true));
        assert_eq!(w.state(), LinkState::Lost);
        let back = w.observe(t0 + secs(61), true, true);
        assert_eq!((back.state, back.changed, back.came_up), (LinkState::Ok, true, true));
    }

    #[test]
    fn a_wake_starts_the_count_over() {
        let w = LinkWatch::default();
        let t0 = Instant::now();
        w.observe(t0, false, true);
        w.reset();
        assert_eq!(w.observe(t0 + secs(20), false, true).state, LinkState::Ok, "counted from the wake");
        assert_eq!(w.observe(t0 + secs(30), false, true).state, LinkState::Waiting);
    }

    #[test]
    fn quiet_on_purpose_is_not_lost() {
        let w = LinkWatch::default();
        let t0 = Instant::now();
        w.observe(t0, false, true);
        assert_eq!(w.observe(t0 + secs(120), false, false).state, LinkState::Ok, "silenced or locked");
        assert_eq!(w.observe(t0 + secs(125), false, true).state, LinkState::Ok, "and the count starts over");
    }
}

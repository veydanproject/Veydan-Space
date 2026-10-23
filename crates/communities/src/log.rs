// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The terminal of the module: every `communities: …` line goes through
//! `clog!`, which puts the local time before it (on a desktop the terminal
//! stamps nothing, and a measurement needs times; Android's logcat stamps
//! its lines itself), and the commands that took long say how long.

use std::time::{Duration, Instant};

/// A command slower than this says so on the terminal.
const SLOW: Duration = Duration::from_millis(300);

/// `HH:MM:SS.mmm ` of the local time; empty on Android, whose log stamps.
pub(crate) fn stamp() -> String {
    #[cfg(target_os = "android")]
    {
        String::new()
    }
    #[cfg(not(target_os = "android"))]
    {
        format!("{} ", chrono::Local::now().format("%H:%M:%S%.3f"))
    }
}

/// One line of the module for the terminal: `<time> communities: <line>`.
macro_rules! clog {
    ($($arg:tt)*) => {
        eprintln!("{}communities: {}", $crate::log::stamp(), format_args!($($arg)*))
    };
}
pub(crate) use clog;

/// Held by a command from its first line to its end (or to the moment its
/// future is dropped): said as `communities: <command>: N ms` when it took
/// more than 300 ms.
pub(crate) struct Timed {
    command: &'static str,
    start: Instant,
}

impl Timed {
    pub(crate) fn start(command: &'static str) -> Self {
        Self { command, start: Instant::now() }
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        let took = self.start.elapsed();
        if took >= SLOW {
            clog!("{}: {} ms", self.command, took.as_millis());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_is_a_time_of_day() {
        let s = stamp();
        #[cfg(not(target_os = "android"))]
        {
            let b = s.as_bytes();
            assert_eq!(s.len(), 13, "{s:?}");
            assert!(b[2] == b':' && b[5] == b':' && b[8] == b'.' && b[12] == b' ', "{s:?}");
        }
        #[cfg(target_os = "android")]
        assert!(s.is_empty());
    }

    #[test]
    fn a_slow_command_is_timed_and_a_quick_one_is_not() {
        // Nothing to assert on the terminal: the guard must simply not panic either way.
        drop(Timed::start("quick"));
        let slow = Timed { command: "slow", start: Instant::now() - Duration::from_millis(400) };
        drop(slow);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Hybrid logical clock: total order across devices regardless of wall-clock skew.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Field order matters: derived `Ord` compares wall, then counter, then device.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    pub device_id: String,
}

impl Hlc {
    /// Sortable string: lexicographic order equals logical order.
    pub fn encode(&self) -> String {
        format!(
            "{:016x}-{:08x}-{}",
            self.wall_ms, self.counter, self.device_id
        )
    }

    pub fn decode(s: &str) -> Option<Hlc> {
        let mut parts = s.splitn(3, '-');
        let wall_ms = u64::from_str_radix(parts.next()?, 16).ok()?;
        let counter = u32::from_str_radix(parts.next()?, 16).ok()?;
        let device_id = parts.next()?.to_string();
        Some(Hlc {
            wall_ms,
            counter,
            device_id,
        })
    }
}

pub struct HlcClock {
    device_id: String,
    last_wall: u64,
    last_counter: u32,
}

fn wall_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl HlcClock {
    pub fn new(device_id: impl Into<String>, last: Option<&Hlc>) -> Self {
        Self {
            device_id: device_id.into(),
            last_wall: last.map(|h| h.wall_ms).unwrap_or(0),
            last_counter: last.map(|h| h.counter).unwrap_or(0),
        }
    }

    /// Next timestamp, strictly greater than everything issued or observed so far.
    pub fn now(&mut self) -> Hlc {
        let wall = wall_now_ms();
        if wall > self.last_wall {
            self.last_wall = wall;
            self.last_counter = 0;
        } else {
            self.last_counter += 1;
        }
        Hlc {
            wall_ms: self.last_wall,
            counter: self.last_counter,
            device_id: self.device_id.clone(),
        }
    }

    /// Advance past a remote timestamp so later local ops sort after it.
    pub fn observe(&mut self, remote: &Hlc) {
        if remote.wall_ms > self.last_wall
            || (remote.wall_ms == self.last_wall && remote.counter > self.last_counter)
        {
            self.last_wall = remote.wall_ms;
            self.last_counter = remote.counter;
        }
    }

    pub fn last(&self) -> Hlc {
        Hlc {
            wall_ms: self.last_wall,
            counter: self.last_counter,
            device_id: self.device_id.clone(),
        }
    }
}

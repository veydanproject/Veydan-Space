// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! When a catch-up has been read: a marker put into the host's stream of
//! events after the pages of a catch-up came (the pool hands them to that
//! stream before a `Sync` returns), heard back once the host read every
//! event before it. The host of the service gives the way into its stream
//! (`ChannelService::set_settle_sink`); without one nothing is known of
//! it, and `settle` says so (`None`).
//!
//! A marker is no event of a relay: its id is 64 zeros, which no signed
//! event has, and it names a token of this run; a relay that sent one
//! would have to guess the token, and would only end a wait early.

use crate::service::ChannelService;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::oneshot;

/// Puts a marker into the host's stream of the channels' events, as an
/// event of a catch-up (behind the pages a catch-up brought, not before
/// them). `false` when the stream is gone.
pub type SettleSink = Arc<dyn Fn(serde_json::Value) -> bool + Send + Sync>;

/// The field of a marker: its token.
pub const SETTLE_FIELD: &str = "veydan_settle";

/// The way into the host's stream and the markers on their way.
#[derive(Default)]
pub struct Settle {
    sink: RwLock<Option<SettleSink>>,
    waiting: Mutex<HashMap<u64, oneshot::Sender<()>>>,
}

/// The id of a marker: no signed event has it.
const ZEROS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// A marker of `token`, as the sink puts it into the stream.
pub fn marker(token: u64) -> serde_json::Value {
    serde_json::json!({ "id": ZEROS, "kind": 0, "created_at": 0, "tags": [], SETTLE_FIELD: token })
}

impl ChannelService {
    /// The way into the stream the host reads the channels' events from
    /// (the app's loop, the testkit's pump): `settle` works with it.
    pub fn set_settle_sink(&self, sink: Option<SettleSink>) {
        if let Ok(mut s) = self.settle.sink.write() {
            *s = sink;
        }
    }

    /// Waits until the host has read every event put into its stream before
    /// now, at most `max`: `Some(true)` when it has, `Some(false)` when the
    /// wait ran out, `None` when this host gives no way into its stream.
    pub async fn settle(&self, max: Duration) -> Option<bool> {
        let sink = self.settle.sink.read().ok().and_then(|s| s.clone())?;
        let token = getrandom::u64().ok()?;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut w) = self.settle.waiting.lock() {
            w.insert(token, tx);
        }
        let done = sink(marker(token)) && matches!(tokio::time::timeout(max, rx).await, Ok(Ok(())));
        if let Ok(mut w) = self.settle.waiting.lock() {
            w.remove(&token);
        }
        Some(done)
    }

    /// Whether `signed` is a marker: one of mine ends its wait; any is
    /// taken here and goes no further.
    pub(crate) fn settle_heard(&self, signed: &serde_json::Value) -> bool {
        if signed["id"].as_str() != Some(ZEROS) {
            return false;
        }
        let Some(token) = signed.get(SETTLE_FIELD).and_then(serde_json::Value::as_u64) else { return false };
        if let Some(tx) = self.settle.waiting.lock().ok().and_then(|mut w| w.remove(&token)) {
            let _ = tx.send(());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_has_no_id_of_an_event() {
        let m = marker(7);
        assert_eq!(m["id"].as_str().map(str::len), Some(64));
        assert_eq!(m[SETTLE_FIELD], 7);
    }
}

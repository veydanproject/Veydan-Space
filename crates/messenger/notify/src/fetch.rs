// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The one thing this crate asks the network: an event the push could not
//! carry, from the relay the push named, by id. A short question with a
//! short patience; the app fetches for real when it runs.
//!
//! The question is asked from a process a push just started, cold, with
//! the radio only now awake: the way to the relay (through a bridge, a hub
//! and the relay when the app goes that way) takes its time before any
//! answer can. So the connection is waited for first, on its own budget,
//! and the question is put once the relay listens; a relay that answers
//! nothing in time is asked once more while the budget lasts. The whole of
//! it stays within what the system gives a push to be handled in
//! (`BUDGET`), and the invitation of a call is good for 45 s from its
//! making: a push that took longer to come rings nothing anyway.
//!
//! In silent mode nothing is asked: the app reaches no relay then, and the
//! process a push started does not either. The push is told as one that
//! could not be had (ROO-1308).

use messenger_core::{MessengerError, RelayUrl, Result};
use messenger_store::{relays, settings, Store};
use messenger_transport::RelayConfig;
use nostr_sdk::prelude::*;
use std::time::{Duration, Instant};

/// How long the relay is given to take the connection.
pub const CONNECT: Duration = Duration::from_secs(6);
/// Through a bridge the way is longer: the bridge, a hub, then the relay.
pub const CONNECT_BRIDGED: Duration = Duration::from_secs(8);
/// How long one question waits for the relay's answer.
pub const PATIENCE: Duration = Duration::from_secs(5);
/// Connection and questions together: a push handler has some twenty
/// seconds before the system takes the process for stuck, and a call
/// rings now or never.
pub const BUDGET: Duration = Duration::from_secs(15);

/// The event as JSON, or None when the relay had nothing to say in time
/// (or said it has no such event: expired, or never there), and in silent
/// mode, where no relay is reached; an unreadable switch counts as on.
pub async fn event(store: &Store, relay: &str, event_id: &str) -> Result<Option<serde_json::Value>> {
    if !matches!(settings::get_bool(store, relays::KEY_SILENT, false).await, Ok(false)) {
        return Ok(None);
    }
    let url = RelayUrl::parse(relay).ok_or_else(|| MessengerError::Invalid("relay url".into()))?;
    // The relay may be one of mine with a gate key on it.
    let api_key = relays::list(store)
        .await?
        .into_iter()
        .find(|r| RelayUrl::parse(&r.url).as_ref() == Some(&url))
        .and_then(|r| r.auth_secret);
    let config = RelayConfig { url, read: true, write: false, api_key, server_id: None };
    let id = EventId::from_hex(event_id).map_err(|_| MessengerError::Invalid("event id".into()))?;

    messenger_transport::ensure_crypto_provider();
    // The way the app was using when it last ran; a push process has nobody
    // else to ask. Unreadable settings mean the direct way, as before.
    let bridged = crate::net::apply_saved(store).await.unwrap_or(false);
    let connect = if bridged { CONNECT_BRIDGED } else { CONNECT };
    let started = Instant::now();
    let client = Client::builder().websocket_transport(messenger_transport::BridgedTransport::default()).build();
    let address = config.connect_url();
    client.add_relay(address.as_str()).await.map_err(|e| MessengerError::Transport(e.to_string()))?;
    // The connection first, waited for: a question put to a relay that is
    // not there yet waits in a queue and its patience runs out on the
    // way, not on the relay. A connection that fails outright (refused,
    // no network) is reported at once, and the question is not put.
    if client.try_connect_relay(address.as_str(), connect).await.is_err() {
        client.disconnect().await;
        return Ok(None);
    }
    let mut found = None;
    while found.is_none() {
        let left = BUDGET.saturating_sub(started.elapsed());
        if left < Duration::from_secs(1) {
            break;
        }
        let patience = PATIENCE.min(left);
        let asked = client.fetch_events(Filter::new().id(id)).timeout(patience).max_events(1);
        match tokio::time::timeout(patience, asked).await {
            Ok(Ok(events)) => {
                // The relay answered: the event, or that it has none (gone
                // with its expiration, or never there). Asked no more.
                found = Some(events.into_iter().find(|e| e.id == id));
            }
            // No answer in time: once more, while the budget lasts.
            Err(_) => continue,
            // The relay would not take the question (the connection went
            // in between): a moment, then once more.
            Ok(Err(_)) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    client.disconnect().await;
    Ok(found.flatten().map(|e| serde_json::to_value(&e).expect("an event serializes")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::MessengerConfig;

    /// In silent mode the event is not fetched: no connection, so no
    /// question either. With silent mode off the relay is reached.
    #[tokio::test]
    async fn silent_mode_fetches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&MessengerConfig::new(dir.path().join("messenger"))).await.unwrap();
        // A relay that only counts who comes.
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", relay.local_addr().unwrap());
        let id = "ab".repeat(32);

        settings::set_bool(&store, relays::KEY_SILENT, true).await.unwrap();
        assert!(event(&store, &url, &id).await.unwrap().is_none());
        // The call is over and leaves nothing running, so a connection it
        // made is in the backlog of the listener by now: an accept that
        // does not block finds it at once, or there is none. No while of
        // quiet is waited for.
        let relay = relay.into_std().unwrap();
        let pending = relay.accept();
        assert!(matches!(&pending, Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "no connection in silent mode: {pending:?}");
        let relay = tokio::net::TcpListener::from_std(relay).unwrap();

        settings::set_bool(&store, relays::KEY_SILENT, false).await.unwrap();
        let came = tokio::select! {
            _ = event(&store, &url, &id) => false,
            accepted = tokio::time::timeout(CONNECT, relay.accept()) => accepted.is_ok(),
        };
        assert!(came, "reached with silent mode off");
        store.close().await;
    }
}

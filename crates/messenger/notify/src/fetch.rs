// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The one thing this crate asks the network: an event the push could not
//! carry, from the relay the push named, by id. A short question with a
//! short patience; the app fetches for real when it runs.

use messenger_core::{MessengerError, RelayUrl, Result};
use messenger_store::{relays, Store};
use messenger_transport::RelayConfig;
use nostr_sdk::prelude::*;
use std::time::Duration;

pub const PATIENCE: Duration = Duration::from_secs(5);
/// Through a bridge the way is longer: the bridge, a hub, then the relay.
pub const PATIENCE_BRIDGED: Duration = Duration::from_secs(8);

/// The event as JSON, or None when the relay had nothing to say in time.
pub async fn event(store: &Store, relay: &str, event_id: &str) -> Result<Option<serde_json::Value>> {
    let url = RelayUrl::parse(relay).ok_or_else(|| MessengerError::Invalid("relay url".into()))?;
    // The relay may be one of mine with a gate key on it.
    let api_key = relays::list(store)
        .await?
        .into_iter()
        .find(|r| RelayUrl::parse(&r.url).as_ref() == Some(&url))
        .and_then(|r| r.auth_secret);
    let config = RelayConfig { url, read: true, write: false, api_key };
    let id = EventId::from_hex(event_id).map_err(|_| MessengerError::Invalid("event id".into()))?;

    messenger_transport::ensure_crypto_provider();
    // The way the app was using when it last ran; a push process has nobody
    // else to ask. Unreadable settings mean the direct way, as before.
    let bridged = crate::net::apply_saved(store).await.unwrap_or(false);
    let patience = if bridged { PATIENCE_BRIDGED } else { PATIENCE };
    let client = Client::builder().websocket_transport(messenger_transport::BridgedTransport::default()).build();
    client.add_relay(config.connect_url()).await.map_err(|e| MessengerError::Transport(e.to_string()))?;
    client.connect().await;
    let found = tokio::time::timeout(patience, client.fetch_events(Filter::new().id(id)).timeout(patience).max_events(1)).await;
    client.disconnect().await;
    let events = match found {
        Ok(Ok(events)) => events,
        _ => return Ok(None),
    };
    Ok(events.into_iter().find(|e| e.id == id).map(|e| serde_json::to_value(&e).expect("an event serializes")))
}

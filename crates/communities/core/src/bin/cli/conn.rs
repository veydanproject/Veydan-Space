// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! One connection to the relay, over nostr-sdk as the messenger's pool
//! holds it (`crates/messenger/transport`, which the engine may not depend
//! on: scripts/boundaries.sh): the key of the relay as `?key=`, NIP-42
//! signed in with the given key, sent events kept out of the client's
//! store, every event of a live subscription told as it came.

use messenger_core::{Ack, MessengerError, RelayUrl, Result};
use nostr::key::Keys;
use nostr_sdk::prelude::{Client, ClientNotification, Event, Filter, RelayMessage, RelayStatus, SignerAuthenticator, SubscriptionId};
use serde_json::Value;
use std::time::Duration;

fn net(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Transport(e.to_string())
}

pub struct Conn {
    client: Client,
    relay: RelayUrl,
}

impl Conn {
    /// Opens the connection; `Ok` once the relay is connected.
    pub async fn open(url: &str, key: Option<&str>, signer: Option<Keys>, wait: Duration) -> Result<Self> {
        // The provider of TLS, as the messenger installs it.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let relay = RelayUrl::parse(url).ok_or_else(|| MessengerError::Invalid("relay url".into()))?;
        let address = match key {
            Some(k) if !k.is_empty() => format!("{}/?key={k}", relay.as_str()),
            _ => relay.as_str().to_string(),
        };
        let builder = Client::builder();
        let client = match signer {
            Some(keys) => builder.authenticator(SignerAuthenticator::new(keys)).build(),
            None => builder.build(),
        };
        client.add_relay(address.as_str()).await.map_err(net)?;
        client.connect().await;
        let conn = Self { client, relay };
        let deadline = tokio::time::Instant::now() + wait;
        while !conn.connected().await {
            if tokio::time::Instant::now() > deadline {
                conn.shutdown().await;
                return Err(net("the relay did not connect"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(conn)
    }

    pub async fn connected(&self) -> bool {
        self.client.relays().await.values().any(|r| r.status() == RelayStatus::Connected)
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }

    /// The events of the live subscriptions whose id starts with one of
    /// `prefixes`, as they come (not those of a fetch).
    pub fn forward(&self, prefixes: &'static [&'static str], mut to: impl FnMut(Value) -> bool + Send + 'static) {
        let mut stream = self.client.notifications();
        tokio::spawn(async move {
            use nostr_sdk::prelude::StreamExt;
            while let Some(n) = stream.next().await {
                match n {
                    ClientNotification::Message { message, .. } => {
                        let RelayMessage::Event { event, subscription_id } = *message else { continue };
                        if !prefixes.iter().any(|p| subscription_id.as_str().starts_with(p)) {
                            continue;
                        }
                        let Ok(json) = serde_json::to_value(event.as_ref()) else { continue };
                        if !to(json) {
                            break;
                        }
                    }
                    ClientNotification::Shutdown => break,
                    _ => {}
                }
            }
        });
    }

    fn filter(f: &Value) -> Result<Filter> {
        serde_json::from_value(f.clone()).map_err(|e| MessengerError::Invalid(format!("filter: {e}")))
    }

    pub async fn subscribe(&self, id: &str, filter: &Value) -> Result<()> {
        let f = Self::filter(filter)?;
        let _ = self.client.unsubscribe(&SubscriptionId::new(id)).await;
        self.client.subscribe(f).with_id(SubscriptionId::new(id)).await.map_err(net)?;
        Ok(())
    }

    /// One REQ, answered until EOSE or `patience`: the events, checked.
    pub async fn fetch(&self, filter: &Value, patience: Duration) -> Result<Vec<Value>> {
        let f = Self::filter(filter)?;
        let events = self.client.fetch_events(f).timeout(patience).await.map_err(net)?;
        Ok(events.iter().filter(|e| e.verify().is_ok()).filter_map(|e| serde_json::to_value(e).ok()).collect())
    }

    pub async fn publish(&self, json: &Value) -> Result<Ack> {
        let event: Event = serde_json::from_value(json.clone()).map_err(|e| MessengerError::Invalid(format!("event: {e}")))?;
        let out = self.client.send_event(&event).save_into_database(false).await.map_err(net)?;
        let accepted_by = if out.success.is_empty() { vec![] } else { vec![self.relay.clone()] };
        let rejected_by = out.failed.values().map(|why| (self.relay.clone(), why.clone())).collect();
        Ok(Ack { accepted_by, rejected_by })
    }
}

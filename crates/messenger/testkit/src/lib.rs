// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Test doubles shared by every messenger crate.
//!
//! - [`MemorySecretStore`]: `SecretStore` in a `Mutex<HashMap>` with a lock switch.
//! - [`FakeTransport`]: records every `Outbound`, lets tests inject `RawEvent`s.
//!
//! - [`FileSecretStore`]: plaintext JSON secrets for the CLI and standalone runs.
//! - [`FakeEngine`]: a media engine with no media, whose sessions connect
//!   to each other in the process (`messenger-calls`).
//! - [`FakeNode`] and [`FakeGroups`]: a call node with rooms and no
//!   network, and the groups a group call asks of (`messenger-calls`).
//! - [`open_dm`]: a gift wrap opened into the `DmInbound` ingress would
//!   make of it, for tests that pass wraps between parties.
//!
//! The `messenger-cli` binary (src/bin) drives `MessengerRuntime` headless.

pub mod fake_engine;
pub mod fake_node;
pub mod file_secrets;
pub use fake_engine::{FakeEngine, FakeHandle};
pub use fake_node::{FakeDevice, FakeGroups, FakeNode, FakeRegistry};
pub use file_secrets::FileSecretStore;

use async_trait::async_trait;
use messenger_core::inbound::Envelope as WireEnvelope;
use messenger_core::outbound::WireEvent;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, DmInbound, EventId, EventSource, MessengerError, Outbound, PubKey, RawEvent, RelayUrl, Result, SecretStore, Timestamp, Transport};
use nostr::key::Keys;
use nostr::nips::nip59::UnwrappedGift;
use nostr::prelude::Event;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

/// In-memory `SecretStore`. `locked()` makes every access fail with
/// `SecretsLocked`, mirroring a host whose vault is closed.
#[derive(Default)]
pub struct MemorySecretStore {
    unlocked: Mutex<bool>,
    map: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemorySecretStore {
    pub fn unlocked() -> Self {
        Self { unlocked: Mutex::new(true), map: Mutex::default() }
    }

    pub fn locked() -> Self {
        Self { unlocked: Mutex::new(false), map: Mutex::default() }
    }

    pub fn set_unlocked(&self, on: bool) {
        *self.unlocked.lock().unwrap() = on;
    }

    fn guard(&self) -> Result<()> {
        if *self.unlocked.lock().unwrap() {
            Ok(())
        } else {
            Err(MessengerError::SecretsLocked)
        }
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        self.guard()?;
        Ok(self.map.lock().unwrap().get(key).cloned().map(Zeroizing::new))
    }

    async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        self.guard()?;
        self.map.lock().unwrap().insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.guard()?;
        self.map.lock().unwrap().remove(key);
        Ok(())
    }

    async fn is_unlocked(&self) -> bool {
        *self.unlocked.lock().unwrap()
    }
}

/// The recipients a wrap is addressed to (its `p` tags), hex.
pub fn wrap_recipients(event: &WireEvent) -> Vec<PubKey> {
    let Ok(ev) = serde_json::from_value::<Event>(event.json.clone()) else { return vec![] };
    ev.tags.iter().filter(|t| t.kind() == "p").filter_map(|t| t.as_slice().get(1)).filter_map(|s| PubKey::parse(s)).collect()
}

/// Open a gift wrap with `keys` into what ingress would hand the DM
/// handlers. `None` when the wrap is not addressed to these keys.
pub fn open_dm(keys: &Keys, event: &WireEvent, via_sync: bool) -> Option<DmInbound> {
    let ev: Event = serde_json::from_value(event.json.clone()).ok()?;
    let u = UnwrappedGift::from_gift_wrap(keys, &ev).ok()?;
    let mut rumor = u.rumor.clone();
    rumor.ensure_id();
    let url = RelayUrl::parse("wss://r.example").unwrap();
    Some(DmInbound {
        envelope: WireEnvelope {
            wire_id: event.id.clone(),
            source: if via_sync { EventSource::Sync { url } } else { EventSource::Relay { url } },
            wire_created_at: Timestamp(ev.created_at.as_secs() as i64),
        },
        rumor_id: EventId::parse(&rumor.id?.to_hex())?,
        sender: PubKey::parse(&u.sender.to_hex())?,
        recipients: rumor
            .tags
            .iter()
            .filter(|t| t.kind() == "p")
            .filter_map(|t| t.as_slice().get(1))
            .filter_map(|s| PubKey::parse(s))
            .collect(),
        created_at: Timestamp(rumor.created_at.as_secs() as i64),
        content: rumor.content.clone(),
        reply_to: rumor.tags.iter().filter(|t| t.kind() == "e").filter_map(|t| t.as_slice().get(1)).filter_map(|s| EventId::parse(s)).next(),
        rumor_kind: rumor.kind.as_u16(),
    })
}

/// Records outbound traffic and feeds inbound events on demand.
pub struct FakeTransport {
    sent: Mutex<Vec<Outbound>>,
    tx: mpsc::Sender<RawEvent>,
    rx: Mutex<Option<mpsc::Receiver<RawEvent>>>,
}

impl Default for FakeTransport {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self { sent: Mutex::new(Vec::new()), tx, rx: Mutex::new(Some(rx)) }
    }
}

impl FakeTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything handlers asked the transport to do, in order.
    pub fn sent(&self) -> Vec<Outbound> {
        self.sent.lock().unwrap().clone()
    }

    /// Inject an event as if a relay delivered it.
    pub async fn inject(&self, event: RawEvent) {
        self.tx.send(event).await.expect("runtime dropped the inbound receiver");
    }
}

#[async_trait]
impl Transport for FakeTransport {
    async fn send(&self, out: Outbound) -> Result<Ack> {
        self.sent.lock().unwrap().push(out);
        Ok(Ack { accepted_by: vec![], rejected_by: vec![] })
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        self.rx.lock().unwrap().take().expect("events() may be called once")
    }

    async fn status(&self) -> RelayStatusSnapshot {
        RelayStatusSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::{EventId, EventSource, PubKey, RelayUrl, Timestamp};

    #[tokio::test]
    async fn memory_secret_store_respects_lock() {
        let s = MemorySecretStore::locked();
        assert!(matches!(s.get("k").await, Err(MessengerError::SecretsLocked)));
        s.set_unlocked(true);
        s.put("k", b"v").await.unwrap();
        assert_eq!(s.get("k").await.unwrap().unwrap().as_slice(), b"v");
        s.delete("k").await.unwrap();
        assert!(s.get("k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn fake_transport_records_and_delivers() {
        let t = FakeTransport::new();
        let mut rx = t.events();
        let hex = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
        t.inject(RawEvent {
            id: EventId::parse(hex).unwrap(),
            kind: 1,
            pubkey: PubKey::parse(hex).unwrap(),
            created_at: Timestamp(1),
            json: serde_json::Value::Null,
            source: EventSource::Relay { url: RelayUrl::parse("wss://x.example").unwrap() },
        })
        .await;
        assert_eq!(rx.recv().await.unwrap().kind, 1);

        let ack = t.send(Outbound::Unsubscribe { id: messenger_core::SubId("s".into()) }).await.unwrap();
        assert!(!ack.is_delivered());
        assert_eq!(t.sent().len(), 1);
    }
}

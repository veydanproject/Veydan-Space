// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Test doubles shared by every messenger crate.
//!
//! - [`MemorySecretStore`]: `SecretStore` in a `Mutex<HashMap>` with a lock switch.
//! - [`FakeTransport`]: records every `Outbound`, lets tests inject `RawEvent`s.
//!
//! - [`FileSecretStore`]: plaintext JSON secrets for the CLI and standalone runs.
//!
//! The `messenger-cli` binary (src/bin) drives `MessengerRuntime` headless.

pub mod file_secrets;
pub use file_secrets::FileSecretStore;

use async_trait::async_trait;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, MessengerError, Outbound, RawEvent, Result, SecretStore, Transport};
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

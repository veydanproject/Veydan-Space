// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a bot keeps besides its database: its secrets in a file of its
//! folder (a trial on test servers, nothing worth a keychain), the blob
//! hosts of the messenger with counters around them, and named counters.

use async_trait::async_trait;
use communities_core::blobs::{BlobHosts, MediaHosts};
use messenger_core::{MessengerError, Result, SecretStore};
use nostr::key::Keys;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use zeroize::Zeroizing;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Counters by name, for the JSON lines of the trial.
#[derive(Default)]
pub struct Counters(Mutex<BTreeMap<String, u64>>);

impl Counters {
    pub fn add(&self, name: &str, n: u64) {
        *lock(&self.0).entry(name.to_string()).or_insert(0) += n;
    }

    pub fn get(&self, name: &str) -> u64 {
        lock(&self.0).get(name).copied().unwrap_or(0)
    }

    pub fn all(&self) -> BTreeMap<String, u64> {
        lock(&self.0).clone()
    }
}

/// The secret store of a bot: a JSON map of hex values in its folder,
/// written whole on every change.
pub struct FileSecrets {
    path: PathBuf,
    map: Mutex<HashMap<String, String>>,
}

impl FileSecrets {
    pub fn open(path: PathBuf) -> Result<Self> {
        let map = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s)?,
            Err(_) => HashMap::new(),
        };
        Ok(Self { path, map: Mutex::new(map) })
    }

    fn save(&self, map: &HashMap<String, String>) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string(map)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[async_trait]
impl SecretStore for FileSecrets {
    async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let map = lock(&self.map);
        match map.get(key) {
            Some(h) => Ok(Some(Zeroizing::new(hex::decode(h).map_err(|e| MessengerError::Storage(e.to_string()))?))),
            None => Ok(None),
        }
    }

    async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        let mut map = lock(&self.map);
        map.insert(key.to_string(), hex::encode(value));
        self.save(&map)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let mut map = lock(&self.map);
        map.remove(key);
        self.save(&map)
    }

    async fn is_unlocked(&self) -> bool {
        true
    }
}

/// One address for every GET of the process: the media hosts limit
/// requests per address (nginx `veydan_media`, 20 r/s, burst 60), and a
/// thousand bots on one machine are one address.
pub struct Pacer {
    gap: Duration,
    next: tokio::sync::Mutex<Instant>,
}

impl Pacer {
    pub fn new(per_sec: u32) -> Arc<Self> {
        Arc::new(Self { gap: Duration::from_secs(1) / per_sec.max(1), next: tokio::sync::Mutex::new(Instant::now()) })
    }

    async fn wait(&self) {
        let at = {
            let mut next = self.next.lock().await;
            let at = (*next).max(Instant::now());
            *next = at + self.gap;
            at
        };
        tokio::time::sleep_until(at).await;
    }
}

/// One HTTP client to the hosts for every bot of the process: a client per
/// bot held its own TLS roots and connections (1.8 GB for 1000 bots of a
/// swarm, a handshake per bot and GET); a device has its own, the stand
/// shares. Counted per bot all the same.
fn shared_hosts() -> Result<Arc<MediaHosts>> {
    static HOSTS: std::sync::OnceLock<Arc<MediaHosts>> = std::sync::OnceLock::new();
    if let Some(h) = HOSTS.get() {
        return Ok(h.clone());
    }
    let h = Arc::new(MediaHosts::new()?);
    Ok(HOSTS.get_or_init(|| h).clone())
}

/// The hosts of the messenger (Blossom), counted: what one client asks
/// of them. A GET that fails is tried again after 1, 2 and 4 s (the
/// limit of the address, not of the client), and the retries counted.
pub struct CountingHosts {
    inner: Arc<MediaHosts>,
    pacer: Arc<Pacer>,
    pub counts: Counters,
}

impl CountingHosts {
    pub fn new(pacer: Arc<Pacer>) -> Result<Self> {
        Ok(Self { inner: shared_hosts()?, pacer, counts: Counters::default() })
    }

    pub fn gets(&self) -> u64 {
        self.counts.get("get")
    }
}

/// How many times a GET is tried.
const GET_TRIES: u32 = 4;

#[async_trait]
impl BlobHosts for CountingHosts {
    async fn put(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.put_typed(signer, host, sha256, bytes, communities_core::blobs::CONTENT_TYPE).await
    }

    async fn put_typed(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        self.put_typed_keyed(signer, None, host, sha256, bytes, content_type).await
    }

    async fn put_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.put_typed_keyed(signer, key, host, sha256, bytes, communities_core::blobs::CONTENT_TYPE).await
    }

    async fn put_typed_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        self.pacer.wait().await;
        let r = self.inner.put_typed_keyed(signer, key, host, sha256, bytes, content_type).await;
        match &r {
            Ok(()) => {
                self.counts.add("put", 1);
                self.counts.add("put_bytes", bytes.len() as u64);
            }
            Err(e) => {
                self.counts.add("put_err", 1);
                eprintln!("put {host} {}: {e}", &sha256[..12]);
            }
        }
        r
    }

    async fn get(&self, host: &str, sha256: &str) -> Result<Option<Vec<u8>>> {
        self.get_up_to(host, sha256, communities_core::blobs::MAX_BLOB_BYTES).await
    }

    async fn get_up_to(&self, host: &str, sha256: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let mut last = None;
        for attempt in 0..GET_TRIES {
            if attempt > 0 {
                self.counts.add("get_retry", 1);
                tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
            }
            self.pacer.wait().await;
            self.counts.add("get", 1);
            match self.inner.get_up_to(host, sha256, max_bytes).await {
                Ok(Some(b)) => {
                    self.counts.add("get_bytes", b.len() as u64);
                    return Ok(Some(b));
                }
                Ok(None) => {
                    self.counts.add("get_missing", 1);
                    return Ok(None);
                }
                Err(e) => last = Some(e),
            }
        }
        self.counts.add("get_err", 1);
        Err(last.unwrap_or_else(|| MessengerError::Transport("get".into())))
    }

    async fn delete(&self, signer: &Keys, host: &str, sha256: &str) -> Result<()> {
        self.delete_keyed(signer, None, host, sha256).await
    }

    async fn delete_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str) -> Result<()> {
        self.pacer.wait().await;
        let r = self.inner.delete_keyed(signer, key, host, sha256).await;
        self.counts.add(if r.is_ok() { "delete" } else { "delete_err" }, 1);
        r
    }

    async fn gate(&self, host: &str) -> Option<communities_core::blobs::Gate> {
        self.inner.gate(host).await
    }

    async fn revoke(&self, app_key: &str, host: &str, label: &str, pubkey: &str) -> Result<()> {
        self.inner.revoke(app_key, host, label, pubkey).await
    }
}

/// Adds the counters of `from` into `into`.
pub fn sum_into(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (k, v) in from {
        *into.entry(k.clone()).or_insert(0) += v;
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The hosts of a channel as the messenger's engine sees a server: a file
//! of a channel goes up and comes down through messenger-media (`upload`,
//! `download`), as a file of a chat does, with its chunks several at once,
//! the retries of a chunk, the resume from the chunks the hosts hold and
//! the counts of its progress. The engine speaks to one server; a channel
//! keeps a file on every host of `storage[]` (wire §9).
//!
//! `HostsBackend` is that one server: every chunk goes to every host still
//! in the file at once, and is stored once `need` of them took it (as
//! many as an epoch needs, `keeper::MIN_HOSTS`). A host that failed a
//! chunk while enough others took it holds no whole file: it is left out
//! of this one, and the descriptor names only those that hold every chunk.
//! Too few took it: the chunk failed, and the engine tries it again on the
//! same hosts (a refusal of a host leaves it out at once: it would refuse
//! again). A chunk is there only when every host still in the file has it.
//!
//! `HostsFetcher` reads a chunk at `<host>/<sha256>` through the seam.

use crate::blobs::BlobHosts;
use async_trait::async_trait;
use bytes::Bytes;
use messenger_core::{MessengerError, Result};
use messenger_media::backend::{BackendError, BackendResult};
use messenger_media::download::BlobFetcher;
use messenger_media::BlobBackend;
use nostr::key::Keys;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The largest chunk the messenger makes (descriptor bounds) and its tag.
const MAX_CHUNK_BYTES: u64 = 64 * 1024 * 1024 + 16;

/// What an upload names as its server: the hosts in the order of
/// `storage`. Another set of hosts is another server, and the file goes
/// there anew under a new key (messenger-media `upload`).
pub fn base_of(hosts: &[String]) -> String {
    hosts.join(" ")
}

/// The hosts of a `base_of`.
pub fn hosts_of(base: &str) -> Vec<String> {
    base.split_whitespace().map(str::to_string).collect()
}

/// The hosts of a channel as one server of the engine; see the module.
pub struct HostsBackend {
    hosts: Vec<String>,
    backends: Vec<Arc<dyn BlobBackend>>,
    need: usize,
    alive: Mutex<Vec<bool>>,
    /// The last refusal of a host, with its address.
    refused: Mutex<Option<(String, BackendError)>>,
}

impl HostsBackend {
    /// `hosts` in the order of `storage`; a chunk is stored once `need` of
    /// them took it (at least one, at most all).
    pub fn new(blobs: &Arc<dyn BlobHosts>, signer: &Keys, hosts: &[String], need: usize) -> Self {
        let hosts: Vec<(String, Option<String>)> = hosts.iter().map(|h| (h.clone(), None)).collect();
        Self::keyed(blobs, signer, &hosts, need)
    }

    /// `new` of hosts each with the key it needs (`Env::writable`, wire §9.1).
    pub fn keyed(blobs: &Arc<dyn BlobHosts>, signer: &Keys, hosts: &[(String, Option<String>)], need: usize) -> Self {
        let hosts: Vec<(String, Option<String>, Keys)> = hosts.iter().map(|(h, k)| (h.clone(), k.clone(), signer.clone())).collect();
        Self::signed(blobs, &hosts, need)
    }

    /// `keyed` with the key each host is written with: the key of the
    /// channel for the owner's Cloud where it serves (wire §9.2), the
    /// person's own elsewhere.
    pub fn signed(blobs: &Arc<dyn BlobHosts>, hosts: &[(String, Option<String>, Keys)], need: usize) -> Self {
        let backends = hosts
            .iter()
            .map(|(h, key, signer)| {
                blobs.chunk_backend_keyed(signer, key.as_deref(), h).unwrap_or_else(|| {
                    Arc::new(SeamBackend { blobs: blobs.clone(), signer: signer.clone(), host: h.clone(), key: key.clone() })
                })
            })
            .collect();
        Self {
            hosts: hosts.iter().map(|(h, _, _)| h.clone()).collect(),
            backends,
            need: need.clamp(1, hosts.len().max(1)),
            alive: Mutex::new(vec![true; hosts.len()]),
            refused: Mutex::new(None),
        }
    }

    fn alive(&self) -> std::sync::MutexGuard<'_, Vec<bool>> {
        self.alive.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn targets(&self) -> Vec<usize> {
        self.alive().iter().enumerate().filter(|(_, a)| **a).map(|(i, _)| i).collect()
    }

    /// The hosts still in the file, in the order of `storage`: once it is
    /// stored, those that hold every chunk.
    pub fn holding(&self) -> Vec<String> {
        let alive = self.alive();
        self.hosts.iter().zip(alive.iter()).filter(|(_, a)| **a).map(|(h, _)| h.clone()).collect()
    }

    /// The last host that refused a chunk, and its answer.
    pub fn refused(&self) -> Option<(String, BackendError)> {
        self.refused.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn leave_out(&self, i: usize, e: &BackendError) {
        tracing::warn!(host = %self.hosts[i], error = %e.message, "a host did not take a chunk");
        self.alive()[i] = false;
        if !e.is_retryable() {
            *self.refused.lock().unwrap_or_else(|e| e.into_inner()) = Some((self.hosts[i].clone(), e.clone()));
        }
    }

    /// No host left to take a chunk: what the last one that refused said,
    /// else a network error (worth another attempt of the file).
    fn none_left(&self) -> BackendError {
        match self.refused() {
            Some((host, e)) => BackendError { status: e.status, message: format!("{host}: {}", e.message) },
            None if self.hosts.is_empty() => BackendError { status: Some(400), message: "the channel names no blob host".into() },
            None => BackendError { status: None, message: "no host of the channel takes the file".into() },
        }
    }
}

#[async_trait]
impl BlobBackend for HostsBackend {
    fn public_base(&self) -> String {
        base_of(&self.hosts)
    }

    /// Every host still in the file has it. A host that cannot tell counts
    /// as one that has not: the chunk goes again, and a host that fails it
    /// then is left out as `put_counted` says.
    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        let targets = self.targets();
        if targets.is_empty() {
            return Ok(false);
        }
        let asks = targets.iter().map(|&i| self.backends[i].exists(sha256));
        Ok(futures_util::future::join_all(asks).await.into_iter().all(|r| matches!(r, Ok(true))))
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        self.put_counted(sha256, bytes.into(), Arc::new(AtomicU64::new(0))).await
    }

    /// To every host still in the file at once. The bytes counted are
    /// those of the first of them: the copies leave side by side, and the
    /// time left is that of one.
    async fn put_counted(&self, sha256: &str, bytes: Bytes, sent: Arc<AtomicU64>) -> BackendResult<()> {
        let targets = self.targets();
        if targets.len() < self.need {
            return Err(self.none_left());
        }
        let puts = targets.iter().enumerate().map(|(n, &i)| {
            let counter = if n == 0 { sent.clone() } else { Arc::new(AtomicU64::new(0)) };
            self.backends[i].put_counted(sha256, bytes.clone(), counter)
        });
        let results = futures_util::future::join_all(puts).await;
        let took = results.iter().filter(|r| r.is_ok()).count();
        let mut retry = None;
        for (&i, r) in targets.iter().zip(results) {
            let Err(e) = r else { continue };
            // A refusal would come again; a failure on the way, while
            // enough others took it, leaves a hole in this host's copy.
            if !e.is_retryable() || took >= self.need {
                self.leave_out(i, &e);
            } else {
                retry = Some(e);
            }
        }
        if took >= self.need {
            sent.store(bytes.len() as u64, Ordering::Relaxed);
            return Ok(());
        }
        Err(retry.unwrap_or_else(|| self.none_left()))
    }

    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        let mut last = Ok(());
        for b in &self.backends {
            if let Err(e) = b.delete(sha256).await {
                last = Err(e);
            }
        }
        last
    }
}

/// One host through the seam, for a seam that has no backend of its own
/// (`BlobHosts::chunk_backend`): the tests' hosts.
struct SeamBackend {
    blobs: Arc<dyn BlobHosts>,
    signer: Keys,
    host: String,
    /// The key the host needs (wire §9.1), when it needs one.
    key: Option<String>,
}

/// An error of the seam as the engine weighs it: the messenger's codes
/// (`err.network: …`) back to what a request would have answered.
fn backend_error(e: MessengerError) -> BackendError {
    let text = match e {
        MessengerError::Transport(m) => m,
        other => other.to_string(),
    };
    let (code, rest) = text.split_once(": ").unwrap_or((text.as_str(), ""));
    let status = match code {
        "err.network" | "err.timeout" => None,
        "err.rate_limited" => Some(429),
        "err.server" => Some(500),
        "err.auth_failed" => Some(403),
        _ => Some(400),
    };
    let message = if status == Some(400) || rest.is_empty() { text.clone() } else { rest.to_string() };
    BackendError { status, message }
}

#[async_trait]
impl BlobBackend for SeamBackend {
    fn public_base(&self) -> String {
        self.host.clone()
    }

    async fn exists(&self, sha256: &str) -> BackendResult<bool> {
        self.blobs.get_up_to(&self.host, sha256, MAX_CHUNK_BYTES).await.map(|b| b.is_some()).map_err(backend_error)
    }

    async fn put(&self, sha256: &str, bytes: Vec<u8>) -> BackendResult<()> {
        self.blobs.put_keyed(&self.signer, self.key.as_deref(), &self.host, sha256, &bytes).await.map_err(backend_error)
    }

    async fn delete(&self, sha256: &str) -> BackendResult<()> {
        self.blobs.delete_keyed(&self.signer, self.key.as_deref(), &self.host, sha256).await.map_err(backend_error)
    }
}

/// Reads a chunk at `<host>/<sha256>` (`MediaDescriptor::chunk_url`)
/// through the seam, for the engine's `download`.
pub struct HostsFetcher {
    pub blobs: Arc<dyn BlobHosts>,
}

fn split(url: &str) -> Result<(&str, &str)> {
    url.rsplit_once('/').ok_or_else(|| MessengerError::Invalid("channel media: the address of a chunk".into()))
}

#[async_trait]
impl BlobFetcher for HostsFetcher {
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let (host, sha256) = split(url)?;
        self.blobs.get_up_to(host, sha256, max_bytes).await
    }

    async fn fetch_counted(&self, url: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        let (host, sha256) = split(url)?;
        self.blobs.get_counted(host, sha256, max_bytes, got).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::MemHosts;

    const H1: &str = "https://h1.test";
    const H2: &str = "https://h2.test";

    fn backend(hosts: &Arc<MemHosts>, need: usize) -> HostsBackend {
        let seam: Arc<dyn BlobHosts> = hosts.clone();
        HostsBackend::new(&seam, &Keys::generate(), &[H1.to_string(), H2.to_string()], need)
    }

    #[tokio::test]
    async fn a_host_that_fails_while_another_takes_the_chunk_is_left_out() {
        let hosts = MemHosts::new();
        let b = backend(&hosts, 1);
        hosts.set_down(H2, true);
        let chunk = b"a chunk".to_vec();
        let sha = crate::blobs::sha256_hex(&chunk);
        b.put(&sha, chunk.clone()).await.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(b.holding(), vec![H1.to_string()]);
        hosts.set_down(H2, false);
        assert!(b.exists(&sha).await.unwrap_or(false), "only the hosts still in the file are asked");
        assert_eq!(hosts.count(H2), 0, "and nothing more goes to the one left out");
    }

    #[tokio::test]
    async fn too_few_hosts_fail_the_chunk_and_a_refusal_says_its_host() {
        let hosts = MemHosts::new();
        let b = backend(&hosts, 2);
        hosts.set_down(H2, true);
        let chunk = b"a chunk".to_vec();
        let sha = crate::blobs::sha256_hex(&chunk);
        let e = b.put(&sha, chunk.clone()).await.err().unwrap_or_else(|| panic!("two were needed"));
        assert!(e.is_retryable(), "a host down is worth another try: {e:?}");
        assert_eq!(b.holding().len(), 2, "nobody left out of a chunk that failed");
        hosts.set_down(H2, false);
        hosts.refusing.lock().unwrap_or_else(|e| e.into_inner()).insert(H2.to_string());
        let e = b.put(&sha, chunk).await.err().unwrap_or_else(|| panic!("refused"));
        assert_eq!(e.status, Some(403));
        assert!(e.message.starts_with(H2), "{e:?}");
        assert_eq!(b.refused().map(|(h, _)| h).as_deref(), Some(H2));
    }

    #[test]
    fn errors_of_the_seam_keep_their_weight() {
        assert!(backend_error(MessengerError::Transport("err.network: down".into())).is_retryable());
        assert_eq!(backend_error(MessengerError::Transport("err.auth_failed: blossom 403".into())).status, Some(403));
        assert!(!backend_error(MessengerError::Invalid("hash".into())).is_retryable());
        assert_eq!(hosts_of(&base_of(&[H1.into(), H2.into()])), vec![H1.to_string(), H2.to_string()]);
    }
}

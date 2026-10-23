// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The blob hosts of a channel (internal/communities-wire.md §9): every
//! host of `storage[]` keeps a blob at `<host>/<sha256>`; whoever reads it
//! checks the hash. Blobs go up signed by the user at work, whose account
//! the hosts know (the keeper's blobs too, while its device has a user;
//! the owner pays for them, communities-spec §13); a member only reads.
//!
//! `BlobHosts` is the seam: the app takes the backends of the messenger
//! (`MediaHosts`: a Blossom upload signed by the writer, a public GET),
//! the tests an in-memory host with failures on demand.

use async_trait::async_trait;
use messenger_core::{MessengerError, Result};
use messenger_media::download::{BlobFetcher, HttpFetcher};
use messenger_media::{BlobBackend, BlossomBackend};
use nostr::key::Keys;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub use crate::upload_keys::Gate;

/// The largest blob a member reads: a shard of 1024 `sd-v1` blocks with
/// its header, an index of 512 hashes, a proof, with room to spare.
pub const MAX_BLOB_BYTES: u64 = 4 * 1024 * 1024;

/// The type a blob of keys is served with: bytes, nothing to render.
pub const CONTENT_TYPE: &str = "application/octet-stream";

/// The hosts of a channel, by their base address.
#[async_trait]
pub trait BlobHosts: Send + Sync {
    /// Puts `bytes` on `host` under their hash, signed by `signer`.
    /// Putting a blob twice is harmless.
    async fn put(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8]) -> Result<()>;
    /// The blob, `None` when this host does not have it. Not checked here.
    async fn get(&self, host: &str, sha256: &str) -> Result<Option<Vec<u8>>>;
    /// `get` of a blob that may be larger than a blob of keys: a chunk of
    /// a file (up to 16 MiB and its tag).
    async fn get_up_to(&self, host: &str, sha256: &str, _max_bytes: u64) -> Result<Option<Vec<u8>>> {
        self.get(host, sha256).await
    }
    /// `put` of a blob served as `content_type` (the cover of a channel is
    /// a picture, not bytes to save).
    async fn put_typed(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8], _content_type: &str) -> Result<()> {
        self.put(signer, host, sha256, bytes).await
    }
    /// Removes the blob; one already gone is fine.
    async fn delete(&self, signer: &Keys, host: &str, sha256: &str) -> Result<()>;
    /// The messenger's backend of `host` for the chunks of a file, signed
    /// by `signer`: one for the whole file, so its requests share their
    /// connections. `None`: the chunks go through this seam
    /// (`transfer::SeamBackend`), as in the tests.
    fn chunk_backend(&self, _signer: &Keys, _host: &str) -> Option<Arc<dyn BlobBackend>> {
        None
    }
    /// `get_up_to`, adding the bytes received so far to `got`, which starts
    /// from 0 with every call. One that cannot tell adds them at the end.
    async fn get_counted(&self, host: &str, sha256: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        got.store(0, Ordering::Relaxed);
        let out = self.get_up_to(host, sha256, max_bytes).await?;
        if let Some(b) = &out {
            got.fetch_add(b.len() as u64, Ordering::Relaxed);
        }
        Ok(out)
    }
    /// `put` with the key a host of the kit needs (wire §9.1: the app key of
    /// its card, or an upload key); `None`: none.
    async fn put_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        let _ = key;
        self.put(signer, host, sha256, bytes).await
    }
    /// `put_typed` with a key.
    async fn put_typed_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        let _ = key;
        self.put_typed(signer, host, sha256, bytes, content_type).await
    }
    /// `delete` with a key.
    async fn delete_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str) -> Result<()> {
        let _ = key;
        self.delete(signer, host, sha256).await
    }
    /// `chunk_backend` with a key.
    fn chunk_backend_keyed(&self, signer: &Keys, key: Option<&str>, host: &str) -> Option<Arc<dyn BlobBackend>> {
        let _ = key;
        self.chunk_backend(signer, host)
    }
    /// What `host` says it needs of a writer (`upload_keys::INFO_PATH`);
    /// `None`: it did not answer.
    async fn gate(&self, _host: &str) -> Option<Gate> {
        Some(Gate::Open)
    }
    /// Revokes the upload keys of `pubkey` (hex) under `label` on `host`,
    /// with the server's app key (wire §9.1). A host without revocation (an
    /// older kit answers 404) counts as done: its keys die with their time.
    async fn revoke(&self, _app_key: &str, _host: &str, _label: &str, _pubkey: &str) -> Result<()> {
        Ok(())
    }
}

/// The hex SHA-256 of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Reads a blob from the first of `hosts` that has it with the right hash:
/// a host that lies about the bytes is passed over like one that is down.
pub async fn fetch_verified(blobs: &dyn BlobHosts, hosts: &[String], sha256: &[u8; 32]) -> Result<Vec<u8>> {
    fetch_verified_up_to(blobs, hosts, sha256, MAX_BLOB_BYTES).await
}

/// `fetch_verified` of a blob up to `max_bytes`.
pub async fn fetch_verified_up_to(blobs: &dyn BlobHosts, hosts: &[String], sha256: &[u8; 32], max_bytes: u64) -> Result<Vec<u8>> {
    let want = hex::encode(sha256);
    for host in hosts {
        match blobs.get_up_to(host, &want, max_bytes).await {
            Ok(Some(bytes)) if Sha256::digest(&bytes).as_slice() == sha256 => return Ok(bytes),
            Ok(Some(_)) => tracing::warn!(host, "channel blob with a wrong hash"),
            Ok(None) => {}
            Err(e) => tracing::debug!(host, error = %e, "channel blob host failed"),
        }
    }
    Err(MessengerError::Transport(format!("err.not_found: blob {}", &want[..12])))
}

/// The hosts by the backends of the messenger: a Blossom upload and
/// delete signed by the writer's key (with the key a server of the kit
/// needs, wire §9.1), a public GET for reading.
pub struct MediaHosts {
    fetcher: HttpFetcher,
}

impl MediaHosts {
    pub fn new() -> Result<Self> {
        Ok(Self { fetcher: HttpFetcher::new()? })
    }

    fn backend(signer: &Keys, key: Option<&str>, host: &str) -> Result<BlossomBackend> {
        Ok(BlossomBackend::new(host, signer.clone())?.with_app_key(key.map(str::to_string)))
    }
}

#[async_trait]
impl BlobHosts for MediaHosts {
    async fn put(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.put_keyed(signer, None, host, sha256, bytes).await
    }

    async fn put_typed(&self, signer: &Keys, host: &str, sha256: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        self.put_typed_keyed(signer, None, host, sha256, bytes, content_type).await
    }

    async fn put_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.put_typed_keyed(signer, key, host, sha256, bytes, CONTENT_TYPE).await
    }

    async fn put_typed_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str, bytes: &[u8], content_type: &str) -> Result<()> {
        let backend = Self::backend(signer, key, host)?;
        backend.put_typed(sha256, bytes.to_vec(), content_type).await.map_err(MessengerError::from)
    }

    async fn get(&self, host: &str, sha256: &str) -> Result<Option<Vec<u8>>> {
        self.get_up_to(host, sha256, MAX_BLOB_BYTES).await
    }

    async fn get_up_to(&self, host: &str, sha256: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let url = format!("{}/{}", host.trim_end_matches('/'), sha256);
        self.fetcher.fetch(&url, max_bytes).await
    }

    async fn delete(&self, signer: &Keys, host: &str, sha256: &str) -> Result<()> {
        self.delete_keyed(signer, None, host, sha256).await
    }

    async fn delete_keyed(&self, signer: &Keys, key: Option<&str>, host: &str, sha256: &str) -> Result<()> {
        let backend = Self::backend(signer, key, host)?;
        backend.delete(sha256).await.map_err(MessengerError::from)
    }

    fn chunk_backend(&self, signer: &Keys, host: &str) -> Option<Arc<dyn BlobBackend>> {
        self.chunk_backend_keyed(signer, None, host)
    }

    fn chunk_backend_keyed(&self, signer: &Keys, key: Option<&str>, host: &str) -> Option<Arc<dyn BlobBackend>> {
        Self::backend(signer, key, host).ok().map(|b| Arc::new(b) as Arc<dyn BlobBackend>)
    }

    async fn get_counted(&self, host: &str, sha256: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        let url = format!("{}/{}", host.trim_end_matches('/'), sha256);
        self.fetcher.fetch_counted(&url, max_bytes, got).await
    }

    /// The JSON of a kit, or 401 of a kit before it that needs the key;
    /// any other answer is a host that needs none, no answer is `None`.
    async fn gate(&self, host: &str) -> Option<Gate> {
        let backend = BlossomBackend::new(host, Keys::generate()).ok()?;
        match backend.gate_info().await {
            Ok((200, body)) => Some(Gate::from_info(&body)),
            Ok((401, _)) => Some(Gate::Key { scoped: false }),
            Ok((s, _)) if s >= 500 => None,
            Ok(_) => Some(Gate::Open),
            Err(_) => None,
        }
    }

    async fn revoke(&self, app_key: &str, host: &str, label: &str, pubkey: &str) -> Result<()> {
        let backend = BlossomBackend::new(host, Keys::generate())?;
        match backend.revoke_upload_keys(app_key, label, pubkey).await.map_err(MessengerError::from)? {
            200 | 404 => Ok(()),
            s => Err(MessengerError::Transport(format!("err.server: {host} answered the revocation with {s}"))),
        }
    }
}

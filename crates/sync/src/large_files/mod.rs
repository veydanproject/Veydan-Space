// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Large files v2: a file stored as independently encrypted chunks plus an
//! encrypted manifest. Memory stays `O(chunk_size × parallelism)`.
//!
//! Protocol (frozen):
//! - keys `large-file/chunk`, `large-file/manifest`, `large-file/id` (see `keys.rs`);
//! - objects `large-files/v2/chunks/<chunk_id>` and `large-files/v2/manifests/<manifest_id>`;
//! - fixed chunk size per file, last chunk shorter, empty file has no chunks;
//! - publish order: chunks -> manifest -> domain op;
//! - a domain op that references large files puts their manifest ids into
//!   the payload field `lf_refs` (see [`LF_REFS_FIELD`]); the GC coordinator
//!   reads that field from every entity type, including unknown ones.
//!
//! The module knows nothing about notes or UI: the domain decides when to use
//! it and keeps file name, mime and ownership in its own payload.

mod crypto;
mod gc;
pub mod manifest;
mod source;
mod transfer;

pub use gc::{refs_from_payload, GcOutcome};
pub use manifest::FORMAT_VERSION;
pub use source::{available_space, FileSource, Opener, PathSink, PathSource, PART_SUFFIX};

use crate::keys::Keys;
use crate::storage::Storage;
use crate::{Result, SyncError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

/// Payload field every op referencing large files must carry: `[manifest_id, ...]`.
pub const LF_REFS_FIELD: &str = "lf_refs";

pub const MIN_CHUNK_SIZE: u64 = 1024 * 1024;
pub const MAX_CHUNK_SIZE: u64 = 64 * 1024 * 1024;
pub const MAX_PARALLELISM: usize = 8;

/// Handle to a stored file. Carries no domain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeFileRef {
    pub version: u32,
    pub manifest_id: String,
    pub size: u64,
    pub file_id: String,
}

/// Infrastructure settings; the domain policy (when to use v2) lives elsewhere.
#[derive(Clone, Debug)]
pub struct LargeFileConfig {
    pub chunk_size: u64,
    pub parallelism: usize,
    pub resume: bool,
}

impl Default for LargeFileConfig {
    fn default() -> Self {
        Self {
            chunk_size: 8 * 1024 * 1024,
            parallelism: 3,
            resume: true,
        }
    }
}

impl LargeFileConfig {
    pub fn validate(&self) -> Result<()> {
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&self.chunk_size) {
            return Err(SyncError::Format(format!(
                "chunk size {} out of range",
                self.chunk_size
            )));
        }
        if !(1..=MAX_PARALLELISM).contains(&self.parallelism) {
            return Err(SyncError::Format(format!(
                "parallelism {} out of range",
                self.parallelism
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Hashing,
    Uploading,
    Downloading,
    Verifying,
    Finalizing,
}

/// Plaintext bytes processed so far; `total` is unknown for generated sources.
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: Option<u64>,
}

pub type ProgressFn<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// Shared cancel signal; checked before every chunk.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(SyncError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Where the bytes come from. Path, content URI and generated data all fit.
#[async_trait]
pub trait LargeFileSource: Send + Sync {
    /// Size hint; `None` for generated data. The real size is counted during upload.
    fn len_hint(&self) -> Option<u64>;
    /// Fresh reader positioned at `offset`; non-seekable sources may skip bytes.
    async fn open_at(&self, offset: u64) -> Result<Box<dyn AsyncRead + Send + Unpin>>;
}

/// Where a downloaded file goes. Nothing is visible before `commit`.
#[async_trait]
pub trait LargeFileSink: Send + Sync {
    /// Bytes already staged for `manifest_id` by an earlier attempt; 0 when none.
    async fn staged_len(&self, manifest_id: &str) -> Result<u64>;
    /// Writer continuing at `offset` (0 restarts). `total` lets the sink check free space.
    async fn open_staging(
        &self,
        manifest_id: &str,
        offset: u64,
        total: u64,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>>;
    /// Atomically publish the completed file.
    async fn commit(&self) -> Result<()>;
    /// Drop staging data.
    async fn discard(&self) -> Result<()>;
}

#[derive(Clone, Debug, Serialize)]
pub struct LargeFileMetadata {
    pub size: u64,
    pub chunk_size: u64,
    pub chunks: usize,
    pub file_id: String,
    pub codec: String,
}

/// Chunked encrypted file store over any [`Storage`].
pub struct LargeFileStore<'a> {
    storage: &'a dyn Storage,
    vault_id: &'a str,
    keys: &'a Keys,
    config: LargeFileConfig,
}

impl<'a> LargeFileStore<'a> {
    pub fn new(
        storage: &'a dyn Storage,
        vault_id: &'a str,
        keys: &'a Keys,
        config: LargeFileConfig,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            storage,
            vault_id,
            keys,
            config,
        })
    }

    pub fn config(&self) -> &LargeFileConfig {
        &self.config
    }

    /// Chunk, encrypt and store `source`; existing chunks are not sent again.
    pub async fn upload(
        &self,
        source: &dyn LargeFileSource,
        progress: ProgressFn<'_>,
        cancel: &CancelFlag,
    ) -> Result<LargeFileRef> {
        transfer::upload(self, source, progress, cancel).await
    }

    /// Fetch, verify and assemble into `sink`; resumes from the sink's staged prefix.
    pub async fn download(
        &self,
        reference: &LargeFileRef,
        sink: &dyn LargeFileSink,
        progress: ProgressFn<'_>,
        cancel: &CancelFlag,
    ) -> Result<()> {
        transfer::download(self, reference, sink, progress, cancel).await
    }

    pub async fn inspect(&self, reference: &LargeFileRef) -> Result<LargeFileMetadata> {
        let m = self.read_manifest(reference).await?;
        Ok(LargeFileMetadata {
            size: m.size,
            chunk_size: m.chunk_size,
            chunks: m.chunks.len(),
            file_id: m.file_id,
            codec: m.codec,
        })
    }

    /// Re-hash `source` and compare with the stored manifest.
    pub async fn verify(
        &self,
        reference: &LargeFileRef,
        source: &dyn LargeFileSource,
        cancel: &CancelFlag,
    ) -> Result<()> {
        transfer::verify(self, reference, source, cancel).await
    }

    /// Destructive sweep. `roots` must be every live manifest id of the whole
    /// vault; `candidates` maps object keys to when they were first seen orphaned.
    pub async fn gc_with_complete_root_set(
        &self,
        roots: &BTreeSet<String>,
        candidates: &HashMap<String, i64>,
        now_ms: i64,
        grace_ms: i64,
    ) -> Result<GcOutcome> {
        gc::run(self, roots, candidates, now_ms, grace_ms).await
    }

    /// Storage keys of v2 objects; lets the v1 blob GC skip this namespace.
    pub fn is_v2_key(key: &str) -> bool {
        key.starts_with(crypto::CHUNKS_PREFIX) || key.starts_with(crypto::MANIFESTS_PREFIX)
    }

    async fn read_manifest(&self, reference: &LargeFileRef) -> Result<manifest::ManifestV2> {
        if reference.version != FORMAT_VERSION {
            return Err(SyncError::Format(format!(
                "unsupported large file version {}",
                reference.version
            )));
        }
        let key = crypto::manifest_key(&reference.manifest_id);
        let bytes = self.storage.get(&key).await?.ok_or_else(|| {
            SyncError::Storage(format!("manifest {} not found", reference.manifest_id))
        })?;
        let m = crypto::open_manifest(self.keys, self.vault_id, &reference.manifest_id, &bytes)?;
        if m.size != reference.size || m.file_id != reference.file_id {
            return Err(SyncError::Integrity(format!(
                "manifest {} does not match reference",
                reference.manifest_id
            )));
        }
        Ok(m)
    }
}

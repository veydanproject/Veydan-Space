// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Streaming upload / download with bounded parallelism, retry and resume.
//!
//! The source is read once, sequentially; up to `parallelism` chunks are in
//! flight (hash + seal + PUT). Downloads fetch in parallel but write in order,
//! so the staged prefix is always valid and resume is a plain length.

use super::crypto::{self, chunk_key, manifest_key};
use super::manifest::{ChunkEntry, ManifestV2, CODEC_ZSTD, FORMAT_VERSION};
use super::{
    CancelFlag, LargeFileRef, LargeFileSink, LargeFileSource, LargeFileStore, Phase, Progress,
    ProgressFn,
};
use crate::{Result, SyncError};
use futures_util::stream::{FuturesOrdered, StreamExt};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const RETRY_ATTEMPTS: u32 = 4;
const RETRY_BASE: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(15);

fn retryable(e: &SyncError) -> bool {
    matches!(e, SyncError::Storage(_))
}

fn backoff(attempt: u32) -> Duration {
    let base = RETRY_BASE
        .saturating_mul(1u32 << attempt.min(6))
        .min(RETRY_MAX);
    let jitter = rand::random::<u64>() % (base.as_millis() as u64 / 2 + 1);
    base + Duration::from_millis(jitter)
}

/// Transport errors are retried with exponential backoff; auth, integrity and
/// format errors return at once.
async fn with_retry<T, F, Fut>(cancel: &CancelFlag, op: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        cancel.check()?;
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if retryable(&e) && attempt + 1 < RETRY_ATTEMPTS => {
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Fill `buf` up to its capacity; short only at EOF.
async fn read_chunk(reader: &mut (dyn AsyncRead + Send + Unpin), size: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; size];
    let mut filled = 0;
    while filled < size {
        let n = reader
            .read(&mut buf[filled..])
            .await
            .map_err(|e| SyncError::Source(e.to_string()))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

struct Counter<'a> {
    done: AtomicU64,
    total: Option<u64>,
    progress: ProgressFn<'a>,
}

impl Counter<'_> {
    fn add(&self, phase: Phase, n: u64) {
        let done = self.done.fetch_add(n, Ordering::Relaxed) + n;
        (self.progress)(Progress {
            phase,
            done,
            total: self.total,
        });
    }

    fn report(&self, phase: Phase) {
        (self.progress)(Progress {
            phase,
            done: self.done.load(Ordering::Relaxed),
            total: self.total,
        });
    }
}

pub(super) async fn upload(
    store: &LargeFileStore<'_>,
    source: &dyn LargeFileSource,
    progress: ProgressFn<'_>,
    cancel: &CancelFlag,
) -> Result<LargeFileRef> {
    let chunk_size = store.config.chunk_size;
    let counter = Counter {
        done: AtomicU64::new(0),
        total: source.len_hint(),
        progress,
    };
    counter.report(Phase::Hashing);

    // Sequential read, up to `parallelism` chunks sealing/uploading at once,
    // results collected in file order.
    let mut reader = source.open_at(0).await?;
    let mut in_flight = FuturesOrdered::new();
    let mut entries: Vec<ChunkEntry> = Vec::new();
    loop {
        let plain = read_chunk(reader.as_mut(), chunk_size as usize).await?;
        let eof = (plain.len() as u64) < chunk_size;
        if !plain.is_empty() {
            in_flight.push_back(upload_chunk(store, plain, &counter, cancel));
        }
        while in_flight.len() >= store.config.parallelism || (eof && !in_flight.is_empty()) {
            match in_flight.next().await {
                Some(entry) => entries.push(entry?),
                None => break,
            }
        }
        if eof {
            break;
        }
    }

    cancel.check()?;
    counter.report(Phase::Finalizing);
    let size = entries.iter().map(|c| c.size).sum();
    let ids: Vec<String> = entries.iter().map(|c| c.id.clone()).collect();
    let file_id = crypto::file_id(store.keys, &ids);
    let manifest = ManifestV2 {
        version: FORMAT_VERSION,
        size,
        file_id: file_id.clone(),
        chunk_size,
        codec: CODEC_ZSTD.into(),
        chunks: entries,
    };
    manifest.validate()?;
    let (manifest_id, sealed) = crypto::seal_manifest(store.keys, store.vault_id, &manifest)?;
    let key = manifest_key(&manifest_id);
    with_retry(cancel, || store.storage.put_if_absent(&key, &sealed)).await?;
    Ok(LargeFileRef {
        version: FORMAT_VERSION,
        manifest_id,
        size,
        file_id,
    })
}

/// Hash, skip when present, otherwise seal and store. Returns the manifest entry.
async fn upload_chunk(
    store: &LargeFileStore<'_>,
    plain: Vec<u8>,
    counter: &Counter<'_>,
    cancel: &CancelFlag,
) -> Result<ChunkEntry> {
    cancel.check()?;
    let size = plain.len() as u64;
    let id = crypto::chunk_id(store.keys, &plain);
    let key = chunk_key(&id);
    let present = with_retry(cancel, || store.storage.exists(&key)).await?;
    if !present {
        let sealed = crypto::seal_chunk(store.keys, store.vault_id, &id, &plain)?;
        drop(plain);
        with_retry(cancel, || store.storage.put_if_absent(&key, &sealed)).await?;
    }
    counter.add(Phase::Uploading, size);
    Ok(ChunkEntry { id, size })
}

pub(super) async fn download(
    store: &LargeFileStore<'_>,
    reference: &LargeFileRef,
    sink: &dyn LargeFileSink,
    progress: ProgressFn<'_>,
    cancel: &CancelFlag,
) -> Result<()> {
    let manifest = store.read_manifest(reference).await?;
    let chunk_size = manifest.chunk_size;
    let staged = if store.config.resume {
        sink.staged_len(&reference.manifest_id).await?
    } else {
        0
    };
    let start = ((staged / chunk_size) as usize).min(manifest.chunks.len());
    let offset = start as u64 * chunk_size;

    let counter = Counter {
        done: AtomicU64::new(offset),
        total: Some(manifest.size),
        progress,
    };
    counter.report(Phase::Downloading);
    let mut writer = sink
        .open_staging(&reference.manifest_id, offset, manifest.size)
        .await?;
    let result = write_chunks(
        store,
        &manifest.chunks[start..],
        writer.as_mut(),
        &counter,
        cancel,
    )
    .await;
    // Settle in-flight writes so the staged length is stable before discard/resume.
    let _ = writer.shutdown().await;
    drop(writer);

    if let Err(e) = result {
        // Integrity failures mean the remote object is bad; a stale prefix is still fine to keep.
        if !store.config.resume {
            sink.discard().await?;
        }
        return Err(e);
    }
    counter.report(Phase::Finalizing);
    sink.commit().await
}

/// Fetch up to `parallelism` chunks at once, write them in manifest order.
async fn write_chunks(
    store: &LargeFileStore<'_>,
    chunks: &[ChunkEntry],
    writer: &mut (dyn AsyncWrite + Send + Unpin),
    counter: &Counter<'_>,
    cancel: &CancelFlag,
) -> Result<()> {
    let mut pending = chunks.iter();
    let mut in_flight = FuturesOrdered::new();
    loop {
        while in_flight.len() < store.config.parallelism {
            match pending.next() {
                Some(entry) => in_flight.push_back(fetch_chunk(store, entry, cancel)),
                None => break,
            }
        }
        let Some(plain) = in_flight.next().await else {
            break;
        };
        let plain = plain?;
        writer
            .write_all(&plain)
            .await
            .map_err(|e| SyncError::Disk(e.to_string()))?;
        counter.add(Phase::Downloading, plain.len() as u64);
    }
    writer
        .flush()
        .await
        .map_err(|e| SyncError::Disk(e.to_string()))?;
    writer
        .shutdown()
        .await
        .map_err(|e| SyncError::Disk(e.to_string()))
}

/// GET with retry, then AEAD + id + size checks. A missing chunk is not retried:
/// the uploader may still be publishing it, the caller comes back next cycle.
async fn fetch_chunk(
    store: &LargeFileStore<'_>,
    entry: &ChunkEntry,
    cancel: &CancelFlag,
) -> Result<Vec<u8>> {
    let key = chunk_key(&entry.id);
    let bytes = with_retry(cancel, || store.storage.get(&key))
        .await?
        .ok_or_else(|| SyncError::Storage(format!("chunk {} not found", entry.id)))?;
    crypto::open_chunk(store.keys, store.vault_id, &entry.id, entry.size, &bytes)
}

pub(super) async fn verify(
    store: &LargeFileStore<'_>,
    reference: &LargeFileRef,
    source: &dyn LargeFileSource,
    cancel: &CancelFlag,
) -> Result<()> {
    let manifest = store.read_manifest(reference).await?;
    let mut reader = source.open_at(0).await?;
    for (i, entry) in manifest.chunks.iter().enumerate() {
        cancel.check()?;
        let plain = read_chunk(reader.as_mut(), manifest.chunk_size as usize).await?;
        if plain.len() as u64 != entry.size || crypto::chunk_id(store.keys, &plain) != entry.id {
            return Err(SyncError::Integrity(format!(
                "source differs from stored file at chunk {i}"
            )));
        }
    }
    if !read_chunk(reader.as_mut(), 1).await?.is_empty() {
        return Err(SyncError::Integrity(
            "source is longer than stored file".into(),
        ));
    }
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Upload: read the file chunk by chunk, encrypt, store. Nothing is
//! written to disk and at most one chunk is in memory.
//!
//! Resume: the key and the chunks finished so far are handed to the caller
//! after every chunk (`UploadState`), so an interrupted upload continues
//! with the same key; chunks the server already has are skipped after one
//! `exists` call.

use crate::backend::{BackendError, BlobBackend};
use crate::crypto::{sha256_hex, FileKey};
use crate::descriptor::{chunk_size_for, mime_for, safe_name, ChunkRef, MediaDescriptor, MediaKind, ALGO, MAX_FILE_BYTES};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;

pub const RUN: u8 = 0;
pub const PAUSE: u8 = 1;
pub const CANCEL: u8 = 2;

#[cfg(not(test))]
const RETRY_DELAYS: [u64; 3] = [1, 2, 4];
#[cfg(test)]
const RETRY_DELAYS: [u64; 3] = [0, 0, 0];

/// What survives a crash or a pause.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadState {
    /// Base64 key and nonce, fixed at the first attempt.
    pub key: String,
    pub iv: String,
    pub chunk_size: u64,
    /// Size and mtime of the file when the upload started: a file that
    /// changed meanwhile starts over.
    pub file_size: u64,
    pub file_mtime: i64,
    /// Finished chunks, in order.
    pub chunks: Vec<ChunkRef>,
    /// Hex SHA-256 state cannot be persisted, so the plaintext hash is
    /// recomputed on resume while the finished chunks are re-read.
    pub done: bool,
}

impl UploadState {
    pub fn file_key(&self) -> Result<FileKey> {
        let bad = |_| MessengerError::Invalid("stored upload key is not base64".into());
        FileKey::from_parts(&B64.decode(&self.key).map_err(bad)?, &B64.decode(&self.iv).map_err(bad)?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UploadOutcome {
    Done(Box<MediaDescriptor>),
    Paused,
    Cancelled,
}

pub struct UploadParams<'a> {
    pub path: &'a Path,
    pub caption: Option<String>,
    pub batch: Option<String>,
    /// State of a previous attempt, if any.
    pub resume: Option<UploadState>,
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn put_with_retry(backend: &dyn BlobBackend, sha: &str, bytes: &[u8]) -> std::result::Result<(), BackendError> {
    let mut attempt = 0;
    loop {
        match backend.put(sha, bytes.to_vec()).await {
            Ok(()) => return Ok(()),
            Err(e) if e.is_retryable() && attempt < RETRY_DELAYS.len() => {
                tokio::time::sleep(Duration::from_secs(RETRY_DELAYS[attempt])).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Upload `path`. `on_progress(done_bytes, total_bytes, state)` is called
/// after every chunk with the state to persist. `control` is polled
/// between chunks.
pub async fn upload<F>(
    backend: &dyn BlobBackend,
    params: UploadParams<'_>,
    control: &AtomicU8,
    mut on_progress: F,
) -> Result<UploadOutcome>
where
    F: FnMut(u64, u64, &UploadState) + Send,
{
    let meta = tokio::fs::metadata(params.path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
    if !meta.is_file() {
        return Err(MessengerError::Invalid("not a regular file".into()));
    }
    let size = meta.len();
    if size == 0 {
        return Err(MessengerError::Invalid("the file is empty".into()));
    }
    if size > MAX_FILE_BYTES {
        return Err(MessengerError::Invalid("err.file_too_large".into()));
    }
    let name = safe_name(&params.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let mime = mime_for(&name).to_string();

    // Continue a previous attempt only when it was for this very file.
    let mut state = match params.resume {
        Some(s) if s.file_size == size && s.file_mtime == mtime(&meta) && !s.key.is_empty() => s,
        _ => {
            let k = FileKey::generate()?;
            UploadState {
                key: B64.encode(k.key),
                iv: B64.encode(k.base_nonce),
                chunk_size: chunk_size_for(size),
                file_size: size,
                file_mtime: mtime(&meta),
                chunks: vec![],
                done: false,
            }
        }
    };
    let key = state.file_key()?;
    let chunk_size = state.chunk_size as usize;
    let total_chunks = size.div_ceil(state.chunk_size) as usize;

    let mut file = tokio::fs::File::open(params.path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; chunk_size];
    let mut done_bytes: u64 = 0;

    for index in 0..total_chunks {
        match control.load(Ordering::SeqCst) {
            PAUSE => return Ok(UploadOutcome::Paused),
            CANCEL => return Ok(UploadOutcome::Cancelled),
            _ => {}
        }
        let want = chunk_size.min((size - done_bytes) as usize);
        file.read_exact(&mut buf[..want]).await.map_err(|e| MessengerError::Io(e.to_string()))?;
        let plain = &buf[..want];
        hasher.update(plain);

        if let Some(known) = state.chunks.get(index) {
            // Finished before the interruption. One cheap check that the
            // server still has it; otherwise send it again.
            if backend.exists(&known.sha256).await.unwrap_or(false) {
                done_bytes += want as u64;
                on_progress(done_bytes, size, &state);
                continue;
            }
        }
        let cipher = key.encrypt_chunk(index as u32, plain)?;
        let sha = sha256_hex(&cipher);
        let present = backend.exists(&sha).await.unwrap_or(false);
        if !present {
            put_with_retry(backend, &sha, &cipher).await?;
        }
        let chunk = ChunkRef { sha256: sha, size: cipher.len() as u64 };
        if index < state.chunks.len() {
            state.chunks[index] = chunk;
        } else {
            state.chunks.push(chunk);
        }
        done_bytes += want as u64;
        on_progress(done_bytes, size, &state);
    }
    state.done = true;
    on_progress(size, size, &state);

    let mut descriptor = MediaDescriptor {
        kind: MediaKind::from_mime(&mime),
        name,
        mime,
        size,
        sha256: hex::encode(hasher.finalize()),
        chunk_size: state.chunk_size,
        chunks: state.chunks.clone(),
        algo: ALGO.into(),
        key: String::new(),
        iv: String::new(),
        servers: vec![backend.public_base()],
        caption: params.caption.filter(|c| !c.trim().is_empty()),
        batch: params.batch,
        dim: None,
        duration_ms: None,
        waveform: None,
    };
    descriptor.set_key(&key);
    Ok(UploadOutcome::Done(Box::new(descriptor)))
}

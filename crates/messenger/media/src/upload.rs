// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Upload: read the file chunk by chunk, encrypt, store, several chunks at
//! once. Nothing is written to disk; memory holds the chunks in flight and
//! a few waiting for a worker, never the file.
//!
//! A reader goes through the file in order (the plaintext hash needs the
//! order), encrypts every chunk off the async threads and hands it to the
//! workers. Each worker stores one chunk at a time, within the chunk slots
//! all transfers share. A fresh chunk is stored without asking first:
//! under a new key its ciphertext cannot be on the server yet.
//!
//! Resume: the key and every finished chunk are handed to the caller after
//! each chunk (`UploadState`), so an interrupted upload continues with the
//! same key. Chunks an earlier attempt stored are looked up on the server
//! first, several at once (the checking stage); only the missing ones are
//! sent again. A server that does not answer the lookup tells nothing: the
//! attempt fails (and is retried) with the state as it was. The descriptor
//! is the one a chunk-after-chunk upload with the same key makes: same
//! order, same hashes.
//!
//! A key is used for one content only. Every chunk whose ciphertext went
//! out under a key (stored, or sent and not confirmed) is encrypted again
//! and compared before anything new is encrypted under it, and again as it
//! is read for the plaintext hash: a file that changed under the same size
//! and time starts over with a new key. What an upload leaves on a server
//! it no longer uses (another server, an earlier key) goes to `stale`, for
//! the caller to remove. An upload that goes on elsewhere does so under a
//! new key too: the same names on two servers could be removed from the
//! one in use with what is left on the other.

use crate::backend::BlobBackend;
use crate::control::{pauses, Slot, TransferCtx};
use crate::crypto::{sha256_hex, FileKey};
use crate::descriptor::{chunk_size_for, mime_for, safe_name, ChunkRef, MediaDescriptor, MediaKind, ALGO, MAX_SEND_BYTES};
use crate::progress::TransferStage;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};
use messenger_core::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;

/// Seconds between the attempts of one chunk.
#[cfg(not(test))]
const RETRY_DELAYS: [u64; 3] = [1, 2, 4];
#[cfg(test)]
const RETRY_DELAYS: [u64; 3] = [0, 0, 0];

/// Chunk sizes a stored state may name (the bounds of the descriptor).
const CHUNK_SIZES: std::ops::RangeInclusive<u64> = 64 * 1024..=64 * 1024 * 1024;
/// Encrypted chunks waiting for a worker. Memory holds at most the chunks
/// of the requests open, one waiting for a slot, these, and the one being
/// encrypted.
pub(crate) const SEALED_AHEAD: usize = 2;
/// A chunk sent before no longer matches the file.
const FILE_CHANGED: &str = "err.file_changed";

/// The wait before attempt `attempt + 1` of a chunk, with a little
/// randomness so workers that failed together do not come back together.
pub(crate) async fn backoff(attempt: usize) {
    let mut jitter = Duration::ZERO;
    if !cfg!(test) {
        let mut b = [0u8; 2];
        if getrandom::fill(&mut b).is_ok() {
            jitter = Duration::from_millis(u64::from(u16::from_le_bytes(b)) % 250);
        }
    }
    tokio::time::sleep(Duration::from_secs(RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)]) + jitter).await;
}

pub(crate) const CHUNK_ATTEMPTS: usize = RETRY_DELAYS.len();

fn changed() -> MessengerError {
    MessengerError::Invalid(FILE_CHANGED.into())
}

/// What `on_chunk` gives back for the state handed to it: resolves once
/// that state is kept, fails when it never will be. `None` when there is
/// nothing to wait for (tests, tools).
pub type Saved = Option<tokio::sync::oneshot::Receiver<()>>;

/// Wait until the state handed over is kept.
async fn kept(saved: Saved) -> Result<()> {
    match saved {
        Some(rx) => rx.await.map_err(|_| MessengerError::Storage("err.storage: the state of the upload was not kept".into())),
        None => Ok(()),
    }
}

/// The plaintext size of chunk `index` of a file of `size` bytes.
fn plain_len(size: u64, chunk_size: u64, index: usize) -> u64 {
    chunk_size.min(size - index as u64 * chunk_size)
}

/// Chunks on one server that an upload no longer uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleChunks {
    /// Public base of the server.
    pub base: String,
    /// Their hex SHA-256 names.
    pub chunks: Vec<String>,
    /// Those of them whose request was dropped: the server may still be
    /// writing one, so they are removed once more a while later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconfirmed: Vec<String>,
}

/// A chunk sent under the key of a state that is not known to be stored:
/// its request was dropped (a pause, a failure), or the server lost it.
/// The server may hold it, and its ciphertext went out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Unconfirmed {
    /// Chunk `index`: a resume must make this very ciphertext of it again,
    /// or start over with a new key.
    Chunk { index: u32, sha256: String },
    /// Noted before the index was kept: which chunk it was is not known,
    /// so the key is not used again.
    Unknown(String),
}

impl Unconfirmed {
    pub fn sha256(&self) -> &str {
        match self {
            Self::Chunk { sha256, .. } | Self::Unknown(sha256) => sha256,
        }
    }
}

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
    /// The mtime to the nanosecond, where the system tells it: a file
    /// rewritten within the same second is told apart too. A state of an
    /// earlier version has none and resumes on the seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mtime_ns: Option<i64>,
    /// Finished chunks by index; `None` for one not stored yet. A state
    /// written before chunks went side by side holds the finished ones in
    /// order without gaps, and reads the same.
    pub chunks: Vec<Option<ChunkRef>>,
    /// Hex SHA-256 state cannot be persisted, so the plaintext hash is
    /// recomputed on resume while the finished chunks are re-read.
    pub done: bool,
    /// Public base of the server the chunks went to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_base: Option<String>,
    /// Chunks sent to that server that are not known to be stored there:
    /// a cancel removes them too, and a resume holds them to the same
    /// ciphertext. A state of an earlier version lists their names only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconfirmed: Vec<Unconfirmed>,
    /// Chunks left on servers this upload no longer uses (it went on
    /// elsewhere, or under a new key), for the caller to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<StaleChunks>,
}

impl UploadState {
    pub fn file_key(&self) -> Result<FileKey> {
        let bad = |_| MessengerError::Invalid("stored upload key is not base64".into());
        FileKey::from_parts(&B64.decode(&self.key).map_err(bad)?, &B64.decode(&self.iv).map_err(bad)?)
    }

    /// Chunks of the whole file.
    pub fn chunks_total(&self) -> u32 {
        if self.chunk_size == 0 {
            0
        } else {
            self.file_size.div_ceil(self.chunk_size) as u32
        }
    }

    /// Chunks stored so far.
    pub fn chunks_done(&self) -> u32 {
        self.chunks.iter().flatten().count() as u32
    }

    /// The chunks stored so far.
    pub fn stored(&self) -> impl Iterator<Item = &ChunkRef> {
        self.chunks.iter().flatten()
    }

    /// Plaintext bytes of the chunks stored so far.
    pub fn stored_bytes(&self) -> u64 {
        let total = self.chunks_total() as usize;
        self.chunks
            .iter()
            .enumerate()
            .filter(|(i, c)| c.is_some() && *i < total)
            .map(|(i, _)| plain_len(self.file_size, self.chunk_size, i))
            .sum()
    }

    /// Every chunk the server of the upload holds or may hold, once each:
    /// what a cancel removes from it.
    pub fn on_server(&self) -> Vec<String> {
        let mut out: Vec<String> = self.stored().map(|c| c.sha256.clone()).collect();
        for u in &self.unconfirmed {
            if !out.iter().any(|s| s == u.sha256()) {
                out.push(u.sha256().to_string());
            }
        }
        out
    }

    /// Note chunks whose request was started (`tried`: index and hash) but
    /// which are not stored: the server may hold them.
    pub fn note_unconfirmed(&mut self, tried: impl IntoIterator<Item = (u32, String)>) {
        for (index, sha256) in tried {
            let stored = self.chunks.get(index as usize).is_some_and(|c| c.as_ref().is_some_and(|c| c.sha256 == sha256));
            let u = Unconfirmed::Chunk { index, sha256 };
            if !stored && !self.unconfirmed.contains(&u) {
                self.unconfirmed.push(u);
            }
        }
    }

    /// Chunk `index` is stored as `chunk`.
    fn store(&mut self, index: usize, chunk: ChunkRef) {
        self.unconfirmed.retain(|u| u.sha256() != chunk.sha256);
        self.chunks[index] = Some(chunk);
    }

    /// Chunk `index` is gone from the server: it is to be sent again, as
    /// the very same ciphertext.
    fn lost(&mut self, index: usize) {
        if let Some(c) = self.chunks[index].take() {
            self.note_unconfirmed([(index as u32, c.sha256)]);
        }
    }

    /// The ciphertext of every chunk that went out under the key, by
    /// index: what the file must make again. `None` when one chunk went
    /// out as two ciphertexts, or as one whose place is not known.
    fn sent(&self) -> Option<Vec<Option<String>>> {
        let mut sent: Vec<Option<String>> = self.chunks.iter().map(|c| c.as_ref().map(|c| c.sha256.clone())).collect();
        for u in &self.unconfirmed {
            let Unconfirmed::Chunk { index, sha256 } = u else { return None };
            match sent.get_mut(*index as usize)? {
                slot @ None => *slot = Some(sha256.clone()),
                Some(s) if s != sha256 => return None,
                Some(_) => {}
            }
        }
        Some(sent)
    }

    /// Leave everything on the server at `base` behind: it goes to `stale`
    /// and nothing counts as stored any more.
    pub(crate) fn leave(&mut self, base: &str) {
        let chunks = self.on_server();
        if !chunks.is_empty() {
            let unconfirmed = self.unconfirmed.iter().map(|u| u.sha256().to_string()).collect();
            self.stale.push(StaleChunks { base: base.to_string(), chunks, unconfirmed });
        }
        self.chunks.iter_mut().for_each(|c| *c = None);
        self.unconfirmed.clear();
    }

    /// Leave everything on the server at `base` behind and start over
    /// under a new key; what is left behind stays in `stale`.
    fn start_over(&mut self, base: &str, meta: &std::fs::Metadata, chunk_size: u64) -> Result<()> {
        let next = fresh(meta, chunk_size)?;
        self.leave(base);
        *self = UploadState { stale: std::mem::take(&mut self.stale), ..next };
        Ok(())
    }

    /// Can an upload of the file of `meta` go on from this state? Only for
    /// this very file, and when what went out under its key is known chunk
    /// by chunk.
    pub fn resumable(&self, meta: &std::fs::Metadata) -> bool {
        let size = meta.len();
        self.file_size == size
            && self.file_mtime == mtime(meta)
            && self.file_mtime_ns.is_none_or(|ns| Some(ns) == mtime_ns(meta))
            && CHUNK_SIZES.contains(&self.chunk_size)
            && self.file_key().is_ok()
            && self.unconfirmed.iter().all(|u| matches!(u, Unconfirmed::Chunk { index, .. } if (*index as u64) < size.div_ceil(self.chunk_size)))
    }

    /// Does `upload` of the file of `meta` look stored chunks up on the
    /// server at `base` first (the checking stage), given what this run
    /// counted done already?
    pub fn will_check(&self, meta: &std::fs::Metadata, base: &str, live: &crate::progress::Live) -> bool {
        self.resumable(meta)
            && self.server_base.as_deref().is_none_or(|b| b == base)
            && self.chunks.iter().enumerate().any(|(i, c)| c.is_some() && !live.is_done(i))
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
    /// Chunk size of a new upload instead of `chunk_size_for` (tests).
    pub chunk_size: Option<u64>,
}

fn modified(meta: &std::fs::Metadata) -> Option<Duration> {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    modified(meta).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn mtime_ns(meta: &std::fs::Metadata) -> Option<i64> {
    modified(meta).and_then(|d| i64::try_from(d.as_nanos()).ok())
}

/// The state of an upload that starts now, under a new key.
fn fresh(meta: &std::fs::Metadata, chunk_size: u64) -> Result<UploadState> {
    let k = FileKey::generate()?;
    Ok(UploadState {
        key: B64.encode(k.key),
        iv: B64.encode(k.base_nonce),
        chunk_size,
        file_size: meta.len(),
        file_mtime: mtime(meta),
        file_mtime_ns: mtime_ns(meta),
        ..Default::default()
    })
}

/// One encrypted chunk on its way to a worker.
struct Sealed {
    index: usize,
    cipher: Bytes,
    sha256: String,
    /// Its plaintext size: what it counts for in the progress.
    plain: u64,
}

impl Sealed {
    fn new(index: usize, cipher: Bytes, sha256: String, plain: u64) -> Self {
        #[cfg(test)]
        SEALED.with(|n| n.set(n.get() + 1));
        Self { index, cipher, sha256, plain }
    }
}

#[cfg(test)]
thread_local! {
    /// Encrypted chunks alive on this thread (a test runs its uploads on one).
    static SEALED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
impl Drop for Sealed {
    fn drop(&mut self) {
        SEALED.with(|n| n.set(n.get() - 1));
    }
}

/// Encrypted chunks alive on this thread now (tests).
#[cfg(test)]
pub(crate) fn sealed_alive() -> usize {
    SEALED.with(|n| n.get())
}

/// Can the file at `path` be sent: a file, not empty, within
/// `MAX_SEND_BYTES`. Its metadata and the name it goes under.
pub(crate) async fn sendable(path: &Path) -> Result<(std::fs::Metadata, String)> {
    let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
    if !meta.is_file() {
        return Err(MessengerError::Invalid("err.not_a_file".into()));
    }
    if meta.len() == 0 {
        return Err(MessengerError::Invalid("err.file_empty".into()));
    }
    if meta.len() > MAX_SEND_BYTES {
        return Err(MessengerError::Invalid("err.file_too_large".into()));
    }
    let name = safe_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    Ok((meta, name))
}

/// Upload `path`. `on_chunk(done_bytes, state)` is called with the state
/// to persist once it is fixed, before every chunk is sent and after every
/// stored chunk. Nothing is sent before the state that names it is kept
/// (`Saved`): a process killed in the middle of a request still leaves
/// the key, the server and the chunk for a cancel and a resume. A pause or
/// a cancel of `ctx.control` drops the requests in flight at once; the
/// chunks stored before stay in the state.
pub async fn upload(
    backend: &dyn BlobBackend,
    params: UploadParams<'_>,
    ctx: &TransferCtx,
    on_chunk: &(dyn Fn(u64, &UploadState) -> Saved + Send + Sync),
) -> Result<UploadOutcome> {
    let (meta, name) = sendable(params.path).await?;
    let size = meta.len();
    let mime = mime_for(&name).to_string();
    let here = backend.public_base();
    let chunk_size = params.chunk_size.filter(|c| CHUNK_SIZES.contains(c)).unwrap_or_else(|| chunk_size_for(size));

    let (mut state, mut new_key) = match params.resume {
        Some(s) => (s, false),
        None => (fresh(&meta, chunk_size)?, true),
    };
    // Another file now, or resumed on another server: what the old key
    // stored is left behind, to be removed, and the chunks go here anew
    // under a new key. A state of an earlier version names no server: its
    // chunks went to the first usable one, which is the one given here.
    // Under the old key the chunks would have the same names here, and
    // removing what is left there would remove them here too, should the
    // upload ever come back to it.
    if !new_key && (!state.resumable(&meta) || state.server_base.as_deref().is_some_and(|b| b != here)) {
        let base = state.server_base.clone().unwrap_or_else(|| here.clone());
        state.start_over(&base, &meta, chunk_size)?;
        new_key = true;
    }
    state.server_base = Some(here.clone());
    // A state of an earlier version learns the finer time now.
    if state.file_mtime_ns.is_none() {
        state.file_mtime_ns = mtime_ns(&meta);
    }
    let total = size.div_ceil(state.chunk_size) as usize;
    state.chunks.resize(total, None);
    state.done = false;
    if new_key {
        ctx.live.start_over(total as u32, state.chunk_size);
    } else {
        ctx.live.start(total as u32, state.chunk_size);
    }
    // What this run counted and the state no longer holds is to be sent.
    for (i, c) in state.chunks.iter().enumerate() {
        if c.is_none() {
            ctx.live.unmark(i);
        }
    }
    // The key and the server are kept before anything is sent: whatever
    // happens to the requests, the state names what they may have left.
    kept(on_chunk(state.stored_bytes(), &state)).await?;

    let mut key = state.file_key()?;
    let state = Mutex::new(state);
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let mut started_over = false;
    let sha256 = loop {
        let work = store_chunks(backend, params.path, &key, &state, ctx, on_chunk);
        let stored = tokio::select! {
            biased;
            stop = ctx.control.stopped() => {
                return Ok(if pauses(stop) { UploadOutcome::Paused } else { UploadOutcome::Cancelled });
            }
            stored = work => stored,
        };
        match stored {
            Ok(sha256) => break sha256,
            Err(e) if !started_over && matches!(&e, MessengerError::Invalid(c) if c == FILE_CHANGED) => {
                // The file changed under the same size and time: everything
                // sent under the old key is left behind, a new key starts.
                started_over = true;
                let saved = {
                    let mut s = lock();
                    s.start_over(&here, &meta, chunk_size)?;
                    s.server_base = Some(here.clone());
                    let total = size.div_ceil(s.chunk_size) as usize;
                    s.chunks.resize(total, None);
                    ctx.live.start_over(total as u32, s.chunk_size);
                    key = s.file_key()?;
                    on_chunk(0, &s)
                };
                kept(saved).await?;
            }
            Err(e) => return Err(e),
        }
    };
    let state = state.into_inner().unwrap_or_else(|e| e.into_inner());
    let chunks = state
        .chunks
        .iter()
        .cloned()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| MessengerError::Other("a chunk was not stored".into()))?;

    let mut descriptor = MediaDescriptor {
        kind: MediaKind::from_mime(&mime),
        name,
        mime,
        size,
        sha256,
        chunk_size: state.chunk_size,
        chunks,
        algo: ALGO.into(),
        key: String::new(),
        iv: String::new(),
        servers: vec![here],
        caption: params.caption.filter(|c| !c.trim().is_empty()),
        batch: params.batch,
        dim: None,
        duration_ms: None,
        waveform: None,
        thumb: None,
    };
    descriptor.set_key(&key);
    Ok(UploadOutcome::Done(Box::new(descriptor)))
}

/// Check what an earlier attempt stored, then store the rest. Returns the
/// hex SHA-256 of the plaintext; `state` ends complete.
async fn store_chunks(
    backend: &dyn BlobBackend,
    path: &Path,
    key: &FileKey,
    state: &Mutex<UploadState>,
    ctx: &TransferCtx,
    on_chunk: &(dyn Fn(u64, &UploadState) -> Saved + Send + Sync),
) -> Result<String> {
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    let (size, chunk_size) = {
        let s = lock();
        (s.file_size, s.chunk_size)
    };

    // Checking: what an earlier attempt stored may be gone from the server.
    // Chunks this run stored or found already are not asked about again.
    let known: Vec<(usize, String)> = lock()
        .chunks
        .iter()
        .enumerate()
        .filter(|(i, _)| !ctx.live.is_done(*i))
        .filter_map(|(i, c)| c.as_ref().map(|c| (i, c.sha256.clone())))
        .collect();
    if !known.is_empty() {
        ctx.live.set_stage(TransferStage::Checking);
        let mut checks = futures_util::stream::iter(known)
            .map(|(i, sha)| async move { exists_with_retry(backend, ctx, &sha).await.map(|there| (i, there)) })
            .buffer_unordered(ctx.workers);
        // A lookup that fails tells nothing: the attempt fails with it, and
        // the state keeps what it had.
        while let Some((i, there)) = checks.try_next().await? {
            if there {
                ctx.live.found(i, plain_len(size, chunk_size, i));
            } else {
                lock().lost(i);
            }
        }
        on_chunk(ctx.live.confirmed(), &lock());
    }

    ctx.live.set_stage(TransferStage::Uploading);
    let (sent, put) = {
        let s = lock();
        (s.sent().ok_or_else(changed)?, s.chunks.iter().map(Option::is_none).collect())
    };
    let (tx, rx) = mpsc::channel::<Sealed>(SEALED_AHEAD);
    let rx = tokio::sync::Mutex::new(rx);
    let reader = seal_chunks(path, key, size, chunk_size, sent, put, tx);
    let workers = futures_util::future::try_join_all((0..ctx.workers).map(|_| store_worker(backend, ctx, &rx, state, on_chunk)));
    // The first chunk that fails for good stops the others where they are.
    let (sha256, _) = tokio::try_join!(reader, workers)?;

    let mut s = lock();
    s.done = true;
    on_chunk(size, &s);
    Ok(sha256)
}

/// Is the chunk on the server? A few quick retries for a server that
/// hiccups; a lookup that still fails is an error, never "not there". A
/// server that refuses the lookup itself (an S3 key without the right to
/// list answers 403 for a blob it does not have, some Blossom servers 401
/// or 405) tells nothing either way: the chunk is taken for lost and sent
/// again, as the very same ciphertext.
async fn exists_with_retry(backend: &dyn BlobBackend, ctx: &TransferCtx, sha256: &str) -> Result<bool> {
    let mut attempt = 0;
    loop {
        let result = {
            let _slot = ctx.slot().await?;
            backend.exists(sha256).await
        };
        match result {
            Ok(there) => return Ok(there),
            Err(e) if e.is_retryable() && attempt < CHUNK_ATTEMPTS => {
                backoff(attempt).await;
                attempt += 1;
            }
            Err(e) if e.is_retryable() => return Err(e.into()),
            Err(_) => return Ok(false),
        }
    }
}

/// The next `len` bytes of the file.
async fn read_chunk(file: &mut tokio::fs::File, len: u64) -> Result<Vec<u8>> {
    let mut plain = vec![0u8; len as usize];
    file.read_exact(&mut plain).await.map_err(|e| MessengerError::Io(e.to_string()))?;
    Ok(plain)
}

/// Hash `plain` into `h` and encrypt it as chunk `index`, with the hash
/// of the ciphertext; off the async threads.
async fn seal(key: &FileKey, index: usize, plain: Vec<u8>, mut h: Sha256) -> Result<(Sha256, Vec<u8>, String)> {
    let key = key.clone();
    tokio::task::spawn_blocking(move || -> Result<_> {
        h.update(&plain);
        let cipher = key.encrypt_chunk(index as u32, &plain)?;
        let sha256 = sha256_hex(&cipher);
        Ok((h, cipher, sha256))
    })
    .await
    .map_err(|e| MessengerError::Other(e.to_string()))?
}

/// Read the file in order, hash it, encrypt every chunk and hand on those
/// to `put`. A chunk whose ciphertext went out before under this key
/// (`sent`) must come out the same, and every one of them is proven so
/// before a chunk that never went out is encrypted. Each chunk is read
/// once for the plaintext hash and its ciphertext together, so the
/// descriptor is true to the chunks even when the file changes meanwhile.
async fn seal_chunks(
    path: &Path,
    key: &FileKey,
    size: u64,
    chunk_size: u64,
    sent: Vec<Option<String>>,
    put: Vec<bool>,
    tx: mpsc::Sender<Sealed>,
) -> Result<String> {
    let plain_len = |i: usize| plain_len(size, chunk_size, i);
    let mut file = tokio::fs::File::open(path).await?;

    // Chunks sent before that come later in the file than the first new
    // one are proven first.
    let first_new = sent.iter().position(Option::is_none).unwrap_or(sent.len());
    for (index, want) in sent.iter().enumerate().skip(first_new) {
        let Some(want) = want else { continue };
        file.seek(std::io::SeekFrom::Start(index as u64 * chunk_size)).await?;
        let plain = read_chunk(&mut file, plain_len(index)).await?;
        let (_, _, sha256) = seal(key, index, plain, Sha256::new()).await?;
        if sha256 != *want {
            return Err(changed());
        }
    }
    file.seek(std::io::SeekFrom::Start(0)).await?;

    let mut hasher = Sha256::new();
    for (index, (want, put)) in sent.into_iter().zip(put).enumerate() {
        let plain = read_chunk(&mut file, plain_len(index)).await?;
        let (h, cipher, sha256) = seal(key, index, plain, std::mem::take(&mut hasher)).await?;
        hasher = h;
        if want.is_some_and(|w| w != sha256) {
            return Err(changed());
        }
        if put {
            let chunk = Sealed::new(index, Bytes::from(cipher), sha256, plain_len(index));
            if tx.send(chunk).await.is_err() {
                // The workers stopped: their error is the answer.
                break;
            }
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Store chunks from the reader until there are none left.
async fn store_worker(
    backend: &dyn BlobBackend,
    ctx: &TransferCtx,
    rx: &tokio::sync::Mutex<mpsc::Receiver<Sealed>>,
    state: &Mutex<UploadState>,
    on_chunk: &(dyn Fn(u64, &UploadState) -> Saved + Send + Sync),
) -> Result<()> {
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        // A chunk first, then a slot for it: a worker never holds a slot
        // while the reader has nothing to send (it re-reads what is stored,
        // for the plaintext hash), so other transfers keep their share. One
        // worker at a time waits for a slot with a chunk in hand; encrypted
        // chunks wait in the queue, not in workers that could not send them.
        let (chunk, slot) = {
            let mut rx = rx.lock().await;
            let Some(chunk) = rx.recv().await else { return Ok(()) };
            (chunk, ctx.slot().await?)
        };
        // Kept as sent before it goes: a process killed in the middle of
        // the request still leaves its name for a cancel, and its
        // ciphertext for a resume to hold the key to.
        let saved = {
            let s = &mut *lock();
            s.note_unconfirmed([(chunk.index as u32, chunk.sha256.clone())]);
            on_chunk(ctx.live.confirmed(), s)
        };
        kept(saved).await?;
        put_with_retry(backend, ctx, &chunk, slot).await?;
        let mut s = lock();
        s.store(chunk.index, ChunkRef { sha256: chunk.sha256.clone(), size: chunk.cipher.len() as u64 });
        on_chunk(ctx.live.confirmed(), &s);
    }
}

/// One chunk, in the slot taken for it, with a few quick retries for a
/// server that hiccups. The slot is given back while waiting.
async fn put_with_retry(backend: &dyn BlobBackend, ctx: &TransferCtx, chunk: &Sealed, slot: Slot) -> Result<()> {
    let mut slot = Some(slot);
    let mut attempt = 0;
    loop {
        let result = {
            let _slot = match slot.take() {
                Some(s) => s,
                None => ctx.slot().await?,
            };
            let flight = ctx.live.begin(chunk.plain);
            let r = backend.put_counted(&chunk.sha256, chunk.cipher.clone(), flight.sent.clone()).await;
            if r.is_ok() {
                flight.confirm(chunk.index);
            }
            r
        };
        match result {
            Ok(()) => return Ok(()),
            Err(e) if e.is_retryable() && attempt < CHUNK_ATTEMPTS => {
                backoff(attempt).await;
                attempt += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use crate::control::PAUSE;

    const KIB: usize = 1024;

    fn content(len: usize) -> Vec<u8> {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0xff) as u8
            })
            .collect()
    }

    fn fixed_key() -> FileKey {
        FileKey { key: [0x5a; 32], base_nonce: [0xa5; 12] }
    }

    /// A state that names the key only: the upload uses that key.
    fn keyed(path: &Path, chunk_size: u64) -> UploadState {
        let meta = std::fs::metadata(path).unwrap();
        let k = fixed_key();
        UploadState {
            key: B64.encode(k.key),
            iv: B64.encode(k.base_nonce),
            chunk_size,
            file_size: meta.len(),
            file_mtime: mtime(&meta),
            ..Default::default()
        }
    }

    /// What the chunk-after-chunk upload made of a file under a key.
    fn sequential(plain: &[u8], key: &FileKey, chunk_size: usize) -> (Vec<ChunkRef>, Vec<Vec<u8>>, String) {
        let mut chunks = Vec::new();
        let mut ciphers = Vec::new();
        for (i, piece) in plain.chunks(chunk_size).enumerate() {
            let c = key.encrypt_chunk(i as u32, piece).unwrap();
            chunks.push(ChunkRef { sha256: sha256_hex(&c), size: c.len() as u64 });
            ciphers.push(c);
        }
        (chunks, ciphers, sha256_hex(plain))
    }

    /// Wait (up to ten seconds) until `check` holds.
    async fn until(check: impl Fn() -> bool) {
        for _ in 0..2000 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the condition never held");
    }

    fn params(path: &Path, resume: Option<UploadState>) -> UploadParams<'_> {
        UploadParams { path, caption: None, batch: None, resume, chunk_size: Some(64 * KIB as u64) }
    }

    /// The last state handed out, and how many times.
    #[derive(Default)]
    struct Kept(Mutex<(Option<UploadState>, u32)>);
    impl Kept {
        fn keep(&self, _: u64, s: &UploadState) -> Saved {
            let mut k = self.0.lock().unwrap();
            k.0 = Some(s.clone());
            k.1 += 1;
            None
        }
        fn state(&self) -> UploadState {
            self.0.lock().unwrap().0.clone().unwrap()
        }
    }

    #[tokio::test]
    async fn parallel_upload_makes_the_sequential_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many.bin");
        let plain = content(64 * KIB * 37 + 1000);
        std::fs::write(&path, &plain).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        // The puts hang until four are under way at once, then all go.
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *backend.put_gate.lock().unwrap() = Some(gate.clone());
        let ctx = TransferCtx::standalone(4);
        let kept = Kept::default();
        let keep = |d, s: &UploadState| kept.keep(d, s);
        let run = upload(&backend, params(&path, Some(keyed(&path, 64 * KIB as u64))), &ctx, &keep);
        let open = async {
            until(|| backend.puts_in_flight.load(std::sync::atomic::Ordering::SeqCst) == 4).await;
            gate.add_permits(1000);
        };
        let (out, ()) = tokio::join!(run, open);
        let out = out.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        d.validate().unwrap();

        let (chunks, ciphers, sha) = sequential(&plain, &fixed_key(), 64 * KIB);
        assert_eq!(d.chunks, chunks, "same order, same hashes");
        assert_eq!(d.sha256, sha);
        assert_eq!(d.file_key().unwrap(), fixed_key());
        assert_eq!(d.chunks.len(), 38);
        for (c, bytes) in chunks.iter().zip(&ciphers) {
            assert_eq!(&backend.get(&c.sha256).unwrap(), bytes);
        }
        assert_eq!(backend.max_puts_in_flight.load(std::sync::atomic::Ordering::SeqCst), 4, "chunks went side by side, four at most");
        assert_eq!(*backend.put_calls.lock().unwrap(), 38, "no chunk twice");
        assert_eq!(backend.exists_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "a fresh chunk is put without asking first");
        let last = kept.state();
        assert!(last.done && last.chunks_done() == 38);
        assert_eq!(last.server_base.as_deref(), Some("https://mem.example/a"));
        assert_eq!(ctx.live.chunks_done(), 38);
        assert_eq!(ctx.live.confirmed(), plain.len() as u64);
    }

    #[tokio::test]
    async fn a_dead_server_keeps_what_was_stored_and_the_retry_sends_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let plain = content(64 * KIB * 20);
        std::fs::write(&path, &plain).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        *backend.die_after_puts.lock().unwrap() = Some(5);
        let kept = Kept::default();
        let err = upload(&backend, params(&path, None), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        assert!(err.to_string().contains("err.network"), "{err}");
        let after = kept.state();
        assert_eq!(after.chunks.len(), 20);
        assert_eq!(after.chunks_done(), 5, "every stored chunk is in the state, wherever it is in the file");
        assert_eq!(backend.len(), 5);

        *backend.die_after_puts.lock().unwrap() = None;
        let before = *backend.put_calls.lock().unwrap();
        let ctx = TransferCtx::standalone(4);
        let out = upload(&backend, params(&path, Some(after.clone())), &ctx, &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!() };
        assert_eq!(*backend.put_calls.lock().unwrap() - before, 15, "only the missing chunks");
        assert_eq!(d.key, after.key, "the same key");
        let key = d.file_key().unwrap();
        assert_eq!(d.chunks, sequential(&plain, &key, 64 * KIB).0);
        for (i, c) in after.chunks.iter().enumerate() {
            if let Some(c) = c {
                assert_eq!(&d.chunks[i], c);
            }
        }
    }

    #[tokio::test]
    async fn chunks_stored_out_of_order_are_kept_and_only_the_gaps_sent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gaps.bin");
        let plain = content(64 * KIB * 20);
        std::fs::write(&path, &plain).unwrap();
        let (chunks, _, _) = sequential(&plain, &fixed_key(), 64 * KIB);
        let backend = MemoryBackend::new("https://mem.example/a");
        // The first chunk hangs; the others go on past it.
        *backend.hang_sha.lock().unwrap() = Some(chunks[0].sha256.clone());
        let ctx = TransferCtx::standalone(4);
        let kept = Kept::default();
        let pause_at_ten = |d: u64, s: &UploadState| {
            if s.chunks_done() >= 10 {
                ctx.control.set(PAUSE);
            }
            kept.keep(d, s)
        };
        let out = upload(&backend, params(&path, Some(keyed(&path, 64 * KIB as u64))), &ctx, &pause_at_ten).await.unwrap();
        assert_eq!(out, UploadOutcome::Paused);
        let s = kept.state();
        assert!(s.chunks[0].is_none(), "the first chunk never got through");
        let stored: Vec<usize> = (0..20).filter(|i| s.chunks[*i].is_some()).collect();
        assert!(stored.len() >= 10, "{stored:?}");
        for i in &stored {
            assert_eq!(s.chunks[*i].as_ref(), Some(&chunks[*i]), "chunk {i} in its own place");
        }
        assert_eq!(stored.len(), backend.len(), "every stored chunk is in the state");

        *backend.hang_sha.lock().unwrap() = None;
        let puts = *backend.put_calls.lock().unwrap();
        let out = upload(&backend, params(&path, Some(s)), &TransferCtx::standalone(4), &|_, _| None).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_eq!(*backend.put_calls.lock().unwrap() - puts, 20 - stored.len() as u32, "exactly the gaps");
        assert_eq!(d.chunks, chunks);
    }

    #[tokio::test]
    async fn a_resume_on_another_server_leaves_the_first_ones_chunks_to_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("move.bin");
        std::fs::write(&path, content(64 * KIB * 6)).unwrap();
        let a = MemoryBackend::new("https://a.example");
        *a.die_after_puts.lock().unwrap() = Some(2);
        let kept = Kept::default();
        upload(&a, params(&path, None), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        let first = kept.state();
        assert_eq!((first.chunks_done(), first.server_base.as_deref()), (2, Some("https://a.example")));

        // The first server is no longer used: all goes to the second, under
        // a new key.
        let b = MemoryBackend::new("https://b.example");
        let out = upload(&b, params(&path, Some(first.clone())), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_eq!(d.servers, vec!["https://b.example".to_string()]);
        assert_ne!(d.key, first.key, "never the same names on two servers");
        assert_eq!(b.len(), 6);
        assert_eq!(b.exists_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing is asked of a server that never had them");
        let last = kept.state();
        assert!(!first.unconfirmed.is_empty(), "the requests that failed may have landed: {first:?}");
        let left = StaleChunks { base: "https://a.example".into(), chunks: first.on_server(), unconfirmed: unconfirmed_of(&first) };
        assert_eq!(last.stale, vec![left], "what the first one kept is left to remove");
        assert_eq!(last.server_base.as_deref(), Some("https://b.example"));
    }

    fn unconfirmed_of(s: &UploadState) -> Vec<String> {
        s.unconfirmed.iter().map(|u| u.sha256().to_string()).collect()
    }

    /// A, then B, then back to A before what A kept was removed (the app
    /// was killed first): what is left to remove on A never names a chunk
    /// the descriptor names.
    #[tokio::test]
    async fn an_upload_back_on_its_first_server_never_removes_what_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("back.bin");
        let plain = content(64 * KIB * 6);
        std::fs::write(&path, &plain).unwrap();
        let (a, b) = (MemoryBackend::new("https://a.example"), MemoryBackend::new("https://b.example"));
        *a.die_after_puts.lock().unwrap() = Some(2);
        *b.die_after_puts.lock().unwrap() = Some(2);
        let kept = Kept::default();
        let resume = Some(keyed(&path, 64 * KIB as u64));
        upload(&a, params(&path, resume), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        upload(&b, params(&path, Some(kept.state())), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        let on_b = kept.state();
        assert_eq!(on_b.server_base.as_deref(), Some("https://b.example"));
        assert!(on_b.stale.iter().any(|s| s.base == "https://a.example"), "{on_b:?}");

        *a.die_after_puts.lock().unwrap() = None;
        let out = upload(&a, params(&path, Some(on_b.clone())), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        let last = kept.state();
        for s in last.stale.iter().filter(|s| s.base == "https://a.example") {
            assert!(d.chunks.iter().all(|c| !s.chunks.contains(&c.sha256)), "a chunk of the message would be removed");
        }
        assert!(last.stale.iter().any(|s| s.base == "https://b.example"), "what B kept goes too");
        let key = d.file_key().unwrap();
        assert_ne!(key, fixed_key());
        assert_ne!(d.key, on_b.key);
        assert_eq!(d.chunks, sequential(&plain, &key, 64 * KIB).0);
    }

    #[tokio::test]
    async fn a_file_rewritten_under_the_same_time_starts_over_with_a_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.bin");
        let old = content(64 * KIB * 8);
        std::fs::write(&path, &old).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        *backend.die_after_puts.lock().unwrap() = Some(3);
        let kept = Kept::default();
        let resume = Some(keyed(&path, 64 * KIB as u64));
        upload(&backend, params(&path, resume), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        let first = kept.state();
        assert_eq!(first.chunks_done(), 3);
        assert!(first.file_mtime_ns.is_some(), "the finer time is kept");

        // Rewritten in place, the same size, its time put back (cp -p, rsync -t).
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let new: Vec<u8> = old.iter().map(|b| !b).collect();
        std::fs::write(&path, &new).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
        *backend.die_after_puts.lock().unwrap() = None;
        let out = upload(&backend, params(&path, Some(first.clone())), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s))
            .await
            .unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_ne!(d.key, first.key, "never the old key for new content");
        assert_eq!(d.chunks, sequential(&new, &d.file_key().unwrap(), 64 * KIB).0);
        assert_eq!(d.sha256, sha256_hex(&new));
        let old_key = first.file_key().unwrap();
        for c in sequential(&new, &old_key, 64 * KIB).0 {
            assert!(backend.get(&c.sha256).is_none(), "nothing new was encrypted under the old key");
        }
        let left = StaleChunks { base: "https://mem.example/a".into(), chunks: first.on_server(), unconfirmed: unconfirmed_of(&first) };
        assert_eq!(kept.state().stale, vec![left.clone()], "what the old key stored is left to remove");

        // A time that differs below the second tells it at once, unread.
        let mut finer = first.clone();
        finer.file_mtime_ns = finer.file_mtime_ns.map(|n| n + 1);
        let out = upload(&backend, params(&path, Some(finer)), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_ne!(d.key, first.key);
        assert_eq!(kept.state().stale, vec![left]);
    }

    #[tokio::test]
    async fn a_state_of_the_old_format_resumes() {
        // Written before chunks went side by side: finished chunks in
        // order, no gaps, no server.
        let literal = r#"{"key":"WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo=","iv":"paWlpaWlpaWlpaWl","chunk_size":4194304,"file_size":10,"file_mtime":5,"chunks":[{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":26}],"done":false}"#;
        let s: UploadState = serde_json::from_str(literal).unwrap();
        assert_eq!(s.chunks, vec![Some(ChunkRef { sha256: "a".repeat(64), size: 26 })]);
        assert_eq!(s.server_base, None);
        assert_eq!(s.file_key().unwrap(), fixed_key());
        assert!(!serde_json::to_string(&UploadState::default()).unwrap().contains("server_base"));

        // A real one: three of eight chunks stored by the old code.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.bin");
        let plain = content(64 * KIB * 7 + 5);
        std::fs::write(&path, &plain).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let (chunks, ciphers, sha) = sequential(&plain, &fixed_key(), 64 * KIB);
        let backend = MemoryBackend::new("https://mem.example/a");
        for (c, bytes) in chunks.iter().zip(&ciphers).take(3) {
            backend.put(&c.sha256, bytes.clone()).await.unwrap();
        }
        let old = format!(
            r#"{{"key":"WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo=","iv":"paWlpaWlpaWlpaWl","chunk_size":65536,"file_size":{},"file_mtime":{},"chunks":[{}],"done":false}}"#,
            meta.len(),
            mtime(&meta),
            chunks[..3].iter().map(|c| format!(r#"{{"sha256":"{}","size":{}}}"#, c.sha256, c.size)).collect::<Vec<_>>().join(",")
        );
        let resume: UploadState = serde_json::from_str(&old).unwrap();
        let before = *backend.put_calls.lock().unwrap();
        let ctx = TransferCtx::standalone(4);
        let out = upload(&backend, params(&path, Some(resume)), &ctx, &|_, _| None).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!() };
        assert_eq!(*backend.put_calls.lock().unwrap() - before, 5);
        assert_eq!((d.chunks, d.sha256), (chunks, sha));
    }

    #[tokio::test]
    async fn too_big_to_send_is_refused_without_reading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.bin");
        // Sparse: nothing is written.
        std::fs::File::create(&path).unwrap().set_len(MAX_SEND_BYTES + 1).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        let err = upload(&backend, params(&path, None), &TransferCtx::standalone(4), &|_, _| None).await.unwrap_err();
        assert!(err.to_string().contains("err.file_too_large"), "{err}");
        assert_eq!(*backend.put_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_refused_chunk_stops_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin");
        std::fs::write(&path, content(64 * KIB * 30)).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        *backend.reject_puts.lock().unwrap() = true;
        let ctx = TransferCtx::standalone(4);
        let err = upload(&backend, params(&path, None), &ctx, &|_, _| None).await.unwrap_err();
        assert!(err.to_string().contains("err.auth_failed"), "{err}");
        assert!(*backend.put_calls.lock().unwrap() <= 4, "nothing new is started after a refusal");
        assert_eq!(ctx.live.done_bytes(), 0, "nothing counts that the server did not take");
    }

    /// The server does not answer the lookups of a resume: nothing it
    /// stored is taken for lost, nothing is sent again, and the state
    /// keeps every chunk.
    #[tokio::test]
    async fn a_lookup_that_fails_keeps_what_was_stored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        std::fs::write(&path, content(64 * KIB * 20)).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        *backend.die_after_puts.lock().unwrap() = Some(5);
        let kept = Kept::default();
        upload(&backend, params(&path, None), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        let before = kept.state();
        assert_eq!(before.chunks_done(), 5);

        *backend.die_after_puts.lock().unwrap() = None;
        *backend.fail_exists.lock().unwrap() = true;
        let puts = *backend.put_calls.lock().unwrap();
        let ctx = TransferCtx::standalone(4);
        let err = upload(&backend, params(&path, Some(before.clone())), &ctx, &|d, s| kept.keep(d, s)).await.unwrap_err();
        assert!(err.to_string().contains("err.network"), "a failure that may pass: {err}");
        assert_eq!(*backend.put_calls.lock().unwrap(), puts, "nothing is sent again");
        let after = kept.state();
        assert_eq!(after.chunks, before.chunks, "every stored chunk is still in the state");
        assert!(before.stored().all(|c| after.on_server().contains(&c.sha256)), "a cancel still finds them all");

        // The server answers again: only the missing chunks go.
        *backend.fail_exists.lock().unwrap() = false;
        let out = upload(&backend, params(&path, Some(after)), &TransferCtx::standalone(4), &|_, _| None).await.unwrap();
        assert!(matches!(out, UploadOutcome::Done(_)), "{out:?}");
        assert_eq!(*backend.put_calls.lock().unwrap() - puts, 15);
    }

    /// A stored chunk the server lost is sent again as the very same
    /// ciphertext. The file changed there meanwhile (its time kept): a new
    /// key starts, and the old one never encrypts the new bytes.
    #[tokio::test]
    async fn a_lost_chunk_of_a_changed_file_is_never_sent_under_the_old_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let old = content(64 * KIB * 4);
        std::fs::write(&path, &old).unwrap();
        let (chunks, ciphers, _) = sequential(&old, &fixed_key(), 64 * KIB);
        let backend = MemoryBackend::new("https://mem.example/a");
        for (c, bytes) in chunks.iter().zip(&ciphers) {
            backend.put(&c.sha256, bytes.clone()).await.unwrap();
        }
        let mut state = keyed(&path, 64 * KIB as u64);
        state.chunks = chunks.iter().cloned().map(Some).collect();
        state.server_base = Some(backend.public_base());

        // The server lost chunk 0, and the header was edited in place.
        backend.delete(&chunks[0].sha256).await.unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut new = old.clone();
        new[..16].copy_from_slice(b"edited in place!");
        std::fs::write(&path, &new).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();

        let kept = Kept::default();
        let out = upload(&backend, params(&path, Some(state)), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        let key = d.file_key().unwrap();
        assert_ne!(key, fixed_key(), "a new key for the new bytes");
        let reused = sequential(&new, &fixed_key(), 64 * KIB).0;
        assert!(backend.get(&reused[0].sha256).is_none(), "the old key never encrypted them");
        assert_eq!(d.chunks, sequential(&new, &key, 64 * KIB).0);
        let stale = kept.state().stale;
        assert!(stale.iter().any(|s| s.chunks.contains(&chunks[0].sha256)), "the lost one is removed too: {stale:?}");
    }

    /// The file changes while a resume with a gap runs, after the chunks
    /// past the gap were proven: they are proven again as they are read
    /// for the plaintext hash, so the descriptor never names ciphertext of
    /// other bytes than it hashed.
    #[tokio::test]
    async fn a_file_that_changes_during_a_resume_never_makes_a_torn_descriptor() {
        use std::io::{Seek, Write};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("torn.bin");
        let plain = content(64 * KIB * 12);
        std::fs::write(&path, &plain).unwrap();
        let (chunks, ciphers, _) = sequential(&plain, &fixed_key(), 64 * KIB);
        let backend = MemoryBackend::new("https://mem.example/a");
        // Chunks 0 and 8 to 11 are stored, 1 to 7 are not.
        let mut state = keyed(&path, 64 * KIB as u64);
        state.server_base = Some(backend.public_base());
        state.chunks = vec![None; 12];
        for i in [0, 8, 9, 10, 11] {
            backend.put(&chunks[i].sha256, ciphers[i].clone()).await.unwrap();
            state.chunks[i] = Some(chunks[i].clone());
        }
        // The gap's chunks hang: the reader stops once the workers and its
        // queue are full, before it reads chunk 8 again.
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        *backend.put_gate.lock().unwrap() = Some(gate.clone());
        let ctx = TransferCtx::standalone(4);
        let run = upload(&backend, params(&path, Some(state)), &ctx, &|_, _| None);
        let edit = async {
            while backend.puts_in_flight.load(std::sync::atomic::Ordering::SeqCst) < 4 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            // The end of the file is rewritten in place, its size kept.
            let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.seek(std::io::SeekFrom::Start(64 * KIB as u64 * 8)).unwrap();
            f.write_all(&vec![7u8; 64 * KIB * 4]).unwrap();
            drop(f);
            gate.add_permits(1000);
        };
        let (out, ()) = tokio::join!(run, edit);
        let UploadOutcome::Done(d) = out.unwrap() else { panic!() };
        let key = d.file_key().unwrap();
        let mut whole = Vec::new();
        for (i, c) in d.chunks.iter().enumerate() {
            whole.extend(key.decrypt_chunk(i as u32, &backend.get(&c.sha256).unwrap()).unwrap());
        }
        assert_eq!(sha256_hex(&whole), d.sha256, "what the chunks hold is what was hashed");
        assert_ne!(key, fixed_key(), "the old key never met the new bytes");
    }

    /// The first version of side-by-side chunks kept the names of dropped
    /// chunks without their place. Which chunk such a name was is unknown,
    /// so its key is not used again.
    #[tokio::test]
    async fn chunks_sent_at_unknown_places_retire_the_key() {
        let literal = format!(
            r#"{{"key":"WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo=","iv":"paWlpaWlpaWlpaWl","chunk_size":65536,"file_size":10,"file_mtime":5,"chunks":[],"done":false,"unconfirmed":["{}"]}}"#,
            "b".repeat(64)
        );
        let s: UploadState = serde_json::from_str(&literal).unwrap();
        assert_eq!(s.unconfirmed, vec![Unconfirmed::Unknown("b".repeat(64))]);
        let mut t = s.clone();
        t.unconfirmed = vec![Unconfirmed::Chunk { index: 3, sha256: "c".repeat(64) }];
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains(r#"{"index":3,"sha256":"#), "{json}");
        assert_eq!(serde_json::from_str::<UploadState>(&json).unwrap(), t);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin");
        std::fs::write(&path, content(64 * KIB * 3)).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        let mut state = keyed(&path, 64 * KIB as u64);
        state.server_base = Some(backend.public_base());
        state.unconfirmed = vec![Unconfirmed::Unknown("b".repeat(64))];
        let kept = Kept::default();
        let out = upload(&backend, params(&path, Some(state)), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_ne!(d.file_key().unwrap(), fixed_key());
        let stale = StaleChunks { base: backend.public_base(), chunks: vec!["b".repeat(64)], unconfirmed: vec!["b".repeat(64)] };
        assert_eq!(kept.state().stale, vec![stale], "and it is removed, once more a while later");
    }

    /// A server that refuses the lookups of a resume (403 for a blob it
    /// lost, where the key may not list the bucket) tells nothing: the
    /// chunks are sent again, as the very same ciphertext, and the resume
    /// ends well.
    #[tokio::test]
    async fn a_refused_lookup_takes_the_chunk_for_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let plain = content(64 * KIB * 20);
        std::fs::write(&path, &plain).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        *backend.die_after_puts.lock().unwrap() = Some(5);
        let kept = Kept::default();
        upload(&backend, params(&path, None), &TransferCtx::standalone(4), &|d, s| kept.keep(d, s)).await.unwrap_err();
        let before = kept.state();
        assert_eq!(before.chunks_done(), 5);

        *backend.die_after_puts.lock().unwrap() = None;
        *backend.exists_status.lock().unwrap() = Some(403);
        let puts = *backend.put_calls.lock().unwrap();
        let out = upload(&backend, params(&path, Some(before.clone())), &TransferCtx::standalone(4), &|_, _| None).await.unwrap();
        let UploadOutcome::Done(d) = out else { panic!("{out:?}") };
        assert_eq!(*backend.put_calls.lock().unwrap() - puts, 20, "the five stored ones go again too");
        assert_eq!(d.key, before.key, "the same key");
        assert_eq!(d.chunks, sequential(&plain, &d.file_key().unwrap(), 64 * KIB).0);
        for c in before.stored() {
            assert!(d.chunks.contains(c), "the very same ciphertext");
        }
    }

    /// Workers waiting for the reader (it re-reads what is stored, for the
    /// plaintext hash) hold no chunk slot: other transfers keep them.
    #[tokio::test]
    async fn a_worker_waiting_for_the_reader_holds_no_slot() {
        let backend = MemoryBackend::new("https://mem.example/a");
        let ctx = TransferCtx::standalone(4);
        let (tx, rx) = mpsc::channel::<Sealed>(SEALED_AHEAD);
        let rx = tokio::sync::Mutex::new(rx);
        let state = Mutex::new(UploadState::default());
        let workers = futures_util::future::try_join_all((0..4).map(|_| store_worker(&backend, &ctx, &rx, &state, &|_, _| None)));
        let free = tokio::select! {
            _ = workers => panic!("the reader is still there"),
            free = async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                ctx.slots.available_permits()
            } => free,
        };
        assert_eq!(free, crate::control::CHUNK_SLOTS, "no slot is held while no chunk is there");
        drop(tx);
    }

    /// The key and every chunk are kept before they go out: while the
    /// state that names them is not kept, nothing is sent, and a state that
    /// cannot be kept stops the upload before anything goes.
    #[tokio::test]
    async fn nothing_goes_out_before_the_state_that_names_it_is_kept() {
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
        use tokio::sync::oneshot;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.bin");
        std::fs::write(&path, content(64 * KIB * 6)).unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        let puts = || *backend.put_calls.lock().unwrap();
        // States handed over and not kept yet.
        let waiting: Mutex<Vec<(UploadState, oneshot::Sender<()>)>> = Mutex::default();
        let hold = AtomicBool::new(true);
        let on_chunk = |_: u64, s: &UploadState| -> Saved {
            if !hold.load(SeqCst) {
                return None;
            }
            let (tx, rx) = oneshot::channel();
            waiting.lock().unwrap().push((s.clone(), tx));
            Some(rx)
        };
        let ctx = TransferCtx::standalone(4);
        let run = upload(&backend, params(&path, None), &ctx, &on_chunk);
        let keep = async {
            until(|| !waiting.lock().unwrap().is_empty()).await;
            assert_eq!(puts(), 0, "nothing before the key is kept");
            let (first, tx) = waiting.lock().unwrap().remove(0);
            assert!(!first.key.is_empty() && first.server_base.is_some(), "{first:?}");
            tx.send(()).unwrap();
            until(|| !waiting.lock().unwrap().is_empty()).await;
            assert_eq!(puts(), 0, "nor a chunk before its name is kept");
            {
                let named = waiting.lock().unwrap();
                assert!(!named.is_empty() && named.iter().all(|(s, _)| !s.unconfirmed.is_empty()), "the chunks about to go are named");
            }
            hold.store(false, SeqCst);
            for (_, tx) in waiting.lock().unwrap().drain(..) {
                let _ = tx.send(());
            }
        };
        let (out, ()) = tokio::join!(run, keep);
        assert!(matches!(out.unwrap(), UploadOutcome::Done(_)));
        assert_eq!(puts(), 6);

        let other = MemoryBackend::new("https://mem.example/b");
        let lost = |_: u64, _: &UploadState| -> Saved { Some(oneshot::channel().1) };
        let err = upload(&other, params(&path, None), &TransferCtx::standalone(4), &lost).await.unwrap_err();
        assert!(err.to_string().contains("err.storage"), "{err}");
        assert_eq!(*other.put_calls.lock().unwrap(), 0, "nothing goes that the state does not name");
    }
}

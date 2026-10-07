// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Download: fetch the chunks, several at once, each from the first server
//! that has it, and check the hash of every ciphertext; then decrypt and
//! join them in order (assembling), check the hash of the whole file
//! (verifying) and move it into the cache.
//!
//! The cache is keyed by the plaintext SHA-256 (`<cache>/<sha256>/<name>`),
//! so the same file received twice is stored once and names never collide.
//! Verified chunks are kept as `<cache>/tmp/<sha256>-<ciphertext id>/<index>`
//! until the file is complete: an interrupted download continues where it
//! stopped, whichever chunks it had. They are verified once per run before
//! anything is fetched (checking); an automatic retry within the run
//! fetches only what is still missing and reads nothing twice.
//!
//! The same picture sent twice is encrypted twice with different keys: the
//! two downloads have the same plaintext hash and must not share chunks,
//! so the folder is named by the ciphertext too, and one download at a
//! time works in a folder.

use crate::backend::BackendError;
use crate::control::{pauses, TransferCtx, CANCEL};
use crate::crypto::{sha256_hex, TAG_LEN};
use crate::descriptor::{safe_name, ChunkRef, MediaDescriptor};
use crate::progress::TransferStage;
use async_trait::async_trait;
use futures_util::StreamExt as _;
use messenger_core::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

/// Reads a blob by url. The real one is HTTP; tests read from memory.
#[async_trait]
pub trait BlobFetcher: Send + Sync {
    /// `Ok(None)` = this server does not have it (try the next one).
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>>;

    /// `fetch`, adding the bytes received so far to `got`, which starts
    /// from 0 with every call. One that cannot tell adds them at the end.
    async fn fetch_counted(&self, url: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        got.store(0, Ordering::Relaxed);
        let out = self.fetch(url, max_bytes).await?;
        if let Some(b) = &out {
            got.fetch_add(b.len() as u64, Ordering::Relaxed);
        }
        Ok(out)
    }
}

pub struct HttpFetcher {
    http: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Result<Self> {
        Ok(Self { http: crate::backend::http_client()? })
    }
}

/// A failed request as a stable code with the details after it.
fn network(e: reqwest::Error) -> MessengerError {
    let code = if e.is_timeout() { "err.timeout" } else { "err.network" };
    MessengerError::Transport(format!("{code}: {e}"))
}

#[async_trait]
impl BlobFetcher for HttpFetcher {
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        self.fetch_counted(url, max_bytes, &AtomicU64::new(0)).await
    }

    async fn fetch_counted(&self, url: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        // The whole read is one attempt: a chunk half read the old way is read
        // again the new way.
        messenger_http::following_route(&crate::backend::host_of(url), || self.fetch_once(url, max_bytes, got))
            .await
            .map_err(|e| MessengerError::Transport(format!("err.network: {e}")))?
    }
}

impl HttpFetcher {
    async fn fetch_once(&self, url: &str, max_bytes: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
        got.store(0, Ordering::Relaxed);
        let mut resp = self.http.get(url).send().await.map_err(network)?;
        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let code = BackendError { status: Some(status), message: String::new() }.code();
            return Err(MessengerError::Transport(format!("{code}: http {status}")));
        }
        if resp.content_length().is_some_and(|n| n > max_bytes) {
            return Err(MessengerError::Invalid("err.blob_too_large".into()));
        }
        // Never trust the length header alone: stop reading past the limit.
        // Room for what it says is made at once, so the chunk is not copied
        // as it grows.
        let mut out = Vec::with_capacity(resp.content_length().unwrap_or(0).min(max_bytes) as usize);
        while let Some(part) = resp.chunk().await.map_err(network)? {
            if (out.len() + part.len()) as u64 > max_bytes {
                return Err(MessengerError::Invalid("err.blob_too_large".into()));
            }
            out.extend_from_slice(&part);
            got.fetch_add(part.len() as u64, Ordering::Relaxed);
        }
        Ok(Some(out))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DownloadOutcome {
    Done(PathBuf),
    Paused,
    Cancelled,
}

/// What the transfer row keeps of a download: how far it got.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadState {
    pub chunks_done: u32,
    pub chunks_total: u32,
    pub chunk_size: u64,
    /// The folder under `<cache>/tmp` its chunks wait in, so a cancel
    /// can remove them without the message at hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// The user started the last run (not the automatic path): one the
    /// app's closing interrupted starts again by itself.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

pub fn cached_path(cache_dir: &Path, d: &MediaDescriptor) -> PathBuf {
    cache_dir.join(&d.sha256).join(safe_name(&d.name))
}

/// Where the chunks of this one encrypted copy wait. Named by the plaintext
/// (to find it by eye) and by the chunk table, which only this copy has.
pub(crate) fn tmp_dir(cache_dir: &Path, d: &MediaDescriptor) -> PathBuf {
    cache_dir.join("tmp").join(tmp_name(d))
}

/// The name of that folder.
pub(crate) fn tmp_name(d: &MediaDescriptor) -> String {
    let mut h = Sha256::new();
    for c in &d.chunks {
        h.update(c.sha256.as_bytes());
    }
    let copy = hex::encode(h.finalize());
    format!("{}-{}", d.sha256, &copy[..16])
}

/// One download at a time per folder, in this process.
fn folder_lock(dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, w| w.strong_count() > 0);
    if let Some(lock) = map.get(dir).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    map.insert(dir.to_path_buf(), Arc::downgrade(&lock));
    lock
}

/// Folders of the old naming (`tmp/<sha256>`) were shared by every copy of
/// a file and may hold a mix of them: nothing in them can be trusted to
/// resume from. Downloads start again in folders of their own.
pub async fn drop_shared_folders(cache_dir: &Path) {
    let Ok(mut dir) = tokio::fs::read_dir(cache_dir.join("tmp")).await else { return };
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

/// The file when it is already in the cache and intact in size.
pub async fn cached(cache_dir: &Path, d: &MediaDescriptor) -> Option<PathBuf> {
    let p = cached_path(cache_dir, d);
    match tokio::fs::metadata(&p).await {
        Ok(m) if m.is_file() && m.len() == d.size => Some(p),
        _ => None,
    }
}

/// Blocking work off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| MessengerError::Other(e.to_string()))
}

/// Is chunk `c` on disk at `part`, whole and untouched?
async fn have(part: &Path, c: &ChunkRef) -> bool {
    let (part, c) = (part.to_path_buf(), c.clone());
    blocking(move || match std::fs::read(&part) {
        Ok(bytes) => bytes.len() as u64 == c.size && sha256_hex(&bytes) == c.sha256,
        Err(_) => false,
    })
    .await
    .unwrap_or(false)
}

async fn fetch_chunk(fetcher: &dyn BlobFetcher, servers: &[String], c: &ChunkRef, got: &AtomicU64) -> Result<Vec<u8>> {
    let mut last = MessengerError::Transport("err.not_found".into());
    for server in servers {
        let url = MediaDescriptor::chunk_url(server, &c.sha256);
        // Two tries per server: a hiccup should not send us to a mirror.
        for _ in 0..2 {
            match fetcher.fetch_counted(&url, c.size, got).await {
                Ok(Some(bytes)) => {
                    let want = c.clone();
                    let (ok, bytes) = blocking(move || (bytes.len() as u64 == want.size && sha256_hex(&bytes) == want.sha256, bytes)).await?;
                    if ok {
                        return Ok(bytes);
                    }
                    last = MessengerError::Crypto("err.chunk_hash_mismatch".into());
                    break;
                }
                Ok(None) => {
                    last = MessengerError::Transport("err.not_found".into());
                    break;
                }
                Err(e) => last = e,
            }
        }
    }
    got.store(0, Ordering::Relaxed);
    Err(last)
}

/// Remove the folder of a cancelled download. A write dropped with the
/// download may still be finishing; one more try after it.
async fn remove_tmp(tmp: &Path) {
    if tokio::fs::remove_dir_all(tmp).await.is_err_and(|e| e.kind() != std::io::ErrorKind::NotFound) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = tokio::fs::remove_dir_all(tmp).await;
    }
}

/// Remove what a download of `d` keeps on disk to resume from, unless
/// another download works in that folder now.
pub async fn discard(cache_dir: &Path, d: &MediaDescriptor) {
    discard_folder(cache_dir, &tmp_name(d)).await;
}

/// `discard` by the name of the folder (`DownloadState::folder`). A name
/// that is not one of ours is left alone.
pub async fn discard_folder(cache_dir: &Path, name: &str) {
    let ours = name.len() == 64 + 1 + 16
        && name.bytes().enumerate().all(|(i, b)| if i == 64 { b == b'-' } else { b.is_ascii_hexdigit() });
    if !ours {
        return;
    }
    let tmp = cache_dir.join("tmp").join(name);
    let lock = folder_lock(&tmp);
    if let Ok(_guard) = lock.try_lock() {
        remove_tmp(&tmp).await;
    };
}

/// Download into the cache. `extra_servers` are tried after the ones the
/// sender listed (our own mirrors). `on_chunk(done_bytes, chunks_done)`
/// after the chunks on disk are checked and after every chunk fetched. A
/// pause of `ctx.control` drops the requests in flight at once and keeps
/// the chunks on disk; a cancel removes them.
pub async fn download(
    fetcher: &dyn BlobFetcher,
    d: &MediaDescriptor,
    extra_servers: &[String],
    cache_dir: &Path,
    ctx: &TransferCtx,
    on_chunk: &(dyn Fn(u64, u32) + Send + Sync),
) -> Result<DownloadOutcome> {
    d.validate()?;
    let n = d.chunks.len() as u32;
    ctx.live.start(n, d.chunk_size);
    let all_found = || {
        for (i, c) in d.chunks.iter().enumerate() {
            ctx.live.found(i, plain_of(c));
        }
    };
    if let Some(p) = cached(cache_dir, d).await {
        all_found();
        return Ok(DownloadOutcome::Done(p));
    }
    let mut servers = d.servers.clone();
    for s in extra_servers {
        if !servers.contains(s) {
            servers.push(s.clone());
        }
    }
    let tmp = tmp_dir(cache_dir, d);
    let lock = folder_lock(&tmp);
    let stopped = |stop: u8| if pauses(stop) { DownloadOutcome::Paused } else { DownloadOutcome::Cancelled };
    let _guard = tokio::select! {
        biased;
        stop = ctx.control.stopped() => return Ok(stopped(stop)),
        guard = lock.lock() => guard,
    };
    // Whoever held the folder before may have finished the file.
    if let Some(p) = cached(cache_dir, d).await {
        all_found();
        return Ok(DownloadOutcome::Done(p));
    }

    let work = fetch_and_join(fetcher, d, &servers, cache_dir, &tmp, ctx, on_chunk);
    tokio::select! {
        biased;
        stop = ctx.control.stopped() => {
            if stop == CANCEL {
                remove_tmp(&tmp).await;
            }
            Ok(stopped(stop))
        }
        done = work => done,
    }
}

/// The plaintext size of a chunk: what it counts for in the progress.
fn plain_of(c: &ChunkRef) -> u64 {
    c.size.saturating_sub(TAG_LEN as u64)
}

/// Chunk files in `tmp` by index (finished ones only, not `.part`).
async fn on_disk(tmp: &Path) -> Vec<usize> {
    let tmp = tmp.to_path_buf();
    blocking(move || {
        let Ok(dir) = std::fs::read_dir(&tmp) else { return Vec::new() };
        dir.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<usize>().ok())).collect()
    })
    .await
    .unwrap_or_default()
}

/// Chunks on disk this run has not verified yet: the ones `download`
/// checks first, before it fetches.
async fn to_check(tmp: &Path, d: &MediaDescriptor, live: &crate::progress::Live) -> Vec<usize> {
    on_disk(tmp).await.into_iter().filter(|i| *i < d.chunks.len() && !live.is_done(*i)).collect()
}

/// Does `download` check chunks on disk first (the checking stage), given
/// what this run counted done already?
pub async fn will_check(cache_dir: &Path, d: &MediaDescriptor, live: &crate::progress::Live) -> bool {
    !to_check(&tmp_dir(cache_dir, d), d, live).await.is_empty()
}

async fn fetch_and_join(
    fetcher: &dyn BlobFetcher,
    d: &MediaDescriptor,
    servers: &[String],
    cache_dir: &Path,
    tmp: &Path,
    ctx: &TransferCtx,
    on_chunk: &(dyn Fn(u64, u32) + Send + Sync),
) -> Result<DownloadOutcome> {
    let key = d.file_key()?;
    tokio::fs::create_dir_all(tmp).await?;

    // Checking: chunks an earlier run left on disk are verified, several
    // at once, before anything is fetched. Those this run verified or
    // fetched already are not read again.
    let kept = to_check(tmp, d, &ctx.live).await;
    if !kept.is_empty() {
        ctx.live.set_stage(TransferStage::Checking);
        let mut checks = futures_util::stream::iter(kept)
            .map(|i| async move { (i, have(&tmp.join(i.to_string()), &d.chunks[i]).await) })
            .buffer_unordered(ctx.workers.max(1));
        while let Some((i, ok)) = checks.next().await {
            if ok {
                ctx.live.found(i, plain_of(&d.chunks[i]));
            } else {
                // Missing from now on: a retry within the run fetches it
                // and never checks it again.
                let _ = tokio::fs::remove_file(tmp.join(i.to_string())).await;
            }
        }
    }
    // How far it got goes to the row at once, so a view after a restart
    // can say "chunk 150 of 200" even when nothing more arrives.
    on_chunk(ctx.live.confirmed(), ctx.live.marked());

    // Every missing chunk fetched and verified on disk (still encrypted),
    // several at once.
    ctx.live.set_stage(TransferStage::Downloading);
    let missing: Vec<usize> = (0..d.chunks.len()).filter(|i| !ctx.live.is_done(*i)).collect();
    let next = AtomicUsize::new(0);
    let failed = watch::channel(false).0;
    let workers = (0..ctx.workers.max(1)).map(|_| fetch_worker(fetcher, d, servers, tmp, ctx, &missing, &next, &failed, on_chunk));
    // The first chunk that fails for good stops the others; a chunk on its
    // way to the disk is written whole first.
    for result in futures_util::future::join_all(workers).await {
        result?;
    }

    // Decrypt in order into one file, hashing the plaintext.
    ctx.live.set_assembled(0);
    ctx.live.set_stage(TransferStage::Assembling);
    let assembled = tmp.join("assembled");
    let mut out = tokio::fs::File::create(&assembled).await?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    for i in 0..d.chunks.len() {
        let (part, key, mut h) = (tmp.join(i.to_string()), key.clone(), std::mem::take(&mut hasher));
        let (h, plain) = blocking(move || -> Result<_> {
            let cipher = std::fs::read(&part)?;
            let plain = key
                .decrypt_chunk(i as u32, &cipher)
                .map_err(|_| MessengerError::Crypto("err.chunk_auth_failed".into()))?;
            h.update(&plain);
            Ok((h, plain))
        })
        .await??;
        hasher = h;
        written += plain.len() as u64;
        out.write_all(&plain).await?;
        ctx.live.set_assembled(i as u32 + 1);
    }
    out.flush().await?;
    drop(out);

    ctx.live.set_stage(TransferStage::Verifying);
    if written != d.size || hex::encode(hasher.finalize()) != d.sha256 {
        let _ = tokio::fs::remove_dir_all(tmp).await;
        return Err(MessengerError::Crypto("err.file_hash_mismatch".into()));
    }
    let target = cached_path(cache_dir, d);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&assembled, &target).await?;
    let _ = tokio::fs::remove_dir_all(tmp).await;
    Ok(DownloadOutcome::Done(target))
}

/// Take the next missing chunk not taken yet until there are none, or
/// until another worker failed (`failed`), and fetch it. Whatever fails
/// here (the fetch, or the disk) stops the other workers too.
#[allow(clippy::too_many_arguments)]
async fn fetch_worker(
    fetcher: &dyn BlobFetcher,
    d: &MediaDescriptor,
    servers: &[String],
    tmp: &Path,
    ctx: &TransferCtx,
    missing: &[usize],
    next: &AtomicUsize,
    failed: &watch::Sender<bool>,
    on_chunk: &(dyn Fn(u64, u32) + Send + Sync),
) -> Result<()> {
    let mut stop = failed.subscribe();
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let Some(&i) = missing.get(next.fetch_add(1, Ordering::SeqCst)) else { return Ok(()) };
        let c = &d.chunks[i];
        let flight = ctx.live.begin(plain_of(c));
        let fetched = tokio::select! {
            biased;
            _ = stop.wait_for(|f| *f) => return Ok(()),
            fetched = async {
                let _slot = ctx.slot().await?;
                fetch_chunk(fetcher, servers, c, &flight.sent).await
            } => fetched,
        };
        let kept = async {
            let bytes = fetched?;
            let staging = tmp.join(format!("{i}.part"));
            // The chunk itself goes to the writing thread, not a copy of it.
            let to = staging.clone();
            blocking(move || std::fs::write(&to, bytes)).await??;
            tokio::fs::rename(&staging, tmp.join(i.to_string())).await?;
            Ok::<_, MessengerError>(())
        };
        if let Err(e) = kept.await {
            failed.send_replace(true);
            return Err(e);
        }
        flight.confirm(i);
        on_chunk(ctx.live.confirmed(), ctx.live.marked());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The bytes of a chunk count as they arrive, from 0 at every fetch,
    /// into room made for them at once.
    #[tokio::test]
    async fn an_http_fetch_counts_what_arrives_from_zero_each_time() {
        let server = MockServer::start().await;
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        Mock::given(method("GET")).and(path("/blob")).respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone())).mount(&server).await;
        Mock::given(method("GET")).and(path("/gone")).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let fetcher = HttpFetcher::new().unwrap();
        let url = format!("{}/blob", server.uri());

        let got = AtomicU64::new(12_345);
        let bytes = fetcher.fetch_counted(&url, 1 << 20, &got).await.unwrap().unwrap();
        assert_eq!(bytes, body);
        assert_eq!(got.load(Ordering::SeqCst), body.len() as u64, "from 0, not from what was there");
        assert_eq!(bytes.capacity(), body.len(), "read into room made for it, never grown");
        let again = fetcher.fetch_counted(&url, 1 << 20, &got).await.unwrap().unwrap();
        assert_eq!(again, body);
        assert_eq!(got.load(Ordering::SeqCst), body.len() as u64, "a second fetch counts from 0 again");

        let e = fetcher.fetch_counted(&url, 1000, &got).await.unwrap_err();
        assert!(e.to_string().contains("err.blob_too_large"), "{e}");
        assert_eq!(fetcher.fetch_counted(&format!("{}/gone", server.uri()), 1000, &got).await.unwrap(), None);
    }
}

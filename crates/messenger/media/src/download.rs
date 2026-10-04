// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Download: fetch every chunk from the first server that has it, check
//! the hash of the ciphertext, decrypt, append; check the hash of the
//! whole file; move it into the cache.
//!
//! The cache is keyed by the plaintext SHA-256 (`<cache>/<sha256>/<name>`),
//! so the same file received twice is stored once and names never collide.
//! Verified chunks are kept as `<cache>/tmp/<sha256>-<ciphertext id>/<index>`
//! until the
//! file is complete: an interrupted download continues where it stopped.
//! The same picture sent twice is encrypted twice with different keys: the
//! two downloads have the same plaintext hash and must not share chunks,
//! so the folder is named by the ciphertext too, and one download at a time
//! works in a folder.

use crate::crypto::sha256_hex;
use crate::descriptor::{safe_name, MediaDescriptor};
use crate::upload::{CANCEL, PAUSE};
use async_trait::async_trait;
use messenger_core::{MessengerError, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::io::AsyncWriteExt;

/// Reads a blob by url. The real one is HTTP; tests read from memory.
#[async_trait]
pub trait BlobFetcher: Send + Sync {
    /// `Ok(None)` = this server does not have it (try the next one).
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>>;
}

pub struct HttpFetcher {
    http: reqwest::Client,
}

impl HttpFetcher {
    pub fn new() -> Result<Self> {
        Ok(Self { http: crate::backend::http_client()? })
    }
}

#[async_trait]
impl BlobFetcher for HttpFetcher {
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        // The whole read is one attempt: a chunk half read the old way is read
        // again the new way.
        messenger_http::following_route(&crate::backend::host_of(url), || self.fetch_once(url, max_bytes))
            .await
            .map_err(|e| MessengerError::Transport(e.to_string()))?
    }
}

impl HttpFetcher {
    async fn fetch_once(&self, url: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let mut resp = self.http.get(url).send().await.map_err(|e| MessengerError::Transport(e.to_string()))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(MessengerError::Transport(format!("http {}", resp.status().as_u16())));
        }
        if resp.content_length().is_some_and(|n| n > max_bytes) {
            return Err(MessengerError::Invalid("blob is larger than announced".into()));
        }
        // Never trust the length header alone: stop reading past the limit.
        let mut out = Vec::new();
        while let Some(part) = resp.chunk().await.map_err(|e| MessengerError::Transport(e.to_string()))? {
            if (out.len() + part.len()) as u64 > max_bytes {
                return Err(MessengerError::Invalid("blob is larger than announced".into()));
            }
            out.extend_from_slice(&part);
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

pub fn cached_path(cache_dir: &Path, d: &MediaDescriptor) -> PathBuf {
    cache_dir.join(&d.sha256).join(safe_name(&d.name))
}

/// Where the chunks of this one encrypted copy wait. Named by the plaintext
/// (to find it by eye) and by the chunk table, which only this copy has.
pub(crate) fn tmp_dir(cache_dir: &Path, d: &MediaDescriptor) -> PathBuf {
    let mut h = Sha256::new();
    for c in &d.chunks {
        h.update(c.sha256.as_bytes());
    }
    let copy = hex::encode(h.finalize());
    cache_dir.join("tmp").join(format!("{}-{}", d.sha256, &copy[..16]))
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

async fn fetch_chunk(fetcher: &dyn BlobFetcher, servers: &[String], sha: &str, size: u64) -> Result<Vec<u8>> {
    let mut last = MessengerError::Transport("err.not_found".into());
    for server in servers {
        let url = MediaDescriptor::chunk_url(server, sha);
        // Two tries per server: a hiccup should not send us to a mirror.
        for _ in 0..2 {
            match fetcher.fetch(&url, size).await {
                Ok(Some(bytes)) => {
                    if bytes.len() as u64 == size && sha256_hex(&bytes) == sha {
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
    Err(last)
}

/// Download into the cache. `extra_servers` are tried after the ones the
/// sender listed (our own mirrors). `on_progress(done_bytes, total)`.
pub async fn download<F>(
    fetcher: &dyn BlobFetcher,
    d: &MediaDescriptor,
    extra_servers: &[String],
    cache_dir: &Path,
    control: &AtomicU8,
    mut on_progress: F,
) -> Result<DownloadOutcome>
where
    F: FnMut(u64, u64) + Send,
{
    d.validate()?;
    if let Some(p) = cached(cache_dir, d).await {
        on_progress(d.size, d.size);
        return Ok(DownloadOutcome::Done(p));
    }
    let key = d.file_key()?;
    let mut servers = d.servers.clone();
    for s in extra_servers {
        if !servers.contains(s) {
            servers.push(s.clone());
        }
    }
    let tmp = tmp_dir(cache_dir, d);
    let lock = folder_lock(&tmp);
    let _guard = lock.lock().await;
    // Whoever held the folder before may have finished the file.
    if let Some(p) = cached(cache_dir, d).await {
        on_progress(d.size, d.size);
        return Ok(DownloadOutcome::Done(p));
    }
    tokio::fs::create_dir_all(&tmp).await?;

    // Phase 1: every chunk verified on disk (still encrypted).
    let mut done: u64 = 0;
    for (i, c) in d.chunks.iter().enumerate() {
        match control.load(Ordering::SeqCst) {
            PAUSE => return Ok(DownloadOutcome::Paused),
            CANCEL => {
                let _ = tokio::fs::remove_dir_all(&tmp).await;
                return Ok(DownloadOutcome::Cancelled);
            }
            _ => {}
        }
        let part = tmp.join(i.to_string());
        let have = match tokio::fs::read(&part).await {
            Ok(bytes) => bytes.len() as u64 == c.size && sha256_hex(&bytes) == c.sha256,
            Err(_) => false,
        };
        if !have {
            let bytes = fetch_chunk(fetcher, &servers, &c.sha256, c.size).await?;
            let staging = tmp.join(format!("{i}.part"));
            tokio::fs::write(&staging, &bytes).await?;
            tokio::fs::rename(&staging, &part).await?;
        }
        done += c.size;
        on_progress(done.min(d.size), d.size);
    }

    // Phase 2: decrypt in order into one file, hashing the plaintext.
    let assembled = tmp.join("assembled");
    let mut out = tokio::fs::File::create(&assembled).await?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    for (i, _) in d.chunks.iter().enumerate() {
        let cipher = tokio::fs::read(tmp.join(i.to_string())).await?;
        let plain = key.decrypt_chunk(i as u32, &cipher)?;
        hasher.update(&plain);
        written += plain.len() as u64;
        out.write_all(&plain).await?;
    }
    out.flush().await?;
    drop(out);
    if written != d.size || hex::encode(hasher.finalize()) != d.sha256 {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err(MessengerError::Crypto("err.file_hash_mismatch".into()));
    }

    let target = cached_path(cache_dir, d);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::rename(&assembled, &target).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(DownloadOutcome::Done(target))
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Servers, transfers and scheduling on top of `upload`/`download`.
//!
//! Scheduling: two lanes. Small transfers have their own slots, so a
//! gigabyte in flight never makes a photo wait. Every transfer can be
//! paused, resumed and cancelled; a restart turns what was running into
//! paused.

use crate::backend::{BlobBackend, BlossomBackend, S3Backend, S3Config};
use crate::descriptor::{mime_for, safe_name, MediaDescriptor};
use crate::download::{self, BlobFetcher, DownloadOutcome, HttpFetcher};
use crate::upload::{self, UploadOutcome, UploadParams, UploadState, CANCEL, PAUSE, RUN};
use messenger_core::{MessengerError, Result, SecretStore};
use messenger_store::media::{self as repo, ServerRow, TransferRow};
use messenger_store::Store;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Transfers at or below this size use the fast lane.
pub const SMALL_BYTES: u64 = 8 * 1024 * 1024;
const SMALL_SLOTS: usize = 3;
const LARGE_SLOTS: usize = 2;
/// A download that failed this many times is not started automatically.
pub const MAX_AUTO_ATTEMPTS: i64 = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub transfer_id: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    /// `up` | `down`
    pub direction: String,
    /// queued | running | paused | done | failed | cancelled
    pub status: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub failure_reason: Option<String>,
    pub local_path: Option<String>,
}

/// Receives progress; the runtime forwards it to the host.
pub trait ProgressSink: Send + Sync {
    fn progress(&self, p: Progress);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaServerView {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub bucket: Option<String>,
    pub region: Option<String>,
    /// The access key id is shown; the secret never is.
    pub access_key: Option<String>,
    pub has_secret: bool,
    pub priority: i64,
    pub enabled: bool,
    pub source: String,
    pub public_base: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MediaServerInput {
    /// Stable id; generated from the url when empty.
    #[serde(default)]
    pub id: Option<String>,
    /// `s3` | `blossom`
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub bucket: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub access_key: Option<String>,
    #[serde(default)]
    pub secret_key: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferView {
    pub id: String,
    pub direction: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub file_name: String,
    pub mime: String,
    pub size: u64,
    pub status: String,
    pub done_bytes: u64,
    pub attempts: i64,
    pub failure_reason: Option<String>,
    pub local_path: Option<String>,
}

impl From<TransferRow> for TransferView {
    fn from(r: TransferRow) -> Self {
        Self {
            id: r.id,
            direction: r.direction,
            message_id: r.message_id,
            chat_id: r.chat_id,
            file_name: r.file_name,
            mime: r.mime,
            size: r.size.max(0) as u64,
            status: r.status,
            done_bytes: r.done_bytes.max(0) as u64,
            attempts: r.attempts,
            failure_reason: r.failure_reason,
            local_path: r.local_path,
        }
    }
}

fn secret_ref(id: &str) -> String {
    format!("media.{id}.secret")
}

fn new_id(prefix: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}-{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

fn short_reason(e: &MessengerError) -> String {
    let s = e.to_string();
    // Stable codes (`err.*`) are kept as they are for the UI to translate.
    match s.find("err.") {
        Some(i) => s[i..].split([':', ' ']).next().unwrap_or("err.unknown").to_string(),
        None => s.chars().take(160).collect(),
    }
}

#[derive(Clone)]
pub struct MediaService {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    cache_dir: PathBuf,
    fetcher: Arc<dyn BlobFetcher>,
    controls: Arc<Mutex<HashMap<String, Arc<AtomicU8>>>>,
    small: Arc<Semaphore>,
    large: Arc<Semaphore>,
    /// Tests and offline development: use this instead of configured servers.
    fixed_backend: Option<Arc<dyn BlobBackend>>,
}

impl MediaService {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>, data_dir: &Path) -> Result<Self> {
        Ok(Self {
            store,
            secrets,
            cache_dir: data_dir.join("media"),
            fetcher: Arc::new(HttpFetcher::new()?),
            controls: Arc::default(),
            small: Arc::new(Semaphore::new(SMALL_SLOTS)),
            large: Arc::new(Semaphore::new(LARGE_SLOTS)),
            fixed_backend: None,
        })
    }

    /// Replace network access (tests).
    pub fn with_backend(mut self, backend: Arc<dyn BlobBackend>, fetcher: Arc<dyn BlobFetcher>) -> Self {
        self.fixed_backend = Some(backend);
        self.fetcher = fetcher;
        self
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Call once at start-up.
    pub async fn recover(&self) -> Result<u64> {
        crate::download::drop_shared_folders(&self.cache_dir).await;
        repo::pause_interrupted(&self.store).await
    }

    // ─── Servers ────────────────────────────────────────────────────────────

    async fn view(&self, r: ServerRow) -> MediaServerView {
        let has_secret = self.secrets.get(&secret_ref(&r.id)).await.ok().flatten().is_some();
        let public_base = match r.kind.as_str() {
            repo::KIND_S3 => format!("{}/{}", r.url.trim_end_matches('/'), r.bucket.clone().unwrap_or_default()),
            _ => r.url.trim_end_matches('/').to_string(),
        };
        MediaServerView {
            id: r.id,
            kind: r.kind,
            url: r.url,
            bucket: r.bucket,
            region: r.region,
            access_key: r.access_key,
            has_secret,
            priority: r.priority,
            enabled: r.enabled,
            source: r.source,
            public_base,
        }
    }

    pub async fn servers(&self) -> Result<Vec<MediaServerView>> {
        let mut out = Vec::new();
        for r in repo::servers(&self.store).await? {
            out.push(self.view(r).await);
        }
        Ok(out)
    }

    /// Add or update a server. For S3 the secret goes to the SecretStore;
    /// an update without a secret keeps the stored one.
    pub async fn put_server(&self, input: MediaServerInput) -> Result<MediaServerView> {
        let url = input.url.trim().trim_end_matches('/').to_string();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(MessengerError::Invalid("server url must start with https:// or http://".into()));
        }
        let kind = input.kind.trim().to_ascii_lowercase();
        let nonempty = |o: Option<String>| o.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let (bucket, region, access_key) = match kind.as_str() {
            repo::KIND_S3 => {
                let bucket = nonempty(input.bucket).ok_or_else(|| MessengerError::Invalid("s3 needs a bucket".into()))?;
                let region = nonempty(input.region).unwrap_or_else(|| "us-east-1".into());
                (Some(bucket), Some(region), nonempty(input.access_key))
            }
            repo::KIND_BLOSSOM => (None, None, None),
            _ => return Err(MessengerError::Invalid("server kind must be s3 or blossom".into())),
        };
        let id = nonempty(input.id).unwrap_or_else(|| {
            let host = url.split("://").nth(1).unwrap_or("server").replace([':', '/', '.'], "-");
            match &bucket {
                Some(b) => format!("{kind}-{host}-{b}"),
                None => format!("{kind}-{host}"),
            }
        });
        if kind == repo::KIND_S3 {
            // Validates endpoint and bucket name.
            S3Backend::new(S3Config {
                endpoint: url.clone(),
                bucket: bucket.clone().unwrap_or_default(),
                region: region.clone().unwrap_or_default(),
                access_key: String::new(),
                secret_key: String::new(),
            })?;
        }
        repo::upsert_server(
            &self.store,
            &ServerRow {
                id: id.clone(),
                kind: kind.clone(),
                url,
                bucket,
                region,
                access_key,
                priority: input.priority.unwrap_or(if kind == repo::KIND_S3 { 10 } else { 50 }),
                enabled: true,
                source: nonempty(input.source).unwrap_or_else(|| "user".into()),
                created_at: 0,
                updated_at: 0,
            },
        )
        .await?;
        if let Some(secret) = nonempty(input.secret_key) {
            self.secrets.put(&secret_ref(&id), secret.as_bytes()).await?;
        }
        let row = repo::server(&self.store, &id).await?.ok_or_else(|| MessengerError::Storage("server vanished".into()))?;
        Ok(self.view(row).await)
    }

    pub async fn remove_server(&self, id: &str) -> Result<()> {
        repo::delete_server(&self.store, id).await?;
        let _ = self.secrets.delete(&secret_ref(id)).await;
        Ok(())
    }

    pub async fn set_server_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        repo::set_server_enabled(&self.store, id, enabled).await
    }

    async fn build(&self, r: &ServerRow, keys: &Keys) -> Result<Arc<dyn BlobBackend>> {
        match r.kind.as_str() {
            repo::KIND_S3 => {
                let secret = self
                    .secrets
                    .get(&secret_ref(&r.id))
                    .await?
                    .ok_or_else(|| MessengerError::Invalid("err.media_no_credentials".into()))?;
                Ok(Arc::new(S3Backend::new(S3Config {
                    endpoint: r.url.clone(),
                    bucket: r.bucket.clone().unwrap_or_default(),
                    region: r.region.clone().unwrap_or_else(|| "us-east-1".into()),
                    access_key: r.access_key.clone().ok_or_else(|| MessengerError::Invalid("err.media_no_credentials".into()))?,
                    secret_key: String::from_utf8_lossy(&secret).into_owned(),
                })?))
            }
            _ => Ok(Arc::new(BlossomBackend::new(&r.url, keys.clone())?)),
        }
    }

    /// The server uploads go to: the first enabled one that can be used
    /// (S3 sorts first by default priority).
    pub async fn upload_backend(&self, keys: &Keys) -> Result<Arc<dyn BlobBackend>> {
        if let Some(b) = &self.fixed_backend {
            return Ok(b.clone());
        }
        let mut last = MessengerError::Invalid("err.media_no_server".into());
        for r in repo::servers(&self.store).await?.into_iter().filter(|r| r.enabled) {
            match self.build(&r, keys).await {
                Ok(b) => return Ok(b),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Check credentials and make the store usable (creates the bucket and
    /// its read policy on S3).
    pub async fn check_server(&self, id: &str, keys: &Keys) -> Result<()> {
        let row = repo::server(&self.store, id).await?.ok_or_else(|| MessengerError::Invalid("unknown server".into()))?;
        let backend = self.build(&row, keys).await?;
        backend.prepare().await?;
        // A throwaway blob proves that writing and public reading work.
        let probe = format!("veydan media probe {}", new_id("p")).into_bytes();
        let sha = crate::crypto::sha256_hex(&probe);
        backend.put(&sha, probe.clone()).await?;
        let url = MediaDescriptor::chunk_url(&backend.public_base(), &sha);
        match self.fetcher.fetch(&url, probe.len() as u64).await? {
            Some(b) if b == probe => Ok(()),
            _ => Err(MessengerError::Transport("err.media_not_readable".into())),
        }
    }

    /// Public bases of our enabled servers: mirrors to try on download.
    async fn own_bases(&self) -> Vec<String> {
        match self.servers().await {
            Ok(list) => list.into_iter().filter(|s| s.enabled).map(|s| s.public_base).collect(),
            Err(_) => vec![],
        }
    }

    // ─── Transfers ──────────────────────────────────────────────────────────

    fn control(&self, id: &str) -> Arc<AtomicU8> {
        self.controls.lock().unwrap().entry(id.to_string()).or_insert_with(|| Arc::new(AtomicU8::new(RUN))).clone()
    }

    fn lane(&self, size: u64) -> Arc<Semaphore> {
        if size <= SMALL_BYTES {
            self.small.clone()
        } else {
            self.large.clone()
        }
    }

    pub async fn transfer(&self, id: &str) -> Result<Option<TransferView>> {
        Ok(repo::transfer(&self.store, id).await?.map(Into::into))
    }

    pub async fn transfer_for_message(&self, message_id: &str, direction: &str) -> Result<Option<TransferView>> {
        Ok(repo::transfer_for_message(&self.store, message_id, direction).await?.map(Into::into))
    }

    pub async fn active_transfers(&self) -> Result<Vec<TransferView>> {
        Ok(repo::transfers_with_status(&self.store, &[repo::ST_RUNNING, repo::ST_QUEUED, repo::ST_PAUSED, repo::ST_FAILED])
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn emit(&self, sink: &dyn ProgressSink, id: &str) {
        if let Ok(Some(r)) = repo::transfer(&self.store, id).await {
            sink.progress(Progress {
                transfer_id: r.id,
                message_id: r.message_id,
                chat_id: r.chat_id,
                direction: r.direction,
                status: r.status,
                done_bytes: r.done_bytes.max(0) as u64,
                total_bytes: r.size.max(0) as u64,
                failure_reason: r.failure_reason,
                local_path: r.local_path,
            });
        }
    }

    /// Register an upload. `message_id` is the placeholder row of the chat.
    pub async fn queue_upload(&self, path: &Path, chat_id: &str, message_id: &str) -> Result<TransferView> {
        let meta = tokio::fs::metadata(path).await.map_err(|_| MessengerError::Io("err.file_not_found".into()))?;
        if !meta.is_file() {
            return Err(MessengerError::Invalid("not a regular file".into()));
        }
        if meta.len() == 0 {
            return Err(MessengerError::Invalid("the file is empty".into()));
        }
        if meta.len() > crate::descriptor::MAX_FILE_BYTES {
            return Err(MessengerError::Invalid("err.file_too_large".into()));
        }
        let name = safe_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let row = TransferRow {
            id: new_id("up"),
            direction: repo::DIR_UP.into(),
            message_id: Some(message_id.into()),
            chat_id: Some(chat_id.into()),
            local_path: Some(path.to_string_lossy().into_owned()),
            mime: mime_for(&name).into(),
            file_name: name,
            size: meta.len() as i64,
            sha256: None,
            status: repo::ST_QUEUED.into(),
            done_bytes: 0,
            attempts: 0,
            failure_reason: None,
            state_json: "{}".into(),
            created_at: 0,
            updated_at: 0,
        };
        repo::insert_transfer(&self.store, &row).await?;
        Ok(row.into())
    }

    /// Run (or resume) an upload to completion, pause or failure.
    /// `Ok(Some(descriptor))` when the file is fully stored.
    pub async fn run_upload(
        &self,
        id: &str,
        keys: &Keys,
        caption: Option<String>,
        sink: &dyn ProgressSink,
    ) -> Result<Option<MediaDescriptor>> {
        let row = repo::transfer(&self.store, id).await?.ok_or_else(|| MessengerError::Invalid("unknown transfer".into()))?;
        if row.direction != repo::DIR_UP {
            return Err(MessengerError::Invalid("not an upload".into()));
        }
        let path = PathBuf::from(row.local_path.clone().unwrap_or_default());
        let control = self.control(id);
        control.store(RUN, Ordering::SeqCst);
        let _permit = self.lane(row.size.max(0) as u64).acquire_owned().await.map_err(|e| MessengerError::Other(e.to_string()))?;
        if control.load(Ordering::SeqCst) == CANCEL {
            repo::set_status(&self.store, id, repo::ST_CANCELLED, None).await?;
            self.emit(sink, id).await;
            return Ok(None);
        }
        repo::set_status(&self.store, id, repo::ST_RUNNING, None).await?;
        repo::set_attempts(&self.store, id, row.attempts.max(0) + 1).await?;
        self.emit(sink, id).await;

        let resume: Option<UploadState> = serde_json::from_str(&row.state_json).ok().filter(|s: &UploadState| !s.key.is_empty());
        let backend = match self.upload_backend(keys).await {
            Ok(b) => b,
            Err(e) => {
                repo::set_status(&self.store, id, repo::ST_FAILED, Some(&short_reason(&e))).await?;
                self.emit(sink, id).await;
                return Err(e);
            }
        };

        // Progress is persisted from a channel so the transfer loop never
        // waits on the database.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, String)>();
        let store = self.store.clone();
        let tid = id.to_string();
        let writer = tokio::spawn(async move {
            while let Some((done, state)) = rx.recv().await {
                let _ = repo::set_progress(&store, &tid, done as i64, Some(&state)).await;
            }
        });

        let result = upload::upload(
            backend.as_ref(),
            UploadParams { path: &path, caption, batch: None, resume },
            &control,
            |done, _total, state| {
                let _ = tx.send((done, serde_json::to_string(state).unwrap_or_else(|_| "{}".into())));
            },
        )
        .await;
        drop(tx);
        let _ = writer.await;
        self.controls.lock().unwrap().remove(id);

        match result {
            Ok(UploadOutcome::Done(d)) => {
                repo::set_result(&self.store, id, None, Some(&d.sha256), None).await?;
                repo::set_progress(&self.store, id, d.size as i64, None).await?;
                repo::set_status(&self.store, id, repo::ST_DONE, None).await?;
                self.emit(sink, id).await;
                Ok(Some(*d))
            }
            Ok(UploadOutcome::Paused) => {
                repo::set_status(&self.store, id, repo::ST_PAUSED, None).await?;
                self.emit(sink, id).await;
                Ok(None)
            }
            Ok(UploadOutcome::Cancelled) => {
                repo::set_status(&self.store, id, repo::ST_CANCELLED, None).await?;
                self.emit(sink, id).await;
                Ok(None)
            }
            Err(e) => {
                repo::set_status(&self.store, id, repo::ST_FAILED, Some(&short_reason(&e))).await?;
                self.emit(sink, id).await;
                Err(e)
            }
        }
    }

    /// Path of a received file when it is already in the cache.
    pub async fn cached(&self, d: &MediaDescriptor) -> Option<PathBuf> {
        download::cached(&self.cache_dir, d).await
    }

    /// Whether a download may start without the user asking for it.
    pub async fn may_auto_download(&self, message_id: &str) -> Result<bool> {
        Ok(match repo::transfer_for_message(&self.store, message_id, repo::DIR_DOWN).await? {
            Some(t) => t.attempts >= 0 && t.attempts < MAX_AUTO_ATTEMPTS && t.status != repo::ST_RUNNING && t.status != repo::ST_QUEUED,
            None => true,
        })
    }

    /// Download the file of a message into the cache (or return it from
    /// there). `manual` resets the automatic-attempt bookkeeping.
    pub async fn run_download(
        &self,
        message_id: &str,
        chat_id: &str,
        d: &MediaDescriptor,
        manual: bool,
        sink: &dyn ProgressSink,
    ) -> Result<Option<PathBuf>> {
        d.validate()?;
        let existing = repo::transfer_for_message(&self.store, message_id, repo::DIR_DOWN).await?;
        let id = match existing {
            Some(t) if t.status == repo::ST_RUNNING || t.status == repo::ST_QUEUED => {
                return Err(MessengerError::Invalid("err.transfer_in_progress".into()));
            }
            Some(t) => t.id,
            None => {
                let row = TransferRow {
                    id: new_id("down"),
                    direction: repo::DIR_DOWN.into(),
                    message_id: Some(message_id.into()),
                    chat_id: Some(chat_id.into()),
                    local_path: None,
                    file_name: safe_name(&d.name),
                    mime: d.mime.clone(),
                    size: d.size as i64,
                    sha256: Some(d.sha256.clone()),
                    status: repo::ST_QUEUED.into(),
                    done_bytes: 0,
                    attempts: 0,
                    failure_reason: None,
                    state_json: "{}".into(),
                    created_at: 0,
                    updated_at: 0,
                };
                repo::insert_transfer(&self.store, &row).await?;
                row.id
            }
        };
        let before = repo::transfer(&self.store, &id).await?.map(|t| t.attempts).unwrap_or(0);
        if manual {
            repo::set_attempts(&self.store, &id, 0).await?;
        }
        let control = self.control(&id);
        control.store(RUN, Ordering::SeqCst);
        repo::set_status(&self.store, &id, repo::ST_QUEUED, None).await?;
        self.emit(sink, &id).await;
        let _permit = self.lane(d.size).acquire_owned().await.map_err(|e| MessengerError::Other(e.to_string()))?;
        repo::set_status(&self.store, &id, repo::ST_RUNNING, None).await?;
        self.emit(sink, &id).await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
        let store = self.store.clone();
        let tid = id.clone();
        let writer = tokio::spawn(async move {
            while let Some(done) = rx.recv().await {
                let _ = repo::set_progress(&store, &tid, done as i64, None).await;
            }
        });
        let mirrors = self.own_bases().await;
        let result = download::download(self.fetcher.as_ref(), d, &mirrors, &self.cache_dir, &control, |done, _| {
            let _ = tx.send(done);
        })
        .await;
        drop(tx);
        let _ = writer.await;
        self.controls.lock().unwrap().remove(&id);

        match result {
            Ok(DownloadOutcome::Done(path)) => {
                repo::set_result(&self.store, &id, Some(&path.to_string_lossy()), None, None).await?;
                repo::set_progress(&self.store, &id, d.size as i64, None).await?;
                repo::set_status(&self.store, &id, repo::ST_DONE, None).await?;
                self.emit(sink, &id).await;
                Ok(Some(path))
            }
            Ok(DownloadOutcome::Paused) => {
                repo::set_status(&self.store, &id, repo::ST_PAUSED, None).await?;
                self.emit(sink, &id).await;
                Ok(None)
            }
            Ok(DownloadOutcome::Cancelled) => {
                // The user said no: never start this one automatically.
                repo::set_attempts(&self.store, &id, -1).await?;
                repo::set_status(&self.store, &id, repo::ST_CANCELLED, None).await?;
                self.emit(sink, &id).await;
                Ok(None)
            }
            Err(e) => {
                let base = if manual { 0 } else { before.max(0) };
                repo::set_attempts(&self.store, &id, base + 1).await?;
                repo::set_status(&self.store, &id, repo::ST_FAILED, Some(&short_reason(&e))).await?;
                self.emit(sink, &id).await;
                Err(e)
            }
        }
    }

    /// Ask a running transfer to stop after the current chunk.
    pub fn pause(&self, id: &str) {
        if let Some(c) = self.controls.lock().unwrap().get(id) {
            c.store(PAUSE, Ordering::SeqCst);
        }
    }

    /// Cancel a transfer, running or not.
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let running = self.controls.lock().unwrap().get(id).cloned();
        match running {
            Some(c) => c.store(CANCEL, Ordering::SeqCst),
            None => {
                if let Some(t) = repo::transfer(&self.store, id).await? {
                    if t.direction == repo::DIR_DOWN {
                        repo::set_attempts(&self.store, id, -1).await?;
                    }
                    repo::set_status(&self.store, id, repo::ST_CANCELLED, None).await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use async_trait::async_trait;
    use zeroize::Zeroizing;

    struct Secrets(Mutex<HashMap<String, Vec<u8>>>);
    #[async_trait]
    impl SecretStore for Secrets {
        async fn get(&self, key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
            Ok(self.0.lock().unwrap().get(key).cloned().map(Zeroizing::new))
        }
        async fn put(&self, key: &str, value: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(key.into(), value.to_vec());
            Ok(())
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
        async fn is_unlocked(&self) -> bool {
            true
        }
    }

    /// Reads from the memory backend; can lose or corrupt blobs.
    struct MemFetcher {
        backend: MemoryBackend,
        corrupt: Mutex<Option<String>>,
        calls: Mutex<u32>,
    }
    #[async_trait]
    impl BlobFetcher for MemFetcher {
        async fn fetch(&self, url: &str, _max: u64) -> Result<Option<Vec<u8>>> {
            *self.calls.lock().unwrap() += 1;
            let sha = url.rsplit('/').next().unwrap_or("");
            if !url.starts_with(&self.backend.base) {
                return Ok(None);
            }
            let mut bytes = self.backend.get(sha);
            if self.corrupt.lock().unwrap().as_deref() == Some(sha) {
                if let Some(b) = bytes.as_mut() {
                    b[0] ^= 0xff;
                }
            }
            Ok(bytes)
        }
    }

    #[derive(Default)]
    struct Sink(Mutex<Vec<Progress>>);
    impl ProgressSink for Sink {
        fn progress(&self, p: Progress) {
            self.0.lock().unwrap().push(p);
        }
    }
    impl Sink {
        fn statuses(&self) -> Vec<String> {
            self.0.lock().unwrap().iter().map(|p| p.status.clone()).collect()
        }
    }

    struct Rig {
        svc: MediaService,
        backend: MemoryBackend,
        dir: tempfile::TempDir,
        keys: Keys,
    }

    async fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().await.unwrap();
        let backend = MemoryBackend::new("https://mem.example/a");
        let fetcher = Arc::new(MemFetcher { backend: backend.clone(), corrupt: Mutex::new(None), calls: Mutex::new(0) });
        let svc = MediaService::new(store, Arc::new(Secrets(Mutex::default())), dir.path())
            .unwrap()
            .with_backend(Arc::new(backend.clone()), fetcher.clone());
        Rig { svc, backend, dir, keys: Keys::generate() }
    }

    /// Deterministic content that is not compressible into a pattern of
    /// chunk-sized repeats.
    fn content(len: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0xff) as u8
            })
            .collect()
    }

    async fn file(r: &Rig, name: &str, len: usize) -> PathBuf {
        let p = r.dir.path().join(name);
        tokio::fs::write(&p, content(len)).await.unwrap();
        p
    }

    const MIB: usize = 1024 * 1024;

    #[tokio::test]
    async fn round_trip_of_a_multi_chunk_file() {
        let r = rig().await;
        let path = file(&r, "video.mp4", 9 * MIB + 123).await;
        let sink = Sink::default();
        let t = r.svc.queue_upload(&path, "chat", "placeholder").await.unwrap();
        assert_eq!(t.mime, "video/mp4");
        let d = r.svc.run_upload(&t.id, &r.keys, Some("clip".into()), &sink).await.unwrap().unwrap();
        d.validate().unwrap();
        assert_eq!(d.chunks.len(), 3);
        assert_eq!(d.size as usize, 9 * MIB + 123);
        assert_eq!(d.caption.as_deref(), Some("clip"));
        assert_eq!(d.servers, vec!["https://mem.example/a".to_string()]);
        assert_eq!(r.backend.len(), 3);
        assert_eq!(sink.statuses(), vec!["running", "done"]);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().done_bytes, d.size);

        // The server holds ciphertext only.
        let plain = content(9 * MIB + 123);
        let first = r.backend.get(&d.chunks[0].sha256).unwrap();
        assert_ne!(&first[..1024], &plain[..1024]);

        let (svc, dd, fetch) = https(&r, &d);
        let sink = Sink::default();
        let out = svc.run_download("msg1", "chat", &dd, false, &sink).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&out).await.unwrap(), plain);
        assert!(out.ends_with(format!("{}/video.mp4", d.sha256)), "cache is keyed by hash: {out:?}");
        assert_eq!(sink.statuses(), vec!["queued", "running", "done"]);
        assert!(!crate::download::tmp_dir(svc.cache_dir(), &dd).exists(), "temp chunks are removed");

        // Second request: served from the cache without touching the network.
        let calls_before = *fetch.calls.lock().unwrap();
        svc.run_download("msg1", "chat", &dd, true, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*fetch.calls.lock().unwrap(), calls_before);
    }

    fn https(r: &Rig, d: &MediaDescriptor) -> (MediaService, MediaDescriptor, Arc<MemFetcher>) {
        let mut dd = d.clone();
        dd.servers = vec!["https://mem.example/a".into()];
        let fetch = Arc::new(MemFetcher {
            backend: MemoryBackend { base: "https://mem.example/a".into(), ..r.backend.clone() },
            corrupt: Mutex::new(None),
            calls: Mutex::new(0),
        });
        (r.svc.clone().with_backend(Arc::new(r.backend.clone()), fetch.clone()), dd, fetch)
    }

    #[tokio::test]
    async fn upload_survives_a_flaky_server_and_resumes_after_a_dead_one() {
        let r = rig().await;
        let path = file(&r, "big.bin", 13 * MIB).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();

        // Two transient failures are absorbed by retries.
        *r.backend.flaky_puts.lock().unwrap() = 2;
        // Then the connection dies after two stored chunks.
        *r.backend.die_after_puts.lock().unwrap() = Some(2);
        let sink = Sink::default();
        let err = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap_err();
        assert!(err.to_string().contains("err.network"), "{err}");
        assert_eq!(sink.statuses(), vec!["running", "failed"]);
        let row = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!(row.failure_reason.as_deref(), Some("err.network"));
        assert_eq!(row.done_bytes as usize, 8 * MIB, "two of four chunks made it");
        assert_eq!(r.backend.len(), 2);
        let key_before: UploadState = serde_json::from_str(
            &repo::transfer(&r.svc.store, &t.id).await.unwrap().unwrap().state_json,
        )
        .unwrap();
        assert_eq!(key_before.chunks.len(), 2);

        // The network is back: only the missing chunks are sent, with the same key.
        *r.backend.die_after_puts.lock().unwrap() = None;
        let puts_before = *r.backend.put_calls.lock().unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*r.backend.put_calls.lock().unwrap() - puts_before, 2, "finished chunks are not sent again");
        assert_eq!(d.chunks.len(), 4);
        assert_eq!(&d.chunks[..2], &key_before.chunks[..], "same key, same ciphertext");
        assert_eq!(d.key, key_before.key);

        let (svc, dd, _) = https(&r, &d);
        let out = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(13 * MIB));
    }

    #[tokio::test]
    async fn a_changed_file_starts_over_with_a_new_key() {
        let r = rig().await;
        let path = file(&r, "doc.pdf", 5 * MIB).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(1);
        r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap_err();
        let old: UploadState = serde_json::from_str(&repo::transfer(&r.svc.store, &t.id).await.unwrap().unwrap().state_json).unwrap();

        tokio::fs::write(&path, content(5 * MIB + 1)).await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = None;
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_ne!(d.key, old.key, "never reuse a key for different content");
        assert_eq!(d.size as usize, 5 * MIB + 1);
    }

    #[tokio::test]
    async fn download_resumes_verifies_and_rejects_tampering() {
        let r = rig().await;
        let path = file(&r, "photo.jpg", 9 * MIB).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);

        // A corrupted chunk on the only server: refused, earlier chunks kept.
        *fetch.corrupt.lock().unwrap() = Some(d.chunks[2].sha256.clone());
        let sink = Sink::default();
        let err = svc.run_download("m", "chat", &dd, false, &sink).await.unwrap_err();
        assert!(err.to_string().contains("err.chunk_hash_mismatch"), "{err}");
        assert_eq!(sink.statuses().last().unwrap(), "failed");
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        assert!(tmp.join("0").exists() && tmp.join("1").exists() && !tmp.join("2").exists());
        assert!(svc.may_auto_download("m").await.unwrap(), "one failure does not stop automatic retries");

        // Healthy again: only the missing chunk is fetched.
        *fetch.corrupt.lock().unwrap() = None;
        let before = *fetch.calls.lock().unwrap();
        let out = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*fetch.calls.lock().unwrap() - before, 1);
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(9 * MIB));

        // A descriptor with the wrong file hash: chunks are fine, the file is refused.
        let mut lying = dd.clone();
        lying.sha256 = "0".repeat(64);
        let err = svc.run_download("m2", "chat", &lying, false, &Sink::default()).await.unwrap_err();
        assert!(err.to_string().contains("err.file_hash_mismatch"), "{err}");
        assert!(svc.cached(&lying).await.is_none());

        // A wrong key: authentication fails, nothing reaches the cache.
        let mut wrong = dd.clone();
        wrong.sha256 = "1".repeat(64);
        wrong.set_key(&crate::crypto::FileKey::generate().unwrap());
        assert!(svc.run_download("m3", "chat", &wrong, false, &Sink::default()).await.is_err());
        assert!(svc.cached(&wrong).await.is_none());
    }

    #[tokio::test]
    async fn shared_folders_of_the_old_naming_are_dropped_at_start() {
        let r = rig().await;
        let tmp = r.svc.cache_dir().join("tmp");
        let old = tmp.join("ab".repeat(32));
        let new = tmp.join(format!("{}-0123456789abcdef", "ab".repeat(32)));
        for d in [&old, &new] {
            tokio::fs::create_dir_all(d).await.unwrap();
            tokio::fs::write(d.join("0"), b"chunk").await.unwrap();
        }
        r.svc.recover().await.unwrap();
        assert!(!old.exists(), "a folder every copy shared cannot be resumed from");
        assert!(new.join("0").exists(), "a folder of one copy is kept to resume");
    }

    /// The same picture sent several times (an album of one screenshot):
    /// every copy has its own key, the plaintext hash is shared. All copies
    /// arrive at once and are fetched at once.
    #[tokio::test]
    async fn copies_of_one_file_download_side_by_side() {
        let r = rig().await;
        let mut copies = Vec::new();
        for n in 0..6 {
            let path = file(&r, &format!("{n}.png"), 3 * MIB + 77).await;
            let t = r.svc.queue_upload(&path, "chat", &format!("ph{n}")).await.unwrap();
            copies.push(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap());
        }
        assert!(copies.iter().all(|d| d.sha256 == copies[0].sha256), "one plaintext");
        assert!(copies.windows(2).all(|w| w[0].chunks != w[1].chunks), "different ciphertext");

        let (svc, _, _) = https(&r, &copies[0]);
        let jobs: Vec<_> = copies
            .iter()
            .enumerate()
            // Every copy twice: an album that redraws starts the same download again.
            .flat_map(|(n, d)| [(n, d.clone()), (n, d.clone())])
            .map(|(n, d)| {
                let (svc, mut dd) = (svc.clone(), d);
                dd.servers = vec!["https://mem.example/a".into()];
                tokio::spawn(async move { svc.run_download(&format!("m{n}"), "chat", &dd, true, &Sink::default()).await })
            })
            .collect();
        let want = content(3 * MIB + 77);
        for job in jobs {
            match job.await.unwrap() {
                Ok(Some(out)) => assert_eq!(tokio::fs::read(&out).await.unwrap(), want),
                // The second start of a download already running is refused, never corrupted.
                Err(e) => assert!(e.to_string().contains("err.transfer_in_progress"), "{e}"),
                Ok(None) => panic!("a manual download always answers"),
            }
        }
        for (n, d) in copies.iter().enumerate() {
            assert!(svc.cached(d).await.is_some(), "copy {n} is on this device");
        }
    }

    #[tokio::test]
    async fn mirrors_are_tried_and_repeated_failures_stop_auto_download() {
        let r = rig().await;
        let path = file(&r, "a.txt", 70_000).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, mut dd, _) = https(&r, &d);

        // First listed server is gone, the second has the file.
        dd.servers = vec!["https://dead.example/x".into(), "https://mem.example/a".into()];
        let out = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(70_000));

        // Nobody has it: three failures, then automatic attempts stop.
        let mut gone = dd.clone();
        gone.sha256 = "2".repeat(64);
        gone.servers = vec!["https://dead.example/x".into()];
        for _ in 0..MAX_AUTO_ATTEMPTS {
            assert!(svc.may_auto_download("lost").await.unwrap());
            let err = svc.run_download("lost", "chat", &gone, false, &Sink::default()).await.unwrap_err();
            assert!(err.to_string().contains("err.not_found"));
        }
        assert!(!svc.may_auto_download("lost").await.unwrap());
        // A manual retry resets the counter.
        svc.run_download("lost", "chat", &gone, true, &Sink::default()).await.unwrap_err();
        assert!(svc.may_auto_download("lost").await.unwrap());

        // Cancelling means "do not start again by yourself".
        let tid = svc.transfer_for_message("lost", repo::DIR_DOWN).await.unwrap().unwrap().id;
        svc.cancel(&tid).await.unwrap();
        assert!(!svc.may_auto_download("lost").await.unwrap());
        assert_eq!(svc.transfer(&tid).await.unwrap().unwrap().attempts, -1);
    }

    #[tokio::test]
    async fn pause_and_resume_an_upload() {
        let r = rig().await;
        let path = file(&r, "big.bin", 9 * MIB).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        // Pause from the progress callback of the first chunk.
        let svc = r.svc.clone();
        let id = t.id.clone();
        let watcher = tokio::spawn(async move {
            loop {
                if svc.transfer(&id).await.unwrap().unwrap().done_bytes > 0 {
                    svc.pause(&id);
                    break;
                }
                tokio::task::yield_now().await;
            }
        });
        let out = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap();
        watcher.await.unwrap();
        if out.is_none() {
            let row = r.svc.transfer(&t.id).await.unwrap().unwrap();
            assert_eq!(row.status, "paused");
            assert!(row.done_bytes > 0 && (row.done_bytes as usize) < 9 * MIB);
            let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
            assert_eq!(d.chunks.len(), 3);
        }
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "done");
    }

    #[tokio::test]
    async fn refusals_and_restart_recovery() {
        let r = rig().await;
        assert!(r.svc.queue_upload(&r.dir.path().join("missing"), "c", "m").await.unwrap_err().to_string().contains("err.file_not_found"));
        let empty = r.dir.path().join("empty");
        tokio::fs::write(&empty, b"").await.unwrap();
        assert!(r.svc.queue_upload(&empty, "c", "m").await.is_err());
        assert!(r.svc.queue_upload(r.dir.path(), "c", "m").await.is_err(), "directories are not files");

        let path = file(&r, "x.bin", 1000).await;
        let t = r.svc.queue_upload(&path, "c", "m").await.unwrap();
        repo::set_status(&r.svc.store, &t.id, repo::ST_RUNNING, None).await.unwrap();
        assert_eq!(r.svc.recover().await.unwrap(), 1);
        let row = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.failure_reason.as_deref()), ("paused", Some("err.interrupted")));
        assert_eq!(r.svc.active_transfers().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn servers_keep_secrets_out_of_views() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().await.unwrap();
        let secrets = Arc::new(Secrets(Mutex::default()));
        let svc = MediaService::new(store, secrets.clone(), dir.path()).unwrap();
        let keys = Keys::generate();
        assert!(svc.upload_backend(&keys).await.err().unwrap().to_string().contains("err.media_no_server"));

        let v = svc
            .put_server(MediaServerInput {
                kind: "S3".into(),
                url: "https://s3.example:9000/".into(),
                bucket: Some("veydan-media".into()),
                access_key: Some("AK".into()),
                secret_key: Some("very-secret".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(v.id, "s3-s3-example-9000-veydan-media");
        assert_eq!(v.public_base, "https://s3.example:9000/veydan-media");
        assert!(v.has_secret);
        assert!(!serde_json::to_string(&v).unwrap().contains("very-secret"));
        assert_eq!(v.region.as_deref(), Some("us-east-1"));
        assert!(secrets.0.lock().unwrap().contains_key("media.s3-s3-example-9000-veydan-media.secret"));

        let b = svc
            .put_server(MediaServerInput { kind: "blossom".into(), url: "https://blossom.example".into(), ..Default::default() })
            .await
            .unwrap();
        let list = svc.servers().await.unwrap();
        assert_eq!(list.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(), vec!["s3", "blossom"], "s3 has priority");
        assert_eq!(svc.upload_backend(&keys).await.unwrap().public_base(), "https://s3.example:9000/veydan-media");

        svc.set_server_enabled(&v.id, false).await.unwrap();
        assert_eq!(svc.upload_backend(&keys).await.unwrap().public_base(), "https://blossom.example");
        svc.remove_server(&v.id).await.unwrap();
        assert!(secrets.0.lock().unwrap().is_empty(), "the secret goes with the server");
        svc.remove_server(&b.id).await.unwrap();

        assert!(svc.put_server(MediaServerInput { kind: "ftp".into(), url: "https://x".into(), ..Default::default() }).await.is_err());
        assert!(svc.put_server(MediaServerInput { kind: "s3".into(), url: "https://x".into(), ..Default::default() }).await.is_err(), "bucket required");
        assert!(svc.put_server(MediaServerInput { kind: "blossom".into(), url: "file:///x".into(), ..Default::default() }).await.is_err());
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Servers, transfers and scheduling on top of `upload`/`download`.
//!
//! Scheduling: two lanes. Small transfers have their own slots, so a
//! gigabyte in flight never makes a photo wait. Under the lanes, the chunk
//! requests of all transfers share one set of slots, so several big files
//! never open more than a few requests at once; large transfers never take
//! the last `SMALL_RESERVED` of them, so a photo's chunks never queue
//! behind a big file's either. Every transfer can be paused, resumed and
//! cancelled at once, also in the middle of a chunk; a restart turns what
//! was running into paused.
//!
//! A transfer runs in the process that started it, one run at a time in
//! all processes with the same data folder (the CLI beside the app; see
//! `runs`). Another process neither pauses it at start nor removes its
//! chunks: it asks the run, which pauses or cancels itself, and it shows
//! the run's stage, speed and time left as the run tells them.
//!
//! An upload ends when its message is out: the runtime publishes the
//! message in the publishing stage (`upload_to_publish`) and only then is
//! the transfer done. A cancel is heeded until the run decides its end;
//! one that comes later waits for that end and acts on it.
//!
//! A failure that may pass (the network, a timeout, a busy server) is
//! retried by itself: the transfer waits (`waiting_retry`) 5 s, 15 s and
//! 60 s, then queues for a slot again (`queued`), and every retry moves
//! only the chunks still missing. A pause, a cancel or "retry now" ends the
//! wait. After the third retry, or at once
//! for a failure that will not pass, the transfer has failed.
//!
//! Public blobs (an avatar) skip all of that: one small piece, not
//! encrypted, stored whole on as many of the enabled servers as asked.

use crate::backend::{valid_content_type, BackendError, BackendResult, BlobBackend, BlossomBackend, S3Backend, S3Config};
use crate::control::{pauses, Control, TransferCtx, CANCEL, CHUNK_SLOTS, INTERRUPT, PAUSE, SMALL_RESERVED, WORKERS};
use crate::descriptor::{mime_for, safe_name, MediaDescriptor};
use crate::download::{self, BlobFetcher, DownloadOutcome, DownloadState, HttpFetcher};
use crate::progress::{Live, ProgressMeter, HEARTBEAT, UNDER_WAY};
pub use crate::progress::{Progress, ProgressSink, TransferStage};
use crate::runs::{self, RunLock, Word};
use crate::upload::{self, UploadOutcome, UploadParams, UploadState};
use messenger_core::{MessengerError, Result, SecretStore};
use messenger_store::media::{self as repo, ServerRow, TransferRow};
use messenger_store::Store;
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use ts_rs::TS;

/// Transfers at or below this size use the fast lane.
pub const SMALL_BYTES: u64 = 8 * 1024 * 1024;
const SMALL_SLOTS: usize = 3;
const LARGE_SLOTS: usize = 2;
/// A download that failed this many times is not started automatically.
pub const MAX_AUTO_ATTEMPTS: i64 = 3;
/// The waits before the automatic retries of one run.
pub const RETRY_DELAYS: [Duration; 3] = [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(60)];
/// A cancel removes the chunks whose request was dropped once more after
/// this long: the server may still have been writing one.
pub const CLEANUP_GRACE: Duration = Duration::from_secs(30);
/// A row that says "under way" with no run here settles within this long
/// (a run about to start registers, one that just ended writes its end).
const SETTLE: Duration = Duration::from_secs(2);
/// Held (shared) by every process that has the data folder open; see
/// `MediaService::recover`.
const LOCK_FILE: &str = "transfers.lock";
/// How often a run looks for a word left for it by another process.
const ASK_POLL: Duration = Duration::from_millis(250);
/// How long a run in another process may take to heed a cancel before
/// it is taken for one that does not hear (`err.transfer_elsewhere`).
const ASK_WAIT: Duration = Duration::from_secs(10);

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

/// A transfer as the UI reads it when it opens and on demand; while it
/// runs, `transfer.progress` events tell the same and more.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TransferView {
    pub id: String,
    #[ts(type = "\"up\" | \"down\"")]
    pub direction: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub file_name: String,
    pub mime: String,
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "\"queued\" | \"running\" | \"waiting_retry\" | \"paused\" | \"done\" | \"failed\" | \"cancelled\"")]
    pub status: String,
    #[ts(type = "number")]
    pub done_bytes: u64,
    /// Uploads: runs started. Downloads: runs that ended failed, -1 once
    /// cancelled.
    #[ts(type = "number")]
    pub attempts: i64,
    /// Always an `err.*` code; `err.interrupted` for a transfer paused
    /// because its app closed, which starts again by itself.
    pub failure_reason: Option<String>,
    pub local_path: Option<String>,
    pub stage: TransferStage,
    pub chunks_done: u32,
    pub chunks_total: u32,
    #[ts(type = "number")]
    pub chunk_size: u64,
    /// Known while the transfer runs, here or in another process with this
    /// data folder: speed, time left and when the next automatic attempt
    /// starts.
    #[ts(type = "number")]
    pub rate_bps: u64,
    #[ts(type = "number | null")]
    pub eta_secs: Option<u64>,
    #[ts(type = "number | null")]
    pub retry_at_ms: Option<i64>,
}

impl From<TransferRow> for TransferView {
    fn from(r: TransferRow) -> Self {
        let (chunks_done, chunks_total, chunk_size) = chunk_counts(&r);
        let stage = match (r.status.as_str(), r.direction.as_str()) {
            (repo::ST_QUEUED, _) => TransferStage::Queued,
            (_, repo::DIR_UP) => TransferStage::Uploading,
            _ => TransferStage::Downloading,
        };
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
            stage,
            chunks_done,
            chunks_total,
            chunk_size,
            rate_bps: 0,
            eta_secs: None,
            retry_at_ms: None,
        }
    }
}

impl TransferView {
    /// What only memory knows of a transfer running here.
    fn with_live(mut self, p: Option<&Progress>) -> Self {
        if let Some(p) = p {
            self.stage = p.stage;
            self.chunks_done = p.chunks_done;
            self.chunks_total = p.chunks_total;
            self.chunk_size = p.chunk_size;
            self.rate_bps = p.rate_bps;
            self.eta_secs = p.eta_secs;
            self.retry_at_ms = p.retry_at_ms;
            self.done_bytes = self.done_bytes.max(p.done_bytes);
        }
        self
    }
}

/// Chunks done, chunks in all and their size, from what the row keeps.
fn chunk_counts(r: &TransferRow) -> (u32, u32, u64) {
    let (done, total, size) = if r.direction == repo::DIR_UP {
        match serde_json::from_str::<UploadState>(&r.state_json) {
            Ok(s) => (s.chunks_done(), s.chunks_total(), s.chunk_size),
            Err(_) => (0, 0, 0),
        }
    } else {
        match serde_json::from_str::<DownloadState>(&r.state_json) {
            Ok(s) => (s.chunks_done, s.chunks_total, s.chunk_size),
            Err(_) => (0, 0, 0),
        }
    };
    if r.status == repo::ST_DONE {
        (total, total, size)
    } else {
        (done.min(total), total, size)
    }
}

/// An event that says what the view says.
pub fn progress_of(v: &TransferView) -> Progress {
    Progress {
        transfer_id: v.id.clone(),
        message_id: v.message_id.clone(),
        chat_id: v.chat_id.clone(),
        direction: v.direction.clone(),
        status: v.status.clone(),
        done_bytes: v.done_bytes,
        total_bytes: v.size,
        failure_reason: v.failure_reason.clone(),
        local_path: v.local_path.clone(),
        stage: v.stage,
        chunks_done: v.chunks_done,
        chunks_total: v.chunks_total,
        chunk_size: v.chunk_size,
        rate_bps: v.rate_bps,
        eta_secs: v.eta_secs,
        retry_at_ms: v.retry_at_ms,
        attempt: 0,
        file_name: v.file_name.clone(),
        mime: v.mime.clone(),
    }
}

/// A public blob on one server: readable by anyone at `url`
/// (`<public_base>/<sha256>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicBlob {
    pub server_id: String,
    pub url: String,
}

/// Public blobs are small and go in one piece.
pub const MAX_PUBLIC_BYTES: usize = SMALL_BYTES as usize;
/// The server id of public blobs while a fixed backend stands in for the
/// configured servers (tests, offline development).
pub const FIXED_SERVER_ID: &str = "fixed";
/// How long one request about a public blob may take before the server
/// is given up (the shared client allows minutes, made for big chunks).
pub const PUBLIC_TIMEOUT: Duration = Duration::from_secs(30);

/// The time an upload of `len` bytes gets: the base and a second for
/// every 64 KiB, so a slow line still carries the largest public blob.
fn public_timeout_for(base: Duration, len: usize) -> Duration {
    base + Duration::from_secs((len / (64 * 1024)) as u64)
}

/// `call`, or a network error (worth another server) when it takes longer
/// than `limit`.
async fn within<T>(limit: Duration, call: impl std::future::Future<Output = BackendResult<T>>) -> BackendResult<T> {
    tokio::time::timeout(limit, call)
        .await
        .unwrap_or_else(|_| Err(BackendError { status: None, message: "the server did not answer in time".into() }))
}

/// A blob name: 64 lowercase hex characters.
fn check_sha256(sha256: &str) -> Result<()> {
    if sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(())
    } else {
        Err(MessengerError::Invalid("err.media_bad_hash".into()))
    }
}

fn secret_ref(id: &str) -> String {
    format!("media.{id}.secret")
}

/// Where a server's blobs are read, from its row alone (no credentials).
fn public_base_of(r: &ServerRow) -> String {
    match r.kind.as_str() {
        repo::KIND_S3 => format!("{}/{}", r.url.trim_end_matches('/'), r.bucket.clone().unwrap_or_default()),
        _ => r.url.trim_end_matches('/').to_string(),
    }
}

fn new_id(prefix: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{prefix}-{t:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The stable code of an error (`err.*`) for the UI to translate; never
/// the text of the error itself.
pub fn short_reason(e: &MessengerError) -> String {
    let s = e.to_string();
    if let Some(i) = s.find("err.") {
        let code: String = s[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
        let code = code.trim_end_matches('.');
        if code.len() > "err.".len() {
            return code.to_string();
        }
    }
    match e {
        MessengerError::Io(_) => "err.io",
        MessengerError::Crypto(_) => "err.crypto",
        MessengerError::Transport(_) => "err.network",
        MessengerError::Storage(_) => "err.storage",
        MessengerError::SecretsLocked => "err.secrets_locked",
        MessengerError::SecretMissing(_) => "err.media_no_credentials",
        MessengerError::Invalid(_) => "err.invalid",
        _ => "err.unknown",
    }
    .to_string()
}

/// A failure that may pass by itself: worth an automatic retry.
fn retryable(e: &MessengerError) -> bool {
    matches!(short_reason(e).as_str(), "err.network" | "err.timeout" | "err.server" | "err.rate_limited")
}

/// How a run of a transfer was stopped by the user (or by the end of the
/// service, see `stop_reason`).
fn stop_status(stop: u8) -> &'static str {
    if pauses(stop) {
        repo::ST_PAUSED
    } else {
        repo::ST_CANCELLED
    }
}

/// The reason a run that stopped with `stop` gives: none for what the user
/// asked, `err.interrupted` for a pause the end of the service made
/// (`MediaService::release`).
fn stop_reason(stop: u8) -> Option<&'static str> {
    (stop == INTERRUPT).then_some(repo::REASON_INTERRUPTED)
}

#[derive(Clone)]
pub struct MediaService {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    cache_dir: PathBuf,
    fetcher: Arc<dyn BlobFetcher>,
    controls: Arc<Mutex<HashMap<String, Control>>>,
    small: Arc<Semaphore>,
    large: Arc<Semaphore>,
    /// Chunk requests of all transfers together.
    chunk_slots: Arc<Semaphore>,
    /// Those of large transfers together: all but `SMALL_RESERVED`.
    large_chunks: Arc<Semaphore>,
    /// Chunks of one transfer at once.
    workers: usize,
    /// The last event of every transfer running here, for views read
    /// meanwhile.
    live_views: Arc<Mutex<HashMap<String, Progress>>>,
    /// The waits before the automatic retries of one run.
    retry_delays: Vec<Duration>,
    /// Chunk size of new uploads instead of `chunk_size_for` (tests).
    chunk_size: Option<u64>,
    /// Tests and offline development: use this instead of configured servers.
    fixed_backend: Option<Arc<dyn BlobBackend>>,
    /// Base time of one request about a public blob.
    public_timeout: Duration,
    /// See `CLEANUP_GRACE`.
    cleanup_grace: Duration,
    /// See `ASK_WAIT`.
    ask_wait: Duration,
    /// The lock file of the data folder, held for as long as the service
    /// lives.
    folder: Arc<Mutex<Option<std::fs::File>>>,
    /// Another process had the data folder open at start-up: rows that
    /// say "under way" may be its transfers. Asked only where file locks
    /// cannot tell (see `runs`).
    others: Arc<AtomicBool>,
    /// Chunks being removed in the background (`remove_chunks`).
    removals: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Tests: a run waits here before it decides its end.
    #[cfg(test)]
    ending_gate: Arc<Mutex<Option<Arc<Semaphore>>>>,
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
            chunk_slots: Arc::new(Semaphore::new(CHUNK_SLOTS)),
            large_chunks: Arc::new(Semaphore::new(CHUNK_SLOTS - SMALL_RESERVED)),
            workers: WORKERS,
            live_views: Arc::default(),
            retry_delays: RETRY_DELAYS.to_vec(),
            chunk_size: None,
            fixed_backend: None,
            public_timeout: PUBLIC_TIMEOUT,
            cleanup_grace: CLEANUP_GRACE,
            ask_wait: ASK_WAIT,
            folder: Arc::default(),
            others: Arc::default(),
            removals: Arc::default(),
            #[cfg(test)]
            ending_gate: Arc::default(),
        })
    }

    /// Replace network access (tests).
    pub fn with_backend(mut self, backend: Arc<dyn BlobBackend>, fetcher: Arc<dyn BlobFetcher>) -> Self {
        self.fixed_backend = Some(backend);
        self.fetcher = fetcher;
        self
    }

    /// Other waits before the automatic retries (tests, tools).
    pub fn with_retry_delays(mut self, delays: Vec<Duration>) -> Self {
        self.retry_delays = delays;
        self
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Call once at start-up: a transfer that was under way when its
    /// process ended is paused. While another process has this data folder
    /// open, its transfers may be running: only those no run holds are
    /// paused, and where that cannot be told a row is left as it is.
    pub async fn recover(&self) -> Result<Recovered> {
        if !self.hold_folder().await {
            self.others.store(true, Ordering::SeqCst);
            return Ok(Recovered::Beside(self.pause_unheld().await?));
        }
        self.others.store(false, Ordering::SeqCst);
        crate::download::drop_shared_folders(&self.cache_dir).await;
        runs::sweep(&self.cache_dir);
        Ok(Recovered::Alone(repo::pause_interrupted(&self.store).await?))
    }

    /// Pause the transfers under way whose run is gone (`runs`). The
    /// placeholders of the uploads left under way: their run may live in
    /// the other process.
    async fn pause_unheld(&self) -> Result<Vec<String>> {
        let mut under_way = Vec::new();
        for t in repo::transfers_with_status(&self.store, &UNDER_WAY).await? {
            let paused = runs::held(&self.cache_dir, &t.id) == Some(false)
                && match runs::take(&self.cache_dir, &t.id).await {
                    // Held while the row is paused: no run starts on it meanwhile.
                    Ok(Some(_lock)) => repo::pause_interrupted_one(&self.store, &t.id).await?,
                    _ => false,
                };
            if !paused && t.direction == repo::DIR_UP {
                under_way.extend(t.message_id);
            }
        }
        Ok(under_way)
    }

    /// Hold the lock file of the data folder, shared, for as long as the
    /// service lives, so that a process that starts later sees this one.
    /// Whether no other process held it. A system without file locks is
    /// taken as one process.
    async fn hold_folder(&self) -> bool {
        // Our own hold would count as another process.
        self.folder.lock().unwrap().take();
        let path = self.cache_dir.join(LOCK_FILE);
        let file = std::fs::create_dir_all(&self.cache_dir)
            .and_then(|_| std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path));
        let Ok(file) = file else { return true };
        let alone = match file.try_lock() {
            Ok(()) => file.unlock().is_ok(),
            Err(std::fs::TryLockError::WouldBlock) => false,
            Err(std::fs::TryLockError::Error(_)) => return true,
        };
        // Another process may hold it alone for a moment while it looks.
        for _ in 0..50 {
            if file.try_lock_shared().is_ok() {
                *self.folder.lock().unwrap() = Some(file);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        alone
    }

    /// The service stops (the messenger is turned off, and may be started
    /// again in this process): what runs here is paused, and the data
    /// folder is let go, though clones of the service live on. A service
    /// started next is alone in the folder and recovers as after a restart.
    /// What it pauses is paused as interrupted (`err.interrupted`), as a
    /// restart leaves it, and starts again by itself; a pause the user
    /// asked for before stays the user's (`Control::ask`).
    pub fn release(&self) {
        for c in self.controls.lock().unwrap().values() {
            c.ask(INTERRUPT);
        }
        self.folder.lock().unwrap().take();
    }

    // ─── Servers ────────────────────────────────────────────────────────────

    async fn view(&self, r: ServerRow) -> MediaServerView {
        let has_secret = self.secrets.get(&secret_ref(&r.id)).await.ok().flatten().is_some();
        let public_base = public_base_of(&r);
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
            repo::KIND_S3 => self.build_s3(r).await,
            _ => Ok(Arc::new(BlossomBackend::new(&r.url, keys.clone())?)),
        }
    }

    /// An S3 server is written to with its own credentials, not my keys.
    async fn build_s3(&self, r: &ServerRow) -> Result<Arc<dyn BlobBackend>> {
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

    // ─── Public blobs ───────────────────────────────────────────────────────

    /// Enabled servers that can be written to, in priority order. One that
    /// cannot be built (an S3 server without its credentials) is left out.
    async fn usable_servers(&self, keys: &Keys) -> Result<Vec<(String, Arc<dyn BlobBackend>)>> {
        if let Some(b) = &self.fixed_backend {
            return Ok(vec![(FIXED_SERVER_ID.to_string(), b.clone())]);
        }
        let mut out = Vec::new();
        for r in repo::servers(&self.store).await?.into_iter().filter(|r| r.enabled) {
            if let Ok(b) = self.build(&r, keys).await {
                out.push((r.id, b));
            }
        }
        Ok(out)
    }

    /// One server by id, enabled or not (a blob left on a server that was
    /// turned off can still be checked and removed).
    async fn server_backend(&self, keys: &Keys, server_id: &str) -> Result<Arc<dyn BlobBackend>> {
        if let Some(b) = &self.fixed_backend {
            return Ok(b.clone());
        }
        let row = repo::server(&self.store, server_id)
            .await?
            .ok_or_else(|| MessengerError::Invalid("err.media_unknown_server".into()))?;
        self.build(&row, keys).await
    }

    /// Store `bytes` unencrypted under their SHA-256 on up to `copies`
    /// enabled servers, tried in priority order; a server that fails is
    /// passed over, and so is one that does not answer in time. The first
    /// success comes first. Fails when no server took the blob:
    /// `err.media_no_server` when none can be used (none enabled, or none
    /// with its credentials), else the error of the last one tried.
    pub async fn upload_public(&self, keys: &Keys, bytes: Vec<u8>, content_type: &str, copies: usize) -> Result<Vec<PublicBlob>> {
        if bytes.is_empty() {
            return Err(MessengerError::Invalid("the file is empty".into()));
        }
        if bytes.len() > MAX_PUBLIC_BYTES {
            return Err(MessengerError::Invalid("err.file_too_large".into()));
        }
        if !valid_content_type(content_type) {
            return Err(MessengerError::Invalid("bad content type".into()));
        }
        let sha = crate::crypto::sha256_hex(&bytes);
        let servers = self.usable_servers(keys).await?;
        if servers.is_empty() {
            return Err(MessengerError::Invalid("err.media_no_server".into()));
        }
        let limit = public_timeout_for(self.public_timeout, bytes.len());
        let (mut out, mut last) = (Vec::new(), None);
        for (server_id, backend) in servers {
            if out.len() >= copies.max(1) {
                break;
            }
            match within(limit, backend.put_typed(&sha, bytes.clone(), content_type)).await {
                Ok(()) => out.push(PublicBlob { server_id, url: MediaDescriptor::chunk_url(&backend.public_base(), &sha) }),
                Err(e) => last = Some(e),
            }
        }
        match (out.is_empty(), last) {
            (true, Some(e)) => Err(e.into()),
            (true, None) => Err(MessengerError::Invalid("err.media_no_server".into())),
            _ => Ok(out),
        }
    }

    /// Store `bytes` as a public blob on that one server again (a copy the
    /// server lost), so the address it had stays good.
    pub async fn put_public(&self, keys: &Keys, server_id: &str, bytes: Vec<u8>, content_type: &str) -> Result<PublicBlob> {
        if bytes.is_empty() || bytes.len() > MAX_PUBLIC_BYTES {
            return Err(MessengerError::Invalid("err.file_too_large".into()));
        }
        if !valid_content_type(content_type) {
            return Err(MessengerError::Invalid("bad content type".into()));
        }
        let sha = crate::crypto::sha256_hex(&bytes);
        let backend = self.server_backend(keys, server_id).await?;
        let limit = public_timeout_for(self.public_timeout, bytes.len());
        within(limit, backend.put_typed(&sha, bytes, content_type)).await?;
        Ok(PublicBlob { server_id: server_id.to_string(), url: MediaDescriptor::chunk_url(&backend.public_base(), &sha) })
    }

    /// Is the blob on that server?
    pub async fn public_exists(&self, keys: &Keys, server_id: &str, sha256: &str) -> Result<bool> {
        check_sha256(sha256)?;
        let backend = self.server_backend(keys, server_id).await?;
        Ok(within(self.public_timeout, backend.exists(sha256)).await?)
    }

    /// Remove the blob from that server; a blob already gone is fine.
    pub async fn public_delete(&self, keys: &Keys, server_id: &str, sha256: &str) -> Result<()> {
        check_sha256(sha256)?;
        let backend = self.server_backend(keys, server_id).await?;
        Ok(within(self.public_timeout, backend.delete(sha256)).await?)
    }

    /// `(server id, public base)` of every enabled server that can be
    /// written to, in priority order: where public blobs go.
    pub async fn enabled_public_bases(&self, keys: &Keys) -> Result<Vec<(String, String)>> {
        let servers = self.usable_servers(keys).await?;
        Ok(servers.into_iter().map(|(id, b)| (id, b.public_base())).collect())
    }

    /// `(server id, public base)` of every enabled server, whether it can
    /// be written to now or not: an S3 server whose secret cannot be read
    /// at the moment (a keyring still locked) is still one I use.
    pub async fn enabled_server_bases(&self) -> Result<Vec<(String, String)>> {
        if let Some(b) = &self.fixed_backend {
            return Ok(vec![(FIXED_SERVER_ID.to_string(), b.public_base())]);
        }
        Ok(repo::servers(&self.store).await?.into_iter().filter(|r| r.enabled).map(|r| (r.id.clone(), public_base_of(&r))).collect())
    }

    // ─── Transfers ──────────────────────────────────────────────────────────

    /// Register the one run of transfer `id`, with a control of its own,
    /// and take its lock (`runs`). Refused while another run of it is
    /// here or in another process; one here that is deciding its end is
    /// waited for. Its control hears no word until the run has claimed its
    /// row (`Run::enter`): a run refused on the way never takes a word
    /// meant for the one that holds the transfer.
    async fn register(&self, id: &str) -> Result<Run> {
        settle_here(&self.controls, id, None).await?;
        let lock = runs::take(&self.cache_dir, id).await.map_err(|_| in_progress())?;
        let (controls, views) = (self.controls.clone(), self.live_views.clone());
        Ok(Run { controls, views, id: id.to_string(), control: Control::new(), lock })
    }

    fn lane(&self, size: u64) -> Arc<Semaphore> {
        if size <= SMALL_BYTES {
            self.small.clone()
        } else {
            self.large.clone()
        }
    }

    /// A slot of the lane, or the word of the user that came while
    /// waiting for one (`Err(PAUSE | CANCEL)`).
    async fn slot(&self, size: u64, control: &Control) -> Result<std::result::Result<OwnedSemaphorePermit, u8>> {
        tokio::select! {
            biased;
            stop = control.stopped() => Ok(Err(stop)),
            permit = self.lane(size).acquire_owned() => Ok(Ok(permit.map_err(|e| MessengerError::Other(e.to_string()))?)),
        }
    }

    /// What the workers of a transfer of `size` bytes share.
    fn ctx(&self, control: &Control, live: &Arc<Live>, size: u64) -> TransferCtx {
        TransferCtx {
            control: control.clone(),
            slots: self.chunk_slots.clone(),
            lane: (size > SMALL_BYTES).then(|| self.large_chunks.clone()),
            workers: self.workers,
            live: live.clone(),
        }
    }

    /// Beside a run, for other processes with this data folder: a word
    /// left for it there reaches its control within moments, and its last
    /// event is shown there about once a second (`runs`).
    async fn beside(&self, id: &str, control: &Control) -> std::convert::Infallible {
        let mut tick = tokio::time::interval(ASK_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut shown: Option<tokio::time::Instant> = None;
        loop {
            tick.tick().await;
            match runs::take_ask(&self.cache_dir, id) {
                Some(Word::Pause) => {
                    control.ask(PAUSE);
                }
                Some(Word::Cancel) => {
                    control.ask(CANCEL);
                }
                Some(Word::Retry) => control.wake(),
                None => {}
            }
            if shown.is_none_or(|t| t.elapsed() >= HEARTBEAT) {
                let view = self.live_views.lock().unwrap().get(id).cloned();
                if let Some(p) = view {
                    runs::write_view(&self.cache_dir, id, &p);
                    shown = Some(tokio::time::Instant::now());
                }
            }
        }
    }

    /// While the message of an upload goes out its run hears no word, but
    /// other processes still see its view, about once a second, and wait
    /// for its end (`publishing_elsewhere`).
    fn show_publishing(&self, id: &str) -> Showing {
        let (cache, views, id) = (self.cache_dir.clone(), self.live_views.clone(), id.to_string());
        let show = move || {
            let view = views.lock().unwrap().get(&id).cloned();
            if let Some(p) = view {
                runs::write_view(&cache, &id, &p);
            }
        };
        // At once: the stage it shows is what others go by.
        show();
        Showing(tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEARTBEAT).await;
                show();
            }
        }))
    }

    /// Does a run of `id` in another process publish its message now? It
    /// hears no word then, and decides its end by itself.
    fn publishing_elsewhere(&self, id: &str) -> bool {
        runs::read_view(&self.cache_dir, id).is_some_and(|p| p.stage == TransferStage::Publishing)
    }

    /// Tests: wait at the ending gate, while one is set.
    #[cfg(test)]
    async fn at_ending(&self) {
        let gate = self.ending_gate.lock().unwrap().clone();
        if let Some(g) = gate {
            if let Ok(p) = g.acquire().await {
                p.forget();
            }
        }
    }

    /// The meter of one run. It starts where the row says the last run
    /// stopped (bytes, chunks and their count), so a resumed transfer
    /// never shows 0 of 0 first.
    fn meter<'a>(&self, sink: &'a dyn ProgressSink, r: &TransferRow) -> ProgressMeter<'a> {
        let mut base = progress_of(&TransferView::from(r.clone()));
        base.stage = TransferStage::Queued;
        if [repo::ST_DONE, repo::ST_CANCELLED].contains(&r.status.as_str()) {
            // Done before, or cancelled and cleaned up: it starts anew.
            (base.done_bytes, base.chunks_done) = (0, 0);
        }
        let live = Arc::new(Live::default());
        if base.chunks_total > 0 {
            live.start(base.chunks_total, base.chunk_size);
        }
        ProgressMeter::new(sink, live, base).with_views(self.live_views.clone())
    }

    fn transfer_view(&self, r: TransferRow) -> TransferView {
        let mut live = self.live_views.lock().unwrap().get(&r.id).cloned();
        // A run in another process shows its last event beside the row.
        if live.is_none() && UNDER_WAY.contains(&r.status.as_str()) {
            live = runs::read_view(&self.cache_dir, &r.id);
        }
        TransferView::from(r).with_live(live.as_ref())
    }

    pub async fn transfer(&self, id: &str) -> Result<Option<TransferView>> {
        Ok(repo::transfer(&self.store, id).await?.map(|r| self.transfer_view(r)))
    }

    pub async fn transfer_for_message(&self, message_id: &str, direction: &str) -> Result<Option<TransferView>> {
        Ok(repo::transfer_for_message(&self.store, message_id, direction).await?.map(|r| self.transfer_view(r)))
    }

    /// Every transfer that is not over: queued, running, waiting for its
    /// next attempt, paused or failed. The newest first.
    pub async fn active_transfers(&self) -> Result<Vec<TransferView>> {
        let statuses = [repo::ST_RUNNING, repo::ST_WAITING_RETRY, repo::ST_QUEUED, repo::ST_PAUSED, repo::ST_FAILED];
        Ok(repo::transfers_newest_first(&self.store, &statuses).await?.into_iter().map(|r| self.transfer_view(r)).collect())
    }

    /// Transfers the end of their process or of their service paused
    /// (`err.interrupted`), and that nothing runs anywhere now: uploads,
    /// and downloads the user started. They may start again by
    /// themselves; a pause the user made is never one of them. Where file
    /// locks cannot tell whether another process runs one, it is left.
    pub async fn interrupted(&self) -> Result<Vec<TransferView>> {
        let mut out = Vec::new();
        for r in repo::transfers_newest_first(&self.store, &[repo::ST_PAUSED]).await? {
            if r.failure_reason.as_deref() != Some(repo::REASON_INTERRUPTED) || self.run_alive(&r.id) {
                continue;
            }
            let by_hand = r.direction == repo::DIR_UP
                || serde_json::from_str::<DownloadState>(&r.state_json).is_ok_and(|s| s.manual);
            if by_hand {
                out.push(self.transfer_view(r));
            }
        }
        Ok(out)
    }

    /// An upload paused or failed before, that nothing runs, goes back to
    /// the queue, as a new one stands there before its run: a pause or a
    /// cancel that comes before the run starts is heard. Whether it did.
    pub async fn queue_again(&self, id: &str) -> Result<bool> {
        if self.run_alive(id) {
            return Ok(false);
        }
        repo::claim(&self.store, id, repo::ST_QUEUED, &[repo::ST_PAUSED, repo::ST_FAILED]).await
    }

    /// Tell `stage` of an upload no run has taken yet, as its row says it
    /// otherwise: the runtime makes a photo smaller before the run starts
    /// (`TransferStage::Preparing`).
    /// Views read meanwhile say it too, until `stage_over`.
    pub async fn emit_stage(&self, id: &str, stage: TransferStage, sink: &dyn ProgressSink) -> Result<()> {
        if let Some(view) = self.transfer(id).await? {
            let p = Progress { stage, ..progress_of(&view) };
            self.live_views.lock().unwrap().insert(id.to_string(), p.clone());
            sink.progress(p);
        }
        Ok(())
    }

    /// The stage `emit_stage` told is over: views say what the row says,
    /// or what the run that took the transfer since tells.
    pub fn stage_over(&self, id: &str, stage: TransferStage) {
        let mut views = self.live_views.lock().unwrap();
        if views.get(id).is_some_and(|p| p.stage == stage) {
            views.remove(id);
        }
    }

    /// The upload sends `path` instead of the file it was queued with (a
    /// photo made smaller): its name, type and size follow. Only while it
    /// is queued and nothing of it was sent; the view then, or `None` when
    /// it was not changed.
    pub async fn replace_file(&self, id: &str, path: &Path) -> Result<Option<TransferView>> {
        let (meta, name) = upload::sendable(path).await?;
        let local = path.to_string_lossy();
        if !repo::set_file(&self.store, id, &local, &name, mime_for(&name), meta.len() as i64).await? {
            return Ok(None);
        }
        self.transfer(id).await
    }

    /// Register an upload. `message_id` is the placeholder row of the chat.
    pub async fn queue_upload(&self, path: &Path, chat_id: &str, message_id: &str) -> Result<TransferView> {
        let (meta, name) = upload::sendable(path).await?;
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

    /// Where an upload goes: back to the server of its earlier attempt
    /// while that one is enabled and can be written to, else the first
    /// usable one.
    async fn upload_target(&self, keys: &Keys, earlier: Option<&str>) -> Result<Arc<dyn BlobBackend>> {
        if let Some(base) = earlier {
            if let Ok(list) = self.usable_servers(keys).await {
                if let Some((_, b)) = list.into_iter().find(|(_, b)| b.public_base() == base) {
                    return Ok(b);
                }
            }
        }
        self.upload_backend(keys).await
    }

    /// The server whose public base is `base`, enabled or not, when it can
    /// be written to. Without keys only servers that need none (S3).
    async fn chunk_home(&self, keys: Option<&Keys>, base: &str) -> Option<Arc<dyn BlobBackend>> {
        if let Some(b) = &self.fixed_backend {
            return (b.public_base() == base).then(|| b.clone());
        }
        let row = repo::servers(&self.store).await.ok()?.into_iter().find(|r| public_base_of(r) == base)?;
        match keys {
            Some(k) => self.build(&row, k).await.ok(),
            None if row.kind == repo::KIND_S3 => self.build_s3(&row).await.ok(),
            None => None,
        }
    }

    /// Remove `chunks` from `home` in the background, and those of `again`
    /// once more a while later: a request dropped with the transfer may
    /// still land after the first removal. A failure leaves them where
    /// they are: encrypted, named by no message.
    fn remove_chunks(&self, home: Arc<dyn BlobBackend>, chunks: Vec<String>, again: Vec<String>) {
        let grace = self.cleanup_grace;
        let task = tokio::spawn(async move {
            for sha in &chunks {
                let _ = home.delete(sha).await;
            }
            if !again.is_empty() {
                tokio::time::sleep(grace).await;
                for sha in &again {
                    let _ = home.delete(sha).await;
                }
            }
        });
        let mut removals = self.removals.lock().unwrap();
        removals.retain(|t| !t.is_finished());
        removals.push(task);
    }

    /// Wait until the chunks being removed in the background are gone (as
    /// far as their servers let them), those removed once more a while
    /// later too. A process that ends sooner leaves the rest on the
    /// servers, named by nothing any more.
    pub async fn removals_done(&self) {
        loop {
            let tasks = std::mem::take(&mut *self.removals.lock().unwrap());
            if tasks.is_empty() {
                return;
            }
            for t in tasks {
                let _ = t.await;
            }
        }
    }

    /// The server at `base`: the one in use when it is that one, else
    /// looked up.
    async fn home_of(&self, keys: Option<&Keys>, used: Option<&Arc<dyn BlobBackend>>, base: &str) -> Option<Arc<dyn BlobBackend>> {
        match used.filter(|b| b.public_base() == base) {
            Some(b) => Some(b.clone()),
            None => self.chunk_home(keys, base).await,
        }
    }

    /// Remove what a cancelled upload left on servers, in the background:
    /// the chunks it stored, those whose request it dropped, and those it
    /// left behind on servers it no longer used.
    async fn clean_up_upload(&self, id: &str, keys: Option<&Keys>, used: Option<Arc<dyn BlobBackend>>) {
        let Ok(Some(row)) = repo::transfer(&self.store, id).await else { return };
        let Ok(mut state) = serde_json::from_str::<UploadState>(&row.state_json) else { return };
        // A state of an earlier version names no server: its chunks went to
        // the one uploads went to then. Names are the ciphertext of this
        // upload's own key, so removing them there touches nothing else.
        let base = match state.server_base.clone() {
            Some(b) => Some(b),
            None if !state.on_server().is_empty() => self.earlier_home(keys).await,
            None => None,
        };
        // What it holds on its server now goes too, after what it left
        // elsewhere.
        if let Some(base) = base {
            state.leave(&base);
        }
        // Nothing is left to resume from.
        let _ = repo::set_progress(&self.store, id, 0, Some("{}")).await;
        for s in state.stale {
            if s.chunks.is_empty() {
                continue;
            }
            if let Some(home) = self.home_of(keys, used.as_ref(), &s.base).await {
                self.remove_chunks(home, s.chunks, s.unconfirmed);
            }
        }
    }

    /// Where an upload of an earlier version, whose state names no server,
    /// sent its chunks: the first usable server, as uploads went then.
    /// Without keys only one that needs none can be told (S3).
    async fn earlier_home(&self, keys: Option<&Keys>) -> Option<String> {
        if let Some(b) = &self.fixed_backend {
            return Some(b.public_base());
        }
        if let Some(k) = keys {
            return self.upload_backend(k).await.ok().map(|b| b.public_base());
        }
        for r in repo::servers(&self.store).await.ok()?.into_iter().filter(|r| r.enabled) {
            if r.kind == repo::KIND_S3 && self.build_s3(&r).await.is_ok() {
                return Some(public_base_of(&r));
            }
        }
        None
    }

    /// After every pass of an upload: the chunks it left on servers it no
    /// longer uses go out of the state and are removed in the background,
    /// those whose request was dropped once more a while later.
    async fn settle_upload(&self, id: &str, keys: &Keys, used: &Arc<dyn BlobBackend>) {
        let Ok(Some(row)) = repo::transfer(&self.store, id).await else { return };
        let Ok(mut state) = serde_json::from_str::<UploadState>(&row.state_json) else { return };
        let stale = std::mem::take(&mut state.stale);
        if !stale.is_empty() {
            let json = serde_json::to_string(&state).unwrap_or_else(|_| "{}".into());
            let _ = repo::set_state(&self.store, id, &json).await;
        }
        for s in stale {
            // Never what the upload holds on its server now, whatever the
            // state says is left there.
            let held = if state.server_base.as_deref() == Some(s.base.as_str()) { state.on_server() } else { Vec::new() };
            let gone = |c: &String| !held.contains(c);
            let (chunks, again) = (s.chunks.into_iter().filter(gone).collect(), s.unconfirmed.into_iter().filter(gone).collect());
            if let Some(home) = self.home_of(Some(keys), Some(used), &s.base).await {
                self.remove_chunks(home, chunks, again);
            }
        }
    }

    /// Wait before automatic retry number `attempt`, `delay` long. A
    /// "retry now" ends the wait early; a pause or a cancel ends it and is
    /// returned.
    async fn wait_retry(
        &self,
        id: &str,
        delay: Duration,
        attempt: u32,
        e: &MessengerError,
        control: &Control,
        meter: &ProgressMeter<'_>,
    ) -> Result<Option<u8>> {
        let mut wakes = control.wakes();
        let reason = short_reason(e);
        repo::set_status(&self.store, id, repo::ST_WAITING_RETRY, Some(&reason)).await?;
        meter.retry(attempt, Some(now_ms() + delay.as_millis() as i64));
        meter.status(repo::ST_WAITING_RETRY, Some(&reason));
        tokio::select! {
            biased;
            stop = control.stopped() => Ok(Some(stop)),
            _ = wakes.changed() => Ok(None),
            _ = tokio::time::sleep(delay) => Ok(None),
        }
    }

    /// The wait before automatic retry number `attempt` is over: the
    /// attempt queues for a slot of its lane like any other transfer, and
    /// says so, not that it waits for a time gone by.
    async fn requeue(&self, id: &str, attempt: u32, meter: &ProgressMeter<'_>) -> Result<()> {
        repo::set_status(&self.store, id, repo::ST_QUEUED, None).await?;
        meter.retry(attempt, None);
        meter.live().set_stage(TransferStage::Queued);
        meter.status(repo::ST_QUEUED, None);
        Ok(())
    }

    /// Every pass of a run starts from the queue: the row is set running.
    /// One that left the queue meanwhile was taken where file locks cannot
    /// tell (a cancel or a pause in another process): the run stops as the
    /// row says, and never runs over it.
    async fn start_pass(&self, id: &str) -> Result<Option<u8>> {
        if repo::claim(&self.store, id, repo::ST_RUNNING, &[repo::ST_QUEUED]).await? {
            return Ok(None);
        }
        let paused = repo::transfer(&self.store, id).await?.is_some_and(|r| r.status == repo::ST_PAUSED);
        Ok(Some(if paused { PAUSE } else { CANCEL }))
    }

    /// Run (or resume) an upload to completion, pause, cancel or failure,
    /// with the automatic retries of the module. `Ok(Some(descriptor))`
    /// when the file is fully stored (the transfer is done), `Ok(None)`
    /// when it was paused or cancelled (also before it started).
    /// `err.transfer_in_progress` while a run of it is under way already,
    /// here or elsewhere. For a file whose message goes out after it, see
    /// `upload_to_publish`.
    pub async fn run_upload(
        &self,
        id: &str,
        keys: &Keys,
        caption: Option<String>,
        sink: &dyn ProgressSink,
    ) -> Result<Option<MediaDescriptor>> {
        let Some(p) = self.store_upload(id, keys, caption, sink, false).await? else { return Ok(None) };
        let d = p.descriptor.clone();
        Ok(p.published(None).await?.then_some(d))
    }

    /// `run_upload` for a file whose message goes out after it. Once the
    /// chunks are stored the transfer stays under way, in the publishing
    /// stage, and the caller ends it with `Publishing::published` or
    /// `Publishing::failed`.
    pub async fn upload_to_publish<'a>(
        &self,
        id: &str,
        keys: &Keys,
        caption: Option<String>,
        sink: &'a dyn ProgressSink,
    ) -> Result<Option<Publishing<'a>>> {
        self.store_upload(id, keys, caption, sink, true).await
    }

    async fn store_upload<'a>(
        &self,
        id: &str,
        keys: &Keys,
        caption: Option<String>,
        sink: &'a dyn ProgressSink,
        publish: bool,
    ) -> Result<Option<Publishing<'a>>> {
        let row = repo::transfer(&self.store, id).await?.ok_or_else(|| MessengerError::Invalid("unknown transfer".into()))?;
        if row.direction != repo::DIR_UP {
            return Err(MessengerError::Invalid("not an upload".into()));
        }
        let run = self.register(id).await?;
        let idle = run.idle(&[repo::ST_QUEUED, repo::ST_PAUSED, repo::ST_FAILED, repo::ST_DONE]);
        if !repo::claim(&self.store, id, repo::ST_QUEUED, &idle).await? {
            drop(run);
            return match repo::transfer(&self.store, id).await?.map(|r| r.status) {
                Some(s) if s == repo::ST_CANCELLED => Ok(None),
                _ => Err(in_progress()),
            };
        }
        run.enter().await?;
        let meter = self.meter(sink, &row);
        let mut used = None;
        let ending = tokio::select! {
            ending = self.upload_runs(&row, keys, caption, &run.control, &meter, &mut used) => {
                ending.unwrap_or_else(Ending::Failed)
            }
            never = self.beside(id, &run.control) => match never {},
        };
        #[cfg(test)]
        self.at_ending().await;
        // From here no word reaches the run: what came until now is heeded,
        // and a cancel that comes later waits for this end.
        let ending = ending.heed(run.control.close());
        // The run is gone from memory before its end is told: a resume
        // started on seeing it registers a run of its own.
        match ending {
            Ending::Done(d) => {
                repo::set_result(&self.store, id, None, Some(&d.sha256), None).await?;
                repo::set_progress(&self.store, id, d.size as i64, None).await?;
                // Still this run's: a cancel elsewhere (where file locks
                // cannot tell) may have taken it, and removed its chunks.
                if !repo::claim(&self.store, id, repo::ST_RUNNING, &[repo::ST_RUNNING]).await? {
                    drop(run);
                    meter.status(repo::ST_CANCELLED, None);
                    return Ok(None);
                }
                if publish {
                    meter.live().set_stage(TransferStage::Publishing);
                    meter.poll();
                }
                let shown = self.show_publishing(id);
                Ok(Some(Publishing { store: self.store.clone(), run, meter, descriptor: d, shown }))
            }
            Ending::Stopped(stop) => {
                // What a cancelled one left on servers is removed, under way
                // before the row says cancelled (`removals_done`).
                if stop == CANCEL {
                    self.clean_up_upload(id, Some(keys), used).await;
                }
                let reason = stop_reason(stop);
                repo::set_status(&self.store, id, stop_status(stop), reason).await?;
                drop(run);
                meter.status(stop_status(stop), reason);
                Ok(None)
            }
            Ending::Failed(e) => {
                let reason = short_reason(&e);
                repo::set_status(&self.store, id, repo::ST_FAILED, Some(&reason)).await?;
                drop(run);
                meter.status(repo::ST_FAILED, Some(&reason));
                Err(e)
            }
        }
    }

    async fn upload_runs(
        &self,
        row: &TransferRow,
        keys: &Keys,
        caption: Option<String>,
        control: &Control,
        meter: &ProgressMeter<'_>,
        used: &mut Option<Arc<dyn BlobBackend>>,
    ) -> Result<Ending<MediaDescriptor>> {
        let id = row.id.as_str();
        let path = PathBuf::from(row.local_path.clone().unwrap_or_default());
        repo::set_attempts(&self.store, id, row.attempts.max(0) + 1).await?;
        meter.status(repo::ST_QUEUED, None);
        let mut retries = 0u32;
        loop {
            let permit = match self.slot(row.size.max(0) as u64, control).await? {
                Ok(p) => p,
                Err(stop) => return Ok(Ending::Stopped(stop)),
            };
            let state = repo::transfer(&self.store, id).await?.map(|r| r.state_json).unwrap_or_default();
            let resume: Option<UploadState> = serde_json::from_str(&state).ok().filter(|s: &UploadState| !s.key.is_empty());
            let backend = match self.upload_target(keys, resume.as_ref().and_then(|s| s.server_base.as_deref())).await {
                Ok(b) => b,
                Err(e) => return Ok(Ending::Failed(e)),
            };
            *used = Some(backend.clone());
            let meta = tokio::fs::metadata(&path).await.ok();
            let checking = resume.as_ref().zip(meta.as_ref()).is_some_and(|(s, m)| s.will_check(m, &backend.public_base(), meter.live()));
            meter.live().set_stage(if checking { TransferStage::Checking } else { TransferStage::Uploading });
            if let Some(stop) = self.start_pass(id).await? {
                return Ok(Ending::Stopped(stop));
            }
            meter.status(repo::ST_RUNNING, None);

            // Progress is persisted from a channel so the workers never
            // block on the database; only the newest state is written, and
            // every state it covers is told kept (an upload waits for that
            // before a chunk goes).
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, String, oneshot::Sender<()>)>();
            let store = self.store.clone();
            let tid = id.to_string();
            let writer = tokio::spawn(async move {
                while let Some((mut done, mut json, ack)) = rx.recv().await {
                    let mut acks = vec![ack];
                    while let Ok(newer) = rx.try_recv() {
                        (done, json) = (newer.0, newer.1);
                        acks.push(newer.2);
                    }
                    if repo::set_progress(&store, &tid, done as i64, Some(&json)).await.is_ok() {
                        for a in acks {
                            let _ = a.send(());
                        }
                    }
                }
            });
            let on_chunk = move |done: u64, state: &UploadState| {
                let (ack, saved) = oneshot::channel();
                let _ = tx.send((done, serde_json::to_string(state).unwrap_or_else(|_| "{}".into()), ack));
                Some(saved)
            };
            let params = UploadParams { path: &path, caption: caption.clone(), batch: None, resume, chunk_size: self.chunk_size };
            let ctx = self.ctx(control, meter.live(), row.size.max(0) as u64);
            let outcome = tokio::select! {
                outcome = upload::upload(backend.as_ref(), params, &ctx, &on_chunk) => outcome,
                never = meter.run() => match never {},
            };
            drop(on_chunk);
            let _ = writer.await;
            meter.poll();
            drop(permit);
            self.settle_upload(id, keys, &backend).await;

            match outcome {
                Ok(UploadOutcome::Done(d)) => return Ok(Ending::Done(*d)),
                Ok(UploadOutcome::Paused) => return Ok(Ending::Stopped(PAUSE)),
                Ok(UploadOutcome::Cancelled) => return Ok(Ending::Stopped(CANCEL)),
                Err(e) if retryable(&e) && (retries as usize) < self.retry_delays.len() => {
                    let delay = self.retry_delays[retries as usize];
                    retries += 1;
                    if let Some(stop) = self.wait_retry(id, delay, retries, &e, control, meter).await? {
                        return Ok(Ending::Stopped(stop));
                    }
                    self.requeue(id, retries, meter).await?;
                }
                Err(e) => return Ok(Ending::Failed(e)),
            }
        }
    }

    /// Path of a received file when it is already in the cache.
    pub async fn cached(&self, d: &MediaDescriptor) -> Option<PathBuf> {
        download::cached(&self.cache_dir, d).await
    }

    /// Whether a download may start without the user asking for it. Never
    /// one the user paused: only one the closing of the app paused goes on
    /// when it is seen.
    pub async fn may_auto_download(&self, message_id: &str) -> Result<bool> {
        Ok(match repo::transfer_for_message(&self.store, message_id, repo::DIR_DOWN).await? {
            Some(t) => {
                let users_pause = t.status == repo::ST_PAUSED && t.failure_reason.as_deref() != Some(repo::REASON_INTERRUPTED);
                t.attempts >= 0
                    && t.attempts < MAX_AUTO_ATTEMPTS
                    && !UNDER_WAY.contains(&t.status.as_str())
                    && !users_pause
            }
            None => true,
        })
    }

    /// Download the file of a message into the cache (or return it from
    /// there). `manual` resets the automatic-attempt bookkeeping.
    ///
    /// Two kinds of retries meet here. Within one call, a failure that may
    /// pass is retried by itself after a wait (`waiting_retry`), up to
    /// three times. `attempts` of the row counts calls that ended failed,
    /// each after its own retries: `may_auto_download` refuses once it
    /// reaches `MAX_AUTO_ATTEMPTS`, a manual call starts it again from
    /// zero, and a cancel sets it to -1 (never again by itself).
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
        let (id, new_row) = match existing {
            // One under way is refused by the claim below, unless its run
            // is gone.
            Some(t) => (t.id, false),
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
                (row.id, true)
            }
        };
        let run = self.register(&id).await?;
        let row = repo::transfer(&self.store, &id).await?.ok_or_else(|| MessengerError::Storage("transfer vanished".into()))?;
        // A new row is claimed too: a cancel may have come since it was made.
        // Paused, it was taken for one left under way (`recover` in another
        // process that started meanwhile).
        let idle = if new_row {
            vec![repo::ST_QUEUED, repo::ST_PAUSED]
        } else {
            run.idle(&[repo::ST_PAUSED, repo::ST_FAILED, repo::ST_CANCELLED, repo::ST_DONE])
        };
        if !repo::claim(&self.store, &id, repo::ST_QUEUED, &idle).await? {
            drop(run);
            return match repo::transfer(&self.store, &id).await?.map(|r| r.status) {
                Some(s) if new_row && s == repo::ST_CANCELLED => Ok(None),
                _ => Err(in_progress()),
            };
        }
        run.enter().await?;
        if manual {
            repo::set_attempts(&self.store, &id, 0).await?;
        }
        // Who started the run is kept with how far it got: one the user
        // started goes on by itself after the app was closed.
        let mut kept: DownloadState = serde_json::from_str(&row.state_json).unwrap_or_default();
        if kept.manual != manual {
            kept.manual = manual;
            repo::set_state(&self.store, &id, &serde_json::to_string(&kept).unwrap_or_else(|_| "{}".into())).await?;
        }
        let meter = self.meter(sink, &row);
        meter.live().start(d.chunks.len() as u32, d.chunk_size);
        let ending = tokio::select! {
            ending = self.download_runs(&id, d, manual, &run.control, &meter) => ending.unwrap_or_else(Ending::Failed),
            never = self.beside(&id, &run.control) => match never {},
        };
        #[cfg(test)]
        self.at_ending().await;
        // See `store_upload`. A file that is in the cache stays there.
        let last = run.control.close();
        let ending = match ending {
            Ending::Done(path) => Ending::Done(path),
            ending => ending.heed(last),
        };
        match ending {
            Ending::Done(path) => {
                let n = d.chunks.len() as u32;
                let state = DownloadState { chunks_done: n, chunks_total: n, chunk_size: d.chunk_size, folder: None, manual };
                let json = serde_json::to_string(&state).unwrap_or_else(|_| "{}".into());
                repo::set_result(&self.store, &id, Some(&path.to_string_lossy()), None, None).await?;
                repo::set_progress(&self.store, &id, d.size as i64, Some(&json)).await?;
                repo::set_status(&self.store, &id, repo::ST_DONE, None).await?;
                drop(run);
                meter.done(Some(path.to_string_lossy().into_owned()));
                Ok(Some(path))
            }
            Ending::Stopped(stop) => {
                // A cancelled one is never started again by itself, and its
                // chunks on disk go.
                if stop == CANCEL {
                    repo::set_attempts(&self.store, &id, -1).await?;
                    download::discard(&self.cache_dir, d).await;
                    repo::set_progress(&self.store, &id, 0, Some("{}")).await?;
                }
                let reason = stop_reason(stop);
                repo::set_status(&self.store, &id, stop_status(stop), reason).await?;
                drop(run);
                meter.status(stop_status(stop), reason);
                Ok(None)
            }
            Ending::Failed(e) => {
                let base = if manual { 0 } else { row.attempts.max(0) };
                let reason = short_reason(&e);
                repo::set_attempts(&self.store, &id, base + 1).await?;
                // Nothing is left on disk to go on from (a file whose hash
                // did not match is fetched anew): the row and the bar say so.
                if !tokio::fs::try_exists(download::tmp_dir(&self.cache_dir, d)).await.unwrap_or(false) {
                    repo::set_progress(&self.store, &id, 0, Some("{}")).await?;
                    meter.live().start_over(d.chunks.len() as u32, d.chunk_size);
                }
                repo::set_status(&self.store, &id, repo::ST_FAILED, Some(&reason)).await?;
                drop(run);
                meter.status(repo::ST_FAILED, Some(&reason));
                Err(e)
            }
        }
    }

    async fn download_runs(
        &self,
        id: &str,
        d: &MediaDescriptor,
        manual: bool,
        control: &Control,
        meter: &ProgressMeter<'_>,
    ) -> Result<Ending<PathBuf>> {
        meter.status(repo::ST_QUEUED, None);
        let mirrors = self.own_bases().await;
        let (chunks_total, chunk_size) = (d.chunks.len() as u32, d.chunk_size);
        let folder = download::tmp_name(d);
        let mut retries = 0u32;
        loop {
            let permit = match self.slot(d.size, control).await? {
                Ok(p) => p,
                Err(stop) => return Ok(Ending::Stopped(stop)),
            };
            let checking = download::will_check(&self.cache_dir, d, meter.live()).await;
            meter.live().set_stage(if checking { TransferStage::Checking } else { TransferStage::Downloading });
            if let Some(stop) = self.start_pass(id).await? {
                return Ok(Ending::Stopped(stop));
            }
            meter.status(repo::ST_RUNNING, None);

            // How far it got goes to the row too, so a view after a restart
            // can say "chunk 10 of 200".
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(u64, u32)>();
            let store = self.store.clone();
            let tid = id.to_string();
            let folder = folder.clone();
            let writer = tokio::spawn(async move {
                while let Some(mut latest) = rx.recv().await {
                    while let Ok(newer) = rx.try_recv() {
                        latest = newer;
                    }
                    let state =
                        DownloadState { chunks_done: latest.1, chunks_total, chunk_size, folder: Some(folder.clone()), manual };
                    let json = serde_json::to_string(&state).unwrap_or_else(|_| "{}".into());
                    let _ = repo::set_progress(&store, &tid, latest.0 as i64, Some(&json)).await;
                }
            });
            let on_chunk = move |done: u64, chunks: u32| {
                let _ = tx.send((done, chunks));
            };
            let ctx = self.ctx(control, meter.live(), d.size);
            let outcome = tokio::select! {
                outcome = download::download(self.fetcher.as_ref(), d, &mirrors, &self.cache_dir, &ctx, &on_chunk) => outcome,
                never = meter.run() => match never {},
            };
            drop(on_chunk);
            let _ = writer.await;
            meter.poll();
            drop(permit);

            match outcome {
                Ok(DownloadOutcome::Done(path)) => return Ok(Ending::Done(path)),
                Ok(DownloadOutcome::Paused) => return Ok(Ending::Stopped(PAUSE)),
                Ok(DownloadOutcome::Cancelled) => return Ok(Ending::Stopped(CANCEL)),
                Err(e) if retryable(&e) && (retries as usize) < self.retry_delays.len() => {
                    let delay = self.retry_delays[retries as usize];
                    retries += 1;
                    if let Some(stop) = self.wait_retry(id, delay, retries, &e, control, meter).await? {
                        return Ok(Ending::Stopped(stop));
                    }
                    self.requeue(id, retries, meter).await?;
                }
                Err(e) => return Ok(Ending::Failed(e)),
            }
        }
    }

    /// Ask a transfer to stop, where it runs: here, or in another process
    /// with this data folder. Chunks in flight are dropped at once;
    /// finished ones are kept for the resume. A transfer under way whose
    /// run is gone is paused here (`Paused::Here`).
    pub async fn pause(&self, id: &str) -> Result<Paused> {
        let control = self.controls.lock().unwrap().get(id).cloned();
        let asked = match control {
            Some(c) => c.ask(PAUSE),
            None => match runs::held(&self.cache_dir, id) {
                Some(true) => !self.publishing_elsewhere(id) && runs::ask(&self.cache_dir, id, Word::Pause),
                // No run anywhere. Held while the row is paused: no run
                // starts on it meanwhile.
                Some(false) => {
                    let Ok(Some(_lock)) = runs::take(&self.cache_dir, id).await else { return Ok(Paused::Nothing) };
                    let paused = repo::claim(&self.store, id, repo::ST_PAUSED, &UNDER_WAY).await?;
                    return Ok(if paused { Paused::Here } else { Paused::Nothing });
                }
                None => false,
            },
        };
        Ok(if asked { Paused::ByRun } else { Paused::Nothing })
    }

    /// Does a run of the transfer live now, here or in another process with
    /// this data folder? Taken for one where that cannot be told. A row
    /// under way with no run is one a resume takes over.
    pub fn run_alive(&self, id: &str) -> bool {
        self.controls.lock().unwrap().contains_key(id) || runs::held(&self.cache_dir, id) != Some(false)
    }

    /// Start the next attempt of a transfer that waits for one now, where
    /// it runs. Whether a run was asked.
    pub fn retry_now(&self, id: &str) -> bool {
        if let Some(c) = self.controls.lock().unwrap().get(id) {
            c.wake();
            return true;
        }
        runs::ask(&self.cache_dir, id, Word::Retry)
    }

    /// Cancel a transfer, running or not; see `cancel_with`.
    pub async fn cancel(&self, id: &str) -> Result<()> {
        self.cancel_with(id, None).await.map(|_| ())
    }

    /// Cancel a transfer, running or not.
    ///
    /// A run is told to stop and finishes the cancel itself, here or in
    /// another process with this data folder (`Cancelled::ByRun`). A run
    /// that is deciding its end already (here, or publishing its message
    /// anywhere) is waited for, and the cancel acts on how it ended. A
    /// transfer nothing runs is marked cancelled here (`Cancelled::Here`): the
    /// chunks a cancelled upload left are removed from its servers in the
    /// background, without `keys` only from a server that needs none for
    /// it (S3), and those a download kept on disk go. An upload whose
    /// message went out is not cancelled (`Cancelled::Nothing`); see
    /// `discard_unpublished` for one whose message never did.
    /// `err.transfer_elsewhere` when a run in another process does not
    /// heed the cancel.
    pub async fn cancel_with(&self, id: &str, keys: Option<&Keys>) -> Result<Cancelled> {
        let settle_by = tokio::time::Instant::now() + SETTLE;
        // When a run in another process was asked, and until when it may take.
        let mut asked: Option<tokio::time::Instant> = None;
        loop {
            let control = self.controls.lock().unwrap().get(id).cloned();
            if let Some(c) = control {
                if c.ask(CANCEL) {
                    return Ok(Cancelled::ByRun);
                }
                // Deciding its end: its row settles in a moment.
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
            let Some(t) = repo::transfer(&self.store, id).await? else { return Ok(Cancelled::Nothing) };
            let up = t.direction == repo::DIR_UP;
            if t.status == repo::ST_CANCELLED {
                return Ok(if asked.is_some() { Cancelled::ByRun } else { Cancelled::Nothing });
            }
            // A finished upload is in a message: its chunks stay.
            if up && t.status == repo::ST_DONE {
                return Ok(Cancelled::Nothing);
            }
            let idle: &[&str] = if up { &[repo::ST_PAUSED, repo::ST_FAILED] } else { &[repo::ST_PAUSED, repo::ST_FAILED, repo::ST_DONE] };
            let (from, _lock) = if idle.contains(&t.status.as_str()) {
                (idle, None)
            } else {
                match runs::held(&self.cache_dir, id) {
                    // A run in another process that publishes its message
                    // is waited for, as one here deciding its end is.
                    Some(true) if self.publishing_elsewhere(id) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        continue;
                    }
                    // A run in another process: it is asked, and finishes
                    // the cancel itself; never its chunks from under it.
                    Some(true) => {
                        let by = *asked.get_or_insert_with(|| {
                            runs::ask(&self.cache_dir, id, Word::Cancel);
                            tokio::time::Instant::now() + self.ask_wait
                        });
                        if tokio::time::Instant::now() >= by {
                            return Err(elsewhere());
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        continue;
                    }
                    // No run anywhere: the one that was ended without a word.
                    // Held while the row is taken and what it left goes: no
                    // run starts on it meanwhile. One that took it first is
                    // asked next time round.
                    Some(false) => match runs::take(&self.cache_dir, id).await {
                        Ok(lock) => (&UNDER_WAY[..], lock),
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            continue;
                        }
                    },
                    // No file locks: a row under way with no run here
                    // settles within moments, or it runs elsewhere.
                    None if tokio::time::Instant::now() < settle_by => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        continue;
                    }
                    None if self.others.load(Ordering::SeqCst) => return Err(elsewhere()),
                    None => (&UNDER_WAY[..], None),
                }
            };
            // Claimed, nothing can start it any more; else it changed meanwhile.
            if repo::claim(&self.store, id, repo::ST_CANCELLED, from).await? {
                if up {
                    self.clean_up_upload(id, keys, None).await;
                } else {
                    repo::set_attempts(&self.store, id, -1).await?;
                    let kept = serde_json::from_str::<DownloadState>(&t.state_json).ok().and_then(|s| s.folder);
                    if let Some(folder) = kept {
                        download::discard_folder(&self.cache_dir, &folder).await;
                    }
                    repo::set_progress(&self.store, id, 0, Some("{}")).await?;
                }
                return Ok(Cancelled::Here);
            }
        }
    }

    /// An upload whose message went out while its row still names
    /// `placeholder`, the message it took the place of (its process ended
    /// before the row was told): the transfer is done, and its chunks stay.
    /// Only while nothing runs it and the row still names the placeholder.
    /// Whether it was marked.
    pub async fn mark_published(&self, id: &str, placeholder: &str) -> Result<bool> {
        let Ok(run) = self.register(id).await else { return Ok(false) };
        let from = run.idle(&[repo::ST_PAUSED, repo::ST_FAILED]);
        repo::claim_for_message(&self.store, id, repo::ST_DONE, &from, placeholder).await
    }

    /// Cancel an upload that is stored but whose message never went out:
    /// its row still names `placeholder`, the message the caller read as
    /// one. Its chunks go like those of any cancelled upload. One whose
    /// message went out meanwhile (a run published it) is not cancelled,
    /// and anything else is cancelled as `cancel_with` does.
    pub async fn discard_unpublished(&self, id: &str, placeholder: &str, keys: Option<&Keys>) -> Result<Cancelled> {
        let done_up = repo::transfer(&self.store, id).await?.is_some_and(|t| t.direction == repo::DIR_UP && t.status == repo::ST_DONE);
        if done_up && repo::claim_for_message(&self.store, id, repo::ST_CANCELLED, &[repo::ST_DONE], placeholder).await? {
            self.clean_up_upload(id, keys, None).await;
            return Ok(Cancelled::Here);
        }
        self.cancel_with(id, keys).await
    }
}

/// What a start-up paused; see `MediaService::recover`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovered {
    /// Alone with the data folder: every transfer under way (how many).
    Alone(u64),
    /// Another process has it open: only those whose run is gone. The
    /// placeholders of the uploads left under way, whose run may be its.
    Beside(Vec<String>),
}

/// What a cancel did; see `MediaService::cancel_with`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cancelled {
    /// A run was told to stop and finishes the cancel itself, placeholder
    /// and all.
    ByRun,
    /// Nothing ran it: it is marked cancelled now, and what it left is
    /// being removed. Its placeholder is the caller's to discard.
    Here,
    /// Nothing to cancel: cancelled before, unknown, or an upload whose
    /// message went out.
    Nothing,
}

/// What a pause did; see `MediaService::pause`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paused {
    /// A run was asked, and pauses itself.
    ByRun,
    /// Nothing ran it: it is marked paused now. Its placeholder is the
    /// caller's to tell.
    Here,
    /// Nothing to pause: not under way, or a run that hears no word now
    /// (it is ending, or publishing its message).
    Nothing,
}

/// An upload whose chunks are all stored, in the publishing stage: the
/// message that names them goes out now. The transfer stays under way
/// until `published` or `failed`, and a cancel meanwhile waits for that
/// end. Dropped without either, its row says running with nothing to run
/// it, and a cancel marks it.
pub struct Publishing<'a> {
    store: Store,
    run: Run,
    meter: ProgressMeter<'a>,
    descriptor: MediaDescriptor,
    shown: Showing,
}

/// Shows the view of a run to other processes until it is dropped; see
/// `MediaService::show_publishing`.
struct Showing(tokio::task::JoinHandle<()>);

impl Drop for Showing {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Publishing<'_> {
    pub fn descriptor(&self) -> &MediaDescriptor {
        &self.descriptor
    }

    /// The message went out as `message_id` (it takes the place of the
    /// placeholder in the row, as the row is marked done; `None` keeps what
    /// is there): the transfer is done. Whether it is: a cancel that took
    /// the row meanwhile (where file locks cannot tell) leaves it
    /// cancelled, naming the placeholder still.
    pub async fn published(self, message_id: Option<&str>) -> Result<bool> {
        let id = self.run.id.clone();
        let done = match message_id {
            Some(m) => repo::claim_naming(&self.store, &id, repo::ST_DONE, &[repo::ST_RUNNING], m).await?,
            None => repo::claim(&self.store, &id, repo::ST_DONE, &[repo::ST_RUNNING]).await?,
        };
        let Publishing { run, meter, shown, .. } = self;
        drop(shown);
        drop(run);
        if done {
            meter.done(None);
        } else {
            meter.status(repo::ST_CANCELLED, None);
        }
        Ok(done)
    }

    /// The message could not go out: the transfer failed, with the reason
    /// of `e`. Its chunks stay; a retry finds them on the server.
    pub async fn failed(self, e: &MessengerError) -> Result<()> {
        let reason = short_reason(e);
        repo::set_status(&self.store, &self.run.id, repo::ST_FAILED, Some(&reason)).await?;
        let Publishing { run, meter, shown, .. } = self;
        drop(shown);
        drop(run);
        meter.status(repo::ST_FAILED, Some(&reason));
        Ok(())
    }
}

/// How one run of a transfer ended.
enum Ending<T> {
    Done(T),
    /// Paused or cancelled by the user.
    Stopped(u8),
    Failed(MessengerError),
}

impl<T> Ending<T> {
    /// The end, heeding the last word of the user (`Control::close`): a
    /// cancel that came while the run was ending cancels it, whether it
    /// was stored by then, had failed or was pausing. A run that paused
    /// says whose pause it was by that word too (a pass tells only that it
    /// paused).
    fn heed(self, last: u8) -> Self {
        match self {
            _ if last == CANCEL => Ending::Stopped(CANCEL),
            Ending::Stopped(stop) if pauses(stop) && pauses(last) => Ending::Stopped(last),
            ending => ending,
        }
    }
}

/// Wait while a run of `id` here decides its end; refused while one runs
/// here. Then `control`, when there is one, becomes that of the run of `id`.
async fn settle_here(controls: &Mutex<HashMap<String, Control>>, id: &str, control: Option<&Control>) -> Result<()> {
    loop {
        {
            let mut controls = controls.lock().unwrap();
            match controls.get(id) {
                Some(c) if !c.is_closed() => return Err(in_progress()),
                Some(_) => {}
                None => {
                    if let Some(c) = control {
                        controls.insert(id.to_string(), c.clone());
                    }
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn in_progress() -> MessengerError {
    MessengerError::Invalid("err.transfer_in_progress".into())
}

fn elsewhere() -> MessengerError {
    MessengerError::Invalid("err.transfer_elsewhere".into())
}

/// The run of a transfer in this process. While it lives it holds the
/// lock of the run in all processes, and once it has entered the service
/// knows its control and its view; dropped (ended, or its future given
/// up) it lets them go, never those of a newer run.
struct Run {
    controls: Arc<Mutex<HashMap<String, Control>>>,
    views: Arc<Mutex<HashMap<String, Progress>>>,
    id: String,
    control: Control,
    lock: Option<RunLock>,
}

impl Run {
    /// The statuses it may claim its row from: `idle`, and those of a
    /// transfer under way once it holds the lock of the run. Then nothing
    /// runs it anywhere, and such a row was left so by a run that is gone
    /// (its process killed while another one had the data folder open, so
    /// nothing paused it at start; or its future given up).
    fn idle<'s>(&self, idle: &[&'s str]) -> Vec<&'s str> {
        let mut from = idle.to_vec();
        if self.lock.is_some() {
            from.extend(UNDER_WAY.iter().copied().filter(|s| !idle.contains(s)));
        }
        from
    }

    /// The run has claimed its row: from now on its control hears the
    /// words of the user. Refused while another run of it is here (where
    /// no file lock keeps it out); one deciding its end is waited for.
    async fn enter(&self) -> Result<()> {
        settle_here(&self.controls, &self.id, Some(&self.control)).await
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        // The lock first: whoever sees it free finds the row settled, and
        // a cancel here meanwhile still finds the control and waits.
        self.lock.take();
        let mut controls = self.controls.lock().unwrap_or_else(|e| e.into_inner());
        if controls.get(&self.id).is_some_and(|c| c.same(&self.control)) {
            controls.remove(&self.id);
            self.views.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use crate::descriptor::MAX_SEND_BYTES;
    use crate::control::RUN;
    use crate::upload::Unconfirmed;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;
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

    /// Reads from the memory backend; can lose or corrupt blobs, be slow,
    /// and counts the reads under way.
    struct MemFetcher {
        backend: MemoryBackend,
        corrupt: Mutex<Option<String>>,
        calls: Mutex<u32>,
        /// Fail this many fetches as the network would, then recover.
        fail: Mutex<u32>,
        /// The line drops this chunk as many times as said (both tries of
        /// a run), whatever the time.
        outage: Mutex<Option<(u32, String)>>,
        /// Every fetch takes this long.
        delay: Mutex<Duration>,
        /// While set, every fetch waits for a permit of it: a line that
        /// hangs, opened a fetch at a time by adding permits.
        gate: Mutex<Option<Arc<Semaphore>>>,
        /// Fetches under way now, and the most there ever were at once.
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
    }

    impl MemFetcher {
        fn new(backend: MemoryBackend) -> Self {
            Self {
                backend,
                corrupt: Mutex::new(None),
                calls: Mutex::new(0),
                fail: Mutex::new(0),
                outage: Mutex::new(None),
                delay: Mutex::new(Duration::ZERO),
                gate: Mutex::new(None),
                in_flight: Arc::default(),
                max_in_flight: Arc::default(),
            }
        }
    }

    /// Counts a fetch as under way for as long as it lives.
    struct Underway(Arc<AtomicUsize>);
    impl Drop for Underway {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl BlobFetcher for MemFetcher {
        async fn fetch(&self, url: &str, _max: u64) -> Result<Option<Vec<u8>>> {
            *self.calls.lock().unwrap() += 1;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let _underway = Underway(self.in_flight.clone());
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            let delay = *self.delay.lock().unwrap();
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                if let Ok(p) = gate.acquire().await {
                    p.forget();
                }
            }
            let down = || Err(MessengerError::Transport("err.network: connection reset".into()));
            {
                let mut fail = self.fail.lock().unwrap();
                if *fail > 0 {
                    *fail -= 1;
                    return down();
                }
            }
            let sha = url.rsplit('/').next().unwrap_or("");
            if let Some((left, dropped)) = self.outage.lock().unwrap().as_mut() {
                if dropped == sha && *left > 0 {
                    *left -= 1;
                    return down();
                }
            }
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
        /// The statuses in order, a repeat (another stage, more bytes) told once.
        fn statuses(&self) -> Vec<String> {
            let mut out: Vec<String> = self.0.lock().unwrap().iter().map(|p| p.status.clone()).collect();
            out.dedup();
            out
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
        let fetcher = Arc::new(MemFetcher::new(backend.clone()));
        let svc = MediaService::new(store, Arc::new(Secrets(Mutex::default())), dir.path())
            .unwrap()
            .with_backend(Arc::new(backend.clone()), fetcher.clone())
            .with_retry_delays(vec![Duration::ZERO; 3]);
        let mut svc = svc;
        svc.cleanup_grace = Duration::from_millis(300);
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
        assert_eq!(sink.statuses(), vec!["queued", "running", "done"]);
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
        let fetch = Arc::new(MemFetcher::new(MemoryBackend { base: "https://mem.example/a".into(), ..r.backend.clone() }));
        (r.svc.clone().with_backend(Arc::new(r.backend.clone()), fetch.clone()), dd, fetch)
    }

    async fn state_of(r: &Rig, id: &str) -> UploadState {
        serde_json::from_str(&repo::transfer(&r.svc.store, id).await.unwrap().unwrap().state_json).unwrap_or_default()
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
        // Three automatic retries, then it has failed.
        let retry = ["waiting_retry", "queued", "running"];
        let waiting = [&["queued", "running"][..], &retry, &retry, &retry, &["failed"]].concat();
        assert_eq!(sink.statuses(), waiting);
        let row = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!(row.failure_reason.as_deref(), Some("err.network"));
        assert_eq!(r.backend.len(), 2, "two of four chunks made it");
        let key_before = state_of(&r, &t.id).await;
        assert_eq!(key_before.chunks_done(), 2);
        let stored: u64 = key_before
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_some())
            .map(|(i, _)| (13 * MIB as u64 - i as u64 * 4 * MIB as u64).min(4 * MIB as u64))
            .sum();
        assert_eq!(row.done_bytes, stored, "the plaintext of the stored chunks");
        assert_eq!((row.chunks_done, row.chunks_total), (2, 4));

        // The network is back: only the missing chunks are sent, with the same key.
        *r.backend.die_after_puts.lock().unwrap() = None;
        let puts_before = *r.backend.put_calls.lock().unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*r.backend.put_calls.lock().unwrap() - puts_before, 2, "finished chunks are not sent again");
        assert_eq!(d.chunks.len(), 4);
        for (i, c) in key_before.chunks.iter().enumerate() {
            if let Some(c) = c {
                assert_eq!(&d.chunks[i], c, "same key, same ciphertext");
            }
        }
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
        let old = state_of(&r, &t.id).await;

        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().done_bytes, 4 * MIB as u64);

        tokio::fs::write(&path, content(5 * MIB + 1)).await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = None;
        let sink = crate::progress::tests::Timed::default();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_ne!(d.key, old.key, "never reuse a key for different content");
        assert_eq!(d.size as usize, 5 * MIB + 1);
        // Nothing of the new file was stored: the bar starts from 0, not
        // from what the old one got.
        let events = sink.0.lock().unwrap().clone();
        assert_eq!(stages(&events), [TransferStage::Queued, TransferStage::Uploading], "nothing to check of another file");
        let sending = events.iter().find(|(_, p)| p.stage == TransferStage::Uploading).unwrap();
        assert_eq!((sending.1.done_bytes, sending.1.chunks_done), (0, 0));
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
        assert_eq!(sink.statuses(), vec!["queued", "running", "failed"], "a bad chunk is not retried by itself");
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        assert!(!tmp.join("2").exists());
        // Chunks fetched side by side: those done before the failure stay.
        let kept = (0..2).filter(|i| tmp.join(i.to_string()).exists()).count();
        let view = svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().unwrap();
        assert_eq!(view.failure_reason.as_deref(), Some("err.chunk_hash_mismatch"));
        assert_eq!((view.chunks_total, view.chunk_size), (3, dd.chunk_size), "how far it got is in the row");
        assert_eq!(view.chunks_done as usize, kept, "every chunk on disk, and only those");
        assert_eq!(view.done_bytes, kept as u64 * 4 * MIB as u64);
        assert!(svc.may_auto_download("m").await.unwrap(), "one failure does not stop automatic retries");

        // Healthy again: only the missing chunks are fetched.
        *fetch.corrupt.lock().unwrap() = None;
        let before = *fetch.calls.lock().unwrap();
        let out = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*fetch.calls.lock().unwrap() - before, 3 - kept as u32);
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(9 * MIB));

        // A descriptor with the wrong file hash: chunks are fine, the file is refused.
        let mut lying = dd.clone();
        lying.sha256 = "0".repeat(64);
        let sink = Sink::default();
        let err = svc.run_download("m2", "chat", &lying, false, &sink).await.unwrap_err();
        assert!(err.to_string().contains("err.file_hash_mismatch"), "{err}");
        assert!(svc.cached(&lying).await.is_none());
        // Nothing is kept to go on from: neither the row nor the last event
        // says the file is all here.
        let view = svc.transfer_for_message("m2", repo::DIR_DOWN).await.unwrap().unwrap();
        assert_eq!((view.done_bytes, view.chunks_done), (0, 0));
        let failed = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!((failed.status.as_str(), failed.done_bytes), ("failed", 0));
        let again = Sink::default();
        svc.run_download("m2", "chat", &lying, true, &again).await.unwrap_err();
        let first = again.0.lock().unwrap()[0].clone();
        assert_eq!((first.status.as_str(), first.done_bytes), ("queued", 0), "a re-fetch starts from 0, not from all of it");

        // A wrong key: authentication fails, nothing reaches the cache.
        let mut wrong = dd.clone();
        wrong.sha256 = "1".repeat(64);
        wrong.set_key(&crate::crypto::FileKey::generate().unwrap());
        assert!(svc.run_download("m3", "chat", &wrong, false, &Sink::default()).await.is_err());
        assert!(svc.cached(&wrong).await.is_none());
        let view = svc.transfer_for_message("m3", repo::DIR_DOWN).await.unwrap().unwrap();
        assert_eq!(view.failure_reason.as_deref(), Some("err.chunk_auth_failed"), "a code, never the text");
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

        // The user's pause is never lifted by itself; one the closing of
        // the app made goes on when the file is seen.
        let tid = svc.transfer_for_message("lost", repo::DIR_DOWN).await.unwrap().unwrap().id;
        repo::set_status(&svc.store, &tid, repo::ST_PAUSED, None).await.unwrap();
        assert!(!svc.may_auto_download("lost").await.unwrap(), "paused by the user");
        repo::set_status(&svc.store, &tid, repo::ST_PAUSED, Some(repo::REASON_INTERRUPTED)).await.unwrap();
        assert!(svc.may_auto_download("lost").await.unwrap(), "paused by the closing of the app");

        // Cancelling means "do not start again by yourself".
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
                    svc.pause(&id).await.unwrap();
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

    // ─── Chunks side by side, control, progress, retries ────────────────────

    const CHUNK: usize = 64 * 1024;

    /// A rig whose uploads use small chunks: many of them in a small file.
    async fn small_rig() -> Rig {
        let mut r = rig().await;
        r.svc.chunk_size = Some(CHUNK as u64);
        r
    }

    /// Wait (up to ten seconds) until `check` holds.
    async fn until(mut check: impl AsyncFnMut() -> bool) {
        for _ in 0..2000 {
            if check().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the condition never held");
    }

    fn upload_job(r: &Rig, id: &str) -> tokio::task::JoinHandle<Result<Option<MediaDescriptor>>> {
        let (svc, keys, id) = (r.svc.clone(), r.keys.clone(), id.to_string());
        tokio::spawn(async move { svc.run_upload(&id, &keys, None, &Sink::default()).await })
    }

    /// Let `n` puts through, then hang every other one.
    fn hang_after(r: &Rig, n: usize) {
        *r.backend.put_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(n)));
    }

    fn in_flight(r: &Rig) -> usize {
        r.backend.puts_in_flight.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn cancel_drops_hanging_puts_at_once_and_removes_what_was_stored() {
        let r = small_rig().await;
        let path = file(&r, "hang.bin", 40 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 3);
        let job = upload_job(&r, &t.id);
        until(async || { r.backend.len() == 3 && in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3 }).await;

        let started = tokio::time::Instant::now();
        r.svc.cancel(&t.id).await.unwrap();
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        assert_eq!(in_flight(&r), 0, "the requests in flight were dropped");
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        // The stored chunks go from the server in the background, and so do
        // the four whose request was dropped, once more a while later.
        until(async || { r.backend.is_empty() }).await;
        until(async || { *r.backend.delete_calls.lock().unwrap() == 3 + 4 + 4 }).await;
    }

    #[tokio::test]
    async fn cancel_of_a_paused_upload_removes_its_chunks() {
        let r = small_rig().await;
        let path = file(&r, "p.bin", 10 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 4);
        let job = upload_job(&r, &t.id);
        until(async || { state_of(&r, &t.id).await.chunks_done() == 4 }).await;
        r.svc.pause(&t.id).await.unwrap();
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert_eq!(r.backend.len(), 4);

        r.svc.cancel(&t.id).await.unwrap();
        until(async || { r.backend.is_empty() }).await;
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
    }

    /// Once the row of a cancelled upload says so, the removal of its
    /// chunks is under way and can be waited for (a tool that ends then),
    /// the second pass too.
    #[tokio::test]
    async fn the_removal_of_a_cancelled_upload_can_be_waited_for() {
        let r = small_rig().await;
        let path = file(&r, "gone.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 3);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3 }).await;
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::ByRun);
        until(async || { r.svc.transfer(&t.id).await.unwrap().unwrap().status == "cancelled" }).await;
        r.svc.removals_done().await;
        assert!(r.backend.is_empty());
        assert_eq!(*r.backend.delete_calls.lock().unwrap(), 3 + 4 + 4, "the dropped ones once more");
        assert_eq!(job.await.unwrap().unwrap(), None);

        // One nothing runs: removed by the cancel itself.
        *r.backend.put_gate.lock().unwrap() = None;
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(2);
        r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap_err();
        assert_eq!(r.backend.len(), 2);
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::Here);
        r.svc.removals_done().await;
        assert!(r.backend.is_empty());
    }

    #[tokio::test]
    async fn pause_mid_way_and_resume_without_sending_a_chunk_twice() {
        let r = small_rig().await;
        let path = file(&r, "movie.mp4", 40 * CHUNK + 99).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 5);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 5 }).await;
        let started = tokio::time::Instant::now();
        r.svc.pause(&t.id).await.unwrap();
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(2), "a pause does not wait for the chunk");
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!(view.status, "paused");
        assert_eq!((view.chunks_done, view.chunks_total, view.chunk_size), (5, 41, CHUNK as u64), "chunk 5 of 41, from the row");
        assert_eq!(view.done_bytes, 5 * CHUNK as u64);

        *r.backend.put_gate.lock().unwrap() = None;
        let puts = *r.backend.put_calls.lock().unwrap();
        let heads = r.backend.exists_calls.load(Ordering::SeqCst);
        let sink = Sink::default();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(*r.backend.put_calls.lock().unwrap() - puts, 36, "only what was missing");
        assert_eq!(r.backend.exists_calls.load(Ordering::SeqCst) - heads, 5, "only the chunks stored before are asked about");
        assert!(sink.0.lock().unwrap().iter().any(|p| p.stage == TransferStage::Checking), "stored chunks are looked up first");
        let (svc, dd, _) = https(&r, &d);
        let out = svc.run_download("m", "chat", &dd, true, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(40 * CHUNK + 99));
    }

    /// Stages in order without repeats.
    fn stages(events: &[(tokio::time::Instant, Progress)]) -> Vec<TransferStage> {
        let mut out: Vec<TransferStage> = events.iter().map(|(_, p)| p.stage).collect();
        out.dedup();
        out
    }

    fn assert_steady(events: &[(tokio::time::Instant, Progress)], total: u64) {
        for w in events.windows(2) {
            assert!(w[0].1.done_bytes <= w[1].1.done_bytes, "bytes never go back: {} then {}", w[0].1.done_bytes, w[1].1.done_bytes);
            if w[0].1.stage == w[1].1.stage {
                assert!(w[0].1.chunks_done <= w[1].1.chunks_done, "chunks never go back within a stage");
            }
        }
        for gap in crate::progress::tests::plain_gaps(events) {
            assert!(gap >= Duration::from_millis(240), "at most one event per 250 ms: {gap:?}");
        }
        let last = &events.last().unwrap().1;
        assert_eq!((last.status.as_str(), last.done_bytes, last.total_bytes), ("done", total, total));
        assert_eq!(last.chunks_done, last.chunks_total);
    }

    #[tokio::test]
    async fn progress_tells_stages_chunks_and_bytes_in_order_and_throttled() {
        let r = small_rig().await;
        let size = 60 * CHUNK + 5;
        let path = file(&r, "clip.webm", size).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        // Slow enough for a speed to be told (after two seconds).
        *r.backend.put_delay.lock().unwrap() = Duration::from_millis(200);
        let sink = crate::progress::tests::Timed::default();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        let up = sink.0.lock().unwrap().clone();
        assert_eq!(stages(&up), vec![TransferStage::Queued, TransferStage::Uploading]);
        assert_steady(&up, size as u64);
        assert_eq!(up.last().unwrap().1.chunks_total, 61);
        assert!(up.iter().all(|(_, p)| p.file_name == "clip.webm" && p.mime == "video/webm" && p.direction == "up"));
        assert!(up.iter().any(|(_, p)| p.status == "running" && p.chunks_done > 0 && p.chunks_done < 61), "progress between");
        assert!(up.iter().any(|(_, p)| p.rate_bps > 0), "a speed while running");
        assert!(r.svc.live_views.lock().unwrap().is_empty(), "nothing stays in memory");

        // Down, a third of it already on disk.
        let (svc, dd, fetch) = https(&r, &d);
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        for i in (0..61).step_by(3) {
            let c = &dd.chunks[i];
            tokio::fs::write(tmp.join(i.to_string()), r.backend.get(&c.sha256).unwrap()).await.unwrap();
        }
        tokio::fs::write(tmp.join("1"), b"not chunk 1").await.unwrap();
        let sink = crate::progress::tests::Timed::default();
        let out = svc.run_download("m", "chat", &dd, true, &sink).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(&out).await.unwrap(), content(size));
        assert_eq!(*fetch.calls.lock().unwrap(), 61 - 21, "chunks on disk are kept, a bad one is fetched again");
        let down = sink.0.lock().unwrap().clone();
        let want = [
            TransferStage::Queued,
            TransferStage::Checking,
            TransferStage::Downloading,
            TransferStage::Assembling,
            TransferStage::Verifying,
        ];
        assert_eq!(stages(&down), want, "the chunks on disk are checked first");
        let checked = down.iter().filter(|(_, p)| p.stage == TransferStage::Checking).map(|(_, p)| p.chunks_done).max().unwrap();
        assert!(checked <= 21, "the good chunks on disk: {checked}");
        let fetching = down.iter().find(|(_, p)| p.stage == TransferStage::Downloading).unwrap();
        assert!(fetching.1.chunks_done >= 21, "fetching starts from them: {}", fetching.1.chunks_done);
        assert_steady(&down, size as u64);
        assert_eq!(down.last().unwrap().1.local_path.as_deref(), Some(out.to_string_lossy().as_ref()));
        let view = svc.transfer(&down[0].1.transfer_id).await.unwrap().unwrap();
        assert_eq!((view.chunks_done, view.chunks_total, view.stage), (61, 61, TransferStage::Downloading));
    }

    #[tokio::test]
    async fn automatic_retries_then_failed() {
        let r = rig().await;
        let path = file(&r, "a.bin", 1000).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(0);
        let sink = Sink::default();
        let err = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap_err();
        assert!(err.to_string().contains("err.network"), "{err}");
        let retry = ["waiting_retry", "queued", "running"];
        let waiting = [&["queued", "running"][..], &retry, &retry, &retry, &["failed"]].concat();
        assert_eq!(sink.statuses(), waiting);
        let events = sink.0.lock().unwrap().clone();
        let waits: Vec<_> = events.iter().filter(|p| p.status == "waiting_retry").collect();
        assert_eq!(waits.iter().map(|p| p.attempt).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert!(waits.iter().all(|p| p.retry_at_ms.is_some() && p.failure_reason.as_deref() == Some("err.network")));
        let requeued: Vec<_> = events.iter().filter(|p| p.status == "queued" && p.attempt > 0).collect();
        assert_eq!(requeued.len(), 3, "every wait that is over queues again");
        assert!(requeued.iter().all(|p| p.retry_at_ms.is_none() && p.stage == TransferStage::Queued), "{requeued:?}");
        let failed = events.last().unwrap();
        assert_eq!((failed.failure_reason.as_deref(), failed.retry_at_ms), (Some("err.network"), None));
        assert_eq!(*r.backend.put_calls.lock().unwrap(), 16, "four runs of four quick tries");
        let row = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.attempts), ("failed", 1));

        // What will not pass fails at once.
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = None;
        *r.backend.reject_puts.lock().unwrap() = true;
        let sink = Sink::default();
        let err = r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap_err();
        assert!(err.to_string().contains("err.auth_failed"), "{err}");
        assert_eq!(sink.statuses(), vec!["queued", "running", "failed"]);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().failure_reason.as_deref(), Some("err.auth_failed"));
    }

    #[tokio::test]
    async fn retry_now_and_pause_end_the_wait() {
        let mut r = rig().await;
        r.svc.retry_delays = vec![Duration::from_secs(3600); 3];
        let path = file(&r, "w.bin", 1000).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(0);
        let job = upload_job(&r, &t.id);
        until(async || { r.svc.transfer(&t.id).await.unwrap().unwrap().status == "waiting_retry" }).await;
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert!(view.retry_at_ms.is_some_and(|at| at > now_ms() + 3_000_000), "an hour away: {:?}", view.retry_at_ms);
        assert_eq!(view.failure_reason.as_deref(), Some("err.network"));
        assert_eq!(r.svc.active_transfers().await.unwrap().len(), 1, "waiting is active");

        *r.backend.die_after_puts.lock().unwrap() = None;
        assert!(r.svc.retry_now(&t.id));
        let d = tokio::time::timeout(Duration::from_secs(10), job).await.expect("woken").unwrap().unwrap();
        assert!(d.is_some());
        assert!(!r.svc.retry_now(&t.id), "nothing runs any more");

        // A pause ends the wait too.
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(0);
        let job = upload_job(&r, &t.id);
        until(async || { r.svc.transfer(&t.id).await.unwrap().unwrap().status == "waiting_retry" }).await;
        r.svc.pause(&t.id).await.unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(10), job).await.unwrap().unwrap().unwrap(), None);
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.retry_at_ms), ("paused", None));
    }

    #[tokio::test]
    async fn a_download_retries_a_network_failure_by_itself() {
        let r = rig().await;
        let path = file(&r, "n.txt", 70_000).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);
        // Both tries of the first run fail; the retry gets it.
        *fetch.fail.lock().unwrap() = 2;
        let sink = Sink::default();
        let out = svc.run_download("m", "chat", &dd, false, &sink).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(70_000));
        assert_eq!(sink.statuses(), vec!["queued", "running", "waiting_retry", "queued", "running", "done"]);
        let row = svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().unwrap();
        assert_eq!(row.attempts, 0, "a run that ended well counts no failure");
    }

    #[tokio::test]
    async fn chunk_slots_are_shared_by_all_uploads() {
        let r = small_rig().await;
        // The puts hang until every slot is taken, more than one upload's
        // workers fill: both uploads have requests under way. Then all go.
        hang_after(&r, 0);
        let a = file(&r, "a.bin", 40 * CHUNK).await;
        let b = file(&r, "b.bin", 40 * CHUNK).await;
        let ta = r.svc.queue_upload(&a, "chat", "pa").await.unwrap();
        let tb = r.svc.queue_upload(&b, "chat", "pb").await.unwrap();
        let (ja, jb) = (upload_job(&r, &ta.id), upload_job(&r, &tb.id));
        const { assert!(CHUNK_SLOTS > WORKERS) };
        until(async || { in_flight(&r) == CHUNK_SLOTS }).await;
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(10_000);
        assert!(ja.await.unwrap().unwrap().is_some());
        assert!(jb.await.unwrap().unwrap().is_some());
        let most = r.backend.max_puts_in_flight.load(Ordering::SeqCst);
        assert_eq!(most, CHUNK_SLOTS, "never more requests at once than the slots");
        assert_eq!(r.backend.len(), 80);
    }

    /// The statuses of timed events in order, a repeat told once.
    fn statuses_of(events: &[(tokio::time::Instant, Progress)]) -> Vec<String> {
        let mut out: Vec<String> = events.iter().map(|(_, p)| p.status.clone()).collect();
        out.dedup();
        out
    }

    /// The network goes down in the middle of a download of many chunks:
    /// the retry goes on from where it was, chunks and bytes never count
    /// back, and what this run has checked on disk is not read again, a
    /// bad chunk it found there neither. The outage drops the last chunk
    /// twice: it is fetched last, so the chunks before it are in (but those
    /// of the other workers), however slow the machine is.
    #[tokio::test]
    async fn a_download_retry_goes_on_from_where_it_was() {
        let mut r = small_rig().await;
        r.svc.retry_delays = vec![Duration::from_millis(300); 3];
        let size = 40 * CHUNK;
        let path = file(&r, "r.bin", size).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);
        // The last chunk on disk is bad, and the outage comes as it is
        // fetched again.
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        tokio::fs::write(tmp.join("39"), b"not chunk 39").await.unwrap();
        *fetch.delay.lock().unwrap() = Duration::from_millis(20);
        *fetch.outage.lock().unwrap() = Some((2, dd.chunks[39].sha256.clone()));
        let sink = crate::progress::tests::Timed::default();
        let out = svc.run_download("m", "chat", &dd, true, &sink).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(size));
        let events = sink.0.lock().unwrap().clone();
        assert_eq!(statuses_of(&events), ["queued", "running", "waiting_retry", "queued", "running", "done"]);
        assert_steady(&events, size as u64);
        let waiting = events.iter().find(|(_, p)| p.status == "waiting_retry").unwrap();
        assert!(waiting.1.chunks_done >= 20, "{}", waiting.1.chunks_done);
        let again = events.iter().skip_while(|(_, p)| p.status != "waiting_retry").find(|(_, p)| p.status == "running").unwrap();
        assert_eq!(again.1.stage, TransferStage::Downloading);
        assert!(again.1.chunks_done >= waiting.1.chunks_done, "not back to 0 after the wait");
        let checks = stages(&events).into_iter().filter(|s| *s == TransferStage::Checking).count();
        assert_eq!(checks, 1, "what this run checked on disk is not read again");
        assert_eq!(stages(&events)[..3], [TransferStage::Queued, TransferStage::Checking, TransferStage::Downloading]);
    }

    /// The connection drops in the middle of an upload of many chunks: the
    /// automatic retry goes on from where it was. What this run stored is
    /// never looked up on the server nor checked again, only the rest is
    /// sent, and chunks and bytes never count back across the wait.
    #[tokio::test]
    async fn an_upload_retry_goes_on_from_where_it_was() {
        let mut r = small_rig().await;
        r.svc.retry_delays = vec![Duration::from_secs(3600); 3];
        let size = 40 * CHUNK + 7;
        let path = file(&r, "u.bin", size).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(12);
        let sink = Arc::new(crate::progress::tests::Timed::default());
        let job = {
            let (svc, keys, id, sink) = (r.svc.clone(), r.keys.clone(), t.id.clone(), sink.clone());
            tokio::spawn(async move { svc.run_upload(&id, &keys, None, &*sink).await })
        };
        until(async || { r.svc.transfer(&t.id).await.unwrap().unwrap().status == "waiting_retry" }).await;
        assert_eq!(state_of(&r, &t.id).await.chunks_done(), 12);

        // The line is back, and the next attempt starts now.
        *r.backend.die_after_puts.lock().unwrap() = None;
        let puts = *r.backend.put_calls.lock().unwrap();
        assert!(r.svc.retry_now(&t.id));
        let d = tokio::time::timeout(Duration::from_secs(120), job).await.expect("woken").unwrap().unwrap().unwrap();
        assert_eq!(*r.backend.put_calls.lock().unwrap() - puts, 41 - 12, "only the chunks still missing");
        assert_eq!(r.backend.exists_calls.load(Ordering::SeqCst), 0, "what this run stored is not looked up");
        let events = sink.0.lock().unwrap().clone();
        assert_eq!(statuses_of(&events), ["queued", "running", "waiting_retry", "queued", "running", "done"]);
        assert!(!stages(&events).contains(&TransferStage::Checking), "{:?}", stages(&events));
        for w in events.windows(2) {
            assert!(w[0].1.chunks_done <= w[1].1.chunks_done, "chunks never go back: {} then {}", w[0].1.chunks_done, w[1].1.chunks_done);
        }
        assert_steady(&events, size as u64);
        let (svc, dd, _) = https(&r, &d);
        let out = svc.run_download("m", "chat", &dd, true, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(size));
    }

    /// The network is gone while most chunks are on disk from an earlier
    /// run: the row still says how many, not how far the failed fetch got.
    #[tokio::test]
    async fn a_download_that_cannot_go_on_still_counts_what_is_on_disk() {
        let r = small_rig().await;
        let path = file(&r, "k.bin", 40 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        tokio::fs::create_dir_all(&tmp).await.unwrap();
        for i in 1..40 {
            tokio::fs::write(tmp.join(i.to_string()), r.backend.get(&dd.chunks[i].sha256).unwrap()).await.unwrap();
        }
        *fetch.fail.lock().unwrap() = 1000;
        let err = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap_err();
        assert!(err.to_string().contains("err.network"), "{err}");
        let view = svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.chunks_done, view.chunks_total), ("failed", 39, 40));
        assert_eq!(view.done_bytes, 39 * CHUNK as u64);
        assert_eq!(*fetch.calls.lock().unwrap(), 4 * 2, "chunk 0 alone, two tries in each of four runs");
    }

    #[tokio::test]
    async fn downloads_go_side_by_side_within_the_shared_slots() {
        let r = small_rig().await;
        let mut sent = Vec::new();
        for name in ["a.bin", "c.bin"] {
            let path = file(&r, name, 40 * CHUNK).await;
            let t = r.svc.queue_upload(&path, "chat", name).await.unwrap();
            sent.push(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap());
        }
        // The fetches hang until the workers all have one under way.
        let (svc, dd, fetch) = https(&r, &sent[0]);
        let gate = Arc::new(Semaphore::new(0));
        *fetch.gate.lock().unwrap() = Some(gate.clone());
        let down = tokio::spawn(async move { svc.run_download("m", "chat", &dd, true, &Sink::default()).await });
        until(async || { fetch.in_flight.load(Ordering::SeqCst) == WORKERS }).await;
        gate.add_permits(10_000);
        down.await.unwrap().unwrap().unwrap();
        assert_eq!(fetch.max_in_flight.load(Ordering::SeqCst), WORKERS, "chunks side by side, as many as the workers");

        // A download beside an upload: their requests together stay within
        // the chunk slots (one gauge counts both). Both hang until every
        // slot is taken, more than one transfer's workers fill.
        let b = file(&r, "b.bin", 40 * CHUNK).await;
        let tb = r.svc.queue_upload(&b, "chat", "pb").await.unwrap();
        hang_after(&r, 0);
        r.backend.max_puts_in_flight.store(0, Ordering::SeqCst);
        let gauge = MemFetcher {
            in_flight: r.backend.puts_in_flight.clone(),
            max_in_flight: r.backend.max_puts_in_flight.clone(),
            ..MemFetcher::new(MemoryBackend { base: "https://mem.example/a".into(), ..r.backend.clone() })
        };
        let gate = Arc::new(Semaphore::new(0));
        *gauge.gate.lock().unwrap() = Some(gate.clone());
        let both = r.svc.clone().with_backend(Arc::new(r.backend.clone()), Arc::new(gauge));
        let mut dc = sent[1].clone();
        dc.servers = vec!["https://mem.example/a".into()];
        let up = {
            let (svc, keys, id) = (both.clone(), r.keys.clone(), tb.id.clone());
            tokio::spawn(async move { svc.run_upload(&id, &keys, None, &Sink::default()).await })
        };
        let down = {
            let svc = both.clone();
            tokio::spawn(async move { svc.run_download("m2", "chat", &dc, true, &Sink::default()).await })
        };
        const { assert!(CHUNK_SLOTS > WORKERS) };
        until(async || { in_flight(&r) == CHUNK_SLOTS }).await;
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(10_000);
        gate.add_permits(10_000);
        assert!(up.await.unwrap().unwrap().is_some());
        assert!(down.await.unwrap().unwrap().is_some());
        let most = r.backend.max_puts_in_flight.load(Ordering::SeqCst);
        assert_eq!(most, CHUNK_SLOTS, "both side by side, never more requests at once than the slots");
    }

    /// Paused at 30 of 70 chunks and resumed: the bar starts at 30, and the
    /// chunks found on the server never make the speed.
    #[tokio::test]
    async fn a_resumed_upload_starts_where_it_stopped_and_what_was_there_makes_no_speed() {
        let r = small_rig().await;
        let size = 70 * CHUNK;
        let path = file(&r, "resume.bin", size).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 30);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 30 }).await;
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let paused = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!(paused.done_bytes, 30 * CHUNK as u64);

        *r.backend.put_gate.lock().unwrap() = None;
        *r.backend.exists_delay.lock().unwrap() = Duration::from_millis(1);
        *r.backend.put_delay.lock().unwrap() = Duration::from_millis(300);
        let sink = crate::progress::tests::Timed::default();
        r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        let events = sink.0.lock().unwrap().clone();
        let first = &events[0].1;
        assert!(first.done_bytes >= paused.done_bytes, "{} then {}", paused.done_bytes, first.done_bytes);
        assert_eq!((first.chunks_done, first.chunks_total, first.chunk_size), (30, 70, CHUNK as u64), "not 0 of 0");
        assert_steady(&events, size as u64);
        // Forty chunks, four at a time, 300 ms each: long enough for a speed to be told.
        let real = WORKERS as f64 * CHUNK as f64 / 0.3;
        let most = events.iter().map(|(_, p)| p.rate_bps).max().unwrap();
        assert!(most > 0 && (most as f64) < 2.0 * real, "{most} B/s, the line {real} B/s");
    }

    #[tokio::test]
    async fn a_cancel_removes_the_chunks_whose_request_was_dropped() {
        let r = small_rig().await;
        let path = file(&r, "landing.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.keep_dropped_puts.lock().unwrap() = true;
        hang_after(&r, 2);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 2 }).await;
        r.svc.cancel(&t.id).await.unwrap();
        assert_eq!(job.await.unwrap().unwrap(), None);
        // The server goes on with what it was sent: four chunks land after
        // the cancel, and go too.
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(100);
        until(async || { *r.backend.put_calls.lock().unwrap() == 2 + WORKERS as u32 && in_flight(&r) == 0 }).await;
        until(async || { r.backend.is_empty() }).await;
        tokio::time::sleep(r.svc.cleanup_grace).await;
        assert!(r.backend.is_empty(), "nothing is left on the server");
    }

    /// A cancel that comes before the run registers stops it; a run never
    /// starts on a cancelled transfer.
    #[tokio::test]
    async fn a_cancel_before_the_run_is_never_lost() {
        let r = small_rig().await;
        let path = file(&r, "early.bin", 10 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 0);
        let cancel = {
            let (svc, id) = (r.svc.clone(), t.id.clone());
            tokio::spawn(async move { svc.cancel(&id).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap(), None);
        cancel.await.unwrap().unwrap();
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");

        *r.backend.put_gate.lock().unwrap() = None;
        let puts = *r.backend.put_calls.lock().unwrap();
        assert_eq!(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap(), None, "cancelled stays cancelled");
        assert_eq!(*r.backend.put_calls.lock().unwrap(), puts);

        // Nothing runs it at all: after a moment the cancel marks it itself.
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        r.svc.cancel(&t.id).await.unwrap();
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        assert_eq!(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap(), None);
        assert_eq!(*r.backend.put_calls.lock().unwrap(), puts);
    }

    #[tokio::test]
    async fn a_run_owns_its_control_and_leaves_nothing_behind() {
        let r = small_rig().await;
        let entered = async |id: &str| {
            let run = r.svc.register(id).await.unwrap();
            run.enter().await.unwrap();
            run
        };
        // One run at a time; an old one that ends never takes a newer one's control.
        let mut old = entered("t").await;
        assert!(r.svc.register("t").await.is_err(), "a second run is refused");
        r.svc.controls.lock().unwrap().remove("t");
        assert!(r.svc.register("t").await.is_err(), "its lock still says it runs");
        old.lock = None;
        let new = entered("t").await;
        drop(old);
        assert_eq!(r.svc.pause("t").await.unwrap(), Paused::ByRun, "the newer run still hears a pause");
        assert_eq!(new.control.get(), PAUSE);
        // A run that is deciding its end hears nothing more, and the next
        // one waits for it to go.
        assert_eq!(new.control.close(), PAUSE);
        assert_eq!(r.svc.pause("t").await.unwrap(), Paused::Nothing);
        let next = {
            let svc = r.svc.clone();
            tokio::spawn(async move { svc.register("t").await.map(|run| run.control.get()) })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!next.is_finished(), "waits for the end of the one before");
        drop(new);
        assert_eq!(next.await.unwrap().unwrap(), RUN);
        assert_eq!(r.svc.pause("t").await.unwrap(), Paused::Nothing);
        // Before it has claimed its row a run hears nothing: a word goes
        // where the transfer is held, and waits there for the run.
        let pending = r.svc.register("u").await.unwrap();
        assert_eq!(r.svc.pause("u").await.unwrap(), Paused::ByRun);
        assert_eq!(pending.control.get(), RUN);
        assert_eq!(runs::take_ask(r.svc.cache_dir(), "u"), Some(Word::Pause));
        drop(pending);

        // A run whose future is given up leaves no control and no view.
        let path = file(&r, "dropped.bin", 10 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 2);
        let job = upload_job(&r, &t.id);
        until(async || { state_of(&r, &t.id).await.chunks_done() == 2 && !r.svc.live_views.lock().unwrap().is_empty() }).await;
        job.abort();
        let _ = job.await;
        assert!(r.svc.controls.lock().unwrap().is_empty());
        assert!(r.svc.live_views.lock().unwrap().is_empty());
        assert!(!r.svc.retry_now(&t.id));
        // Its row still says running; nothing runs it, so a cancel marks it
        // and removes its chunks.
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "running");
        r.svc.cancel(&t.id).await.unwrap();
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;
    }

    /// The CLI beside the app: a second process with the same data folder
    /// neither pauses the first one's transfers at start nor removes their
    /// chunks from under them. Those whose run is gone it pauses.
    #[tokio::test]
    async fn another_process_never_takes_over_a_running_transfer() {
        let r = small_rig().await;
        assert_eq!(r.svc.recover().await.unwrap(), Recovered::Alone(0));
        let path = file(&r, "live.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 3);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3 }).await;
        // Left under way by a process that was killed.
        let mut gone = Vec::new();
        for (placeholder, status) in [("ph-run", repo::ST_RUNNING), ("ph-wait", repo::ST_WAITING_RETRY)] {
            let g = r.svc.queue_upload(&path, "chat", placeholder).await.unwrap();
            repo::set_status(&r.svc.store, &g.id, status, None).await.unwrap();
            gone.push(g.id);
        }

        let other = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path())
            .unwrap()
            .with_backend(Arc::new(r.backend.clone()), Arc::new(MemFetcher::new(r.backend.clone())));
        let Recovered::Beside(under_way) = other.recover().await.unwrap() else { panic!("not alone") };
        assert_eq!(under_way, ["ph"], "only the upload whose run lives goes on");
        for id in &gone {
            let v = other.transfer(id).await.unwrap().unwrap();
            assert_eq!((v.status.as_str(), v.failure_reason.as_deref()), ("paused", Some("err.interrupted")));
        }
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "running", "the first one lives: it is not paused");
        assert!(other.run_upload(&t.id, &r.keys, None, &Sink::default()).await.is_err(), "one run in all processes");

        // A pause asked there is heeded where it runs, at once.
        assert_eq!(other.pause(&t.id).await.unwrap(), Paused::ByRun, "the run is asked");
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), job).await.unwrap().unwrap().unwrap(), None);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "paused");
        assert_eq!(r.backend.len(), 3, "nothing is removed from under it");
        // Paused, it runs nowhere: the other process cancels it itself.
        assert_eq!(other.cancel_with(&t.id, None).await.unwrap(), Cancelled::Here);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;

        // Once the first one is gone, the next to start is alone.
        drop(other);
        let r2 = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path()).unwrap();
        r.svc.folder.lock().unwrap().take();
        assert_eq!(r2.recover().await.unwrap(), Recovered::Alone(0));
    }

    /// A resume that lands on another server: the chunks the first one kept
    /// are removed from it.
    #[tokio::test]
    async fn a_resume_on_another_server_removes_what_the_first_one_kept() {
        let (mut svc, dir) = plain_service().await;
        svc.chunk_size = Some(CHUNK as u64);
        let keys = Keys::generate();
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        let ok = || ResponseTemplate::new(200);
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ok()).up_to_n_times(2).with_priority(1).mount(&a).await;
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(401)).mount(&a).await;
        Mock::given(method("DELETE")).respond_with(ok()).mount(&a).await;
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ok()).mount(&b).await;
        blossom(&svc, "a", &a.uri(), 10).await;
        blossom(&svc, "b", &b.uri(), 20).await;
        let p = dir.path().join("moved.bin");
        tokio::fs::write(&p, content(10 * CHUNK)).await.unwrap();
        let t = svc.queue_upload(&p, "chat", "ph").await.unwrap();
        let e = svc.run_upload(&t.id, &keys, None, &Sink::default()).await.unwrap_err();
        assert!(e.to_string().contains("err.auth_failed"), "{e}");
        let row = repo::transfer(&svc.store, &t.id).await.unwrap().unwrap();
        let first: UploadState = serde_json::from_str(&row.state_json).unwrap();
        assert_eq!(first.server_base.as_deref(), Some(a.uri().as_str()));
        let kept: Vec<String> = first.stored().map(|c| c.sha256.clone()).collect();
        assert_eq!(kept.len(), 2);

        assert_eq!(svc.transfer(&t.id).await.unwrap().unwrap().done_bytes, 2 * CHUNK as u64);

        svc.set_server_enabled("a", false).await.unwrap();
        let sink = crate::progress::tests::Timed::default();
        let d = svc.run_upload(&t.id, &keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(d.servers, vec![b.uri()]);
        assert_ne!(d.key, first.key, "another server, another key");
        assert_eq!(calls(&b, "PUT").await, 10);
        // All ten chunks go again: the bar follows them from 0, as the
        // count does, not from the two the first server had.
        let events = sink.0.lock().unwrap().clone();
        let sending: Vec<_> = events.iter().filter(|(_, p)| p.stage == TransferStage::Uploading).map(|(_, p)| p.clone()).collect();
        assert_eq!((sending[0].done_bytes, sending[0].chunks_done), (0, 0));
        for p in &sending {
            assert!(p.done_bytes <= (p.chunks_done as u64 + WORKERS as u64) * CHUNK as u64, "{} bytes at chunk {}", p.done_bytes, p.chunks_done);
        }
        until(async || {
            let gone: Vec<String> = a
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method.as_str() == "DELETE")
                .map(|r| r.url.path().trim_start_matches('/').to_string())
                .collect();
            kept.iter().all(|sha| gone.contains(sha))
        })
        .await;
        let row = repo::transfer(&svc.store, &t.id).await.unwrap().unwrap();
        assert!(serde_json::from_str::<UploadState>(&row.state_json).unwrap().stale.is_empty(), "nothing left to remove");
    }

    #[tokio::test]
    async fn too_big_to_send() {
        let r = rig().await;
        let path = r.dir.path().join("huge.bin");
        // Sparse: nothing is written.
        std::fs::File::create(&path).unwrap().set_len(MAX_SEND_BYTES + 1).unwrap();
        let e = r.svc.queue_upload(&path, "c", "m").await.unwrap_err();
        assert!(e.to_string().contains("err.file_too_large"), "{e}");
        std::fs::File::options().write(true).open(&path).unwrap().set_len(MAX_SEND_BYTES).unwrap();
        assert!(r.svc.queue_upload(&path, "c", "m").await.is_ok(), "1 GiB itself may go");
    }

    #[test]
    fn reasons_are_codes() {
        let cases = [
            (MessengerError::Transport("err.network: connection reset by peer".into()), "err.network"),
            (MessengerError::Transport("err.server: <html>busy</html>".into()), "err.server"),
            (MessengerError::Crypto("err.file_hash_mismatch".into()), "err.file_hash_mismatch"),
            (MessengerError::Io("No space left on device (os error 28)".into()), "err.io"),
            (MessengerError::Crypto("chunk does not authenticate".into()), "err.crypto"),
            (MessengerError::Transport("tls handshake eof".into()), "err.network"),
            (MessengerError::Other("join error".into()), "err.unknown"),
            // What the runtime once showed as it was.
            (MessengerError::Invalid("unknown transfer".into()), "err.invalid"),
            (MessengerError::Storage("database is locked".into()), "err.storage"),
            (MessengerError::Invalid("err.transfer_in_progress".into()), "err.transfer_in_progress"),
        ];
        for (e, code) in cases {
            assert_eq!(short_reason(&e), code, "{e}");
        }
        assert!(retryable(&MessengerError::Transport("err.timeout: slow".into())));
        assert!(retryable(&BackendError { status: Some(503), message: "x".into() }.into()));
        assert!(!retryable(&BackendError { status: Some(403), message: "x".into() }.into()));
        assert!(!retryable(&MessengerError::Transport("err.not_found".into())));
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
        assert_eq!(r.svc.recover().await.unwrap(), Recovered::Alone(1));
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

    // ─── Public blobs ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn public_blob_on_the_fixed_backend() {
        let r = rig().await;
        let jpeg = content(40_000);
        let sha = crate::crypto::sha256_hex(&jpeg);
        let blobs = r.svc.upload_public(&r.keys, jpeg.clone(), "image/jpeg", 2).await.unwrap();
        assert_eq!(
            blobs,
            vec![PublicBlob { server_id: FIXED_SERVER_ID.into(), url: format!("https://mem.example/a/{sha}") }],
            "one server, one copy however many are asked"
        );
        assert_eq!(r.backend.get(&sha).unwrap(), jpeg, "stored as it is, not encrypted");
        assert_eq!(r.backend.content_type(&sha).as_deref(), Some("image/jpeg"));
        assert!(r.svc.active_transfers().await.unwrap().is_empty(), "no transfer rows");

        assert_eq!(
            r.svc.enabled_public_bases(&r.keys).await.unwrap(),
            vec![(FIXED_SERVER_ID.to_string(), "https://mem.example/a".to_string())]
        );
        assert!(r.svc.public_exists(&r.keys, FIXED_SERVER_ID, &sha).await.unwrap());
        r.svc.public_delete(&r.keys, FIXED_SERVER_ID, &sha).await.unwrap();
        assert!(!r.svc.public_exists(&r.keys, FIXED_SERVER_ID, &sha).await.unwrap());
        r.svc.public_delete(&r.keys, FIXED_SERVER_ID, &sha).await.unwrap();
        assert_eq!(*r.backend.delete_calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_lost_public_blob_is_put_back_on_its_server() {
        let r = rig().await;
        let jpeg = content(5_000);
        let sha = crate::crypto::sha256_hex(&jpeg);
        let blob = r.svc.put_public(&r.keys, FIXED_SERVER_ID, jpeg.clone(), "image/jpeg").await.unwrap();
        assert_eq!(blob, PublicBlob { server_id: FIXED_SERVER_ID.into(), url: format!("https://mem.example/a/{sha}") });
        assert_eq!(r.backend.get(&sha).unwrap(), jpeg);
        assert_eq!(r.backend.content_type(&sha).as_deref(), Some("image/jpeg"));
        assert!(r.svc.put_public(&r.keys, FIXED_SERVER_ID, Vec::new(), "image/jpeg").await.is_err());
        assert!(r.svc.put_public(&r.keys, FIXED_SERVER_ID, jpeg.clone(), "image/jpeg\r\nx: y").await.is_err());

        let (svc, _dir) = plain_service().await;
        let e = svc.put_public(&r.keys, "nope", jpeg, "image/jpeg").await.unwrap_err();
        assert!(e.to_string().contains("err.media_unknown_server"), "{e}");
    }

    #[tokio::test]
    async fn public_blob_refusals() {
        let r = rig().await;
        let k = &r.keys;
        assert!(r.svc.upload_public(k, Vec::new(), "image/jpeg", 1).await.is_err(), "empty");
        let big = r.svc.upload_public(k, vec![1; MAX_PUBLIC_BYTES + 1], "image/jpeg", 1).await.unwrap_err();
        assert!(big.to_string().contains("err.file_too_large"), "{big}");
        assert!(r.svc.upload_public(k, b"x".to_vec(), "image/jpeg\nx: y", 1).await.is_err(), "header injection");
        assert!(r.svc.upload_public(k, b"x".to_vec(), "", 1).await.is_err());
        assert!(r.backend.is_empty(), "nothing was sent");

        let good = "0a".repeat(32);
        for bad in [String::new(), "0A".repeat(32), "0a".repeat(31), format!("{}g", "0".repeat(63)), "0".repeat(65), "../etc/passwd".into()] {
            let e = r.svc.public_exists(k, FIXED_SERVER_ID, &bad).await.unwrap_err();
            assert!(e.to_string().contains("err.media_bad_hash"), "{bad:?}: {e}");
            assert!(r.svc.public_delete(k, FIXED_SERVER_ID, &bad).await.is_err(), "{bad:?}");
        }
        assert_eq!(*r.backend.delete_calls.lock().unwrap(), 0, "a bad hash never reaches the server");
        assert!(!r.svc.public_exists(k, FIXED_SERVER_ID, &good).await.unwrap());

        // copies = 0 still means one.
        assert_eq!(r.svc.upload_public(k, b"x".to_vec(), "image/png", 0).await.unwrap().len(), 1);
        // A failing fixed backend: the backend's error, as a stable code.
        *r.backend.die_after_puts.lock().unwrap() = Some(0);
        let e = r.svc.upload_public(k, b"y".to_vec(), "image/png", 1).await.unwrap_err();
        assert!(e.to_string().contains("err.network"), "{e}");
    }

    async fn plain_service() -> (MediaService, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().await.unwrap();
        let svc = MediaService::new(store, Arc::new(Secrets(Mutex::default())), dir.path()).unwrap();
        (svc, dir)
    }

    async fn blossom(svc: &MediaService, id: &str, url: &str, priority: i64) {
        svc.put_server(MediaServerInput {
            id: Some(id.into()),
            kind: "blossom".into(),
            url: url.into(),
            priority: Some(priority),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn public_blob_without_servers() {
        let (svc, _dir) = plain_service().await;
        let keys = Keys::generate();
        let e = svc.upload_public(&keys, b"x".to_vec(), "image/jpeg", 2).await.unwrap_err();
        assert!(e.to_string().contains("err.media_no_server"), "{e}");
        assert!(svc.enabled_public_bases(&keys).await.unwrap().is_empty());
        let e = svc.public_exists(&keys, "nope", &"a".repeat(64)).await.unwrap_err();
        assert!(e.to_string().contains("err.media_unknown_server"), "{e}");

        // An S3 server without its secret cannot take a blob: no usable
        // server, the code the contract names.
        svc.put_server(MediaServerInput {
            id: Some("s3".into()),
            kind: "s3".into(),
            url: "https://s3.example".into(),
            bucket: Some("avatars".into()),
            access_key: Some("AK".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let e = svc.upload_public(&keys, b"x".to_vec(), "image/jpeg", 2).await.unwrap_err();
        assert!(e.to_string().contains("err.media_no_server"), "{e}");
        assert!(!e.to_string().contains("err.media_no_credentials"), "{e}");
        assert!(svc.enabled_public_bases(&keys).await.unwrap().is_empty());
        // Still a server I use, though nothing can be put on it now.
        assert_eq!(svc.enabled_server_bases().await.unwrap(), vec![("s3".to_string(), "https://s3.example/avatars".to_string())]);
        svc.set_server_enabled("s3", false).await.unwrap();
        assert!(svc.enabled_server_bases().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stalled_server_is_given_up_and_the_next_one_tried() {
        let (mut svc, _dir) = plain_service().await;
        svc.public_timeout = Duration::from_millis(300);
        let keys = Keys::generate();
        let (stalled, ok) = (MockServer::start().await, MockServer::start().await);
        let hang = || ResponseTemplate::new(200).set_delay(Duration::from_secs(30));
        Mock::given(method("PUT")).respond_with(hang()).mount(&stalled).await;
        Mock::given(method("HEAD")).respond_with(hang()).mount(&stalled).await;
        Mock::given(method("DELETE")).respond_with(hang()).mount(&stalled).await;
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(&ok).await;
        blossom(&svc, "stalled", &stalled.uri(), 10).await;
        blossom(&svc, "ok", &ok.uri(), 20).await;

        let started = std::time::Instant::now();
        let blobs = svc.upload_public(&keys, b"avatar".to_vec(), "image/jpeg", 1).await.unwrap();
        assert_eq!(blobs.iter().map(|b| b.server_id.as_str()).collect::<Vec<_>>(), vec!["ok"]);
        let sha = crate::crypto::sha256_hex(b"avatar");
        let e = svc.public_exists(&keys, "stalled", &sha).await.unwrap_err();
        assert!(e.to_string().contains("err.network"), "{e}");
        let e = svc.public_delete(&keys, "stalled", &sha).await.unwrap_err();
        assert!(e.to_string().contains("err.network"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());

        // Only stalled servers: the expiry is the reason told.
        svc.set_server_enabled("ok", false).await.unwrap();
        let e = svc.upload_public(&keys, b"avatar".to_vec(), "image/jpeg", 1).await.unwrap_err();
        assert!(e.to_string().contains("err.network"), "{e}");
    }

    #[test]
    fn public_timeout_grows_with_the_blob() {
        assert_eq!(public_timeout_for(Duration::from_secs(30), 0), Duration::from_secs(30));
        assert_eq!(public_timeout_for(Duration::from_secs(30), 512 * 1024), Duration::from_secs(38));
        assert!(public_timeout_for(Duration::from_secs(30), MAX_PUBLIC_BYTES) <= Duration::from_secs(30 + 128));
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn calls(server: &MockServer, verb: &str) -> usize {
        server.received_requests().await.unwrap().iter().filter(|r| r.method.as_str() == verb).count()
    }

    #[tokio::test]
    async fn public_blob_goes_to_servers_in_priority_order() {
        let (svc, _dir) = plain_service().await;
        let keys = Keys::generate();
        let (down, b, c, off) = (MockServer::start().await, MockServer::start().await, MockServer::start().await, MockServer::start().await);
        Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(503)).mount(&down).await;
        for s in [&b, &c, &off] {
            Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(200)).mount(s).await;
        }
        // Inserted out of order; priority decides.
        blossom(&svc, "c", &c.uri(), 30).await;
        blossom(&svc, "down", &down.uri(), 10).await;
        blossom(&svc, "off", &off.uri(), 5).await;
        blossom(&svc, "b", &b.uri(), 20).await;
        svc.set_server_enabled("off", false).await.unwrap();
        // Without a secret this one cannot be written to and is passed over.
        svc.put_server(MediaServerInput {
            id: Some("s3".into()),
            kind: "s3".into(),
            url: "https://s3.example".into(),
            bucket: Some("avatars".into()),
            priority: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();

        assert_eq!(
            svc.enabled_public_bases(&keys).await.unwrap(),
            vec![("down".to_string(), down.uri()), ("b".to_string(), b.uri()), ("c".to_string(), c.uri())]
        );

        let jpeg = content(20_000);
        let sha = crate::crypto::sha256_hex(&jpeg);
        let one = svc.upload_public(&keys, jpeg.clone(), "image/jpeg", 1).await.unwrap();
        assert_eq!(one, vec![PublicBlob { server_id: "b".into(), url: format!("{}/{sha}", b.uri()) }]);
        assert_eq!((calls(&down, "PUT").await, calls(&c, "PUT").await), (1, 0), "a failed server is passed over, enough is enough");

        let two = svc.upload_public(&keys, jpeg.clone(), "image/jpeg", 2).await.unwrap();
        assert_eq!(two.iter().map(|p| p.server_id.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
        assert_eq!(two[1].url, format!("{}/{sha}", c.uri()));
        assert_eq!(calls(&off, "PUT").await, 0, "a disabled server is never written to");

        let seen = c.received_requests().await.unwrap();
        let put = seen.iter().find(|r| r.method.as_str() == "PUT").unwrap();
        assert_eq!(put.body, jpeg);
        assert_eq!(put.headers.get("content-type").unwrap(), "image/jpeg");
        assert_eq!(put.headers.get("x-sha-256").unwrap().to_str().unwrap(), sha);
    }

    #[tokio::test]
    async fn public_blob_checks_and_deletes_on_one_server() {
        let (svc, _dir) = plain_service().await;
        let keys = Keys::generate();
        let server = MockServer::start().await;
        let (here, gone) = ("1".repeat(64), "2".repeat(64));
        Mock::given(method("HEAD")).and(path(format!("/{here}"))).respond_with(ResponseTemplate::new(200)).mount(&server).await;
        Mock::given(method("HEAD")).and(path(format!("/{gone}"))).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        Mock::given(method("DELETE")).and(path(format!("/{here}"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        Mock::given(method("DELETE")).and(path(format!("/{gone}"))).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        blossom(&svc, "m", &server.uri(), 10).await;
        // Turned off later: what is left there can still be checked and removed.
        svc.set_server_enabled("m", false).await.unwrap();

        assert!(svc.public_exists(&keys, "m", &here).await.unwrap());
        assert!(!svc.public_exists(&keys, "m", &gone).await.unwrap());
        svc.public_delete(&keys, "m", &here).await.unwrap();
        svc.public_delete(&keys, "m", &gone).await.unwrap();

        let seen = server.received_requests().await.unwrap();
        let del = seen.iter().find(|r| r.method.as_str() == "DELETE").unwrap();
        let auth = del.headers.get("authorization").unwrap().to_str().unwrap();
        use base64::Engine as _;
        let json = base64::engine::general_purpose::STANDARD.decode(auth.strip_prefix("Nostr ").unwrap()).unwrap();
        let ev: nostr::prelude::Event = serde_json::from_slice(&json).unwrap();
        ev.verify().unwrap();
        assert_eq!(ev.pubkey, keys.public_key());
        let t = |k: &str| ev.tags.iter().find(|t| t.kind() == k).and_then(|t| t.as_slice().get(1).cloned());
        assert_eq!(t("t").as_deref(), Some("delete"));
        assert_eq!(t("x"), Some(here));
    }

    #[tokio::test]
    async fn public_blob_fails_when_every_server_fails() {
        let (svc, _dir) = plain_service().await;
        let keys = Keys::generate();
        let (a, b) = (MockServer::start().await, MockServer::start().await);
        Mock::given(method("PUT")).respond_with(ResponseTemplate::new(500)).mount(&a).await;
        Mock::given(method("PUT")).respond_with(ResponseTemplate::new(401)).mount(&b).await;
        blossom(&svc, "a", &a.uri(), 10).await;
        blossom(&svc, "b", &b.uri(), 20).await;
        let e = svc.upload_public(&keys, b"img".to_vec(), "image/webp", 2).await.unwrap_err();
        assert!(e.to_string().contains("err.auth_failed"), "the last server's reason: {e}");
        assert_eq!((calls(&a, "PUT").await, calls(&b, "PUT").await), (1, 1));
    }

    // ─── The end of a run, other processes, lanes ───────────────────────────

    #[test]
    fn a_cancel_heard_as_a_run_ends_decides_it() {
        let failed = || Ending::<()>::Failed(MessengerError::Transport("err.network".into()));
        assert!(matches!(Ending::Done(()).heed(CANCEL), Ending::Stopped(CANCEL)));
        assert!(matches!(failed().heed(CANCEL), Ending::Stopped(CANCEL)));
        assert!(matches!(Ending::<()>::Stopped(PAUSE).heed(CANCEL), Ending::Stopped(CANCEL)));
        assert!(matches!(Ending::Done(()).heed(PAUSE), Ending::Done(())));
        assert!(matches!(failed().heed(PAUSE), Ending::Failed(_)));
        assert!(matches!(failed().heed(RUN), Ending::Failed(_)));
        assert!(matches!(Ending::<()>::Stopped(PAUSE).heed(INTERRUPT), Ending::Stopped(INTERRUPT)), "whose pause it was");
        assert!(matches!(Ending::<()>::Stopped(INTERRUPT).heed(PAUSE), Ending::Stopped(PAUSE)));
        assert!(matches!(Ending::<()>::Stopped(PAUSE).heed(RUN), Ending::Stopped(PAUSE)));
        assert!(matches!(failed().heed(INTERRUPT), Ending::Failed(_)));
    }

    /// A cancel that comes after the last chunk, while the run is ending,
    /// is heeded: nothing is published, and the chunks go.
    #[tokio::test]
    async fn a_cancel_as_a_run_ends_is_heeded() {
        let r = small_rig().await;
        let gate = Arc::new(Semaphore::new(0));
        *r.svc.ending_gate.lock().unwrap() = Some(gate.clone());
        let path = file(&r, "end.bin", 10 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let job = upload_job(&r, &t.id);
        // Every chunk is stored; the run waits to decide how it ended.
        until(async || { state_of(&r, &t.id).await.done }).await;
        assert_eq!(r.svc.cancel_with(&t.id, Some(&r.keys)).await.unwrap(), Cancelled::ByRun);
        gate.add_permits(1);
        assert_eq!(job.await.unwrap().unwrap(), None, "nothing to publish");
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;

        // A failure that comes with a cancel ends cancelled too.
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        *r.backend.reject_puts.lock().unwrap() = true;
        let job = upload_job(&r, &t.id);
        // Refused at once, for good: the run has failed, and waits.
        until(async || { *r.backend.put_calls.lock().unwrap() > 0 }).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(r.svc.cancel_with(&t.id, Some(&r.keys)).await.unwrap(), Cancelled::ByRun);
        gate.add_permits(1);
        assert_eq!(job.await.unwrap().unwrap(), None, "cancelled, not failed");
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
    }

    /// The message goes out in the publishing stage, and only then is the
    /// upload done. A cancel meanwhile waits for that end.
    #[tokio::test]
    async fn publishing_ends_the_upload_and_a_cancel_waits_for_it() {
        let r = small_rig().await;
        let path = file(&r, "pub.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let sink = crate::progress::tests::Timed::default();
        let p = r.svc.upload_to_publish(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(p.descriptor().chunks.len(), 3);
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.stage), ("running", TransferStage::Publishing), "under way until the message is out");

        let cancel = {
            let (svc, id) = (r.svc.clone(), t.id.clone());
            tokio::spawn(async move { svc.cancel_with(&id, None).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!cancel.is_finished(), "a cancel waits for the end of the publishing");
        assert!(p.published(Some("msg-1")).await.unwrap());
        assert_eq!(cancel.await.unwrap().unwrap(), Cancelled::Nothing, "the message is out: its chunks stay");
        let row = repo::transfer(&r.svc.store, &t.id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.message_id.as_deref()), ("done", Some("msg-1")));
        assert_eq!(r.backend.len(), 3);

        let events = sink.0.lock().unwrap().clone();
        assert_eq!(stages(&events), [TransferStage::Queued, TransferStage::Uploading, TransferStage::Publishing]);
        assert_eq!(statuses_of(&events), ["queued", "running", "done"], "publishing comes before done");
        let publishing = events.iter().find(|(_, p)| p.stage == TransferStage::Publishing).unwrap();
        assert_eq!(publishing.1.status, "running");
        assert_eq!(publishing.1.message_id.as_deref(), Some("ph"), "told about the placeholder");
        assert_eq!((publishing.1.rate_bps, publishing.1.eta_secs), (0, None), "nothing moves while the message goes out");

        // A message that could not go out: the upload failed, and a cancel
        // removes its chunks.
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        let sink = Sink::default();
        let p = r.svc.upload_to_publish(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(r.backend.len(), 6);
        p.failed(&MessengerError::Transport("err.network: down".into())).await.unwrap();
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.failure_reason.as_deref()), ("failed", Some("err.network")));
        assert_eq!(sink.0.lock().unwrap().last().unwrap().status, "failed");
        assert_eq!(r.svc.cancel_with(&t.id, Some(&r.keys)).await.unwrap(), Cancelled::Here);
        until(async || { r.backend.len() == 3 }).await;
    }

    /// Stored, but the message never went out (it failed after the upload
    /// was done): a cancel of the published kind leaves it, a discard of
    /// the unpublished one removes its chunks.
    #[tokio::test]
    async fn a_stored_upload_whose_message_never_went_out_can_be_discarded() {
        let r = small_rig().await;
        let path = file(&r, "unsent.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(r.svc.cancel_with(&t.id, Some(&r.keys)).await.unwrap(), Cancelled::Nothing, "a message may name them");
        assert_eq!(r.backend.len(), 3);
        assert_eq!(r.svc.discard_unpublished(&t.id, "ph", Some(&r.keys)).await.unwrap(), Cancelled::Here);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;
        assert_eq!(r.svc.discard_unpublished(&t.id, "ph", Some(&r.keys)).await.unwrap(), Cancelled::Nothing);

        // A run published it after the caller read the placeholder: the
        // message names its chunks, and they stay.
        let t = r.svc.queue_upload(&path, "chat", "local:ph2").await.unwrap();
        r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        repo::set_result(&r.svc.store, &t.id, None, None, Some("msg-2")).await.unwrap();
        assert_eq!(r.svc.discard_unpublished(&t.id, "local:ph2", Some(&r.keys)).await.unwrap(), Cancelled::Nothing);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "done");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(r.backend.len(), 3);
    }

    /// The message went out and its process ended before the row was told:
    /// the row still names the placeholder. It is marked done and keeps
    /// its chunks; never while a run holds it.
    #[tokio::test]
    async fn a_row_whose_message_went_out_is_marked_done() {
        let r = small_rig().await;
        let path = file(&r, "out.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "local:ph").await.unwrap();
        let sink = Sink::default();
        let publishing = r.svc.upload_to_publish(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        // Killed as the message went out.
        drop(publishing);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "running");

        let run = r.svc.register(&t.id).await.unwrap();
        assert!(!r.svc.mark_published(&t.id, "local:ph").await.unwrap(), "a run holds it");
        drop(run);
        assert!(!r.svc.mark_published(&t.id, "local:other").await.unwrap(), "only the placeholder it names");
        assert!(r.svc.mark_published(&t.id, "local:ph").await.unwrap());
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "done");
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::Nothing);
        r.svc.removals_done().await;
        assert_eq!(r.backend.len(), 3, "the chunks of the message stay");
    }

    /// The app starts first, the CLI beside it runs an upload. The app sees
    /// it as it runs there, makes it retry now, and cancels it: the run
    /// there does each, its chunks are never removed from under it.
    #[tokio::test]
    async fn a_run_in_another_process_is_seen_and_asked_from_here() {
        let r = small_rig().await;
        assert_eq!(r.svc.recover().await.unwrap(), Recovered::Alone(0), "the app, alone");
        let mut cli = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path())
            .unwrap()
            .with_backend(Arc::new(r.backend.clone()), Arc::new(MemFetcher::new(r.backend.clone())))
            .with_retry_delays(vec![Duration::from_secs(3600); 3]);
        cli.chunk_size = Some(CHUNK as u64);
        assert_eq!(cli.recover().await.unwrap(), Recovered::Beside(vec![]));
        let path = file(&r, "cli.bin", 20 * CHUNK).await;
        let t = cli.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(3);
        let job = {
            let (cli, keys, id) = (cli.clone(), r.keys.clone(), t.id.clone());
            tokio::spawn(async move { cli.run_upload(&id, &keys, None, &Sink::default()).await })
        };

        // Seen from the app: what only the run knows, as it tells it.
        until(async || { r.svc.transfer(&t.id).await.unwrap().unwrap().retry_at_ms.is_some() }).await;
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.stage, view.chunks_done, view.chunks_total), ("waiting_retry", TransferStage::Uploading, 3, 20));
        assert_eq!(r.svc.active_transfers().await.unwrap()[0].retry_at_ms, view.retry_at_ms, "the list says it too");

        // Retry now, asked from the app: the run there goes on (and hangs).
        *r.backend.die_after_puts.lock().unwrap() = None;
        hang_after(&r, 0);
        assert!(r.svc.retry_now(&t.id));
        until(async || { in_flight(&r) == WORKERS }).await;
        assert_eq!(r.backend.len(), 3);

        // Cancelled from the app: the run there stops and removes its chunks.
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::ByRun);
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), job).await.unwrap().unwrap().unwrap(), None);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;
        assert!(!r.svc.retry_now(&t.id), "nothing runs it any more");
    }

    /// The app starts a transfer the CLI runs already: refused, its run
    /// never takes the cancel meant for the one there, however soon it
    /// comes.
    #[tokio::test]
    async fn a_run_refused_here_never_takes_a_word_for_the_run_elsewhere() {
        let r = small_rig().await;
        let mut cli = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path())
            .unwrap()
            .with_backend(Arc::new(r.backend.clone()), Arc::new(MemFetcher::new(r.backend.clone())));
        cli.chunk_size = Some(CHUNK as u64);
        let path = file(&r, "both.bin", 20 * CHUNK).await;
        let t = cli.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 3);
        let job = {
            let (cli, keys, id) = (cli.clone(), r.keys.clone(), t.id.clone());
            tokio::spawn(async move { cli.run_upload(&id, &keys, None, &Sink::default()).await })
        };
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3 }).await;

        // Here a run of it starts and waits for the lock the CLI holds.
        let here = upload_job(&r, &t.id);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!here.is_finished(), "still waiting for the lock");
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::ByRun);
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), job).await.unwrap().unwrap().unwrap(), None, "the CLI's run heard it");
        assert!(!matches!(here.await.unwrap(), Ok(Some(_))));
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        until(async || { r.backend.is_empty() }).await;
    }

    /// Chunks of the download on disk so far (not those half written).
    fn on_disk(tmp: &Path) -> usize {
        std::fs::read_dir(tmp)
            .map(|d| d.flatten().filter(|e| e.file_name().to_str().is_some_and(|n| n.parse::<usize>().is_ok())).count())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn a_download_pauses_at_once_and_a_cancel_removes_what_it_kept() {
        let r = small_rig().await;
        let mut sent = Vec::new();
        for n in 0..3 {
            let path = file(&r, &format!("{n}.bin"), 40 * CHUNK + n).await;
            let t = r.svc.queue_upload(&path, "chat", &format!("p{n}")).await.unwrap();
            sent.push(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap());
        }
        // Ten chunks come, then the line hangs.
        let start = |n: usize| {
            let (svc, dd, fetch) = https(&r, &sent[n]);
            *fetch.gate.lock().unwrap() = Some(Arc::new(Semaphore::new(10)));
            let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
            let job = {
                let (svc, dd) = (svc.clone(), dd.clone());
                tokio::spawn(async move { svc.run_download(&format!("m{n}"), "chat", &dd, true, &Sink::default()).await })
            };
            (svc, dd, fetch, tmp, job)
        };
        let id_of = async |svc: &MediaService, n: usize| svc.transfer_for_message(&format!("m{n}"), repo::DIR_DOWN).await.unwrap().unwrap().id;

        let (svc, dd, fetch, tmp, job) = start(0);
        until(async || { on_disk(&tmp) == 10 && fetch.in_flight.load(Ordering::SeqCst) == WORKERS }).await;
        let id = id_of(&svc, 0).await;
        let started = tokio::time::Instant::now();
        assert_eq!(svc.pause(&id).await.unwrap(), Paused::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert!(started.elapsed() < Duration::from_secs(2), "a pause does not wait for the chunks in flight");
        assert_eq!(fetch.in_flight.load(Ordering::SeqCst), 0, "they were dropped");
        let view = svc.transfer(&id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.chunks_done), ("paused", 10));
        assert_eq!(on_disk(&tmp), 10, "the chunks on disk stay for the resume");

        // Resumed: only what is missing is fetched.
        *fetch.gate.lock().unwrap() = None;
        let calls = *fetch.calls.lock().unwrap();
        let out = svc.run_download("m0", "chat", &dd, true, &Sink::default()).await.unwrap().unwrap();
        assert_eq!(*fetch.calls.lock().unwrap() - calls, 30);
        assert_eq!(tokio::fs::read(out).await.unwrap(), content(40 * CHUNK));

        // Cancelled while it runs: what it has on disk goes, and it never
        // starts again by itself.
        let (svc, _, _, tmp, job) = start(1);
        until(async || { on_disk(&tmp) == 10 }).await;
        let id = id_of(&svc, 1).await;
        assert_eq!(svc.cancel_with(&id, None).await.unwrap(), Cancelled::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert!(!tmp.exists(), "a cancel removes the chunks on disk");
        assert_eq!(svc.transfer(&id).await.unwrap().unwrap().attempts, -1);

        // Paused, then cancelled from the list: what it kept goes too.
        let (svc, _, _, tmp, job) = start(2);
        until(async || { on_disk(&tmp) == 10 }).await;
        let id = id_of(&svc, 2).await;
        assert_eq!(svc.pause(&id).await.unwrap(), Paused::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert_eq!(on_disk(&tmp), 10);
        assert_eq!(svc.cancel_with(&id, None).await.unwrap(), Cancelled::Here);
        assert!(!tmp.exists(), "nothing stays on disk for a cancelled download");
        let view = svc.transfer(&id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.attempts, view.chunks_done), ("cancelled", -1, 0));
    }

    /// A chunk that cannot be written (here a folder stands where it goes)
    /// fails the download at once: the other workers stop too.
    #[tokio::test]
    async fn a_chunk_that_cannot_be_written_stops_the_others() {
        let r = small_rig().await;
        let path = file(&r, "w.bin", 40 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);
        *fetch.delay.lock().unwrap() = Duration::from_millis(5);
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        tokio::fs::create_dir_all(tmp.join("5.part")).await.unwrap();
        let err = svc.run_download("m", "chat", &dd, false, &Sink::default()).await.unwrap_err();
        assert_eq!(short_reason(&err), "err.io", "{err}");
        let calls = *fetch.calls.lock().unwrap();
        assert!(calls < 20, "nothing more is fetched once a chunk failed for good: {calls} of 40");
    }

    /// Paused with a chunk in flight, which the server got all the same.
    /// Then that chunk's bytes change in place, the time of the file kept:
    /// the resume never encrypts them under the old key, it starts over.
    #[tokio::test]
    async fn a_chunk_in_flight_at_a_pause_holds_its_key_to_the_same_bytes() {
        let r = small_rig().await;
        let path = file(&r, "edit.jpg", 8 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.keep_dropped_puts.lock().unwrap() = true;
        hang_after(&r, 3);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3 }).await;
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let paused = state_of(&r, &t.id).await;
        assert_eq!(paused.unconfirmed.len(), WORKERS, "{paused:?}");
        let Some(Unconfirmed::Chunk { index, .. }) = paused.unconfirmed.first().cloned() else { panic!("{paused:?}") };
        let index = index as usize;
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(100);
        until(async || { r.backend.len() == 3 + WORKERS }).await;
        *r.backend.put_gate.lock().unwrap() = None;
        *r.backend.keep_dropped_puts.lock().unwrap() = false;

        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[index * CHUNK] ^= 0xff;
        std::fs::write(&path, &bytes).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();

        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_ne!(d.key, paused.key, "a new key for the new bytes");
        let again = paused.file_key().unwrap().encrypt_chunk(index as u32, &bytes[index * CHUNK..(index + 1) * CHUNK]).unwrap();
        assert!(r.backend.get(&crate::crypto::sha256_hex(&again)).is_none(), "the old key never encrypted them");
    }

    /// Cancelled while the first chunks are in flight, before any was
    /// confirmed: the key is kept from the start, so the chunks the server
    /// gets after the cancel are removed too.
    #[tokio::test]
    async fn a_cancel_before_any_chunk_is_stored_removes_the_dropped_ones() {
        let r = small_rig().await;
        let path = file(&r, "early.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.keep_dropped_puts.lock().unwrap() = true;
        hang_after(&r, 0);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS }).await;
        r.svc.cancel(&t.id).await.unwrap();
        assert_eq!(job.await.unwrap().unwrap(), None);
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(100);
        until(async || { *r.backend.put_calls.lock().unwrap() == WORKERS as u32 && in_flight(&r) == 0 }).await;
        until(async || { r.backend.is_empty() }).await;
        tokio::time::sleep(r.svc.cleanup_grace).await;
        assert!(r.backend.is_empty(), "nothing is left on the server");
    }

    /// Two big uploads take all the chunk slots they may and hang there: a
    /// photo still goes at once.
    #[tokio::test]
    async fn a_photo_beside_two_big_files_never_waits_for_their_chunks() {
        let r = small_rig().await;
        *r.backend.gate_from.lock().unwrap() = CHUNK;
        hang_after(&r, 0);
        let mut big = Vec::new();
        for n in 0..2 {
            let path = file(&r, &format!("big{n}.bin"), 9 * MIB).await;
            let t = r.svc.queue_upload(&path, "chat", &format!("pb{n}")).await.unwrap();
            big.push((t.id.clone(), upload_job(&r, &t.id)));
        }
        until(async || { in_flight(&r) == CHUNK_SLOTS - SMALL_RESERVED }).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(in_flight(&r), CHUNK_SLOTS - SMALL_RESERVED, "no more than their share");

        let photo = file(&r, "photo.jpg", 1000).await;
        let t = r.svc.queue_upload(&photo, "chat", "pp").await.unwrap();
        let sent = tokio::time::timeout(Duration::from_secs(5), r.svc.run_upload(&t.id, &r.keys, None, &Sink::default())).await;
        assert!(sent.expect("the photo went at once").unwrap().is_some());
        assert_eq!(in_flight(&r), CHUNK_SLOTS - SMALL_RESERVED, "the big ones still hang");
        for (id, job) in big {
            r.svc.cancel(&id).await.unwrap();
            assert_eq!(job.await.unwrap().unwrap(), None);
        }
    }

    // ─── Runs that are gone, waits that are over, what is left ──────────────

    /// A row left under way by a run that is gone (the app killed while the
    /// CLI had the data folder open, so nothing paused it at start): a
    /// resume takes it over, an upload and a download alike.
    #[tokio::test]
    async fn a_row_left_under_way_by_a_run_that_is_gone_is_taken_over() {
        let r = small_rig().await;
        let path = file(&r, "left.bin", 10 * CHUNK).await;
        let mut sent = None;
        for status in [repo::ST_RUNNING, repo::ST_WAITING_RETRY, repo::ST_QUEUED] {
            let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
            repo::set_status(&r.svc.store, &t.id, status, None).await.unwrap();
            assert!(!r.svc.run_alive(&t.id), "nothing runs it");
            let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap();
            assert!(d.is_some(), "{status}: taken over");
            assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "done");
            sent = d;
        }

        let (svc, dd, fetch) = https(&r, &sent.unwrap());
        *fetch.fail.lock().unwrap() = 1000;
        svc.run_download("m", "chat", &dd, true, &Sink::default()).await.unwrap_err();
        *fetch.fail.lock().unwrap() = 0;
        let id = svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().unwrap().id;
        for status in [repo::ST_RUNNING, repo::ST_WAITING_RETRY] {
            repo::set_status(&svc.store, &id, status, None).await.unwrap();
            let out = svc.run_download("m", "chat", &dd, true, &Sink::default()).await.unwrap();
            assert!(out.is_some(), "{status}: taken over");
            repo::set_status(&svc.store, &id, repo::ST_FAILED, None).await.unwrap();
        }

        // While a run holds it, it is not taken.
        let run = r.svc.register("held").await.unwrap();
        assert!(r.svc.run_alive("held"));
        drop(run);
        assert!(!r.svc.run_alive("held"));
    }

    /// The wait before an automatic retry is over while another transfer
    /// has the only slot of the lane: the transfer says it is queued for
    /// one, not that it waits for a time gone by.
    #[tokio::test]
    async fn a_retry_whose_wait_is_over_queues_for_a_slot_and_says_so() {
        let mut r = small_rig().await;
        r.svc.small = Arc::new(Semaphore::new(1));
        r.svc.retry_delays = vec![Duration::from_secs(3600); 3];
        // Small puts fail at once; the chunks of the bigger file hang.
        *r.backend.gate_from.lock().unwrap() = CHUNK;
        hang_after(&r, 0);
        *r.backend.die_after_puts.lock().unwrap() = Some(0);
        let a = r.svc.queue_upload(&file(&r, "a.bin", 1000).await, "chat", "pa").await.unwrap();
        let sink = Arc::new(Sink::default());
        let job_a = {
            let (svc, keys, id, sink) = (r.svc.clone(), r.keys.clone(), a.id.clone(), sink.clone());
            tokio::spawn(async move { svc.run_upload(&id, &keys, None, &*sink).await })
        };
        until(async || { r.svc.transfer(&a.id).await.unwrap().unwrap().status == "waiting_retry" }).await;
        let b = r.svc.queue_upload(&file(&r, "b.bin", 2 * CHUNK).await, "chat", "pb").await.unwrap();
        let job_b = upload_job(&r, &b.id);
        until(async || { in_flight(&r) > 0 }).await;

        assert!(r.svc.retry_now(&a.id));
        until(async || { r.svc.transfer(&a.id).await.unwrap().unwrap().status == "queued" }).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let view = r.svc.transfer(&a.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.stage, view.retry_at_ms), ("queued", TransferStage::Queued, None));
        let last = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!((last.status.as_str(), last.retry_at_ms, last.attempt), ("queued", None, 1));

        for (id, job) in [(a.id, job_a), (b.id, job_b)] {
            r.svc.cancel(&id).await.unwrap();
            assert_eq!(job.await.unwrap().unwrap(), None);
        }
    }

    /// Chunks left on a server the upload no longer uses go after the pass,
    /// those whose request was dropped once more a while later; never one
    /// the upload holds on its server now.
    #[tokio::test]
    async fn what_an_upload_left_goes_twice_and_what_it_holds_stays() {
        let r = small_rig().await;
        let path = file(&r, "s.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let late = b"a dropped request that lands late".to_vec();
        let sha = crate::crypto::sha256_hex(&late);
        let mut s = state_of(&r, &t.id).await;
        let base = r.backend.public_base();
        s.stale = vec![upload::StaleChunks { base, chunks: vec![d.chunks[0].sha256.clone(), sha.clone()], unconfirmed: vec![sha.clone()] }];
        repo::set_state(&r.svc.store, &t.id, &serde_json::to_string(&s).unwrap()).await.unwrap();

        let used: Arc<dyn BlobBackend> = Arc::new(r.backend.clone());
        r.svc.settle_upload(&t.id, &r.keys, &used).await;
        until(async || { *r.backend.delete_calls.lock().unwrap() == 1 }).await;
        r.backend.put(&sha, late).await.unwrap();
        until(async || { r.backend.get(&sha).is_none() }).await;
        assert_eq!(*r.backend.delete_calls.lock().unwrap(), 2);
        assert!(d.chunks.iter().all(|c| r.backend.get(&c.sha256).is_some()), "the chunks of the message stay");
        assert!(state_of(&r, &t.id).await.stale.is_empty());
    }

    /// The process dies with chunks in flight: the row names them already,
    /// so a cancel later removes them too when the server got them.
    #[tokio::test]
    async fn a_run_killed_mid_request_leaves_their_names_for_a_cancel() {
        let r = small_rig().await;
        let path = file(&r, "killed.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.keep_dropped_puts.lock().unwrap() = true;
        hang_after(&r, 2);
        let job = upload_job(&r, &t.id);
        until(async || {
            let s = state_of(&r, &t.id).await;
            in_flight(&r) == WORKERS && s.chunks_done() == 2 && s.unconfirmed.len() == WORKERS
        })
        .await;
        // Nothing settles the run: it is gone at once.
        job.abort();
        let _ = job.await;
        assert_eq!(state_of(&r, &t.id).await.unconfirmed.len(), WORKERS);

        r.svc.cancel(&t.id).await.unwrap();
        r.backend.put_gate.lock().unwrap().as_ref().unwrap().add_permits(100);
        until(async || { *r.backend.put_calls.lock().unwrap() == 2 + WORKERS as u32 && in_flight(&r) == 0 }).await;
        until(async || { r.backend.is_empty() }).await;
        tokio::time::sleep(r.svc.cleanup_grace).await;
        assert!(r.backend.is_empty(), "nothing is left on the server");
    }

    /// Paused under an earlier version, whose state names no server: a
    /// cancel still removes its chunks, from the server uploads went to.
    #[tokio::test]
    async fn a_cancel_of_an_upload_of_an_earlier_version_removes_its_chunks() {
        let r = small_rig().await;
        let path = file(&r, "old.bin", 10 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.die_after_puts.lock().unwrap() = Some(3);
        r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap_err();
        assert_eq!(r.backend.len(), 3);
        let mut s = state_of(&r, &t.id).await;
        (s.server_base, s.unconfirmed) = (None, Vec::new());
        repo::set_state(&r.svc.store, &t.id, &serde_json::to_string(&s).unwrap()).await.unwrap();
        repo::set_status(&r.svc.store, &t.id, repo::ST_PAUSED, Some("err.interrupted")).await.unwrap();

        assert_eq!(r.svc.cancel_with(&t.id, Some(&r.keys)).await.unwrap(), Cancelled::Here);
        until(async || { r.backend.is_empty() }).await;
    }

    /// Reads from memory in pieces, and every fetch waits at the gate
    /// before it ends: the first gets all of the chunk but a byte and then
    /// fails as the network would, the others get a quarter first.
    struct Piecewise {
        backend: MemoryBackend,
        gate: Arc<Semaphore>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl BlobFetcher for Piecewise {
        async fn fetch(&self, url: &str, max: u64) -> Result<Option<Vec<u8>>> {
            self.fetch_counted(url, max, &AtomicU64::new(0)).await
        }

        async fn fetch_counted(&self, url: &str, _max: u64, got: &AtomicU64) -> Result<Option<Vec<u8>>> {
            got.store(0, Ordering::SeqCst);
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let Some(bytes) = self.backend.get(url.rsplit('/').next().unwrap_or("")) else { return Ok(None) };
            let len = bytes.len() as u64;
            got.fetch_add(if call == 0 { len - 1 } else { len / 4 }, Ordering::SeqCst);
            if let Ok(p) = self.gate.acquire().await {
                p.forget();
            }
            if call == 0 {
                return Err(MessengerError::Transport("err.network: connection reset".into()));
            }
            got.store(len, Ordering::SeqCst);
            Ok(Some(bytes))
        }
    }

    /// A chunk half on its way counts for what arrived, at most 95% of it
    /// until it is all here, and a fetch that starts again never takes
    /// the bar back.
    #[tokio::test]
    async fn bytes_of_a_chunk_on_its_way_count_and_never_go_back() {
        let r = small_rig().await;
        let path = file(&r, "one.bin", CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let mut dd = d.clone();
        dd.servers = vec![r.backend.public_base()];
        let fetcher = Arc::new(Piecewise { backend: r.backend.clone(), gate: Arc::new(Semaphore::new(0)), calls: AtomicUsize::new(0) });
        let svc = r.svc.clone().with_backend(Arc::new(r.backend.clone()), fetcher.clone());
        let sink = Arc::new(crate::progress::tests::Timed::default());
        let job = {
            let (svc, dd, sink) = (svc.clone(), dd.clone(), sink.clone());
            tokio::spawn(async move { svc.run_download("m", "chat", &dd, true, &*sink).await })
        };
        let cap = CHUNK as u64 * 950 / 1000;
        let done = async || svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().map(|v| v.done_bytes).unwrap_or(0);
        until(async || { done().await == cap }).await;

        // The fetch fails and starts again with a quarter: the bar stays.
        fetcher.gate.add_permits(1);
        until(async || { fetcher.calls.load(Ordering::SeqCst) == 2 }).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(done().await, cap, "never back after a fetch that starts again");
        fetcher.gate.add_permits(1);
        assert!(job.await.unwrap().unwrap().is_some());

        let events = sink.0.lock().unwrap().clone();
        assert!(events.iter().any(|(_, p)| p.stage == TransferStage::Downloading && p.done_bytes == cap), "told while on its way");
        let mut on_its_way = events.iter().filter(|(_, p)| p.stage == TransferStage::Downloading && p.chunks_done == 0);
        assert!(on_its_way.all(|(_, p)| p.done_bytes <= cap), "at most 95% until it is all here");
        assert_steady(&events, CHUNK as u64);
    }

    /// Two big uploads behind a line that hangs: the encrypted chunks in
    /// memory are about the requests open and a few waiting (one for a
    /// slot, the queue and the one being encrypted, of each), not one in
    /// every worker besides.
    #[tokio::test]
    async fn encrypted_chunks_wait_for_a_slot_in_the_queue_not_in_workers() {
        let r = small_rig().await;
        hang_after(&r, 0);
        let mut jobs = Vec::new();
        for n in 0..2 {
            let path = file(&r, &format!("big{n}.bin"), 9 * MIB).await;
            let t = r.svc.queue_upload(&path, "chat", &format!("pb{n}")).await.unwrap();
            jobs.push((t.id.clone(), upload_job(&r, &t.id)));
        }
        let open = CHUNK_SLOTS - SMALL_RESERVED;
        until(async || { in_flight(&r) == open }).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let alive = crate::upload::sealed_alive();
        assert!(alive <= open + 2 * (1 + crate::upload::SEALED_AHEAD + 1), "{alive} encrypted chunks for {open} requests");
        for (id, job) in jobs {
            r.svc.cancel(&id).await.unwrap();
            assert_eq!(job.await.unwrap().unwrap(), None);
        }
        assert_eq!(crate::upload::sealed_alive(), 0);
    }

    /// Paused with requests half sent: the last event says what is stored,
    /// as the row does, and the resumed run starts from that very number.
    #[tokio::test]
    async fn a_pause_tells_what_is_stored_not_what_was_on_its_way() {
        let r = small_rig().await;
        let path = file(&r, "half.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        *r.backend.streamed.lock().unwrap() = true;
        hang_after(&r, 3);
        let sink = Arc::new(Sink::default());
        let job = {
            let (svc, keys, id, sink) = (r.svc.clone(), r.keys.clone(), t.id.clone(), sink.clone());
            tokio::spawn(async move { svc.run_upload(&id, &keys, None, &*sink).await })
        };
        let stored = 3 * CHUNK as u64;
        until(async || {
            let on_its_way = r.svc.transfer(&t.id).await.unwrap().unwrap().done_bytes > stored;
            on_its_way && in_flight(&r) == WORKERS && state_of(&r, &t.id).await.chunks_done() == 3
        })
        .await;
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let paused = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!((paused.status.as_str(), paused.done_bytes), ("paused", stored), "the dropped requests count for nothing");
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().done_bytes, stored, "as the row says");

        *r.backend.put_gate.lock().unwrap() = None;
        let sink = Sink::default();
        r.svc.run_upload(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(sink.0.lock().unwrap()[0].done_bytes, stored, "the next run starts from it");
    }

    /// A cancel removes what was stored: the last event of the run says
    /// so, as the row does, for an upload and a download alike.
    #[tokio::test]
    async fn a_cancelled_run_tells_nothing_is_kept() {
        let r = small_rig().await;
        let path = file(&r, "gone.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 3);
        let sink = Arc::new(Sink::default());
        let job = {
            let (svc, keys, id, sink) = (r.svc.clone(), r.keys.clone(), t.id.clone(), sink.clone());
            tokio::spawn(async move { svc.run_upload(&id, &keys, None, &*sink).await })
        };
        until(async || { state_of(&r, &t.id).await.chunks_done() == 3 }).await;
        assert_eq!(r.svc.cancel_with(&t.id, None).await.unwrap(), Cancelled::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let last = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!((last.status.as_str(), last.done_bytes, last.chunks_done), ("cancelled", 0, 0));
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.done_bytes, view.chunks_done), (0, 0), "as the row says");

        *r.backend.put_gate.lock().unwrap() = None;
        let t = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, fetch) = https(&r, &d);
        *fetch.gate.lock().unwrap() = Some(Arc::new(Semaphore::new(10)));
        let tmp = crate::download::tmp_dir(svc.cache_dir(), &dd);
        let sink = Arc::new(Sink::default());
        let job = {
            let (svc, dd, sink) = (svc.clone(), dd.clone(), sink.clone());
            tokio::spawn(async move { svc.run_download("m", "chat", &dd, true, &*sink).await })
        };
        until(async || { on_disk(&tmp) == 10 }).await;
        let id = svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().unwrap().id;
        assert_eq!(svc.cancel_with(&id, None).await.unwrap(), Cancelled::ByRun);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let last = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!((last.status.as_str(), last.done_bytes, last.chunks_done), ("cancelled", 0, 0));
        let view = svc.transfer(&id).await.unwrap().unwrap();
        assert_eq!((view.done_bytes, view.chunks_done), (0, 0), "as the row says");
    }

    /// The messenger turned off and on again in one process: the service
    /// that stops pauses what it runs and lets the data folder go, though
    /// a clone of it lives on, so the next one recovers as after a restart.
    #[tokio::test]
    async fn a_service_started_again_in_the_same_process_recovers() {
        let r = small_rig().await;
        assert_eq!(r.svc.recover().await.unwrap(), Recovered::Alone(0));
        let path = file(&r, "again.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 2);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS }).await;
        let left = r.svc.queue_upload(&path, "chat", "ph2").await.unwrap();
        repo::set_status(&r.svc.store, &left.id, repo::ST_RUNNING, None).await.unwrap();

        // One the user paused before.
        let mine = r.svc.queue_upload(&path, "chat", "ph3").await.unwrap();
        repo::set_status(&r.svc.store, &mine.id, repo::ST_PAUSED, None).await.unwrap();

        r.svc.release();
        assert_eq!(job.await.unwrap().unwrap(), None);
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!(view.status, "paused", "what ran here is paused");
        assert_eq!(view.failure_reason.as_deref(), Some(repo::REASON_INTERRUPTED), "by the end of the service, not by the user");
        let next = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path()).unwrap();
        assert_eq!(next.recover().await.unwrap(), Recovered::Alone(1), "alone: the row left under way is paused");
        let again: Vec<String> = next.interrupted().await.unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(again, [left.id, t.id], "both go on by themselves, the newest first; the user's pause stays");
    }

    /// The user paused a run, and the service stopped before that run had
    /// ended: the pause stays the user's, and nothing takes it up again.
    #[tokio::test]
    async fn a_pause_of_the_user_as_the_service_stops_stays_the_users() {
        let r = small_rig().await;
        assert_eq!(r.svc.recover().await.unwrap(), Recovered::Alone(0));
        let gate = Arc::new(Semaphore::new(0));
        *r.svc.ending_gate.lock().unwrap() = Some(gate.clone());
        let path = file(&r, "mine.bin", 20 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        hang_after(&r, 2);
        let job = upload_job(&r, &t.id);
        until(async || { in_flight(&r) == WORKERS }).await;

        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::ByRun);
        // The run waits to decide its end while the service stops.
        r.svc.release();
        gate.add_permits(1);
        assert_eq!(job.await.unwrap().unwrap(), None);
        let view = r.svc.transfer(&t.id).await.unwrap().unwrap();
        assert_eq!((view.status.as_str(), view.failure_reason), ("paused", None), "paused by the user");
        assert!(r.svc.interrupted().await.unwrap().is_empty(), "nothing goes on by itself");
    }

    /// Who started a download is kept with it: only one the user started
    /// goes on by itself after its process ended, and only what the end
    /// of a process paused.
    #[tokio::test]
    async fn only_what_the_end_of_a_process_paused_goes_on_by_itself() {
        let r = small_rig().await;
        let path = file(&r, "both.bin", 3 * CHUNK).await;
        let up = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&up.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let (svc, dd, _) = https(&r, &d);
        svc.run_download("by-hand", "chat", &dd, true, &Sink::default()).await.unwrap().unwrap();
        svc.run_download("by-itself", "chat", &dd, false, &Sink::default()).await.unwrap().unwrap();
        let down = async |m: &str| svc.transfer_for_message(m, repo::DIR_DOWN).await.unwrap().unwrap().id;
        let (by_hand, by_itself) = (down("by-hand").await, down("by-itself").await);
        let paused_by_user = svc.queue_upload(&path, "chat", "ph2").await.unwrap().id;

        for id in [&up.id, &by_hand, &by_itself] {
            repo::set_status(&svc.store, id, repo::ST_PAUSED, Some(repo::REASON_INTERRUPTED)).await.unwrap();
        }
        repo::set_status(&svc.store, &paused_by_user, repo::ST_PAUSED, None).await.unwrap();
        let again: Vec<String> = svc.interrupted().await.unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(again, [by_hand, up.id]);
    }

    /// Before its run takes it, an upload tells its stage as its row says
    /// it otherwise, and may send another file (a photo made smaller); once
    /// anything of it was sent its file stays.
    #[tokio::test]
    async fn a_queued_upload_tells_its_stage_and_may_send_another_file() {
        let r = small_rig().await;
        let big = file(&r, "photo.png", 5 * CHUNK).await;
        let smaller = file(&r, "photo.jpg", 2 * CHUNK).await;
        let t = r.svc.queue_upload(&big, "chat", "ph").await.unwrap();
        let sink = Sink::default();
        r.svc.emit_stage(&t.id, TransferStage::Preparing, &sink).await.unwrap();
        let told = sink.0.lock().unwrap().clone();
        assert_eq!(told.len(), 1);
        let p = &told[0];
        assert_eq!(
            (p.stage, p.status.as_str(), p.message_id.as_deref(), p.file_name.as_str(), p.rate_bps),
            (TransferStage::Preparing, "queued", Some("ph"), "photo.png", 0)
        );
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().stage, TransferStage::Preparing, "a view says it too");
        r.svc.stage_over(&t.id, TransferStage::Preparing);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().stage, TransferStage::Queued, "until it is over");

        let v = r.svc.replace_file(&t.id, &smaller).await.unwrap().unwrap();
        assert_eq!(
            (v.file_name.as_str(), v.mime.as_str(), v.size, v.local_path.clone()),
            ("photo.jpg", "image/jpeg", 2 * CHUNK as u64, Some(smaller.to_string_lossy().into_owned()))
        );
        assert!(r.svc.replace_file(&t.id, &r.dir.path().join("gone.jpg")).await.is_err(), "a file that is not there");
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        assert_eq!((d.name.as_str(), d.size), ("photo.jpg", 2 * CHUNK as u64));
        assert!(r.svc.replace_file(&t.id, &big).await.unwrap().is_none(), "sent: its file stays");
    }

    /// A run in another process publishes its message: from here a pause
    /// is not taken for heard, and a cancel waits for its end however long
    /// the message takes, the run's view fresh all the while.
    #[tokio::test]
    async fn a_run_elsewhere_that_publishes_is_waited_for() {
        let r = small_rig().await;
        let mut other = MediaService::new(r.svc.store.clone(), Arc::new(Secrets(Mutex::default())), r.dir.path())
            .unwrap()
            .with_backend(Arc::new(r.backend.clone()), Arc::new(MemFetcher::new(r.backend.clone())));
        other.ask_wait = Duration::from_millis(300);
        let path = file(&r, "pub.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let sink = Sink::default();
        let p = r.svc.upload_to_publish(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert_eq!(other.pause(&t.id).await.unwrap(), Paused::Nothing, "it hears no word now");
        let cancel = {
            let (other, id) = (other.clone(), t.id.clone());
            tokio::spawn(async move { other.cancel_with(&id, None).await })
        };
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!cancel.is_finished(), "waits longer than a run is given to heed a word");
        assert_eq!(other.transfer(&t.id).await.unwrap().unwrap().stage, TransferStage::Publishing);
        let view = r.dir.path().join("media").join("runs").join(format!("{}.json", t.id));
        let age = std::fs::metadata(view).unwrap().modified().unwrap().elapsed().unwrap_or_default();
        assert!(age < Duration::from_millis(1200), "the view is kept fresh: {age:?}");

        assert!(p.published(Some("msg-1")).await.unwrap());
        assert_eq!(cancel.await.unwrap().unwrap(), Cancelled::Nothing, "the message is out: its chunks stay");
        assert_eq!(r.backend.len(), 3);
    }

    /// A keyring that holds every read until a permit comes, and counts
    /// the reads.
    struct SlowSecrets {
        gate: Semaphore,
        reads: AtomicUsize,
    }

    #[async_trait]
    impl SecretStore for SlowSecrets {
        async fn get(&self, _key: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if let Ok(p) = self.gate.acquire().await {
                p.forget();
            }
            Ok(None)
        }
        async fn put(&self, _key: &str, _value: &[u8]) -> Result<()> {
            Ok(())
        }
        async fn delete(&self, _key: &str) -> Result<()> {
            Ok(())
        }
        async fn is_unlocked(&self) -> bool {
            true
        }
    }

    /// A cancel of a transfer no run holds holds its run itself while it
    /// takes the row and removes what the transfer left: a run that starts
    /// meanwhile is refused, and never sends what is being removed.
    #[tokio::test]
    async fn a_cancel_holds_the_run_while_it_ends_the_transfer() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(SlowSecrets { gate: Semaphore::new(0), reads: AtomicUsize::new(0) });
        let svc = MediaService::new(Store::open_in_memory().await.unwrap(), secrets.clone(), dir.path()).unwrap();
        // The chunks went to an S3 server, whose secret is read slowly.
        let server = ServerRow {
            id: "s3".into(),
            kind: repo::KIND_S3.into(),
            url: "https://s3.example".into(),
            bucket: Some("b".into()),
            region: None,
            access_key: Some("AK".into()),
            priority: 10,
            enabled: true,
            source: "user".into(),
            created_at: 0,
            updated_at: 0,
        };
        repo::upsert_server(&svc.store, &server).await.unwrap();
        let path = dir.path().join("left.bin");
        tokio::fs::write(&path, content(1000)).await.unwrap();
        let t = svc.queue_upload(&path, "chat", "ph").await.unwrap();
        // Left under way by a run that is gone, a chunk on its way.
        let state = UploadState {
            server_base: Some("https://s3.example/b".into()),
            unconfirmed: vec![Unconfirmed::Unknown("ab".repeat(32))],
            ..Default::default()
        };
        repo::set_state(&svc.store, &t.id, &serde_json::to_string(&state).unwrap()).await.unwrap();
        repo::set_status(&svc.store, &t.id, repo::ST_RUNNING, None).await.unwrap();

        let cancel = {
            let (svc, id) = (svc.clone(), t.id.clone());
            tokio::spawn(async move { svc.cancel_with(&id, None).await })
        };
        until(async || { secrets.reads.load(Ordering::SeqCst) > 0 }).await;
        assert_eq!(runs::held(svc.cache_dir(), &t.id), Some(true), "the cancel holds the run");
        let e = svc.run_upload(&t.id, &Keys::generate(), None, &Sink::default()).await.unwrap_err();
        assert!(e.to_string().contains("err.transfer_in_progress"), "{e}");
        secrets.gate.add_permits(1);
        assert_eq!(cancel.await.unwrap().unwrap(), Cancelled::Here);
        assert_eq!(svc.transfer(&t.id).await.unwrap().unwrap().status, "cancelled");
        assert_eq!(runs::held(svc.cache_dir(), &t.id), Some(false), "and lets it go");
    }

    /// A cancel or a pause that took the row where file locks cannot tell
    /// (in another process, no lock on the run): a run waiting for its slot
    /// never sets it running again, it stops as the row says.
    #[tokio::test]
    async fn a_run_never_runs_over_a_stop_it_could_not_hear() {
        let mut r = small_rig().await;
        let path = file(&r, "taken.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        let d = r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().unwrap();
        let puts = *r.backend.put_calls.lock().unwrap();
        for stop in [repo::ST_CANCELLED, repo::ST_PAUSED] {
            r.svc.small = Arc::new(Semaphore::new(0));
            let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
            let job = upload_job(&r, &t.id);
            until(async || { r.svc.controls.lock().unwrap().contains_key(&t.id) }).await;
            assert!(repo::claim(&r.svc.store, &t.id, stop, &[repo::ST_QUEUED]).await.unwrap());
            r.svc.small.add_permits(1);
            assert_eq!(job.await.unwrap().unwrap(), None);
            assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, stop);
        }
        assert_eq!(*r.backend.put_calls.lock().unwrap(), puts, "nothing was sent");

        let (mut svc, dd, fetch) = https(&r, &d);
        svc.small = Arc::new(Semaphore::new(0));
        let job = {
            let (svc, dd) = (svc.clone(), dd.clone());
            tokio::spawn(async move { svc.run_download("m", "chat", &dd, true, &Sink::default()).await })
        };
        let id_of = async || svc.transfer_for_message("m", repo::DIR_DOWN).await.unwrap().map(|v| v.id);
        until(async || { id_of().await.is_some_and(|id| svc.controls.lock().unwrap().contains_key(&id)) }).await;
        let id = id_of().await.unwrap();
        assert!(repo::claim(&svc.store, &id, repo::ST_CANCELLED, &[repo::ST_QUEUED]).await.unwrap());
        svc.small.add_permits(1);
        assert_eq!(job.await.unwrap().unwrap(), None);
        assert_eq!(svc.transfer(&id).await.unwrap().unwrap().status, "cancelled");
        assert_eq!(*fetch.calls.lock().unwrap(), 0, "nothing was fetched");
    }

    /// The message is named on the row only as its upload ends: a cancel
    /// that took the row meanwhile (where file locks cannot tell) leaves it
    /// naming the placeholder, whose message never went out.
    #[tokio::test]
    async fn a_message_is_named_only_as_its_upload_ends() {
        let r = small_rig().await;
        let path = file(&r, "named.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "local:ph").await.unwrap();
        let sink = Sink::default();
        let p = r.svc.upload_to_publish(&t.id, &r.keys, None, &sink).await.unwrap().unwrap();
        assert!(repo::claim(&r.svc.store, &t.id, repo::ST_CANCELLED, &[repo::ST_RUNNING]).await.unwrap());
        assert!(!p.published(Some("msg-1")).await.unwrap());
        let row = repo::transfer(&r.svc.store, &t.id).await.unwrap().unwrap();
        assert_eq!((row.status.as_str(), row.message_id.as_deref()), ("cancelled", Some("local:ph")));
    }

    /// A transfer left under way by a run that is gone (the CLI killed
    /// while the app had the data folder open): a pause marks it paused
    /// here, never while a run holds it, and a resume goes on from there.
    #[tokio::test]
    async fn a_transfer_whose_run_is_gone_is_paused_here() {
        let r = small_rig().await;
        let path = file(&r, "gone.bin", 3 * CHUNK).await;
        let t = r.svc.queue_upload(&path, "chat", "ph").await.unwrap();
        repo::set_status(&r.svc.store, &t.id, repo::ST_RUNNING, None).await.unwrap();
        let run = r.svc.register(&t.id).await.unwrap();
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::ByRun, "a run holds it: it is asked");
        drop(run);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "running");
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::Here);
        assert_eq!(r.svc.transfer(&t.id).await.unwrap().unwrap().status, "paused");
        assert_eq!(r.svc.pause(&t.id).await.unwrap(), Paused::Nothing, "not under way any more");
        assert!(r.svc.run_upload(&t.id, &r.keys, None, &Sink::default()).await.unwrap().is_some());
    }
}

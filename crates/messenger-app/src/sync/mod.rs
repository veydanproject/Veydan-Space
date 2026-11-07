// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The archive of the conversation in the user's sync storage: the handler
//! of its two entities (`internal/platform-spec.md` 9.6,
//! `internal/messenger-archive-plan.md` §4, stage Э2).
//!
//! - `messenger_segment` — the events of one scope and month as a large
//!   file v2 (chunk 1 MiB); every device writes the segments it has news
//!   for, after reading and merging the version of the storage, and replays
//!   the segments of the others (`archive`);
//! - `messenger_secret` — the keys and link secrets of groups from the box
//!   `messenger` of the lock (`secrets`).
//!
//! Both ids begin with the owner's public key (D13): a storage two
//! identities share holds both archives, and a device takes its own.
//!
//! What a segment holds and how it is merged is the archive's
//! ([`ArchiveStore`], `messenger_archive::Archive` over the store of the
//! messenger, stage Э1); what an event does when it is replayed is the
//! runtime's ([`ArchiveRuntime`] and its [`Replayer`], stage Э3). Both are
//! handed to [`ArchiveSync`] when the messenger starts (`live`,
//! `MessengerState::start`) and taken away when it stops; without them the
//! handler publishes and applies nothing of the segments, and the ops it
//! takes wait in its table. The archive records into the messenger's own
//! database for as long as the messenger runs, whatever the storage does:
//! the storage gates only what goes up and comes down. The rest of the app
//! tells the handler of the chat on screen (`scope_of_chat`), of the app
//! leaving the screen (`background`), of a group forgotten
//! (`group_forgotten`), of a private server removed (`server_forgotten`)
//! and of the storage left (`watch_storage`).
//!
//! The sync positions live in the data file of the product (not in the
//! database of the messenger): a new binding empties them with the others
//! (`Handler::state_tables`).

mod archive;
mod live;
mod secrets;
#[cfg(test)]
mod tests;

pub(crate) use archive::MessengerArchive;
pub(crate) use live::LiveRuntime;
/// The names of the two entities, for the tests of the products.
pub use messenger_archive::{ENTITY_SECRET, ENTITY_SEGMENT};

use async_trait::async_trait;
use messenger_archive::{
    ArchivePhase, ArchiveProgress, ArchiveScope, PauseReason, Replayer, SegmentId, SegmentMeta, EVENT_PROGRESS,
    EVENT_SYNCED,
};
use messenger_core::traits::UiEvent;
use messenger_core::{PubKey, Timestamp};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener, Manager};
use ts_rs::TS;
use veydan_core::{AppError, BoxFuture, CmdResult, Core, Schema};
use veydan_sync_host::Registry;

/// The name of the handler in the plan of a product.
pub const HANDLER: &str = "messenger archive";
/// The sync position of every segment the device knows.
pub(crate) const SEGMENT_STATE: &str = "messenger_archive_segments";
/// The sync position of every secret of a group the device published or took.
pub(crate) const SECRET_STATE: &str = "messenger_archive_secrets";

/// The tables of the messenger in the data file of the product: the sync
/// positions of the archive. The rest of the messenger's data is in its own
/// database (`messenger/`).
pub const SCHEMA: Schema = Schema {
    module: "messenger",
    steps: &[concat!(
        // `head_hlc`, `hash`, `lf`, `meta`: the newest op of the segment
        // this device knows, its own or another's (hash = sha256 of the
        // plaintext file, `lf` = LargeFileRef JSON, `meta` = SegmentMeta
        // JSON). `merged_hash`: the version this device wrote or merged into
        // its own archive; `replayed_hash`: the version it replayed;
        // `refused`: `<hash>@<build>` of a version a build could not merge
        // (a format it does not read). `tomb`: a tombstone to publish (a
        // scope removed here); `republish`: the push of its own op was lost
        // with the log (`forget_pushed_after`).
        "CREATE TABLE messenger_archive_segments (
            entity_id     TEXT PRIMARY KEY NOT NULL,
            head_hlc      TEXT NOT NULL DEFAULT '',
            hash          TEXT NOT NULL DEFAULT '',
            lf            TEXT NOT NULL DEFAULT '',
            meta          TEXT NOT NULL DEFAULT '',
            merged_hash   TEXT NOT NULL DEFAULT '',
            replayed_hash TEXT NOT NULL DEFAULT '',
            refused       TEXT NOT NULL DEFAULT '',
            deleted       INTEGER NOT NULL DEFAULT 0,
            tomb          INTEGER NOT NULL DEFAULT 0,
            republish     INTEGER NOT NULL DEFAULT 0
        );",
        // `name`: the name of the secret in the box (the entity id without
        // the owner); `hash`: sha256 of the value published or taken;
        // `tomb`: a tombstone to publish (a group forgotten here).
        "CREATE TABLE messenger_archive_secrets (
            name     TEXT PRIMARY KEY NOT NULL,
            head_hlc TEXT NOT NULL DEFAULT '',
            hash     TEXT NOT NULL DEFAULT '',
            deleted  INTEGER NOT NULL DEFAULT 0,
            tomb     INTEGER NOT NULL DEFAULT 0
        );",
    )],
};

/// The entities of the archive, for `Module::sync`.
pub(crate) fn register(registry: &mut Registry) {
    registry.handler(HANDLER, MessengerArchive);
}

// ── The seam to the archive (stage Э1) ───────────────────────────────────────

/// What the archive builds, merges, queues and reports (`messenger_archive`).
pub use messenger_archive::{Built, Enqueued, Merged, QueueItem, QueueProgress, ReplayReport};

/// The archive of this device as the sync handler needs it: the methods of
/// `messenger_archive::Archive` over the store of the messenger, which
/// implements it call for call (`live`); a fake in the tests. Every method
/// may be called from the cycle of sync and from the background restore at
/// the same time.
#[async_trait]
pub trait ArchiveStore: Send + Sync {
    /// Whose archive this is: the identity of the messenger's store; `None`
    /// before a key was made or imported (nothing is written or queued).
    async fn owner(&self) -> messenger_core::Result<Option<PubKey>>;
    /// The segments to write at `now`: a dirty open month once the flush
    /// interval passed, a dirty closed month at once; `force` takes every
    /// dirty segment (exit, background).
    async fn due(&self, now: Timestamp, force: bool) -> messenger_core::Result<Vec<SegmentId>>;
    /// The segment as this device holds it, compacted (D5). A segment whose
    /// events were all compacted away has `count` 0: the handler publishes
    /// a tombstone, not an empty file.
    async fn build(&self, seg: &SegmentId) -> messenger_core::Result<Built>;
    /// The monthly compaction of a closed segment: how many events were
    /// dropped (the segment is dirty then, and due).
    async fn compact(&self, seg: &SegmentId) -> messenger_core::Result<u64>;
    /// Merge the storage's file: its events this device lacks are kept and
    /// replayed through `replayer`; the union to upload when it differs.
    async fn merge(&self, seg: &SegmentId, remote: &[u8], replayer: &dyn Replayer) -> messenger_core::Result<Merged>;
    /// The merge of a version this device holds already, from what is held,
    /// without the file; `None` when it does not hold that version.
    async fn merge_held(&self, seg: &SegmentId, hash: &str, replayer: &dyn Replayer) -> messenger_core::Result<Option<Merged>>;
    /// Whether this device holds the version `hash` (wrote it, or read it).
    async fn holds(&self, seg: &SegmentId, hash: &str) -> messenger_core::Result<bool>;
    /// The file of `meta` is in the storage now (or the storage holds the
    /// union already): the segment is clean until a new event.
    async fn written(&self, seg: &SegmentId, meta: &SegmentMeta) -> messenger_core::Result<()>;
    /// The storage's file could not be merged: the segment waits for the
    /// flush interval before it is due again.
    async fn defer(&self, seg: &SegmentId) -> messenger_core::Result<()>;
    /// Every segment counts as not written: the storage is another than the
    /// one they were written to (a new binding); every segment with events
    /// is dirty.
    async fn forget_written(&self) -> messenger_core::Result<()>;
    /// The latest `at` the archive holds of a scope (the relay catch-up).
    async fn high_water(&self, scope: &ArchiveScope) -> messenger_core::Result<Option<Timestamp>>;
    /// An op of a segment was applied: queue its file unless this device
    /// holds that very version.
    async fn enqueue(&self, entity_id: &str, meta: &SegmentMeta) -> messenger_core::Result<Enqueued>;
    /// The next segment to replay, those of `open` first, then by priority
    /// (new months first); it stays pending until its status is set.
    async fn dequeue(&self, open: Option<&ArchiveScope>) -> messenger_core::Result<Option<QueueItem>>;
    /// The version `hash` is replayed whole.
    async fn done(&self, entity_id: &str, hash: &str) -> messenger_core::Result<()>;
    /// The segment could not be had; `retry_failed` puts it back.
    async fn failed(&self, entity_id: &str, hash: &str, error: &str) -> messenger_core::Result<()>;
    /// The runtime refuses it for now (`group_unknown`, `server_unknown`);
    /// `retry_parked` puts it back.
    async fn park(&self, entity_id: &str, hash: &str, why: &str) -> messenger_core::Result<()>;
    /// A file `build` does not read; `retry_refused` of another build puts
    /// it back.
    async fn refuse(&self, entity_id: &str, hash: &str, error: &str, build: &str) -> messenger_core::Result<()>;
    async fn retry_failed(&self) -> messenger_core::Result<u64>;
    async fn retry_parked(&self) -> messenger_core::Result<u64>;
    async fn retry_refused(&self, build: &str) -> messenger_core::Result<u64>;
    /// The segments the restore does not wait for: parked and refused.
    async fn blocked(&self) -> messenger_core::Result<Vec<String>>;
    /// The scopes with a segment still to come, as their keys.
    async fn restoring_scopes(&self) -> messenger_core::Result<Vec<String>>;
    /// The months of the queue refused as over the bounds of the format
    /// (the file, a record, the count of records): not restored here.
    async fn refused_too_large(&self) -> messenger_core::Result<Vec<String>>;
    /// The months this device holds and `build` refuses, over the bounds
    /// of the format: in no backup.
    async fn unwritable(&self) -> messenger_core::Result<Vec<String>>;
    /// The restore is over: what was done is forgotten, the next counts
    /// from zero.
    async fn clear_done(&self) -> messenger_core::Result<()>;
    async fn progress(&self) -> messenger_core::Result<QueueProgress>;
    /// Replay a downloaded segment file of `item`; the caller marks the
    /// item `done` once this returns.
    async fn replay_segment(&self, item: &QueueItem, bytes: &[u8], replayer: &dyn Replayer) -> messenger_core::Result<ReplayReport>;
    /// Replay what is held of a version this device holds already, without
    /// the file; `None` when it does not hold that version.
    async fn replay_held_of(&self, item: &QueueItem, replayer: &dyn Replayer) -> messenger_core::Result<Option<ReplayReport>>;
    /// The scope is gone here: its events, segments and queue go. Returns
    /// the entity ids of its segments, newest first, which go as tombstones
    /// when the scope is gone for good.
    async fn remove_scope(&self, scope: &ArchiveScope) -> messenger_core::Result<Vec<String>>;
    /// Whether `id` is a private server of this device: a segment of
    /// `dm@<id>` is parked before its file comes down otherwise.
    async fn has_server(&self, id: &str) -> messenger_core::Result<bool>;
    /// The scopes the store forgot since the last call (a private server
    /// removed here or by the snapshot of another device, a group
    /// forgotten), taken once: their segments count as not merged nor
    /// replayed from here on.
    async fn take_forgotten_scopes(&self) -> messenger_core::Result<Vec<ArchiveScope>>;
    /// The groups the store's identity has a row of: the secrets of no
    /// other group are published.
    async fn group_ids(&self) -> messenger_core::Result<Vec<String>>;
}

// ── The seam to the runtime (stage Э3) ───────────────────────────────────────

/// What the handler asks of the runtime of the messenger. Each method is
/// one of `messenger_runtime::MessengerRuntime`; the running runtime with
/// its archive implements it by forwarding (`live::LiveRuntime`:
/// `set_archive` installs the archive as the runtime's `ArchiveHooks`, or
/// `None`), a fake in the tests.
#[async_trait]
pub trait ArchiveRuntime: Send + Sync {
    /// `MessengerRuntime::replayer`: the way in for restored events. It
    /// answers `NotLoggedIn` between sessions: the segment stays pending.
    fn replayer(&self) -> Arc<dyn Replayer>;
    /// `MessengerRuntime::set_archive`: `true` — the archive is installed as
    /// the runtime's sink and marks; `false` — it is taken away.
    fn set_archive(&self, installed: bool);
    /// `MessengerRuntime::archive_backfill`: put the history this device
    /// already has into the archive; the future answers how many events
    /// were handed over, or why the walk broke off (it is asked again).
    fn archive_backfill(&self) -> messenger_core::Result<BoxFuture<'static, messenger_core::Result<u64>>>;
    /// `MessengerRuntime::archive_restored`: every segment the storage holds
    /// is replayed; the relay catch-up of a fresh database waits for it.
    fn archive_restored(&self);
    /// `MessengerRuntime::group_secret_arrived`: a key of a group was put in
    /// the box; messages waiting for it are opened.
    async fn group_secret_arrived(&self, name: &str) -> messenger_core::Result<()>;
}

// ── The state of the module ──────────────────────────────────────────────────

/// A pause for `read_only` or `quota` holds the writing this long before
/// the storage is tried again (a successful write, or the app leaving the
/// screen, ends it earlier).
const PAUSE_BACKOFF: Duration = Duration::from_secs(3600);
/// Parked segments are tried again at least this often, besides when a
/// key or a segment that may hold their welcome comes.
const PARKED_RETRY: Duration = Duration::from_secs(3600);

/// What the handler keeps between cycles, managed by the module: the seams
/// handed to it, why the archive stands, the chat open now, what it last
/// told the UI.
pub struct ArchiveSync {
    store: RwLock<Option<Arc<dyn ArchiveStore>>>,
    runtime: RwLock<Option<Arc<dyn ArchiveRuntime>>>,
    /// The archive is installed in the runtime.
    installed: AtomicBool,
    /// The past of the device was handed to the archive since the runtime
    /// was handed in; `backfilling` while the walk runs (shared with the
    /// task of the walk).
    backfilled: Arc<AtomicBool>,
    backfilling: Arc<AtomicBool>,
    /// The runtime was told the archive is restored since it was handed in.
    restored: AtomicBool,
    open: Mutex<Option<ArchiveScope>>,
    /// Why the writing stands, and since when.
    pause: Mutex<Option<(PauseReason, Instant)>>,
    /// Write every segment with news in the next collect (exit, background).
    flush: AtomicBool,
    /// The background restore runs; `again` — it was kicked meanwhile.
    draining: AtomicBool,
    again: AtomicBool,
    /// The parked segments were last put back then.
    parked_retried: Mutex<Option<Instant>>,
    /// A collect ran in this process (its staging files are swept once).
    collected: AtomicBool,
    told: Mutex<Told>,
}

struct Told {
    last: ArchiveProgress,
    at: Option<Instant>,
    #[cfg(test)]
    events: Vec<UiEvent>,
}

/// Progress while bytes move goes out at most this often.
const TELL_EVERY: Duration = Duration::from_secs(1);

pub(crate) fn idle() -> ArchiveProgress {
    ArchiveProgress { phase: ArchivePhase::Idle, done: 0, total: 0, bytes_done: 0, bytes_total: 0, month: None, reason: None }
}

impl Default for ArchiveSync {
    fn default() -> Self {
        Self {
            store: RwLock::new(None),
            runtime: RwLock::new(None),
            installed: AtomicBool::new(false),
            backfilled: Arc::new(AtomicBool::new(false)),
            backfilling: Arc::new(AtomicBool::new(false)),
            restored: AtomicBool::new(false),
            collected: AtomicBool::new(false),
            open: Mutex::new(None),
            pause: Mutex::new(None),
            flush: AtomicBool::new(false),
            draining: AtomicBool::new(false),
            again: AtomicBool::new(false),
            parked_retried: Mutex::new(None),
            told: Mutex::new(Told {
                last: idle(),
                at: None,
                #[cfg(test)]
                events: Vec::new(),
            }),
        }
    }
}

impl ArchiveSync {
    /// The archive of the messenger that runs (Э5); `None` when it stops.
    pub fn provide_store(&self, store: Option<Arc<dyn ArchiveStore>>) {
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = store;
    }

    /// The runtime that runs (when the messenger starts); `None` when it
    /// stops. With an app, the archive goes into the runtime at once —
    /// before the first relay catch-up of the session asks how far the
    /// archive reaches — and a restore left by the last run goes on.
    pub fn provide_runtime(&self, runtime: Option<Arc<dyn ArchiveRuntime>>, app: Option<&AppHandle>) {
        let some = runtime.is_some();
        // The runtime that goes (it stops, or the identity changes) gives
        // the archive back first.
        self.uninstall();
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = runtime;
        self.backfilling.store(false, Ordering::SeqCst);
        self.restored.store(false, Ordering::SeqCst);
        if let (true, Some(app)) = (some, app) {
            self.install();
            kick(app);
        }
    }

    pub(crate) fn store(&self) -> Option<Arc<dyn ArchiveStore>> {
        self.store.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn runtime(&self) -> Option<Arc<dyn ArchiveRuntime>> {
        self.runtime.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn replayer(&self) -> Option<Arc<dyn Replayer>> {
        self.runtime().map(|rt| rt.replayer())
    }

    /// Install the archive in the runtime, once both are here: recording
    /// is local, into the messenger's own database, and goes on whatever
    /// the storage does (a storage left, a folder on a phone: nothing goes
    /// up, nothing is lost). The past of the device is handed to the
    /// archive once; a walk that broke off is asked again by the next
    /// cycle (the runtime continues where it stood).
    pub(crate) fn install(&self) {
        let Some(rt) = self.runtime() else { return };
        if self.store().is_none() {
            return;
        }
        if !self.installed.swap(true, Ordering::SeqCst) {
            rt.set_archive(true);
        }
        if self.backfilled.load(Ordering::SeqCst) || self.backfilling.swap(true, Ordering::SeqCst) {
            return;
        }
        match rt.archive_backfill() {
            Ok(walk) => {
                let (backfilled, backfilling) = (self.backfilled.clone(), self.backfilling.clone());
                tokio::spawn(async move {
                    match walk.await {
                        Ok(n) => {
                            if n > 0 {
                                eprintln!("messenger: archive: {n} events of the past handed over");
                            }
                            backfilled.store(true, Ordering::SeqCst);
                        }
                        Err(e) => eprintln!("messenger: archive backfill: {e}"),
                    }
                    backfilling.store(false, Ordering::SeqCst);
                });
            }
            Err(e) => {
                eprintln!("messenger: archive backfill: {e}");
                self.backfilling.store(false, Ordering::SeqCst);
            }
        }
    }

    /// The archive is taken out of the runtime: the runtime stops, or the
    /// identity changes.
    pub(crate) fn uninstall(&self) {
        self.backfilled.store(false, Ordering::SeqCst);
        if self.installed.swap(false, Ordering::SeqCst) {
            if let Some(rt) = self.runtime() {
                rt.set_archive(false);
            }
        }
    }

    /// The sync storage is left (Э5 calls it when the binding goes): the
    /// recording goes on, nothing goes up or comes down until a storage is
    /// joined again (the next cycle of one judges anew).
    pub fn storage_gone(&self) {
        self.set_paused(Some(PauseReason::NoStorage));
    }

    /// The first collect of this process: once `true`.
    pub(crate) fn first_collect(&self) -> bool {
        !self.collected.swap(true, Ordering::SeqCst)
    }

    pub(crate) fn told_restored(&self) -> bool {
        self.restored.load(Ordering::SeqCst)
    }

    /// The queue is drained: the runtime hears it once per run, the UI
    /// with every restore that replayed something.
    pub(crate) fn restored(&self, app: Option<&AppHandle>, replayed: bool) {
        let first = !self.restored.swap(true, Ordering::SeqCst);
        if first || replayed {
            if let Some(rt) = self.runtime() {
                rt.archive_restored();
            }
            self.tell_synced(app);
        }
    }

    /// The chat open now: its segments are restored before all others.
    pub fn set_open(&self, scope: Option<ArchiveScope>) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = scope;
    }

    pub(crate) fn open(&self) -> Option<ArchiveScope> {
        self.open.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The next collect writes every segment with news, not only those due
    /// by the interval: the app quits or goes to the background. A pause
    /// of the storage is tried again then.
    pub fn flush_soon(&self) {
        self.flush.store(true, Ordering::SeqCst);
        self.resume();
    }

    pub(crate) fn take_flush(&self) -> bool {
        self.flush.swap(false, Ordering::SeqCst)
    }

    pub fn paused(&self) -> Option<PauseReason> {
        self.pause.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(why, _)| why.clone())
    }

    pub(crate) fn set_paused(&self, reason: Option<PauseReason>) {
        *self.pause.lock().unwrap_or_else(|e| e.into_inner()) = reason.map(|why| (why, Instant::now()));
    }

    /// Whether a pause of the storage still holds the writing: `read_only`
    /// and `quota` for [`PAUSE_BACKOFF`] (the storage says no for hours or
    /// days; asking every cycle spends retries for nothing), the rest for
    /// one cycle.
    pub(crate) fn pause_holds(&self) -> Option<PauseReason> {
        let pause = self.pause.lock().unwrap_or_else(|e| e.into_inner());
        match pause.as_ref() {
            Some((why @ (PauseReason::ReadOnly { .. } | PauseReason::Quota), since)) if since.elapsed() < PAUSE_BACKOFF => {
                Some(why.clone())
            }
            _ => None,
        }
    }

    /// The pause is over: a write went through, or the user asks (the app
    /// leaves the screen, a manual cycle).
    pub fn resume(&self) {
        *self.pause.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether the parked segments are due another try by time alone.
    pub(crate) fn parked_retry_due(&self) -> bool {
        let mut at = self.parked_retried.lock().unwrap_or_else(|e| e.into_inner());
        if at.is_none_or(|t| t.elapsed() >= PARKED_RETRY) {
            *at = Some(Instant::now());
            return true;
        }
        false
    }

    pub(crate) fn last_progress(&self) -> ArchiveProgress {
        self.told.lock().unwrap_or_else(|e| e.into_inner()).last.clone()
    }

    pub(crate) fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    /// Tell the UI where the archive stands (`archive.progress`): every
    /// change of phase or of the reason of a pause at once, the rest at most
    /// once a second. The last state is kept for `messenger_archive_status`.
    pub(crate) fn tell(&self, app: Option<&AppHandle>, progress: ArchiveProgress) {
        let mut told = self.told.lock().unwrap_or_else(|e| e.into_inner());
        let changed = told.last.phase != progress.phase || told.last.reason != progress.reason;
        let due = told.at.is_none_or(|at| at.elapsed() >= TELL_EVERY);
        told.last = progress.clone();
        if !changed && !due {
            return;
        }
        told.at = Some(Instant::now());
        let event = UiEvent { name: EVENT_PROGRESS.into(), payload: serde_json::to_value(&progress).unwrap_or_default() };
        #[cfg(test)]
        told.events.push(event.clone());
        drop(told);
        if let Some(app) = app {
            let _ = app.emit(crate::commands::EVENT_RUNTIME, &event);
        }
    }

    /// Every segment the storage holds is replayed (`archive.synced`).
    pub(crate) fn tell_synced(&self, app: Option<&AppHandle>) {
        let event = UiEvent { name: EVENT_SYNCED.into(), payload: serde_json::json!({}) };
        #[cfg(test)]
        self.told.lock().unwrap_or_else(|e| e.into_inner()).events.push(event.clone());
        if let Some(app) = app {
            let _ = app.emit(crate::commands::EVENT_RUNTIME, &event);
        }
    }

    #[cfg(test)]
    pub(crate) fn told(&self) -> Vec<UiEvent> {
        self.told.lock().unwrap_or_else(|e| e.into_inner()).events.clone()
    }

    /// The scope is gone here for good (a group deleted, Э5 calls it): its
    /// segments and, for a group, its secrets go as tombstones with the
    /// next collect.
    pub async fn forget_scope(&self, db: &sqlx::SqlitePool, scope: &ArchiveScope) -> CmdResult<()> {
        let ids = match self.store() {
            Some(store) => store.remove_scope(scope).await.map_err(|e| AppError::other(e.to_string()))?,
            None => Vec::new(),
        };
        archive::mark_tombs(db, scope, &ids).await?;
        if let ArchiveScope::Group { id } = scope {
            secrets::mark_tombs(db, id).await?;
        }
        Ok(())
    }

    /// The scope is gone here, not from the storage (a private server
    /// removed: its segments stay for the server put in again): what this
    /// device holds of it goes, and its segments count as not merged nor
    /// replayed, so the next restore takes them anew.
    pub async fn forget_scope_locally(&self, db: &sqlx::SqlitePool, scope: &ArchiveScope) -> CmdResult<()> {
        let mut scopes = vec![scope.clone()];
        if let Some(store) = self.store() {
            let err = |e: messenger_core::MessengerError| AppError::other(e.to_string());
            store.remove_scope(scope).await.map_err(err)?;
            // The mark the removal left is this very forgetting, taken here
            // with any other the store holds.
            scopes.extend(store.take_forgotten_scopes().await.map_err(err)?);
        }
        for scope in &scopes {
            archive::forget_replayed(db, scope).await?;
        }
        Ok(())
    }
}

/// Something a parked segment may have waited for came — a group's welcome
/// landed, a private server was put in or taken out
/// (`MessengerRuntime::set_archive_waker`): the parked go back to the queue
/// and the restore looks again.
pub(crate) fn woke(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let arch = app.state::<ArchiveSync>();
        if let Some(store) = arch.store() {
            if let Err(e) = store.retry_parked().await {
                eprintln!("messenger: archive: the parked segments were not tried again: {e}");
            }
        }
        kick(&app);
    });
}

/// Start the background restore, or have the one that runs go round once
/// more. It opens an engine of its own, as a download asked for by the UI
/// does, and leaves the cycle of sync free. Without a joined storage
/// nothing is restored; the recording goes on.
pub(crate) fn kick(app: &AppHandle) {
    let Some(arch) = app.try_state::<ArchiveSync>() else { return };
    if arch.draining.swap(true, Ordering::SeqCst) {
        arch.again.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let arch = app.state::<ArchiveSync>();
        loop {
            arch.again.store(false, Ordering::SeqCst);
            let ran = async {
                let core = app.state::<Core>();
                let sync = app.state::<veydan_sync_host::SyncManager>();
                if !veydan_sync_host::vault_active(&core).await {
                    return Ok(());
                }
                let engine = veydan_sync_host::open_engine(&core).await?;
                archive::drain(&arch, &engine, &core, &sync, Some(&app)).await
            }
            .await;
            if let Err(e) = ran {
                eprintln!("messenger: archive restore: {e}");
            }
            if !arch.again.swap(false, Ordering::SeqCst) {
                break;
            }
        }
        arch.draining.store(false, Ordering::SeqCst);
        // The bar of sync drops the phase `archive_down` it was shown.
        let _ = app.emit(veydan_sync_host::EVENT_STATUS, ());
    });
}

// ── What the rest of the app tells the handler ───────────────────────────────

/// The scope of the archive a chat's events are in: the home chat and the
/// private chat on a server (`messenger_store::chats::dm_chat_id`), a
/// group (`group:<id>`); `None` for an id of another shape.
pub(crate) fn scope_of_chat(chat_id: &str) -> Option<ArchiveScope> {
    if let Some((_, server)) = messenger_store::chats::parse_dm_chat_id(chat_id) {
        return Some(match server {
            Some(id) => ArchiveScope::DmServer { id: id.into() },
            None => ArchiveScope::Dm,
        });
    }
    let group = chat_id.strip_prefix("group:")?;
    (!group.is_empty()).then(|| ArchiveScope::Group { id: group.into() })
}

/// How long the exit of a computer waits for the open months to be
/// written.
#[cfg(desktop)]
const EXIT_FLUSH_WAIT: Duration = Duration::from_secs(10);

/// Whether the archive holds anything to write now, had it to write all.
async fn has_news(arch: &ArchiveSync) -> bool {
    let Some(store) = arch.store() else { return false };
    match store.due(now(), true).await {
        Ok(due) => !due.is_empty(),
        Err(e) => {
            eprintln!("messenger: archive: {e}");
            false
        }
    }
}

/// The app left the screen (the phone put it in the background, the
/// window went out of sight): the open months with news are written by
/// the next cycle, started now when there is something to write.
pub(crate) fn background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let arch = app.state::<ArchiveSync>();
        if has_news(&arch).await {
            arch.flush_soon();
            veydan_sync_host::trigger_cycle(&app, "messenger archive");
        }
    });
}

/// The app quits (a computer): one cycle writes the open months with
/// news, for at most `EXIT_FLUSH_WAIT`; what it does not finish the next
/// start writes, as the segments stay dirty.
#[cfg(desktop)]
pub(crate) async fn exit_flush(app: &AppHandle) {
    let arch = app.state::<ArchiveSync>();
    if !has_news(&arch).await {
        return;
    }
    arch.flush_soon();
    match tokio::time::timeout(EXIT_FLUSH_WAIT, veydan_sync_host::run_cycle(app, "messenger exit")).await {
        Ok(Err(e)) => eprintln!("messenger: archive: the cycle at exit: {e}"),
        Err(_) => eprintln!("messenger: archive: the cycle at exit did not finish in time; the next start writes the rest"),
        Ok(Ok(_)) => {}
    }
}

/// A group was removed from this device for good: its segments and its
/// secrets go as tombstones with the next collect.
pub(crate) async fn group_forgotten(app: &AppHandle, group_id: &str) {
    let arch = app.state::<ArchiveSync>();
    let core = app.state::<Core>();
    if let Err(e) = arch.forget_scope(&core.db, &ArchiveScope::Group { id: group_id.into() }).await {
        eprintln!("messenger: archive: the group {group_id} was not forgotten: {e}");
    }
}

/// A private server was removed from this device: what the archive holds
/// of its private chats here goes with it (the storage keeps the segments;
/// the server put in again restores them anew), and the restore looks
/// again.
pub(crate) async fn server_forgotten(app: &AppHandle, server_id: &str) {
    let arch = app.state::<ArchiveSync>();
    let core = app.state::<Core>();
    if let Err(e) = arch.forget_scope_locally(&core.db, &ArchiveScope::DmServer { id: server_id.into() }).await {
        eprintln!("messenger: archive: the server {server_id} was not forgotten: {e}");
    }
    kick(app);
}

/// Hear every change of the status of sync (`sync://status`: a vault
/// joined, left, its storage changed) and note when no vault is joined any
/// more; a cycle of a joined one takes the note away.
pub(crate) fn watch_storage(app: &AppHandle) {
    let handle = app.clone();
    app.listen(veydan_sync_host::EVENT_STATUS, move |_| {
        let app = handle.clone();
        tauri::async_runtime::spawn(async move {
            let core = app.state::<Core>();
            if !veydan_sync_host::vault_active(&core).await {
                app.state::<ArchiveSync>().storage_gone();
            }
        });
    });
}

// ── The status for the UI ────────────────────────────────────────────────────

/// The card «Архив переписки» of the settings of the messenger
/// (`messenger_archive_status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct ArchiveStatus {
    /// The archive works on this device: the messenger runs, a storage is
    /// joined and it is one the archive uses (not a folder on a phone).
    pub available: bool,
    pub progress: ArchiveProgress,
    /// Bytes of every segment the storage holds, as their ops say.
    #[ts(type = "number")]
    pub size_bytes: u64,
    /// The first and the last month in the archive, `yyyy-mm`.
    pub from_month: Option<String>,
    pub to_month: Option<String>,
    /// Why nothing is written now.
    pub paused: Option<PauseReason>,
    /// The scopes (`dm`, `dm@<server>`, `group:<id>`) with a segment still
    /// to restore: an open chat of one says so under its oldest message.
    pub restoring_scopes: Vec<String>,
    /// The months (`yyyy-mm`) this device does not restore, their files
    /// over the bounds of the format: they stay in the storage and in the
    /// queue, and the card says so.
    pub too_large: Vec<String>,
    /// The months (`yyyy-mm`) this device holds and does not write, over
    /// the bounds of the format: they are in no backup, and the card says so.
    pub not_backed_up: Vec<String>,
}

/// The status of the archive from the table of positions, the queue and the
/// state of the handler.
pub(crate) async fn status(arch: &ArchiveSync, core: &Core) -> CmdResult<ArchiveStatus> {
    let cfg = veydan_sync_host::config::load_config(&core.db).await;
    let joined = veydan_sync_host::vault_active(core).await;
    let usable = joined && !archive::no_storage(&cfg);
    let store = arch.store();
    let owner = match &store {
        Some(store) => store.owner().await.ok().flatten(),
        None => None,
    };
    let (size_bytes, from_month, to_month) = archive::extent(&core.db, owner.as_ref()).await?;
    let mut too_large = archive::too_large_months(&core.db, owner.as_ref()).await?;
    let mut progress = arch.last_progress();
    let mut restoring_scopes = Vec::new();
    let mut not_backed_up = Vec::new();
    if let Some(store) = &store {
        if progress.phase == ArchivePhase::Idle {
            let q = store.progress().await.unwrap_or_default();
            progress = counts(q, progress);
        }
        restoring_scopes = store.restoring_scopes().await.unwrap_or_default();
        // A file read and refused for a record or a count of records over
        // the bounds, not only one the op names too big.
        too_large.extend(store.refused_too_large().await.unwrap_or_default());
        too_large.sort();
        too_large.dedup();
        not_backed_up = store.unwritable().await.unwrap_or_default();
    }
    // A storage there is: `NoStorage` left by the runtime's last stop is
    // not a word about this one until the first collect has looked.
    let paused = if usable { arch.paused().filter(|why| *why != PauseReason::NoStorage) } else { Some(PauseReason::NoStorage) };
    Ok(ArchiveStatus {
        available: usable && store.is_some(),
        progress,
        size_bytes,
        from_month,
        to_month,
        paused,
        restoring_scopes,
        too_large,
        not_backed_up,
    })
}

/// The archive of the conversation: how big, which months, whether it is
/// written now and how far a restore is.
#[tauri::command]
pub async fn messenger_archive_status(app: AppHandle) -> CmdResult<ArchiveStatus> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let arch = app.state::<ArchiveSync>();
        let core = app.state::<Core>();
        status(&arch, &core).await
    })
    .await
}

/// The unix time now.
pub(crate) fn now() -> Timestamp {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    Timestamp(secs)
}

/// The counts of the queue in a progress for the UI.
pub(crate) fn counts(q: QueueProgress, base: ArchiveProgress) -> ArchiveProgress {
    let n = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    let b = |v: i64| u64::try_from(v.max(0)).unwrap_or_default();
    ArchiveProgress { done: n(q.done), total: n(q.total), bytes_done: b(q.bytes_done), bytes_total: b(q.bytes_total), ..base }
}

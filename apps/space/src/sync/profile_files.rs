// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Firefox profile directories as sync entities.
//!
//! `profile_lease` (id = profile id): `{ device_id, device_name, since }`; a
//! tombstone releases it. The device holding the lease may run the browser,
//! others see a badge. Two devices leasing at once: the lower HLC wins.
//!
//! `profile_snapshot` (id = profile id): `{ manifest_blob, taken_at, files, bytes }`.
//! The manifest blob lists `[{ path, blob, size, hash }]`; every file is its
//! own content-addressed blob, so unchanged files are never uploaded twice.
//! Binary state is not merged: snapshots are LWW, and when both sides changed
//! the files the profile is flagged `diverged` for the user to pick a side.

use super::state::{
    delete_profile_files_state, load_profile_files_state, load_profile_files_states,
    save_profile_files_state, ProfileFilesState,
};
use super::SyncManager;
use crate::browser::BrowserState;
use crate::commands::profiles::is_blacklisted;
use crate::error::{AppError, CmdResult};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use veydan_core::Core;
use veydan_sync::{sha256_hex, Engine, Hlc, HlcClock, Op};
use veydan_sync_host::state::load_row_state;
use veydan_sync_host::{Collected, Cycle, Handler, HashCache, EVENT_STATUS};

pub const LEASE_ENTITY: &str = "profile_lease";
pub const SNAPSHOT_ENTITY: &str = "profile_snapshot";

#[derive(Debug, Default, Serialize, Deserialize)]
struct LeasePayload {
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    since: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SnapshotPayload {
    #[serde(default)]
    manifest_blob: String,
    #[serde(default)]
    taken_at: String,
    #[serde(default)]
    files: u64,
    #[serde(default)]
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestFile {
    /// Relative path inside `firefox-profile/`, `/`-separated.
    path: String,
    blob: String,
    size: u64,
    /// SHA-256 of the plaintext; compared against local files without downloading.
    hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    files: Vec<ManifestFile>,
}

impl Manifest {
    /// Fails on malformed JSON: an empty manifest would delete every profile file.
    fn parse(json: &str) -> CmdResult<Self> {
        serde_json::from_str(json).map_err(|e| AppError::other(format!("profile manifest: {e}")))
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn blob_by_hash(&self) -> HashMap<&str, &str> {
        self.files
            .iter()
            .map(|f| (f.hash.as_str(), f.blob.as_str()))
            .collect()
    }
}

// ── Local files ──────────────────────────────────────────────────────────────

/// `firefox-profile/` of a known profile.
async fn profile_dir(core: &Core, profile_id: &str) -> CmdResult<Option<PathBuf>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT profile_path FROM profiles WHERE id = ?")
        .bind(profile_id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(row.map(|(p,)| PathBuf::from(p).join("firefox-profile")))
}

/// Every syncable file below `dir` as (relative path, absolute path).
fn scan(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if is_blacklisted(&name) || name.ends_with(".tmp") {
                continue;
            }
            let Ok(ty) = e.file_type() else { continue };
            let path = e.path();
            if ty.is_dir() {
                walk(root, &path, out);
            } else if ty.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    out.push((rel, path));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Atomic write: tmp file next to the target, then rename.
fn write_raw(path: &PathBuf, raw: &[u8]) -> CmdResult<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(AppError::io)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp).map_err(AppError::io)?;
    f.write_all(raw).map_err(AppError::io)?;
    f.sync_all().map_err(AppError::io)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(AppError::io)?;
    Ok(())
}

fn has_files(dir: &Path) -> bool {
    dir.is_dir() && !scan(dir).is_empty()
}

/// A manifest path may only descend: no absolute parts, no `..`, no blacklisted names.
fn valid_rel_path(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 1024
        && rel.split('/').all(|c| {
            !c.is_empty() && c != "." && c != ".." && !c.contains('\\') && !is_blacklisted(c)
        })
}

/// Timestamps come as RFC 3339 (ours) or SQLite `datetime('now')` (profiles table).
fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

fn lease_op(profile_id: &str, hlc: Hlc, payload: Option<LeasePayload>) -> Op {
    Op {
        entity_type: LEASE_ENTITY.into(),
        entity_id: profile_id.into(),
        hlc,
        deleted: payload.is_none(),
        payload: serde_json::to_value(payload.unwrap_or_default()).unwrap_or_default(),
    }
}

fn snapshot_op(profile_id: &str, hlc: Hlc, payload: SnapshotPayload) -> Op {
    Op {
        entity_type: SNAPSHOT_ENTITY.into(),
        entity_id: profile_id.into(),
        hlc,
        deleted: false,
        payload: serde_json::to_value(payload).unwrap_or_default(),
    }
}

// ── Lease bookkeeping (called from launch / stop) ────────────────────────────

/// Another device's lease, if any: (device_id, device_name).
pub async fn foreign_lease(core: &Core, profile_id: &str) -> CmdResult<Option<(String, String)>> {
    let device = super::config::device_id(&core.db).await?;
    Ok(load_profile_files_state(&core.db, profile_id)
        .await?
        .filter(|s| !s.lease_device.is_empty() && s.lease_device != device)
        .map(|s| (s.lease_device, s.lease_name)))
}

/// Take the lease locally; the next cycle publishes it.
pub async fn acquire_lease(core: &Core, profile_id: &str) -> CmdResult<()> {
    let db = &core.db;
    let mut st = load_profile_files_state(db, profile_id)
        .await?
        .unwrap_or_else(|| ProfileFilesState::new(profile_id));
    st.lease_device = super::config::device_id(db).await?;
    st.lease_name = super::config::device_name(db).await;
    st.lease_since = Utc::now().to_rfc3339();
    st.lease_synced = false;
    save_profile_files_state(db, &st).await
}

/// The browser exited: files changed, our lease is released.
pub async fn on_profile_stopped(core: &Core, profile_id: &str) -> CmdResult<()> {
    let db = &core.db;
    let mut st = load_profile_files_state(db, profile_id)
        .await?
        .unwrap_or_else(|| ProfileFilesState::new(profile_id));
    st.dirty = true;
    // A remote snapshot arrived while the browser ran: both sides changed now.
    if !st.pending_manifest.is_empty() {
        st.diverged = true;
    }
    if st.lease_device == super::config::device_id(db).await? {
        st.lease_device.clear();
        st.lease_name.clear();
        st.lease_since.clear();
        st.lease_synced = false;
    }
    save_profile_files_state(db, &st).await
}

/// A remote snapshot waits for this profile and can be applied now.
pub async fn has_pending(core: &Core, profile_id: &str) -> CmdResult<bool> {
    Ok(load_profile_files_state(&core.db, profile_id)
        .await?
        .map(|s| !s.pending_manifest.is_empty() && !s.diverged)
        .unwrap_or(false))
}

// ── Push ─────────────────────────────────────────────────────────────────────

pub struct LocalChanges {
    pub ops: Vec<Op>,
    pub states: Vec<ProfileFilesState>,
}

/// Names uploaded in this cycle; skip a second PUT of the same content.
struct BlobIndex<'a> {
    engine: &'a Engine,
    uploaded: BTreeSet<String>,
}

impl<'a> BlobIndex<'a> {
    async fn exists(&self, name: &str) -> CmdResult<bool> {
        if self.uploaded.contains(name) {
            return Ok(true);
        }
        self.engine.blob_exists(name).await.map_err(AppError::other)
    }

    async fn put(&mut self, data: &[u8]) -> CmdResult<String> {
        let name = self.engine.blob_name(data);
        if self.uploaded.contains(&name) {
            return Ok(name);
        }
        self.engine.put_blob(data).await.map_err(AppError::other)?;
        self.uploaded.insert(name.clone());
        Ok(name)
    }
}

/// Manifest of the directory; only files whose hash is new are read and uploaded.
async fn build_manifest(
    cache: &HashCache,
    blobs: &mut BlobIndex<'_>,
    dir: &Path,
    prev: &Manifest,
    app: Option<&AppHandle>,
) -> CmdResult<Manifest> {
    let known = prev.blob_by_hash();
    let entries = scan(dir);
    let total = entries.len() as u32;
    let mut files = Vec::new();
    for (i, (rel, abs)) in entries.into_iter().enumerate() {
        let current = i as u32 + 1;
        progress(
            app,
            "profiles_up",
            super::progress_pct(55, 75, current, total.max(1)),
            current,
            total,
            &rel,
        );
        let Some(hash) = cache.file_hash(&abs) else {
            continue;
        };
        let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        // A name from the previous manifest is reused only while the blob is
        // still in the vault; GC may have dropped it since.
        let reusable = match known.get(hash.as_str()) {
            Some(b) if blobs.exists(b).await? => Some((*b).to_string()),
            _ => None,
        };
        let blob = match reusable {
            Some(b) => b,
            None => {
                let Ok(data) = std::fs::read(&abs) else {
                    continue;
                };
                blobs.put(&data).await?
            }
        };
        files.push(ManifestFile {
            path: rel,
            blob,
            size,
            hash,
        });
    }
    Ok(Manifest { files })
}

/// Lease ops only: a tiny payload, never waits on profile files.
pub async fn collect_leases(core: &Core, clock: &mut HlcClock) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let device = super::config::device_id(db).await?;
    let mut out = LocalChanges {
        ops: Vec::new(),
        states: Vec::new(),
    };
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM profiles")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    for (id,) in rows {
        let mut st = load_profile_files_state(db, &id)
            .await?
            .unwrap_or_else(|| ProfileFilesState::new(&id));
        if st.lease_synced {
            continue;
        }
        let hlc = clock.now();
        let payload = (st.lease_device == device).then(|| LeasePayload {
            device_id: device.clone(),
            device_name: st.lease_name.clone(),
            since: st.lease_since.clone(),
        });
        out.ops.push(lease_op(&id, hlc.clone(), payload));
        st.lease_hlc = Some(hlc);
        st.lease_synced = true;
        out.states.push(st);
    }
    Ok(out)
}

/// Snapshots of profiles that ran since the last one. Leases are collected separately.
/// One profile decision for the developer sync log.
fn trace_profile(app: Option<&AppHandle>, id: &str, message: &str) {
    if !app.is_some_and(super::debug_on) {
        return;
    }
    let short: String = id.chars().take(8).collect();
    trace(app, "info", &format!("{short} {message}"));
}

/// A line of the developer sync log, when the app runs.
fn trace(app: Option<&AppHandle>, level: &str, message: &str) {
    if let Some(app) = app {
        super::trace(app, level, "profiles", message);
    }
}

/// Where the progress bar stands, when the app runs.
fn progress(
    app: Option<&AppHandle>,
    phase: &str,
    percent: u32,
    current: u32,
    total: u32,
    detail: &str,
) {
    if let Some(app) = app {
        super::emit_progress(app, phase, percent, current, total, detail);
    }
}

pub async fn collect_local_changes(
    engine: &Engine,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    clock: &mut HlcClock,
    app: Option<&AppHandle>,
) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let device = super::config::device_id(db).await?;
    let mut out = LocalChanges {
        ops: Vec::new(),
        states: Vec::new(),
    };
    let mut states = load_profile_files_states(db).await?;
    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, profile_path, last_launch_at FROM profiles")
            .fetch_all(db)
            .await
            .map_err(AppError::db)?;
    if rows.is_empty() {
        trace(app, "info", "no profiles");
    }
    let mut blobs = BlobIndex {
        engine,
        uploaded: BTreeSet::new(),
    };

    for (id, profile_path, last_launch_at) in rows {
        let mut st = states
            .remove(&id)
            .unwrap_or_else(|| ProfileFilesState::new(&id));

        let dir = PathBuf::from(&profile_path).join("firefox-profile");
        let launched_after = match (
            last_launch_at.as_deref().and_then(parse_time),
            parse_time(&st.snapshot_at),
        ) {
            (Some(launch), Some(snap)) => launch > snap,
            (Some(_), None) => true,
            _ => false,
        };
        let candidate = st.dirty || launched_after || (st.head_hlc.is_none() && has_files(&dir));
        let running = browser.running.is_running(&id).await;
        let foreign_lease = !st.lease_device.is_empty() && st.lease_device != device;
        if !candidate || running || st.diverged || foreign_lease {
            // Someone else is running it: our local changes cannot win, let the user decide.
            let now_diverged = foreign_lease && candidate && !running && st.dirty && !st.diverged;
            if now_diverged {
                st.diverged = true;
                save_profile_files_state(db, &st).await?;
            }
            let reason = if running {
                "running"
            } else if foreign_lease {
                "foreign lease"
            } else if st.diverged {
                "diverged"
            } else {
                "unchanged"
            };
            trace_profile(app, &id, reason);
            continue;
        }

        // Local bookkeeping only; empty before the first snapshot.
        let prev = Manifest::parse(&st.manifest_json).unwrap_or_default();
        let manifest = build_manifest(&sync.file_hashes, &mut blobs, &dir, &prev, app).await?;
        let json = manifest.to_json();
        let hash = sha256_hex(json.as_bytes());
        let now = Utc::now().to_rfc3339();
        if hash == st.synced_hash {
            st.dirty = false;
            st.snapshot_at = now;
            save_profile_files_state(db, &st).await?;
            trace_profile(app, &id, "hash unchanged");
            continue;
        }
        let manifest_blob = blobs.put(json.as_bytes()).await?;
        let hlc = clock.now();
        let files = manifest.files.len();
        let bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
        out.ops.push(snapshot_op(
            &id,
            hlc.clone(),
            SnapshotPayload {
                manifest_blob,
                taken_at: now.clone(),
                files: files as u64,
                bytes,
            },
        ));
        st.head_hlc = Some(hlc);
        st.synced_hash = hash;
        st.manifest_json = json;
        st.snapshot_at = now;
        st.dirty = false;
        st.pending_manifest.clear();
        out.states.push(st);
        trace_profile(app, &id, &format!("snapshot {files} files, {bytes} bytes"));
    }
    // Profiles deleted locally leave no state behind.
    for id in states.keys() {
        delete_profile_files_state(db, id).await?;
    }
    Ok(out)
}

// ── Pull ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ApplyOutcome {
    /// Set when an op was skipped on a transient condition; peer heads must not advance.
    pub retry: Option<String>,
    /// Blobs collected by GC. Reported as warnings; peer heads still advance.
    pub skipped: Vec<String>,
}

enum BlobMiss {
    Retry(String),
    Gone(String),
}

/// A missing blob younger than the GC grace is delayed; an older one was collected.
fn blob_miss(op: Option<&Op>, delayed: String, collected: String) -> BlobMiss {
    let now_ms = Utc::now().timestamp_millis().max(0) as u64;
    if op.is_some_and(|op| super::blob_gone(op, now_ms)) {
        BlobMiss::Gone(collected)
    } else {
        BlobMiss::Retry(delayed)
    }
}

/// Bring `dir` to the manifest: download changed files, drop files it does not list.
/// `Some` when a blob is missing.
async fn apply_manifest(
    engine: &Engine,
    cache: &HashCache,
    dir: &Path,
    manifest: &Manifest,
    app: Option<&AppHandle>,
    op: Option<&Op>,
) -> CmdResult<Option<BlobMiss>> {
    std::fs::create_dir_all(dir).map_err(AppError::io)?;
    let mut keep: HashSet<&str> = HashSet::new();
    let listed: Vec<&ManifestFile> = manifest
        .files
        .iter()
        .filter(|f| valid_rel_path(&f.path))
        .collect();
    let total = listed.len() as u32;
    for (i, f) in listed.into_iter().enumerate() {
        let current = i as u32 + 1;
        progress(
            app,
            "profiles_down",
            super::progress_pct(78, 93, current, total.max(1)),
            current,
            total,
            &f.path,
        );
        keep.insert(f.path.as_str());
        let dest = dir.join(&f.path);
        if cache.file_hash(&dest).as_deref() == Some(f.hash.as_str()) {
            continue;
        }
        let Some(data) = engine.get_blob(&f.blob).await.map_err(AppError::other)? else {
            return Ok(Some(blob_miss(
                op,
                format!("blob for profile file {} not available yet", f.path),
                format!("blob for profile file {} was collected", f.path),
            )));
        };
        write_raw(&dest, &data)?;
    }
    for (rel, abs) in scan(dir) {
        if !keep.contains(rel.as_str()) {
            let _ = std::fs::remove_file(abs);
        }
    }
    Ok(None)
}

/// Download and apply a manifest blob. `Err` means a blob is missing.
async fn fetch_and_apply(
    engine: &Engine,
    sync: &SyncManager,
    app: Option<&AppHandle>,
    dir: &Path,
    manifest_blob: &str,
    op: Option<&Op>,
) -> CmdResult<Result<Manifest, BlobMiss>> {
    let Some(raw) = engine
        .get_blob(manifest_blob)
        .await
        .map_err(AppError::other)?
    else {
        return Ok(Err(blob_miss(
            op,
            format!("manifest {manifest_blob} not available yet"),
            format!("manifest {manifest_blob} was collected"),
        )));
    };
    let manifest = Manifest::parse(&String::from_utf8_lossy(&raw))?;
    match apply_manifest(engine, &sync.file_hashes, dir, &manifest, app, op).await? {
        None => Ok(Ok(manifest)),
        Some(miss) => Ok(Err(miss)),
    }
}

fn record_applied(st: &mut ProfileFilesState, manifest: &Manifest) {
    let json = manifest.to_json();
    st.synced_hash = sha256_hex(json.as_bytes());
    st.manifest_json = json;
    st.snapshot_at = Utc::now().to_rfc3339();
    st.dirty = false;
    st.pending_manifest.clear();
    st.diverged = false;
}

/// Apply remote lease ops. Independent of profile file snapshots.
pub async fn apply_leases(core: &Core, ops: &[Op]) -> CmdResult<ApplyOutcome> {
    let db = &core.db;
    let device = super::config::device_id(db).await?;
    let outcome = ApplyOutcome::default();

    for op in ops {
        if op.entity_type != LEASE_ENTITY {
            continue;
        }
        let id = op.entity_id.as_str();
        let mut st = load_profile_files_state(db, id)
            .await?
            .unwrap_or_else(|| ProfileFilesState::new(id));
        let stale = st.lease_hlc.as_ref().map(|h| *h >= op.hlc).unwrap_or(false);
        if op.deleted {
            if stale {
                continue;
            }
            if st.lease_device == op.hlc.device_id {
                st.lease_device.clear();
                st.lease_name.clear();
                st.lease_since.clear();
            }
        } else {
            if st.lease_device == device {
                // Concurrent launches: the earlier one (lower HLC) keeps the profile.
                if st.lease_hlc.as_ref().map(|h| *h < op.hlc).unwrap_or(false) {
                    continue;
                }
            } else if stale {
                continue;
            }
            let p: LeasePayload = serde_json::from_value(op.payload.clone()).unwrap_or_default();
            st.lease_device = p.device_id;
            st.lease_name = p.device_name;
            st.lease_since = p.since;
            st.lease_synced = true;
        }
        st.lease_hlc = Some(op.hlc.clone());
        save_profile_files_state(db, &st).await?;
    }
    Ok(outcome)
}

/// Apply remote snapshot ops. Ops are already HLC-sorted. Without the app no
/// progress goes out.
pub async fn apply_remote(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    ops: &[Op],
) -> CmdResult<ApplyOutcome> {
    let db = &core.db;
    let mut outcome = ApplyOutcome::default();

    for op in ops {
        let id = op.entity_id.as_str();
        if op.entity_type == SNAPSHOT_ENTITY {
            if op.deleted {
                continue;
            }
            let mut st = load_profile_files_state(db, id)
                .await?
                .unwrap_or_else(|| ProfileFilesState::new(id));
            if st.head_hlc.as_ref().map(|h| *h >= op.hlc).unwrap_or(false) {
                continue;
            }
            let p: SnapshotPayload =
                serde_json::from_value(op.payload.clone()).unwrap_or_default();
            let Some(dir) = profile_dir(core, id).await? else {
                // No retry brings the row of a deleted profile back.
                if profile_deleted(db, id).await? {
                    continue;
                }
                // Profile row not here yet; the retry batch brings both.
                outcome.retry = Some(format!("profile {id} not known yet"));
                continue;
            };
            let running = browser.running.is_running(id).await;
            let both_changed = st.dirty || (st.head_hlc.is_none() && has_files(&dir));
            if running || both_changed {
                st.pending_manifest = p.manifest_blob;
                st.diverged = both_changed;
                st.head_hlc = Some(op.hlc.clone());
                save_profile_files_state(db, &st).await?;
                continue;
            }
            match fetch_and_apply(engine, sync, app, &dir, &p.manifest_blob, Some(op)).await? {
                Ok(manifest) => {
                    record_applied(&mut st, &manifest);
                    st.head_hlc = Some(op.hlc.clone());
                    save_profile_files_state(db, &st).await?;
                }
                Err(BlobMiss::Gone(reason)) => outcome.skipped.push(reason),
                Err(BlobMiss::Retry(reason)) => outcome.retry = Some(reason),
            }
        }
    }
    Ok(outcome)
}

/// The profile row went with a tombstone this device applied. Its snapshot
/// stays in the log of the device that took it and is never deleted there.
async fn profile_deleted(db: &sqlx::SqlitePool, profile_id: &str) -> CmdResult<bool> {
    Ok(load_row_state(db, "profile", profile_id)
        .await?
        .is_some_and(|st| st.deleted))
}

// ── Explicit actions (launch, conflict UI) ───────────────────────────────────

/// Apply the snapshot that waited while the browser ran. No-op without one.
pub async fn apply_pending(
    engine: &Engine,
    core: &Core,
    sync: &SyncManager,
    app: &AppHandle,
    profile_id: &str,
) -> CmdResult<()> {
    let db = &core.db;
    let Some(mut st) = load_profile_files_state(db, profile_id).await? else {
        return Ok(());
    };
    if st.pending_manifest.is_empty() {
        return Ok(());
    }
    let dir = profile_dir(core, profile_id)
        .await?
        .ok_or_else(|| AppError::not_found("Profile not found"))?;
    match fetch_and_apply(engine, sync, Some(app), &dir, &st.pending_manifest, None).await? {
        Ok(manifest) => {
            record_applied(&mut st, &manifest);
            save_profile_files_state(db, &st).await
        }
        Err(BlobMiss::Retry(reason) | BlobMiss::Gone(reason)) => Err(AppError::other(reason)),
    }
}

/// Conflict choice: replace local files with the remote snapshot.
pub async fn take_remote(
    engine: &Engine,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    app: &AppHandle,
    profile_id: &str,
) -> CmdResult<()> {
    if browser.running.is_running(profile_id).await {
        return Err(AppError::other("Stop the profile first"));
    }
    apply_pending(engine, core, sync, app, profile_id).await
}

/// Conflict choice: keep local files and publish them as the new snapshot.
pub async fn push_mine(core: &Core, profile_id: &str) -> CmdResult<()> {
    let db = &core.db;
    let Some(mut st) = load_profile_files_state(db, profile_id).await? else {
        return Ok(());
    };
    st.diverged = false;
    st.dirty = true;
    st.pending_manifest.clear();
    save_profile_files_state(db, &st).await
}

/// Blobs a snapshot op keeps alive: the manifest and every file in it.
pub async fn blob_refs(engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
    if op.entity_type != SNAPSHOT_ENTITY || op.deleted {
        return Ok(Vec::new());
    }
    let p: SnapshotPayload = serde_json::from_value(op.payload.clone()).unwrap_or_default();
    if p.manifest_blob.is_empty() {
        return Ok(Vec::new());
    }
    let raw = engine
        .get_blob(&p.manifest_blob)
        .await
        .map_err(AppError::other)?
        .ok_or_else(|| AppError::other(format!("manifest {} missing", p.manifest_blob)))?;
    let mut refs = vec![p.manifest_blob];
    refs.extend(
        Manifest::parse(&String::from_utf8_lossy(&raw))?
            .files
            .into_iter()
            .map(|f| f.blob),
    );
    Ok(refs)
}

/// Leases and snapshots of the files of profiles, with a handler of their
/// own. Leases go out with the other changes; the files after the apply,
/// with a push of their own, so a stuck upload cannot block the rest.
pub(crate) struct ProfileFiles;

#[async_trait::async_trait]
impl Handler for ProfileFiles {
    fn entities(&self) -> &'static [&'static str] {
        &[LEASE_ENTITY, SNAPSHOT_ENTITY]
    }

    fn state_tables(&self) -> &'static [&'static str] {
        &["sync_profile_files_state"]
    }

    /// Leases and snapshots are applied after the rows, in `late`.
    fn has_late(&self) -> bool {
        true
    }

    async fn collect(&self, cx: &mut Cycle<'_>) -> CmdResult<Collected> {
        let leases = collect_leases(cx.core, cx.clock).await?;
        let states = leases.states;
        Ok(Collected {
            ops: leases.ops,
            commit: Some(Box::new(move |db| {
                Box::pin(async move {
                    for st in &states {
                        save_profile_files_state(&db, st).await?;
                    }
                    Ok(())
                })
            })),
            ..Collected::default()
        })
    }

    /// Leases and Firefox profile files: upload local snapshots, apply remote ones.
    /// Returns the retry reason when a remote snapshot could not be applied yet.
    async fn late(&self, cx: &mut Cycle<'_>, pulled_ops: &[Op]) -> CmdResult<Option<String>> {
        let app = cx.host.app();
        let browser = cx
            .host
            .state::<BrowserState>()
            .ok_or_else(|| AppError::other("the browser is not set up"))?;
        let db = &cx.core.db;
        match apply_leases(cx.core, pulled_ops).await {
            Ok(_) => {
                if let Some(app) = app {
                    let _ = app.emit(EVENT_STATUS, ());
                }
            }
            Err(e) => {
                trace(app, "error", &format!("leases: {e}"));
                cx.warnings.push(format!("leases: {e}"));
            }
        }
        if !cx.config.profile_files {
            trace(app, "info", "off");
            return Ok(None);
        }
        progress(app, "profiles_up", 55, 0, 0, "");
        match collect_local_changes(cx.engine, cx.core, cx.sync, browser, cx.clock, app).await {
            Ok(files) => {
                let n = files.ops.len();
                trace(app, "info", &format!("upload {n} ops"));
                if !files.ops.is_empty() {
                    if let Err(e) = cx.engine.push(cx.local, files.ops).await {
                        if matches!(e, veydan_sync::SyncError::OwnLogCollision(_)) {
                            trace(app, "error", "own log collision, rotating device id");
                            return Err(veydan_sync_host::push_error(db, e).await);
                        }
                        trace(app, "error", &format!("upload: {e}"));
                        cx.warnings.push(format!("profiles upload: {e}"));
                    } else {
                        veydan_sync_host::state::save_own_state(db, cx.local, &cx.clock.last())
                            .await?;
                        for st in &files.states {
                            save_profile_files_state(db, st).await?;
                        }
                    }
                }
            }
            Err(e) => {
                trace(app, "error", &format!("upload: {e}"));
                cx.warnings.push(format!("profiles upload: {e}"));
            }
        }
        progress(app, "profiles_down", 78, 0, 0, "");
        match apply_remote(cx.engine, app, cx.core, cx.sync, browser, pulled_ops).await {
            Ok(o) => {
                for reason in &o.skipped {
                    trace(app, "warn", reason);
                    cx.warnings.push(reason.clone());
                }
                match &o.retry {
                    Some(reason) => trace(app, "retry", reason),
                    None => trace(app, "info", "download ok"),
                }
                Ok(o.retry)
            }
            Err(e) => {
                trace(app, "error", &format!("download: {e}"));
                cx.warnings.push(format!("profiles download: {e}"));
                Ok(None)
            }
        }
    }

    async fn blob_refs(&self, engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
        blob_refs(engine, op).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hlc() -> Hlc {
        Hlc {
            wall_ms: 1,
            counter: 0,
            device_id: "dev-a".into(),
        }
    }

    #[test]
    fn a_lease_of_4_0_7_is_read_and_written_alike() {
        let wire = json!({
            "device_id": "dev-a",
            "device_name": "Laptop",
            "since": "2026-09-30T10:00:00+00:00",
        });

        let read: LeasePayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(read.device_id, "dev-a");
        assert_eq!(read.device_name, "Laptop");
        assert_eq!(read.since, "2026-09-30T10:00:00+00:00");

        let op = lease_op("p1", hlc(), Some(read));
        assert_eq!(op.entity_type, "profile_lease");
        assert_eq!(op.entity_id, "p1");
        assert!(!op.deleted);
        assert_eq!(op.payload, wire);
    }

    #[test]
    fn a_lease_release_of_4_0_7_is_written_alike() {
        let op = lease_op("p1", hlc(), None);
        assert_eq!(op.entity_type, "profile_lease");
        assert!(op.deleted);
        assert_eq!(
            op.payload,
            json!({ "device_id": "", "device_name": "", "since": "" })
        );
    }

    #[test]
    fn a_snapshot_of_4_0_7_is_read_and_written_alike() {
        let wire = json!({
            "manifest_blob": "m1",
            "taken_at": "2026-09-30T10:00:00+00:00",
            "files": 2,
            "bytes": 4096,
        });

        let read: SnapshotPayload = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(read.manifest_blob, "m1");
        assert_eq!(read.taken_at, "2026-09-30T10:00:00+00:00");
        assert_eq!(read.files, 2);
        assert_eq!(read.bytes, 4096);

        let op = snapshot_op("p1", hlc(), read);
        assert_eq!(op.entity_type, "profile_snapshot");
        assert_eq!(op.entity_id, "p1");
        assert!(!op.deleted);
        assert_eq!(op.payload, wire);
    }

    /// Every lease and snapshot op 4.0.7 wrote to a vault reads into the
    /// payload and comes out of it as 4.0.7 wrote it.
    #[test]
    fn every_op_of_profile_files_4_0_7_wrote_is_read_and_written_alike() {
        let leases = crate::sync::golden::ops(LEASE_ENTITY);
        assert!(leases.iter().any(|op| op.deleted) && leases.iter().any(|op| !op.deleted));
        let snapshots = crate::sync::golden::ops(SNAPSHOT_ENTITY);
        assert!(!snapshots.is_empty());
        for theirs in leases.into_iter().chain(snapshots) {
            let id = theirs.entity_id.as_str();
            let hlc = theirs.hlc.clone();
            let ours = match (theirs.entity_type.as_str(), theirs.deleted) {
                (LEASE_ENTITY, true) => lease_op(id, hlc, None),
                (LEASE_ENTITY, false) => {
                    lease_op(id, hlc, serde_json::from_value(theirs.payload.clone()).ok())
                }
                _ => snapshot_op(
                    id,
                    hlc,
                    serde_json::from_value(theirs.payload.clone()).unwrap(),
                ),
            };
            assert_eq!(
                serde_json::to_value(&ours).unwrap(),
                serde_json::to_value(&theirs).unwrap()
            );
        }
    }

    #[test]
    fn a_payload_with_fields_missing_or_unknown_is_read() {
        let lease: LeasePayload =
            serde_json::from_value(json!({ "device_id": "dev-a", "added_later": true })).unwrap();
        assert_eq!(lease.device_id, "dev-a");
        assert!(lease.since.is_empty());

        let snapshot: SnapshotPayload =
            serde_json::from_value(json!({ "manifest_blob": "m1", "added_later": 1 })).unwrap();
        assert_eq!(snapshot.manifest_blob, "m1");
        assert_eq!(snapshot.files, 0);
    }

    /// The manifest is a blob of the vault: its text is part of the format.
    #[test]
    fn a_manifest_of_4_0_7_is_read_and_written_alike() {
        let wire = r#"{"files":[{"path":"places.sqlite","blob":"b1","size":10,"hash":"h1"},{"path":"storage/default/x","blob":"b2","size":20,"hash":"h2"}]}"#;

        let manifest = Manifest::parse(wire).unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files[1].path, "storage/default/x");
        assert_eq!(manifest.files[1].blob, "b2");
        assert_eq!(manifest.files[1].size, 20);
        assert_eq!(manifest.files[1].hash, "h2");
        assert_eq!(manifest.blob_by_hash().get("h1"), Some(&"b1"));

        assert_eq!(manifest.to_json(), wire);
        assert!(Manifest::parse("not a manifest").is_err());
    }

    /// A snapshot outlives its profile in the vault. It waits for a profile
    /// that has not arrived, and for nothing once the profile was deleted.
    #[tokio::test]
    async fn a_snapshot_of_a_deleted_profile_is_not_waited_for() {
        use crate::modules::{TestStates, SYNC_PLAN};
        use veydan_sync_host::rows::apply_ops;

        let state = TestStates::new().await;
        let profile = |wall_ms: u64, payload: Option<serde_json::Value>| Op {
            entity_type: "profile".into(),
            entity_id: "pr-1".into(),
            hlc: Hlc {
                wall_ms,
                counter: 0,
                device_id: "dev-a".into(),
            },
            deleted: payload.is_none(),
            payload: payload.unwrap_or_else(|| json!({})),
        };
        assert!(profile_dir(&state.core, "pr-1").await.unwrap().is_none());
        assert!(!profile_deleted(&state.core.db, "pr-1").await.unwrap());

        let row = json!({
            "name": "Brand A", "browser_type": "camoufox", "fingerprint_preset": "windows",
            "created_at": "2026-09-01T10:00:00+00:00",
        });
        let ops = [profile(1000, Some(row)), profile(2000, None)];
        let tables = state.sync.registry().step_rows(SYNC_PLAN.apply, "app rows");
        apply_ops(&state.host(), &state.core, &tables, &ops)
            .await
            .unwrap();

        assert!(profile_dir(&state.core, "pr-1").await.unwrap().is_none());
        assert!(profile_deleted(&state.core.db, "pr-1").await.unwrap());

        state.core.db.close().await;
        let _ = std::fs::remove_dir_all(&state.core.app_data_dir);
    }
}

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
//! A deleted profile leaves a tombstone of its snapshot (an empty payload),
//! so its blobs can be collected.

use super::state::{
    delete_profile_files_state, load_profile_files_state, load_profile_files_states,
    save_profile_files_state, ProfileFilesState,
};
use super::SyncManager;
use crate::browser::{BrowserState, LaunchWaits};
use crate::commands::profiles::is_blacklisted;
use crate::error::{AppError, CmdResult};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use veydan_core::Core;
use veydan_sync::{large_files, sha256_hex, Engine, Hlc, HlcClock, Op};
use veydan_sync_host::state::load_row_state;
use veydan_sync_host::{Collected, Cycle, Handler, HashCache, EVENT_STATUS};

pub const LEASE_ENTITY: &str = "profile_lease";
pub const SNAPSHOT_ENTITY: &str = "profile_snapshot";
/// Local: "1" while the snapshots of the vault were read with the switch of
/// the files on, "0" once a cycle read past them with it off. A switch on
/// after "0", or without the key (a build before it), has them read again.
const READ_ON_KEY: &str = "sync_profile_files_read";

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

/// Where the files of a profile stand that are not out from here, or not
/// here: for its badge (`profiles_files_status`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilesStatus {
    pub profile_id: String,
    /// `sending` (`done` of `total` files walked), `open` (its browser runs
    /// and no copy went out since it started), `failed` (`detail` says
    /// why), `elsewhere` (no files here, `detail` names the device that
    /// holds the profile).
    pub state: &'static str,
    pub done: u32,
    pub total: u32,
    pub detail: String,
}

/// The profiles of this device whose files are not out, as the last
/// collect left them. Kept while the app runs; one sent leaves it.
#[derive(Default)]
pub struct UnsentFiles(std::sync::Mutex<HashMap<String, FilesStatus>>);

impl UnsentFiles {
    fn set(&self, id: &str, state: &'static str, detail: String) {
        if let Ok(mut all) = self.0.lock() {
            all.insert(
                id.to_string(),
                FilesStatus {
                    profile_id: id.to_string(),
                    state,
                    done: 0,
                    total: 0,
                    detail,
                },
            );
        }
    }

    fn sending(&self, id: &str, done: u32, total: u32) {
        if let Ok(mut all) = self.0.lock() {
            all.insert(
                id.to_string(),
                FilesStatus {
                    profile_id: id.to_string(),
                    state: "sending",
                    done,
                    total,
                    detail: String::new(),
                },
            );
        }
    }

    fn sent(&self, id: &str) {
        if let Ok(mut all) = self.0.lock() {
            all.remove(id);
        }
    }

    /// A profile that is not looked at any more (gone, another device's).
    fn keep_only(&self, ids: &HashSet<String>) {
        if let Ok(mut all) = self.0.lock() {
            all.retain(|id, _| ids.contains(id));
        }
    }

    fn list(&self) -> Vec<FilesStatus> {
        self.0
            .lock()
            .map(|all| all.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// The files of the profiles not out from here, and those of profiles
/// another device holds that have none here: what the badges show.
pub async fn files_status(core: &Core, unsent: &UnsentFiles) -> CmdResult<Vec<FilesStatus>> {
    let mut out = unsent.list();
    let device = super::config::device_id(&core.db).await?;
    for st in load_profile_files_states(&core.db).await?.into_values() {
        if st.lease_device.is_empty() || st.lease_device == device {
            continue;
        }
        if no_files_here(core, &st).await? {
            out.push(FilesStatus {
                profile_id: st.profile_id.clone(),
                state: "elsewhere",
                done: 0,
                total: 0,
                detail: st.lease_name.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.profile_id.cmp(&b.profile_id));
    Ok(out)
}

/// Its folder holds no files: it would start empty. A snapshot known here
/// (`head_hlc`, `pending_manifest`) is not one whose files came: its
/// download may have failed before any was written.
async fn no_files_here(core: &Core, st: &ProfileFilesState) -> CmdResult<bool> {
    Ok(match profile_dir(core, &st.profile_id).await? {
        Some(dir) => !has_files(&dir),
        None => false,
    })
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

/// `firefox-profile/` of a known profile and when it was last launched.
async fn profile_row(
    core: &Core,
    profile_id: &str,
) -> CmdResult<Option<(PathBuf, Option<String>)>> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT profile_path, last_launch_at FROM profiles WHERE id = ?")
            .bind(profile_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?;
    Ok(row.map(|(p, launched)| (PathBuf::from(p).join("firefox-profile"), launched)))
}

/// The browser ran since the files were last snapshotted or applied.
fn launched_after(last_launch_at: Option<&str>, snapshot_at: &str) -> bool {
    match (last_launch_at.and_then(parse_time), parse_time(snapshot_at)) {
        (Some(launch), Some(snap)) => launch > snap,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The files changed here since the head: marked dirty, the browser ran
/// since, or files lie in a folder no snapshot was taken of or applied to.
/// The collect sends such files; the apply of a remote snapshot over them
/// makes the profile `diverged` instead of laying the snapshot over them.
fn locally_changed(st: &ProfileFilesState, last_launch_at: Option<&str>, dir: &Path) -> bool {
    st.dirty
        || launched_after(last_launch_at, &st.snapshot_at)
        || (st.head_hlc.is_none() && has_files(dir))
}

/// Every syncable file below `dir` as (relative path, absolute path).
///
/// Any error of the walk fails it: a list short of an unreadable folder or
/// entry would go out as the whole profile, and the other devices would
/// delete what it lacks. A name the manifest cannot carry (`valid_rel_path`:
/// not UTF-8, a backslash, a colon, a name Windows reserves) is left out:
/// it is neither published nor, when a manifest is applied, deleted.
fn scan(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let Some(name) = e.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if is_blacklisted(&name) || name.ends_with(".tmp") {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if !valid_rel_path(&rel) {
                continue;
            }
            let ty = e.file_type()?;
            if ty.is_dir() {
                walk(&e.path(), &rel, out)?;
            } else if ty.is_file() {
                out.push((rel, e.path()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
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

/// A folder that cannot be read may hold files: it counts as having them,
/// so that nothing is laid over it unasked.
fn has_files(dir: &Path) -> bool {
    dir.is_dir() && scan(dir).map_or(true, |files| !files.is_empty())
}

/// A manifest path may only descend, and only by names every desktop can
/// write: no absolute parts, no `..`, no blacklisted names, nothing Windows
/// reads as a drive, a stream or a device.
fn valid_rel_path(rel: &str) -> bool {
    !rel.is_empty() && rel.len() <= 1024 && rel.split('/').all(valid_component)
}

/// One name of a manifest path: exactly one normal component on this system.
/// `:` (a drive, `C:x`, or a stream, `a:b`), `\`, the characters Windows
/// forbids, a trailing dot or space and the device names (`CON`, `nul.txt`,
/// `COM1`) are refused on every system, so a file is synced alike everywhere.
fn valid_component(c: &str) -> bool {
    crate::fsname::valid_component(c) && !is_blacklisted(c)
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

/// The snapshot of a deleted profile: the deletion of 9.5, an empty payload.
/// It names no blob, so the garbage collection takes the manifest and the
/// files of the last snapshot; a reader of the payload alone finds an empty
/// `manifest_blob`, and 4.0.7 skips a deleted snapshot op.
fn snapshot_tombstone(profile_id: &str, hlc: Hlc) -> Op {
    Op {
        entity_type: SNAPSHOT_ENTITY.into(),
        entity_id: profile_id.into(),
        hlc,
        deleted: true,
        payload: serde_json::json!({}),
    }
}

/// A lease of this device for a profile that is gone has to be released in
/// the vault: ours there, or a change of it not sent yet.
fn holds_lease(st: &ProfileFilesState, device: &str) -> bool {
    st.lease_device == device || !st.lease_synced
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

/// No files of the profile came here: a launch would start it empty.
pub async fn starts_empty(core: &Core, profile_id: &str) -> CmdResult<bool> {
    let st = load_profile_files_state(&core.db, profile_id)
        .await?
        .unwrap_or_else(|| ProfileFilesState::new(profile_id));
    no_files_here(core, &st).await
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

/// The browser exited: files changed. Our lease stays until they are out
/// (`collect_local_changes`), or till the next cycle when profile files
/// are not synced (`collect_leases`): the other devices see the profile
/// held here and do not start it on files that are still on their way.
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
    save_profile_files_state(db, &st).await
}

/// Takes the lease of a profile whose browser runs here and no device
/// holds: one started before sync was on, or whose state went with a new
/// binding of the vault. The op that publishes it.
fn take_lease(st: &mut ProfileFilesState, device: &str, name: &str, clock: &mut HlcClock) -> Op {
    st.lease_device = device.to_string();
    st.lease_name = name.to_string();
    st.lease_since = Utc::now().to_rfc3339();
    let hlc = clock.now();
    st.lease_hlc = Some(hlc.clone());
    st.lease_synced = true;
    lease_op(
        &st.profile_id,
        hlc,
        Some(LeasePayload {
            device_id: device.to_string(),
            device_name: st.lease_name.clone(),
            since: st.lease_since.clone(),
        }),
    )
}

/// Lets our lease go; the op that publishes the release.
fn release_lease(st: &mut ProfileFilesState, clock: &mut HlcClock) -> Op {
    st.lease_device.clear();
    st.lease_name.clear();
    st.lease_since.clear();
    let hlc = clock.now();
    st.lease_hlc = Some(hlc.clone());
    st.lease_synced = true;
    lease_op(&st.profile_id, hlc, None)
}

/// A remote snapshot waits for this profile and can be applied now.
pub async fn has_pending(core: &Core, profile_id: &str) -> CmdResult<bool> {
    Ok(load_profile_files_state(&core.db, profile_id)
        .await?
        .map(|s| !s.pending_manifest.is_empty() && !s.diverged)
        .unwrap_or(false))
}

// ── Push ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LocalChanges {
    pub ops: Vec<Op>,
    pub states: Vec<ProfileFilesState>,
    /// Profiles that sent nothing this cycle, and why; they stay `dirty`.
    pub warnings: Vec<String>,
    /// Profiles gone here whose tombstones are among `ops`: their state is
    /// deleted once the ops are out.
    pub gone: Vec<String>,
    /// The locks of the profiles in `states` by profile id, held until
    /// they are saved.
    pub held: Vec<(String, tokio::sync::OwnedMutexGuard<()>)>,
}

/// Names uploaded in this cycle; skip a second PUT of the same content.
/// A blob found in the vault or written is touched, so a GC pass of this
/// device keeps it while the snapshot that names it is not out yet.
struct BlobIndex<'a> {
    engine: &'a Engine,
    uploaded: BTreeSet<String>,
}

impl<'a> BlobIndex<'a> {
    async fn exists(&self, name: &str) -> CmdResult<bool> {
        if self.uploaded.contains(name) {
            return Ok(true);
        }
        let found = self
            .engine
            .blob_exists(name)
            .await
            .map_err(AppError::other)?;
        if found {
            large_files::touch(name);
        }
        Ok(found)
    }

    async fn put(&mut self, data: &[u8]) -> CmdResult<String> {
        let name = self.engine.blob_name(data);
        if self.uploaded.contains(&name) {
            return Ok(name);
        }
        self.engine.put_blob(data).await.map_err(AppError::other)?;
        large_files::touch(&name);
        self.uploaded.insert(name.clone());
        Ok(name)
    }

    /// The blob of a file read for a snapshot. A large one is looked for
    /// first: a collect cut short (a launch, the network, the app closed)
    /// sent it already, and the next one does not send it again.
    async fn upload(&mut self, data: &[u8]) -> CmdResult<String> {
        if data.len() as u64 >= LOOK_FIRST {
            let name = self.engine.blob_name(data);
            if self.exists(&name).await? {
                return Ok(name);
            }
        }
        self.put(data).await
    }

    /// Just before the snapshot goes out: a blob found or written longer
    /// than `RECHECK_AFTER` ago may have been taken by the GC of another
    /// device since (a walk of a large profile outlives its grace); one that
    /// is gone is uploaded again from its file. A blob the head names
    /// (`None` in `found`) is not looked at. The inner `Err` says why it
    /// cannot be: the file is gone or changed since. `None` when a launch
    /// waits for a profile of `held`.
    async fn recheck(
        &self,
        files: &[ManifestFile],
        found: &[Option<(PathBuf, Instant)>],
        held: &Held<'_>,
    ) -> CmdResult<Option<Result<(), String>>> {
        for (f, found) in files.iter().zip(found) {
            let Some((abs, at)) = found else {
                continue;
            };
            if at.elapsed() < RECHECK_AFTER {
                continue;
            }
            let Some(exists) = held.unless_wanted(self.engine.blob_exists(&f.blob)).await else {
                return Ok(None);
            };
            if exists.map_err(AppError::other)? {
                large_files::touch(&f.blob);
                continue;
            }
            let data = match std::fs::read(abs) {
                Ok(data) => data,
                Err(e) => return Ok(Some(Err(format!("{}: {e}", f.path)))),
            };
            if self.engine.blob_name(&data) != f.blob {
                return Ok(Some(Err(format!(
                    "{}: changed while the snapshot was taken",
                    f.path
                ))));
            }
            let Some(put) = held.unless_wanted(self.engine.put_blob(&data)).await else {
                return Ok(None);
            };
            put.map_err(AppError::other)?;
            large_files::touch(&f.blob);
        }
        Ok(Some(Ok(())))
    }
}

/// The profiles whose files a collect holds: the one it reads and those
/// whose snapshots wait to go out. A launch that waits for one of them
/// stops the reading, which may upload for minutes.
struct Held<'a> {
    waits: &'a LaunchWaits,
    ids: Vec<String>,
}

impl Held<'_> {
    fn wanted(&self) -> bool {
        self.waits.waits_for_any(&self.ids)
    }

    /// `fut`, or `None` (and `fut` dropped) once a launch waits for one of
    /// the profiles. A blob cut off on its way up is not in the vault: an
    /// upload is whole or none.
    async fn unless_wanted<T>(&self, fut: impl std::future::Future<Output = T>) -> Option<T> {
        tokio::select! {
            biased;
            _ = self.waits.until_any(&self.ids) => None,
            v = fut => Some(v),
        }
    }
}

/// Gives up the snapshots waiting to go out of the profiles a launch waits
/// for: their locks go, their files stay changed (the state is not saved)
/// and go out with a later cycle, the release of our lease with them.
/// Their blobs are left to the GC.
fn give_up_wanted(out: &mut LocalChanges, waits: &LaunchWaits, app: Option<&AppHandle>) {
    let wanted: Vec<String> = out
        .held
        .iter()
        .filter(|(id, _)| waits.waits_for(id))
        .map(|(id, _)| id.clone())
        .collect();
    for id in wanted {
        // The release of our lease too: it goes with the files.
        out.ops.retain(|op| op.entity_id != id);
        out.states.retain(|st| st.profile_id != id);
        out.held.retain(|(held, _)| *held != id);
        trace_profile(app, &id, "a launch waits, snapshot later");
    }
}

/// A blob found in the vault longer ago than this is looked at again before
/// the snapshot that names it goes out.
const RECHECK_AFTER: Duration = Duration::from_secs(60);

/// A file this large is looked for in the vault before it is sent.
const LOOK_FIRST: u64 = 64 * 1024;

/// A profile whose browser runs sends a copy of its files at most this
/// often: the files change all the time, and a whole profile goes up
/// again for every database written since.
const LIVE_EVERY: Duration = Duration::from_secs(30 * 60);

/// Reads of the files of a database before the copy of a running profile
/// gives up on them for this time.
const STABLE_TRIES: usize = 3;

/// The journals SQLite keeps beside a database. Read apart from it, the
/// database and its write-ahead log may come from either side of a
/// checkpoint. The `-shm` index is not one: SQLite builds it again, and a
/// running browser writes it at every read.
const JOURNALS: [&str; 2] = ["-wal", "-journal"];

/// The database a file of a running profile is read with, if it is one
/// of a database or the database itself.
fn sqlite_base(rel: &str) -> Option<&str> {
    JOURNALS
        .iter()
        .find_map(|j| rel.strip_suffix(j))
        .or_else(|| (rel.ends_with(".sqlite") || rel.ends_with(".db")).then_some(rel))
}

/// Files as (relative path, content).
type DbFiles = Vec<(String, Vec<u8>)>;

/// The database `base` and its journals as (relative path, content), read
/// twice alike: what a running browser wrote between the reads would show.
/// `None` when every read differed from the one before. A file absent
/// from a read is left out of it.
fn read_stable(dir: &Path, base: &str) -> std::io::Result<Option<DbFiles>> {
    let names: Vec<String> = std::iter::once(base.to_string())
        .chain(JOURNALS.iter().map(|j| format!("{base}{j}")))
        .collect();
    let read = || -> std::io::Result<DbFiles> {
        let mut out = Vec::new();
        for name in &names {
            match std::fs::read(dir.join(name)) {
                Ok(data) => out.push((name.clone(), data)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    };
    let mut last = read()?;
    for _ in 0..STABLE_TRIES {
        let again = read()?;
        if again == last {
            return Ok(Some(again));
        }
        last = again;
    }
    Ok(None)
}

/// A copy of the files of a running profile is due: none was taken or
/// applied since `LIVE_EVERY`, or ever.
fn live_due(st: &ProfileFilesState) -> bool {
    let every = chrono::Duration::from_std(LIVE_EVERY).unwrap_or_default();
    st.head_hlc.is_none() || parse_time(&st.snapshot_at).is_none_or(|at| Utc::now() - at >= every)
}

/// The last GC pass found a blob of a snapshot this device holds gone (the
/// GC of another device took it between its mark and the reference): the
/// profile is collected again and goes out as a new snapshot, for which
/// `build_manifest` uploads what is gone. Only where the files are that
/// snapshot: its HLC is the head, nothing waits to be applied.
async fn heal_missing(db: &sqlx::SqlitePool) -> CmdResult<()> {
    let missing = veydan_sync_host::gc::missing_refs(db).await;
    for at in missing.iter().filter(|m| m.entity_type == SNAPSHOT_ENTITY) {
        let Some(mut st) = load_profile_files_state(db, &at.entity_id).await? else {
            continue;
        };
        let healing = st.dirty && st.synced_hash.is_empty();
        if st.head_hlc.as_ref() != Some(&at.hlc)
            || st.diverged
            || !st.pending_manifest.is_empty()
            || healing
        {
            continue;
        }
        st.dirty = true;
        st.synced_hash.clear();
        save_profile_files_state(db, &st).await?;
    }
    Ok(())
}

/// Manifest of the directory; only files whose hash is new are read and uploaded.
/// `trusted`: `prev` is the head in the vault, so its blobs are there (the
/// GC keeps what the head names; one it took anyway is told by
/// `heal_missing`, which clears the trust): a file of the same hash takes
/// its blob without a request to the vault. Otherwise the blob is looked
/// for first. The inner `Err` says why the files cannot be read: a file gone since the
/// walk is left out, any other error of a folder or a file gives no manifest.
/// A file is listed with the hash of what was read of it and sent.
///
/// `live`: the browser of the profile runs and writes its files meanwhile.
/// A database is read with its journals until two reads agree
/// (`read_stable`); one that keeps changing, or a file that cannot be
/// read (Windows locks some), keeps what `prev` has of it, and is left out
/// when `prev` has nothing: a copy of a running profile is what a browser
/// that crashed leaves, which Firefox opens. Without a copy of a database
/// the manifest is not made. `None` when a launch waits for a profile of
/// `held`. `sending` is told how far the walk came.
#[allow(clippy::too_many_arguments)]
async fn build_manifest(
    cache: &HashCache,
    blobs: &mut BlobIndex<'_>,
    dir: &Path,
    prev: &Manifest,
    trusted: bool,
    live: bool,
    app: Option<&AppHandle>,
    held: &Held<'_>,
    sending: &(dyn Fn(u32, u32) + Sync),
) -> CmdResult<Option<Result<Manifest, String>>> {
    let known = prev.blob_by_hash();
    let entries = match scan(dir) {
        Ok(entries) => entries,
        Err(e) => return Ok(Some(Err(format!("{}: {e}", dir.display())))),
    };
    let total = entries.len() as u32;
    let mut files = Vec::new();
    let mut found = Vec::new();
    // The databases of a live copy read so far.
    let mut databases: HashSet<String> = HashSet::new();
    for (i, (rel, abs)) in entries.into_iter().enumerate() {
        if held.wanted() {
            return Ok(None);
        }
        let current = i as u32 + 1;
        progress(
            app,
            "profiles_up",
            super::progress_pct(78, 93, current, total.max(1)),
            current,
            total,
            &rel,
        );
        if current % 64 == 1 || current == total {
            sending(current, total);
        }
        if live && rel.ends_with("-shm") {
            continue;
        }
        if let Some(base) = sqlite_base(&rel).filter(|_| live) {
            if !databases.insert(base.to_string()) {
                continue;
            }
            let read = match read_stable(dir, base) {
                Ok(Some(read)) => Ok(read),
                Ok(None) => Err("changes while it is read".to_string()),
                Err(e) => Err(e.to_string()),
            };
            let read = match read {
                Ok(read) => read,
                Err(why) => {
                    let last: Vec<&ManifestFile> = prev
                        .files
                        .iter()
                        .filter(|f| sqlite_base(&f.path) == Some(base) && !f.path.ends_with("-shm"))
                        .collect();
                    if last.is_empty() {
                        return Ok(Some(Err(format!("{base}: {why}"))));
                    }
                    for f in last {
                        match held.unless_wanted(keep_last(blobs, f, trusted)).await {
                            Some(Ok(true)) => {}
                            Some(Err(e)) => return Err(e),
                            Some(Ok(false)) => {
                                return Ok(Some(Err(format!(
                                    "{base}: {why}, its last copy is gone"
                                ))))
                            }
                            None => return Ok(None),
                        }
                        found.push(None);
                        files.push(f.clone());
                    }
                    continue;
                }
            };
            for (path, data) in read {
                let hash = sha256_hex(&data);
                let size = data.len() as u64;
                let blob = match known.get(hash.as_str()).filter(|_| trusted) {
                    Some(b) => {
                        large_files::touch(b);
                        found.push(None);
                        (*b).to_string()
                    }
                    None => match held.unless_wanted(blobs.upload(&data)).await {
                        Some(put) => {
                            found.push(Some((dir.join(&path), Instant::now())));
                            put?
                        }
                        None => return Ok(None),
                    },
                };
                files.push(ManifestFile {
                    path,
                    blob,
                    size,
                    hash,
                });
            }
            continue;
        }
        let read = cache
            .try_file_hash(&abs)
            .and_then(|hash| std::fs::metadata(&abs).map(|m| (hash, m.len())));
        let (hash, size) = match read {
            Ok(found) => found,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) if live => {
                if let Some(f) = prev.files.iter().find(|f| f.path == rel) {
                    match held.unless_wanted(keep_last(blobs, f, trusted)).await {
                        Some(Ok(true)) => {
                            found.push(None);
                            files.push(f.clone());
                        }
                        Some(Ok(false)) => {}
                        Some(Err(e)) => return Err(e),
                        None => return Ok(None),
                    }
                }
                trace(app, "info", &format!("{rel}: {e}, the last copy kept"));
                continue;
            }
            Err(e) => return Ok(Some(Err(format!("{rel}: {e}")))),
        };
        if let Some(b) = known.get(hash.as_str()).filter(|_| trusted) {
            large_files::touch(b);
            found.push(None);
            files.push(ManifestFile {
                path: rel,
                blob: (*b).to_string(),
                size,
                hash,
            });
            continue;
        }
        // A name from a manifest that is not the head is reused only while
        // the blob is still in the vault; GC may have dropped it since.
        let reusable = match known.get(hash.as_str()) {
            Some(b) => match held.unless_wanted(blobs.exists(b)).await {
                Some(exists) => exists?.then(|| (*b).to_string()),
                None => return Ok(None),
            },
            None => None,
        };
        if let Some(blob) = reusable {
            found.push(Some((abs, Instant::now())));
            files.push(ManifestFile {
                path: rel,
                blob,
                size,
                hash,
            });
            continue;
        }
        let data = match std::fs::read(&abs) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Ok(Some(Err(format!("{rel}: {e}")))),
        };
        let blob = match held.unless_wanted(blobs.upload(&data)).await {
            Some(put) => put?,
            None => return Ok(None),
        };
        found.push(Some((abs, Instant::now())));
        files.push(ManifestFile {
            path: rel,
            blob,
            size: data.len() as u64,
            hash: sha256_hex(&data),
        });
    }
    // The files of a database go in the order of the walk, as the others.
    let mut listed: Vec<(ManifestFile, Option<(PathBuf, Instant)>)> =
        files.into_iter().zip(found).collect();
    listed.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    let (files, found): (Vec<_>, Vec<_>) = listed.into_iter().unzip();
    match blobs.recheck(&files, &found, held).await? {
        None => Ok(None),
        Some(Err(reason)) => Ok(Some(Err(reason))),
        Some(Ok(())) => Ok(Some(Ok(Manifest { files }))),
    }
}

/// A file of the last manifest can be taken again as it was: its blob is
/// there when the manifest is the head (`trusted`), else it is looked for.
async fn keep_last(blobs: &BlobIndex<'_>, f: &ManifestFile, trusted: bool) -> CmdResult<bool> {
    if trusted {
        large_files::touch(&f.blob);
        return Ok(true);
    }
    blobs.exists(&f.blob).await
}

/// Lease ops only: a tiny payload, never waits on profile files. A
/// profile that runs here and no device holds is taken (`take_lease`);
/// our lease of one that stopped (`dirty`) goes here only when profile
/// files are not synced (`files`), else with its files
/// (`collect_local_changes`). `running`: the browsers that run here,
/// `launching` those that start.
pub async fn collect_leases(
    core: &Core,
    clock: &mut HlcClock,
    running: &HashSet<String>,
    launching: &HashSet<String>,
    files: bool,
) -> CmdResult<LocalChanges> {
    let db = &core.db;
    let device = super::config::device_id(db).await?;
    let name = super::config::device_name(db).await;
    let mut out = LocalChanges::default();
    let rows: Vec<(String,)> = sqlx::query_as("SELECT id FROM profiles")
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    for (id,) in rows {
        let mut st = load_profile_files_state(db, &id)
            .await?
            .unwrap_or_else(|| ProfileFilesState::new(&id));
        let runs = running.contains(&id);
        if runs && st.lease_device.is_empty() {
            out.ops.push(take_lease(&mut st, &device, &name, clock));
            out.states.push(st);
            continue;
        }
        if !runs && !launching.contains(&id) && !files && st.dirty && st.lease_device == device {
            out.ops.push(release_lease(&mut st, clock));
            out.states.push(st);
            continue;
        }
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
    heal_missing(db).await?;
    let device = super::config::device_id(db).await?;
    let mut out = LocalChanges::default();
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
    // A launch goes first: a profile it waits for is given up, read or
    // waiting to go out, and sent by a later cycle.
    let waits = browser.files.launch_waits();
    let mut looked = HashSet::new();

    for (id, profile_path, last_launch_at) in rows {
        states.remove(&id);
        looked.insert(id.clone());
        // Held until the snapshot is out and its state saved (or dropped
        // with `out` when the push fails): no launch, stop or apply of the
        // profile comes between the files read and the state written. The
        // wait for it (a launch of this profile may hold it for minutes)
        // gives up the profiles held meanwhile that a launch waits for.
        let files = loop {
            give_up_wanted(&mut out, &waits, app);
            let held: Vec<String> = out.held.iter().map(|(held, _)| held.clone()).collect();
            tokio::select! {
                biased;
                files = browser.files.lock(&id) => break files,
                _ = waits.until_any(&held), if !held.is_empty() => {}
            }
        };
        let mut st = load_profile_files_state(db, &id)
            .await?
            .unwrap_or_else(|| ProfileFilesState::new(&id));

        let dir = PathBuf::from(&profile_path).join("firefox-profile");
        let candidate = locally_changed(&st, last_launch_at.as_deref(), &dir);
        let running = browser.running.is_running(&id).await;
        let own = st.lease_device == device;
        let foreign_lease = !st.lease_device.is_empty() && !own;
        // A browser that runs sends a copy of its files now and then
        // (`live_due`), not over a snapshot of another device that waits:
        // that one makes the profile diverge when the browser stops.
        let live = running && live_due(&st) && st.pending_manifest.is_empty();
        if !(candidate || live) || (running && !live) || st.diverged || foreign_lease {
            // Someone else is running it: our local changes cannot win, let the user decide.
            let now_diverged = foreign_lease && candidate && !running && st.dirty && !st.diverged;
            if now_diverged {
                st.diverged = true;
                save_profile_files_state(db, &st).await?;
            }
            let reason = if running && foreign_lease {
                "running, foreign lease"
            } else if running && !st.pending_manifest.is_empty() {
                "running, a snapshot waits"
            } else if running {
                "running, copy later"
            } else if foreign_lease {
                "foreign lease"
            } else if st.diverged {
                "diverged"
            } else {
                "unchanged"
            };
            trace_profile(app, &id, reason);
            // The badge of a profile that diverged or that another device
            // holds tells what there is to do.
            if running
                && !foreign_lease
                && launched_after(last_launch_at.as_deref(), &st.snapshot_at)
            {
                browser.unsent.set(&id, "open", String::new());
            } else {
                browser.unsent.sent(&id);
            }
            // Our lease of a profile whose browser stopped and whose files
            // diverged goes: the user picks a side. One whose files are out
            // went with them; one taken without a browser since (a launch
            // that failed stops it, and the files go) stays.
            if own && !running && candidate && st.diverged {
                out.ops.push(release_lease(&mut st, clock));
                out.states.push(st);
                out.held.push((id.clone(), files));
            }
            continue;
        }

        // Local bookkeeping only; empty before the first snapshot.
        let prev = Manifest::parse(&st.manifest_json).unwrap_or_default();
        let held = Held {
            waits: &waits,
            ids: out
                .held
                .iter()
                .map(|(held, _)| held.clone())
                .chain([id.clone()])
                .collect(),
        };
        // Files that cannot be read, or none where the last snapshot had
        // some (the folder gone, a disk not mounted), send nothing: the
        // others would delete their copies. The profile stays dirty.
        // The manifest is the head here and in the vault: what it was
        // pushed or applied as, no other snapshot waits over it.
        let trusted = !st.synced_hash.is_empty()
            && st.pending_manifest.is_empty()
            && sha256_hex(st.manifest_json.as_bytes()) == st.synced_hash;
        progress(app, "profiles_up", 78, 0, 0, "");
        let sending = |done, total| browser.unsent.sending(&id, done, total);
        let built = build_manifest(
            &sync.file_hashes,
            &mut blobs,
            &dir,
            &prev,
            trusted,
            live,
            app,
            &held,
            &sending,
        )
        .await?;
        let manifest = match built {
            // A launch waits for this profile or one held: this one waits
            // for a later cycle as it is, so does the other.
            None => {
                trace_profile(app, &id, "a launch waits, snapshot later");
                drop(files);
                give_up_wanted(&mut out, &waits, app);
                continue;
            }
            Some(Ok(m)) if m.files.is_empty() && !prev.files.is_empty() => Err(format!(
                "{} holds no files, the last snapshot {}",
                dir.display(),
                prev.files.len()
            )),
            Some(found) => found,
        };
        let manifest = match manifest {
            Ok(m) => m,
            // The copy of a running browser is tried again next cycle; the
            // files go out whole when it stops.
            Err(reason) if live => {
                trace_profile(app, &id, &format!("running, no copy: {reason}"));
                browser.unsent.set(&id, "open", reason);
                continue;
            }
            Err(reason) => {
                let short: String = id.chars().take(8).collect();
                let warning = format!("profile {short}: nothing sent, {reason}");
                trace(app, "warn", &warning);
                out.warnings.push(warning);
                browser.unsent.set(&id, "failed", reason);
                if !st.dirty {
                    st.dirty = true;
                    save_profile_files_state(db, &st).await?;
                }
                continue;
            }
        };
        let json = manifest.to_json();
        let hash = sha256_hex(json.as_bytes());
        let now = Utc::now().to_rfc3339();
        if hash == st.synced_hash {
            st.dirty = false;
            st.snapshot_at = now;
            trace_profile(app, &id, "hash unchanged");
            // The release of our lease goes with the state: a push that
            // fails, or a launch that wants the profile, leaves it dirty,
            // and a later cycle sends the release again.
            if own && !running {
                out.ops.push(release_lease(&mut st, clock));
                out.states.push(st);
                out.held.push((id.clone(), files));
            } else {
                save_profile_files_state(db, &st).await?;
                browser.unsent.sent(&id);
            }
            continue;
        }
        let manifest_blob = blobs.put(json.as_bytes()).await?;
        let hlc = clock.now();
        let files_n = manifest.files.len();
        let bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
        out.ops.push(snapshot_op(
            &id,
            hlc.clone(),
            SnapshotPayload {
                manifest_blob,
                taken_at: now.clone(),
                files: files_n as u64,
                bytes,
            },
        ));
        st.head_hlc = Some(hlc);
        st.synced_hash = hash;
        st.manifest_json = json;
        st.snapshot_at = now;
        st.dirty = false;
        st.pending_manifest.clear();
        // The files are out with the release of our lease: no device sees
        // the profile free before they can be applied there.
        if own && !running {
            out.ops.push(release_lease(&mut st, clock));
        }
        out.states.push(st);
        out.held.push((id.clone(), files));
        trace_profile(
            app,
            &id,
            &format!(
                "{} {files_n} files, {bytes} bytes",
                if live {
                    "copy of the running browser,"
                } else {
                    "snapshot"
                }
            ),
        );
    }
    browser.unsent.keep_only(&looked);
    give_up_wanted(&mut out, &waits, app);
    // A profile deleted here, or by a tombstone of its row, whose snapshot
    // or lease of this device is in the vault: the snapshot gets a
    // tombstone (the garbage collection then takes its blobs, and a device
    // that joins later does not wait for its row), our lease a release.
    // The state goes after the push. Any other leaves no state behind.
    for (id, st) in states {
        let mut gone = Vec::new();
        if st.head_hlc.is_some() {
            gone.push(snapshot_tombstone(&id, clock.now()));
        }
        if holds_lease(&st, &device) {
            gone.push(lease_op(&id, clock.now(), None));
        }
        if gone.is_empty() {
            delete_profile_files_state(db, &id).await?;
            continue;
        }
        trace_profile(app, &id, "deleted, tombstones");
        out.ops.extend(gone);
        out.gone.push(id);
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
    /// Snapshots that failed to apply and wait as `pending_manifest`, to be
    /// tried again next cycle. Reported as warnings; peer heads still advance.
    pub failed: Vec<String>,
    /// Profiles a snapshot op of this batch was for.
    pub seen: HashSet<String>,
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
            super::progress_pct(55, 75, current, total.max(1)),
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
    if let Err(e) = drop_unlisted(dir, &keep) {
        // Not applied: the next cycle brings the snapshot again.
        return Ok(Some(BlobMiss::Retry(format!(
            "profile files in {} cannot be listed: {e}",
            dir.display()
        ))));
    }
    Ok(None)
}

/// Delete the files below `dir` a manifest does not list. Nothing goes when
/// the folder cannot be walked whole, a name the manifest cannot carry is
/// never touched, and nothing outside `dir` once links are resolved.
fn drop_unlisted(dir: &Path, keep: &HashSet<&str>) -> std::io::Result<()> {
    let files = scan(dir)?;
    let root = std::fs::canonicalize(dir)?;
    for (rel, abs) in files {
        if keep.contains(rel.as_str()) || !valid_rel_path(&rel) {
            continue;
        }
        match std::fs::canonicalize(&abs) {
            Ok(real) if real.starts_with(&root) && real != root => {
                let _ = std::fs::remove_file(&abs);
            }
            _ => {}
        }
    }
    Ok(())
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
    let former = super::config::former_device_ids(db).await;
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
            // A lease of a former id of this device, read back from its old
            // log: the rotation moved it here or let it go.
            if former.contains(&op.hlc.device_id) {
                continue;
            }
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

/// Apply remote snapshot ops. Ops are already HLC-sorted; only the newest
/// op of each profile in the batch counts, the ones before it would be laid
/// over at once. Each profile under the lock of its files, and on its own:
/// one that fails waits as pending and the others go on. Without the app no
/// progress goes out.
pub async fn apply_remote(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    ops: &[Op],
) -> CmdResult<ApplyOutcome> {
    let mut outcome = ApplyOutcome::default();
    let mut newest: HashMap<&str, &Op> = HashMap::new();
    for op in ops.iter().filter(|op| op.entity_type == SNAPSHOT_ENTITY) {
        let slot = newest.entry(op.entity_id.as_str()).or_insert(op);
        if op.hlc > slot.hlc {
            *slot = op;
        }
    }
    for op in ops {
        let id = op.entity_id.as_str();
        if newest.get(id).is_none_or(|n| !std::ptr::eq(*n, op)) {
            continue;
        }
        outcome.seen.insert(id.to_string());
        let _files = browser.files.lock(id).await;
        if let Err(e) = apply_snapshot(engine, app, core, sync, browser, op, &mut outcome).await {
            let short: String = id.chars().take(8).collect();
            outcome
                .failed
                .push(format!("profile {short}: snapshot not applied, {e}"));
        }
    }
    Ok(outcome)
}

/// One snapshot op, under the lock of the profile's files.
async fn apply_snapshot(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    op: &Op,
    outcome: &mut ApplyOutcome,
) -> CmdResult<()> {
    let db = &core.db;
    let id = op.entity_id.as_str();
    let mut st = load_profile_files_state(db, id)
        .await?
        .unwrap_or_else(|| ProfileFilesState::new(id));
    if st.head_hlc.as_ref().map(|h| *h >= op.hlc).unwrap_or(false) {
        return Ok(());
    }
    let row = profile_row(core, id).await?;
    if op.deleted {
        // The profile was deleted where the tombstone comes from. Here its
        // row is gone with it: nothing to send for the snapshot, only a
        // lease of ours to release. A row still here (its browser runs)
        // gives a tombstone of its own once it goes.
        if row.is_none() {
            if holds_lease(&st, &super::config::device_id(db).await?) {
                st.head_hlc = None;
                save_profile_files_state(db, &st).await?;
            } else {
                delete_profile_files_state(db, id).await?;
            }
        }
        return Ok(());
    }
    let p: SnapshotPayload = serde_json::from_value(op.payload.clone()).unwrap_or_default();
    let Some((dir, last_launch_at)) = row else {
        // No retry brings the row of a deleted profile back.
        if profile_deleted(db, id).await? {
            return Ok(());
        }
        // Profile row not here yet; the retry batch brings both.
        outcome.retry = Some(format!("profile {id} not known yet"));
        return Ok(());
    };
    let running = browser.running.is_running(id).await;
    let changed = locally_changed(&st, last_launch_at.as_deref(), &dir);
    if running || changed {
        // A running browser's changes are judged when it stops
        // (`on_profile_stopped`); files changed here are the user's to pick.
        st.pending_manifest = p.manifest_blob;
        st.diverged = st.diverged || (changed && !running);
        st.head_hlc = Some(op.hlc.clone());
        save_profile_files_state(db, &st).await?;
        return Ok(());
    }
    match fetch_and_apply(engine, sync, app, &dir, &p.manifest_blob, Some(op)).await {
        Ok(Ok(manifest)) => {
            record_applied(&mut st, &manifest);
            st.head_hlc = Some(op.hlc.clone());
            save_profile_files_state(db, &st).await?;
        }
        Ok(Err(BlobMiss::Gone(reason))) => outcome.skipped.push(reason),
        Ok(Err(BlobMiss::Retry(reason))) => outcome.retry = Some(reason),
        Err(e) => {
            // Files may be half written: the snapshot is this profile's
            // head and waits, tried again every cycle (`retry_pending`)
            // and before a launch, without holding back the heads of peers.
            st.pending_manifest = p.manifest_blob;
            st.head_hlc = Some(op.hlc.clone());
            save_profile_files_state(db, &st).await?;
            return Err(e);
        }
    }
    Ok(())
}

/// Snapshots that wait as pending although nothing here changed — their
/// apply failed — are tried again: not for a profile that runs, that
/// diverged or whose op is in this batch (`skip`). A profile whose files
/// changed meanwhile diverges instead. Returns the warnings.
pub async fn retry_pending(
    engine: &Engine,
    app: Option<&AppHandle>,
    core: &Core,
    sync: &SyncManager,
    browser: &BrowserState,
    skip: &HashSet<String>,
) -> CmdResult<Vec<String>> {
    let db = &core.db;
    let mut warnings = Vec::new();
    let waiting: Vec<String> = load_profile_files_states(db)
        .await?
        .into_values()
        .filter(|st| !st.pending_manifest.is_empty() && !st.diverged)
        .map(|st| st.profile_id)
        .filter(|id| !skip.contains(id))
        .collect();
    for id in waiting {
        let _files = browser.files.lock(&id).await;
        let short: String = id.chars().take(8).collect();
        let Some(mut st) = load_profile_files_state(db, &id).await? else {
            continue;
        };
        if st.pending_manifest.is_empty() || st.diverged || browser.running.is_running(&id).await {
            continue;
        }
        let Some((dir, last_launch_at)) = profile_row(core, &id).await? else {
            continue;
        };
        if locally_changed(&st, last_launch_at.as_deref(), &dir) {
            st.diverged = true;
            save_profile_files_state(db, &st).await?;
            continue;
        }
        // The age of the op tells a blob still on its way from one taken.
        let head = st.head_hlc.clone().map(|hlc| Op {
            entity_type: SNAPSHOT_ENTITY.into(),
            entity_id: id.clone(),
            hlc,
            deleted: false,
            payload: serde_json::Value::Null,
        });
        let blob = st.pending_manifest.clone();
        match fetch_and_apply(engine, sync, app, &dir, &blob, head.as_ref()).await {
            Ok(Ok(manifest)) => {
                record_applied(&mut st, &manifest);
                save_profile_files_state(db, &st).await?;
                trace_profile(app, &id, "pending applied");
            }
            Ok(Err(BlobMiss::Gone(reason))) => {
                st.pending_manifest.clear();
                save_profile_files_state(db, &st).await?;
                warnings.push(format!("profile {short}: {reason}"));
            }
            Ok(Err(BlobMiss::Retry(reason))) => {
                warnings.push(format!("profile {short}: snapshot still waits, {reason}"));
            }
            Err(e) => warnings.push(format!("profile {short}: snapshot still waits, {e}")),
        }
    }
    Ok(warnings)
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
/// The caller holds the lock of the profile's files.
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
    let _files = browser.files.lock(profile_id).await;
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
    // The manifest here was overtaken in the vault, its blobs may be
    // collected: they are looked for, and an unchanged one goes out too.
    st.synced_hash.clear();
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

/// Apply remote lease ops and tell the UI; a failure is a warning.
async fn apply_leases_told(
    core: &Core,
    warnings: &mut Vec<String>,
    app: Option<&AppHandle>,
    ops: &[Op],
) {
    match apply_leases(core, ops).await {
        Ok(_) => {
            if let Some(app) = app {
                let _ = app.emit(EVENT_STATUS, ());
            }
        }
        Err(e) => {
            trace(app, "error", &format!("leases: {e}"));
            warnings.push(format!("leases: {e}"));
        }
    }
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
        let (running, launching): (HashSet<String>, HashSet<String>) =
            match cx.host.state::<BrowserState>() {
                Some(browser) => (
                    browser.running.running_ids().await.into_iter().collect(),
                    browser.launches.payload().launching.into_iter().collect(),
                ),
                None => Default::default(),
            };
        let files = cx.config.profile_files;
        let leases = collect_leases(cx.core, cx.clock, &running, &launching, files).await?;
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

    /// Leases and Firefox profile files: apply remote snapshots, try again
    /// those that wait, then upload local ones.
    /// Returns the retry reason when a remote snapshot could not be applied yet.
    async fn late(&self, cx: &mut Cycle<'_>, pulled_ops: &[Op]) -> CmdResult<Option<String>> {
        let app = cx.host.app();
        let browser = cx
            .host
            .state::<BrowserState>()
            .ok_or_else(|| AppError::other("the browser is not set up"))?;
        let db = &cx.core.db;
        // The release of a lease that came with a snapshot of the profile
        // goes in after the snapshot: till then the profile is held there,
        // and a launch here does not start it on the files of before.
        let files = cx.config.profile_files;
        let mut newest: HashMap<&str, &Op> = HashMap::new();
        for op in pulled_ops
            .iter()
            .filter(|op| op.entity_type == SNAPSHOT_ENTITY)
        {
            let slot = newest.entry(op.entity_id.as_str()).or_insert(op);
            if op.hlc > slot.hlc {
                *slot = op;
            }
        }
        let snapshots: HashSet<&str> = newest
            .into_iter()
            .filter(|(_, op)| files && !op.deleted)
            .map(|(id, _)| id)
            .collect();
        let (released, leases): (Vec<Op>, Vec<Op>) = pulled_ops
            .iter()
            .filter(|op| op.entity_type == LEASE_ENTITY)
            .cloned()
            .partition(|op| op.deleted && snapshots.contains(op.entity_id.as_str()));
        apply_leases_told(cx.core, cx.warnings, app, &leases).await;
        let fresh = veydan_sync_host::join::fresh_binding(db).await.is_some();
        if !files {
            // What comes while the switch is off is read past for good: it
            // is read again once the switch is on (`READ_ON_KEY`).
            if veydan_core::settings::get(db, READ_ON_KEY).await.as_deref() != Some("0") {
                veydan_core::settings::set(db, READ_ON_KEY, "0").await?;
            }
            trace(app, "info", "off");
            return Ok(None);
        }
        if veydan_core::settings::get(db, READ_ON_KEY).await.as_deref() != Some("1") {
            // The switch was off (or this build does not know since when it
            // is on): the snapshots read past meanwhile are read again by the
            // next cycle. The first read of a join takes them all anyway.
            if !fresh {
                let registry = cx.sync.registry();
                let item = format!("entity:{SNAPSHOT_ENTITY}");
                veydan_sync_host::state::reread_items(db, registry, &[item.as_str()]).await?;
                trace(app, "info", "on again: the snapshots are read again");
            }
            veydan_core::settings::set(db, READ_ON_KEY, "1").await?;
        }
        // The vault first, then the files here: a snapshot of another device
        // over files changed here makes the profile diverge before the
        // collect could send ours over it (audit SYNCROWS-05). The stage
        // comes with the first snapshot applied (`apply_manifest`).
        let retry = match apply_remote(cx.engine, app, cx.core, cx.sync, browser, pulled_ops).await
        {
            Ok(o) => {
                for reason in o.skipped.iter().chain(&o.failed) {
                    trace(app, "warn", reason);
                    cx.warnings.push(reason.clone());
                }
                match &o.retry {
                    Some(reason) => trace(app, "retry", reason),
                    None => trace(app, "info", "download ok"),
                }
                match retry_pending(cx.engine, app, cx.core, cx.sync, browser, &o.seen).await {
                    Ok(warnings) => {
                        for reason in warnings {
                            trace(app, "warn", &reason);
                            cx.warnings.push(reason);
                        }
                    }
                    Err(e) => {
                        trace(app, "error", &format!("pending: {e}"));
                        cx.warnings.push(format!("profiles pending: {e}"));
                    }
                }
                o.retry
            }
            Err(e) => {
                trace(app, "error", &format!("download: {e}"));
                cx.warnings.push(format!("profiles download: {e}"));
                None
            }
        };
        // A snapshot to try again brings the batch again, the release too.
        if retry.is_none() && !released.is_empty() {
            apply_leases_told(cx.core, cx.warnings, app, &released).await;
        }
        // A device that has just joined uploads no snapshot before it has
        // read the vault in full: its states went with the old binding.
        let collected = if veydan_sync_host::join::fresh_binding(db).await.is_some() {
            trace(app, "info", "upload waits until the vault was read");
            Ok(LocalChanges::default())
        } else {
            collect_local_changes(cx.engine, cx.core, cx.sync, browser, cx.clock, app).await
        };
        match collected {
            Ok(files) => {
                cx.warnings.extend(files.warnings.iter().cloned());
                let n = files.ops.len();
                trace(app, "info", &format!("upload {n} ops"));
                if !files.ops.is_empty() {
                    let registry = cx.sync.registry();
                    match veydan_sync_host::push_own(
                        cx.engine, db, registry, cx.local, cx.clock, files.ops,
                    )
                    .await
                    {
                        Err(failed) if failed.rotated => {
                            trace(app, "error", "own log collision, rotating device id");
                            return Err(failed.error);
                        }
                        Err(failed) => {
                            let e = failed.error;
                            trace(app, "error", &format!("upload: {e}"));
                            cx.warnings.push(format!("profiles upload: {e}"));
                            for (id, _) in &files.held {
                                browser.unsent.set(id, "failed", e.to_string());
                            }
                        }
                        Ok(()) => {
                            for st in &files.states {
                                save_profile_files_state(db, st).await?;
                                browser.unsent.sent(&st.profile_id);
                            }
                            for id in &files.gone {
                                delete_profile_files_state(db, id).await?;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                trace(app, "error", &format!("upload: {e}"));
                cx.warnings.push(format!("profiles upload: {e}"));
            }
        }
        Ok(retry)
    }

    async fn blob_refs(&self, engine: &Engine, op: &Op) -> CmdResult<Vec<String>> {
        blob_refs(engine, op).await
    }

    /// A lease of the old id whose profile runs here, or whose files are not
    /// out since it stopped (the lease waits for them), goes with the device to
    /// the new id and is published again: under the old one it was another
    /// device's, and no device would ever release it (audit A2-07, A2-19).
    /// A lease of any other profile that does not run here was taken by the copy
    /// this data file came from (a cloned machine runs it there, a restored
    /// one runs it nowhere): it is let go here without a word to the vault,
    /// where the old id keeps it.
    async fn device_rotated(
        &self,
        tx: &mut sqlx::SqliteConnection,
        old: &str,
        new: &str,
    ) -> CmdResult<()> {
        sqlx::query(
            "UPDATE sync_profile_files_state SET lease_device = ?, lease_synced = 0
             WHERE lease_device = ?
               AND (dirty = 1 OR profile_id IN (SELECT id FROM profiles WHERE status = 'running'))",
        )
        .bind(new)
        .bind(old)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        sqlx::query(
            "UPDATE sync_profile_files_state
             SET lease_device = '', lease_name = '', lease_since = '', lease_synced = 1
             WHERE lease_device = ?",
        )
        .bind(old)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
        Ok(())
    }

    /// A snapshot pushed after `newest` was lost with its chunk: the profile
    /// is collected again, as `heal_missing` does for one whose files went.
    async fn forget_pushed_after(
        &self,
        tx: &mut sqlx::SqliteConnection,
        device: &str,
        newest: Option<&Hlc>,
    ) -> CmdResult<()> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE sync_profile_files_state SET dirty = 1, synced_hash = '' WHERE {}",
            veydan_sync_host::state::pushed_after("head_hlc")
        )))
        .bind(device)
        .bind(veydan_sync_host::state::hlc_floor(newest))
        .execute(tx)
        .await
        .map_err(AppError::db)?;
        Ok(())
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

    #[test]
    fn a_manifest_path_takes_only_names_every_desktop_can_write() {
        for path in [
            "prefs.js",
            "storage/default/https+++example.com^userContextId=1/ls/data.sqlite",
            "places.sqlite-wal",
            "a b.c",
            "CONFIG",
            "com10",
            "lpt0.txt",
            "..a",
        ] {
            assert!(valid_rel_path(path), "{path}");
        }
        for path in [
            "",
            "/etc/passwd",
            "a//b",
            "../x",
            "a/./b",
            "a/..",
            "C:/x",
            "C:",
            "a:b",
            "x/C:",
            "CON",
            "con.txt",
            "Nul",
            "aux .txt",
            "COM1",
            "lpt9.log",
            "a\\b",
            "trail.",
            "trail ",
            "a?b",
            "a*b",
            "a\u{1}b",
            "lock",
            "cache2/x",
        ] {
            assert!(!valid_rel_path(path), "{path:?}");
        }
    }

    /// The browser of the profile ran here and stopped.
    async fn ran(state: &crate::modules::TestStates) {
        acquire_lease(&state.core, "pr-1").await.unwrap();
        on_profile_stopped(&state.core, "pr-1").await.unwrap();
    }

    /// A cycle and its warnings.
    async fn cycle_warnings(state: &crate::modules::TestStates) -> Vec<String> {
        veydan_sync_host::cycle(state.host(), &state.core, &state.sync)
            .await
            .unwrap()
    }

    /// Unix permissions stand for a file the antivirus holds or a folder
    /// that cannot be read, a renamed folder for a disk not mounted: the
    /// device that cannot read the files sends no snapshot, the other keeps
    /// every file, and the profile stays dirty until they can be read
    /// (audit A2-06, KA2-01). A name the manifest cannot carry is neither
    /// sent nor deleted where it lies (KA2-12).
    #[cfg(unix)]
    #[tokio::test]
    async fn files_that_cannot_be_read_send_no_snapshot() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, join_vault, write,
        };
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let profile = a.core.app_data_dir.join("profiles").join("pr-1");
        sqlx::query(
            "INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset,
               created_at, updated_at)
             VALUES ('pr-1', 'Brand A', 'stopped', ?, 'camoufox', 'windows',
               '2026-09-01T10:00:00+00:00', '2026-09-01T10:00:00+00:00')",
        )
        .bind(profile.to_string_lossy().as_ref())
        .execute(&a.core.db)
        .await
        .unwrap();
        let firefox = profile.join("firefox-profile");
        let (prefs, storage) = (firefox.join("prefs.js"), firefox.join("storage"));
        write(&prefs, b"user_pref(\"a\", 1);");
        write(&storage.join("data.bin"), b"stored");
        write(&firefox.join("odd\\name"), b"a name Windows cannot hold");
        ran(&a).await;
        cycle(&a).await;
        let sent = load_profile_files_state(&a.core.db, "pr-1")
            .await
            .unwrap()
            .unwrap();
        assert!(!sent.dirty);
        assert_eq!(Manifest::parse(&sent.manifest_json).unwrap().files.len(), 2);

        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        let theirs = b.core.app_data_dir.join("profiles/pr-1/firefox-profile");
        assert!(theirs.join("prefs.js").is_file());
        assert!(theirs.join("storage/data.bin").is_file());
        assert!(!theirs.join("odd\\name").exists());
        write(&theirs.join("odd\\name"), b"theirs");

        std::fs::set_permissions(&storage, Permissions::from_mode(0o000)).unwrap();
        let root = std::fs::read_dir(&storage).is_ok();
        std::fs::set_permissions(&storage, Permissions::from_mode(0o755)).unwrap();
        if root {
            eprintln!("permissions do not stop this user; skipped");
            close(a).await;
            close(b).await;
            let _ = std::fs::remove_dir_all(&vault);
            return;
        }

        // A folder, a file whose content changed, the folder gone, the
        // folder empty.
        let away = profile.join("firefox-profile.away");
        for case in ["folder", "file", "gone", "empty"] {
            ran(&a).await;
            match case {
                "folder" => {
                    std::fs::set_permissions(&storage, Permissions::from_mode(0o000)).unwrap()
                }
                "file" => {
                    write(&prefs, b"user_pref(\"a\", 2); // changed");
                    std::fs::set_permissions(&prefs, Permissions::from_mode(0o000)).unwrap();
                }
                "gone" => std::fs::rename(&firefox, &away).unwrap(),
                _ => {
                    std::fs::rename(&firefox, &away).unwrap();
                    std::fs::create_dir(&firefox).unwrap();
                }
            }
            let warnings = cycle_warnings(&a).await;
            match case {
                "folder" => {
                    std::fs::set_permissions(&storage, Permissions::from_mode(0o755)).unwrap()
                }
                "file" => std::fs::set_permissions(&prefs, Permissions::from_mode(0o644)).unwrap(),
                "gone" => std::fs::rename(&away, &firefox).unwrap(),
                _ => {
                    std::fs::remove_dir(&firefox).unwrap();
                    std::fs::rename(&away, &firefox).unwrap();
                }
            }
            assert!(
                warnings.iter().any(|w| w.contains("nothing sent")),
                "case {case}: {warnings:?}"
            );
            let st = load_profile_files_state(&a.core.db, "pr-1")
                .await
                .unwrap()
                .unwrap();
            assert!(st.dirty, "case {case}");
            assert_eq!(st.head_hlc, sent.head_hlc, "case {case}");

            cycle(&b).await;
            assert_eq!(
                std::fs::read(theirs.join("prefs.js")).unwrap(),
                b"user_pref(\"a\", 1);"
            );
            assert!(theirs.join("storage/data.bin").is_file(), "case {case}");
        }

        // Readable again: the change goes out, and the name the manifest
        // cannot carry stays on the device that applies it.
        cycle(&a).await;
        let st = load_profile_files_state(&a.core.db, "pr-1")
            .await
            .unwrap()
            .unwrap();
        assert!(!st.dirty);
        assert_ne!(st.head_hlc, sent.head_hlc);
        cycle(&b).await;
        assert_eq!(
            std::fs::read(theirs.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 2); // changed"
        );
        assert!(theirs.join("storage/data.bin").is_file());
        assert_eq!(std::fs::read(theirs.join("odd\\name")).unwrap(), b"theirs");
        assert!(firefox.join("odd\\name").is_file());

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// The late push of a snapshot goes from a position the vault agrees
    /// with even when the collect of the cycle had nothing: a data file put
    /// back behind its log, compacted past it, rotates the device id first,
    /// and the next cycle sends the files under the new one where the other
    /// device reads them (audit KA2-11, KA2-02).
    #[tokio::test]
    async fn a_late_snapshot_is_pushed_from_a_position_the_vault_agrees_with() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, device_id, join_vault, write,
        };
        use veydan_core::settings;

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let profile = a.core.app_data_dir.join("profiles").join("pr-1");
        sqlx::query(
            "INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset,
               created_at, updated_at)
             VALUES ('pr-1', 'Brand A', 'stopped', ?, 'camoufox', 'windows',
               '2026-09-01T10:00:00+00:00', '2026-09-01T10:00:00+00:00')",
        )
        .bind(profile.to_string_lossy().as_ref())
        .execute(&a.core.db)
        .await
        .unwrap();
        let prefs = profile.join("firefox-profile").join("prefs.js");
        write(&prefs, b"user_pref(\"a\", 1);");
        ran(&a).await;
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        let theirs = b
            .core
            .app_data_dir
            .join("profiles/pr-1/firefox-profile/prefs.js");
        assert_eq!(std::fs::read(&theirs).unwrap(), b"user_pref(\"a\", 1);");

        let db = &a.core.db;
        let copy = (
            settings::get(db, "sync_own_seq").await.unwrap(),
            settings::get(db, "sync_own_head").await.unwrap(),
        );
        for n in 2..4 {
            write(&prefs, format!("user_pref(\"a\", {n});").as_bytes());
            ran(&a).await;
            cycle(&a).await;
        }
        let engine = crate::sync::open_engine(&a.core).await.unwrap();
        let (local, _) = veydan_sync_host::state::load_local_state(db).await.unwrap();
        engine.compact(&local).await.unwrap();
        settings::set(db, "sync_own_seq", &copy.0).await.unwrap();
        settings::set(db, "sync_own_head", &copy.1).await.unwrap();
        // Only the files changed: nothing for the collect, a snapshot for
        // the late step.
        write(&prefs, b"user_pref(\"a\", 9);");
        sqlx::query("UPDATE sync_profile_files_state SET dirty = 1")
            .execute(db)
            .await
            .unwrap();
        let old = device_id(&a).await;

        let err = veydan_sync_host::cycle(a.host(), &a.core, &a.sync)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("device id rotated"), "{err}");
        assert_ne!(device_id(&a).await, old);
        cycle(&a).await;
        assert!(
            !load_profile_files_state(db, "pr-1")
                .await
                .unwrap()
                .unwrap()
                .dirty
        );
        cycle(&b).await;
        assert_eq!(std::fs::read(&theirs).unwrap(), b"user_pref(\"a\", 9);");

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A profile row of Space at `id` on `state`; its `firefox-profile/`.
    async fn add_profile(state: &crate::modules::TestStates, id: &str) -> PathBuf {
        let profile = state.core.app_data_dir.join("profiles").join(id);
        sqlx::query(
            "INSERT INTO profiles (id, name, status, profile_path, browser_type, fingerprint_preset,
               created_at, updated_at)
             VALUES (?, ?, 'stopped', ?, 'camoufox', 'windows',
               '2026-09-01T10:00:00+00:00', '2026-09-01T10:00:00+00:00')",
        )
        .bind(id)
        .bind(format!("Brand {id}"))
        .bind(profile.to_string_lossy().as_ref())
        .execute(&state.core.db)
        .await
        .unwrap();
        profile.join("firefox-profile")
    }

    /// The browser of profile `id` ran here and stopped.
    async fn ran_id(state: &crate::modules::TestStates, id: &str) {
        acquire_lease(&state.core, id).await.unwrap();
        on_profile_stopped(&state.core, id).await.unwrap();
    }

    async fn files_state(state: &crate::modules::TestStates, id: &str) -> ProfileFilesState {
        load_profile_files_state(&state.core.db, id)
            .await
            .unwrap()
            .unwrap()
    }

    /// The other device's files of `id`.
    fn their_files(state: &crate::modules::TestStates, id: &str) -> PathBuf {
        state
            .core
            .app_data_dir
            .join("profiles")
            .join(id)
            .join("firefox-profile")
    }

    /// Both devices ran the profile and changed its files before either
    /// saw the other's snapshot. The second applies the vault before it
    /// collects: the profile diverges there and neither side's files are
    /// laid over the other's; keeping its own then sends them on (audit
    /// SYNCROWS-05).
    #[tokio::test]
    async fn a_snapshot_taken_on_two_devices_diverges_and_loses_nothing() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, join_vault, write,
        };

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        ran(&a).await;
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        let theirs = their_files(&b, "pr-1");
        assert_eq!(
            std::fs::read(theirs.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 1);"
        );

        write(&ours.join("prefs.js"), b"user_pref(\"a\", 2); // A");
        ran(&a).await;
        cycle(&a).await;
        write(&theirs.join("prefs.js"), b"user_pref(\"a\", 3); // B");
        ran(&b).await;
        cycle(&b).await;

        let st = files_state(&b, "pr-1").await;
        assert!(st.diverged);
        assert!(!st.pending_manifest.is_empty());
        assert!(st.dirty);
        assert_eq!(st.head_hlc, files_state(&a, "pr-1").await.head_hlc);
        assert_eq!(
            std::fs::read(theirs.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 3); // B"
        );
        cycle(&a).await;
        assert_eq!(
            std::fs::read(ours.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 2); // A"
        );

        // B keeps its files: they go out, and A, which changed nothing
        // since, takes them.
        push_mine(&b.core, "pr-1").await.unwrap();
        cycle(&b).await;
        cycle(&a).await;
        assert_eq!(
            std::fs::read(ours.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 3); // B"
        );
        assert!(!files_state(&a, "pr-1").await.diverged);

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A snapshot that cannot be written (a folder stands where a file
    /// goes) leaves the other profiles of the batch applied, the heads of
    /// the peers moving and the snapshot pending; every cycle tries it
    /// again until it goes in (audit SYNCROWS-06).
    #[tokio::test]
    async fn a_snapshot_that_fails_to_apply_waits_alone() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, join_vault, write,
        };

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        for id in ["pr-1", "pr-2"] {
            write(
                &add_profile(&a, id).await.join("prefs.js"),
                format!("{id} 1").as_bytes(),
            );
            ran_id(&a, id).await;
        }
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        for id in ["pr-1", "pr-2"] {
            write(
                &a.core
                    .app_data_dir
                    .join("profiles")
                    .join(id)
                    .join("firefox-profile/prefs.js"),
                format!("{id} 2").as_bytes(),
            );
            ran_id(&a, id).await;
        }
        cycle(&a).await;
        // The failing one comes first in the batch.
        assert!(files_state(&a, "pr-1").await.head_hlc < files_state(&a, "pr-2").await.head_hlc);

        let blocked = their_files(&b, "pr-1").join("prefs.js");
        std::fs::remove_file(&blocked).unwrap();
        write(&blocked.join("inside"), b"a folder where a file goes");
        let warnings = cycle_warnings(&b).await;
        assert!(
            warnings.iter().any(|w| w.contains("snapshot not applied")),
            "{warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("retry next cycle")),
            "{warnings:?}"
        );
        assert_eq!(
            std::fs::read(their_files(&b, "pr-2").join("prefs.js")).unwrap(),
            b"pr-2 2"
        );
        let st = files_state(&b, "pr-1").await;
        assert!(!st.pending_manifest.is_empty());
        assert!(!st.diverged);
        assert_eq!(st.head_hlc, files_state(&a, "pr-1").await.head_hlc);

        let warnings = cycle_warnings(&b).await;
        assert!(
            warnings.iter().any(|w| w.contains("snapshot still waits")),
            "{warnings:?}"
        );

        std::fs::remove_dir_all(&blocked).unwrap();
        cycle(&b).await;
        assert_eq!(std::fs::read(&blocked).unwrap(), b"pr-1 2");
        let st = files_state(&b, "pr-1").await;
        assert!(st.pending_manifest.is_empty());
        assert!(!st.dirty && !st.diverged);

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A deleted profile leaves a tombstone of its snapshot and of its
    /// lease here; the garbage collection then takes the blobs of the last
    /// snapshot, and a device that joins later neither waits for the
    /// profile nor keeps a state of it (audit SYNCROWS-07, A2-23).
    #[tokio::test]
    async fn a_deleted_profile_leaves_tombstones_and_frees_its_blobs() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, collect_garbage, create_vault, cycle, device, device_id, join_vault, write,
        };

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        ran(&a).await;
        cycle(&a).await;
        acquire_lease(&a.core, "pr-1").await.unwrap();
        cycle(&a).await;
        let engine = crate::sync::open_engine(&a.core).await.unwrap();
        let latest = |entity: &'static str| {
            let engine = &engine;
            async move {
                engine
                    .all_latest_ops()
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|op| op.entity_type == entity && op.entity_id == "pr-1")
                    .unwrap()
            }
        };
        let snapshot = latest(SNAPSHOT_ENTITY).await;
        let manifest_blob = snapshot.payload["manifest_blob"].as_str().unwrap();
        let named = blob_refs(&engine, &snapshot).await.unwrap();
        assert_eq!(named.len(), 2);
        assert_eq!(named[0], manifest_blob);
        let dead = |blob: &String| {
            let (engine, db, blob) = (&engine, &a.core.db, blob.clone());
            async move {
                let candidate: Option<String> =
                    sqlx::query_scalar("SELECT blob FROM sync_gc_candidates WHERE blob = ?")
                        .bind(&blob)
                        .fetch_optional(db)
                        .await
                        .unwrap();
                candidate.is_some() || !engine.list_blobs().await.unwrap().contains(&blob)
            }
        };
        // While the snapshot lives, its blobs are live.
        collect_garbage(&a).await;
        for blob in &named {
            assert!(!dead(blob).await, "{blob}");
        }
        assert!(!latest(LEASE_ENTITY).await.deleted);

        crate::commands::profiles::crud::remove_profile("pr-1", &a.core)
            .await
            .unwrap();
        cycle(&a).await;
        let me = device_id(&a).await;
        for entity in [SNAPSHOT_ENTITY, LEASE_ENTITY] {
            let op = latest(entity).await;
            assert!(op.deleted, "{entity}");
            assert_eq!(op.hlc.device_id, me, "{entity}");
            assert!(blob_refs(&engine, &op).await.unwrap().is_empty());
        }
        assert!(load_profile_files_state(&a.core.db, "pr-1")
            .await
            .unwrap()
            .is_none());
        // Nothing more goes out for it.
        let tombstone = latest(SNAPSHOT_ENTITY).await.hlc;
        cycle(&a).await;
        assert_eq!(latest(SNAPSHOT_ENTITY).await.hlc, tombstone);

        let c = device(sync_registry(), &vault).await;
        join_vault(&c).await;
        cycle(&c).await;
        assert!(load_profile_files_state(&c.core.db, "pr-1")
            .await
            .unwrap()
            .is_none());

        // Dead to the garbage collection now: deleted, or a candidate whose
        // grace runs (this process uploaded them and holds them that long).
        collect_garbage(&a).await;
        for blob in &named {
            assert!(dead(blob).await, "{blob}");
        }

        close(a).await;
        close(c).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// The live snapshot of a profile and its tombstone in one batch: only
    /// the newest counts, so a device whose vault lacks the tombstone of
    /// the profile's row (a 4.0.7 compaction drops it) does not wait for a
    /// row that never comes.
    #[tokio::test]
    async fn the_tombstone_of_a_snapshot_spares_the_wait_for_its_profile() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{close, create_vault, device};

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let engine = crate::sync::open_engine(&a.core).await.unwrap();
        let at = |wall_ms| Hlc {
            wall_ms,
            counter: 0,
            device_id: "dev-x".into(),
        };
        let live = snapshot_op(
            "pr-9",
            at(1000),
            SnapshotPayload {
                manifest_blob: "m1".into(),
                ..Default::default()
            },
        );
        let browser = &a.browser;
        let outcome = apply_remote(
            &engine,
            None,
            &a.core,
            &a.sync,
            browser,
            std::slice::from_ref(&live),
        )
        .await
        .unwrap();
        assert!(outcome.retry.is_some_and(|r| r.contains("not known yet")));

        let ops = [live, snapshot_tombstone("pr-9", at(2000))];
        let outcome = apply_remote(&engine, None, &a.core, &a.sync, browser, &ops)
            .await
            .unwrap();
        assert!(outcome.retry.is_none());
        assert!(outcome.failed.is_empty());
        assert!(load_profile_files_state(&a.core.db, "pr-9")
            .await
            .unwrap()
            .is_none());

        close(a).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// The tombstone of a snapshot is the deletion of 9.5: an empty payload,
    /// which reads as a snapshot naming no manifest.
    #[test]
    fn the_tombstone_of_a_snapshot_names_no_blob() {
        let op = snapshot_tombstone("p1", hlc());
        assert_eq!(op.entity_type, "profile_snapshot");
        assert!(op.deleted);
        assert_eq!(op.payload, json!({}));
        let read: SnapshotPayload = serde_json::from_value(op.payload).unwrap();
        assert!(read.manifest_blob.is_empty());
    }

    /// A launch holds the lock of the profile's files until its process is
    /// registered: the apply of a snapshot that came meanwhile waits for it,
    /// then finds the browser running and lays nothing over its files; the
    /// snapshot waits, and the stop makes the profile diverge (audit
    /// SYNCROWS-12).
    #[tokio::test]
    async fn an_apply_waits_for_a_launch_and_spares_the_files_of_its_browser() {
        use crate::browser::launch::BrowserProcess;
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, join_vault, write,
        };
        use std::sync::atomic::{AtomicBool, Ordering};

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        ran(&a).await;
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 2);");
        ran(&a).await;
        cycle(&a).await;

        let theirs = their_files(&b, "pr-1").join("prefs.js");
        let launch = b.browser.files.lock("pr-1").await;
        let done = AtomicBool::new(false);
        let synced = async {
            let warnings = cycle_warnings(&b).await;
            done.store(true, Ordering::SeqCst);
            warnings
        };
        let launched = async {
            tokio::time::sleep(Duration::from_millis(800)).await;
            assert!(!done.load(Ordering::SeqCst), "the apply did not wait");
            b.browser
                .running
                .processes
                .lock()
                .await
                .insert("pr-1".into(), BrowserProcess::for_test(1));
            drop(launch);
        };
        let (warnings, ()) = tokio::join!(synced, launched);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(std::fs::read(&theirs).unwrap(), b"user_pref(\"a\", 1);");
        let st = files_state(&b, "pr-1").await;
        assert!(!st.pending_manifest.is_empty());
        assert!(!st.diverged);

        b.browser.running.processes.lock().await.remove("pr-1");
        on_profile_stopped(&b.core, "pr-1").await.unwrap();
        assert!(files_state(&b, "pr-1").await.diverged);
        assert_eq!(std::fs::read(&theirs).unwrap(), b"user_pref(\"a\", 1);");

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A launch waits for the files of a profile: the collect gives them up
    /// and sends the snapshot of the others; the profile stays changed and
    /// goes out with the next cycle once the launch is gone.
    #[tokio::test]
    async fn a_collect_gives_up_a_profile_a_launch_waits_for() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{close, create_vault, cycle, device, write};

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        for id in ["pr-1", "pr-2"] {
            let files = add_profile(&a, id).await;
            write(&files.join("prefs.js"), id.as_bytes());
            ran_id(&a, id).await;
        }

        let launch = a.browser.files.launch_waiting("pr-1");
        let warnings = cycle_warnings(&a).await;
        assert!(warnings.is_empty(), "{warnings:?}");
        let wanted = files_state(&a, "pr-1").await;
        assert!(wanted.dirty);
        assert!(wanted.head_hlc.is_none());
        let other = files_state(&a, "pr-2").await;
        assert!(!other.dirty);
        assert!(other.head_hlc.is_some());

        drop(launch);
        cycle(&a).await;
        let sent = files_state(&a, "pr-1").await;
        assert!(!sent.dirty);
        assert!(sent.head_hlc.is_some());

        close(a).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A snapshot read and waiting to go out leaves with its lock and its
    /// state when a launch waits for the profile; the others stay.
    #[tokio::test]
    async fn a_snapshot_waiting_to_go_out_is_given_up_for_a_launch() {
        let locks = crate::browser::ProfileLocks::default();
        let waits = locks.launch_waits();
        let mut out = LocalChanges::default();
        for id in ["pr-1", "pr-2"] {
            out.ops
                .push(snapshot_op(id, hlc(), SnapshotPayload::default()));
            out.states.push(ProfileFilesState::new(id));
            out.held.push((id.to_string(), locks.lock(id).await));
        }
        out.ops.push(lease_op("pr-1", hlc(), None));

        give_up_wanted(&mut out, &waits, None);
        assert_eq!(out.ops.len(), 3);

        let _launch = locks.launch_waiting("pr-1");
        give_up_wanted(&mut out, &waits, None);
        let ops: Vec<_> = out
            .ops
            .iter()
            .map(|op| (op.entity_type.as_str(), op.entity_id.as_str()))
            .collect();
        assert_eq!(ops, [(SNAPSHOT_ENTITY, "pr-2")]);
        assert_eq!(out.states.len(), 1);
        assert_eq!(out.states[0].profile_id, "pr-2");
        assert_eq!(out.held.len(), 1);
        let _free = locks
            .lock_for_launch("pr-1", || panic!("pr-1 is held"))
            .await;
    }

    /// Requests of a storage: looks (`exists`, `get`, `list`) and writes.
    #[derive(Default)]
    struct Requests {
        looks: std::sync::atomic::AtomicUsize,
        puts: std::sync::atomic::AtomicUsize,
    }

    struct Counting {
        inner: veydan_sync::LocalDir,
        n: std::sync::Arc<Requests>,
    }

    #[async_trait::async_trait]
    impl veydan_sync::Storage for Counting {
        async fn list(&self, prefix: &str) -> veydan_sync::Result<Vec<String>> {
            self.n
                .looks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.list(prefix).await
        }
        async fn get(&self, key: &str) -> veydan_sync::Result<Option<Vec<u8>>> {
            self.n
                .looks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> veydan_sync::Result<()> {
            self.n
                .puts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.put(key, data).await
        }
        async fn delete(&self, key: &str) -> veydan_sync::Result<()> {
            self.inner.delete(key).await
        }
        async fn exists(&self, key: &str) -> veydan_sync::Result<bool> {
            self.n
                .looks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.exists(key).await
        }
    }

    /// A file of the hash the head names takes its blob without a request
    /// to the vault; a changed one alone goes up. A manifest that is not
    /// the head (`trusted` off: after `heal_missing`, a side kept over
    /// another's) has its blobs looked for, and one gone goes up again.
    #[tokio::test]
    async fn a_file_of_the_hash_of_the_head_asks_the_vault_nothing() {
        use crate::sync::registries_tests::write;
        use std::sync::atomic::Ordering::SeqCst;

        let root = crate::db::test_dir();
        let (vault, firefox) = (root.join("vault"), root.join("firefox-profile"));
        let n = std::sync::Arc::new(Requests::default());
        let storage = Counting {
            inner: veydan_sync::LocalDir::new(&vault),
            n: n.clone(),
        };
        let vmk = veydan_sync::Vmk::from_base64(&format!("{}=", "A".repeat(43))).unwrap();
        let engine = Engine::with_key(Box::new(storage), &vmk, "vault", "device");
        let cache = HashCache::default();
        let locks = crate::browser::ProfileLocks::default();
        let waits = locks.launch_waits();
        let held = Held {
            waits: &waits,
            ids: vec!["pr-1".into()],
        };
        let mut blobs = BlobIndex {
            engine: &engine,
            uploaded: BTreeSet::new(),
        };
        write(&firefox.join("prefs.js"), b"user_pref(\"a\", 1);");
        write(&firefox.join("storage/data.bin"), b"stored");
        write(&firefox.join("cache.bin"), b"cached");
        macro_rules! build {
            ($blobs:expr, $prev:expr, $trusted:expr) => {
                build_manifest(
                    &cache,
                    $blobs,
                    &firefox,
                    $prev,
                    $trusted,
                    false,
                    None,
                    &held,
                    &|_, _| {},
                )
                .await
                .unwrap()
                .unwrap()
                .unwrap()
            };
        }
        let first = build!(&mut blobs, &Manifest::default(), false);
        assert_eq!(first.files.len(), 3);
        assert_eq!(n.puts.load(SeqCst), 3);

        let mut blobs = BlobIndex {
            engine: &engine,
            uploaded: BTreeSet::new(),
        };
        n.looks.store(0, SeqCst);
        n.puts.store(0, SeqCst);
        let same = build!(&mut blobs, &first, true);
        assert_eq!(same.to_json(), first.to_json());
        assert_eq!((n.looks.load(SeqCst), n.puts.load(SeqCst)), (0, 0));

        // Another length: the mtime of a write this soon may be the same.
        write(&firefox.join("prefs.js"), b"user_pref(\"a\", 20);");
        let changed = build!(&mut blobs, &first, true);
        assert_ne!(changed.to_json(), first.to_json());
        assert_eq!((n.looks.load(SeqCst), n.puts.load(SeqCst)), (0, 1));

        // The GC took a blob of the manifest, which is no head any more.
        let gone = &changed
            .files
            .iter()
            .find(|f| f.path == "cache.bin")
            .unwrap()
            .blob;
        engine.delete_blob(gone).await.unwrap();
        let mut blobs = BlobIndex {
            engine: &engine,
            uploaded: BTreeSet::new(),
        };
        n.looks.store(0, SeqCst);
        n.puts.store(0, SeqCst);
        let healed = build!(&mut blobs, &changed, false);
        assert_eq!(healed.to_json(), changed.to_json());
        assert_eq!((n.looks.load(SeqCst), n.puts.load(SeqCst)), (3, 1));
        assert!(engine.blob_exists(gone).await.unwrap());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The browser of profile `id` runs on `state`.
    async fn runs(state: &crate::modules::TestStates, id: &str) {
        use crate::browser::launch::BrowserProcess;
        state
            .browser
            .running
            .processes
            .lock()
            .await
            .insert(id.into(), BrowserProcess::for_test(1));
    }

    /// The browser of profile `id` on `state` exits.
    async fn stops(state: &crate::modules::TestStates, id: &str) {
        state.browser.running.processes.lock().await.remove(id);
        on_profile_stopped(&state.core, id).await.unwrap();
    }

    /// A browser that ran before sync was on holds no lease: the cycle
    /// takes it, and the other device sees the profile held there, and
    /// that it would start empty. A copy of its files goes out while it
    /// runs: a database with its journal, without the index SQLite builds
    /// again. The profile stays held there until the browser stopped and
    /// its files are out, the release coming after them (ROO-888).
    #[tokio::test]
    async fn a_profile_open_elsewhere_is_held_and_sends_a_copy_of_its_files() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, device_id, join_vault, write,
        };

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        write(&ours.join("places.sqlite"), b"database");
        write(&ours.join("places.sqlite-wal"), b"frames");
        write(&ours.join("places.sqlite-shm"), b"index");
        runs(&a, "pr-1").await;
        cycle(&a).await;
        cycle(&b).await;

        let me = device_id(&a).await;
        let held = foreign_lease(&b.core, "pr-1").await.unwrap();
        assert_eq!(held.map(|(device, _)| device), Some(me.clone()));
        let theirs = their_files(&b, "pr-1");
        assert_eq!(
            std::fs::read(theirs.join("places.sqlite")).unwrap(),
            b"database"
        );
        assert_eq!(
            std::fs::read(theirs.join("places.sqlite-wal")).unwrap(),
            b"frames"
        );
        assert!(!theirs.join("places.sqlite-shm").exists());
        assert!(!starts_empty(&b.core, "pr-1").await.unwrap());
        assert!(a.browser.unsent.list().is_empty());

        // The browser stops; until its files are out the lease stays.
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 2);");
        stops(&a, "pr-1").await;
        let launch = a.browser.files.launch_waiting("pr-1");
        cycle(&a).await;
        drop(launch);
        cycle(&b).await;
        assert!(foreign_lease(&b.core, "pr-1").await.unwrap().is_some());
        cycle(&a).await;
        cycle(&b).await;
        assert!(foreign_lease(&b.core, "pr-1").await.unwrap().is_none());
        assert_eq!(
            std::fs::read(theirs.join("prefs.js")).unwrap(),
            b"user_pref(\"a\", 2);"
        );
        assert!(files_state(&a, "pr-1").await.lease_device.is_empty());

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A profile held elsewhere whose files did not come is told as such:
    /// a launch would start it empty, and its badge says where they are.
    #[tokio::test]
    async fn a_profile_held_elsewhere_without_its_files_starts_empty() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{close, create_vault, cycle, device, join_vault};

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        add_profile(&a, "pr-1").await;
        acquire_lease(&a.core, "pr-1").await.unwrap();
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;

        assert!(starts_empty(&b.core, "pr-1").await.unwrap());
        let status = files_status(&b.core, &b.browser.unsent).await.unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, "elsewhere");
        assert!(!status[0].detail.is_empty());

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// The files of a profile that stopped are those of the copy sent
    /// while it ran: the release of the lease goes alone. A push of it
    /// that fails, or that a launch gives up, leaves the profile dirty and
    /// held here, and a later cycle releases it (ROO-888).
    #[cfg(unix)]
    #[tokio::test]
    async fn the_release_of_unchanged_files_goes_again_after_a_failed_push() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, device_id, join_vault, write,
        };
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        /// The vault read only, or writable again.
        fn read_only(path: &Path, on: bool) {
            let dir = path.is_dir();
            if dir && !on {
                std::fs::set_permissions(path, Permissions::from_mode(0o755)).unwrap();
            }
            if dir {
                for e in std::fs::read_dir(path).unwrap() {
                    read_only(&e.unwrap().path(), on);
                }
            }
            let mode = match (dir, on) {
                (true, true) => 0o555,
                (true, false) => 0o755,
                (false, true) => 0o444,
                (false, false) => 0o644,
            };
            std::fs::set_permissions(path, Permissions::from_mode(mode)).unwrap();
        }

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;
        cycle(&b).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        runs(&a, "pr-1").await;
        cycle(&a).await;
        stops(&a, "pr-1").await;
        let me = device_id(&a).await;

        read_only(&vault, true);
        let written = std::fs::write(vault.join("probe"), b"").is_ok();
        let _ = veydan_sync_host::cycle(a.host(), &a.core, &a.sync).await;
        read_only(&vault, false);
        if written {
            eprintln!("permissions do not stop this user; skipped");
            close(a).await;
            close(b).await;
            let _ = std::fs::remove_dir_all(&vault);
            return;
        }
        assert_eq!(files_state(&a, "pr-1").await.lease_device, me);
        assert!(files_state(&a, "pr-1").await.dirty);
        cycle(&b).await;
        assert!(foreign_lease(&b.core, "pr-1").await.unwrap().is_some());

        let launch = a.browser.files.launch_waiting("pr-1");
        cycle(&a).await;
        drop(launch);
        assert_eq!(files_state(&a, "pr-1").await.lease_device, me);
        assert!(files_state(&a, "pr-1").await.dirty);

        cycle(&a).await;
        cycle(&b).await;
        assert!(foreign_lease(&b.core, "pr-1").await.unwrap().is_none());
        let st = files_state(&a, "pr-1").await;
        assert!(st.lease_device.is_empty());
        assert!(!st.dirty);

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A first download that failed before any file was written leaves
    /// the profile empty here though its snapshot is known: held
    /// elsewhere, it is told so, and the retry brings the files (ROO-888).
    #[cfg(unix)]
    #[tokio::test]
    async fn a_first_download_that_failed_still_starts_empty() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{
            close, create_vault, cycle, device, join_vault, write,
        };
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        runs(&a, "pr-1").await;
        cycle(&a).await;
        let b = device(sync_registry(), &vault).await;
        join_vault(&b).await;

        let shut = b.core.app_data_dir.join("profiles/pr-1");
        std::fs::create_dir_all(&shut).unwrap();
        std::fs::set_permissions(&shut, Permissions::from_mode(0o555)).unwrap();
        let written = std::fs::write(shut.join("probe"), b"").is_ok();
        let _ = veydan_sync_host::cycle(b.host(), &b.core, &b.sync).await;
        std::fs::set_permissions(&shut, Permissions::from_mode(0o755)).unwrap();
        if written {
            eprintln!("permissions do not stop this user; skipped");
            close(a).await;
            close(b).await;
            let _ = std::fs::remove_dir_all(&vault);
            return;
        }
        let st = files_state(&b, "pr-1").await;
        assert!(st.head_hlc.is_some());
        assert!(!st.pending_manifest.is_empty());
        assert!(starts_empty(&b.core, "pr-1").await.unwrap());
        let status = files_status(&b.core, &b.browser.unsent).await.unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, "elsewhere");

        cycle(&b).await;
        assert!(their_files(&b, "pr-1").join("prefs.js").is_file());
        assert!(!starts_empty(&b.core, "pr-1").await.unwrap());

        close(a).await;
        close(b).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// A file of a running profile that cannot be read keeps its last
    /// copy, and the copy of the others goes out; a database of which no
    /// copy is in a manifest yet keeps the whole copy back, and the badge
    /// tells it. Files that cannot be read once it stopped send nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_running_profile_keeps_the_last_copy_of_what_it_cannot_read() {
        use crate::modules::sync_registry;
        use crate::sync::registries_tests::{close, create_vault, cycle, device, write};
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt;

        let vault = crate::db::test_dir();
        let a = device(sync_registry(), &vault).await;
        create_vault(&a).await;
        let ours = add_profile(&a, "pr-1").await;
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 1);");
        write(&ours.join("cookies.sqlite"), b"cookies");
        ran(&a).await;
        cycle(&a).await;
        let first = Manifest::parse(&files_state(&a, "pr-1").await.manifest_json).unwrap();

        runs(&a, "pr-1").await;
        let mut st = files_state(&a, "pr-1").await;
        st.snapshot_at = "2026-01-01T00:00:00+00:00".into();
        save_profile_files_state(&a.core.db, &st).await.unwrap();
        write(&ours.join("prefs.js"), b"user_pref(\"a\", 22);");
        std::fs::set_permissions(ours.join("cookies.sqlite"), Permissions::from_mode(0o000))
            .unwrap();
        cycle(&a).await;
        let live = Manifest::parse(&files_state(&a, "pr-1").await.manifest_json).unwrap();
        let cookies = |m: &Manifest| m.files.iter().find(|f| f.path == "cookies.sqlite").cloned();
        assert_eq!(cookies(&live).unwrap().blob, cookies(&first).unwrap().blob);
        let prefs = live.files.iter().find(|f| f.path == "prefs.js").unwrap();
        assert_eq!(prefs.hash, sha256_hex(b"user_pref(\"a\", 22);"));

        // A database no manifest has a copy of.
        write(&ours.join("places.sqlite"), b"places");
        std::fs::set_permissions(ours.join("places.sqlite"), Permissions::from_mode(0o000))
            .unwrap();
        let mut st = files_state(&a, "pr-1").await;
        st.snapshot_at = "2026-01-01T00:00:00+00:00".into();
        save_profile_files_state(&a.core.db, &st).await.unwrap();
        cycle(&a).await;
        assert_eq!(files_state(&a, "pr-1").await.manifest_json, live.to_json());
        let status = a.browser.unsent.list();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, "open");
        assert!(
            status[0].detail.contains("places.sqlite"),
            "{:?}",
            status[0]
        );

        stops(&a, "pr-1").await;
        let warnings = cycle_warnings(&a).await;
        assert!(
            warnings.iter().any(|w| w.contains("nothing sent")),
            "{warnings:?}"
        );
        assert_eq!(a.browser.unsent.list()[0].state, "failed");
        assert!(!files_state(&a, "pr-1").await.lease_device.is_empty());

        for f in ["cookies.sqlite", "places.sqlite"] {
            std::fs::set_permissions(ours.join(f), Permissions::from_mode(0o644)).unwrap();
        }
        close(a).await;
        let _ = std::fs::remove_dir_all(&vault);
    }

    #[test]
    fn a_database_is_read_with_its_journals() {
        assert_eq!(sqlite_base("places.sqlite"), Some("places.sqlite"));
        assert_eq!(sqlite_base("places.sqlite-wal"), Some("places.sqlite"));
        assert_eq!(
            sqlite_base("storage/x/idb/1.sqlite-journal"),
            Some("storage/x/idb/1.sqlite")
        );
        assert_eq!(sqlite_base("cert9.db"), Some("cert9.db"));
        assert_eq!(sqlite_base("prefs.js"), None);

        let dir = crate::db::test_dir();
        crate::sync::registries_tests::write(&dir.join("a.sqlite"), b"db");
        crate::sync::registries_tests::write(&dir.join("a.sqlite-wal"), b"wal");
        let read = read_stable(&dir, "a.sqlite").unwrap().unwrap();
        assert_eq!(
            read,
            [
                ("a.sqlite".to_string(), b"db".to_vec()),
                ("a.sqlite-wal".to_string(), b"wal".to_vec()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A large file a collect cut short sent already is not sent again:
    /// the next walk finds it in the vault. A small one goes up again.
    #[tokio::test]
    async fn a_large_file_sent_before_is_not_sent_again() {
        use crate::sync::registries_tests::write;
        use std::sync::atomic::Ordering::SeqCst;

        let root = crate::db::test_dir();
        let (vault, firefox) = (root.join("vault"), root.join("firefox-profile"));
        let n = std::sync::Arc::new(Requests::default());
        let storage = Counting {
            inner: veydan_sync::LocalDir::new(&vault),
            n: n.clone(),
        };
        let vmk = veydan_sync::Vmk::from_base64(&format!("{}=", "A".repeat(43))).unwrap();
        let engine = Engine::with_key(Box::new(storage), &vmk, "vault", "device");
        let cache = HashCache::default();
        let locks = crate::browser::ProfileLocks::default();
        let waits = locks.launch_waits();
        let held = Held {
            waits: &waits,
            ids: vec!["pr-1".into()],
        };
        write(&firefox.join("prefs.js"), b"user_pref(\"a\", 1);");
        write(
            &firefox.join("places.sqlite"),
            &vec![7u8; LOOK_FIRST as usize],
        );
        for puts in [2, 1] {
            let mut blobs = BlobIndex {
                engine: &engine,
                uploaded: BTreeSet::new(),
            };
            n.puts.store(0, SeqCst);
            let built = build_manifest(
                &cache,
                &mut blobs,
                &firefox,
                &Manifest::default(),
                false,
                false,
                None,
                &held,
                &|_, _| {},
            )
            .await
            .unwrap()
            .unwrap()
            .unwrap();
            assert_eq!(built.files.len(), 2);
            assert_eq!(n.puts.load(SeqCst), puts);
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Sync positions of the files of browser profiles, and the conflicts and
//! leases browser adds to the status of sync. The table belongs to the
//! schema of browser.

use crate::error::{AppError, CmdResult};
#[cfg(desktop)]
use sqlx::{Pool, Sqlite};
#[cfg(desktop)]
use std::collections::HashMap;
use veydan_core::{BoxFuture, Core};
#[cfg(desktop)]
use veydan_sync::Hlc;
use veydan_sync_host::{ConflictInfo, LeaseInfo};

/// Where the local `firefox-profile/` stands relative to the vault, plus the
/// lease telling which device may run the profile right now.
#[cfg(desktop)]
#[derive(Debug, Clone, Default)]
pub struct ProfileFilesState {
    pub profile_id: String,
    /// HLC of the last snapshot op pushed or applied.
    pub head_hlc: Option<Hlc>,
    /// Hash of the manifest JSON at that point.
    pub synced_hash: String,
    /// That manifest; lets unchanged files reuse their blob without re-reading.
    pub manifest_json: String,
    pub snapshot_at: String,
    /// Local files changed since the last snapshot (browser ran).
    pub dirty: bool,
    /// Remote manifest blob that could not be applied while the browser ran.
    pub pending_manifest: String,
    pub lease_device: String,
    pub lease_name: String,
    pub lease_since: String,
    pub lease_hlc: Option<Hlc>,
    /// Our own lease change was pushed.
    pub lease_synced: bool,
    /// Both sides changed the files; the user picks a side.
    pub diverged: bool,
}

#[cfg(desktop)]
impl ProfileFilesState {
    pub fn new(profile_id: &str) -> Self {
        Self {
            profile_id: profile_id.into(),
            lease_synced: true,
            ..Default::default()
        }
    }
}

#[cfg(desktop)]
type ProfileFilesRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
);

#[cfg(desktop)]
const SELECT_PROFILE_FILES: &str = "SELECT profile_id, head_hlc, synced_hash, manifest_json, snapshot_at, dirty,
    pending_manifest, lease_device, lease_name, lease_since, lease_hlc, lease_synced, diverged FROM sync_profile_files_state";

#[cfg(desktop)]
fn row_to_profile_files(r: ProfileFilesRow) -> ProfileFilesState {
    ProfileFilesState {
        profile_id: r.0,
        head_hlc: Hlc::decode(&r.1),
        synced_hash: r.2,
        manifest_json: r.3,
        snapshot_at: r.4,
        dirty: r.5 != 0,
        pending_manifest: r.6,
        lease_device: r.7,
        lease_name: r.8,
        lease_since: r.9,
        lease_hlc: Hlc::decode(&r.10),
        lease_synced: r.11 != 0,
        diverged: r.12 != 0,
    }
}

#[cfg(desktop)]
pub async fn load_profile_files_states(
    db: &Pool<Sqlite>,
) -> CmdResult<HashMap<String, ProfileFilesState>> {
    let rows: Vec<ProfileFilesRow> = sqlx::query_as(SELECT_PROFILE_FILES)
        .fetch_all(db)
        .await
        .map_err(AppError::db)?;
    Ok(rows
        .into_iter()
        .map(row_to_profile_files)
        .map(|s| (s.profile_id.clone(), s))
        .collect())
}

#[cfg(desktop)]
pub async fn load_profile_files_state(
    db: &Pool<Sqlite>,
    profile_id: &str,
) -> CmdResult<Option<ProfileFilesState>> {
    let row: Option<ProfileFilesRow> = sqlx::query_as(
        "SELECT profile_id, head_hlc, synced_hash, manifest_json, snapshot_at, dirty,
         pending_manifest, lease_device, lease_name, lease_since, lease_hlc, lease_synced, diverged
         FROM sync_profile_files_state WHERE profile_id = ?",
    )
    .bind(profile_id)
    .fetch_optional(db)
    .await
    .map_err(AppError::db)?;
    Ok(row.map(row_to_profile_files))
}

#[cfg(desktop)]
pub async fn save_profile_files_state(db: &Pool<Sqlite>, s: &ProfileFilesState) -> CmdResult<()> {
    sqlx::query(
        "INSERT INTO sync_profile_files_state (profile_id, head_hlc, synced_hash, manifest_json, snapshot_at, dirty,
           pending_manifest, lease_device, lease_name, lease_since, lease_hlc, lease_synced, diverged)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(profile_id) DO UPDATE SET
           head_hlc = excluded.head_hlc, synced_hash = excluded.synced_hash, manifest_json = excluded.manifest_json,
           snapshot_at = excluded.snapshot_at, dirty = excluded.dirty, pending_manifest = excluded.pending_manifest,
           lease_device = excluded.lease_device, lease_name = excluded.lease_name, lease_since = excluded.lease_since,
           lease_hlc = excluded.lease_hlc, lease_synced = excluded.lease_synced, diverged = excluded.diverged",
    )
    .bind(&s.profile_id)
    .bind(s.head_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(&s.synced_hash)
    .bind(&s.manifest_json)
    .bind(&s.snapshot_at)
    .bind(s.dirty as i64)
    .bind(&s.pending_manifest)
    .bind(&s.lease_device)
    .bind(&s.lease_name)
    .bind(&s.lease_since)
    .bind(s.lease_hlc.as_ref().map(Hlc::encode).unwrap_or_default())
    .bind(s.lease_synced as i64)
    .bind(s.diverged as i64)
    .execute(db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

#[cfg(desktop)]
pub async fn delete_profile_files_state(db: &Pool<Sqlite>, profile_id: &str) -> CmdResult<()> {
    sqlx::query("DELETE FROM sync_profile_files_state WHERE profile_id = ?")
        .bind(profile_id)
        .execute(db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Leased profiles, for `sync_status`: this device owns the leases it holds.
pub(crate) fn profile_leases<'a>(
    core: &'a Core,
    device_id: &'a str,
) -> BoxFuture<'a, CmdResult<Vec<LeaseInfo>>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT profile_id, lease_device, lease_name FROM sync_profile_files_state WHERE lease_device != ''",
        )
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
        Ok(rows
            .into_iter()
            .map(|(profile_id, lease_device, device_name)| LeaseInfo {
                profile_id,
                own: lease_device == device_id,
                device_id: lease_device,
                device_name,
            })
            .collect())
    })
}

/// Profiles whose files diverged, for `sync_status`: (id, name).
pub(crate) fn profile_conflicts(core: &Core) -> BoxFuture<'_, CmdResult<Vec<ConflictInfo>>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT s.profile_id, COALESCE(p.name, s.profile_id) FROM sync_profile_files_state s
             LEFT JOIN profiles p ON p.id = s.profile_id WHERE s.diverged = 1",
        )
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
        Ok(conflict_infos(rows))
    })
}

fn conflict_infos(rows: Vec<(String, String)>) -> Vec<ConflictInfo> {
    rows.into_iter()
        .map(|(note_id, title)| ConflictInfo { note_id, title })
        .collect()
}

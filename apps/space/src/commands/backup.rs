// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Full-application backup & restore: the driver of Space's `backup` service
//! (internal/platform-spec.md 5.9).
//!
//! A backup is a single encrypted file `veydan-backup-<ts>.vbk2` written to a
//! user-chosen folder. It holds the data file and the parts of the product's
//! modules (`Module::backup`): the driver takes their paths from the shell
//! and knows none by name — in Space the profiles of browser, the folder of
//! the notes and the user's folder of notes, the folder of the messenger.
//! Each part lies in the archive under its name. The Camoufox runtime binary
//! (`camoufox/`) is not a part: it is large and re-downloadable on demand.
//!
//! Pipeline (fully streaming, no intermediate plaintext archive on disk):
//! `tar` (file tree + manifest.json) → `zstd` (compression) → `age`
//! (passphrase / scrypt authenticated encryption) → `*.vbk2`.
//!
//! The DB is captured via `VACUUM INTO`, giving a transactionally-consistent
//! single-file snapshot without the live WAL/SHM sidecars.
//!
//! A restore stops every module (`Shell::shutdown`, as on exit), closes
//! the data file, swaps the data file and the parts the archive holds under
//! them and restarts the app. The folders of the parts outside the data
//! directory are swapped by renames with a whole copy made beside them,
//! before the swap is marked complete; a marker lets the next start finish
//! or undo a swap cut short, the previous folder stays beside the restored
//! one, and the outcome is kept for the settings.
//!
//! Scheduling: a background task ticks every 60s, reads the schedule from
//! `app_settings`, and runs a backup when one is due (catching up missed runs
//! after the app was closed). Once a new backup is whole, old ones are
//! rotated to the newest N. The list and the rotation see only archives of
//! this format: the extension carries the format version, so archives of
//! another one are left alone.

use crate::error::{AppError, CmdResult};
use age::secrecy::SecretString;
use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};
use veydan_core::{settings, Core};
use veydan_shell::{BackupPath, Shell};

use crate::commands::profiles::is_blacklisted;
use veydan_core::db::DB_FILE;

/// Archive layout: `manifest.json`, the data file under its own name
/// (`app.db`), then one folder per part under the part's name. An archive
/// of any other version is refused. The manifest names the parts since
/// stage 10; an archive without them holds `profiles/`, `notes/` and
/// `notes_custom/`, the parts of then.
const FORMAT_VERSION: u32 = 2;
/// Max scrypt log2(N) accepted on restore (age default is 18-22 on modern HW).
const MAX_SCRYPT_WORK_FACTOR: u8 = 22;
const FILE_PREFIX: &str = "veydan-backup-";
/// `vbk` plus `FORMAT_VERSION`.
const FILE_EXT: &str = "vbk2";
const MANIFEST_FILE: &str = "manifest.json";
/// What an archive is written as until it is whole; neither listed nor
/// rotated, and a leftover one goes at the next start.
const PART_SUFFIX: &str = ".part";
/// Folder of the snapshot of the data file while a backup runs.
const BACKUP_STAGING: &str = "backup_tmp";
/// The `created_at` of the newest archive made of this data (a local key of
/// `app_settings`, not synced): a restore of an older one asks first.
const LAST_CREATED_KEY: &str = "backup_last_created";
/// The start of the error of a restore refused because the archive is older
/// than the data: `backup_older:<created_at of the archive>|<of the newest>`.
const OLDER_ERROR: &str = "backup_older:";

/// Journals of the live data file; they move with it.
const SWAPPED_SIDECARS: [&str; 2] = ["-wal", "-shm"];
/// Folder of the unpacked archive inside the data directory.
const RESTORE_STAGING: &str = "restore_tmp";
/// Folder holding the live entries while a restore swaps them: it exists
/// from the first move until the swap is complete.
const SAVED_PREFIX: &str = "restore_backup_";
/// The same folder once the swap is complete and it is only waiting to be removed.
const DONE_PREFIX: &str = "restore_done_";
/// Marker of a swap under way in the data directory (`SwapMarker`): what
/// it moves besides the live entries, the folders outside first of all.
const SWAP_MARKER: &str = "restore_external.json";
/// The outcome of the last restore (`RestoreOutcome`), in the data directory.
const RESTORE_OUTCOME: &str = "restore_result.json";
/// The staged copy of a folder outside the data directory, beside it.
const NEW_INFIX: &str = ".restore_new_";
/// The live folder outside the data directory once a swap has moved it aside.
const OLD_INFIX: &str = ".restore_old_";
/// The previous folder outside the data directory once a restore is complete.
const KEPT_INFIX: &str = ".before-restore-";

// ── Shared state ────────────────────────────────────────────────────────────

/// Tracks whether a backup is currently running so manual + scheduled runs
/// can't overlap and corrupt the staging area.
#[derive(Default)]
pub struct BackupManager {
    running: AtomicBool,
}

/// RAII guard: clears the running flag on drop so an early `?` return can never
/// leave the backup wedged as "in progress".
struct RunningGuard<'a>(&'a AtomicBool);
impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

// ── Config (persisted in app_settings) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    pub dir: Option<String>,
    /// Never sent to the UI. On save: `None` keeps the stored password,
    /// `Some("")` clears it.
    #[serde(skip_serializing, default)]
    pub password: Option<String>,
    #[serde(default)]
    pub has_password: bool,
    pub schedule_enabled: bool,
    /// "interval" | "daily" | "weekly"
    pub schedule_mode: String,
    pub interval_hours: i64,
    /// "HH:MM" local time, used by daily/weekly modes.
    pub time: String,
    /// 0 = Monday … 6 = Sunday (weekly mode).
    pub weekday: i64,
    /// How many newest backups to keep (rotation). 0 = keep all.
    pub keep: i64,
    /// RFC3339 UTC timestamp of the last successful backup.
    pub last_run: Option<String>,
    /// What the last restore came to (read from the data directory; never saved).
    #[serde(default, skip_deserializing)]
    pub last_restore: Option<RestoreOutcome>,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            dir: None,
            password: None,
            has_password: false,
            schedule_enabled: false,
            schedule_mode: "interval".into(),
            interval_hours: 24,
            time: "03:00".into(),
            weekday: 0,
            keep: 5,
            last_run: None,
            last_restore: None,
        }
    }
}

async fn load_config(db: &Pool<Sqlite>) -> BackupConfig {
    let d = BackupConfig::default();
    let password = settings::get(db, "backup_password")
        .await
        .filter(|s| !s.is_empty());
    BackupConfig {
        dir: settings::get(db, "backup_dir")
            .await
            .filter(|s| !s.is_empty()),
        has_password: password.is_some(),
        password,
        schedule_enabled: settings::get(db, "backup_schedule_enabled")
            .await
            .map(|v| v == "1")
            .unwrap_or(d.schedule_enabled),
        schedule_mode: settings::get(db, "backup_schedule_mode")
            .await
            .unwrap_or(d.schedule_mode),
        interval_hours: settings::get(db, "backup_interval_hours")
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.interval_hours),
        time: settings::get(db, "backup_time").await.unwrap_or(d.time),
        weekday: settings::get(db, "backup_weekday")
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.weekday),
        keep: settings::get(db, "backup_keep")
            .await
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.keep),
        last_run: settings::get(db, "backup_last_run").await,
        last_restore: None,
    }
}

// ── Commands: config ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn backup_get_config(core: tauri::State<'_, Core>) -> CmdResult<BackupConfig> {
    let mut cfg = load_config(&core.db).await;
    cfg.last_restore = read_outcome(&core.app_data_dir);
    Ok(cfg)
}

#[tauri::command]
pub async fn backup_set_config(cfg: BackupConfig, core: tauri::State<'_, Core>) -> CmdResult<()> {
    let db = &core.db;
    match cfg.dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => settings::set(db, "backup_dir", v).await?,
        None => settings::set(db, "backup_dir", "").await?,
    }
    // None = keep the stored password; "" = clear it.
    if let Some(v) = cfg.password.as_deref() {
        settings::set(db, "backup_password", v).await?;
    }
    settings::set(
        db,
        "backup_schedule_enabled",
        if cfg.schedule_enabled { "1" } else { "0" },
    )
    .await?;
    settings::set(db, "backup_schedule_mode", &cfg.schedule_mode).await?;
    settings::set(db, "backup_interval_hours", &cfg.interval_hours.to_string()).await?;
    settings::set(db, "backup_time", &cfg.time).await?;
    settings::set(db, "backup_weekday", &cfg.weekday.to_string()).await?;
    settings::set(db, "backup_keep", &cfg.keep.to_string()).await?;
    Ok(())
}

// ── Commands: list ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BackupFileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// Modified time, RFC3339 UTC.
    pub modified: String,
}

/// True for the name of an archive of this format.
fn is_backup_name(name: &str) -> bool {
    name.starts_with(FILE_PREFIX)
        && Path::new(name).extension().and_then(|e| e.to_str()) == Some(FILE_EXT)
}

fn list_backups(dir: &Path) -> Vec<BackupFileInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_backup_name(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| DateTime::<Utc>::from(t).to_rfc3339().into())
            .unwrap_or_default();
        out.push(BackupFileInfo {
            name,
            path: entry.path().to_string_lossy().to_string(),
            size: meta.len(),
            modified,
        });
    }
    // Newest first.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

#[tauri::command]
pub async fn backup_list(core: tauri::State<'_, Core>) -> CmdResult<Vec<BackupFileInfo>> {
    let dir = settings::get(&core.db, "backup_dir")
        .await
        .filter(|s| !s.is_empty());
    match dir {
        Some(d) => Ok(list_backups(Path::new(&d))),
        None => Ok(Vec::new()),
    }
}

// ── Commands: run ───────────────────────────────────────────────────────────

/// Kick off a manual backup. Returns immediately; progress and completion are
/// reported via `backup://progress`, `backup://done`, `backup://error`.
#[tauri::command]
pub async fn backup_run_now(app: AppHandle) -> CmdResult<()> {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = perform_backup(&app).await {
            let _ = app.emit("backup://error", e);
        }
    });
    Ok(())
}

#[derive(Serialize, Clone)]
struct Progress {
    phase: String,
    percent: u32,
}

fn emit_progress(app: &AppHandle, phase: &str, percent: u32) {
    let _ = app.emit(
        "backup://progress",
        Progress {
            phase: phase.into(),
            percent,
        },
    );
}

/// The full backup routine. Emits progress events; returns Err(message) on
/// failure. Safe to call from both the command and the scheduler.
async fn perform_backup(app: &AppHandle) -> Result<PathBuf, String> {
    let core = app.state::<Core>();
    let backup = app.state::<BackupManager>();

    // Guard against overlapping runs.
    if backup
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("A backup is already running".into());
    }
    let _guard = RunningGuard(&backup.running);

    let cfg = load_config(&core.db).await;
    let dir = cfg
        .dir
        .clone()
        .ok_or_else(|| "No backup folder configured".to_string())?;
    let password = cfg
        .password
        .clone()
        .ok_or_else(|| "No backup password configured".to_string())?;
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create backup folder: {e}"))?;

    // What the modules of the product keep beside the data file.
    let parts = app.state::<Shell>().backup_paths();
    let out_path = back_up_into(
        Some(app),
        &core.db,
        &core.app_data_dir,
        &parts,
        &dir,
        &password,
        app.package_info().version.to_string(),
        usize::try_from(cfg.keep).unwrap_or(0),
    )
    .await?;

    let now = Utc::now().to_rfc3339();
    // If last_run can't be persisted the scheduler would re-run the backup on
    // every tick — the backup itself succeeded, so just log the failure loudly.
    if let Err(e) = settings::set(&core.db, "backup_last_run", &now).await {
        eprintln!("backup: failed to persist backup_last_run: {e}");
    }

    emit_progress(app, "done", 100);
    let _ = app.emit("backup://done", out_path.to_string_lossy().to_string());
    Ok(out_path)
}

/// Write a new archive into `dir` and, once it is whole, rotate the folder
/// to the `keep` newest archives (0 keeps all). A backup that fails leaves
/// the folder as it was: an archive cut short or missing a part never takes
/// the place of a whole one.
#[allow(clippy::too_many_arguments)]
async fn back_up_into(
    app: Option<&AppHandle>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    parts: &[BackupPath],
    dir: &Path,
    password: &str,
    app_version: String,
    keep: usize,
) -> Result<PathBuf, String> {
    let out_name = format!(
        "{FILE_PREFIX}{}.{FILE_EXT}",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let out_path = dir.join(&out_name);
    let created_at = make_archive(app, db, data_dir, parts, &out_path, password, app_version).await?;
    // What a restore tells an older archive by (`older_than_the_data`).
    if let Err(e) = settings::set(db, LAST_CREATED_KEY, &created_at).await {
        eprintln!("backup: failed to persist {LAST_CREATED_KEY}: {e}");
    }
    if keep > 0 {
        rotate(dir, keep);
    }
    Ok(out_path)
}

/// Write the archive at `out_path`: a snapshot of the data file of
/// `data_dir` and the files of `parts`, with the manifest naming them. The
/// archive is written as `<out_path>.part` and takes its name only once
/// every layer is finished and on disk; a part that cannot be read in full
/// fails the backup. Gives the `created_at` of its manifest. `app` is
/// optional so the pipeline can be exercised in unit tests.
async fn make_archive(
    app: Option<&AppHandle>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    parts: &[BackupPath],
    out_path: &Path,
    password: &str,
    app_version: String,
) -> Result<String, String> {
    if let Some(app) = app {
        emit_progress(app, "snapshot", 2);
    }

    // 1) Consistent DB snapshot via VACUUM INTO into a staging dir.
    let staging = data_dir
        .join(BACKUP_STAGING)
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let db_snapshot = staging.join(DB_FILE);
    if let Err(e) = snapshot_db(db, &db_snapshot).await {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("DB snapshot failed: {e}"));
    }

    // 2) The files of the parts (archive_name, fs_path).
    let (files, dirs, recorded) = match collect_parts(data_dir, parts) {
        Ok(collected) => collected,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    // The live databases among them (the messenger's), captured whole.
    let files = snapshot_part_databases(files, &staging).await;

    // 3) Manifest.
    let created_at = Utc::now().to_rfc3339();
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        app_version,
        created_at: created_at.clone(),
        includes_camoufox: false,
        notes_custom_dir: None,
        parts: Some(recorded),
        entries: files.iter().map(|(n, _)| n.clone()).collect(),
    };

    // 4) Stream the archive (blocking CPU/IO work off the async runtime).
    let app2 = app.cloned();
    let db_snapshot2 = db_snapshot.clone();
    let part_path = part_path_of(out_path);
    let part_path2 = part_path.clone();
    let password = password.to_owned();
    let write_res = tauri::async_runtime::spawn_blocking(move || {
        write_archive(
            app2.as_ref(),
            &part_path2,
            &password,
            &manifest,
            &db_snapshot2,
            &dirs,
            &files,
        )
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|res| res)
    .and_then(|()| {
        std::fs::rename(&part_path, out_path).map_err(|e| format!("Cannot name the backup: {e}"))
    });

    // Clean staging regardless of outcome.
    let _ = std::fs::remove_dir_all(&staging);
    write_res
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&part_path);
        })
        .map(|()| created_at)
}

/// The name an archive is written under until it is whole.
fn part_path_of(out_path: &Path) -> PathBuf {
    let mut name = out_path.as_os_str().to_owned();
    name.push(PART_SUFFIX);
    PathBuf::from(name)
}

/// Remove what a backup the process did not live to finish left in the
/// backup folder `dir`: archives written under their `.part` name.
fn sweep_parts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.strip_suffix(PART_SUFFIX).is_some_and(is_backup_name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A part of an archive as its manifest records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ManifestPart {
    /// Its folder in the archive.
    name: String,
    /// Where it was: one folder or file of the data directory, or an
    /// absolute path outside it.
    path: String,
    external: bool,
}

/// Files (archive name, path), the parts that are folders, the parts as
/// the manifest records them.
type Collected = (Vec<(String, PathBuf)>, Vec<String>, Vec<ManifestPart>);

/// The files of `parts`, each under the name of its part, the parts that
/// are folders (written as a folder of the archive, so an empty one is
/// restored empty), and the parts as the manifest records them. A part
/// that does not keep the promise of `BackupPath` (one name; one entry of
/// the data directory or an absolute path) is left out and told, and so is
/// a second part of one name. A folder of a part that cannot be read fails
/// the backup.
fn collect_parts(data_dir: &Path, parts: &[BackupPath]) -> Result<Collected, String> {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut recorded: Vec<ManifestPart> = Vec::new();
    for part in parts {
        let Some(source) = part_source(data_dir, part) else {
            eprintln!(
                "backup: part `{}` at {} left out: not a name of the data directory or an absolute path",
                part.name,
                part.path.display()
            );
            continue;
        };
        if recorded.iter().any(|r| r.name == part.name) {
            eprintln!("backup: a second part `{}` left out", part.name);
            continue;
        }
        if source.is_file() {
            files.push((part.name.to_string(), source));
        } else {
            if source.is_dir() {
                dirs.push(part.name.to_string());
            }
            collect_files(&source, part.name, &mut files)?;
        }
        recorded.push(ManifestPart {
            name: part.name.to_string(),
            path: part.path.to_string_lossy().into_owned(),
            external: part.external,
        });
    }
    Ok((files, dirs, recorded))
}

/// Where a part lives, if it keeps the promise of `BackupPath`.
fn part_source(data_dir: &Path, part: &BackupPath) -> Option<PathBuf> {
    if !is_part_name(part.name) {
        return None;
    }
    if part.external {
        part.path.is_absolute().then(|| part.path.clone())
    } else {
        is_part_name(&part.path.to_string_lossy()).then(|| data_dir.join(&part.path))
    }
}

/// One normal path component and nothing else.
fn is_plain_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    )
}

/// One plain name, which is neither the data file nor the manifest nor a
/// folder of the backup and restore themselves: a folder of the archive, or
/// an entry of the data directory a restore may replace.
fn is_part_name(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) && name != MANIFEST_FILE
        && !name.starts_with(DB_FILE)
        && name != RESTORE_STAGING
        && name != BACKUP_STAGING
        && !name.starts_with(SAVED_PREFIX)
        && !name.starts_with(DONE_PREFIX)
}

/// Transactionally-consistent copy of the live database in one file. The
/// copy carries the marker and the schema version of the original.
async fn snapshot_db(db: &Pool<Sqlite>, target: &Path) -> Result<(), sqlx::Error> {
    let vacuum_sql = format!(
        "VACUUM INTO '{}'",
        target.to_string_lossy().replace('\'', "''")
    );
    // Dynamic SQL: VACUUM INTO can't take a bound parameter for the filename,
    // so the path is escaped and embedded. `raw_sql` is sqlx's opt-in for
    // audited dynamic statements.
    sqlx::raw_sql(sqlx::AssertSqlSafe(vacuum_sql))
        .execute(db)
        .await
        .map(|_| ())
}

/// Journals SQLite keeps beside a database file.
const DB_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// A part's SQLite database (the messenger's `messenger.db`) is live while
/// the backup runs: copied file by file, it and its `-wal` would be taken
/// at different moments, and the copy would not open. Each is captured as
/// the data file is, by `VACUUM INTO` through a connection of its own, into
/// `staging`, and its journals stay out of the archive (the snapshot holds
/// what they held). Only `*.db` files are taken so: the SQLite files of a
/// browser profile (`*.sqlite`) belong to a browser that may hold them
/// locked, and are copied as before. One that cannot be captured — not a
/// database after all, say — is copied as it is, as before.
async fn snapshot_part_databases(
    files: Vec<(String, PathBuf)>,
    staging: &Path,
) -> Vec<(String, PathBuf)> {
    let mut out = Vec::with_capacity(files.len());
    let mut captured: Vec<PathBuf> = Vec::new();
    for (arch, path) in files {
        if path.extension().and_then(|e| e.to_str()) != Some("db") || !is_sqlite_file(&path) {
            out.push((arch, path));
            continue;
        }
        let target = staging.join(format!("part-{}.db", out.len()));
        match snapshot_db_file(&path, &target).await {
            Ok(()) => {
                captured.push(path);
                out.push((arch, target));
            }
            Err(e) => {
                eprintln!("backup: {} copied as it is: {e}", path.display());
                out.push((arch, path));
            }
        }
    }
    out.retain(|(_, path)| {
        !captured.iter().any(|db| {
            DB_SIDECARS.iter().any(|s| {
                let mut name = db.as_os_str().to_owned();
                name.push(s);
                path.as_os_str() == name
            })
        })
    });
    out
}

/// The file starts as an SQLite database does.
fn is_sqlite_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head == b"SQLite format 3\0"
}

/// `snapshot_db` of a database file nobody gave a pool of: a connection of
/// its own, which neither creates the file nor changes its journal mode.
async fn snapshot_db_file(path: &Path, target: &Path) -> Result<(), sqlx::Error> {
    use sqlx::{ConnectOptions, Connection};
    let mut conn = veydan_core::db::options(path)
        .create_if_missing(false)
        .connect()
        .await?;
    let vacuum_sql = format!(
        "VACUUM INTO '{}'",
        target.to_string_lossy().replace('\'', "''")
    );
    let res = sqlx::raw_sql(sqlx::AssertSqlSafe(vacuum_sql))
        .execute(&mut conn)
        .await
        .map(|_| ());
    let _ = conn.close().await;
    res
}

/// Recursively gather every file under `base_fs`, mapping it to an archive path
/// rooted at `base_arch`, skipping runtime-cache blacklist names. What cannot
/// be read fails: an archive without it would pass for a whole copy. What is
/// not there (a part never made, a cache gone meanwhile, a dangling link)
/// holds nothing.
fn collect_files(
    base_fs: &Path,
    base_arch: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let unreadable =
        |path: &Path, e: std::io::Error| format!("Cannot read {}: {e}", path.display());
    let entries = match std::fs::read_dir(base_fs) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(unreadable(base_fs, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| unreadable(base_fs, e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_blacklisted(&name) {
            continue;
        }
        let path = entry.path();
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(unreadable(&path, e)),
        };
        let arch = format!("{base_arch}/{name}");
        if meta.is_dir() {
            collect_files(&path, &arch, out)?;
        } else if meta.is_file() {
            out.push((arch, path));
        }
    }
    Ok(())
}

/// Write manifest + db snapshot + the folders of the parts + files as
/// tar→zstd→age into `out_path`, and sync it to disk. A file gone since it
/// was listed is left out, and the manifest is written once more at the end
/// without it (the last copy is the one a restore unpacks); a file that
/// cannot be read fails the archive. `app` is optional so the pipeline can
/// be exercised in unit tests.
fn write_archive(
    app: Option<&AppHandle>,
    out_path: &Path,
    password: &str,
    manifest: &Manifest,
    db_snapshot: &Path,
    dirs: &[String],
    files: &[(String, PathBuf)],
) -> Result<(), String> {
    let manifest_json = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    let file = File::create(out_path).map_err(|e| e.to_string())?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(password.to_owned()));
    let age_writer = encryptor
        .wrap_output(file)
        .map_err(|e| format!("Encryption init failed: {e}"))?;
    let zstd_writer =
        zstd::stream::write::Encoder::new(age_writer, 3).map_err(|e| e.to_string())?;
    let mut tar = tar::Builder::new(zstd_writer);

    append_bytes(&mut tar, MANIFEST_FILE, &manifest_json).map_err(|e| e.to_string())?;

    let total = files.len() + 1;
    let mut done = 0usize;
    // DB snapshot.
    {
        let mut f = File::open(db_snapshot).map_err(|e| e.to_string())?;
        tar.append_file(DB_FILE, &mut f)
            .map_err(|e| e.to_string())?;
        done += 1;
        if let Some(app) = app {
            emit_progress(app, "archiving", pct(done, total));
        }
    }
    for name in dirs {
        append_dir(&mut tar, name).map_err(|e| e.to_string())?;
    }
    let mut gone: Vec<&str> = Vec::new();
    for (arch, path) in files {
        match File::open(path) {
            Ok(mut f) => tar
                .append_file(arch, &mut f)
                .map_err(|e| format!("Cannot back up {}: {e}", path.display()))?,
            // Files can vanish mid-backup (temp caches).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => gone.push(arch),
            Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
        }
        done += 1;
        if let Some(app) = app {
            if done.is_multiple_of(25) || done == total {
                emit_progress(app, "archiving", pct(done, total));
            }
        }
    }

    if !gone.is_empty() {
        let mut kept = manifest.clone();
        kept.entries.retain(|e| !gone.contains(&e.as_str()));
        let kept_json = serde_json::to_vec_pretty(&kept).map_err(|e| e.to_string())?;
        append_bytes(&mut tar, MANIFEST_FILE, &kept_json).map_err(|e| e.to_string())?;
    }

    // Finish each layer in order: tar → zstd → age.
    let zstd_writer = tar.into_inner().map_err(|e| e.to_string())?;
    let age_writer = zstd_writer.finish().map_err(|e| e.to_string())?;
    let file = age_writer
        .finish()
        .map_err(|e| format!("Encryption finalize failed: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("Cannot write the backup: {e}"))
}

fn pct(done: usize, total: usize) -> u32 {
    if total == 0 {
        return 100;
    }
    ((done as f64 / total as f64) * 100.0).round() as u32
}

fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    data: &[u8],
) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, name, data)
}

/// A folder of the archive: a part restored empty when it holds nothing.
fn append_dir<W: Write>(builder: &mut tar::Builder<W>, name: &str) -> std::io::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(0o755);
    header.set_cksum();
    builder.append_data(&mut header, name, std::io::empty())
}

/// Delete all but the `keep` newest archives of this format in `dir`.
fn rotate(dir: &Path, keep: usize) {
    let mut backups = list_backups(dir); // already newest-first
    if backups.len() <= keep {
        return;
    }
    for old in backups.split_off(keep) {
        let _ = std::fs::remove_file(&old.path);
    }
}

// ── Commands: restore ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    app_version: String,
    created_at: String,
    includes_camoufox: bool,
    /// The user's folder of notes in an archive made before the parts were
    /// recorded; a newer one records it as a part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notes_custom_dir: Option<String>,
    /// The parts of the archive; none in one made before stage 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parts: Option<Vec<ManifestPart>>,
    entries: Vec<String>,
}

impl Manifest {
    /// The parts the archive holds. One made before they were recorded holds
    /// the profiles, the notes and the user's folder of notes, under the
    /// names of the parts that took them since.
    fn parts(&self) -> Vec<ManifestPart> {
        if let Some(parts) = &self.parts {
            return parts.clone();
        }
        let inside = |name: &str| ManifestPart {
            name: name.into(),
            path: name.into(),
            external: false,
        };
        let mut parts = vec![inside("profiles"), inside("notes")];
        if let Some(dir) = &self.notes_custom_dir {
            parts.push(ManifestPart {
                name: veydan_notes::BACKUP_CUSTOM_DIR.into(),
                path: dir.clone(),
                external: true,
            });
        }
        parts
    }
}

/// Where a restore puts the folders of an archive.
#[derive(Debug, Default, PartialEq, Eq)]
struct Placement {
    /// Folder of the archive → the entry of the data directory it replaces.
    swapped: Vec<(String, PathBuf)>,
    /// Folder of the archive → the folder outside the data directory its
    /// files are copied into.
    copied: Vec<(String, PathBuf)>,
}

impl Placement {
    fn target_of(&self, name: &str) -> Option<&Path> {
        self.copied
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, target)| target.as_path())
    }
}

/// Where the parts `archived` go in `data_dir`, given the parts the product
/// keeps there now (`live`). A part inside the data directory replaces the
/// entry of the product's part of that name; one the product no longer has
/// is not restored, and an entry the archive does not hold (the messenger's
/// folder under an archive made before stage 10) stays as it is. A part
/// outside goes back to its folder when the product still uses that folder;
/// otherwise into the data directory under its name, so a foreign archive
/// cannot write to an arbitrary path.
fn placement(data_dir: &Path, live: &[BackupPath], archived: &[ManifestPart]) -> Placement {
    let mut place = Placement::default();
    for part in archived {
        // The folders of the pinned tools hold code the app runs: never written.
        if !is_part_name(&part.name) || matches!(part.name.as_str(), "tor" | "camoufox") {
            continue;
        }
        let mine = live
            .iter()
            .find(|l| l.name == part.name && l.external == part.external);
        if part.external {
            let target = match mine {
                Some(l) if l.path == Path::new(&part.path) => l.path.clone(),
                // The fallback is for the parts the app promises (a live
                // one, or the folder of notes), and only into a place that
                // holds nothing.
                _ => {
                    let fallback = data_dir.join(&part.name);
                    let promised = live.iter().any(|l| l.name == part.name)
                        || part.name == veydan_notes::BACKUP_CUSTOM_DIR;
                    let empty = std::fs::read_dir(&fallback)
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(!fallback.exists());
                    if !promised || !empty {
                        eprintln!(
                            "backup restore: part {:?} has no place to go, skipped",
                            part.name
                        );
                        continue;
                    }
                    fallback
                }
            };
            place.copied.push((part.name.clone(), target));
        } else if let Some(l) = mine.filter(|l| is_part_name(&l.path.to_string_lossy())) {
            place.swapped.push((part.name.clone(), l.path.clone()));
        }
    }
    place
}

/// The archive `path` names, when it is one the list of backups shows: a
/// file of this format right in the folder of the backups. A restore takes
/// no other file.
fn listed_archive(dir: Option<&str>, path: &str) -> CmdResult<PathBuf> {
    let refused = || AppError::other("Not a backup of the backup folder");
    let dir = std::fs::canonicalize(dir.ok_or_else(refused)?).map_err(|_| refused())?;
    let file = std::fs::canonicalize(path).map_err(|_| refused())?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if file.parent() != Some(dir.as_path()) || !is_backup_name(name) || !file.is_file() {
        return Err(refused());
    }
    Ok(file)
}

/// A data file joined to a vault leaves it before it is put in place. Its
/// device restarts with a new id, and the first cycle would read what the
/// device published since the backup — the tombstones of a clear of the
/// data, say — as another device's, newer than the restored rows, and
/// delete them again. Joined again in the settings, the device publishes
/// its rows anew. True when the file was joined.
async fn leave_vault(
    staging: &Path,
    registry: &veydan_sync_host::Registry,
) -> Result<bool, String> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(veydan_core::db::options(&staging.join(DB_FILE)))
        .await
        .map_err(|e| format!("Cannot open restored DB: {e}"))?;
    let joined = veydan_sync_host::config::load_binding(&pool)
        .await
        .is_some();
    let left = match joined {
        true => veydan_sync_host::join::leave(&pool, registry)
            .await
            .map_err(|e| e.to_string()),
        false => Ok(()),
    };
    pool.close().await;
    left.map(|()| joined)
}

/// Whether an archive made at `archive` is older than the newest one made
/// of the data now in place (`LAST_CREATED_KEY`): a copy put in the place
/// of a fresher one would turn the data back without a word. Both are the
/// `created_at` of a manifest; one that does not read is no reason to ask.
fn older_than_the_data(archive: &str, newest: Option<&str>) -> bool {
    let Some(newest) = newest else { return false };
    match (
        DateTime::parse_from_rfc3339(archive),
        DateTime::parse_from_rfc3339(newest),
    ) {
        (Ok(archive), Ok(newest)) => archive < newest,
        _ => false,
    }
}

/// Restore from a `.vbk2` file of the list, then relaunch the app so the
/// fresh data is picked up cleanly. Never returns on success (process
/// restarts). Behind an open lock: the restore replaces every secret. An
/// archive older than the newest one made of the data now in place is
/// refused with `backup_older:<its date>|<the newest>` unless
/// `accept_older`: the window asks, and asks again with it.
#[tauri::command]
pub async fn backup_restore(
    path: String,
    password: Option<String>,
    accept_older: Option<bool>,
    app: AppHandle,
) -> CmdResult<()> {
    if app.state::<veydan_lock::Lock>().is_locked().await {
        return Err(AppError::VaultLocked);
    }
    let core = app.state::<Core>();
    let shell = app.state::<Shell>();
    let data_dir = core.app_data_dir.clone();
    let config = load_config(&core.db).await;
    let path = listed_archive(config.dir.as_deref(), &path)?;

    // Empty input means "use the configured backup password".
    let password = match password.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => config
            .password
            .ok_or_else(|| AppError::other("No backup password configured"))?,
    };

    let staging = data_dir
        .join(RESTORE_STAGING)
        .join(uuid::Uuid::new_v4().to_string());

    // Decrypt + decompress + unpack into staging (blocking work).
    emit_restore_progress(&app, "decrypting", 0);
    let staging2 = staging.clone();
    let app2 = app.clone();
    let unpack_res = tauri::async_runtime::spawn_blocking(move || {
        extract_archive(Some(&app2), &path, &password, &staging2)
    })
    .await
    .map_err(AppError::other)?;

    if let Err(e) = unpack_res {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::other(e));
    }

    let manifest = match read_manifest(&staging).await {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::other(e));
        }
    };
    let newest = settings::get(&core.db, LAST_CREATED_KEY).await;
    if accept_older != Some(true) && older_than_the_data(&manifest.created_at, newest.as_deref()) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::Other(format!(
            "{OLDER_ERROR}{}|{}",
            manifest.created_at,
            newest.unwrap_or_default()
        )));
    }
    let place = placement(&data_dir, &shell.backup_paths(), &manifest.parts());
    let custom_target = place
        .target_of(veydan_notes::BACKUP_CUSTOM_DIR)
        .map(Path::to_path_buf);

    // The source machine's data dir may differ from ours: rewrite stored
    // absolute paths so profiles/notes resolve after the swap.
    emit_restore_progress(&app, "remapping", 95);
    if let Err(e) = remap_paths(&staging, &data_dir, custom_target.as_deref()) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::other(e));
    }
    // The dialog of the restore told a joined device it leaves its vault.
    let registry = app.state::<veydan_sync_host::SyncManager>();
    if let Err(e) = leave_vault(&staging, registry.registry()).await {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(AppError::other(e));
    }

    // Every handle on the live data goes before it is renamed: Windows
    // refuses to rename files and folders still open by SQLite, a running
    // browser, the notes directory watcher or the messenger's database.
    // The pool is closed at this point, so we restart even on failure —
    // swap_in has already rolled the live data back.
    emit_restore_progress(&app, "swapping", 98);
    let running = TheApp {
        shell: &shell,
        core: &core,
    };
    let (data_dir2, staging2) = (data_dir.clone(), staging.clone());
    let swap_res =
        release_and_swap(&running, move || swap_in(&data_dir2, &staging2, &place)).await;
    // The settings tell it after the restart.
    match swap_res {
        Ok(kept) => write_outcome(&data_dir, "restored", None, kept),
        Err(e) => {
            eprintln!("backup restore: swap failed, keeping current data: {e}");
            write_outcome(&data_dir, "failed", Some(e), Vec::new());
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    emit_restore_progress(&app, "restarting", 100);

    // Relaunch so the app rebinds to the restored data.
    // `restart()` diverges (-> !), so this is the function's tail expression.
    app.restart()
}

/// The manifest of an unpacked archive, if this version restores it: of
/// this format, with a data file this build opens in place of the live one.
async fn read_manifest(staging: &Path) -> Result<Manifest, String> {
    let manifest: Manifest = std::fs::read(staging.join(MANIFEST_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Invalid or corrupt backup (no manifest)")?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(format!(
            "Backup format v{} is not supported (this version reads v{})",
            manifest.format_version, FORMAT_VERSION
        ));
    }
    if !crate::db::is_data_file(&staging.join(DB_FILE)).await {
        return Err("Invalid or corrupt backup (no data file)".into());
    }
    Ok(manifest)
}

/// What a restore asks of the running app before it swaps the data.
trait Running {
    /// The app is on its way out, then `Module::stop` of every module, on
    /// or off, in the order of the product (`Shell::shutdown`, as on exit).
    async fn shutdown(&self);
    /// No connection to the data file is left open.
    async fn close_data(&self);
}

/// The running app: its shell and its core.
struct TheApp<'a> {
    shell: &'a Shell,
    core: &'a Core,
}

impl Running for TheApp<'_> {
    async fn shutdown(&self) {
        self.shell.shutdown().await;
    }

    async fn close_data(&self) {
        self.core.db.close().await;
    }
}

/// Stop the modules as on exit (internal/platform-spec.md 12; 19, № 59): the
/// browsers they started, the watcher of the notes, the runtime of the
/// messenger with its database; close the data file; then swap the data
/// under them.
async fn release_and_swap<F, T>(running: &impl Running, swap: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    running.shutdown().await;
    running.close_data().await;
    tauri::async_runtime::spawn_blocking(swap)
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Rewrite absolute paths inside the staged DB to point at `data_dir`.
/// Profiles always live at `profiles/<id>`; note files are matched by basename
/// against the staged `notes/documents`, custom-dir notes against the staged
/// `notes_custom` and remapped to `custom_target`.
fn remap_paths(
    staging: &Path,
    data_dir: &Path,
    custom_target: Option<&Path>,
) -> Result<(), String> {
    let conn = rusqlite::Connection::open(staging.join(DB_FILE))
        .map_err(|e| format!("Cannot open restored DB: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT id FROM profiles")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for id in ids {
        // The ids come from the archive: one plain name, as in `unpack_in`.
        if !is_plain_name(&id) {
            continue;
        }
        // The folder sync gave the profile: a hash for an id that is not a plain name.
        let path = crate::commands::profiles::crud::profile_dir_of(data_dir, &id)
            .to_string_lossy()
            .to_string();
        conn.execute(
            "UPDATE profiles SET profile_path = ?1 WHERE id = ?2",
            rusqlite::params![path, id],
        )
        .map_err(|e| e.to_string())?;
    }

    let staged_docs = staging.join("notes").join("documents");
    let docs_dir = data_dir.join("notes").join("documents");
    let mut stmt = conn
        .prepare("SELECT id, file_path FROM notes")
        .map_err(|e| e.to_string())?;
    let notes = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<(String, String)>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    let staged_custom = staging.join("notes_custom");
    for (id, file_path) in notes {
        // Basename split on both separators: the backup may come from Windows.
        let Some(name) = file_path
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        if !is_plain_name(name) {
            continue;
        }
        let path = if staged_docs.join(name).is_file() {
            docs_dir.join(name)
        } else if let Some(target) = custom_target.filter(|_| staged_custom.join(name).is_file()) {
            target.join(name)
        } else {
            continue;
        };
        conn.execute(
            "UPDATE notes SET file_path = ?1 WHERE id = ?2",
            rusqlite::params![path.to_string_lossy().to_string(), id],
        )
        .map_err(|e| e.to_string())?;
    }

    // The restored DB has a stale own-log position: it must push to a fresh
    // device log or peers see a chain break.
    conn.execute(
        "DELETE FROM app_settings WHERE key IN ('sync_device_id', 'sync_own_seq', 'sync_own_head', 'sync_own_pending')",
        [],
    )
    .map_err(|e| e.to_string())?;

    // The restored settings must not point at a path from another machine.
    match custom_target {
        Some(target) => conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('notes_custom_dir', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![target.to_string_lossy().to_string()],
        ),
        None => conn.execute(
            "DELETE FROM app_settings WHERE key = 'notes_custom_dir'",
            [],
        ),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn emit_restore_progress(app: &AppHandle, phase: &str, percent: u32) {
    let _ = app.emit(
        "backup://restore-progress",
        Progress {
            phase: phase.into(),
            percent,
        },
    );
}

/// What a restore unpacks at most: entries and bytes of the archive.
const MAX_ARCHIVE_ENTRIES: usize = 200_000;
const MAX_ARCHIVE_BYTES: u64 = 64 << 30;

/// Decrypt + decompress + unpack `src` into `dest`, reporting extraction
/// progress. `manifest.json` is the first tar entry, so its entry count gives
/// the total. Extraction spans 5..95% (decrypt before, remap/swap after).
fn extract_archive(
    app: Option<&AppHandle>,
    src: &Path,
    password: &str,
    dest: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let file = File::open(src).map_err(|e| e.to_string())?;
    let decryptor = age::Decryptor::new(file).map_err(|e| format!("Cannot read backup: {e}"))?;
    let mut identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    // age caps the accepted work factor relative to *this* machine's speed;
    // a backup made on a faster machine (or read by a debug build) would be
    // rejected as ExcessiveWork. Use a fixed cap instead.
    identity.set_max_work_factor(MAX_SCRYPT_WORK_FACTOR);
    let reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| match e {
            age::DecryptError::ExcessiveWork { .. } => {
                "Backup encryption is too expensive to unwrap on this machine".to_string()
            }
            _ => "Wrong password or corrupt backup".to_string(),
        })?;
    let zstd_reader = zstd::stream::read::Decoder::new(reader).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(zstd_reader);

    // manifest.json + the data file + entries; refined once the manifest is read.
    let mut total = 2usize;
    let mut done = 0usize;
    let mut last_pct = 0u32;
    let (mut entries, mut bytes) = (0usize, 0u64);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        // A backup holds files and folders only: a link could point a later
        // entry out of `dest`, a device or a fifo has no place in it.
        if !matches!(
            entry.header().entry_type(),
            tar::EntryType::Regular | tar::EntryType::Directory | tar::EntryType::Continuous
        ) {
            return Err("Backup holds an entry that is not a file or a folder".into());
        }
        entries += 1;
        bytes = bytes.saturating_add(entry.header().size().unwrap_or(0));
        if entries > MAX_ARCHIVE_ENTRIES || bytes > MAX_ARCHIVE_BYTES {
            return Err("Backup is larger than a restore accepts".into());
        }
        // `unpack_in` rejects `..` and absolute paths like `unpack` does.
        let unpacked = entry.unpack_in(dest).map_err(|e| e.to_string())?;
        done += 1;

        if unpacked && done == 1 {
            let is_manifest = entry
                .path()
                .map(|p| p.as_os_str() == "manifest.json")
                .unwrap_or(false);
            if is_manifest {
                if let Some(m) = std::fs::read(dest.join("manifest.json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok())
                {
                    total = m.entries.len() + 2;
                }
            }
        }

        if let Some(app) = app {
            let pct = 5 + pct(done, total.max(done)) * 90 / 100;
            if pct != last_pct {
                last_pct = pct;
                emit_restore_progress(app, "extracting", pct);
            }
        }
    }
    Ok(())
}

/// Names of the live data file's journals.
fn swapped_sidecars() -> impl Iterator<Item = String> {
    SWAPPED_SIDECARS
        .iter()
        .map(|sidecar| format!("{DB_FILE}{sidecar}"))
}

/// One folder outside the data directory a restore replaces: the staged
/// copy is made beside it, on the same file system, and two renames swap
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExternalSwap {
    /// The part of the archive.
    name: String,
    /// The folder of the part.
    dst: PathBuf,
    /// The staged copy, `<dst>.restore_new_<ts>`, until it takes the place of `dst`.
    new: PathBuf,
    /// The live folder, `<dst>.restore_old_<ts>`, once it has given its place up.
    old: PathBuf,
    /// Where the live folder stays once the swap is complete,
    /// `<dst>.before-restore-<date>`: a restore never deletes it.
    #[serde(default)]
    kept: PathBuf,
    /// A live folder stood at `dst` before the restore.
    had_live: bool,
}

/// What a swap moves besides the folder of the live entries
/// (`restore_backup_<ts>`), written before anything moves and removed once
/// the swap is complete or undone, so that the next start finishes or
/// undoes a swap the process did not live to end.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SwapMarker {
    ts: String,
    /// Entries of the data directory the swap places where no live one stood.
    #[serde(default)]
    added: Vec<PathBuf>,
    #[serde(default)]
    external: Vec<ExternalSwap>,
}

/// What the last restore came to, for the settings after the restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreOutcome {
    /// RFC3339 UTC.
    pub at: String,
    /// `restored`; `failed` (the swap failed and was undone); `undone` (it
    /// was cut short and the next start put the previous data back);
    /// `finished` (it was cut short after it was complete and the next
    /// start finished it).
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The previous folders outside the data directory, kept beside theirs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<String>,
}

/// Write `value` as JSON to `path` whole or not at all.
fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

fn read_marker(data_dir: &Path) -> Option<SwapMarker> {
    let bytes = std::fs::read(data_dir.join(SWAP_MARKER)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_outcome(data_dir: &Path, result: &str, error: Option<String>, kept: Vec<PathBuf>) {
    let outcome = RestoreOutcome {
        at: Utc::now().to_rfc3339(),
        result: result.into(),
        error,
        kept: kept
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    if let Err(e) = write_json_atomic(&data_dir.join(RESTORE_OUTCOME), &outcome) {
        eprintln!("backup restore: cannot record the outcome: {e}");
    }
}

/// The outcome of the last restore of this data directory, if any.
fn read_outcome(data_dir: &Path) -> Option<RestoreOutcome> {
    let bytes = std::fs::read(data_dir.join(RESTORE_OUTCOME)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A path beside `dst`: its name with `infix` and `ts` appended.
fn beside(dst: &Path, infix: &str, ts: &str) -> Option<PathBuf> {
    let name = dst.file_name()?.to_string_lossy().into_owned();
    Some(dst.with_file_name(format!("{name}{infix}{ts}")))
}

/// A free name beside `dst` for its previous folder:
/// `<dst>.before-restore-<date>`, with `-2`, `-3` … when that one is taken.
fn kept_beside(dst: &Path, ts: &str) -> Option<PathBuf> {
    let date = chrono::NaiveDateTime::parse_from_str(ts, "%Y%m%d-%H%M%S")
        .map(|d| d.format("%Y-%m-%d-%H%M%S").to_string())
        .unwrap_or_else(|_| ts.to_string());
    let first = beside(dst, KEPT_INFIX, &date)?;
    let name = first.file_name()?.to_string_lossy().into_owned();
    std::iter::once(first.clone())
        .chain((2..1000).map(|n| first.with_file_name(format!("{name}-{n}"))))
        .find(|p| p.symlink_metadata().is_err())
}

/// Undo a partial swap: the entries placed where no live one stood go,
/// then everything saved in `old` goes back to its live path (dropping
/// whatever staged entry may have been placed there meanwhile). `old`
/// itself goes only once nothing is left in it.
fn roll_back(data_dir: &Path, old: &Path, added: &[PathBuf]) {
    for entry in added {
        // Names of the data directory only, whatever the marker says.
        if !is_plain_name(&entry.to_string_lossy()) {
            continue;
        }
        let placed = data_dir.join(entry);
        let _ = std::fs::remove_dir_all(&placed);
        let _ = std::fs::remove_file(&placed);
    }
    if let Ok(entries) = std::fs::read_dir(old) {
        for entry in entries.flatten() {
            let live = data_dir.join(entry.file_name());
            let _ = std::fs::remove_dir_all(&live);
            let _ = std::fs::remove_file(&live);
            let _ = std::fs::rename(entry.path(), &live);
        }
    }
    let _ = std::fs::remove_dir(old);
}

/// Put the live folder of `ext` back. `moved`: the swap got as far as
/// moving the live entries, so a staged copy may stand at `dst`. True when
/// nothing of the swap is left but the live folder at its place.
fn undo_external(ext: &ExternalSwap, moved: bool) -> bool {
    let mut clean = true;
    if ext.old.symlink_metadata().is_ok() {
        // The live folder gave its place up: whatever stands there is the staged copy.
        if ext.dst.symlink_metadata().is_ok() {
            if let Err(e) = std::fs::remove_dir_all(&ext.dst) {
                eprintln!("backup restore: cannot remove {}: {e}", ext.dst.display());
                return false;
            }
        }
        if let Err(e) = rename_retry(&ext.old, &ext.dst) {
            eprintln!(
                "backup restore: cannot put {} back from {}: {e}",
                ext.dst.display(),
                ext.old.display()
            );
            return false;
        }
    } else if !ext.had_live && moved && ext.new.symlink_metadata().is_err() {
        // The staged copy took a place no live folder held.
        if ext.dst.symlink_metadata().is_ok() {
            clean &= std::fs::remove_dir_all(&ext.dst).is_ok();
        }
    }
    if ext.new.symlink_metadata().is_ok() {
        clean &= std::fs::remove_dir_all(&ext.new).is_ok();
    }
    clean
}

/// The external part of a complete swap: the staged copy stands at `dst`,
/// the previous folder stays beside it under the name of `kept`. Where the
/// previous folder is now, if one stood there.
fn finish_external(ext: &ExternalSwap) -> Option<PathBuf> {
    if ext.dst.symlink_metadata().is_err() && ext.new.symlink_metadata().is_ok() {
        if let Err(e) = rename_retry(&ext.new, &ext.dst) {
            eprintln!(
                "backup restore: cannot place {} from {}: {e}",
                ext.dst.display(),
                ext.new.display()
            );
        }
    }
    if ext.old.symlink_metadata().is_ok() {
        if ext.kept.as_os_str().is_empty() {
            return Some(ext.old.clone());
        }
        return match rename_retry(&ext.old, &ext.kept) {
            Ok(()) => Some(ext.kept.clone()),
            Err(e) => {
                eprintln!(
                    "backup restore: cannot rename {} to {}: {e}",
                    ext.old.display(),
                    ext.kept.display()
                );
                Some(ext.old.clone())
            }
        };
    }
    // Renamed already, by the run cut short after it.
    (ext.had_live && ext.kept.symlink_metadata().is_ok()).then(|| ext.kept.clone())
}

/// Undo the swap `marker` describes, wherever it stopped. The marker goes
/// once nothing of the swap is left; otherwise the next start tries again.
fn undo_swap(data_dir: &Path, marker: &SwapMarker) -> bool {
    let saved = data_dir.join(format!("{SAVED_PREFIX}{}", marker.ts));
    let moved = saved.exists();
    if moved {
        roll_back(data_dir, &saved, &marker.added);
    }
    let mut clean = !saved.exists();
    for ext in marker.external.iter().rev() {
        clean &= undo_external(ext, moved);
    }
    if clean {
        let _ = std::fs::remove_file(data_dir.join(SWAP_MARKER));
    }
    clean
}

/// Finish the swap `marker` describes, which is complete: the previous
/// folders outside the data directory stay beside theirs, the marker goes.
/// Where the previous folders are.
fn finish_swap(data_dir: &Path, marker: &SwapMarker) -> Vec<PathBuf> {
    let kept = marker.external.iter().filter_map(finish_external).collect();
    let _ = std::fs::remove_file(data_dir.join(SWAP_MARKER));
    kept
}

/// Where the staged part `ext.name` goes beside its folder: moved there
/// when staging and the folder share a file system, copied otherwise. The
/// copy takes the permissions of the live folder.
fn stage_beside(src: &Path, ext: &ExternalSwap) -> std::io::Result<()> {
    if let Some(parent) = ext.dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(src, &ext.new).is_err() {
        copy_dir(src, &ext.new)?;
    }
    if ext.had_live {
        if let Ok(meta) = std::fs::metadata(&ext.dst) {
            let _ = std::fs::set_permissions(&ext.new, meta.permissions());
        }
    }
    Ok(())
}

/// The swaps of the parts outside the data directory the archive holds.
/// A folder that holds the data directory or lies inside an entry the
/// swap replaces cannot be swapped on its own: refused.
fn plan_external(
    data_dir: &Path,
    staging: &Path,
    place: &Placement,
    swapped: &[(&str, &Path)],
    ts: &str,
) -> Result<Vec<ExternalSwap>, String> {
    let data_real = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let mut planned = Vec::new();
    for (name, dst) in &place.copied {
        if !staging.join(name).exists() {
            continue;
        }
        // A link to the folder keeps pointing at it: the folder itself is swapped.
        let dst = std::fs::canonicalize(dst).unwrap_or_else(|_| dst.clone());
        let overlaps = data_dir.starts_with(&dst)
            || data_real.starts_with(&dst)
            || swapped.iter().any(|(_, entry)| {
                dst.starts_with(data_dir.join(entry)) || dst.starts_with(data_real.join(entry))
            });
        let refused = || format!("Cannot restore {name} into {}", dst.display());
        if overlaps {
            return Err(refused());
        }
        let (Some(new), Some(old)) = (beside(&dst, NEW_INFIX, ts), beside(&dst, OLD_INFIX, ts))
        else {
            return Err(refused());
        };
        // What stands beside the folder under these names is not the swap's:
        // an undo would take it for the live folder.
        if new.symlink_metadata().is_ok() || old.symlink_metadata().is_ok() {
            return Err(refused());
        }
        let kept = kept_beside(&dst, ts).ok_or_else(refused)?;
        planned.push(ExternalSwap {
            name: name.clone(),
            had_live: dst.symlink_metadata().is_ok(),
            dst,
            new,
            old,
            kept,
        });
    }
    Ok(planned)
}

/// Replace `app.db` and the entries of the parts in the live data dir, and
/// the folders of the parts outside it, with the staged versions, keeping
/// the live ones until the swap succeeds; the previous folders outside stay
/// beside theirs, and the result says where.
fn swap_in(data_dir: &Path, staging: &Path, place: &Placement) -> Result<Vec<PathBuf>, String> {
    let ts = Local::now().format("%Y%m%d-%H%M%S").to_string();
    swap_in_at(data_dir, staging, place, &ts)
}

/// `swap_in` with the stamp of its folders given.
///
/// The order: the marker; the staged copies of the parts outside beside
/// their folders; the live journals and entries into `restore_backup_<ts>`;
/// the staged entries into place; each folder outside swapped with its copy
/// by two renames; then one rename of `restore_backup_<ts>` into
/// `restore_done_<ts>` says the swap is complete. Any error before that
/// undoes everything; an interruption is undone or finished at the next
/// start (`finish_interrupted_restore`) by the marker. After it the previous
/// folder outside is renamed `<dst>.before-restore-<date>` and stays: a
/// file made in it after the backup is not lost. Where they are.
fn swap_in_at(
    data_dir: &Path,
    staging: &Path,
    place: &Placement,
    ts: &str,
) -> Result<Vec<PathBuf>, String> {
    let old = data_dir.join(format!("{SAVED_PREFIX}{ts}"));
    // The data file, then each part: (folder of the archive, live entry).
    let swapped: Vec<(&str, &Path)> = std::iter::once((DB_FILE, Path::new(DB_FILE)))
        .chain(
            place
                .swapped
                .iter()
                .map(|(name, path)| (name.as_str(), path.as_path())),
        )
        .collect();
    let external = plan_external(data_dir, staging, place, &swapped, ts)?;
    let added = swapped
        .iter()
        .filter(|(name, entry)| staging.join(name).exists() && !data_dir.join(entry).exists())
        .map(|(_, entry)| entry.to_path_buf())
        .collect();
    let marker = SwapMarker {
        ts: ts.to_string(),
        added,
        external,
    };
    // Before anything moves.
    write_json_atomic(&data_dir.join(SWAP_MARKER), &marker)
        .map_err(|e| format!("Cannot start the swap: {e}"))?;
    let rollback = |err: String| {
        undo_swap(data_dir, &marker);
        err
    };

    // The parts outside the data dir: a whole staged copy beside each folder.
    for ext in &marker.external {
        stage_beside(&staging.join(&ext.name), ext)
            .map_err(|e| rollback(format!("Cannot copy {}: {e}", ext.name)))?;
    }

    std::fs::create_dir_all(&old).map_err(|e| rollback(e.to_string()))?;
    // Move current → old. Also relocate the live DB's WAL/SHM sidecars: leaving
    // a stale `-wal` next to the freshly restored `app.db` would let SQLite
    // replay mismatched frames and corrupt it.
    for sidecar in swapped_sidecars() {
        let live = data_dir.join(&sidecar);
        if live.exists() {
            rename_retry(&live, &old.join(&sidecar))
                .map_err(|e| rollback(format!("Cannot move {sidecar}: {e}")))?;
        }
    }
    // A live entry moves out only for one the archive holds: a part it
    // recorded but holds no folder of (an archive cut short, one made
    // before the folders were written) stays as it is.
    for (name, entry) in &swapped {
        let live = data_dir.join(entry);
        if live.exists() && staging.join(name).exists() {
            rename_retry(&live, &old.join(entry))
                .map_err(|e| rollback(format!("Cannot move {}: {e}", entry.display())))?;
        }
    }
    // Move staged → live (only what the backup actually contains).
    for (name, entry) in &swapped {
        let staged = staging.join(name);
        if staged.exists() {
            rename_retry(&staged, &data_dir.join(entry))
                .map_err(|e| rollback(format!("Cannot place {name}: {e}")))?;
        }
    }
    // The folders outside: the live one aside, the staged copy in its place.
    for ext in &marker.external {
        if ext.had_live {
            rename_retry(&ext.dst, &ext.old)
                .map_err(|e| rollback(format!("Cannot move {}: {e}", ext.dst.display())))?;
        }
        #[cfg(test)]
        if tests::FAIL_TO_PLACE.with(|f| f.borrow().as_deref() == Some(ext.name.as_str())) {
            return Err(rollback(format!(
                "Cannot place {}: refused by the test",
                ext.name
            )));
        }
        rename_retry(&ext.new, &ext.dst)
            .map_err(|e| rollback(format!("Cannot place {}: {e}", ext.name)))?;
    }
    // The swap is complete. One rename says so to the next start, which
    // otherwise takes the folder for a swap cut short and undoes it.
    let done = data_dir.join(format!("{DONE_PREFIX}{ts}"));
    std::fs::rename(&old, &done).map_err(|e| rollback(format!("Cannot finish the swap: {e}")))?;

    // Success — the previous folders outside stay beside theirs; the marker,
    // then the safety copy go.
    let kept = finish_swap(data_dir, &marker);
    let _ = std::fs::remove_dir_all(&done);
    Ok(kept)
}

/// Clean up after a restore or a backup the process did not live to finish.
/// Call before the data file is opened.
///
/// A swap cut short left the live entries in its `restore_backup_*` folder;
/// they go back, as after a swap that failed. Without this the start would
/// find no data file and create an empty one over a user's data that still
/// lies in that folder. The marker of the swap says what else it moved:
/// with its `restore_done_*` folder the swap was complete and its folders
/// outside are finished, without it they are put back as well; the outcome
/// is recorded for the settings.
pub fn finish_interrupted_restore(data_dir: &Path) {
    // A marker cut short while written: nothing had moved yet.
    let _ = std::fs::remove_file(data_dir.join(SWAP_MARKER).with_extension("json.tmp"));
    if let Some(marker) = read_marker(data_dir) {
        if data_dir
            .join(format!("{DONE_PREFIX}{}", marker.ts))
            .is_dir()
        {
            eprintln!("backup restore: was interrupted after the swap; finishing it");
            let kept = finish_swap(data_dir, &marker);
            write_outcome(data_dir, "finished", None, kept);
        } else {
            eprintln!("backup restore: was interrupted; putting the data back");
            undo_swap(data_dir, &marker);
            write_outcome(data_dir, "undone", None, Vec::new());
        }
    }
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return;
    };
    let mut cut_short: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if name.starts_with(SAVED_PREFIX) {
            cut_short.push(path);
        } else if name.starts_with(DONE_PREFIX) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
    // Oldest first, so the entries of the latest swap are the ones that stay.
    cut_short.sort();
    for old in &cut_short {
        eprintln!(
            "backup restore: was interrupted; putting the data back from {}",
            old.display()
        );
        roll_back(data_dir, old, &[]);
    }
    // No restore runs at start: whatever is unpacked there is a leftover.
    let _ = std::fs::remove_dir_all(data_dir.join(RESTORE_STAGING));
    // Nor does a backup: a snapshot of the data file left by one cut short
    // is a plain copy of every secret.
    let _ = std::fs::remove_dir_all(data_dir.join(BACKUP_STAGING));
}

/// `fs::rename` with retries: on Windows a just-killed browser or an AV
/// scanner can hold a handle for a moment and fail the rename with a
/// sharing violation.
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    const ATTEMPTS: u32 = 15;
    const DELAY: std::time::Duration = std::time::Duration::from_millis(200);
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt + 1 < ATTEMPTS => {
                attempt += 1;
                eprintln!(
                    "backup restore: rename {} failed ({e}), retry {attempt}",
                    from.display()
                );
                std::thread::sleep(DELAY);
            }
            Err(e) => return Err(e),
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

// ── Scheduler ───────────────────────────────────────────────────────────────

/// Spawn the background scheduler. The first tick fires immediately so a backup
/// missed while the app was closed is caught up on startup.
pub fn start_backup_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // No backup runs yet: an archive under its `.part` name is one the
        // last run of the app did not finish.
        if let Some(dir) = load_config(&app.state::<Core>().db).await.dir {
            sweep_parts(Path::new(&dir));
        }
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let core = app.state::<Core>();
            let backup = app.state::<BackupManager>();
            let cfg = load_config(&core.db).await;
            if !cfg.schedule_enabled || cfg.dir.is_none() || cfg.password.is_none() {
                continue;
            }
            if backup.running.load(Ordering::SeqCst) {
                continue;
            }
            let last_run = cfg
                .last_run
                .as_deref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));
            if is_due(&cfg, last_run, Local::now()) {
                if let Err(e) = perform_backup(&app).await {
                    let _ = app.emit("backup://error", e);
                }
            }
        }
    });
}

/// Whether a scheduled backup is due right now given the last run.
fn is_due(cfg: &BackupConfig, last_run: Option<DateTime<Utc>>, now_local: DateTime<Local>) -> bool {
    match cfg.schedule_mode.as_str() {
        "interval" => {
            let hours = cfg.interval_hours.max(1);
            match last_run {
                None => true,
                // Derive "now" from the injected clock so tests control it.
                Some(lr) => now_local.with_timezone(&Utc) - lr >= chrono::Duration::hours(hours),
            }
        }
        "daily" | "weekly" => {
            let (h, m) = parse_hhmm(&cfg.time);
            let scheduled = if cfg.schedule_mode == "weekly" {
                last_scheduled_weekly(now_local, cfg.weekday, h, m)
            } else {
                last_scheduled_daily(now_local, h, m)
            };
            match scheduled {
                None => false,
                Some(sched) => {
                    let sched_utc = sched.with_timezone(&Utc);
                    last_run.is_none_or(|lr| lr < sched_utc)
                }
            }
        }
        _ => false,
    }
}

fn parse_hhmm(s: &str) -> (u32, u32) {
    let mut parts = s.split(':');
    let h = parts.next().and_then(|v| v.parse().ok()).unwrap_or(3);
    let m = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    (h.min(23), m.min(59))
}

/// Most recent local datetime at HH:MM that is <= now (today, or yesterday).
fn last_scheduled_daily(now: DateTime<Local>, h: u32, m: u32) -> Option<DateTime<Local>> {
    let today = now.date_naive().and_hms_opt(h, m, 0)?;
    let today_l = Local.from_local_datetime(&today).single()?;
    if today_l <= now {
        Some(today_l)
    } else {
        Local
            .from_local_datetime(&(today - chrono::Duration::days(1)))
            .single()
    }
}

/// Most recent local datetime on `weekday` (0=Mon..6=Sun) at HH:MM that is <= now.
fn last_scheduled_weekly(
    now: DateTime<Local>,
    weekday: i64,
    h: u32,
    m: u32,
) -> Option<DateTime<Local>> {
    let target = (weekday.rem_euclid(7)) as u32;
    let mut date = now.date_naive();
    for _ in 0..8 {
        if date.weekday().num_days_from_monday() == target {
            if let Some(naive) = date.and_hms_opt(h, m, 0) {
                if let Some(dt) = Local.from_local_datetime(&naive).single() {
                    if dt <= now {
                        return Some(dt);
                    }
                }
            }
        }
        date = date.pred_opt()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn tmp_root(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("vb_backup_test_{tag}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A real tar→zstd→age backup can be decrypted+extracted with the right
    /// password and reproduces the original files byte-for-byte.
    #[test]
    fn archive_roundtrip_reproduces_files() {
        let root = tmp_root("rt");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("profiles/p1/firefox-profile")).unwrap();
        std::fs::write(
            src.join("profiles/p1/firefox-profile/prefs.js"),
            b"user_pref(1);",
        )
        .unwrap();
        std::fs::write(
            src.join("profiles/p1/firefox-profile/startupCache"),
            b"CACHE",
        )
        .unwrap(); // blacklisted
        std::fs::create_dir_all(src.join("notes")).unwrap();
        std::fs::write(src.join("notes/hello.md"), b"# hello \xE2\x9C\x93").unwrap();

        let db = root.join("db.sqlite");
        std::fs::write(&db, b"SQLITE-SNAPSHOT-BYTES").unwrap();

        let mut files = Vec::new();
        collect_files(&src.join("profiles"), "profiles", &mut files).unwrap();
        collect_files(&src.join("notes"), "notes", &mut files).unwrap();

        // Blacklisted cache must be excluded.
        assert!(files.iter().all(|(n, _)| !n.contains("startupCache")));

        let out = root.join("backup.vbk");
        let manifest = test_manifest(&files);
        write_archive(None, &out, "s3cret", &manifest, &db, &[], &files).expect("write");
        assert!(out.exists() && std::fs::metadata(&out).unwrap().len() > 0);

        let dest = root.join("restored");
        extract_archive(None, &out, "s3cret", &dest).expect("extract");

        assert_eq!(
            std::fs::read(dest.join(DB_FILE)).unwrap(),
            b"SQLITE-SNAPSHOT-BYTES"
        );
        assert_eq!(
            std::fs::read(dest.join("profiles/p1/firefox-profile/prefs.js")).unwrap(),
            b"user_pref(1);"
        );
        assert_eq!(
            std::fs::read(dest.join("notes/hello.md")).unwrap(),
            "# hello ✓".as_bytes()
        );
        assert!(!dest
            .join("profiles/p1/firefox-profile/startupCache")
            .exists());

        let mut mf = String::new();
        File::open(dest.join("manifest.json"))
            .unwrap()
            .read_to_string(&mut mf)
            .unwrap();
        assert!(mf.contains("format_version"));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// What a backup stores as `app.db` is a data file of ours, and the
    /// restore points its rows at the data directory it lands in.
    #[tokio::test]
    async fn db_snapshot_keeps_the_marker_and_is_remapped() {
        let (db, source) = crate::db::test_pool().await;
        for sql in [
            "INSERT INTO profiles (id, name, profile_path, workspace_id, created_at, updated_at)
             VALUES ('p1', 'Brand A', '/old/data/profiles/p1', 'default', 't', 't')",
            "INSERT INTO notes (id, title, file_path, created_at, updated_at)
             VALUES ('n1', 'Deploy', '/old/data/notes/documents/n1.md', 't', 't')",
            "INSERT INTO app_settings (key, value) VALUES ('sync_device_id', 'dev-a'), ('sync_own_pending', '{}'),
             ('ui_locale', 'ru')",
        ] {
            sqlx::query(sql).execute(&db).await.unwrap();
        }

        let staging = source.join("staging");
        std::fs::create_dir_all(staging.join("notes/documents")).unwrap();
        std::fs::write(staging.join("notes/documents/n1.md"), b"# Deploy").unwrap();
        snapshot_db(&db, &staging.join(DB_FILE)).await.unwrap();
        db.close().await;
        assert!(crate::db::is_data_file(&staging.join(DB_FILE)).await);

        let target = source.join("target");
        remap_paths(&staging, &target, None).unwrap();
        // The remap goes through another SQLite connection; the marker stays.
        assert!(crate::db::is_data_file(&staging.join(DB_FILE)).await);

        let restored = crate::db::open(&staging.join(DB_FILE)).await.unwrap();
        let profile_path: String =
            sqlx::query_scalar("SELECT profile_path FROM profiles WHERE id = 'p1'")
                .fetch_one(&restored)
                .await
                .unwrap();
        assert_eq!(
            PathBuf::from(profile_path),
            target.join("profiles").join("p1")
        );
        let file_path: String = sqlx::query_scalar("SELECT file_path FROM notes WHERE id = 'n1'")
            .fetch_one(&restored)
            .await
            .unwrap();
        assert_eq!(
            PathBuf::from(file_path),
            target.join("notes").join("documents").join("n1.md")
        );
        let settings: Vec<String> = sqlx::query_scalar("SELECT key FROM app_settings ORDER BY key")
            .fetch_all(&restored)
            .await
            .unwrap();
        assert_eq!(settings, vec!["ui_locale".to_string()]);
        restored.close().await;
        let _ = std::fs::remove_dir_all(&source);
    }

    /// A restored data file that was joined to a vault leaves it: no
    /// binding, no states of rows, sync off; the rest of its settings stay.
    #[tokio::test]
    async fn a_restored_data_file_leaves_its_vault() {
        let (db, source) = crate::db::test_pool().await;
        for sql in [
            "INSERT INTO app_settings (key, value) VALUES
             ('sync_vault_id', 'v1'), ('sync_vault_key', 'a2V5'), ('sync_enabled', '1'), ('ui_locale', 'ru')",
            "INSERT INTO sync_row_state (entity_type, entity_id, head_hlc, synced_hash)
             VALUES ('password', 'p1', 'h', 's')",
        ] {
            sqlx::query(sql).execute(&db).await.unwrap();
        }
        let staging = source.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        snapshot_db(&db, &staging.join(DB_FILE)).await.unwrap();
        db.close().await;

        let registry = veydan_sync_host::Registry::new();
        assert!(leave_vault(&staging, &registry).await.unwrap());
        assert!(!leave_vault(&staging, &registry).await.unwrap());
        assert!(crate::db::is_data_file(&staging.join(DB_FILE)).await);

        let restored = crate::db::open(&staging.join(DB_FILE)).await.unwrap();
        assert!(veydan_sync_host::config::load_binding(&restored)
            .await
            .is_none());
        assert_eq!(
            settings::get(&restored, "sync_enabled").await.as_deref(),
            Some("0")
        );
        assert_eq!(
            settings::get(&restored, "ui_locale").await.as_deref(),
            Some("ru")
        );
        let states: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_row_state")
            .fetch_one(&restored)
            .await
            .unwrap();
        assert_eq!(states, 0);
        restored.close().await;
        let _ = std::fs::remove_dir_all(&source);
    }

    /// A restore takes a file of the list alone: an archive of this format
    /// right in the folder of the backups, however its path is spelled.
    #[test]
    fn a_restore_takes_only_an_archive_of_the_backup_folder() {
        let root = tmp_root("listed");
        let dir = root.join("backups");
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        let name = format!("{FILE_PREFIX}2026-10-14.{FILE_EXT}");
        let good = dir.join(&name);
        let other_format = dir.join(format!("{FILE_PREFIX}2026-10-14.vbk1"));
        let elsewhere = root.join(&name);
        let nested = dir.join("inner").join(&name);
        for file in [&good, &other_format, &elsewhere, &nested] {
            std::fs::write(file, b"x").unwrap();
        }
        let folder = dir.to_str();
        let at = |p: &Path| listed_archive(folder, p.to_str().unwrap());

        assert_eq!(at(&good).unwrap(), std::fs::canonicalize(&good).unwrap());
        assert!(at(&dir.join("inner").join("..").join(&name)).is_ok());
        for refused in [
            other_format,
            elsewhere,
            nested,
            dir.join("..").join(&name),
            dir.join(format!("{FILE_PREFIX}missing.{FILE_EXT}")),
            dir.clone(),
        ] {
            assert!(at(&refused).is_err(), "{refused:?}");
        }
        assert!(listed_archive(None, good.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_extension_carries_the_format_version() {
        assert_eq!(FILE_EXT, format!("vbk{FORMAT_VERSION}"));
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Archives of 4.x sit in the same folder under the same prefix. This
    /// version cannot restore them, so it neither offers nor rotates them out.
    #[test]
    fn archives_of_another_format_are_left_alone() {
        let root = tmp_root("fmt");
        for name in [
            "veydan-backup-20260101-030000.vbk",
            "veydan-backup-20260102-030000.vbk",
            "veydan-backup-20261001-030000.vbk2",
            "veydan-backup-20261002-030000.vbk2",
            "veydan-backup-20261003-030000.vbk2",
            "veydan-backup-20261004-030000.vbk2.part",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }

        let listed: Vec<String> = list_backups(&root).into_iter().map(|b| b.name).collect();
        assert_eq!(
            listed,
            [
                "veydan-backup-20261003-030000.vbk2",
                "veydan-backup-20261002-030000.vbk2",
                "veydan-backup-20261001-030000.vbk2",
            ]
        );

        rotate(&root, 1);
        assert_eq!(
            names_in(&root),
            [
                "veydan-backup-20260101-030000.vbk",
                "veydan-backup-20260102-030000.vbk",
                "veydan-backup-20261003-030000.vbk2",
                "veydan-backup-20261004-030000.vbk2.part",
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn put(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_swap_replaces_the_live_entries_and_leaves_nothing_behind() {
        let data = tmp_root("swap");
        put(&data.join(DB_FILE), "live db");
        put(&data.join("app.db-wal"), "live wal");
        put(&data.join("profiles/p1/prefs.js"), "live");
        put(&data.join("notes/documents/n1.md"), "live");
        put(&data.join("install.id"), "kept");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("profiles/p2/prefs.js"), "restored");
        put(&staging.join("notes/documents/n2.md"), "restored");

        swap_in(&data, &staging, &space_place()).unwrap();

        assert_eq!(text(&data.join(DB_FILE)), "restored db");
        assert_eq!(text(&data.join("profiles/p2/prefs.js")), "restored");
        assert_eq!(text(&data.join("notes/documents/n2.md")), "restored");
        assert!(!data.join("profiles/p1").exists());
        assert!(!data.join("notes/documents/n1.md").exists());
        assert_eq!(
            names_in(&data),
            ["app.db", "install.id", "notes", "profiles", RESTORE_STAGING]
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    /// The process died after the live entries were moved out and before the
    /// staged ones were all in place.
    #[test]
    fn a_swap_cut_short_is_undone_at_the_next_start() {
        // Nothing staged was placed yet, `notes` was not yet moved out.
        let data = tmp_root("cut1");
        let old = data.join(format!("{SAVED_PREFIX}20261002-120000"));
        put(&old.join(DB_FILE), "live db");
        put(&old.join("profiles/p1/prefs.js"), "live");
        put(&data.join("notes/documents/n1.md"), "live");
        put(
            &data.join(RESTORE_STAGING).join("s1").join(DB_FILE),
            "restored db",
        );

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(text(&data.join("profiles/p1/prefs.js")), "live");
        assert_eq!(text(&data.join("notes/documents/n1.md")), "live");
        assert_eq!(names_in(&data), ["app.db", "notes", "profiles"]);
        let _ = std::fs::remove_dir_all(&data);

        // Everything was moved out and only the staged data file was placed.
        let data = tmp_root("cut2");
        let old = data.join(format!("{SAVED_PREFIX}20261002-120000"));
        put(&old.join(DB_FILE), "live db");
        put(&old.join("app.db-wal"), "live wal");
        put(&old.join("profiles/p1/prefs.js"), "live");
        put(&old.join("notes/documents/n1.md"), "live");
        put(&data.join(DB_FILE), "restored db");
        put(
            &data.join(RESTORE_STAGING).join("s1/profiles/p2/prefs.js"),
            "restored",
        );

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(text(&data.join("app.db-wal")), "live wal");
        assert_eq!(text(&data.join("profiles/p1/prefs.js")), "live");
        assert_eq!(text(&data.join("notes/documents/n1.md")), "live");
        assert_eq!(
            names_in(&data),
            ["app.db", "app.db-wal", "notes", "profiles"]
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    /// The process died after the swap was complete, while removing what it
    /// had moved aside: the restored data stays.
    #[test]
    fn a_complete_swap_is_not_undone_at_the_next_start() {
        let data = tmp_root("done");
        put(&data.join(DB_FILE), "restored db");
        put(&data.join("notes/documents/n2.md"), "restored");
        let done = data.join(format!("{DONE_PREFIX}20261002-120000"));
        put(&done.join(DB_FILE), "previous db");
        put(&done.join("notes/documents/n1.md"), "previous");

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "restored db");
        assert_eq!(text(&data.join("notes/documents/n2.md")), "restored");
        assert_eq!(names_in(&data), ["app.db", "notes"]);
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Without the clean-up the start would find no data file and create an
    /// empty one beside the user's rows.
    #[tokio::test]
    async fn after_a_swap_cut_short_the_start_opens_the_previous_rows() {
        let (db, data) = crate::db::test_pool().await;
        sqlx::query("INSERT INTO app_settings (key, value) VALUES ('ui_locale', 'ru')")
            .execute(&db)
            .await
            .unwrap();
        db.close().await;
        let old = data.join(format!("{SAVED_PREFIX}20261002-120000"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::rename(data.join(DB_FILE), old.join(DB_FILE)).unwrap();

        finish_interrupted_restore(&data);
        let db = crate::db::open(&data.join(DB_FILE)).await.unwrap();

        let value: String =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'ui_locale'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(value, "ru");
        db.close().await;
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn wrong_password_fails() {
        let root = tmp_root("wp");
        let db = root.join("db");
        std::fs::write(&db, b"x").unwrap();
        let out = root.join("b.vbk");
        write_archive(None, &out, "right", &test_manifest(&[]), &db, &[], &[]).unwrap();

        let dest = root.join("out");
        let err = extract_archive(None, &out, "wrong", &dest);
        assert!(err.is_err(), "wrong password must fail");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A link in the archive is refused: it could send a later entry out of
    /// the staging folder.
    #[test]
    fn an_archive_with_a_symlink_is_refused() {
        let root = tmp_root("symlink");
        let out = root.join("b.vbk");
        {
            let file = File::create(&out).unwrap();
            let w = age::Encryptor::with_user_passphrase(SecretString::from("pw".to_owned()))
                .wrap_output(file)
                .unwrap();
            let z = zstd::stream::write::Encoder::new(w, 3).unwrap();
            let mut tar = tar::Builder::new(z);
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            tar.append_link(&mut h, "evil", "/etc").unwrap();
            let z = tar.into_inner().unwrap();
            z.finish().unwrap().finish().unwrap();
        }
        let dest = root.join("out");
        assert!(extract_archive(None, &out, "pw", &dest).is_err());
        assert!(!dest.join("evil").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn interval_due_logic() {
        let cfg = BackupConfig {
            schedule_mode: "interval".into(),
            interval_hours: 24,
            ..Default::default()
        };
        let now = Local::now();
        assert!(is_due(&cfg, None, now), "never-run is due");
        assert!(is_due(
            &cfg,
            Some(Utc::now() - chrono::Duration::hours(25)),
            now
        ));
        assert!(!is_due(
            &cfg,
            Some(Utc::now() - chrono::Duration::hours(1)),
            now
        ));
    }

    #[test]
    fn daily_catches_up_missed_run() {
        // Daily at 03:00; last run 2 days ago → the most recent 03:00 is later
        // than last_run, so a catch-up is due.
        let cfg = BackupConfig {
            schedule_mode: "daily".into(),
            time: "03:00".into(),
            ..Default::default()
        };
        let now = Local::now();
        assert!(is_due(
            &cfg,
            Some(Utc::now() - chrono::Duration::days(2)),
            now
        ));
        // A run at the current instant is at/after the last scheduled 03:00,
        // so nothing is due.
        assert!(!is_due(&cfg, Some(Utc::now()), now));
    }

    /// What a restore of Space puts into the data directory: the folders of
    /// browser and notes.
    fn space_place() -> Placement {
        Placement {
            swapped: vec![
                ("profiles".into(), PathBuf::from("profiles")),
                ("notes".into(), PathBuf::from("notes")),
            ],
            copied: vec![],
        }
    }

    /// Every file under `dir` with its bytes, by its path inside `dir`.
    fn tree(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let name = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                    out.insert(name, std::fs::read(&path).unwrap());
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// The paths of the parts of Space's modules, as the shell hands them
    /// to the driver, on a mock app whose notes live in `custom`.
    fn space_parts(custom: &Path) -> Vec<BackupPath> {
        let app = tauri::test::mock_app();
        app.manage(veydan_notes::NotesState::new(Some(custom.to_path_buf())));
        crate::modules::backup_parts()
            .into_iter()
            .flat_map(|(_, part)| (part.paths)(app.handle()))
            .collect()
    }

    /// A database of a part that is open and writing while the backup runs
    /// — rows still in its `-wal` — lands in the archive whole, as one file
    /// that opens with every row, and without its journals.
    #[tokio::test]
    async fn a_live_database_of_a_part_is_captured_whole() {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        let (db, data) = crate::db::test_pool().await;
        let live = data.join("messenger/messenger.db");
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        let messenger = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&live)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal),
            )
            .await
            .unwrap();
        sqlx::raw_sql("PRAGMA wal_autocheckpoint = 0; CREATE TABLE m (id INTEGER PRIMARY KEY, body TEXT)")
            .execute(&messenger)
            .await
            .unwrap();
        for i in 0..50 {
            sqlx::query("INSERT INTO m (body) VALUES (?)")
                .bind(format!("message {i}"))
                .execute(&messenger)
                .await
                .unwrap();
        }
        assert!(data.join("messenger/messenger.db-wal").metadata().unwrap().len() > 0);
        std::fs::write(data.join("messenger/media.bin"), [1u8; 10]).unwrap();
        let parts = [BackupPath {
            name: "messenger",
            path: PathBuf::from("messenger"),
            external: false,
        }];

        let out = data.with_extension("vbk2");
        make_archive(None, &db, &data, &parts, &out, "s3cret", "5".into())
            .await
            .unwrap();
        // Still open and still writing: the archive is what was there at the snapshot.
        sqlx::query("INSERT INTO m (body) VALUES ('after')")
            .execute(&messenger)
            .await
            .unwrap();

        let staging = data.join(RESTORE_STAGING).join("s1");
        extract_archive(None, &out, "s3cret", &staging).unwrap();
        let manifest = read_manifest(&staging).await.unwrap();
        assert!(manifest.entries.contains(&"messenger/messenger.db".to_string()));
        assert!(!manifest.entries.iter().any(|e| e.ends_with("-wal") || e.ends_with("-shm")));
        assert!(!staging.join("messenger/messenger.db-wal").exists());
        assert_eq!(std::fs::read(staging.join("messenger/media.bin")).unwrap(), [1u8; 10]);
        let restored = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(staging.join("messenger/messenger.db"))
                    .read_only(true),
            )
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM m")
            .fetch_one(&restored)
            .await
            .unwrap();
        assert_eq!(n, 50);
        restored.close().await;
        messenger.close().await;
        db.close().await;
    }

    /// A backup of a data directory with notes, a password, a profile and a
    /// messenger identity — its key in the lock's box `messenger`, its
    /// database and media in `messenger/` — and the user's folder of notes,
    /// restored over the same directory after everything changed, gives back
    /// every file byte for byte and every row.
    #[tokio::test]
    async fn a_backup_of_notes_passwords_and_a_messenger_restores_byte_for_byte() {
        let (db, data) = crate::db::test_pool().await;
        let custom = data.with_extension("custom");
        put(&data.join("notes/documents/n1.md"), "# Deploy\nsteps");
        std::fs::create_dir_all(data.join("notes/attachments/n1")).unwrap();
        std::fs::write(data.join("notes/attachments/n1/a.png"), [0u8, 159, 146, 150]).unwrap();
        put(&data.join("profiles/p1/firefox-profile/prefs.js"), "user_pref(1);");
        put(&data.join("profiles/p1/firefox-profile/startupCache"), "CACHE");
        std::fs::create_dir_all(data.join("messenger/media")).unwrap();
        std::fs::write(data.join("messenger/messenger.db"), b"SQLite format 3\0rows").unwrap();
        std::fs::write(data.join("messenger/media/m1.bin"), [7u8; 300]).unwrap();
        put(&custom.join("c1.md"), "# Custom");
        let n1 = data.join("notes/documents/n1.md").to_string_lossy().into_owned();
        sqlx::query(
            "INSERT INTO notes (id, title, file_path, created_at, updated_at)
             VALUES ('n1', 'Deploy', ?, 't', 't')",
        )
        .bind(&n1)
        .execute(&db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO passwords (id, title, username, password_enc, vault_id, created_at, updated_at)
             VALUES ('pw1', 'Bank', 'me', 'v2:sealed', 'default', 't', 't')",
        )
        .execute(&db)
        .await
        .unwrap();
        let lock = veydan_lock::Lock::with_key_users(db.clone(), crate::modules::key_users());
        lock.open_default().await.unwrap();
        lock.secret_box("messenger")
            .put("identity", b"nsec1-the-key")
            .await
            .unwrap();

        let parts = space_parts(&custom);
        let names: Vec<&str> = parts.iter().map(|p| p.name).collect();
        assert_eq!(names, ["profiles", "notes", "notes_custom", "messenger"]);
        let mut before = tree(&data);
        before.retain(|name, _| !name.starts_with(DB_FILE));
        before.remove("profiles/p1/firefox-profile/startupCache");
        let custom_before = tree(&custom);

        let out = data.with_extension("vbk2");
        make_archive(None, &db, &data, &parts, &out, "s3cret", "5.0.0".into())
            .await
            .unwrap();
        db.close().await;

        // Everything changes after the backup.
        std::fs::remove_file(data.join("notes/documents/n1.md")).unwrap();
        put(&data.join("notes/documents/n2.md"), "# Later");
        std::fs::remove_dir_all(data.join("profiles")).unwrap();
        std::fs::write(data.join("messenger/messenger.db"), b"another identity").unwrap();
        std::fs::remove_file(data.join("messenger/media/m1.bin")).unwrap();
        put(&custom.join("c1.md"), "# Custom, edited");
        let later = crate::db::test_pool().await;
        later.0.close().await;
        std::fs::copy(later.1.join(DB_FILE), data.join(DB_FILE)).unwrap();

        // The restore, as `backup_restore` runs it.
        let staging = data.join(RESTORE_STAGING).join("s1");
        extract_archive(None, &out, "s3cret", &staging).unwrap();
        let manifest = read_manifest(&staging).await.unwrap();
        let place = placement(&data, &parts, &manifest.parts());
        assert_eq!(place.target_of(veydan_notes::BACKUP_CUSTOM_DIR), Some(custom.as_path()));
        remap_paths(&staging, &data, Some(&custom)).unwrap();
        let kept = swap_in(&data, &staging, &place).unwrap();
        std::fs::remove_dir_all(data.join(RESTORE_STAGING)).unwrap();

        let mut after = tree(&data);
        after.retain(|name, _| !name.starts_with(DB_FILE));
        assert_eq!(after, before);
        assert_eq!(tree(&custom), custom_before);
        let db = crate::db::open(&data.join(DB_FILE)).await.unwrap();
        let note: (String, String) =
            sqlx::query_as("SELECT title, file_path FROM notes WHERE id = 'n1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(note, ("Deploy".to_string(), n1));
        let password: (String, String, String) =
            sqlx::query_as("SELECT title, username, password_enc FROM passwords")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(
            password,
            ("Bank".into(), "me".into(), "v2:sealed".into())
        );
        let lock = veydan_lock::Lock::with_key_users(db.clone(), crate::modules::key_users());
        lock.open_default().await.unwrap();
        let key = lock.secret_box("messenger").get("identity").await.unwrap();
        assert_eq!(key.as_deref().map(Vec::as_slice), Some(&b"nsec1-the-key"[..]));
        db.close().await;
        for dir in [&data, &custom, &later.1].into_iter().chain(&kept) {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_file(&out);
    }

    /// An archive made before the parts were recorded holds the profiles,
    /// the notes and the user's folder of notes: those are restored, and
    /// the messenger's folder, which it does not hold, stays as it is.
    #[test]
    fn an_archive_without_parts_leaves_the_messenger_in_place() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"format_version":2,"app_version":"5.0.0","created_at":"t",
                "includes_camoufox":false,"notes_custom_dir":"/home/u/Notes","entries":[]}"#,
        )
        .unwrap();
        let data = Path::new("/data");
        let live = [
            BackupPath { name: "profiles", path: "profiles".into(), external: false },
            BackupPath { name: "notes", path: "notes".into(), external: false },
            BackupPath { name: "notes_custom", path: "/home/u/Notes".into(), external: true },
            BackupPath { name: "messenger", path: "messenger".into(), external: false },
        ];
        let place = placement(data, &live, &manifest.parts());
        assert_eq!(
            place,
            Placement {
                swapped: vec![
                    ("profiles".into(), "profiles".into()),
                    ("notes".into(), "notes".into()),
                ],
                copied: vec![("notes_custom".into(), "/home/u/Notes".into())],
            }
        );
        // On a machine that keeps its notes elsewhere, or not outside at all,
        // they land in the data directory.
        let place = placement(data, &live[..2], &manifest.parts());
        assert_eq!(place.target_of("notes_custom"), Some(Path::new("/data/notes_custom")));
    }

    /// A part names one entry of the data directory or an absolute path;
    /// what an archive names otherwise is not restored anywhere.
    #[test]
    fn a_part_outside_its_promise_is_neither_backed_up_nor_restored() {
        let data = tmp_root("promise");
        put(&data.join("notes/n.md"), "n");
        let odd = [
            BackupPath { name: "up", path: "../outside".into(), external: false },
            BackupPath { name: "deep", path: "notes/n.md".into(), external: false },
            BackupPath { name: "rel", path: "relative".into(), external: true },
            BackupPath { name: "app.db", path: "x".into(), external: false },
            BackupPath { name: "notes", path: "notes".into(), external: false },
            BackupPath { name: "notes", path: "notes".into(), external: false },
        ];
        let (files, dirs, recorded) = collect_parts(&data, &odd).unwrap();
        assert_eq!(files, [("notes/n.md".to_string(), data.join("notes/n.md"))]);
        assert_eq!(dirs, ["notes"]);
        assert_eq!(recorded.len(), 1);
        let foreign = [ManifestPart { name: "../../etc".into(), path: "/etc".into(), external: true }];
        assert_eq!(placement(&data, &odd, &foreign), Placement::default());
        for name in ["tor", "camoufox", "unpromised"] {
            let part = [ManifestPart { name: name.into(), path: "/x".into(), external: true }];
            assert_eq!(placement(&data, &odd, &part), Placement::default(), "{name}");
        }
        // The fallback never lands on a folder that holds something.
        put(&data.join("notes_custom/keep.md"), "k");
        let custom = [ManifestPart {
            name: veydan_notes::BACKUP_CUSTOM_DIR.into(),
            path: "/elsewhere".into(),
            external: true,
        }];
        assert_eq!(placement(&data, &odd, &custom), Placement::default());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// What a test sees the running app do.
    #[derive(Default)]
    struct Seen(std::sync::Mutex<Vec<&'static str>>);

    impl Seen {
        fn push(&self, what: &'static str) {
            self.0.lock().unwrap().push(what);
        }
    }

    impl Running for std::sync::Arc<Seen> {
        async fn shutdown(&self) {
            self.push("shutdown");
        }
        async fn close_data(&self) {
            self.push("close_data");
        }
    }

    /// The restore stops every module as the exit does (`Shell::shutdown`:
    /// `exiting` first, so a module that is off stops what it left running)
    /// and closes the data file before anything is swapped (spec 12; 19,
    /// № 59).
    #[tokio::test]
    async fn a_restore_stops_the_modules_before_it_swaps() {
        let seen = std::sync::Arc::new(Seen::default());
        let in_swap = seen.clone();
        release_and_swap(&seen, move || {
            in_swap.push("swap");
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(*seen.0.lock().unwrap(), ["shutdown", "close_data", "swap"]);
    }

    fn test_manifest(files: &[(String, PathBuf)]) -> Manifest {
        Manifest {
            format_version: FORMAT_VERSION,
            app_version: "5.0.0".into(),
            created_at: "t".into(),
            includes_camoufox: false,
            notes_custom_dir: None,
            parts: None,
            entries: files.iter().map(|(n, _)| n.clone()).collect(),
        }
    }

    fn inside(name: &'static str) -> BackupPath {
        BackupPath { name, path: name.into(), external: false }
    }

    /// Old archives of the folder of the backups, oldest first.
    const OLD: [&str; 3] = [
        "veydan-backup-20200101-030000.vbk2",
        "veydan-backup-20200102-030000.vbk2",
        "veydan-backup-20200103-030000.vbk2",
    ];

    /// `path` loses every permission; false when that does not keep this
    /// process out of it (root), and the test has nothing to check.
    #[cfg(unix)]
    fn lock_out(path: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = if path.is_dir() {
            std::fs::read_dir(path).is_ok()
        } else {
            File::open(path).is_ok()
        };
        if readable {
            eprintln!("skipped: {} stays readable to this user", path.display());
        }
        !readable
    }

    #[cfg(unix)]
    fn let_in(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A folder of a part that cannot be read fails the backup: no archive
    /// that lacks it passes for a whole copy, and nothing is left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_part_that_cannot_be_read_fails_the_backup() {
        let (db, data) = crate::db::test_pool().await;
        put(&data.join("notes/documents/n1.md"), "n1");
        put(&data.join("notes/locked/n2.md"), "n2");
        let locked = data.join("notes/locked");
        if lock_out(&locked) {
            let out = data.with_extension("vbk2");
            let err = make_archive(None, &db, &data, &[inside("notes")], &out, "s3cret", "5".into())
                .await
                .unwrap_err();
            assert!(err.contains("Cannot read"), "{err}");
            assert!(!out.exists() && !part_path_of(&out).exists());
            assert_eq!(names_in(&data.join(BACKUP_STAGING)), Vec::<String>::new());
        }
        let_in(&locked);
        db.close().await;
        let _ = std::fs::remove_dir_all(&data);
    }

    /// A backup that fails while it writes leaves no `.part` and rotates
    /// nothing out; the next whole one takes its name and rotates.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_backup_rotates_nothing_out() {
        let (db, data) = crate::db::test_pool().await;
        let dir = data.with_extension("backups");
        for name in OLD {
            put(&dir.join(name), "old");
        }
        put(&data.join("notes/documents/n1.md"), "n1");
        let unreadable = data.join("notes/documents/n2.md");
        put(&unreadable, "n2");
        let parts = [inside("notes")];
        if lock_out(&unreadable) {
            let err = back_up_into(None, &db, &data, &parts, &dir, "s3cret", "5".into(), 2)
                .await
                .unwrap_err();
            assert!(err.contains("Cannot read"), "{err}");
            assert_eq!(names_in(&dir), OLD);
        }
        let_in(&unreadable);
        let out = back_up_into(None, &db, &data, &parts, &dir, "s3cret", "5".into(), 2)
            .await
            .unwrap();
        let name = out.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(names_in(&dir), [OLD[2].to_string(), name]);
        db.close().await;
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A whole backup is found under its own name only, with no `.part`
    /// beside it, and holds the folder of a part with nothing in it.
    #[tokio::test]
    async fn a_whole_backup_takes_its_name_and_holds_an_empty_part() {
        let (db, data) = crate::db::test_pool().await;
        let dir = data.with_extension("backups");
        for name in OLD {
            put(&dir.join(name), "old");
        }
        put(&data.join("notes/documents/n1.md"), "n1");
        std::fs::create_dir_all(data.join("messenger")).unwrap();
        let parts = [inside("notes"), inside("messenger"), inside("profiles")];
        let out = back_up_into(None, &db, &data, &parts, &dir, "s3cret", "5".into(), 2)
            .await
            .unwrap();
        let name = out.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(names_in(&dir), [OLD[2].to_string(), name]);

        let staging = data.join(RESTORE_STAGING).join("s1");
        extract_archive(None, &out, "s3cret", &staging).unwrap();
        assert_eq!(text(&staging.join("notes/documents/n1.md")), "n1");
        assert!(staging.join("messenger").is_dir());
        // A part the data directory did not have is not made up.
        assert!(!staging.join("profiles").exists());
        db.close().await;
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The data keeps the date of its newest archive, and an archive older
    /// than that is told apart, so a restore of it asks first (A5-16).
    #[tokio::test]
    async fn an_archive_older_than_the_newest_one_of_the_data_is_told_apart() {
        let (db, data) = crate::db::test_pool().await;
        let dir = data.with_extension("backups");
        std::fs::create_dir_all(&dir).unwrap();
        let out = back_up_into(None, &db, &data, &[], &dir, "s3cret", "5".into(), 0)
            .await
            .unwrap();
        let staging = data.join(RESTORE_STAGING).join("s1");
        extract_archive(None, &out, "s3cret", &staging).unwrap();
        let manifest = read_manifest(&staging).await.unwrap();
        let newest = settings::get(&db, LAST_CREATED_KEY).await;
        assert_eq!(newest.as_deref(), Some(manifest.created_at.as_str()));
        // The newest archive itself restores without a question.
        assert!(!older_than_the_data(&manifest.created_at, newest.as_deref()));

        let week_ago = "2026-10-07T10:00:00+00:00";
        let now = "2026-10-14T10:00:00+03:00";
        assert!(older_than_the_data(week_ago, Some(now)));
        assert!(!older_than_the_data(now, Some(week_ago)));
        // No archive made of this data yet, or a date that does not read: no question.
        assert!(!older_than_the_data(week_ago, None));
        assert!(!older_than_the_data("t", Some(now)));
        db.close().await;
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file gone between the list and the write is left out, and the
    /// manifest a restore reads does not promise it.
    #[test]
    fn a_file_gone_since_the_list_is_not_in_the_manifest() {
        let root = tmp_root("gone");
        put(&root.join("notes/kept.md"), "kept");
        let db = root.join("db.sqlite");
        std::fs::write(&db, b"DB").unwrap();
        let files = vec![
            ("notes/kept.md".to_string(), root.join("notes/kept.md")),
            ("notes/gone.md".to_string(), root.join("notes/gone.md")),
        ];
        let out = root.join("backup.vbk2");
        let dirs = ["notes".to_string()];
        write_archive(None, &out, "s3cret", &test_manifest(&files), &db, &dirs, &files).unwrap();

        let dest = root.join("restored");
        extract_archive(None, &out, "s3cret", &dest).unwrap();
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(dest.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.entries, ["notes/kept.md"]);
        assert_eq!(text(&dest.join("notes/kept.md")), "kept");
        assert!(!dest.join("notes/gone.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An archive that records the messenger as a part but holds no folder
    /// of it leaves the live folder as it is.
    #[test]
    fn a_part_the_archive_does_not_hold_stays_in_place() {
        let data = tmp_root("unheld");
        put(&data.join(DB_FILE), "live db");
        put(&data.join("notes/documents/n1.md"), "live");
        put(&data.join("messenger/messenger.db"), "live messenger");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("notes/documents/n2.md"), "restored");
        let place = Placement {
            swapped: vec![
                ("notes".into(), PathBuf::from("notes")),
                ("messenger".into(), PathBuf::from("messenger")),
            ],
            copied: vec![],
        };

        swap_in(&data, &staging, &place).unwrap();

        assert_eq!(text(&data.join(DB_FILE)), "restored db");
        assert_eq!(text(&data.join("notes/documents/n2.md")), "restored");
        assert!(!data.join("notes/documents/n1.md").exists());
        assert_eq!(text(&data.join("messenger/messenger.db")), "live messenger");
        assert_eq!(
            names_in(&data),
            ["app.db", "messenger", "notes", RESTORE_STAGING]
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    /// The start removes the snapshot of the data file a backup cut short
    /// left, and the archives it left under their `.part` name.
    #[test]
    fn the_start_sweeps_what_a_backup_cut_short_left() {
        let data = tmp_root("sweep");
        put(&data.join(DB_FILE), "live db");
        put(&data.join(BACKUP_STAGING).join("u1").join(DB_FILE), "snapshot");
        finish_interrupted_restore(&data);
        assert_eq!(names_in(&data), ["app.db"]);

        let dir = tmp_root("sweep_dir");
        for name in [
            "veydan-backup-20261001-030000.vbk2",
            "veydan-backup-20261002-030000.vbk2.part",
            "veydan-backup-20260101-030000.vbk.part",
            "notes.part",
        ] {
            put(&dir.join(name), "x");
        }
        sweep_parts(&dir);
        assert_eq!(
            names_in(&dir),
            [
                "notes.part",
                "veydan-backup-20260101-030000.vbk.part",
                "veydan-backup-20261001-030000.vbk2",
            ]
        );
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    thread_local! {
        /// The part whose folder outside `swap_in_at` fails to place on this
        /// thread, once the live folder is aside.
        pub(super) static FAIL_TO_PLACE: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    /// The stamp of the swaps the tests cut short.
    const TS: &str = "20261014-120000";
    /// The date in the name of the previous folder of a swap at `TS`.
    const KEPT_DATE: &str = "2026-10-14-120000";

    /// The name of the previous folder of `Notes` after a swap at `TS`.
    fn previous_name() -> String {
        format!("Notes.before-restore-{KEPT_DATE}")
    }

    /// Space's placement with the user's folder of notes at `custom`.
    fn custom_place(custom: &Path) -> Placement {
        Placement {
            copied: vec![(veydan_notes::BACKUP_CUSTOM_DIR.into(), custom.to_path_buf())],
            ..space_place()
        }
    }

    fn files(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, Vec<u8>> {
        pairs
            .iter()
            .map(|(name, text)| (name.to_string(), text.as_bytes().to_vec()))
            .collect()
    }

    /// The marker of a swap at `TS` of the user's folder of notes at
    /// `custom` that stood there before, which places `added` where no
    /// live entry stood.
    fn marker_of(custom: &Path, added: &[&str]) -> SwapMarker {
        SwapMarker {
            ts: TS.into(),
            added: added.iter().map(PathBuf::from).collect(),
            external: vec![ExternalSwap {
                name: veydan_notes::BACKUP_CUSTOM_DIR.into(),
                dst: custom.to_path_buf(),
                new: beside(custom, NEW_INFIX, TS).unwrap(),
                old: beside(custom, OLD_INFIX, TS).unwrap(),
                kept: beside(custom, KEPT_INFIX, KEPT_DATE).unwrap(),
                had_live: true,
            }],
        }
    }

    fn result_of(data: &Path) -> String {
        read_outcome(data).map(|o| o.result).unwrap_or_default()
    }

    /// The user's folder of notes is replaced whole by the archive's; the
    /// previous one stays beside it, with the file made after the backup,
    /// and nothing else of the swap stays beside it or in the data directory.
    #[test]
    fn a_swap_replaces_the_users_folder_of_notes_and_keeps_the_previous_one() {
        let data = tmp_root("ext");
        let outside = tmp_root("ext_out");
        let custom = outside.join("Notes");
        put(&data.join(DB_FILE), "live db");
        put(&custom.join("c1.md"), "live");
        put(&custom.join("later.md"), "made after the backup");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("notes_custom/c1.md"), "restored");

        let kept = swap_in_at(&data, &staging, &custom_place(&custom), TS).unwrap();

        let previous = outside.join(format!("Notes.before-restore-{KEPT_DATE}"));
        assert_eq!(kept, std::slice::from_ref(&previous));
        assert_eq!(text(&data.join(DB_FILE)), "restored db");
        assert_eq!(tree(&custom), files(&[("c1.md", "restored")]));
        assert_eq!(
            tree(&previous),
            files(&[("c1.md", "live"), ("later.md", "made after the backup")])
        );
        assert_eq!(names_in(&outside), ["Notes".to_string(), previous_name()]);
        // A second previous folder of the same minute takes the next free name.
        assert_eq!(
            kept_beside(&custom, TS),
            Some(outside.join(format!("Notes.before-restore-{KEPT_DATE}-2")))
        );
        assert_eq!(names_in(&data), ["app.db", RESTORE_STAGING]);
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// The folder of notes cannot be replaced where it stands (its parent
    /// takes no new entry): the restore fails before anything moves, and
    /// the live data and the folder are as they were.
    #[cfg(unix)]
    #[test]
    fn a_folder_of_notes_that_cannot_be_replaced_fails_the_restore() {
        use std::os::unix::fs::PermissionsExt;
        let data = tmp_root("ro");
        let outside = tmp_root("ro_out");
        let custom = outside.join("Notes");
        put(&data.join(DB_FILE), "live db");
        put(&data.join("app.db-wal"), "live wal");
        put(&data.join("notes/documents/n1.md"), "live");
        put(&custom.join("c1.md"), "live");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("notes/documents/n2.md"), "restored");
        put(&staging.join("notes_custom/c1.md"), "restored");
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o555)).unwrap();
        let writable = File::create(outside.join("probe")).is_ok();

        let result = (!writable).then(|| swap_in(&data, &staging, &custom_place(&custom)));
        let_in(&outside);
        match result {
            None => eprintln!("skipped: {} stays writable to this user", outside.display()),
            Some(result) => {
                let err = result.unwrap_err();
                assert!(err.contains("notes_custom"), "{err}");
                assert_eq!(text(&data.join(DB_FILE)), "live db");
                assert_eq!(text(&data.join("app.db-wal")), "live wal");
                assert_eq!(text(&data.join("notes/documents/n1.md")), "live");
                assert_eq!(tree(&custom), files(&[("c1.md", "live")]));
                assert_eq!(names_in(&outside), ["Notes"]);
                assert_eq!(
                    names_in(&data),
                    ["app.db", "app.db-wal", "notes", RESTORE_STAGING]
                );
            }
        }
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// The folder of notes was moved aside and its copy failed to take its
    /// place: every live entry comes back, and the part the archive placed
    /// where none stood (the messenger) goes again.
    #[test]
    fn a_swap_that_fails_after_the_folder_was_moved_aside_puts_everything_back() {
        let data = tmp_root("fail");
        let outside = tmp_root("fail_out");
        let custom = outside.join("Notes");
        put(&data.join(DB_FILE), "live db");
        put(&data.join("app.db-wal"), "live wal");
        put(&data.join("notes/documents/n1.md"), "live");
        put(&custom.join("c1.md"), "live");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("notes/documents/n2.md"), "restored");
        put(
            &staging.join("messenger/messenger.db"),
            "restored messenger",
        );
        put(&staging.join("notes_custom/c1.md"), "restored");
        let place = Placement {
            swapped: vec![
                ("notes".into(), PathBuf::from("notes")),
                ("messenger".into(), PathBuf::from("messenger")),
            ],
            copied: vec![(veydan_notes::BACKUP_CUSTOM_DIR.into(), custom.clone())],
        };

        FAIL_TO_PLACE.with(|f| *f.borrow_mut() = Some(veydan_notes::BACKUP_CUSTOM_DIR.into()));
        let err = swap_in_at(&data, &staging, &place, TS).unwrap_err();
        FAIL_TO_PLACE.with(|f| *f.borrow_mut() = None);

        assert!(err.contains("notes_custom"), "{err}");
        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(text(&data.join("app.db-wal")), "live wal");
        assert_eq!(
            tree(&data.join("notes")),
            files(&[("documents/n1.md", "live")])
        );
        assert!(!data.join("messenger").exists());
        assert_eq!(tree(&custom), files(&[("c1.md", "live")]));
        assert_eq!(names_in(&outside), ["Notes"]);
        assert_eq!(
            names_in(&data),
            ["app.db", "app.db-wal", "notes", RESTORE_STAGING]
        );
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// A folder outside that holds the data directory, or one beside which
    /// a name of the swap is taken, is not swapped: refused before anything
    /// moves.
    #[test]
    fn a_folder_the_swap_cannot_own_is_refused() {
        let root = tmp_root("own");
        let data = root.join("data");
        put(&data.join(DB_FILE), "live db");
        let staging = data.join(RESTORE_STAGING).join("s1");
        put(&staging.join(DB_FILE), "restored db");
        put(&staging.join("notes_custom/c1.md"), "restored");

        swap_in_at(&data, &staging, &custom_place(&root), TS).unwrap_err();
        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(names_in(&data), ["app.db", RESTORE_STAGING]);
        assert_eq!(names_in(&root), ["data"]);

        let custom = root.join("Notes");
        put(&custom.join("c1.md"), "live");
        let foreign = beside(&custom, OLD_INFIX, TS).unwrap();
        put(&foreign.join("x.md"), "not the swap's");
        swap_in_at(&data, &staging, &custom_place(&custom), TS).unwrap_err();
        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(tree(&custom), files(&[("c1.md", "live")]));
        assert_eq!(tree(&foreign), files(&[("x.md", "not the swap's")]));
        assert_eq!(names_in(&data), ["app.db", RESTORE_STAGING]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The process died before the folder of notes was swapped: while its
    /// copy was made, and after the live entries were moved and the
    /// messenger placed. The next start puts every live entry back.
    #[test]
    fn a_restore_cut_short_before_the_folder_of_notes_was_swapped_is_undone() {
        // The copy was under way: nothing in the data directory moved yet.
        let data = tmp_root("cut_copy");
        let outside = tmp_root("cut_copy_out");
        let custom = outside.join("Notes");
        let marker = marker_of(&custom, &[]);
        put(&data.join(DB_FILE), "live db");
        put(&custom.join("c1.md"), "live");
        put(&marker.external[0].new.join("c1.md"), "restored, half");
        write_json_atomic(&data.join(SWAP_MARKER), &marker).unwrap();

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(tree(&custom), files(&[("c1.md", "live")]));
        assert_eq!(names_in(&outside), ["Notes"]);
        assert_eq!(names_in(&data), ["app.db", RESTORE_OUTCOME]);
        assert_eq!(result_of(&data), "undone");
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);

        // The live entries were aside and the archive's in place.
        let data = tmp_root("cut_moved");
        let outside = tmp_root("cut_moved_out");
        let custom = outside.join("Notes");
        let marker = marker_of(&custom, &["messenger"]);
        let old = data.join(format!("{SAVED_PREFIX}{TS}"));
        put(&old.join(DB_FILE), "live db");
        put(&data.join(DB_FILE), "restored db");
        put(&data.join("messenger/messenger.db"), "restored messenger");
        put(&custom.join("c1.md"), "live");
        put(&marker.external[0].new.join("c1.md"), "restored");
        write_json_atomic(&data.join(SWAP_MARKER), &marker).unwrap();

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(tree(&custom), files(&[("c1.md", "live")]));
        assert_eq!(names_in(&outside), ["Notes"]);
        assert_eq!(names_in(&data), ["app.db", RESTORE_OUTCOME]);
        assert_eq!(result_of(&data), "undone");
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// The process died after the folder of notes was swapped and before
    /// the swap was marked complete: the data file goes back to the live
    /// one, and so does the folder.
    #[test]
    fn a_restore_cut_short_after_the_folder_of_notes_was_swapped_is_undone() {
        let data = tmp_root("cut_swapped");
        let outside = tmp_root("cut_swapped_out");
        let custom = outside.join("Notes");
        let marker = marker_of(&custom, &[]);
        let old = data.join(format!("{SAVED_PREFIX}{TS}"));
        put(&old.join(DB_FILE), "live db");
        put(&data.join(DB_FILE), "restored db");
        put(&custom.join("c1.md"), "restored");
        put(&marker.external[0].old.join("c1.md"), "live");
        put(&marker.external[0].old.join("c2.md"), "live too");
        write_json_atomic(&data.join(SWAP_MARKER), &marker).unwrap();

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "live db");
        assert_eq!(
            tree(&custom),
            files(&[("c1.md", "live"), ("c2.md", "live too")])
        );
        assert_eq!(names_in(&outside), ["Notes"]);
        assert_eq!(names_in(&data), ["app.db", RESTORE_OUTCOME]);
        assert_eq!(result_of(&data), "undone");
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// The process died after the swap was marked complete, while removing
    /// what it had moved aside: the next start finishes it, and the
    /// restored folder of notes stays.
    #[test]
    fn a_restore_cut_short_after_it_was_complete_is_finished() {
        let data = tmp_root("cut_done");
        let outside = tmp_root("cut_done_out");
        let custom = outside.join("Notes");
        let marker = marker_of(&custom, &[]);
        let done = data.join(format!("{DONE_PREFIX}{TS}"));
        put(&done.join(DB_FILE), "previous db");
        put(&data.join(DB_FILE), "restored db");
        put(&custom.join("c1.md"), "restored");
        put(&marker.external[0].old.join("c1.md"), "previous");
        write_json_atomic(&data.join(SWAP_MARKER), &marker).unwrap();

        finish_interrupted_restore(&data);

        assert_eq!(text(&data.join(DB_FILE)), "restored db");
        assert_eq!(tree(&custom), files(&[("c1.md", "restored")]));
        let previous = &marker.external[0].kept;
        assert_eq!(tree(previous), files(&[("c1.md", "previous")]));
        assert_eq!(names_in(&outside), ["Notes".to_string(), previous_name()]);
        assert_eq!(names_in(&data), ["app.db", RESTORE_OUTCOME]);
        assert_eq!(result_of(&data), "finished");
        let kept = read_outcome(&data).unwrap().kept;
        assert_eq!(kept, [previous.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);

        // The previous folder had its name already: only the rest is finished.
        let data = tmp_root("cut_kept");
        let outside = tmp_root("cut_kept_out");
        let custom = outside.join("Notes");
        let marker = marker_of(&custom, &[]);
        let done = data.join(format!("{DONE_PREFIX}{TS}"));
        put(&done.join(DB_FILE), "previous db");
        put(&data.join(DB_FILE), "restored db");
        put(&custom.join("c1.md"), "restored");
        put(&marker.external[0].kept.join("c1.md"), "previous");
        write_json_atomic(&data.join(SWAP_MARKER), &marker).unwrap();

        finish_interrupted_restore(&data);

        assert_eq!(tree(&custom), files(&[("c1.md", "restored")]));
        let previous = &marker.external[0].kept;
        assert_eq!(tree(previous), files(&[("c1.md", "previous")]));
        assert_eq!(names_in(&data), ["app.db", RESTORE_OUTCOME]);
        assert_eq!(result_of(&data), "finished");
        let kept = read_outcome(&data).unwrap().kept;
        assert_eq!(kept, [previous.to_string_lossy().into_owned()]);
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&outside);
    }
}

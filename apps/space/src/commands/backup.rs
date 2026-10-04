// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Full-application backup & restore: the driver of Space's `backup` service
//! (docs/platform-spec.md 5.9).
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
//! A restore stops every module (`Shell::stop_modules`, as on exit), closes
//! the data file, swaps the data file and the parts the archive holds under
//! them and restarts the app.
//!
//! Scheduling: a background task ticks every 60s, reads the schedule from
//! `app_settings`, and runs a backup when one is due (catching up missed runs
//! after the app was closed). Old backups are rotated to the newest N. The
//! list and the rotation see only archives of this format: the extension
//! carries the format version, so archives of another one are left alone.

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

/// Journals of the live data file; they move with it.
const SWAPPED_SIDECARS: [&str; 2] = ["-wal", "-shm"];
/// Folder of the unpacked archive inside the data directory.
const RESTORE_STAGING: &str = "restore_tmp";
/// Folder holding the live entries while a restore swaps them: it exists
/// from the first move until the swap is complete.
const SAVED_PREFIX: &str = "restore_backup_";
/// The same folder once the swap is complete and it is only waiting to be removed.
const DONE_PREFIX: &str = "restore_done_";

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
    }
}

// ── Commands: config ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn backup_get_config(core: tauri::State<'_, Core>) -> CmdResult<BackupConfig> {
    Ok(load_config(&core.db).await)
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
    let out_name = format!(
        "{FILE_PREFIX}{}.{FILE_EXT}",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let out_path = dir.join(&out_name);
    make_archive(
        Some(app),
        &core.db,
        &core.app_data_dir,
        &parts,
        &out_path,
        &password,
        app.package_info().version.to_string(),
    )
    .await?;

    // Rotation + bookkeeping.
    if cfg.keep > 0 {
        rotate(&dir, cfg.keep as usize);
    }
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

/// Write the archive at `out_path`: a snapshot of the data file of
/// `data_dir` and the files of `parts`, with the manifest naming them. `app`
/// is optional so the pipeline can be exercised in unit tests.
async fn make_archive(
    app: Option<&AppHandle>,
    db: &Pool<Sqlite>,
    data_dir: &Path,
    parts: &[BackupPath],
    out_path: &Path,
    password: &str,
    app_version: String,
) -> Result<(), String> {
    if let Some(app) = app {
        emit_progress(app, "snapshot", 2);
    }

    // 1) Consistent DB snapshot via VACUUM INTO into a staging dir.
    let staging = data_dir
        .join("backup_tmp")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let db_snapshot = staging.join(DB_FILE);
    if let Err(e) = snapshot_db(db, &db_snapshot).await {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("DB snapshot failed: {e}"));
    }

    // 2) The files of the parts (archive_name, fs_path).
    let (files, recorded) = collect_parts(data_dir, parts);

    // 3) Manifest.
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        app_version,
        created_at: Utc::now().to_rfc3339(),
        includes_camoufox: false,
        notes_custom_dir: None,
        parts: Some(recorded),
        entries: files.iter().map(|(n, _)| n.clone()).collect(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;

    // 4) Stream the archive (blocking CPU/IO work off the async runtime).
    let app2 = app.cloned();
    let db_snapshot2 = db_snapshot.clone();
    let out_path2 = out_path.to_path_buf();
    let password = password.to_owned();
    let write_res = tauri::async_runtime::spawn_blocking(move || {
        write_archive(
            app2.as_ref(),
            &out_path2,
            &password,
            &manifest_json,
            &db_snapshot2,
            &files,
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    // Clean staging regardless of outcome.
    let _ = std::fs::remove_dir_all(&staging);
    write_res.inspect_err(|_| {
        let _ = std::fs::remove_file(out_path);
    })
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

/// The files of `parts`, each under the name of its part, and the parts as
/// the manifest records them. A part that does not keep the promise of
/// `BackupPath` (one name; one entry of the data directory or an absolute
/// path) is left out and told, and so is a second part of one name.
fn collect_parts(
    data_dir: &Path,
    parts: &[BackupPath],
) -> (Vec<(String, PathBuf)>, Vec<ManifestPart>) {
    let mut files = Vec::new();
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
            collect_files(&source, part.name, &mut files);
        }
        recorded.push(ManifestPart {
            name: part.name.to_string(),
            path: part.path.to_string_lossy().into_owned(),
            external: part.external,
        });
    }
    (files, recorded)
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
        && name != "backup_tmp"
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

/// Recursively gather every file under `base_fs`, mapping it to an archive path
/// rooted at `base_arch`, skipping runtime-cache blacklist names.
fn collect_files(base_fs: &Path, base_arch: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(base_fs) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_blacklisted(&name) {
            continue;
        }
        let path = entry.path();
        let arch = format!("{base_arch}/{name}");
        if path.is_dir() {
            collect_files(&path, &arch, out);
        } else if path.is_file() {
            out.push((arch, path));
        }
    }
}

/// Write manifest + db snapshot + files as tar→zstd→age into `out_path`.
/// `app` is optional so the pipeline can be exercised in unit tests.
fn write_archive(
    app: Option<&AppHandle>,
    out_path: &Path,
    password: &str,
    manifest_json: &[u8],
    db_snapshot: &Path,
    files: &[(String, PathBuf)],
) -> Result<(), String> {
    let file = File::create(out_path).map_err(|e| e.to_string())?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::from(password.to_owned()));
    let age_writer = encryptor
        .wrap_output(file)
        .map_err(|e| format!("Encryption init failed: {e}"))?;
    let zstd_writer =
        zstd::stream::write::Encoder::new(age_writer, 3).map_err(|e| e.to_string())?;
    let mut tar = tar::Builder::new(zstd_writer);

    append_bytes(&mut tar, "manifest.json", manifest_json).map_err(|e| e.to_string())?;

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
    for (arch, path) in files {
        // Files can vanish mid-backup (temp caches); skip rather than abort.
        if let Ok(mut f) = File::open(path) {
            tar.append_file(arch, &mut f).map_err(|e| e.to_string())?;
        }
        done += 1;
        if let Some(app) = app {
            if done.is_multiple_of(25) || done == total {
                emit_progress(app, "archiving", pct(done, total));
            }
        }
    }

    // Finish each layer in order: tar → zstd → age.
    let zstd_writer = tar.into_inner().map_err(|e| e.to_string())?;
    let age_writer = zstd_writer.finish().map_err(|e| e.to_string())?;
    age_writer
        .finish()
        .map_err(|e| format!("Encryption finalize failed: {e}"))?;
    Ok(())
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

#[derive(Debug, Serialize, Deserialize)]
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
        if !is_part_name(&part.name) {
            continue;
        }
        let mine = live
            .iter()
            .find(|l| l.name == part.name && l.external == part.external);
        if part.external {
            let target = match mine {
                Some(l) if l.path == Path::new(&part.path) => l.path.clone(),
                _ => data_dir.join(&part.name),
            };
            place.copied.push((part.name.clone(), target));
        } else if let Some(l) = mine.filter(|l| is_part_name(&l.path.to_string_lossy())) {
            place.swapped.push((part.name.clone(), l.path.clone()));
        }
    }
    place
}

/// Restore from a `.vbk2` file, then relaunch the app so the fresh data is
/// picked up cleanly. Never returns on success (process restarts).
#[tauri::command]
pub async fn backup_restore(
    path: String,
    password: Option<String>,
    app: AppHandle,
) -> CmdResult<()> {
    let core = app.state::<Core>();
    let shell = app.state::<Shell>();
    let data_dir = core.app_data_dir.clone();

    // Empty input means "use the configured backup password".
    let password = match password.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => load_config(&core.db)
            .await
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
        extract_archive(Some(&app2), Path::new(&path), &password, &staging2)
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
    if let Err(e) = swap_res {
        eprintln!("backup restore: swap failed, keeping current data: {e}");
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
    /// The app is on its way out: `stop` stops all a module left running.
    fn begin_exit(&self);
    /// `Module::stop` of every module, on or off, in the order of the product.
    async fn stop_modules(&self);
    /// No connection to the data file is left open.
    async fn close_data(&self);
}

/// The running app: its shell and its core.
struct TheApp<'a> {
    shell: &'a Shell,
    core: &'a Core,
}

impl Running for TheApp<'_> {
    fn begin_exit(&self) {
        self.shell.begin_exit();
    }

    async fn stop_modules(&self) {
        self.shell.stop_modules().await;
    }

    async fn close_data(&self) {
        self.core.db.close().await;
    }
}

/// Stop the modules as on exit (docs/platform-spec.md 12; 19, № 59): the
/// browsers they started, the watcher of the notes, the runtime of the
/// messenger with its database; close the data file; then swap the data
/// under them.
async fn release_and_swap<F>(running: &impl Running, swap: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    running.begin_exit();
    running.stop_modules().await;
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

    let profiles_dir = data_dir.join("profiles");
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
        let path = profiles_dir.join(&id).to_string_lossy().to_string();
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
        "DELETE FROM app_settings WHERE key IN ('sync_device_id', 'sync_own_seq', 'sync_own_head')",
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
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
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

/// Undo a partial swap: everything saved in `old` goes back to its live path
/// (dropping whatever staged entry may have been placed there meanwhile).
/// `old` itself goes only once nothing is left in it.
fn roll_back(data_dir: &Path, old: &Path) {
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

/// Replace `app.db` and the entries of the parts in the live data dir with
/// the staged versions, keeping a temporary backup until the swap succeeds;
/// then copy the parts outside the data dir into their folders.
fn swap_in(data_dir: &Path, staging: &Path, place: &Placement) -> Result<(), String> {
    let ts = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let old = data_dir.join(format!("{SAVED_PREFIX}{ts}"));
    std::fs::create_dir_all(&old).map_err(|e| e.to_string())?;

    // Move current → old. Also relocate the live DB's WAL/SHM sidecars: leaving
    // a stale `-wal` next to the freshly restored `app.db` would let SQLite
    // replay mismatched frames and corrupt it.
    for sidecar in swapped_sidecars() {
        let live = data_dir.join(&sidecar);
        if live.exists() {
            let _ = std::fs::rename(&live, old.join(&sidecar));
        }
    }
    let rollback = |err: String| {
        roll_back(data_dir, &old);
        err
    };
    // The data file, then each part: (folder of the archive, live entry).
    let swapped: Vec<(&str, &Path)> = std::iter::once((DB_FILE, Path::new(DB_FILE)))
        .chain(
            place
                .swapped
                .iter()
                .map(|(name, path)| (name.as_str(), path.as_path())),
        )
        .collect();
    for (_, entry) in &swapped {
        let live = data_dir.join(entry);
        if live.exists() {
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
    // The swap is complete. One rename says so to the next start, which
    // otherwise takes the folder for a swap cut short and undoes it.
    let done = data_dir.join(format!("{DONE_PREFIX}{ts}"));
    std::fs::rename(&old, &done).map_err(|e| rollback(format!("Cannot finish the swap: {e}")))?;

    // The parts outside the data dir go to the folders the placement chose.
    for (name, dst) in &place.copied {
        let src = staging.join(name);
        if src.exists() && std::fs::create_dir_all(dst).is_ok() {
            let _ = copy_dir(&src, dst);
        }
    }

    // Success — drop the safety copy.
    let _ = std::fs::remove_dir_all(&done);
    Ok(())
}

/// Clean up after a restore the process did not live to finish. Call before
/// the data file is opened.
///
/// A swap cut short left the live entries in its `restore_backup_*` folder;
/// they go back, as after a swap that failed. Without this the start would
/// find no data file and create an empty one over a user's data that still
/// lies in that folder.
pub fn finish_interrupted_restore(data_dir: &Path) {
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
        roll_back(data_dir, old);
    }
    // No restore runs at start: whatever is unpacked there is a leftover.
    let _ = std::fs::remove_dir_all(data_dir.join(RESTORE_STAGING));
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
        collect_files(&src.join("profiles"), "profiles", &mut files);
        collect_files(&src.join("notes"), "notes", &mut files);

        // Blacklisted cache must be excluded.
        assert!(files.iter().all(|(n, _)| !n.contains("startupCache")));

        let out = root.join("backup.vbk");
        let manifest = br#"{"format_version":1}"#;
        write_archive(None, &out, "s3cret", manifest, &db, &files).expect("write");
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
            "INSERT INTO app_settings (key, value) VALUES ('sync_device_id', 'dev-a'), ('ui_locale', 'ru')",
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
        write_archive(None, &out, "right", b"{}", &db, &[]).unwrap();

        let dest = root.join("out");
        let err = extract_archive(None, &out, "wrong", &dest);
        assert!(err.is_err(), "wrong password must fail");
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
        swap_in(&data, &staging, &place).unwrap();
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
        for dir in [&data, &custom, &later.1] {
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
        let (files, recorded) = collect_parts(&data, &odd);
        assert_eq!(files, [("notes/n.md".to_string(), data.join("notes/n.md"))]);
        assert_eq!(recorded.len(), 1);
        let foreign = [ManifestPart { name: "../../etc".into(), path: "/etc".into(), external: true }];
        assert_eq!(placement(&data, &odd, &foreign), Placement::default());
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
        fn begin_exit(&self) {
            self.push("begin_exit");
        }
        async fn stop_modules(&self) {
            self.push("stop_modules");
        }
        async fn close_data(&self) {
            self.push("close_data");
        }
    }

    /// The restore stops every module as the exit does — `exiting` first, so
    /// a module that is off stops what it left running — and closes the
    /// data file before anything is swapped (spec 12; 19, № 59).
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
        assert_eq!(
            *seen.0.lock().unwrap(),
            ["begin_exit", "stop_modules", "close_data", "swap"]
        );
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Transfer engine for the file browser: local↔remote, chunked, cancellable,
//! and resumable.
//!
//! Every file is written to a `<name>.veydanpart` temp next to its final
//! destination and renamed on completion. If a transfer is cancelled or the
//! connection drops, the part file stays; the next transfer of the same file
//! seeks past the already-written bytes and continues (resume). A part longer
//! than the source (source changed) restarts from zero.

use crate::commands::sftp::{get_or_connect, is_safe_name, join_posix, SftpSessionState};
use crate::commands::ssh::SshState;
use crate::error::{AppError, CmdResult};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use veydan_core::Core;

const PART_SUFFIX: &str = ".veydanpart";
const CHUNK_SIZE: usize = 256 * 1024;
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

// ── Inputs / payloads ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferKind {
    Download,
    Upload,
}

#[derive(Debug, Deserialize)]
pub struct TransferItemInput {
    pub src_path: String,
    pub dst_path: String,
    /// Replace the destination if it already exists; `false` skips such files.
    /// Nested files of a directory item inherit its flag. Independent of
    /// resume: an unfinished `.veydanpart` is always continued.
    pub overwrite: bool,
}

#[derive(Serialize, Clone)]
struct TransferProgressPayload {
    transfer_id: String,
    kind: TransferKind,
    current_file: String,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
}

#[derive(Serialize, Clone)]
struct TransferDonePayload {
    transfer_id: String,
    kind: TransferKind,
    error: Option<String>,
    cancelled: bool,
    files_done: u64,
    files_skipped: u64,
}

// ── Job planning ───────────────────────────────────────────────────────────────

struct FileJob {
    src: String,
    dst: String,
    size: u64,
    mode: u32,
    overwrite: bool,
}

struct Plan {
    jobs: Vec<FileJob>,
    total_bytes: u64,
}

/// Recursively expand an upload item (local walk). Directories are created on
/// the remote side immediately; files become jobs. Symlinked files are
/// followed, symlinked directories are skipped (cycle safety).
async fn plan_upload(
    sess: &SftpSessionState,
    src: &str,
    dst: &str,
    overwrite: bool,
    jobs: &mut Vec<FileJob>,
) -> anyhow::Result<()> {
    let md = std::fs::metadata(src)?; // follows symlinks
    if md.is_file() {
        jobs.push(FileJob {
            src: src.to_string(),
            dst: dst.to_string(),
            size: md.len(),
            mode: unix_mode(&md),
            overwrite,
        });
        return Ok(());
    }

    ensure_remote_dir(sess, dst).await?;
    let mut entries: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(src)?
        .filter_map(|r| r.ok())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, path) in entries {
        let link_md = std::fs::symlink_metadata(&path)?;
        let target_md = if link_md.file_type().is_symlink() {
            match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => continue, // don't follow dir symlinks
                Ok(m) => m,
                Err(_) => continue, // broken link
            }
        } else {
            link_md
        };
        let child_dst = join_posix(dst, &name);
        if target_md.is_dir() {
            Box::pin(plan_upload(
                sess,
                &path.to_string_lossy(),
                &child_dst,
                overwrite,
                jobs,
            ))
            .await?;
        } else {
            jobs.push(FileJob {
                src: path.to_string_lossy().into_owned(),
                dst: child_dst,
                size: target_md.len(),
                mode: unix_mode(&target_md),
                overwrite,
            });
        }
    }
    Ok(())
}

/// Recursively expand a download item (remote walk); mirrors `plan_upload`.
async fn plan_download(
    sess: &SftpSessionState,
    src: &str,
    dst: &str,
    overwrite: bool,
    jobs: &mut Vec<FileJob>,
) -> anyhow::Result<()> {
    let md = sess.sftp.metadata(src).await?; // follows symlinks
    if !md.is_dir() {
        jobs.push(FileJob {
            src: src.to_string(),
            dst: dst.to_string(),
            size: md.size.unwrap_or(0),
            mode: md.permissions.unwrap_or(0),
            overwrite,
        });
        return Ok(());
    }

    std::fs::create_dir_all(dst)?;
    let dir = sess.sftp.read_dir(src).await?;
    let mut entries: Vec<_> = dir
        .map(|e| (e.file_name(), e.metadata()))
        .filter(|(n, _)| n != "." && n != "..")
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, md) in entries {
        // Names come from the server; one that could leave `dst` aborts the plan.
        if !is_safe_name(&name) {
            anyhow::bail!("unsafe file name from server: {name:?}");
        }
        let child_src = join_posix(src, &name);
        let child_dst = format!("{}/{}", dst.trim_end_matches('/'), name);
        let md = if md.is_symlink() {
            match sess.sftp.metadata(&child_src).await {
                Ok(t) if t.is_dir() => continue, // don't follow dir symlinks
                Ok(t) => t,
                Err(_) => continue, // broken link
            }
        } else {
            md
        };
        if md.is_dir() {
            Box::pin(plan_download(sess, &child_src, &child_dst, overwrite, jobs)).await?;
        } else {
            jobs.push(FileJob {
                src: child_src,
                dst: child_dst,
                size: md.size.unwrap_or(0),
                mode: md.permissions.unwrap_or(0),
                overwrite,
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn unix_mode(md: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    md.mode()
}
#[cfg(not(unix))]
fn unix_mode(_md: &std::fs::Metadata) -> u32 {
    0
}

/// mkdir -p on the remote side, tolerating already-existing directories.
async fn ensure_remote_dir(sess: &SftpSessionState, path: &str) -> anyhow::Result<()> {
    if let Ok(md) = sess.sftp.metadata(path).await {
        if md.is_dir() {
            return Ok(());
        }
        anyhow::bail!("Destination {path} exists and is not a directory");
    }
    // Create parents first (skip the root)
    let parent = crate::commands::sftp::parent_posix(path);
    if parent != path && parent != "/" {
        Box::pin(ensure_remote_dir(sess, &parent)).await?;
    }
    match sess.sftp.create_dir(path).await {
        Ok(()) => Ok(()),
        // Lost a race / already there — fine as long as it's a dir now
        Err(_)
            if sess
                .sftp
                .metadata(path)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false) =>
        {
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

// ── Progress reporting ─────────────────────────────────────────────────────────

struct ProgressReporter {
    app: AppHandle,
    transfer_id: String,
    kind: TransferKind,
    files_total: u64,
    files_done: u64,
    bytes_total: u64,
    bytes_done: u64,
    current_file: String,
    last_emit: std::time::Instant,
}

impl ProgressReporter {
    fn add_bytes(&mut self, n: u64) {
        self.bytes_done += n;
        if self.last_emit.elapsed() >= PROGRESS_INTERVAL {
            self.emit();
        }
    }

    fn emit(&mut self) {
        self.last_emit = std::time::Instant::now();
        let _ = self.app.emit(
            "sftp://transfer-progress",
            TransferProgressPayload {
                transfer_id: self.transfer_id.clone(),
                kind: self.kind,
                current_file: self.current_file.clone(),
                files_done: self.files_done,
                files_total: self.files_total,
                bytes_done: self.bytes_done,
                bytes_total: self.bytes_total,
            },
        );
    }
}

// ── File copy (chunk loops) ────────────────────────────────────────────────────

enum CopyOutcome {
    Done,
    Skipped,
    Cancelled,
}

async fn download_file(
    sess: &SftpSessionState,
    job: &FileJob,
    token: &CancellationToken,
    progress: &mut ProgressReporter,
) -> anyhow::Result<CopyOutcome> {
    if !job.overwrite && std::fs::symlink_metadata(&job.dst).is_ok() {
        return Ok(CopyOutcome::Skipped);
    }

    let part = format!("{}{}", job.dst, PART_SUFFIX);
    let mut offset = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if offset > job.size {
        offset = 0; // source shrank — restart
    }

    let mut src = sess.sftp.open(&job.src).await?;
    if offset > 0 {
        src.seek(std::io::SeekFrom::Start(offset)).await?;
    }
    let mut dst = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(offset == 0)
        .append(offset > 0)
        .write(offset == 0)
        .open(&part)
        .await?;

    progress.add_bytes(offset);

    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        if token.is_cancelled() {
            dst.flush().await?;
            return Ok(CopyOutcome::Cancelled);
        }
        let n = src.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await?;
        progress.add_bytes(n as u64);
    }
    dst.flush().await?;
    drop(dst);

    if std::fs::symlink_metadata(&job.dst).is_ok() {
        std::fs::remove_file(&job.dst)?;
    }
    std::fs::rename(&part, &job.dst)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if job.mode & 0o7777 != 0 {
            let _ = std::fs::set_permissions(
                &job.dst,
                std::fs::Permissions::from_mode(job.mode & 0o7777),
            );
        }
    }

    Ok(CopyOutcome::Done)
}

async fn upload_file(
    sess: &SftpSessionState,
    job: &FileJob,
    token: &CancellationToken,
    progress: &mut ProgressReporter,
) -> anyhow::Result<CopyOutcome> {
    use russh_sftp::protocol::OpenFlags;

    if !job.overwrite && sess.sftp.try_exists(&job.dst).await.unwrap_or(false) {
        return Ok(CopyOutcome::Skipped);
    }

    let part = format!("{}{}", job.dst, PART_SUFFIX);
    let mut offset = sess
        .sftp
        .metadata(&part)
        .await
        .ok()
        .and_then(|m| m.size)
        .unwrap_or(0);
    if offset > job.size {
        offset = 0;
    }

    let mut src = tokio::fs::File::open(&job.src).await?;
    if offset > 0 {
        src.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    let flags = if offset > 0 {
        OpenFlags::WRITE
    } else {
        OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE
    };
    let mut dst = sess.sftp.open_with_flags(&part, flags).await?;
    if offset > 0 {
        dst.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    progress.add_bytes(offset);

    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        if token.is_cancelled() {
            dst.flush().await?;
            dst.shutdown().await?;
            return Ok(CopyOutcome::Cancelled);
        }
        let n = src.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await?;
        progress.add_bytes(n as u64);
    }
    dst.flush().await?;
    dst.shutdown().await?;
    drop(dst);

    // SFTP rename fails if the target exists — remove a stale destination first
    if sess.sftp.try_exists(&job.dst).await.unwrap_or(false) {
        sess.sftp.remove_file(&job.dst).await?;
    }
    sess.sftp.rename(&part, &job.dst).await?;

    if job.mode & 0o7777 != 0 {
        let attrs = russh_sftp::protocol::FileAttributes {
            size: None,
            uid: None,
            user: None,
            gid: None,
            group: None,
            permissions: Some(job.mode & 0o7777),
            atime: None,
            mtime: None,
        };
        let _ = sess.sftp.set_metadata(&job.dst, attrs).await;
    }

    Ok(CopyOutcome::Done)
}

// ── Transfer runner ────────────────────────────────────────────────────────────

async fn build_plan(
    sess: &SftpSessionState,
    kind: TransferKind,
    items: &[TransferItemInput],
) -> anyhow::Result<Plan> {
    let mut jobs = Vec::new();
    for item in items {
        match kind {
            TransferKind::Upload => {
                plan_upload(
                    sess,
                    &item.src_path,
                    &item.dst_path,
                    item.overwrite,
                    &mut jobs,
                )
                .await?
            }
            TransferKind::Download => {
                plan_download(
                    sess,
                    &item.src_path,
                    &item.dst_path,
                    item.overwrite,
                    &mut jobs,
                )
                .await?
            }
        }
    }
    let total_bytes = jobs.iter().map(|j| j.size).sum();
    Ok(Plan { jobs, total_bytes })
}

async fn run_transfer(
    app: AppHandle,
    sess: Arc<SftpSessionState>,
    kind: TransferKind,
    items: Vec<TransferItemInput>,
    transfer_id: String,
    token: CancellationToken,
) -> (Option<String>, bool, u64, u64) {
    let plan = match build_plan(&sess, kind, &items).await {
        Ok(p) => p,
        Err(e) => return (Some(e.to_string()), false, 0, 0),
    };

    let mut progress = ProgressReporter {
        app: app.clone(),
        transfer_id: transfer_id.clone(),
        kind,
        files_total: plan.jobs.len() as u64,
        files_done: 0,
        bytes_total: plan.total_bytes,
        bytes_done: 0,
        current_file: String::new(),
        last_emit: std::time::Instant::now() - PROGRESS_INTERVAL,
    };
    progress.emit();

    let mut skipped: u64 = 0;
    for job in &plan.jobs {
        if token.is_cancelled() {
            return (None, true, progress.files_done, skipped);
        }
        progress.current_file = job.src.clone();
        progress.emit();

        let outcome = match kind {
            TransferKind::Download => download_file(&sess, job, &token, &mut progress).await,
            TransferKind::Upload => upload_file(&sess, job, &token, &mut progress).await,
        };
        match outcome {
            Ok(CopyOutcome::Done) => progress.files_done += 1,
            Ok(CopyOutcome::Skipped) => {
                skipped += 1;
                // Skipped bytes still count toward the total so the bar completes
                progress.add_bytes(job.size);
            }
            Ok(CopyOutcome::Cancelled) => {
                return (None, true, progress.files_done, skipped);
            }
            Err(e) => {
                return (
                    Some(format!("{}: {}", job.src, e)),
                    false,
                    progress.files_done,
                    skipped,
                );
            }
        }
        progress.emit();
    }

    (None, false, progress.files_done, skipped)
}

// ── Commands ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn sftp_transfer_start(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    kind: String,
    connection_id: String,
    items: Vec<TransferItemInput>,
) -> CmdResult<String> {
    let kind = match kind.as_str() {
        "download" => TransferKind::Download,
        "upload" => TransferKind::Upload,
        other => return Err(AppError::other(format!("Unknown transfer kind: {other}"))),
    };
    if items.is_empty() {
        return Err(AppError::other("Nothing to transfer"));
    }

    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;

    let transfer_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    {
        let mut transfers = ssh
            .sftp
            .transfers
            .lock()
            .map_err(|e| AppError::other(e.to_string()))?;
        transfers.insert(transfer_id.clone(), token.clone());
    }

    let sftp_state = ssh.sftp.clone();
    let id = transfer_id.clone();
    tokio::spawn(async move {
        let (error, cancelled, files_done, files_skipped) =
            run_transfer(app.clone(), sess, kind, items, id.clone(), token).await;

        if let Ok(mut transfers) = sftp_state.transfers.lock() {
            transfers.remove(&id);
        }
        let _ = app.emit(
            "sftp://transfer-done",
            TransferDonePayload {
                transfer_id: id,
                kind,
                error,
                cancelled,
                files_done,
                files_skipped,
            },
        );
    });

    Ok(transfer_id)
}

#[tauri::command]
pub async fn sftp_transfer_cancel(ssh: State<'_, SshState>, transfer_id: String) -> CmdResult<()> {
    let transfers = ssh
        .sftp
        .transfers
        .lock()
        .map_err(|e| AppError::other(e.to_string()))?;
    if let Some(token) = transfers.get(&transfer_id) {
        token.cancel();
    }
    Ok(())
}

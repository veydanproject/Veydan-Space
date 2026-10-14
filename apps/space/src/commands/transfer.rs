// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Transfer engine for the file browser: local↔remote, chunked, cancellable,
//! and resumable.
//!
//! Every file is written to a `<name>.veydanpart` temp next to its final
//! destination and renamed on completion; beside it `<name>.veydanpart.json`
//! records what the part is made from (source path, size, mtime, connection).
//! If a transfer is cancelled or the connection drops, both stay; the next
//! transfer of the same file continues past the bytes already written only
//! when that record matches the source as it is now, and starts from zero
//! otherwise. A part whose length is not the size of the source at the end
//! is an error ("the source changed"), never a finished file. Two running
//! transfers never write into the same place (`SftpState::active`).

use crate::commands::sftp::{
    get_or_connect, is_safe_name, join_posix, SftpSessionState, SftpState,
};
use crate::commands::ssh::SshState;
use crate::error::{AppError, CmdResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use veydan_core::Core;

const PART_SUFFIX: &str = ".veydanpart";
/// What a part is made from, beside it.
const PART_META_SUFFIX: &str = ".veydanpart.json";
/// A record of a part larger than this on a server is not read.
const PART_META_MAX: u64 = 64 * 1024;
const CHUNK_SIZE: usize = 256 * 1024;
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// A name the listings of the file browser leave out: the part of an
/// unfinished transfer and the record beside it.
pub(crate) fn is_transfer_temp(name: &str) -> bool {
    name.ends_with(PART_SUFFIX) || name.ends_with(PART_META_SUFFIX)
}

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
    /// resume: an unfinished `.veydanpart` of the same source is continued.
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
    /// `src_path` of every item that arrived whole: each of its files done,
    /// nothing of it skipped or left out. A move deletes only these.
    complete_items: Vec<String>,
}

// ── Running transfers ──────────────────────────────────────────────────────────

/// A running transfer in `SftpState::active`: where it writes (local files
/// for a download, the server of `conn_id` for an upload) and over which
/// connection it runs.
pub(crate) struct ActiveTransfer {
    pub kind: TransferKind,
    pub conn_id: String,
    pub dsts: Vec<String>,
}

impl ActiveTransfer {
    /// Whether a new item of `kind` over `conn_id` into `dst` would write
    /// where this transfer writes: the same path, one inside it or one
    /// containing it, on the same side.
    fn clashes(&self, kind: TransferKind, conn_id: &str, dst: &str) -> bool {
        let local = kind == TransferKind::Download;
        self.kind == kind
            && (local || self.conn_id == conn_id)
            && self
                .dsts
                .iter()
                .any(|d| paths_overlap(d, dst, local && cfg!(windows)))
    }
}

/// The components of a path; on Windows (`fold`) both separators count and
/// the case does not.
fn path_components(path: &str, fold: bool) -> Vec<String> {
    path.split(|c| c == '/' || (fold && c == '\\'))
        .filter(|c| !c.is_empty() && *c != ".")
        .map(|c| {
            if fold {
                c.to_lowercase()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// `a` and `b` are the same path, or one lies inside the other.
fn paths_overlap(a: &str, b: &str, fold: bool) -> bool {
    let (a, b) = (path_components(a, fold), path_components(b, fold));
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}

/// Holds a transfer in the registries of `SftpState` (where it writes, its
/// cancellation token) for as long as its task runs, panics included.
struct ActiveGuard {
    state: Arc<SftpState>,
    id: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.state
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        self.state
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Cancel every transfer that runs over a connection: for a session that is
/// dropped because its connection was edited, deleted or got a new host key.
pub(crate) fn cancel_connection_transfers(state: &SftpState, connection_id: &str) {
    let ids: Vec<String> = state
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(_, a)| a.conn_id == connection_id)
        .map(|(id, _)| id.clone())
        .collect();
    let transfers = state.transfers.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(token) = transfers.get(&id) {
            token.cancel();
        }
    }
}

// ── Job planning ───────────────────────────────────────────────────────────────

struct FileJob {
    src: String,
    dst: String,
    size: u64,
    /// Seconds since the epoch, when the source side reports it.
    mtime: Option<u64>,
    mode: u32,
    overwrite: bool,
    /// The index of the item of the request this file belongs to.
    item: usize,
}

#[derive(Default)]
struct Plan {
    jobs: Vec<FileJob>,
    total_bytes: u64,
    /// Items with an entry the plan left out (an entry that could not be
    /// read, a link to a directory, a broken link, a name that cannot be
    /// written here): they do not arrive whole.
    incomplete: HashSet<usize>,
    /// Entries left out by the plan.
    skipped: u64,
}

impl Plan {
    fn leave_out(&mut self, item: usize) {
        self.incomplete.insert(item);
        self.skipped += 1;
    }
}

fn local_mtime(md: &std::fs::Metadata) -> Option<u64> {
    md.modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Recursively expand an upload item (local walk). Directories are created on
/// the remote side immediately; files become jobs. Symlinked files are
/// followed, symlinked directories are left out (cycle safety).
async fn plan_upload(
    sess: &SftpSessionState,
    src: &str,
    dst: &str,
    overwrite: bool,
    item: usize,
    plan: &mut Plan,
) -> anyhow::Result<()> {
    let md = std::fs::metadata(src)?; // follows symlinks
    if md.is_file() {
        plan.jobs.push(FileJob {
            src: src.to_string(),
            dst: dst.to_string(),
            size: md.len(),
            mtime: local_mtime(&md),
            mode: unix_mode(&md),
            overwrite,
            item,
        });
        return Ok(());
    }

    ensure_remote_dir(sess, dst).await?;
    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(src)? {
        match entry {
            Ok(e) => entries.push((e.file_name().to_string_lossy().into_owned(), e.path())),
            Err(_) => plan.leave_out(item),
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, path) in entries {
        let Ok(link_md) = std::fs::symlink_metadata(&path) else {
            plan.leave_out(item);
            continue;
        };
        let target_md = if link_md.file_type().is_symlink() {
            match std::fs::metadata(&path) {
                Ok(m) if !m.is_dir() => m,
                // a link to a directory is not followed, a broken one has nothing
                _ => {
                    plan.leave_out(item);
                    continue;
                }
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
                item,
                plan,
            ))
            .await?;
        } else {
            plan.jobs.push(FileJob {
                src: path.to_string_lossy().into_owned(),
                dst: child_dst,
                size: target_md.len(),
                mtime: local_mtime(&target_md),
                mode: unix_mode(&target_md),
                overwrite,
                item,
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
    data_dir: &Path,
    item: usize,
    plan: &mut Plan,
) -> anyhow::Result<()> {
    // The local side is judged before anything is made or written there.
    crate::commands::fs_guard::check_mutable(Path::new(dst), data_dir)?;
    let md = sess.sftp.metadata(src).await?; // follows symlinks
    if !md.is_dir() {
        plan.jobs.push(FileJob {
            src: src.to_string(),
            dst: dst.to_string(),
            size: md.size.unwrap_or(0),
            mtime: md.mtime.map(u64::from),
            mode: md.permissions.unwrap_or(0),
            overwrite,
            item,
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
        // Names come from the server; one that could leave `dst` or cannot be
        // written here is left out (the rest of the folder still arrives).
        if !is_safe_name(&name) {
            plan.leave_out(item);
            continue;
        }
        let child_src = join_posix(src, &name);
        let child_dst = format!("{}/{}", dst.trim_end_matches('/'), name);
        let md = if md.is_symlink() {
            match sess.sftp.metadata(&child_src).await {
                Ok(t) if !t.is_dir() => t,
                // a link to a directory is not followed, a broken one has nothing
                _ => {
                    plan.leave_out(item);
                    continue;
                }
            }
        } else {
            md
        };
        if md.is_dir() {
            Box::pin(plan_download(
                sess, &child_src, &child_dst, overwrite, data_dir, item, plan,
            ))
            .await?;
        } else {
            plan.jobs.push(FileJob {
                src: child_src,
                dst: child_dst,
                size: md.size.unwrap_or(0),
                mtime: md.mtime.map(u64::from),
                mode: md.permissions.unwrap_or(0),
                overwrite,
                item,
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

// ── Resume ─────────────────────────────────────────────────────────────────────

/// What a part is made from, kept beside it as `<dst>.veydanpart.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PartMeta {
    src: String,
    size: u64,
    mtime: Option<u64>,
    conn_id: String,
}

impl PartMeta {
    fn of(job: &FileJob, conn_id: &str) -> Self {
        Self {
            src: job.src.clone(),
            size: job.size,
            mtime: job.mtime,
            conn_id: conn_id.to_string(),
        }
    }
}

/// Where the copy of a file starts in its part: past the bytes already there
/// only when the record beside the part names this very source (path, size,
/// mtime, connection) and the part is not longer than it; from zero when
/// anything differs, the record is missing or the source has no mtime.
fn resume_offset(part_len: Option<u64>, meta: Option<&PartMeta>, want: &PartMeta) -> u64 {
    match (part_len, meta) {
        (Some(len), Some(meta)) if want.mtime.is_some() && meta == want && len <= want.size => len,
        _ => 0,
    }
}

/// The mode a downloaded file gets: the permission bits of the server
/// (never setuid, setgid or sticky) within what the umask let the part be
/// made with (`created`), execute where read is allowed.
fn local_mode(server: u32, created: u32) -> u32 {
    let created = created & 0o666;
    server & 0o777 & (created | (created & 0o444) >> 2)
}

fn source_changed(written: u64, size: u64) -> anyhow::Error {
    anyhow::anyhow!(
        "the source changed during the transfer ({written} bytes instead of {size}); transfer it again"
    )
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
    let meta_path = format!("{}{}", job.dst, PART_META_SUFFIX);
    let want = PartMeta::of(job, &sess.connection_id);
    let part_len = std::fs::metadata(&part)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len());
    let meta = std::fs::read(&meta_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<PartMeta>(&b).ok());
    let offset = resume_offset(part_len, meta.as_ref(), &want);

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
    // Written once the part is opened (and emptied for a fresh start), so a
    // record never stands beside the bytes of another source.
    tokio::fs::write(&meta_path, serde_json::to_vec(&want)?).await?;

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
    dst.sync_all().await?;
    let written = dst.metadata().await?;
    drop(dst);

    if written.len() != job.size {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&meta_path);
        return Err(source_changed(written.len(), job.size));
    }

    // rename replaces an existing destination in one step
    std::fs::rename(&part, &job.dst)?;
    let _ = std::fs::remove_file(&meta_path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = local_mode(job.mode, written.permissions().mode());
        if job.mode & 0o777 != 0 {
            let _ = std::fs::set_permissions(&job.dst, std::fs::Permissions::from_mode(mode));
        }
    }

    Ok(CopyOutcome::Done)
}

/// The record beside a part on the server, if there is a readable one.
async fn read_remote_meta(sess: &SftpSessionState, path: &str) -> Option<PartMeta> {
    let md = sess.sftp.metadata(path).await.ok()?;
    if !md.is_regular() || md.size.unwrap_or(u64::MAX) > PART_META_MAX {
        return None;
    }
    let bytes = sess.sftp.read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_remote_meta(
    sess: &SftpSessionState,
    path: &str,
    meta: &PartMeta,
) -> anyhow::Result<()> {
    use russh_sftp::protocol::OpenFlags;
    let mut f = sess
        .sftp
        .open_with_flags(
            path,
            OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
        )
        .await?;
    f.write_all(&serde_json::to_vec(meta)?).await?;
    f.shutdown().await?;
    Ok(())
}

/// Put `part` in the place of `dst` on the server. A plain SFTP rename
/// refuses an existing target: the old file steps aside under a temp name
/// and comes back if the part cannot take its place; it is removed only
/// once the part stands there.
async fn replace_remote(sess: &SftpSessionState, part: &str, dst: &str) -> anyhow::Result<()> {
    let existing = match sess.sftp.symlink_metadata(dst).await {
        Ok(md) => md,
        Err(_) => return Ok(sess.sftp.rename(part, dst).await?),
    };
    if existing.is_dir() {
        anyhow::bail!("Destination {dst} is a directory");
    }
    if sess.sftp.rename(part, dst).await.is_ok() {
        return Ok(()); // the server replaces on rename
    }
    let old = format!("{dst}.veydanold-{}", Uuid::new_v4().simple());
    sess.sftp.rename(dst, &old).await?;
    if let Err(e) = sess.sftp.rename(part, dst).await {
        let _ = sess.sftp.rename(&old, dst).await;
        return Err(e.into());
    }
    let _ = sess.sftp.remove_file(&old).await;
    Ok(())
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
    let meta_path = format!("{}{}", job.dst, PART_META_SUFFIX);
    let want = PartMeta::of(job, &sess.connection_id);
    let part_len = sess
        .sftp
        .metadata(&part)
        .await
        .ok()
        .filter(|m| m.is_regular())
        .and_then(|m| m.size);
    let meta = read_remote_meta(sess, &meta_path).await;
    let offset = resume_offset(part_len, meta.as_ref(), &want);

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
    // As for a download: the record follows the opened (emptied) part.
    write_remote_meta(sess, &meta_path, &want).await?;

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
    // flush asks the server for fsync@openssh.com when it has it
    dst.flush().await?;
    dst.shutdown().await?;
    drop(dst);

    let written = sess.sftp.metadata(&part).await?.size.unwrap_or(0);
    if written != job.size {
        let _ = sess.sftp.remove_file(&part).await;
        let _ = sess.sftp.remove_file(&meta_path).await;
        return Err(source_changed(written, job.size));
    }

    replace_remote(sess, &part, &job.dst).await?;
    let _ = sess.sftp.remove_file(&meta_path).await;

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
    data_dir: &Path,
    sess: &SftpSessionState,
    kind: TransferKind,
    items: &[TransferItemInput],
) -> anyhow::Result<Plan> {
    let mut plan = Plan::default();
    for (idx, item) in items.iter().enumerate() {
        match kind {
            TransferKind::Upload => {
                plan_upload(
                    sess,
                    &item.src_path,
                    &item.dst_path,
                    item.overwrite,
                    idx,
                    &mut plan,
                )
                .await?
            }
            TransferKind::Download => {
                plan_download(
                    sess,
                    &item.src_path,
                    &item.dst_path,
                    item.overwrite,
                    data_dir,
                    idx,
                    &mut plan,
                )
                .await?
            }
        }
    }
    plan.total_bytes = plan.jobs.iter().map(|j| j.size).sum();
    Ok(plan)
}

/// The items that arrived whole: none of their entries left out by the plan
/// and each of their jobs (`(item, done)`) done. An item without files (an
/// empty folder) is whole when nothing of it was left out.
fn complete_items(
    n_items: usize,
    jobs: &[(usize, bool)],
    incomplete: &HashSet<usize>,
) -> Vec<usize> {
    let mut whole = vec![true; n_items];
    for &i in incomplete {
        if let Some(w) = whole.get_mut(i) {
            *w = false;
        }
    }
    for &(i, done) in jobs {
        if !done {
            if let Some(w) = whole.get_mut(i) {
                *w = false;
            }
        }
    }
    (0..n_items).filter(|&i| whole[i]).collect()
}

struct RunResult {
    error: Option<String>,
    cancelled: bool,
    files_done: u64,
    files_skipped: u64,
    complete_items: Vec<String>,
}

async fn run_transfer(
    app: AppHandle,
    sess: Arc<SftpSessionState>,
    kind: TransferKind,
    items: Vec<TransferItemInput>,
    transfer_id: String,
    data_dir: PathBuf,
    token: CancellationToken,
) -> RunResult {
    let plan = match build_plan(&data_dir, &sess, kind, &items).await {
        Ok(p) => p,
        Err(e) => {
            return RunResult {
                error: Some(e.to_string()),
                cancelled: false,
                files_done: 0,
                files_skipped: 0,
                complete_items: Vec::new(),
            }
        }
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

    let mut skipped: u64 = plan.skipped;
    let mut error = None;
    let mut cancelled = false;
    // A skipped job leaves its item incomplete: it stays false here.
    let mut done = vec![false; plan.jobs.len()];
    for (i, job) in plan.jobs.iter().enumerate() {
        if token.is_cancelled() {
            cancelled = true;
            break;
        }
        progress.current_file = job.src.clone();
        progress.emit();

        let outcome = match kind {
            TransferKind::Download => download_file(&sess, job, &token, &mut progress).await,
            TransferKind::Upload => upload_file(&sess, job, &token, &mut progress).await,
        };
        match outcome {
            Ok(CopyOutcome::Done) => {
                progress.files_done += 1;
                done[i] = true;
            }
            Ok(CopyOutcome::Skipped) => {
                skipped += 1;
                // Skipped bytes still count toward the total so the bar completes
                progress.add_bytes(job.size);
            }
            Ok(CopyOutcome::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(e) => {
                error = Some(format!("{}: {}", job.src, e));
                break;
            }
        }
        progress.emit();
    }

    let jobs: Vec<(usize, bool)> = plan
        .jobs
        .iter()
        .zip(&done)
        .map(|(j, d)| (j.item, *d))
        .collect();
    let complete = complete_items(items.len(), &jobs, &plan.incomplete);
    RunResult {
        error,
        cancelled,
        files_done: progress.files_done,
        files_skipped: skipped,
        complete_items: complete
            .into_iter()
            .map(|i| items[i].src_path.clone())
            .collect(),
    }
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
    // Checked and taken under one lock: two transfers never share a part.
    let guard = {
        let mut active = ssh
            .sftp
            .active
            .lock()
            .map_err(|e| AppError::other(e.to_string()))?;
        if let Some(dst) = items.iter().map(|i| &i.dst_path).find(|dst| {
            active
                .values()
                .any(|a| a.clashes(kind, &connection_id, dst))
        }) {
            return Err(AppError::other(format!(
                "Another transfer is already writing to {dst}"
            )));
        }
        active.insert(
            transfer_id.clone(),
            ActiveTransfer {
                kind,
                conn_id: connection_id.clone(),
                dsts: items.iter().map(|i| i.dst_path.clone()).collect(),
            },
        );
        ssh.sftp
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(transfer_id.clone(), token.clone());
        ActiveGuard {
            state: ssh.sftp.clone(),
            id: transfer_id.clone(),
        }
    };

    let data_dir = core.app_data_dir.clone();
    let id = transfer_id.clone();
    tokio::spawn(async move {
        let result =
            run_transfer(app.clone(), sess, kind, items, id.clone(), data_dir, token).await;
        // Released before the event: the UI may start the next one at once.
        drop(guard);
        let _ = app.emit(
            "sftp://transfer-done",
            TransferDonePayload {
                transfer_id: id,
                kind,
                error: result.error,
                cancelled: result.cancelled,
                files_done: result.files_done,
                files_skipped: result.files_skipped,
                complete_items: result.complete_items,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(src: &str, size: u64, mtime: Option<u64>, conn: &str) -> PartMeta {
        PartMeta {
            src: src.to_string(),
            size,
            mtime,
            conn_id: conn.to_string(),
        }
    }

    #[test]
    fn a_part_continues_only_from_the_same_source() {
        let want = meta("/srv/a.iso", 1000, Some(1_700_000_000), "c1");
        assert_eq!(resume_offset(Some(400), Some(&want.clone()), &want), 400);
        assert_eq!(resume_offset(Some(1000), Some(&want.clone()), &want), 1000);
        // no part, no record, a record that cannot be read
        assert_eq!(resume_offset(None, Some(&want.clone()), &want), 0);
        assert_eq!(resume_offset(Some(400), None, &want), 0);
        // the part is longer than the source
        assert_eq!(resume_offset(Some(1001), Some(&want.clone()), &want), 0);
        // anything of the source differs
        for other in [
            meta("/srv/b.iso", 1000, Some(1_700_000_000), "c1"),
            meta("/srv/a.iso", 1200, Some(1_700_000_000), "c1"),
            meta("/srv/a.iso", 1000, Some(1_700_000_001), "c1"),
            meta("/srv/a.iso", 1000, None, "c1"),
            meta("/srv/a.iso", 1000, Some(1_700_000_000), "c2"),
        ] {
            assert_eq!(
                resume_offset(Some(400), Some(&other), &want),
                0,
                "{other:?}"
            );
        }
        // a source without an mtime is never continued
        let blind = meta("/srv/a.iso", 1000, None, "c1");
        assert_eq!(resume_offset(Some(400), Some(&blind.clone()), &blind), 0);
    }

    #[test]
    fn the_record_reads_back() {
        let m = meta("/srv/a b.iso", 7, Some(3), "c1");
        let bytes = serde_json::to_vec(&m).unwrap();
        assert_eq!(serde_json::from_slice::<PartMeta>(&bytes).unwrap(), m);
        assert!(serde_json::from_slice::<PartMeta>(b"{\"src\":\"x\"}").is_err());
    }

    #[test]
    fn the_temps_of_a_transfer_are_known_by_their_suffix() {
        assert!(is_transfer_temp("a.iso.veydanpart"));
        assert!(is_transfer_temp("a.iso.veydanpart.json"));
        assert!(!is_transfer_temp("a.iso"));
        assert!(!is_transfer_temp("a.veydanpart.txt"));
    }

    #[test]
    fn a_downloaded_file_gets_no_special_bits_and_keeps_the_umask() {
        // umask 022: the part is made 0644
        assert_eq!(local_mode(0o4777, 0o644), 0o755);
        assert_eq!(local_mode(0o2755, 0o644), 0o755);
        assert_eq!(local_mode(0o1777, 0o644), 0o755);
        assert_eq!(local_mode(0o644, 0o644), 0o644);
        assert_eq!(local_mode(0o600, 0o644), 0o600);
        // umask 077: nothing for group and others
        assert_eq!(local_mode(0o777, 0o600), 0o700);
        // umask 002: group write stays
        assert_eq!(local_mode(0o775, 0o664), 0o775);
    }

    #[test]
    fn an_item_is_complete_when_every_file_of_it_arrived() {
        // item 0: two files done; item 1: one done, one skipped;
        // item 2: an empty folder; item 3: an entry left out by the plan;
        // item 4: a file not reached (error or cancel)
        let jobs = [
            (0, true),
            (0, true),
            (1, true),
            (1, false),
            (3, true),
            (4, false),
        ];
        let incomplete: HashSet<usize> = [3].into_iter().collect();
        assert_eq!(complete_items(5, &jobs, &incomplete), vec![0, 2]);
        assert!(complete_items(0, &[], &HashSet::new()).is_empty());
    }

    #[test]
    fn destinations_clash_when_one_lies_in_the_other() {
        assert!(paths_overlap("/home/u/a", "/home/u/a", false));
        assert!(paths_overlap("/home/u/a/", "/home/u/a", false));
        assert!(paths_overlap("/home/u/a", "/home/u/a/b/c", false));
        assert!(paths_overlap("/home/u/a/b", "/home/u/a", false));
        assert!(paths_overlap("/home//u/./a", "/home/u/a", false));
        // a shared prefix of a name is not a path inside
        assert!(!paths_overlap("/home/u/a", "/home/u/ab", false));
        assert!(!paths_overlap("/home/u/a", "/home/u/b", false));
        // case and separators count on unix, not on windows
        assert!(!paths_overlap("/home/u/A", "/home/u/a", false));
        assert!(paths_overlap(
            r"C:\Users\U\Doc",
            "c:/users/u/doc/x.txt",
            true
        ));
        assert!(!paths_overlap(r"C:\Users\U\Doc", r"C:\Users\U\Docs", true));
    }

    #[test]
    fn a_transfer_clashes_on_its_own_side_only() {
        let up = ActiveTransfer {
            kind: TransferKind::Upload,
            conn_id: "c1".into(),
            dsts: vec!["/srv/data".into()],
        };
        assert!(up.clashes(TransferKind::Upload, "c1", "/srv/data/x"));
        assert!(up.clashes(TransferKind::Upload, "c1", "/srv"));
        // another server, or the local side of a download
        assert!(!up.clashes(TransferKind::Upload, "c2", "/srv/data"));
        assert!(!up.clashes(TransferKind::Download, "c1", "/srv/data"));

        let down = ActiveTransfer {
            kind: TransferKind::Download,
            conn_id: "c1".into(),
            dsts: vec!["/home/u/a".into()],
        };
        // local files are one place whatever server they come from
        assert!(down.clashes(TransferKind::Download, "c2", "/home/u/a"));
        assert!(!down.clashes(TransferKind::Download, "c2", "/home/u/b"));
    }
}

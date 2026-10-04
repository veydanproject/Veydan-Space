// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! SFTP sessions for the two-panel file browser.
//!
//! One lazily-established session per `ssh_connections` row, keyed by
//! connection_id and shared by both panels (SFTP multiplexes concurrent
//! requests over a single channel). Transport (direct / SOCKS5 / SSH-jump /
//! HTTP CONNECT) and authentication (password / key / keyboard-interactive
//! 2FA with TOTP auto-answer) are reused from `commands::ssh`.

use crate::commands::ssh::{
    confirm_host_key, do_authenticate, establish_transport, ssh_connection_get, SshInputCommand,
    SshState, TerminalHandler,
};
use crate::error::{AppError, CmdResult};
use crate::models::{format_octal, format_permissions, FileEntry};
use russh::client;
use russh_sftp::client::SftpSession;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use veydan_core::Core;

// ── Session state ──────────────────────────────────────────────────────────────

pub struct SftpSessionState {
    pub connection_id: String,
    pub connection_name: String,
    pub home: String,
    pub connected_at: String,
    pub sftp: SftpSession,
    /// Kept alive so the SSH transport (and jump tunnel, if any) stays open.
    handle: client::Handle<TerminalHandler>,
    _jump: Option<crate::proxy::ssh::SharedSession>,
}

#[derive(Default)]
pub struct SftpState {
    pub sessions: tokio::sync::RwLock<HashMap<String, Arc<SftpSessionState>>>,
    /// Senders for keyboard-interactive prompts of connects in progress,
    /// keyed by connection_id (answered via `sftp_respond_prompt`).
    pub auth_prompts: std::sync::Mutex<HashMap<String, mpsc::Sender<SshInputCommand>>>,
    /// Cancellation tokens of running transfers, keyed by transfer_id.
    pub transfers: std::sync::Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
}

pub type SftpSessions = Arc<SftpState>;

#[derive(Serialize, Clone)]
pub struct SftpSessionInfo {
    pub connection_id: String,
    pub connection_name: String,
    pub home: String,
    pub connected_at: String,
}

impl SftpSessionInfo {
    fn from_state(s: &SftpSessionState) -> Self {
        Self {
            connection_id: s.connection_id.clone(),
            connection_name: s.connection_name.clone(),
            home: s.home.clone(),
            connected_at: s.connected_at.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
struct SftpStatusPayload {
    connection_id: String,
    status: String,
    error: Option<String>,
}

fn emit_status(app: &AppHandle, connection_id: &str, status: &str, error: Option<String>) {
    let _ = app.emit(
        "sftp://status-changed",
        SftpStatusPayload {
            connection_id: connection_id.to_string(),
            status: status.to_string(),
            error,
        },
    );
}

// ── Path helpers (remote paths are POSIX strings, never PathBuf) ───────────────

/// A single path component that cannot escape its parent directory.
pub(crate) fn is_safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

pub(crate) fn join_posix(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub(crate) fn parent_posix(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
    }
}

// ── Connection management ──────────────────────────────────────────────────────

/// Get a live session for the connection, establishing one if needed.
pub(crate) async fn get_or_connect(
    app: &AppHandle,
    core: &State<'_, Core>,
    ssh: &State<'_, SshState>,
    connection_id: &str,
) -> CmdResult<Arc<SftpSessionState>> {
    {
        let sessions = ssh.sftp.sessions.read().await;
        if let Some(s) = sessions.get(connection_id) {
            if !s.handle.is_closed() {
                return Ok(s.clone());
            }
        }
    }
    // Drop the dead entry (re-check under the write lock) and reconnect.
    {
        let mut sessions = ssh.sftp.sessions.write().await;
        if let Some(s) = sessions.get(connection_id) {
            if !s.handle.is_closed() {
                return Ok(s.clone());
            }
            sessions.remove(connection_id);
            emit_status(app, connection_id, "disconnected", None);
        }
    }
    connect_session(app, core, ssh, connection_id).await
}

/// If an operation failed because the transport died, evict the session so
/// the next call reconnects, and notify the frontend.
async fn evict_if_dead(app: &AppHandle, ssh: &State<'_, SshState>, sess: &Arc<SftpSessionState>) {
    if sess.handle.is_closed() {
        let mut sessions = ssh.sftp.sessions.write().await;
        if let Some(current) = sessions.get(&sess.connection_id) {
            if Arc::ptr_eq(current, sess) {
                sessions.remove(&sess.connection_id);
                emit_status(app, &sess.connection_id, "disconnected", None);
            }
        }
    }
}

async fn connect_session(
    app: &AppHandle,
    core: &State<'_, Core>,
    ssh: &State<'_, SshState>,
    connection_id: &str,
) -> CmdResult<Arc<SftpSessionState>> {
    let mut conn = ssh_connection_get(core.clone(), connection_id.to_string()).await?;
    crate::commands::ssh::resolve_key_material(&core.db, &mut conn).await?;

    // Fails closed: a dangling or unroutable `proxy_id` aborts the connect
    // instead of falling back to a direct one (see `proxies::resolve_required`).
    let proxy =
        crate::commands::proxies::resolve_required(&core.db, conn.proxy_id.as_deref()).await?;

    emit_status(app, connection_id, "connecting", None);

    let result = do_connect(app, core, ssh, &conn, proxy.as_ref()).await;

    // Always clear any pending prompt sender for this connection.
    // A poisoned lock is a real error — swallowing it would leave the sender
    // registered and future prompts routed nowhere.
    ssh.sftp
        .auth_prompts
        .lock()
        .map_err(|e| AppError::other(e.to_string()))?
        .remove(connection_id);

    match result {
        Ok(sess) => {
            let sess = {
                let mut sessions = ssh.sftp.sessions.write().await;
                // If a concurrent connect won the race, keep the existing live
                // session — dropping ours closes only our own transport.
                if let Some(existing) = sessions.get(connection_id) {
                    if !existing.handle.is_closed() {
                        existing.clone()
                    } else {
                        sessions.insert(connection_id.to_string(), sess.clone());
                        sess
                    }
                } else {
                    sessions.insert(connection_id.to_string(), sess.clone());
                    sess
                }
            };
            emit_status(app, connection_id, "connected", None);
            Ok(sess)
        }
        Err(e) => {
            let msg = e.to_string();
            emit_status(app, connection_id, "error", Some(msg.clone()));
            Err(AppError::other(msg))
        }
    }
}

async fn do_connect(
    app: &AppHandle,
    core: &State<'_, Core>,
    ssh: &State<'_, SshState>,
    conn: &crate::commands::ssh::SshConnection,
    proxy: Option<&crate::models::Proxy>,
) -> anyhow::Result<Arc<SftpSessionState>> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(std::time::Duration::from_secs(
            conn.keepalive_sec.max(5) as u64
        )),
        ..Default::default()
    });

    let timeout = std::time::Duration::from_secs(conn.connect_timeout_sec.max(1) as u64);

    let (jump, mut handle, received_fp) =
        tokio::time::timeout(timeout, establish_transport(conn, proxy, config, &core.db))
            .await
            .map_err(|_| anyhow::anyhow!("Connection timeout ({}s)", timeout.as_secs()))??;

    // Keyboard-interactive prompts are relayed to the frontend under a
    // synthetic session id; answers arrive through `sftp_respond_prompt`.
    // Failing to register the sender must abort the connect — otherwise the
    // user's prompt answer goes nowhere and auth waits out the 120s timeout.
    let (tx, mut rx) = mpsc::channel::<SshInputCommand>(8);
    ssh.sftp
        .auth_prompts
        .lock()
        .map_err(|e| anyhow::anyhow!("auth prompt registry lock poisoned: {e}"))?
        .insert(conn.id.clone(), tx);
    let prompt_session_id = format!("sftp:{}", conn.id);

    confirm_host_key(
        app,
        &prompt_session_id,
        conn,
        received_fp.as_deref(),
        &mut rx,
    )
    .await?;
    do_authenticate(
        &mut handle,
        app,
        &prompt_session_id,
        &conn.username,
        &conn.auth_type,
        conn.password.as_deref(),
        conn.private_key.as_deref(),
        conn.key_passphrase.as_deref(),
        conn.requires_2fa,
        conn.totp_entry_id.as_deref(),
        &mut rx,
    )
    .await?;

    // Pin the host key on first connect (TOFU) + stamp last_connected_at.
    crate::commands::ssh::persist_connect_success(&core.db, conn, received_fp.as_deref()).await;

    let channel = handle.channel_open_session().await?;
    channel.request_subsystem(true, "sftp").await?;
    let sftp = SftpSession::new(channel.into_stream()).await?;

    let home = sftp
        .canonicalize(".")
        .await
        .unwrap_or_else(|_| "/".to_string());

    Ok(Arc::new(SftpSessionState {
        connection_id: conn.id.clone(),
        connection_name: conn.name.clone(),
        home,
        connected_at: chrono::Utc::now().to_rfc3339(),
        sftp,
        handle,
        _jump: jump,
    }))
}

// ── Listing ────────────────────────────────────────────────────────────────────

async fn list_dir(sess: &SftpSessionState, path: &str) -> anyhow::Result<Vec<FileEntry>> {
    let dir = sess.sftp.read_dir(path).await?;
    let mut entries: Vec<FileEntry> = Vec::new();

    for item in dir {
        let name = item.file_name();
        // Server-supplied names: drop anything that could escape a directory.
        if !is_safe_name(&name) {
            continue;
        }
        let md = item.metadata();
        let full_path = join_posix(path, &name);
        let is_symlink = md.is_symlink();

        // For symlinks report the *target*'s kind so navigation works; stat
        // failures (broken links) must not kill the listing.
        let is_dir = if is_symlink {
            match sess.sftp.metadata(&full_path).await {
                Ok(target) => target.is_dir(),
                Err(_) => false,
            }
        } else {
            md.is_dir()
        };

        let mode = md.permissions.unwrap_or(0);
        entries.push(FileEntry {
            name,
            path: full_path,
            is_dir,
            is_symlink,
            size: md.size.unwrap_or(0),
            mtime: md.mtime.map(|s| s as i64 * 1000),
            mode,
            permissions: format_permissions(mode),
            octal: format_octal(mode),
            owner: md.user.clone().or_else(|| md.uid.map(|u| u.to_string())),
            group: md.group.clone().or_else(|| md.gid.map(|g| g.to_string())),
        });
    }

    entries.sort_by_key(|e| e.name.to_lowercase());
    Ok(entries)
}

// ── Commands ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn sftp_connect(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
) -> CmdResult<SftpSessionInfo> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    Ok(SftpSessionInfo::from_state(&sess))
}

#[tauri::command]
pub async fn sftp_disconnect(
    app: AppHandle,
    ssh: State<'_, SshState>,
    connection_id: String,
) -> CmdResult<()> {
    let removed = {
        let mut sessions = ssh.sftp.sessions.write().await;
        sessions.remove(&connection_id)
    };
    if let Some(sess) = removed {
        let _ = sess.sftp.close().await;
        emit_status(&app, &connection_id, "disconnected", None);
    }
    Ok(())
}

/// Every SFTP session closes and every transfer is cancelled: the module ssh
/// is switched off, or the app quits.
pub(crate) async fn close_all(app: &AppHandle, ssh: &SshState) {
    if let Ok(transfers) = ssh.sftp.transfers.lock() {
        for token in transfers.values() {
            token.cancel();
        }
    }
    let sessions: Vec<_> = ssh.sftp.sessions.write().await.drain().collect();
    for (connection_id, sess) in sessions {
        let _ = sess.sftp.close().await;
        emit_status(app, &connection_id, "disconnected", None);
    }
}

#[tauri::command]
pub async fn sftp_session_list(ssh: State<'_, SshState>) -> CmdResult<Vec<SftpSessionInfo>> {
    let sessions = ssh.sftp.sessions.read().await;
    Ok(sessions
        .values()
        .map(|s| SftpSessionInfo::from_state(s))
        .collect())
}

#[tauri::command]
pub async fn sftp_home(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
) -> CmdResult<String> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    Ok(sess.home.clone())
}

#[tauri::command]
pub async fn sftp_list(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
) -> CmdResult<Vec<FileEntry>> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    match list_dir(&sess, &path).await {
        Ok(entries) => Ok(entries),
        Err(e) => {
            evict_if_dead(&app, &ssh, &sess).await;
            Err(AppError::other(e))
        }
    }
}

// ── Mutating operations ────────────────────────────────────────────────────────

/// Wrap a fallible SFTP op: on transport death, evict the session so the next
/// call reconnects, and surface the error.
async fn guard<T>(
    app: &AppHandle,
    ssh: &State<'_, SshState>,
    sess: &Arc<SftpSessionState>,
    result: anyhow::Result<T>,
) -> CmdResult<T> {
    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            evict_if_dead(app, ssh, sess).await;
            Err(AppError::other(e))
        }
    }
}

#[tauri::command]
pub async fn sftp_mkdir(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
) -> CmdResult<()> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    let r = sess.sftp.create_dir(&path).await.map_err(Into::into);
    guard(&app, &ssh, &sess, r).await
}

#[tauri::command]
pub async fn sftp_create_file(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
) -> CmdResult<()> {
    use russh_sftp::protocol::OpenFlags;
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    let r: anyhow::Result<()> = async {
        if sess.sftp.try_exists(&path).await.unwrap_or(false) {
            anyhow::bail!("Target already exists: {path}");
        }
        // EXCL rejects an existing file; CREATE|WRITE makes an empty one
        let f = sess
            .sftp
            .open_with_flags(
                &path,
                OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::EXCLUDE,
            )
            .await?;
        f.sync_all().await.ok();
        Ok(())
    }
    .await;
    guard(&app, &ssh, &sess, r).await
}

#[tauri::command]
pub async fn sftp_rename(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    from: String,
    to: String,
) -> CmdResult<()> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    let r: anyhow::Result<()> = async {
        if sess.sftp.try_exists(&to).await.unwrap_or(false) {
            anyhow::bail!("Target already exists: {to}");
        }
        sess.sftp.rename(&from, &to).await?;
        Ok(())
    }
    .await;
    guard(&app, &ssh, &sess, r).await
}

/// Recursively delete a remote path. Directories are emptied depth-first,
/// then removed; files (and symlinks) are unlinked directly.
async fn remove_recursive(sess: &SftpSessionState, path: &str) -> anyhow::Result<()> {
    // lstat so we unlink symlinks rather than descending into their targets
    let md = sess.sftp.symlink_metadata(path).await?;
    if md.is_dir() && !md.is_symlink() {
        let dir = sess.sftp.read_dir(path).await?;
        let names: Vec<String> = dir
            .map(|e| e.file_name())
            .filter(|n| n != "." && n != "..")
            .collect();
        for name in names {
            Box::pin(remove_recursive(sess, &join_posix(path, &name))).await?;
        }
        sess.sftp.remove_dir(path).await?;
    } else {
        sess.sftp.remove_file(path).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn sftp_delete(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
) -> CmdResult<()> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    let r = remove_recursive(&sess, &path).await;
    guard(&app, &ssh, &sess, r).await
}

#[tauri::command]
pub async fn sftp_chmod(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
    mode: u32,
) -> CmdResult<()> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    let attrs = russh_sftp::protocol::FileAttributes {
        size: None,
        uid: None,
        user: None,
        gid: None,
        group: None,
        permissions: Some(mode & 0o7777),
        atime: None,
        mtime: None,
    };
    let r = sess
        .sftp
        .set_metadata(&path, attrs)
        .await
        .map_err(Into::into);
    guard(&app, &ssh, &sess, r).await
}

/// Stat a single remote path; `None` if it doesn't exist. Used for
/// conflict detection before transfers.
#[tauri::command]
pub async fn sftp_stat(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
    path: String,
) -> CmdResult<Option<FileEntry>> {
    let sess = get_or_connect(&app, &core, &ssh, &connection_id).await?;
    match sess.sftp.metadata(&path).await {
        Ok(md) => {
            let mode = md.permissions.unwrap_or(0);
            let name = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .to_string();
            Ok(Some(FileEntry {
                name,
                path: path.clone(),
                is_dir: md.is_dir(),
                is_symlink: md.is_symlink(),
                size: md.size.unwrap_or(0),
                mtime: md.mtime.map(|s| s as i64 * 1000),
                mode,
                permissions: format_permissions(mode),
                octal: format_octal(mode),
                owner: md.user.clone().or_else(|| md.uid.map(|u| u.to_string())),
                group: md.group.clone().or_else(|| md.gid.map(|g| g.to_string())),
            }))
        }
        Err(russh_sftp::client::error::Error::Status(status))
            if status.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
        {
            Ok(None)
        }
        Err(e) => {
            evict_if_dead(&app, &ssh, &sess).await;
            Err(AppError::other(e))
        }
    }
}

#[tauri::command]
pub async fn sftp_respond_prompt(
    ssh: State<'_, SshState>,
    connection_id: String,
    response: String,
) -> CmdResult<()> {
    let sender = ssh
        .sftp
        .auth_prompts
        .lock()
        .map_err(|e| AppError::other(e.to_string()))?
        .get(&connection_id)
        .cloned();
    if let Some(tx) = sender {
        let _ = tx.send(SshInputCommand::PromptResponse(response)).await;
    }
    Ok(())
}

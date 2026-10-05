// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use crate::error::{AppError, CmdResult};
use crate::modules::directory;
use base64::Engine as _;
use chrono::Utc;
use russh::keys::{decode_secret_key, HashAlg, PrivateKeyWithHashAlg};
use russh::{client, ChannelMsg};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use uuid::Uuid;
use veydan_core::Core;

// ── DB row ─────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct SshConnectionRow {
    id: String,
    name: String,
    host: String,
    port: i64,
    username: String,
    auth_type: String,
    password: Option<String>,
    private_key: Option<String>,
    key_passphrase: Option<String>,
    ssh_key_id: Option<String>,
    requires_2fa: i64,
    totp_entry_id: Option<String>,
    proxy_id: Option<String>,
    connect_timeout_sec: i64,
    keepalive_sec: i64,
    terminal_theme: Option<String>,
    default_cols: i64,
    default_rows: i64,
    /// SHA256 fingerprint of the server host key, pinned on first successful
    /// connection (TOFU) — same scheme as `proxies.server_fingerprint`.
    server_fingerprint: Option<String>,
    last_connected_at: Option<String>,
    created_at: String,
    updated_at: String,
}

// ── Output model ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SshConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    // Secrets stay in the backend; the UI only learns whether one is stored.
    #[serde(skip_serializing)]
    pub password: Option<String>,
    #[serde(skip_serializing)]
    pub private_key: Option<String>,
    #[serde(skip_serializing)]
    pub key_passphrase: Option<String>,
    pub has_password: bool,
    pub has_private_key: bool,
    pub has_key_passphrase: bool,
    pub ssh_key_id: Option<String>,
    pub requires_2fa: bool,
    pub totp_entry_id: Option<String>,
    pub proxy_id: Option<String>,
    /// Name of the referenced proxy, resolved by `load_links`. `None` while
    /// `proxy_id` is `Some` means the proxy row is gone: the connect will fail
    /// closed, and the UI must show that instead of a plain "proxy" badge.
    pub proxy_name: Option<String>,
    pub workspace_ids: Vec<String>,
    pub profile_ids: Vec<String>,
    pub connect_timeout_sec: i64,
    pub keepalive_sec: i64,
    pub terminal_theme: Option<String>,
    pub default_cols: i64,
    pub default_rows: i64,
    pub server_fingerprint: Option<String>,
    pub last_connected_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn has_value(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

impl From<SshConnectionRow> for SshConnection {
    fn from(r: SshConnectionRow) -> Self {
        SshConnection {
            id: r.id,
            name: r.name,
            host: r.host,
            port: r.port,
            username: r.username,
            auth_type: r.auth_type,
            has_password: has_value(&r.password),
            has_private_key: has_value(&r.private_key),
            has_key_passphrase: has_value(&r.key_passphrase),
            password: r.password,
            private_key: r.private_key,
            key_passphrase: r.key_passphrase,
            ssh_key_id: r.ssh_key_id,
            requires_2fa: r.requires_2fa != 0,
            totp_entry_id: r.totp_entry_id,
            proxy_id: r.proxy_id,
            proxy_name: None,
            workspace_ids: vec![],
            profile_ids: vec![],
            connect_timeout_sec: r.connect_timeout_sec,
            keepalive_sec: r.keepalive_sec,
            terminal_theme: r.terminal_theme,
            default_cols: r.default_cols,
            default_rows: r.default_rows,
            server_fingerprint: r.server_fingerprint,
            last_connected_at: r.last_connected_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

// ── Create / Update inputs ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SshConnectionCreateInput {
    pub name: String,
    pub host: String,
    pub port: Option<i64>,
    pub username: String,
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub key_passphrase: Option<String>,
    pub ssh_key_id: Option<String>,
    pub requires_2fa: Option<bool>,
    pub totp_entry_id: Option<String>,
    pub proxy_id: Option<String>,
    pub workspace_ids: Option<Vec<String>>,
    pub profile_ids: Option<Vec<String>>,
    pub connect_timeout_sec: Option<i64>,
    pub keepalive_sec: Option<i64>,
    pub terminal_theme: Option<String>,
    pub default_cols: Option<i64>,
    pub default_rows: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SshConnectionUpdateInput {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub key_passphrase: Option<String>,
    pub ssh_key_id: Option<String>,
    pub requires_2fa: Option<bool>,
    pub totp_entry_id: Option<String>,
    pub proxy_id: Option<String>,
    pub workspace_ids: Option<Vec<String>>,
    pub profile_ids: Option<Vec<String>>,
    pub connect_timeout_sec: Option<i64>,
    pub keepalive_sec: Option<i64>,
    pub terminal_theme: Option<String>,
    pub default_cols: Option<i64>,
    pub default_rows: Option<i64>,
}

// ── Session types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SshStatus {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct SshSessionInfo {
    pub session_id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub host: String,
    pub port: i64,
    pub status: SshStatus,
    pub error: Option<String>,
    pub connected_at: Option<String>,
}

pub enum SshInputCommand {
    Data(Vec<u8>),
    Resize {
        cols: u32,
        rows: u32,
    },
    Disconnect,
    /// Response to a keyboard-interactive prompt relayed by the frontend
    PromptResponse(String),
}

pub struct SshSessionState {
    pub info: SshSessionInfo,
    pub writer: mpsc::Sender<SshInputCommand>,
}

pub type SshSessions = Arc<std::sync::RwLock<HashMap<String, SshSessionState>>>;

/// The terminals and the SFTP sessions that are open.
#[derive(Default)]
pub struct SshState {
    pub sessions: SshSessions,
    pub sftp: crate::commands::sftp::SftpSessions,
}

// ── Event payloads ─────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct StatusChangedPayload {
    session_id: String,
    connection_id: String,
    status: SshStatus,
    error: Option<String>,
}

#[derive(Serialize, Clone)]
struct DataPayload {
    session_id: String,
    data_base64: String,
}

#[derive(Serialize, Clone)]
struct KeyboardPromptPayload {
    session_id: String,
    name: String,
    instructions: String,
    prompts: Vec<KeyboardPromptItem>,
    /// Set for the first-connection host-key question; the UI shows trust/reject buttons.
    host_key: Option<String>,
}

#[derive(Serialize, Clone)]
struct KeyboardPromptItem {
    prompt: String,
    echo: bool,
}

// ── russh Handler ──────────────────────────────────────────────────────────────

/// Marker prefix of the host-key-mismatch error. The frontend greps for it
/// (see `parseHostKeyMismatch` in ui/src/lib/core/utils.ts) to show the
/// "trust new host key" prompt instead of a plain error.
pub(crate) const HOST_KEY_MISMATCH_MARKER: &str = "HOST_KEY_MISMATCH";

/// Host-key verification for terminal + SFTP sessions (TOFU, mirrors
/// `proxy::ssh::SshHandler`): first connection is accepted and the fingerprint
/// is pinned after successful auth; a pinned fingerprint must match exactly.
pub(crate) struct TerminalHandler {
    /// Fingerprint stored in `ssh_connections.server_fingerprint` (None = first connection).
    known_fingerprint: Option<String>,
    /// Fingerprint received from the server during this handshake.
    received_fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for TerminalHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        let fp = crate::proxy::ssh::compute_fingerprint(server_public_key);
        if let Ok(mut slot) = self.received_fingerprint.lock() {
            *slot = Some(fp.clone());
        }

        let result = match &self.known_fingerprint {
            // First connection — accept; the caller pins it after successful auth
            None => Ok(true),
            Some(known) if known == &fp => Ok(true),
            // Fingerprint mismatch — possible MITM, reject the handshake
            Some(_) => Err(russh::Error::WrongServerSig),
        };

        std::future::ready(result)
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// One answer from the prompt overlay; disconnect or 120 s silence aborts.
async fn wait_prompt_response(rx: &mut mpsc::Receiver<SshInputCommand>) -> anyhow::Result<String> {
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx.recv()).await {
            Ok(Some(SshInputCommand::PromptResponse(r))) => return Ok(r),
            Ok(Some(SshInputCommand::Disconnect)) | Ok(None) => {
                anyhow::bail!("Disconnected during authentication");
            }
            Err(_) => anyhow::bail!("Authentication timeout (120s)"),
            Ok(Some(_)) => {}
        }
    }
}

/// First connection to a host: the user approves the fingerprint before any
/// credential (password, TOTP answer) is sent. Pinned hosts skip this.
pub(crate) async fn confirm_host_key(
    app: &AppHandle,
    session_id: &str,
    conn: &SshConnection,
    received_fp: Option<&str>,
    rx: &mut mpsc::Receiver<SshInputCommand>,
) -> anyhow::Result<()> {
    if conn.server_fingerprint.is_some() {
        return Ok(());
    }
    let Some(fp) = received_fp else { return Ok(()) };
    let _ = app.emit(
        "ssh://keyboard-prompt",
        KeyboardPromptPayload {
            session_id: session_id.to_string(),
            name: "Host key verification".into(),
            instructions: format!("{}:{} {fp}", conn.host, conn.port),
            prompts: vec![KeyboardPromptItem {
                prompt: "Trust this host? (yes/no)".into(),
                echo: true,
            }],
            host_key: Some(fp.to_string()),
        },
    );
    let answer = wait_prompt_response(rx).await?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "yes" | "y") {
        Ok(())
    } else {
        anyhow::bail!("Host key {fp} was not trusted; connection aborted")
    }
}

fn emit_status(
    app: &AppHandle,
    session_id: &str,
    connection_id: &str,
    status: SshStatus,
    error: Option<String>,
) {
    let _ = app.emit(
        "ssh://status-changed",
        StatusChangedPayload {
            session_id: session_id.to_string(),
            connection_id: connection_id.to_string(),
            status,
            error,
        },
    );
}

fn emit_data(app: &AppHandle, session_id: &str, data: &[u8]) {
    let data_base64 = base64::engine::general_purpose::STANDARD.encode(data);
    let _ = app.emit(
        "ssh://data",
        DataPayload {
            session_id: session_id.to_string(),
            data_base64,
        },
    );
}

/// Authenticate with the SSH server based on auth_type.
/// Shared by the terminal (`run_session`) and the SFTP subsystem (`commands::sftp`).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_authenticate(
    handle: &mut client::Handle<TerminalHandler>,
    app: &AppHandle,
    session_id: &str,
    username: &str,
    auth_type: &str,
    password: Option<&str>,
    private_key: Option<&str>,
    key_passphrase: Option<&str>,
    requires_2fa: bool,
    totp_entry_id: Option<&str>,
    rx: &mut mpsc::Receiver<SshInputCommand>,
) -> anyhow::Result<()> {
    match auth_type {
        "password" => {
            if requires_2fa {
                do_keyboard_interactive(
                    handle,
                    app,
                    session_id,
                    username,
                    password,
                    totp_entry_id,
                    rx,
                )
                .await
            } else {
                let pw = password.unwrap_or("");
                let result = handle.authenticate_password(username, pw).await?;
                if matches!(result, russh::client::AuthResult::Success) {
                    Ok(())
                } else {
                    anyhow::bail!("Password authentication failed")
                }
            }
        }
        "key" | "key_password" => {
            let pem = private_key.ok_or_else(|| anyhow::anyhow!("No private key provided"))?;
            let pem = pem.trim();

            let key = if auth_type == "key_password" {
                let passphrase = key_passphrase.unwrap_or("");
                russh::keys::PrivateKey::from_openssh(pem)
                    .or_else(|_| decode_secret_key(pem, Some(passphrase)))
                    .map_err(|e| anyhow::anyhow!("Failed to parse private key: {e}"))?
            } else {
                russh::keys::PrivateKey::from_openssh(pem)
                    .or_else(|_| decode_secret_key(pem, None))
                    .map_err(|e| anyhow::anyhow!("Failed to parse private key: {e}"))?
            };

            let algo = key.algorithm();
            let algo_str = format!("{algo}");
            let fingerprint = key.fingerprint(HashAlg::Sha256);
            let hash_alg = if algo.is_rsa() {
                Some(HashAlg::Sha256)
            } else {
                None
            };
            let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
            let auth_algo = key_with_alg.algorithm();

            let result = handle
                .authenticate_publickey(username, key_with_alg)
                .await?;
            match result {
                russh::client::AuthResult::Success => Ok(()),
                russh::client::AuthResult::Failure {
                    partial_success: true,
                    ref remaining_methods,
                } if remaining_methods
                    .iter()
                    .any(|m| matches!(m, russh::MethodKind::KeyboardInteractive)) =>
                {
                    do_keyboard_interactive(
                        handle,
                        app,
                        session_id,
                        username,
                        password,
                        totp_entry_id,
                        rx,
                    )
                    .await
                }
                russh::client::AuthResult::Failure {
                    remaining_methods, ..
                } => {
                    let methods: Vec<String> =
                        remaining_methods.iter().map(|m| format!("{m:?}")).collect();
                    anyhow::bail!(
                        "Public key authentication failed.\n\
                         Key: {algo_str} / {auth_algo}, Fingerprint: {fingerprint}\n\
                         Server remaining: [{}]\n\
                         Check that the public key is in ~/.ssh/authorized_keys on the server.",
                        methods.join(", ")
                    )
                }
            }
        }
        other => anyhow::bail!("Unknown auth_type: {other}"),
    }
}

/// Keyboard-interactive auth.
/// Prompts are auto-answered with:
///   1. Stored password (if prompt contains "password")
///   2. TOTP code generated from linked profile (if prompt contains "verification" / "code" / etc.)
///   3. Otherwise relayed to the frontend overlay for interactive input.
async fn do_keyboard_interactive(
    handle: &mut client::Handle<TerminalHandler>,
    app: &AppHandle,
    session_id: &str,
    username: &str,
    auto_password: Option<&str>,
    totp_entry_id: Option<&str>,
    rx: &mut mpsc::Receiver<SshInputCommand>,
) -> anyhow::Result<()> {
    let mut result = handle
        .authenticate_keyboard_interactive_start(username, None)
        .await?;

    loop {
        match result {
            russh::client::KeyboardInteractiveAuthResponse::Success => return Ok(()),
            russh::client::KeyboardInteractiveAuthResponse::Failure { .. } => {
                anyhow::bail!("Keyboard-interactive authentication failed")
            }
            russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let mut answers: Vec<String> = Vec::with_capacity(prompts.len());

                for (i, prompt) in prompts.iter().enumerate() {
                    let prompt_lower = prompt.prompt.to_lowercase();

                    let is_password_prompt = prompt_lower.contains("password")
                        || prompt_lower.contains("пароль")
                        || prompt_lower.contains("passwd");

                    let is_totp_prompt = prompt_lower.contains("verification")
                        || prompt_lower.contains("code")
                        || prompt_lower.contains("totp")
                        || prompt_lower.contains("2fa")
                        || prompt_lower.contains("authenticator")
                        || prompt_lower.contains("otp");

                    let auto = if is_password_prompt {
                        auto_password.map(|s| s.to_string())
                    } else if is_totp_prompt {
                        // The current code of the linked TOTP entry, from its owner
                        if let Some(entry_id) = totp_entry_id {
                            totp_code(app, entry_id).await
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let answer = if let Some(a) = auto {
                        a
                    } else {
                        // Relay the whole InfoRequest to frontend — frontend sends one response per prompt
                        let _ = app.emit(
                            "ssh://keyboard-prompt",
                            KeyboardPromptPayload {
                                session_id: session_id.to_string(),
                                name: name.clone(),
                                instructions: instructions.clone(),
                                // Only send remaining prompts starting from i
                                prompts: prompts[i..]
                                    .iter()
                                    .map(|p| KeyboardPromptItem {
                                        prompt: p.prompt.clone(),
                                        echo: p.echo,
                                    })
                                    .collect(),
                                host_key: None,
                            },
                        );
                        wait_prompt_response(rx).await?
                    };

                    answers.push(answer);
                }

                result = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

/// The current code of the TOTP entry `entry_id`, asked of the entity
/// directory: the entry belongs to pass. None when the product has no pass,
/// the entry is gone or the app is locked — the user is asked instead.
async fn totp_code(app: &AppHandle, entry_id: &str) -> Option<String> {
    app.state::<Core>()
        .directory
        .action("totp", entry_id, "code", None)
        .await
        .ok()
}

// ── CRUD Commands ──────────────────────────────────────────────────────────────

/// Load workspace_ids and profile_ids for a list of connections in one query each.
async fn load_links(
    pool: &sqlx::SqlitePool,
    connections: &mut [SshConnection],
) -> anyhow::Result<()> {
    if connections.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = connections.iter().map(|c| c.id.clone()).collect();
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    // workspace links
    let ws_query = format!(
        "SELECT connection_id, workspace_id FROM ssh_connection_workspaces WHERE connection_id IN ({})",
        placeholders
    );
    let mut q = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(ws_query));
    for id in &ids {
        q = q.bind(id);
    }
    let ws_rows = q.fetch_all(pool).await?;

    // profile links
    let pr_query = format!(
        "SELECT connection_id, profile_id FROM ssh_connection_profiles WHERE connection_id IN ({})",
        placeholders
    );
    let mut q = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(pr_query));
    for id in &ids {
        q = q.bind(id);
    }
    let pr_rows = q.fetch_all(pool).await?;

    // Referenced proxy names, so the UI can tell "uses proxy X" from "points at a
    // proxy that no longer exists" (the latter refuses to connect at launch).
    let proxy_ids: Vec<String> = connections
        .iter()
        .filter_map(|c| c.proxy_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let proxy_rows: Vec<(String, String)> = if proxy_ids.is_empty() {
        vec![]
    } else {
        let px_query = format!(
            "SELECT id, name FROM proxies WHERE id IN ({})",
            proxy_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",")
        );
        let mut q = sqlx::query_as::<_, (String, String)>(sqlx::AssertSqlSafe(px_query));
        for id in &proxy_ids {
            q = q.bind(id);
        }
        q.fetch_all(pool).await?
    };

    for conn in connections.iter_mut() {
        conn.proxy_name = conn.proxy_id.as_ref().and_then(|pid| {
            proxy_rows
                .iter()
                .find(|(id, _)| id == pid)
                .map(|(_, name)| name.clone())
        });
        conn.workspace_ids = ws_rows
            .iter()
            .filter(|(cid, _)| cid == &conn.id)
            .map(|(_, wid)| wid.clone())
            .collect();
        conn.profile_ids = pr_rows
            .iter()
            .filter(|(cid, _)| cid == &conn.id)
            .map(|(_, pid)| pid.clone())
            .collect();
    }
    Ok(())
}

/// Replace all workspace links for a connection.
async fn sync_workspace_links(
    pool: &sqlx::SqlitePool,
    connection_id: &str,
    workspace_ids: &[String],
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM ssh_connection_workspaces WHERE connection_id = ?")
        .bind(connection_id)
        .execute(pool)
        .await?;
    for wid in workspace_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO ssh_connection_workspaces (connection_id, workspace_id) VALUES (?, ?)",
        )
        .bind(connection_id)
        .bind(wid)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Replace all profile links for a connection.
async fn sync_profile_links(
    pool: &sqlx::SqlitePool,
    connection_id: &str,
    profile_ids: &[String],
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM ssh_connection_profiles WHERE connection_id = ?")
        .bind(connection_id)
        .execute(pool)
        .await?;
    for pid in profile_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO ssh_connection_profiles (connection_id, profile_id) VALUES (?, ?)",
        )
        .bind(connection_id)
        .bind(pid)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_connection_list(
    core: State<'_, Core>,
    workspace_id: Option<String>,
    profile_id: Option<String>,
    search: Option<String>,
) -> CmdResult<Vec<SshConnection>> {
    // Build query depending on context filter
    let sql = if profile_id.is_some() {
        // Profile context: only connections linked to this profile
        "SELECT DISTINCT c.* FROM ssh_connections c
         JOIN ssh_connection_profiles p ON p.connection_id = c.id
         WHERE p.profile_id = ?1
           AND (?2 IS NULL OR lower(c.name) LIKE lower(?2) ESCAPE '\\' OR lower(c.host) LIKE lower(?2) ESCAPE '\\' OR lower(c.username) LIKE lower(?2) ESCAPE '\\')
         ORDER BY c.name"
    } else if workspace_id.is_some() {
        // Workspace context: global (no links) OR linked to this workspace
        "SELECT DISTINCT c.* FROM ssh_connections c
         WHERE (
           NOT EXISTS (SELECT 1 FROM ssh_connection_workspaces w WHERE w.connection_id = c.id)
           OR EXISTS (SELECT 1 FROM ssh_connection_workspaces w WHERE w.connection_id = c.id AND w.workspace_id = ?1)
         )
         AND (?2 IS NULL OR lower(c.name) LIKE lower(?2) ESCAPE '\\' OR lower(c.host) LIKE lower(?2) ESCAPE '\\' OR lower(c.username) LIKE lower(?2) ESCAPE '\\')
         ORDER BY c.name"
    } else {
        // Global: all connections
        "SELECT c.* FROM ssh_connections c
         WHERE ?1 IS NULL
           AND (?2 IS NULL OR lower(c.name) LIKE lower(?2) ESCAPE '\\' OR lower(c.host) LIKE lower(?2) ESCAPE '\\' OR lower(c.username) LIKE lower(?2) ESCAPE '\\')
         ORDER BY c.name"
    };

    let filter_param = profile_id.as_deref().or(workspace_id.as_deref());
    // Escape LIKE wildcards so user input matches literally (paired with ESCAPE '\')
    let search_param = search.as_deref().map(|s| {
        let escaped = s
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        format!("%{escaped}%")
    });

    let rows = sqlx::query_as::<_, SshConnectionRow>(sql)
        .bind(filter_param)
        .bind(search_param.as_deref())
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;

    let mut conns: Vec<SshConnection> = rows.into_iter().map(SshConnection::from).collect();
    load_links(&core.db, &mut conns)
        .await
        .map_err(AppError::other)?;
    Ok(conns)
}

#[tauri::command]
pub async fn ssh_connection_get(core: State<'_, Core>, id: String) -> CmdResult<SshConnection> {
    let row = sqlx::query_as::<_, SshConnectionRow>("SELECT * FROM ssh_connections WHERE id = ?")
        .bind(&id)
        .fetch_optional(&core.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::not_found(format!("SSH connection {id}")))?;

    let mut conn = SshConnection::from(row);
    let mut vec = vec![conn];
    load_links(&core.db, &mut vec)
        .await
        .map_err(AppError::other)?;
    conn = vec.remove(0);
    Ok(conn)
}

/// `Some("")` → `None`: the frontend sends the empty string for "no value" on
/// nullable fields (see the update-semantics comment on `ssh_connection_update`).
fn none_if_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

#[tauri::command]
pub async fn ssh_connection_create(
    core: State<'_, Core>,
    input: SshConnectionCreateInput,
) -> CmdResult<SshConnection> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "INSERT INTO ssh_connections (
            id, name, host, port, username, auth_type,
            password, private_key, key_passphrase, ssh_key_id,
            requires_2fa, totp_entry_id, proxy_id,
            connect_timeout_sec, keepalive_sec, terminal_theme,
            default_cols, default_rows, created_at, updated_at
        ) VALUES (
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?,
            ?, ?, ?,
            ?, ?, ?,
            ?, ?, ?, ?
        )",
    )
    .bind(&id)
    .bind(&input.name)
    .bind(&input.host)
    .bind(input.port.unwrap_or(22))
    .bind(&input.username)
    .bind(&input.auth_type)
    .bind(none_if_empty(&input.password))
    .bind(none_if_empty(&input.private_key))
    .bind(none_if_empty(&input.key_passphrase))
    .bind(none_if_empty(&input.ssh_key_id))
    .bind(input.requires_2fa.unwrap_or(false) as i64)
    .bind(none_if_empty(&input.totp_entry_id))
    .bind(none_if_empty(&input.proxy_id))
    .bind(input.connect_timeout_sec.unwrap_or(15))
    .bind(input.keepalive_sec.unwrap_or(30))
    .bind(none_if_empty(&input.terminal_theme))
    .bind(input.default_cols.unwrap_or(120))
    .bind(input.default_rows.unwrap_or(32))
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::SSH, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    if let Some(ref wids) = input.workspace_ids {
        sync_workspace_links(&core.db, &id, wids)
            .await
            .map_err(AppError::other)?;
    }
    if let Some(ref pids) = input.profile_ids {
        sync_profile_links(&core.db, &id, pids)
            .await
            .map_err(AppError::other)?;
    }

    ssh_connection_get(core, id).await
}

/// Partial update.
///
/// Semantics (must match what the frontend sends — see SSHConnectionForm.svelte
/// and SshProfileTab.svelte):
/// - Required scalars (name, host, port, username, auth_type, requires_2fa,
///   connect_timeout_sec, keepalive_sec, default_cols, default_rows):
///   `None` = keep the current value (COALESCE).
/// - Nullable/clearable fields (password, private_key, key_passphrase,
///   ssh_key_id, totp_entry_id, proxy_id, terminal_theme) are three-state:
///   `None` (field omitted) = keep, `Some("")` = clear to NULL, `Some(v)` = set.
///   The edit form always sends the full desired state, using `""` for
///   "cleared / detached / no saved key"; partial callers (e.g. profile
///   link/unlink sending only `profile_ids`) omit them entirely, so stored
///   secrets can no longer be wiped by omission.
/// - workspace_ids / profile_ids: `None` = keep links, `Some(list)` = replace.
/// - server_fingerprint / last_connected_at are managed by the connect flow
///   and `ssh_connection_trust_fingerprint`, never by this command.
#[tauri::command]
pub async fn ssh_connection_update(
    core: State<'_, Core>,
    id: String,
    input: SshConnectionUpdateInput,
) -> CmdResult<SshConnection> {
    let now = Utc::now().to_rfc3339();

    let mut tx = core.db.begin().await.map_err(AppError::db)?;
    sqlx::query(
        "UPDATE ssh_connections SET
            name                = COALESCE(?, name),
            host                = COALESCE(?, host),
            port                = COALESCE(?, port),
            username            = COALESCE(?, username),
            auth_type           = COALESCE(?, auth_type),
            password            = CASE WHEN ? IS NULL THEN password       WHEN ? = '' THEN NULL ELSE ? END,
            private_key         = CASE WHEN ? IS NULL THEN private_key    WHEN ? = '' THEN NULL ELSE ? END,
            key_passphrase      = CASE WHEN ? IS NULL THEN key_passphrase WHEN ? = '' THEN NULL ELSE ? END,
            ssh_key_id          = CASE WHEN ? IS NULL THEN ssh_key_id     WHEN ? = '' THEN NULL ELSE ? END,
            requires_2fa        = COALESCE(?, requires_2fa),
            totp_entry_id       = CASE WHEN ? IS NULL THEN totp_entry_id  WHEN ? = '' THEN NULL ELSE ? END,
            proxy_id            = CASE WHEN ? IS NULL THEN proxy_id       WHEN ? = '' THEN NULL ELSE ? END,
            connect_timeout_sec = COALESCE(?, connect_timeout_sec),
            keepalive_sec       = COALESCE(?, keepalive_sec),
            terminal_theme      = CASE WHEN ? IS NULL THEN terminal_theme WHEN ? = '' THEN NULL ELSE ? END,
            default_cols        = COALESCE(?, default_cols),
            default_rows        = COALESCE(?, default_rows),
            updated_at          = ?
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.host)
    .bind(input.port)
    .bind(&input.username)
    .bind(&input.auth_type)
    .bind(&input.password).bind(&input.password).bind(&input.password)
    .bind(&input.private_key).bind(&input.private_key).bind(&input.private_key)
    .bind(&input.key_passphrase).bind(&input.key_passphrase).bind(&input.key_passphrase)
    .bind(&input.ssh_key_id).bind(&input.ssh_key_id).bind(&input.ssh_key_id)
    .bind(input.requires_2fa.map(|v| v as i64))
    .bind(&input.totp_entry_id).bind(&input.totp_entry_id).bind(&input.totp_entry_id)
    .bind(&input.proxy_id).bind(&input.proxy_id).bind(&input.proxy_id)
    .bind(input.connect_timeout_sec)
    .bind(input.keepalive_sec)
    .bind(&input.terminal_theme).bind(&input.terminal_theme).bind(&input.terminal_theme)
    .bind(input.default_cols)
    .bind(input.default_rows)
    .bind(&now)
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::SSH, &id).await?;
    tx.commit().await.map_err(AppError::db)?;

    if let Some(ref wids) = input.workspace_ids {
        sync_workspace_links(&core.db, &id, wids)
            .await
            .map_err(AppError::other)?;
    }
    if let Some(ref pids) = input.profile_ids {
        sync_profile_links(&core.db, &id, pids)
            .await
            .map_err(AppError::other)?;
    }

    ssh_connection_get(core, id).await
}

#[tauri::command]
pub async fn ssh_connection_delete(core: State<'_, Core>, id: String) -> CmdResult<()> {
    // Delete paths do not lean on the schema's ON DELETE CASCADE on the two link
    // tables (see `db::schemas`) — drop those rows explicitly, in one transaction
    // with the delete so a failure cannot leave the row gone but its links behind.
    // Mirrors the delete of the `ssh_connection` entity of sync (`modules::ssh`).
    let mut tx = core.db.begin().await.map_err(AppError::db)?;

    sqlx::query("DELETE FROM ssh_connection_workspaces WHERE connection_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM ssh_connection_profiles WHERE connection_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;

    sqlx::query("DELETE FROM ssh_connections WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::db)?;
    directory::relabel(&core.directory, &mut tx, &directory::SSH, &id).await?;

    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

/// Save a trusted SSH server host-key fingerprint (the user confirmed the new
/// fingerprint in UI after a `HOST_KEY_MISMATCH` error — TOFU re-pin, mirrors
/// `proxy_trust_fingerprint`).
#[tauri::command]
pub async fn ssh_connection_trust_fingerprint(
    core: State<'_, Core>,
    id: String,
    fingerprint: String,
) -> CmdResult<()> {
    sqlx::query("UPDATE ssh_connections SET server_fingerprint = ?, updated_at = ? WHERE id = ?")
        .bind(&fingerprint)
        .bind(Utc::now().to_rfc3339())
        .bind(&id)
        .execute(&core.db)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

/// Resolve a saved-key reference (`ssh_key_id`) into inline key material.
/// Called at connect time only (terminal + SFTP), so `ssh_connection_get`
/// keeps returning raw rows for the edit form.
pub async fn resolve_key_material(
    db: &sqlx::SqlitePool,
    conn: &mut SshConnection,
) -> Result<(), AppError> {
    if conn.auth_type != "key" && conn.auth_type != "key_password" {
        return Ok(());
    }
    let Some(key_id) = conn.ssh_key_id.clone() else {
        return Ok(());
    };
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT private_key, passphrase FROM ssh_keys WHERE id = ?")
            .bind(&key_id)
            .fetch_optional(db)
            .await
            .map_err(AppError::db)?;
    let (private_key, passphrase) = row.ok_or_else(|| {
        AppError::not_found(format!(
            "Saved SSH key {key_id} no longer exists — edit the connection and pick another key"
        ))
    })?;
    let has_passphrase = passphrase.as_deref().is_some_and(|p| !p.is_empty());
    conn.private_key = Some(private_key);
    conn.key_passphrase = passphrase;
    // Normalize so do_authenticate applies the stored passphrase regardless of
    // what auth_type the connection row says.
    conn.auth_type = if has_passphrase {
        "key_password".into()
    } else {
        "key".into()
    };
    Ok(())
}

// ── Session Commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn ssh_session_list(ssh: State<'_, SshState>) -> CmdResult<Vec<SshSessionInfo>> {
    let sessions = ssh
        .sessions
        .read()
        .map_err(|e| AppError::other(e.to_string()))?;
    Ok(sessions.values().map(|s| s.info.clone()).collect())
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    core: State<'_, Core>,
    ssh: State<'_, SshState>,
    connection_id: String,
) -> CmdResult<String> {
    let mut conn = ssh_connection_get(core.clone(), connection_id.clone()).await?;
    resolve_key_material(&core.db, &mut conn).await?;

    let session_id = Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<SshInputCommand>(64);

    let info = SshSessionInfo {
        session_id: session_id.clone(),
        connection_id: connection_id.clone(),
        connection_name: conn.name.clone(),
        host: conn.host.clone(),
        port: conn.port,
        status: SshStatus::Connecting,
        error: None,
        connected_at: None,
    };

    {
        let mut sessions = ssh
            .sessions
            .write()
            .map_err(|e| AppError::other(e.to_string()))?;
        sessions.insert(
            session_id.clone(),
            SshSessionState {
                info: info.clone(),
                writer: tx,
            },
        );
    }

    emit_status(
        &app,
        &session_id,
        &connection_id,
        SshStatus::Connecting,
        None,
    );

    // Load the proxy if one is configured. Fails closed: a dangling or
    // unroutable `proxy_id` aborts the connect instead of going out directly.
    let proxy =
        crate::commands::proxies::resolve_required(&core.db, conn.proxy_id.as_deref()).await?;

    let sessions_arc = ssh.sessions.clone();
    let db = core.db.clone();
    let session_id_ret = session_id.clone();

    tokio::spawn(async move {
        let result = run_session(&app, &session_id, &conn, proxy, db, rx).await;

        let (status, error) = match result {
            Ok(()) => (SshStatus::Disconnected, None),
            Err(e) => (SshStatus::Error, Some(e.to_string())),
        };

        emit_status(&app, &session_id, &conn.id, status, error);

        // Prune the finished session — the registry only holds live sessions.
        // The frontend keeps its own copy of session info (updated via the
        // status event above) for the disconnected banner / reconnect UX, and
        // the terminal buffer lives entirely on the frontend side, so nothing
        // needs `ssh_session_list` to keep returning dead sessions.
        // (`ssh_session_remove` from the frontend then becomes a no-op.)
        if let Ok(mut sessions) = sessions_arc.write() {
            sessions.remove(&session_id);
        }
    });

    Ok(session_id_ret)
}

/// Establishes an HTTP CONNECT tunnel through an HTTP/HTTPS proxy.
async fn http_connect_tunnel(
    proxy_host: &str,
    proxy_port: u16,
    target_host: &str,
    target_port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> anyhow::Result<tokio::net::TcpStream> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream =
        tokio::net::TcpStream::connect(format!("{}:{}", proxy_host, proxy_port)).await?;

    let mut request = format!(
        "CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n",
        target_host, target_port, target_host, target_port
    );

    if let (Some(user), Some(pass)) = (username, password) {
        if !user.is_empty() {
            let credentials =
                base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, pass));
            request.push_str(&format!("Proxy-Authorization: Basic {}\r\n", credentials));
        }
    }
    request.push_str("\r\n");

    stream.write_all(request.as_bytes()).await?;

    // Read response until \r\n\r\n
    let mut buf = Vec::with_capacity(512);
    loop {
        let mut tmp = [0u8; 256];
        let n = stream.read(&mut tmp).await?;
        anyhow::ensure!(n > 0, "HTTP proxy closed connection during CONNECT");
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        anyhow::ensure!(buf.len() < 8192, "HTTP proxy response too large");
    }

    let response = String::from_utf8_lossy(&buf);
    let status_line = response.lines().next().unwrap_or("");
    anyhow::ensure!(
        status_line.contains("200"),
        "HTTP proxy CONNECT failed: {}",
        status_line
    );

    Ok(stream)
}

/// Establish the SSH transport for a connection: direct TCP, or tunneled
/// through a SOCKS5 / SSH-jump / HTTP-CONNECT proxy.
/// Shared by the terminal (`run_session`) and the SFTP subsystem (`commands::sftp`).
///
/// The returned `SharedSession` (if any) is the jump-host session — the caller
/// must keep it alive for as long as the handle is used, or the tunnel closes.
///
/// The last tuple element is the host-key fingerprint the *target* server
/// presented (TOFU: pin it via `persist_connect_success` after auth succeeds).
/// If a pinned fingerprint doesn't match, this fails with a
/// `HOST_KEY_MISMATCH`-marked error the frontend recognizes.
/// What a transport needs kept for as long as the session lives: the session
/// of the jump host, or the lease of the tor instance. Never read, only held.
#[allow(dead_code)]
pub(crate) enum TransportHold {
    Jump(crate::proxy::ssh::SharedSession),
    Tor(veydan_tor::Lease),
}

pub(crate) async fn establish_transport(
    conn: &SshConnection,
    proxy: Option<&crate::models::Proxy>,
    config: Arc<client::Config>,
    db: &sqlx::SqlitePool,
    tor: &veydan_tor::TorManager,
) -> anyhow::Result<(
    Option<TransportHold>,
    client::Handle<TerminalHandler>,
    Option<String>,
)> {
    let host = conn.host.as_str();
    let port = conn.port as u16;

    let received: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let make_handler = || TerminalHandler {
        known_fingerprint: conn.server_fingerprint.clone(),
        received_fingerprint: Arc::clone(&received),
    };

    let result: anyhow::Result<(Option<TransportHold>, client::Handle<TerminalHandler>)> = async {
        Ok(match proxy {
            // Tor: the instance of the row's exit countries, with circuits
            // of this connection's own. No lease, no connection.
            Some(p) if crate::proxy::tor::is_tor(p) => {
                let lease = crate::proxy::tor::lease(tor, p, &format!("ssh:{}", conn.id))
                    .await
                    .map_err(anyhow::Error::msg)?;
                let stream = crate::proxy::local::socks5_connect(
                    "127.0.0.1",
                    lease.socks_port,
                    host,
                    port,
                    Some(&lease.username),
                    Some(&lease.password),
                )
                .await?;
                (
                    Some(TransportHold::Tor(lease)),
                    client::connect_stream(config, stream, make_handler()).await?,
                )
            }
            Some(p) if p.proxy_type == "socks5" => {
                let stream = crate::proxy::local::socks5_connect(
                    &p.host,
                    p.port as u16,
                    host,
                    port,
                    p.username.as_deref(),
                    p.password.as_deref(),
                )
                .await?;
                (
                    None,
                    client::connect_stream(config, stream, make_handler()).await?,
                )
            }
            Some(p) if p.proxy_type == "ssh" => {
                let jump_auth = if let Some(ref key) = p.private_key {
                    crate::proxy::ssh::SshAuth::PrivateKey(key.clone())
                } else {
                    crate::proxy::ssh::SshAuth::Password(p.password.clone().unwrap_or_default())
                };
                let jump_result = crate::proxy::ssh::SshSession::connect(
                    &p.host,
                    p.port as u16,
                    p.username.as_deref().unwrap_or("root"),
                    jump_auth,
                    p.server_fingerprint.clone(),
                )
                .await?;
                crate::commands::proxies::pin_ssh_fingerprint(db, p, &jump_result).await;
                let channel = jump_result.session.open_channel(host, port).await?;
                let stream = channel.into_stream();
                // Keep the jump session Arc alive — dropping it would close the tunnel
                let session_arc = jump_result.session.clone();
                (
                    Some(TransportHold::Jump(session_arc)),
                    client::connect_stream(config, stream, make_handler()).await?,
                )
            }
            Some(p) if p.proxy_type == "http" || p.proxy_type == "https" => {
                // HTTP CONNECT tunnel
                let stream = http_connect_tunnel(
                    &p.host,
                    p.port as u16,
                    host,
                    port,
                    p.username.as_deref(),
                    p.password.as_deref(),
                )
                .await?;
                (
                    None,
                    client::connect_stream(config, stream, make_handler()).await?,
                )
            }
            None => (
                None,
                client::connect(config, (host, port), make_handler()).await?,
            ),
            // `proxies::resolve_required` rejects unsupported types before we get
            // here, so this is defence in depth — but "proxy configured, type not
            // implemented" must never resolve to a direct connection, which is
            // exactly what the previous catch-all arm did.
            Some(p) => anyhow::bail!(
                "Proxy '{}' has type '{}', which no SSH transport implements — \
                 refusing to connect directly.",
                p.name,
                p.proxy_type
            ),
        })
    }
    .await;

    let received_fp = received.lock().ok().and_then(|g| g.clone());
    match result {
        Ok((jump, handle)) => Ok((jump, handle, received_fp)),
        Err(e) => {
            // Distinguish a host-key mismatch from generic connect failures so
            // the frontend can offer to trust the new fingerprint.
            if let (Some(known), Some(recv)) =
                (conn.server_fingerprint.as_deref(), received_fp.as_deref())
            {
                if known != recv {
                    anyhow::bail!(
                        "{HOST_KEY_MISMATCH_MARKER}:{recv}: SSH server host key changed \
                         (pinned {known}, received {recv}) — possible man-in-the-middle attack. \
                         Verify the server before trusting the new key."
                    );
                }
            }
            Err(e)
        }
    }
}

/// Persist connection bookkeeping right after successful authentication:
/// pin the host-key fingerprint on first connect (TOFU, mirrors
/// `proxies.server_fingerprint`) and stamp `last_connected_at`.
/// DB failures are logged, never fatal — the live session keeps working.
pub(crate) async fn persist_connect_success(
    db: &sqlx::SqlitePool,
    conn: &SshConnection,
    received_fp: Option<&str>,
) {
    if conn.server_fingerprint.is_none() {
        if let Some(fp) = received_fp {
            if let Err(e) = sqlx::query(
                "UPDATE ssh_connections SET server_fingerprint = ? \
                 WHERE id = ? AND server_fingerprint IS NULL",
            )
            .bind(fp)
            .bind(&conn.id)
            .execute(db)
            .await
            {
                eprintln!(
                    "[ssh] failed to pin host-key fingerprint for {}: {e}",
                    conn.id
                );
            }
        }
    }
    if let Err(e) = sqlx::query("UPDATE ssh_connections SET last_connected_at = ? WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(&conn.id)
        .execute(db)
        .await
    {
        eprintln!(
            "[ssh] failed to update last_connected_at for {}: {e}",
            conn.id
        );
    }
}

async fn run_session(
    app: &AppHandle,
    session_id: &str,
    conn: &SshConnection,
    proxy: Option<crate::models::Proxy>,
    db: sqlx::SqlitePool,
    mut rx: mpsc::Receiver<SshInputCommand>,
) -> anyhow::Result<()> {
    let config = Arc::new(client::Config::default());

    let tor = app.state::<veydan_tor::TorManager>().inner().clone();
    // _hold keeps the jump session or the tor lease for the duration of the session
    let (_hold, mut handle, received_fp) =
        establish_transport(conn, proxy.as_ref(), config, &db, &tor).await?;
    confirm_host_key(app, session_id, conn, received_fp.as_deref(), &mut rx).await?;

    // Authenticate
    do_authenticate(
        &mut handle,
        app,
        session_id,
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

    // Pin the host key (first connect) + stamp last_connected_at now that
    // the server is authenticated — not when the session eventually ends.
    persist_connect_success(&db, conn, received_fp.as_deref()).await;

    // Open shell channel
    let mut channel = handle.channel_open_session().await?;

    channel
        .request_pty(
            false,
            "xterm-256color",
            conn.default_cols as u32,
            conn.default_rows as u32,
            0,
            0,
            &[],
        )
        .await?;

    channel.request_shell(false).await?;

    emit_status(app, session_id, &conn.id, SshStatus::Connected, None);

    // Main loop: forward channel output to frontend, forward frontend input to channel
    loop {
        tokio::select! {
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { ref data }) => {
                        emit_data(app, session_id, data);
                    }
                    Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                        emit_data(app, session_id, data);
                    }
                    Some(ChannelMsg::ExitStatus { .. }) | None => break,
                    _ => {}
                }
            }
            cmd = rx.recv() => {
                match cmd {
                    Some(SshInputCommand::Data(data)) => {
                        channel.data(data.as_slice()).await?;
                    }
                    Some(SshInputCommand::Resize { cols, rows }) => {
                        channel.window_change(cols, rows, 0, 0).await?;
                    }
                    Some(SshInputCommand::Disconnect) | None => break,
                    Some(SshInputCommand::PromptResponse(_)) => {} // handled during auth phase
                }
            }
        }
    }

    channel.close().await?;
    Ok(())
}

/// Every terminal session is told to disconnect: the module ssh is switched
/// off, or the app quits. Each session ends as `ssh_disconnect` ends it.
pub(crate) async fn disconnect_all(ssh: &SshState) {
    let senders: Vec<_> = match ssh.sessions.read() {
        Ok(sessions) => sessions.values().map(|s| s.writer.clone()).collect(),
        Err(_) => return,
    };
    for tx in senders {
        let _ = tx.send(SshInputCommand::Disconnect).await;
    }
}

#[tauri::command]
pub async fn ssh_disconnect(ssh: State<'_, SshState>, session_id: String) -> CmdResult<()> {
    let sender = {
        let sessions = ssh
            .sessions
            .read()
            .map_err(|e| AppError::other(e.to_string()))?;
        sessions.get(&session_id).map(|s| s.writer.clone())
    };
    if let Some(tx) = sender {
        let _ = tx.send(SshInputCommand::Disconnect).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_send_data(
    ssh: State<'_, SshState>,
    session_id: String,
    data: Vec<u8>,
) -> CmdResult<()> {
    let sender = {
        let sessions = ssh
            .sessions
            .read()
            .map_err(|e| AppError::other(e.to_string()))?;
        sessions.get(&session_id).map(|s| s.writer.clone())
    };
    if let Some(tx) = sender {
        let _ = tx.send(SshInputCommand::Data(data)).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_resize(
    ssh: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> CmdResult<()> {
    let sender = {
        let sessions = ssh
            .sessions
            .read()
            .map_err(|e| AppError::other(e.to_string()))?;
        sessions.get(&session_id).map(|s| s.writer.clone())
    };
    if let Some(tx) = sender {
        let _ = tx.send(SshInputCommand::Resize { cols, rows }).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn ssh_session_remove(ssh: State<'_, SshState>, session_id: String) -> CmdResult<()> {
    let mut sessions = ssh
        .sessions
        .write()
        .map_err(|e| AppError::other(e.to_string()))?;
    sessions.remove(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn ssh_respond_prompt(
    ssh: State<'_, SshState>,
    session_id: String,
    response: String,
) -> CmdResult<()> {
    let sender = {
        let sessions = ssh
            .sessions
            .read()
            .map_err(|e| AppError::other(e.to_string()))?;
        sessions.get(&session_id).map(|s| s.writer.clone())
    };
    if let Some(tx) = sender {
        let _ = tx.send(SshInputCommand::PromptResponse(response)).await;
    }
    Ok(())
}

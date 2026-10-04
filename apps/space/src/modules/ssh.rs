// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! SSH connections and keys, the terminal, the file manager (SFTP and the
//! local file system).

#[cfg(desktop)]
use veydan_core::{AppError, BoxFuture};
use veydan_shell::Module;
#[cfg(desktop)]
use veydan_shell::{TrayGroup, TrayItem, TrayLabels, TrayPart};
#[cfg(desktop)]
use veydan_sync_host::{Delete, Hooks, LinkSpec, Ref, Registry, TableSpec};

pub(crate) fn module() -> Module {
    Module {
        schema: Some(crate::db::SSH),
        #[cfg(desktop)]
        stop: Some(stop),
        // A phone has no terminal and no files: nothing to switch.
        #[cfg(mobile)]
        switch: veydan_shell::Switch::Always,
        #[cfg(desktop)]
        directory: Some(super::directory::ssh),
        #[cfg(desktop)]
        sync: Some(sync),
        #[cfg(desktop)]
        demo: Some(crate::commands::demo::ssh()),
        #[cfg(desktop)]
        tray: Some(TrayPart {
            items: tray_items,
            on_click: tray_click,
            tooltip: None,
        }),
        ..commands()
    }
}

#[cfg(desktop)]
fn commands() -> Module {
    veydan_shell::module! {
        id: "ssh",
        setup: setup,
        commands: [
            crate::commands::ssh::ssh_connection_list,
            crate::commands::ssh::ssh_connection_get,
            crate::commands::ssh::ssh_connection_create,
            crate::commands::ssh::ssh_connection_update,
            crate::commands::ssh::ssh_connection_delete,
            crate::commands::ssh::ssh_connection_trust_fingerprint,
            crate::commands::ssh::ssh_connect,
            crate::commands::ssh::ssh_disconnect,
            crate::commands::ssh::ssh_send_data,
            crate::commands::ssh::ssh_resize,
            crate::commands::ssh::ssh_session_list,
            crate::commands::ssh::ssh_session_remove,
            crate::commands::ssh::ssh_respond_prompt,
            crate::commands::ssh_keys::ssh_key_list,
            crate::commands::ssh_keys::ssh_key_get,
            crate::commands::ssh_keys::ssh_key_import,
            crate::commands::ssh_keys::ssh_key_generate,
            crate::commands::ssh_keys::ssh_key_update,
            crate::commands::ssh_keys::ssh_key_delete,
            crate::commands::ssh_keys::ssh_key_export_private,
            crate::commands::sftp::sftp_connect,
            crate::commands::sftp::sftp_disconnect,
            crate::commands::sftp::sftp_session_list,
            crate::commands::sftp::sftp_home,
            crate::commands::sftp::sftp_list,
            crate::commands::sftp::sftp_stat,
            crate::commands::sftp::sftp_respond_prompt,
            crate::commands::transfer::sftp_transfer_start,
            crate::commands::transfer::sftp_transfer_cancel,
            crate::commands::sftp::sftp_mkdir,
            crate::commands::sftp::sftp_create_file,
            crate::commands::sftp::sftp_rename,
            crate::commands::sftp::sftp_delete,
            crate::commands::sftp::sftp_chmod,
            crate::commands::fs::fs_home,
            crate::commands::fs::fs_list,
            crate::commands::fs::fs_stat,
            crate::commands::fs::fs_mkdir,
            crate::commands::fs::fs_create_file,
            crate::commands::fs::fs_rename,
            crate::commands::fs::fs_delete,
            crate::commands::fs::fs_chmod,
        ],
    }
}

/// Keys and connections sync between desktops; a phone has neither.
#[cfg(desktop)]
fn sync(registry: &mut Registry) {
    registry.table(
        TableSpec {
            delete: Delete::Plain(&[
                "UPDATE ssh_connections SET ssh_key_id = NULL WHERE ssh_key_id = ?",
            ]),
            ..TableSpec::plain(
                "ssh_key",
                "ssh_keys",
                &[
                    "name",
                    "algorithm",
                    "bits",
                    "comment",
                    "private_key",
                    "public_key",
                    "passphrase",
                    "fingerprint",
                    "source",
                    "created_at",
                    "updated_at",
                ],
            )
        },
        Hooks::default(),
    );
    registry.table(
        TableSpec {
            links: &[
                LinkSpec {
                    table: "ssh_connection_workspaces",
                    parent_col: "connection_id",
                    child_col: "workspace_id",
                    key: "workspace_ids",
                    child_table: Some("workspaces"),
                },
                LinkSpec {
                    table: "ssh_connection_profiles",
                    parent_col: "connection_id",
                    child_col: "profile_id",
                    key: "profile_ids",
                    child_table: None,
                },
            ],
            refs: &[
                Ref {
                    key: "proxy_id",
                    table: "proxies",
                },
                Ref {
                    key: "ssh_key_id",
                    table: "ssh_keys",
                },
            ],
            // Delete paths do not lean on the schema's ON DELETE CASCADE on both link
            // tables (see `db::schemas`) — drop the link rows explicitly.
            delete: Delete::Plain(&[
                "DELETE FROM ssh_connection_workspaces WHERE connection_id = ?",
                "DELETE FROM ssh_connection_profiles WHERE connection_id = ?",
            ]),
            ..TableSpec::plain(
                "ssh_connection",
                "ssh_connections",
                &[
                    "name",
                    "host",
                    "port",
                    "username",
                    "auth_type",
                    "password",
                    "private_key",
                    "key_passphrase",
                    "requires_2fa",
                    "totp_entry_id",
                    "proxy_id",
                    "ssh_key_id",
                    "connect_timeout_sec",
                    "keepalive_sec",
                    "terminal_theme",
                    "default_cols",
                    "default_rows",
                    "server_fingerprint",
                    "created_at",
                    "updated_at",
                ],
            )
        },
        Hooks {
            after_row: Some(super::directory::ssh_synced),
            ..Hooks::default()
        },
    );
}

#[cfg(desktop)]
fn setup(app: &mut tauri::App) -> veydan_shell::SetupResult {
    use tauri::Manager;
    app.manage(crate::commands::ssh::SshState::default());
    Ok(())
}

/// The module is switched off, or the app quits: the open terminal and SFTP
/// sessions close. Nothing starts again by itself when it is switched on.
#[cfg(desktop)]
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    use tauri::Manager;
    Box::pin(async move {
        let ssh = app.state::<crate::commands::ssh::SshState>();
        crate::commands::ssh::disconnect_all(&ssh).await;
        crate::commands::sftp::close_all(&app, &ssh).await;
        Ok(())
    })
}

/// A phone keeps the tables, which sync fills, and has no commands for them.
#[cfg(mobile)]
fn commands() -> Module {
    veydan_shell::module! { id: "ssh", setup: |_app| Ok(()), commands: [] }
}

#[cfg(desktop)]
fn tray_items(_app: tauri::AppHandle, labels: TrayLabels) -> BoxFuture<'static, Vec<TrayItem>> {
    use veydan_shell::tray_label;
    Box::pin(async move {
        vec![
            TrayItem::entry(
                TrayGroup::Sections,
                "nav:/terminal",
                tray_label(&labels, "section_terminal", "Terminal"),
            ),
            TrayItem::entry(
                TrayGroup::Sections,
                "nav:/files",
                tray_label(&labels, "section_files", "Files"),
            ),
        ]
    })
}

#[cfg(desktop)]
fn tray_click(app: &tauri::AppHandle, id: &str) {
    super::tray_navigate(app, id);
}

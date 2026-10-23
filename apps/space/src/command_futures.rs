// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a command leaves on the thread that calls it. On Android a command
//! is called on the WebView's bridge thread, whose stack is small: Tauri
//! builds the command's future there and moves it a few times before it is
//! spawned, so a large one overflows that stack and the app dies (живой
//! запуск 2026-10-23: `channel_open_link` of the channels, 19 KB, SIGSEGV
//! in a release build). A command whose work is large boxes it
//! (`Box::pin`), so that only the box crosses the bridge. The size is read
//! from the type of the command, without calling it.

fn r0<R>(_: fn() -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r1<A, R>(_: fn(A) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r2<A, B, R>(_: fn(A, B) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r3<A, B, C, R>(_: fn(A, B, C) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r4<A, B, C, D, R>(_: fn(A, B, C, D) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r5<A, B, C, D, E, R>(_: fn(A, B, C, D, E) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r6<A, B, C, D, E, G, R>(_: fn(A, B, C, D, E, G) -> R) -> usize {
    std::mem::size_of::<R>()
}

/// The bound every command keeps: well below a stack of the bridge thread
/// even when Tauri copies the future a few times.
const MAX_FUTURE_BYTES: usize = 4096;

#[test]
#[allow(clippy::vec_init_then_push)]
fn every_command_leaves_a_small_future_on_the_calling_thread() {
    let mut v: Vec<(&str, usize)> = Vec::new();
    v.push(("notes_capture_rules_get", r1(crate::capture::rules::notes_capture_rules_get)));
    v.push(("notes_capture_rules_set", r2(crate::capture::rules::notes_capture_rules_set)));
    v.push(("sftp_connect", r4(crate::commands::sftp::sftp_connect)));
    v.push(("sftp_disconnect", r3(crate::commands::sftp::sftp_disconnect)));
    v.push(("sftp_session_list", r1(crate::commands::sftp::sftp_session_list)));
    v.push(("sftp_home", r4(crate::commands::sftp::sftp_home)));
    v.push(("sftp_list", r5(crate::commands::sftp::sftp_list)));
    v.push(("sftp_mkdir", r5(crate::commands::sftp::sftp_mkdir)));
    v.push(("sftp_create_file", r5(crate::commands::sftp::sftp_create_file)));
    v.push(("sftp_rename", r6(crate::commands::sftp::sftp_rename)));
    v.push(("sftp_delete", r5(crate::commands::sftp::sftp_delete)));
    v.push(("sftp_chmod", r6(crate::commands::sftp::sftp_chmod)));
    v.push(("sftp_stat", r5(crate::commands::sftp::sftp_stat)));
    v.push(("sftp_respond_prompt", r3(crate::commands::sftp::sftp_respond_prompt)));
    v.push(("fs_home", r0(crate::commands::fs::fs_home)));
    v.push(("fs_stat", r2(crate::commands::fs::fs_stat)));
    v.push(("fs_list", r2(crate::commands::fs::fs_list)));
    v.push(("fs_mkdir", r2(crate::commands::fs::fs_mkdir)));
    v.push(("fs_create_file", r2(crate::commands::fs::fs_create_file)));
    v.push(("fs_rename", r3(crate::commands::fs::fs_rename)));
    v.push(("fs_delete", r2(crate::commands::fs::fs_delete)));
    v.push(("fs_chmod", r3(crate::commands::fs::fs_chmod)));
    v.push(("workspace_list", r1(crate::commands::workspaces::workspace_list)));
    v.push(("workspace_get", r2(crate::commands::workspaces::workspace_get)));
    v.push(("workspace_create", r2(crate::commands::workspaces::workspace_create)));
    v.push(("workspace_update", r3(crate::commands::workspaces::workspace_update)));
    v.push(("workspace_delete", r5(crate::commands::workspaces::workspace_delete)));
    v.push(("workspace_stats", r2(crate::commands::workspaces::workspace_stats)));
    v.push(("profiles_list_by_workspace", r2(crate::commands::workspaces::profiles_list_by_workspace)));
    v.push(("workspace_column_list", r2(crate::commands::workspaces::workspace_column_list)));
    v.push(("workspace_column_create", r3(crate::commands::workspaces::workspace_column_create)));
    v.push(("workspace_column_update", r3(crate::commands::workspaces::workspace_column_update)));
    v.push(("workspace_column_delete", r2(crate::commands::workspaces::workspace_column_delete)));
    v.push(("profile_set_tags", r3(crate::commands::workspaces::profile_set_tags)));
    v.push(("profile_move_to_kanban_column", r4(crate::commands::workspaces::profile_move_to_kanban_column)));
    v.push(("ssh_key_list", r1(crate::commands::ssh_keys::ssh_key_list)));
    v.push(("ssh_key_get", r2(crate::commands::ssh_keys::ssh_key_get)));
    v.push(("ssh_key_export_private", r2(crate::commands::ssh_keys::ssh_key_export_private)));
    v.push(("ssh_key_import", r2(crate::commands::ssh_keys::ssh_key_import)));
    v.push(("ssh_key_generate", r2(crate::commands::ssh_keys::ssh_key_generate)));
    v.push(("ssh_key_update", r3(crate::commands::ssh_keys::ssh_key_update)));
    v.push(("ssh_key_delete", r2(crate::commands::ssh_keys::ssh_key_delete)));
    v.push(("sftp_transfer_start", r6(crate::commands::transfer::sftp_transfer_start)));
    v.push(("sftp_transfer_cancel", r2(crate::commands::transfer::sftp_transfer_cancel)));
    v.push(("backup_get_config", r1(crate::commands::backup::backup_get_config)));
    v.push(("backup_set_config", r2(crate::commands::backup::backup_set_config)));
    v.push(("backup_list", r1(crate::commands::backup::backup_list)));
    v.push(("backup_run_now", r1(crate::commands::backup::backup_run_now)));
    v.push(("backup_restore", r4(crate::commands::backup::backup_restore)));
    v.push(("proxies_list", r1(crate::commands::proxies::proxies_list)));
    v.push(("proxy_get", r2(crate::commands::proxies::proxy_get)));
    v.push(("proxy_export_url", r2(crate::commands::proxies::proxy_export_url)));
    v.push(("proxy_create", r2(crate::commands::proxies::proxy_create)));
    v.push(("proxies_bulk_create", r2(crate::commands::proxies::proxies_bulk_create)));
    v.push(("proxy_update", r3(crate::commands::proxies::proxy_update)));
    v.push(("proxy_usage", r2(crate::commands::proxies::proxy_usage)));
    v.push(("proxy_delete", r2(crate::commands::proxies::proxy_delete)));
    v.push(("proxy_check", r3(crate::commands::proxies::proxy_check)));
    v.push(("proxy_trust_fingerprint", r6(crate::commands::proxies::proxy_trust_fingerprint)));
    v.push(("camoufox_status", r1(crate::commands::camoufox::camoufox_status)));
    v.push(("camoufox_latest_version", r0(crate::commands::camoufox::camoufox_latest_version)));
    v.push(("camoufox_download_state", r1(crate::commands::camoufox::camoufox_download_state)));
    v.push(("camoufox_download_cancel", r1(crate::commands::camoufox::camoufox_download_cancel)));
    v.push(("camoufox_download", r3(crate::commands::camoufox::camoufox_download)));
    v.push(("ssh_connection_list", r4(crate::commands::ssh::ssh_connection_list)));
    v.push(("ssh_connection_get", r2(crate::commands::ssh::ssh_connection_get)));
    v.push(("ssh_connection_create", r2(crate::commands::ssh::ssh_connection_create)));
    v.push(("ssh_connection_update", r5(crate::commands::ssh::ssh_connection_update)));
    v.push(("ssh_connection_delete", r4(crate::commands::ssh::ssh_connection_delete)));
    v.push(("ssh_connection_trust_fingerprint", r5(crate::commands::ssh::ssh_connection_trust_fingerprint)));
    v.push(("ssh_session_list", r1(crate::commands::ssh::ssh_session_list)));
    v.push(("ssh_connect", r4(crate::commands::ssh::ssh_connect)));
    v.push(("ssh_disconnect", r2(crate::commands::ssh::ssh_disconnect)));
    v.push(("ssh_send_data", r3(crate::commands::ssh::ssh_send_data)));
    v.push(("ssh_resize", r4(crate::commands::ssh::ssh_resize)));
    v.push(("ssh_session_remove", r2(crate::commands::ssh::ssh_session_remove)));
    v.push(("ssh_respond_prompt", r3(crate::commands::ssh::ssh_respond_prompt)));
    v.push(("profiles_list", r1(crate::commands::profiles::crud::profiles_list)));
    v.push(("profile_get", r2(crate::commands::profiles::crud::profile_get)));
    v.push(("profile_create", r2(crate::commands::profiles::crud::profile_create)));
    v.push(("profile_update", r4(crate::commands::profiles::crud::profile_update)));
    v.push(("profile_delete", r4(crate::commands::profiles::crud::profile_delete)));
    v.push(("profile_clone", r3(crate::commands::profiles::crud::profile_clone)));
    v.push(("profile_raw_data", r2(crate::commands::profiles::crud::profile_raw_data)));
    v.push(("sync_profile_files_take_remote", r5(crate::commands::profiles::sync::sync_profile_files_take_remote)));
    v.push(("sync_profile_files_push_mine", r4(crate::commands::profiles::sync::sync_profile_files_push_mine)));
    v.push(("profile_export_json", r3(crate::commands::profiles::import_export::profile_export_json)));
    v.push(("profile_export_zip", r4(crate::commands::profiles::import_export::profile_export_zip)));
    v.push(("profile_import_json", r3(crate::commands::profiles::import_export::profile_import_json)));
    v.push(("profile_import_zip", r3(crate::commands::profiles::import_export::profile_import_zip)));
    v.push(("profile_import_zip_data", r3(crate::commands::profiles::import_export::profile_import_zip_data)));
    v.push(("profile_export_json_to_file", r4(crate::commands::profiles::import_export::profile_export_json_to_file)));
    v.push(("profile_launch", r5(crate::commands::profiles::launch::profile_launch)));
    v.push(("profile_stop", r4(crate::commands::profiles::launch::profile_stop)));
    v.push(("profile_is_running", r2(crate::commands::profiles::launch::profile_is_running)));
    v.push(("profiles_running_ids", r1(crate::commands::profiles::launch::profiles_running_ids)));
    v.push(("profiles_launching", r1(crate::commands::profiles::launch::profiles_launching)));
    v.push(("profile_import_cookies", r4(crate::commands::profiles::cookies::profile_import_cookies)));
    v.push(("profile_export_cookies", r2(crate::commands::profiles::cookies::profile_export_cookies)));
    v.push(("profile_export_cookies_to_file", r3(crate::commands::profiles::cookies::profile_export_cookies_to_file)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

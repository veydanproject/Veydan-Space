// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The messenger: the adapter between a product and the messenger crates.
//!
//! The messenger knows nothing of the product it runs in. What it takes from
//! the host goes through the services of the shell (`veydan_shell::Shell`:
//! the product, the unread count, the main window, picked files, the camera
//! and the microphone) and through the lock (`veydan_lock::Lock`: the box
//! `messenger` and its events); its data is the folder `messenger/` of the
//! data directory. The shell switches it on and off (`Module::start`,
//! `Module::stop`): off, the runtime is shut down and holds no connection.

mod commands;

pub use commands::MessengerState;

use std::path::PathBuf;
use tauri::{Manager, Runtime};
use veydan_core::{AppError, BoxFuture, Core};
use veydan_shell::{BackupPart, BackupPath, Module, SetupResult};

pub fn module() -> Module {
    Module {
        start: Some(start),
        stop: Some(stop),
        backup: Some(backup()),
        ..veydan_shell::module! {
            id: "messenger",
            setup: setup,
            commands: [
                commands::messenger_status,
                commands::messenger_set_enabled,
                commands::messenger_identity_get,
                commands::messenger_identity_create,
                commands::messenger_identity_import,
                commands::messenger_identity_export,
                commands::messenger_identity_delete,
                commands::messenger_relays_list,
                commands::messenger_relays_add,
                commands::messenger_relays_remove,
                commands::messenger_relays_set_enabled,
                commands::messenger_relays_set_silent,
                commands::messenger_manifest_info,
                commands::messenger_manifest_set_region,
                commands::messenger_servers_use_veydan,
                commands::messenger_servers_use_own,
                commands::messenger_manifest_refresh,
                commands::messenger_net_status,
                commands::messenger_net_set_mode,
                commands::messenger_net_check,
                commands::messenger_net_bridge_add,
                commands::messenger_net_bridge_remove,
                commands::messenger_net_offer_dismiss,
                commands::push::messenger_push_status,
                commands::push::messenger_push_set_enabled,
                commands::push::messenger_push_mark_offered,
                commands::push::messenger_push_set_server,
                commands::push::messenger_push_set_prefs,
                commands::push::messenger_push_refresh,
                commands::push::messenger_push_test,
                commands::push::messenger_push_take_tap,
                commands::push::messenger_push_clear,
                #[cfg(desktop)]
                commands::desktop_notify::messenger_desktop_notify_get,
                #[cfg(desktop)]
                commands::desktop_notify::messenger_desktop_notify_set,
                #[cfg(desktop)]
                commands::desktop_notify::messenger_desktop_notify_test,
                #[cfg(desktop)]
                commands::desktop_notify::messenger_notice_words,
                #[cfg(desktop)]
                commands::desktop_notify::messenger_desktop_notify_keep_running,
                commands::push::messenger_notify_get,
                commands::push::messenger_notify_set,
                commands::messenger_privacy_get,
                commands::messenger_privacy_set,
                commands::messenger_presence_list,
                commands::messenger_presence_foreground,
                commands::messenger_dm_send_text,
                commands::messenger_chats_list,
                commands::messenger_chat_open,
                commands::messenger_chat_messages,
                commands::messenger_chat_shared_counts,
                commands::messenger_chat_shared,
                commands::messenger_chat_mark_read,
                commands::messenger_chat_set_pinned,
                commands::messenger_chat_set_archived,
                commands::messenger_chat_set_muted,
                commands::messenger_chat_delete,
                commands::messenger_dm_edit,
                commands::messenger_dm_react,
                commands::messenger_emoji_used,
                commands::messenger_emoji_top,
                commands::messenger_dm_delete,
                commands::messenger_dm_retry,
                commands::messenger_dm_relation,
                commands::messenger_dm_action,
                commands::messenger_dm_blocked,
                commands::messenger_groups_list,
                commands::messenger_group_get,
                commands::messenger_group_create,
                commands::messenger_group_invite,
                commands::messenger_group_invites,
                commands::messenger_group_answer_invite,
                commands::messenger_group_open_link,
                commands::messenger_group_answer_request,
                commands::messenger_group_act,
                commands::messenger_group_rotate_link,
                commands::messenger_group_link_qr,
                commands::messenger_links_inspect,
                commands::messenger_contact_link,
                commands::messenger_link_preview,
                commands::messenger_group_forget,
                commands::messenger_media_servers,
                commands::messenger_media_server_put,
                commands::messenger_media_server_remove,
                commands::messenger_media_server_set_enabled,
                commands::messenger_media_server_check,
                commands::messenger_dm_send_file,
                commands::messenger_media_import,
                commands::messenger_media_download,
                commands::messenger_media_transfer,
                commands::messenger_media_pause,
                commands::messenger_media_resume,
                commands::messenger_media_cancel,
                commands::messenger_media_save_as,
                commands::messenger_media_data_url,
                commands::messenger_media_local_path,
                commands::messenger_open_url,
                commands::messenger_media_open,
                commands::messenger_dm_send_recording,
                commands::messenger_media_grant_access,
                commands::messenger_profile_get,
                commands::messenger_profile_request,
                commands::messenger_profile_own_get,
                commands::messenger_profile_own_set,
                commands::messenger_nip05_verify,
                commands::messenger_contacts_list,
                commands::messenger_contacts_add,
                commands::messenger_contacts_update,
                commands::messenger_contacts_remove,
                commands::messenger_contacts_set_followed,
            ],
        }
    }
}

fn setup(app: &mut tauri::App) -> SetupResult {
    // Two rustls providers are linked (host: aws-lc-rs, nostr-sdk: ring).
    // Pick the host's one for the whole process before the runtime makes a
    // TLS handshake; nothing that runs before the setup of the messenger
    // does. A second call (already installed) is not an error worth reporting.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    // Push notifications of the messenger; see commands/push.rs. The
    // messenger's start already talks to the plugin.
    #[cfg(target_os = "android")]
    app.handle().plugin(tauri_plugin_veydan_push::init())?;
    let core = app.state::<Core>();
    // The switch is the shell's now: the old setting goes.
    tauri::async_runtime::block_on(veydan_core::settings::delete(
        &core.db,
        commands::OLD_ENABLED_KEY,
    ))?;
    app.manage(MessengerState::new(app.handle().clone(), &core.app_data_dir));
    Ok(())
}

/// The module is switched on, or the app started with it on: the runtime
/// starts. A runtime that fails to start leaves the app running.
fn start(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move { app.state::<MessengerState>().start().await })
}

/// The module is switched off, or the app quits: the runtime stops, and the
/// notifications of the messenger go with it.
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        app.state::<MessengerState>().stop().await;
        Ok(())
    })
}

/// What a backup takes of the messenger, on any runtime; `module()` holds
/// it on the app's: its folder as a whole — the database, media and avatars
/// of `messenger/`.
pub fn backup<R: Runtime>() -> BackupPart<R> {
    BackupPart {
        paths: |_app| vec![messenger_dir()],
    }
}

/// The folder of the messenger, relative to the data directory.
fn messenger_dir() -> BackupPath {
    BackupPath {
        name: "messenger",
        path: PathBuf::from(commands::DATA_SUBDIR),
        external: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The messenger keeps its data in one folder of the data directory, and
    /// a backup takes that folder whole.
    #[test]
    fn a_backup_takes_the_folder_of_the_messenger_as_a_whole() {
        let module = module();
        assert_eq!(module.id, "messenger");
        assert_eq!(module.id, commands::MODULE_ID);
        assert!(module.backup.is_some());
        assert_eq!(
            messenger_dir(),
            BackupPath {
                name: "messenger",
                path: PathBuf::from("messenger"),
                external: false,
            }
        );
    }
}

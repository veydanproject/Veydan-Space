// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The module Communities: the channels of a community in a product
//! (internal/communities-stage1.md §3.1, §6), as Notes and Pass are
//! modules of the platform. The engine is `communities-core`, free of
//! Tauri; this crate is its adapter: the commands `channel_*`, the events
//! of the page (`communities://event`), the loop that listens to the
//! relays of the channels and runs the keepers kept on this device, and
//! what the channels show outside their pages.
//!
//! It speaks the network of the messenger as the same person, through the
//! seams of the shell alone (`net_lend` of the messenger: its relays and
//! the identity at work); products list it after the messenger. Its data
//! is the folder `communities/` of the data directory, a database of the
//! core for each identity (`<npub>.db`); its secrets are in the lock's box
//! `communities`. Space gives it a switch of its own, Chat has it on with
//! the messenger (`Switch::Follows`). Off, it registers nothing, runs
//! nothing and keeps its database closed.

mod background;
mod log;
pub mod bindings;
mod commands;
mod seams;
mod state;
pub mod views;
mod wake;

pub use state::{CommunitiesState, EVENT, UI_EVENT_CHANNEL_KEEPER, UI_EVENT_CHANNEL_READY};

use std::path::PathBuf;
use tauri::Manager;
use veydan_core::{AppError, BoxFuture, Core};
use veydan_shell::{BackupPart, BackupPath, Module, SetupResult};

/// The id of the module, as the products list it.
pub const MODULE_ID: &str = "communities";

/// The module with a switch of its own (Space). A product that has it on
/// with the messenger sets `switch: Switch::Follows("messenger")`.
///
/// No `key_user`: nothing of the module is kept under the vault key in
/// tables of its own. Its secrets (keeper keys, root secrets, masters,
/// link secrets) live in the lock's box `communities`, which the lock
/// keeps and moves to a new key itself; its databases hold the public
/// and the sealed only.
pub fn module() -> Module {
    Module {
        start: Some(start),
        stop: Some(stop),
        backup: Some(backup()),
        ..veydan_shell::module! {
            // A literal: scripts/boundaries.mjs reads the list by it.
            id: "communities",
            setup: setup,
            commands: [
                commands::channels_list,
                commands::channel_get,
                commands::channel_create,
                commands::channel_storage_plan,
                commands::channel_invite,
                commands::channel_invites,
                commands::channel_answer_invite,
                commands::channel_open_link,
                commands::channel_preview_link,
                commands::channel_answer_request,
                commands::channel_act,
                commands::channel_avatar_set,
                commands::channel_avatar_remove,
                commands::channel_rotate_link,
                commands::channel_link_qr,
                commands::channel_forget,
                commands::channel_outbox_retry,
                commands::channel_history,
                commands::channel_thread,
                commands::channel_post,
                commands::channel_comment,
                commands::channel_react,
                commands::channel_edit,
                commands::channel_md_preview,
                commands::channel_delete,
                commands::channel_pin,
                commands::channel_read,
                commands::channel_media_send,
                commands::channel_media_record,
                commands::channel_media_fetch,
                commands::channel_media_url,
                commands::channel_media_play,
                commands::channel_media_import,
                commands::channel_cover,
                commands::channel_picked_preview,
                commands::channel_media_cancel,
                commands::channel_media_retry,
                commands::channel_media_pause,
                commands::channel_media_fetch_cancel,
                commands::channel_media_wake,
                commands::channel_uploads,
                commands::moderation::channel_moderation_queue,
                commands::moderation::channel_moderation_mod,
                commands::moderation::channel_moderation_log,
                commands::moderation::channel_moderation_settings,
                commands::moderation::channel_moderation_settings_set,
                commands::moderation::channel_moderation_inherit,
                commands::moderation::channel_moderation_invite_create,
                commands::moderation::channel_moderation_invite_list,
                commands::moderation::channel_moderation_invite_revoke,
                commands::moderation::channel_moderation_invite_status,
                commands::moderation::channel_moderation_blocked,
                commands::moderation::channel_moderation_requests,
                commands::moderation::channel_moderation_request_act,
                commands::moderation::channel_moderation_requests_count,
                commands::moderation::channel_keeper_status,
                commands::moderation::channel_keeper_keep_on,
                commands::moderation::channel_keeper_take,
                commands::moderation::channel_transfer,
                commands::moderation::channel_transfer_answer,
                commands::moderation::channel_tickets,
                commands::moderation::channel_tickets_request,
                commands::moderation::channel_rotate_now,
                commands::levels::channel_scopes,
                commands::levels::channel_scope_add,
                commands::levels::channel_scope_del,
                commands::levels::channel_scope_set,
                commands::levels::channel_scopes_order,
                commands::levels::channel_scope_photo_set,
                commands::levels::channel_scope_photo,
                commands::levels::channel_levels,
                commands::levels::channel_level_set,
                commands::levels::channel_level_del,
                commands::levels::channel_set_levels,
                commands::levels::channel_level_request,
                commands::levels::channel_level_requests,
                commands::levels::channel_level_request_act,
            ],
        }
    }
}

fn setup(app: &mut tauri::App) -> SetupResult {
    let core = app.state::<Core>();
    app.manage(CommunitiesState::new(app.handle().clone(), &core.app_data_dir));
    Ok(())
}

/// The module is switched on, or the app started with it on: the database
/// opens and the loop starts. A start that fails leaves the app running;
/// the commands then say `communities_stopped`.
fn start(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        if let Err(e) = app.state::<CommunitiesState>().start().await {
            crate::log::clog!("start failed: {e}");
        }
        Ok(())
    })
}

/// The module is switched off, or the app quits: the loop ends, the routes
/// of the channels go back to the messenger, the database closes.
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        app.state::<CommunitiesState>().stop().await;
        Ok(())
    })
}

/// What a backup takes of the module: its folder as a whole, the databases;
/// not the files it fetched or the copies of those it sends, which live
/// beside it (`communities-files/`).
pub fn backup<R: tauri::Runtime>() -> BackupPart<R> {
    BackupPart { paths: |_app| vec![folder()] }
}

fn folder() -> BackupPath {
    BackupPath { name: MODULE_ID, path: PathBuf::from(state::DATA_SUBDIR), external: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names of the commands are those of internal/communities-stage1.md
    /// §6.1, and every one of them is the module's (`channel…`).
    #[test]
    fn the_commands_are_those_of_the_stage() {
        let module = module();
        assert_eq!(module.id, MODULE_ID);
        for name in [
            "channels_list", "channel_get", "channel_create", "channel_storage_plan", "channel_invite", "channel_invites", "channel_answer_invite",
            "channel_open_link", "channel_preview_link", "channel_answer_request", "channel_act", "channel_avatar_set", "channel_avatar_remove",
            "channel_rotate_link", "channel_link_qr", "channel_forget", "channel_outbox_retry", "channel_history", "channel_thread", "channel_post",
            "channel_comment", "channel_react", "channel_edit", "channel_md_preview", "channel_delete", "channel_pin", "channel_read",
            "channel_media_send", "channel_media_record", "channel_media_fetch", "channel_cover", "channel_picked_preview", "channel_media_url", "channel_media_play", "channel_media_import", "channel_media_cancel", "channel_media_retry", "channel_media_pause", "channel_media_fetch_cancel", "channel_media_wake", "channel_uploads",
            "channel_moderation_queue", "channel_moderation_mod", "channel_moderation_log", "channel_moderation_settings",
            "channel_moderation_settings_set", "channel_moderation_inherit", "channel_moderation_invite_create",
            "channel_moderation_invite_list", "channel_moderation_invite_revoke", "channel_moderation_invite_status",
            "channel_moderation_blocked", "channel_moderation_requests", "channel_moderation_request_act",
            "channel_moderation_requests_count", "channel_keeper_status", "channel_keeper_keep_on", "channel_keeper_take", "channel_transfer", "channel_transfer_answer", "channel_tickets", "channel_tickets_request", "channel_rotate_now",
            "channel_scopes", "channel_scope_add", "channel_scope_del", "channel_scope_set", "channel_scopes_order", "channel_scope_photo_set", "channel_scope_photo",
            "channel_levels", "channel_level_set", "channel_level_del",
            "channel_set_levels", "channel_level_request", "channel_level_requests", "channel_level_request_act",
        ] {
            assert!(module.commands.contains(&name), "{name}");
        }
        assert!(module.commands.iter().all(|c| c.starts_with("channel")));
        assert!(module.key_user.is_none() && module.schema.is_none());
        assert_eq!(module.switch, veydan_shell::Switch::Own);
    }

    /// The module keeps its data in one folder, and a backup takes it
    /// whole; the files fetched and the copies to send are not in it
    /// (ревью модуля п. 8).
    #[test]
    fn a_backup_takes_the_folder_of_the_module() {
        assert!(module().backup.is_some());
        assert_eq!(folder(), BackupPath { name: "communities", path: PathBuf::from("communities"), external: false });
        let files = PathBuf::from(state::FILES_SUBDIR);
        assert!(!files.starts_with(&folder().path) && files.components().count() == 1);
    }

    /// A refusal of the core reaches the page under a stable code.
    #[test]
    fn a_refusal_of_the_core_has_its_code() {
        use messenger_core::MessengerError;
        let text = |e: MessengerError| state::map_err(e).to_string();
        assert!(text(MessengerError::Invalid("channel: not permitted".into())).contains("channel_not_permitted: channel: not permitted"));
        assert!(text(MessengerError::Invalid("channel: unknown channel".into())).contains("channel_unknown:"));
        assert!(text(MessengerError::Invalid("channel link: owner".into())).contains("channel_bad_link:"));
        assert!(text(MessengerError::Invalid("channel: Rejected(NotPermitted)".into())).contains("channel_invalid:"));
        // The levels and the sections (wire §14), by the core or the journal.
        for (core, code) in [
            ("channel: level name", "channel_level_name:"),
            ("channel: level last", "channel_level_last:"),
            ("channel: level limit", "channel_level_limit:"),
            ("channel: scope everyone", "channel_scope_everyone:"),
            ("channel: scope limit", "channel_scope_limit:"),
            ("channel: Invalid { what: \"scope limit\" }", "channel_scope_limit:"),
            ("channel: not member of scope", "channel_not_member_of_scope:"),
            ("channel: no post in this scope", "channel_scope_no_post:"),
            // Stage 3 (wire §15–§17).
            ("channel: no transfer offer", "channel_no_transfer:"),
            ("channel: no tickets in an open channel", "channel_tickets_open:"),
            ("channel: transfer to oneself", "channel_self:"),
            ("channel: this device keeps the channel", "channel_kept_here:"),
        ] {
            assert!(text(MessengerError::Invalid(core.into())).contains(code), "{core}");
        }
        assert!(text(MessengerError::Transport("err.not_delivered".into())).contains("channel_not_delivered:"));
        // A command on its way when the module stopped (ревью модуля п. 16).
        let closed = MessengerError::Storage("attempted to acquire a connection on a closed pool".into());
        assert!(text(closed).contains("communities_stopped:"));
        assert!(matches!(state::map_err(MessengerError::Storage("disk full".into())), AppError::Db(_)));
    }
}

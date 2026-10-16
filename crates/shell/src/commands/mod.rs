// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands every product has, the lock's (`crate::lock`) and sync's
//! (`veydan_sync_host::commands`) among them. The shell is a module too:
//! they go through the same router as the commands of the product's modules.

pub mod host;
pub mod labels;
pub mod media;
// The tray and its settings exist on a computer only.
#[cfg(desktop)]
pub mod settings;

use crate::start::commands as start;
use crate::{Grant, Module, Switch, WindowGrant, ANY_WINDOW};

pub(crate) fn module() -> Module {
    Module {
        #[cfg(desktop)]
        sync: Some(sync),
        switch: Switch::Always,
        windows: &[WindowGrant {
            label: ANY_WINDOW,
            commands: Grant::Only(SECONDARY),
        }],
        ..commands()
    }
}

/// What every window besides `main` reaches of the shell (platform-spec
/// 5.2): the start of the page, the lock screen, its own frame, links and
/// the clipboard, names of entities, the status of sync, the camera. A
/// module's own window gets the rest it needs from the module's grants.
/// Not here, so main alone reaches them: setting the lock, the data's
/// deletion and the demo, sync's configuration, the updater, the switches of
/// the modules, the tray and the language.
pub(crate) const SECONDARY: &[&str] = &[
    "app_start_error",
    "host_info",
    "modules_list",
    "lock_status",
    "lock_unlock",
    "lock_touch",
    "lock_recovery_check",
    "lock_recover",
    "lock_recover_keep",
    #[cfg(desktop)]
    "window_minimize",
    "open_url",
    "labels_resolve",
    #[cfg(desktop)]
    "clipboard_write_text",
    "sync_status",
    "media_grant_access",
];

/// The settings of the tray are shared between desktops.
#[cfg(desktop)]
fn sync(registry: &mut veydan_sync_host::Registry) {
    for key in crate::services::TRAY_KEYS {
        registry.setting(key);
    }
}

fn commands() -> Module {
    crate::module! {
        id: "shell",
        setup: |_app| Ok(()),
        commands: [
            start::app_start_error,
            host::host_info,
            host::open_url,
            #[cfg(desktop)]
            host::clipboard_write_text,
            #[cfg(desktop)]
            host::update_supported,
            #[cfg(desktop)]
            settings::tray_settings_get,
            #[cfg(desktop)]
            settings::tray_settings_set,
            #[cfg(desktop)]
            settings::tray_set_labels,
            #[cfg(desktop)]
            settings::app_locale_set,
            #[cfg(desktop)]
            settings::app_locale_get,
            #[cfg(desktop)]
            settings::window_minimize,
            media::media_grant_access,
            labels::labels_resolve,
            crate::update::update_check,
            crate::update::update_open,
            #[cfg(desktop)]
            crate::updater::updater_check,
            #[cfg(desktop)]
            crate::updater::updater_install,
            #[cfg(desktop)]
            crate::updater::app_relaunch,
            crate::switches::modules_list,
            crate::switches::modules_set_enabled,
            crate::switches::modules_choose,
            crate::switches::modules_first_run_done,
            crate::lock::lock_status,
            crate::lock::lock_set,
            crate::lock::lock_timeout_set,
            crate::lock::lock_unlock,
            crate::lock::lock_lock,
            crate::lock::lock_touch,
            crate::lock::lock_recovery_regenerate,
            crate::lock::lock_recovery_check,
            crate::lock::lock_recover,
            crate::lock::lock_recover_keep,
            crate::lock::vault_replaced_keys,
            crate::lock::vault_replaced_keys_open,
            crate::demo::demo_seed,
            crate::demo::app_clear_data,
            veydan_sync_host::commands::sync_get_config,
            veydan_sync_host::commands::sync_set_config,
            veydan_sync_host::commands::sync_probe,
            veydan_sync_host::commands::sync_create_vault,
            veydan_sync_host::commands::sync_join_vault,
            veydan_sync_host::commands::sync_leave,
            veydan_sync_host::commands::sync_change_storage,
            veydan_sync_host::commands::sync_change_passphrase,
            veydan_sync_host::commands::sync_status,
            veydan_sync_host::commands::sync_debug_get,
            veydan_sync_host::commands::sync_debug_set,
            veydan_sync_host::commands::sync_debug_clear,
            veydan_sync_host::commands::sync_run_now,
            veydan_sync_host::commands::sync_trigger,
        ],
    }
}

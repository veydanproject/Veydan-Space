// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The module Tor: the `tor` daemon of the official Tor Expert Bundle, for
//! the products that send traffic through Tor.
//!
//! Where the files of tor come from (`source`) is kept apart from how tor
//! runs: on a computer the app downloads the bundle from torproject.org,
//! with its version and checksums pinned here (`install`), or the user puts
//! the files in place by hand; on Android the binary will ship in the APK.
//! How tor runs: its settings (`settings`), the torrc of an instance
//! (`torrc`), the control port (`control`) and the instances themselves
//! (`manager`), which other crates use through [`TorManager::acquire`].
//! The module keeps no tables and knows no other module.

#[cfg(desktop)]
mod commands;
pub mod control;
#[cfg(desktop)]
pub mod install;
pub mod manager;
pub mod settings;
pub mod source;
pub mod torrc;

pub use manager::{ExitSet, InstanceInfo, InstanceState, Lease, TorError, TorManager};
pub use settings::{TorSettings, SETTINGS_KEY};
pub use source::{bundle_dir, resolve, Bundle};

use tauri::{Emitter, Manager};
use veydan_core::{AppError, BoxFuture, Core};
use veydan_shell::{Module, SetupResult, Switch};

/// The event with the list of instances, sent whenever any of it changes.
pub const EVENT_INSTANCES: &str = "tor://instances";

pub fn module() -> Module {
    Module {
        // A service of the product: never off, no switch in the settings.
        switch: Switch::Always,
        start: Some(start),
        stop: Some(stop),
        ..veydan_shell::module! {
            id: "tor",
            setup: setup,
            commands: [
                #[cfg(desktop)]
                install::tor_status,
                #[cfg(desktop)]
                install::tor_download,
                #[cfg(desktop)]
                install::tor_download_cancel,
                #[cfg(desktop)]
                install::tor_download_state,
                #[cfg(desktop)]
                install::tor_install_from_archive,
                #[cfg(desktop)]
                install::tor_remove,
                #[cfg(desktop)]
                commands::tor_get_settings,
                #[cfg(desktop)]
                commands::tor_set_settings,
                #[cfg(desktop)]
                commands::tor_instances,
                #[cfg(desktop)]
                commands::tor_start,
                #[cfg(desktop)]
                commands::tor_stop,
                #[cfg(desktop)]
                commands::tor_new_identity,
                #[cfg(desktop)]
                commands::tor_log,
                #[cfg(desktop)]
                commands::tor_builtin_bridges,
            ],
        }
    }
}

/// The manager of the instances with the stored settings, and the state of
/// the installer.
fn setup(app: &mut tauri::App) -> SetupResult {
    let core = app.state::<Core>();
    let stored = tauri::async_runtime::block_on(veydan_core::settings::get(&core.db, SETTINGS_KEY));
    let settings = TorSettings::from_stored(stored.as_deref());
    let handle = app.handle().clone();
    let manager = TorManager::new(core.app_data_dir.clone(), settings, move |list| {
        handle.emit(EVENT_INSTANCES, list).ok();
    });
    app.manage(manager);
    #[cfg(desktop)]
    app.manage(install::Installer::default());
    Ok(())
}

/// The instance with no exit countries, when the settings start it with
/// the app.
fn start(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        let manager = app.state::<TorManager>().inner().clone();
        manager.start_with_app().await;
        Ok(())
    })
}

/// The module is never switched off, so this runs when the app exits and
/// before a backup is restored: every tor stops.
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        let manager = app.state::<TorManager>().inner().clone();
        manager.stop_all().await;
        Ok(())
    })
}

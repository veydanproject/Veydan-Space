// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The shell: what starts a product assembled from modules.
//!
//! A product crate holds the Tauri config, a [`Product`] and the list of its
//! modules, and calls [`run`]. The shell builds the app, opens the data file
//! with the schemas of the lock and of the modules, puts the lock in place
//! before the modules start, routes every command to the module that
//! declared it, draws the tray around the modules' entries, puts the demo
//! data together from their parts, starts the modules the user keeps on and
//! stops them when they are switched off and when the app exits. Modules
//! offer each other services through it (`seams`).
//! It knows no module by name.

#[cfg(test)]
mod command_futures;
mod app;
mod commands;
pub mod demo;
pub mod links;
mod distribution;
mod lock;
#[cfg(target_os = "android")]
mod logcat;
mod module;
mod navigation;
mod plan;
mod router;
pub mod seams;
mod services;
mod source;
mod start;
mod switches;
mod update;
#[cfg(desktop)]
mod tray;
#[cfg(desktop)]
mod updater;
#[cfg(desktop)]
pub mod workdir;

pub use app::{command_table, run, schemas, sync_registry, window_table};
pub use lock::{
    LockSetResult, EVENT_LOCKED, EVENT_UNLOCKED, EVENT_VAULT_CHANGED, TIMEOUT_KEY as LOCK_TIMEOUT_KEY,
};
pub use commands::labels::{resolve as resolve_labels, LabelName, LabelRef};
#[cfg(desktop)]
pub use commands::settings::app_locale;
pub use plan::default_plan;
pub use navigation::guard_window;
pub use module::{
    tray_label, BackupPart, BackupPath, DemoPart, Grant, Handler, Hook, Module, Setup, SetupResult,
    Switch, TrayGroup, TrayItem, TrayLabels, TrayPart, WindowGrant, ANY_WINDOW, MAIN_WINDOW,
};
pub use router::{
    unknown_command, window_denied, DuplicateCommand, Router, RouterError, UnknownGrant,
};
pub use seams::{provide, seam, CloudError, Seam};
pub use services::{Product, Shell};
pub use source::{FileSource, Opener};
pub use start::{StartError, StartState};
pub use switches::{ModuleState, ModulesView, EVENT_MODULES_CHANGED};
pub use update::{Skipped as UpdateSkipped, UpdateCheck};
#[cfg(desktop)]
pub use tray::TraySettings;

// A product names the plan of its sync through the shell.
pub use veydan_sync_host::{Plan, Step};
// The module cloud registers the backend `cloud` of sync and keeps its hosts
// through the shell: it does not depend on sync-host.
pub use veydan_sync_host::config as sync_config;

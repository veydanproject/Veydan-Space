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
//! stops them when they are switched off and when the app exits.
//! It knows no module by name.

mod app;
mod commands;
pub mod demo;
mod lock;
mod module;
mod plan;
mod router;
mod services;
mod source;
mod start;
mod switches;
mod update;
#[cfg(desktop)]
mod tray;
#[cfg(desktop)]
pub mod workdir;

pub use app::{command_table, run, schemas, sync_registry};
pub use lock::{
    LockSetResult, EVENT_LOCKED, EVENT_UNLOCKED, EVENT_VAULT_CHANGED, TIMEOUT_KEY as LOCK_TIMEOUT_KEY,
};
pub use commands::labels::{resolve as resolve_labels, LabelName, LabelRef};
#[cfg(desktop)]
pub use commands::settings::app_locale;
pub use plan::default_plan;
pub use module::{
    tray_label, BackupPart, BackupPath, DemoPart, Handler, Hook, Module, Setup, SetupResult, Switch,
    TrayGroup, TrayItem, TrayLabels, TrayPart,
};
pub use router::{unknown_command, DuplicateCommand, Router};
pub use services::{Product, Shell};
pub use source::{FileSource, Opener};
pub use start::{StartError, StartState};
pub use switches::{ModuleState, ModulesView, EVENT_MODULES_CHANGED};
pub use update::{Skipped as UpdateSkipped, UpdateCheck};
#[cfg(desktop)]
pub use tray::TraySettings;

// A product names the plan of its sync through the shell.
pub use veydan_sync_host::{Plan, Step};

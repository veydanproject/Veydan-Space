// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Veydan Space: the product crate. It holds the Tauri config, the strings of
//! the product, the plan of its sync and the list of its modules — the
//! crates of notes, pass and the messenger, the crate of the service tor,
//! and the modules that live here (`modules`): browser and ssh, the
//! services backup and capture; `veydan_shell` starts them. What each module
//! keeps in a backup is its `Module::backup`: browser the profiles, notes
//! its folders, the messenger its folder.
//!
//! One library for every platform. Browser profiles, proxies, SSH/SFTP,
//! backups, the capture bridge and tor are `cfg(desktop)`; on a phone
//! browser and ssh keep only their tables.

#[cfg(desktop)]
mod browser;
#[cfg(desktop)]
pub mod capture;
// Public so integration smoke examples (examples/*.rs) can exercise command internals.
pub mod commands;
mod db;
pub use veydan_core::error;
#[cfg(desktop)]
mod fingerprint;
#[cfg(desktop)]
mod models;
mod modules;
#[cfg(desktop)]
mod proxy;
mod sync;

use veydan_shell::{Module, Plan, Step};

/// What Space is (internal/platform-spec.md 13.1, 13.5). Its identifiers are in
/// the Tauri configs: `net.veydan.space` on a computer (`tauri.conf.json`),
/// `net.veydan.mobile` on Android (`tauri.android.conf.json`); the keys of
/// the messenger and the app of Firebase are bound to them. The version is
/// the package's (`VERSION`, written by `scripts/set-version.sh space`).
const PRODUCT: veydan_shell::Product = veydan_shell::Product {
    id: "space",
    name: "Veydan Space",
    desktop_entry: "veydanspace",
    icon: "veydanspace",
    sync: &SYNC_PLAN,
};

/// The modules in the order the shell sets them up, stops them and lists
/// their tray entries inside a group. A setup puts the state of its module
/// in place, so a module comes after those whose state it reaches once it
/// has started: `capture` after `notes`, `backup` after `notes`; the lock
/// and sync are the shell's, in place before any of them, and the cycle of
/// sync starts once all are set up. `browser`, `ssh`, `notes` is the order
/// of their pages in the tray; `pass` before `notes` puts the password
/// generator above the quick note; `browser` before `messenger` stops the
/// browsers before the messenger takes its notices back.
pub(crate) fn module_list() -> Vec<Module> {
    let mut list = vec![
        veydan_pass::module(),
        modules::browser::module(),
        modules::ssh::module(),
        veydan_notes::module(),
    ];
    #[cfg(desktop)]
    list.extend([
        modules::capture::module(),
        modules::backup::module(),
        veydan_tor::module(),
    ]);
    list.push(veydan_messenger_app::module());
    list
}

/// The rows of notes, collected in one step and applied around the files of
/// the notes: tags and folders before them, the flags of a note after it.
const NOTE_ROWS: &[&str] = &["note_tag", "note_folder", "note_smart_view", "note_meta"];
const APP_ROWS: &[&str] = &[
    "workspace",
    "workspace_column",
    "proxy",
    "ssh_key",
    "profile",
    "ssh_connection",
    "totp",
    "password_vault",
    "password",
    "pw_history",
    "setting",
];

/// The cycle of Space, step for step as at platform-stage-3: the order of the
/// ops of a push and of their clock is what other devices read, and the
/// apply leans on it — parents before children, puts before tombstones, a
/// note before its flags. Rows of every module share one step, in the order
/// 4.0.7 kept them in. A phone has no profile files. The labels, which 4.0.7
/// does not know, come last each way: what came before keeps its order and
/// its clock, and the rows they name are applied first.
pub(crate) const SYNC_PLAN: Plan = Plan {
    collect: &[
        Step::rows("note rows", Some(8), "notes", NOTE_ROWS),
        Step::handler("attachments", Some(12), "attachments"),
        Step::handler("notes", Some(16), "notes"),
        Step::rows("app rows", Some(18), "app", APP_ROWS),
        Step::handler("leases", None, "profile files"),
        Step::rows("labels", None, "app", &["label"]),
    ],
    apply: &[
        Step::handler("attachments", Some(40), "attachments"),
        Step::rows(
            "notes catalog",
            Some(44),
            "notes",
            &["note_tag", "note_folder", "note_smart_view"],
        ),
        Step::handler_then("notes", Some(48), "notes", "notes index"),
        Step::rows("notes meta", Some(52), "notes", &["note_meta"]),
        Step::rows("app rows", Some(54), "app", APP_ROWS),
        Step::rows("labels", None, "app", &["label"]),
    ],
    late: &["profile files"],
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Started by the browser as the extension's native messaging host: no UI, just relay.
    #[cfg(desktop)]
    if capture::is_host_invocation() {
        capture::host::run();
        return;
    }
    veydan_shell::run(tauri::generate_context!(), PRODUCT, module_list());
}

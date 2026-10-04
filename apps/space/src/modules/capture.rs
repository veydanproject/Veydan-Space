// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The bridge of the web clipper: a local socket the browser extension's
//! native host talks to. What it brings, notes file
//! (`veydan_notes::capture::handle_capture`) by the rules capture keeps. It
//! has no switch of its own: it runs while the notes are on.

use std::sync::Mutex;
use tauri::Manager;
use veydan_core::{AppError, BoxFuture, Core};
use veydan_shell::{Module, SetupResult, Switch};

pub(crate) fn module() -> Module {
    Module {
        switch: Switch::Follows("notes"),
        start: Some(start),
        stop: Some(stop),
        sync: Some(sync),
        demo: Some(crate::commands::demo::capture()),
        ..veydan_shell::module! {
            id: "capture",
            setup: setup,
            commands: [
                crate::capture::rules::notes_capture_rules_get,
                crate::capture::rules::notes_capture_rules_set,
            ],
        }
    }
}

/// The rules of the clipper are shared between desktops.
fn sync(registry: &mut veydan_sync_host::Registry) {
    registry.setting(crate::capture::rules::KEY);
}

/// The server of the local socket while it runs.
#[derive(Default)]
struct Bridge(Mutex<Option<crate::capture::server::Server>>);

/// The native messaging manifest; the socket opens with `start`.
fn setup(app: &mut tauri::App) -> SetupResult {
    app.manage(Bridge::default());
    let data_dir = &app.state::<Core>().app_data_dir;
    if let Err(e) = crate::capture::extension::register_native_host(data_dir) {
        eprintln!("capture: native host registration failed: {e}");
    }
    Ok(())
}

/// The notes are on: the local socket opens, unless it is open.
fn start(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        let bridge = app.state::<Bridge>();
        let mut server = bridge
            .0
            .lock()
            .map_err(|e| AppError::other(e.to_string()))?;
        if server.is_none() {
            *server = Some(crate::capture::server::start(app.clone()));
        }
        Ok(())
    })
}

/// The notes are off, or the app quits: the local socket closes.
fn stop(app: tauri::AppHandle) -> BoxFuture<'static, Result<(), AppError>> {
    Box::pin(async move {
        let server = app
            .state::<Bridge>()
            .0
            .lock()
            .ok()
            .and_then(|mut s| s.take());
        if let Some(server) = server {
            crate::capture::server::stop(server);
        }
        Ok(())
    })
}

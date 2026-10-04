// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Backups of the whole app and their restore.

use tauri::Manager;
use veydan_shell::{Module, SetupResult, Switch};

pub(crate) fn module() -> Module {
    Module {
        // A service of the product: never off.
        switch: Switch::Always,
        before_open: Some(crate::commands::backup::finish_interrupted_restore),
        ..veydan_shell::module! {
            id: "backup",
            setup: setup,
            commands: [
                crate::commands::backup::backup_get_config,
                crate::commands::backup::backup_set_config,
                crate::commands::backup::backup_list,
                crate::commands::backup::backup_run_now,
                crate::commands::backup::backup_restore,
            ],
        }
    }
}

/// Scheduled backups: ticks every 60s, catches up missed runs on start.
fn setup(app: &mut tauri::App) -> SetupResult {
    app.manage(crate::commands::backup::BackupManager::default());
    crate::commands::backup::start_backup_scheduler(app.handle().clone());
    Ok(())
}

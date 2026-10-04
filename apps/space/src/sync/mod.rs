// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What of the sync of Space is still this crate's: the files of browser
//! profiles through a handler of their own, and the tests of the cycle of
//! Space as a whole. The cycle is `veydan_sync_host`; notes bring their
//! handlers in `veydan_notes`, browser takes these along when it moves.
//!
//! - `profile_files` — Firefox profile directories: lease + file snapshots
//! - `state`         — the sync positions of profile files and what browser adds to the status

#[cfg(all(test, desktop))]
mod collect_tests;
#[cfg(all(test, desktop))]
mod compat_tests;
#[cfg(test)]
mod golden;
#[cfg(all(test, desktop))]
mod join_tests;
#[cfg(desktop)]
pub(crate) mod profile_files;
#[cfg(all(test, desktop))]
mod registries_tests;
#[cfg(test)]
mod rows_tests;
pub(crate) mod state;

#[cfg(desktop)]
pub(crate) use veydan_sync_host::{
    blob_gone, config, emit_progress, open_engine, progress_pct, SyncManager,
};
#[cfg(desktop)]
pub(crate) use veydan_sync_host::{debug_on, trace, trigger_cycle, EVENT_CHANGED};

#[cfg(desktop)]
use crate::error::{AppError, CmdResult};
#[cfg(desktop)]
use tauri::{AppHandle, Manager};
#[cfg(desktop)]
use veydan_core::Core;

#[cfg(desktop)]
pub const ERR_PROFILE_IN_USE: &str = "profile_in_use";

/// Sync is on, a vault is joined and profile files are included.
#[cfg(desktop)]
async fn profile_files_active(core: &Core) -> bool {
    config::load_config(&core.db).await.profile_files && veydan_sync_host::vault_active(core).await
}

/// Before the browser starts: refuse a profile leased elsewhere (unless forced),
/// apply a snapshot that waited, take the lease.
#[cfg(desktop)]
pub async fn before_profile_launch(
    app: &AppHandle,
    profile_id: &str,
    force: bool,
) -> CmdResult<()> {
    let core = app.state::<Core>();
    let sync = app.state::<SyncManager>();
    if !veydan_sync_host::vault_active(&core).await {
        return Ok(());
    }
    if !force {
        if let Some((_, name)) = profile_files::foreign_lease(&core, profile_id).await? {
            return Err(AppError::other(format!("{ERR_PROFILE_IN_USE}:{name}")));
        }
        if profile_files_active(&core).await
            && profile_files::has_pending(&core, profile_id).await?
        {
            let engine = open_engine(&core).await?;
            profile_files::apply_pending(&engine, &core, &sync, app, profile_id).await?;
        }
    }
    profile_files::acquire_lease(&core, profile_id).await?;
    trigger_cycle(app, "profile-launch");
    Ok(())
}

/// After the browser exited: mark files dirty, release the lease, sync soon.
#[cfg(desktop)]
pub async fn on_profile_stopped(app: &AppHandle, profile_id: &str) {
    let core = app.state::<Core>();
    if !veydan_sync_host::vault_active(&core).await {
        return;
    }
    if let Err(e) = profile_files::on_profile_stopped(&core, profile_id).await {
        eprintln!("sync: {e}");
    }
    trigger_cycle(app, "profile-stop");
}

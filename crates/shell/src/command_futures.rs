// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a command leaves on the thread that calls it. On Android a command
//! is called on the WebView's bridge thread, whose stack is small: Tauri
//! builds the command's future there and moves it a few times before it is
//! spawned, so a large one overflows that stack and the app dies (живой
//! запуск 2026-10-23: `channel_open_link` of the channels, 19 KB, SIGSEGV
//! in a release build). A command whose work is large boxes it
//! (`Box::pin`), so that only the box crosses the bridge. The size is read
//! from the type of the command, without calling it.

fn r1<A, R>(_: fn(A) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r2<A, B, R>(_: fn(A, B) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r3<A, B, C, R>(_: fn(A, B, C) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r4<A, B, C, D, R>(_: fn(A, B, C, D) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r5<A, B, C, D, E, R>(_: fn(A, B, C, D, E) -> R) -> usize {
    std::mem::size_of::<R>()
}

/// The bound every command keeps: well below a stack of the bridge thread
/// even when Tauri copies the future a few times.
const MAX_FUTURE_BYTES: usize = 4096;

#[test]
#[allow(clippy::vec_init_then_push)]
fn every_command_leaves_a_small_future_on_the_calling_thread() {
    let mut v: Vec<(&str, usize)> = Vec::new();
    v.push(("app_clear_data", r3(crate::demo::app_clear_data)));
    v.push(("demo_seed", r4(crate::demo::demo_seed)));
    v.push(("host_info", r1(crate::commands::host::host_info)));
    v.push(("open_url", r2(crate::commands::host::open_url)));
    #[cfg(desktop)]
    v.push(("clipboard_write_text", r1(crate::commands::host::clipboard_write_text)));
    v.push(("updater_check", r1(crate::updater::updater_check)));
    v.push(("updater_install", r2(crate::updater::updater_install)));
    v.push(("app_relaunch", r2(crate::updater::app_relaunch)));
    v.push(("lock_status", r1(crate::lock::lock_status)));
    v.push(("lock_set", r5(crate::lock::lock_set)));
    v.push(("lock_recovery_regenerate", r2(crate::lock::lock_recovery_regenerate)));
    v.push(("lock_recovery_check", r2(crate::lock::lock_recovery_check)));
    v.push(("lock_recover", r5(crate::lock::lock_recover)));
    v.push(("lock_recover_keep", r5(crate::lock::lock_recover_keep)));
    v.push(("lock_timeout_set", r3(crate::lock::lock_timeout_set)));
    v.push(("lock_unlock", r2(crate::lock::lock_unlock)));
    v.push(("lock_lock", r1(crate::lock::lock_lock)));
    v.push(("lock_touch", r1(crate::lock::lock_touch)));
    v.push(("vault_replaced_keys", r1(crate::lock::vault_replaced_keys)));
    v.push(("vault_replaced_keys_open", r2(crate::lock::vault_replaced_keys_open)));
    v.push(("update_check", r3(crate::update::update_check)));
    v.push(("update_open", r2(crate::update::update_open)));
    v.push(("labels_resolve", r2(crate::commands::labels::labels_resolve)));
    v.push(("tray_settings_get", r1(crate::commands::settings::tray_settings_get)));
    v.push(("tray_settings_set", r4(crate::commands::settings::tray_settings_set)));
    v.push(("window_minimize", r3(crate::commands::settings::window_minimize)));
    v.push(("app_locale_set", r2(crate::commands::settings::app_locale_set)));
    v.push(("app_locale_get", r1(crate::commands::settings::app_locale_get)));
    v.push(("tray_set_labels", r2(crate::commands::settings::tray_set_labels)));
    v.push(("modules_set_enabled", r3(crate::switches::modules_set_enabled)));
    v.push(("modules_choose", r2(crate::switches::modules_choose)));
    v.push(("modules_first_run_done", r1(crate::switches::modules_first_run_done)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

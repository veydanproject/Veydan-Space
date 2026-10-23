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

/// The bound every command keeps: well below a stack of the bridge thread
/// even when Tauri copies the future a few times.
const MAX_FUTURE_BYTES: usize = 4096;

#[test]
#[allow(clippy::vec_init_then_push)]
fn every_command_leaves_a_small_future_on_the_calling_thread() {
    let mut v: Vec<(&str, usize)> = Vec::new();
    v.push(("sync_get_config", r1(crate::commands::sync_get_config)));
    v.push(("sync_set_config", r2(crate::commands::sync_set_config)));
    v.push(("sync_probe", r1(crate::commands::sync_probe)));
    v.push(("sync_create_vault", r4(crate::commands::sync_create_vault)));
    v.push(("sync_join_vault", r4(crate::commands::sync_join_vault)));
    v.push(("sync_change_storage", r4(crate::commands::sync_change_storage)));
    v.push(("sync_leave", r3(crate::commands::sync_leave)));
    v.push(("sync_change_passphrase", r3(crate::commands::sync_change_passphrase)));
    v.push(("sync_status", r2(crate::commands::sync_status)));
    v.push(("sync_debug_get", r2(crate::commands::sync_debug_get)));
    v.push(("sync_debug_set", r3(crate::commands::sync_debug_set)));
    v.push(("sync_debug_clear", r1(crate::commands::sync_debug_clear)));
    v.push(("sync_run_now", r3(crate::commands::sync_run_now)));
    v.push(("sync_trigger", r2(crate::commands::sync_trigger)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

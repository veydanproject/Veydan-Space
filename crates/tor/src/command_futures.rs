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
    v.push(("tor_status", r1(crate::install::tor_status)));
    v.push(("tor_download_state", r1(crate::install::tor_download_state)));
    v.push(("tor_download_cancel", r1(crate::install::tor_download_cancel)));
    v.push(("tor_download", r4(crate::install::tor_download)));
    v.push(("tor_install_from_archive", r5(crate::install::tor_install_from_archive)));
    v.push(("tor_remove", r2(crate::install::tor_remove)));
    v.push(("tor_set_settings", r3(crate::commands::tor_set_settings)));
    v.push(("tor_start", r2(crate::commands::tor_start)));
    v.push(("tor_new_identity", r2(crate::commands::tor_new_identity)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

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
    v.push(("pwgen_history_list", r2(crate::password::pwgen_history_list)));
    v.push(("pwgen_history_add", r3(crate::password::pwgen_history_add)));
    v.push(("pwgen_history_clear", r2(crate::password::pwgen_history_clear)));
    v.push(("pwgen_history_trim", r3(crate::password::pwgen_history_trim)));
    v.push(("password_list", r1(crate::passwords::password_list)));
    v.push(("password_get", r2(crate::passwords::password_get)));
    v.push(("password_create", r3(crate::passwords::password_create)));
    v.push(("password_update", r4(crate::passwords::password_update)));
    v.push(("password_delete", r2(crate::passwords::password_delete)));
    v.push(("password_reveal", r4(crate::passwords::password_reveal)));
    #[cfg(desktop)]
    v.push(("password_copy", r3(crate::passwords::password_copy)));
    v.push(("password_vault_reset", r3(crate::passwords::password_vault_reset)));
    v.push(("totp_list", r1(crate::totp::totp_list)));
    v.push(("totp_add", r2(crate::totp::totp_add)));
    v.push(("totp_update", r3(crate::totp::totp_update)));
    v.push(("totp_delete", r2(crate::totp::totp_delete)));
    v.push(("totp_generate_code", r3(crate::totp::totp_generate_code)));
    v.push(("totp_generate_codes", r3(crate::totp::totp_generate_codes)));
    v.push(("totp_preview_uri", r1(crate::totp::totp_preview_uri)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

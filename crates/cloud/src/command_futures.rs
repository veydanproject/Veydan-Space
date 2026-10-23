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
fn r6<A, B, C, D, E, G, R>(_: fn(A, B, C, D, E, G) -> R) -> usize {
    std::mem::size_of::<R>()
}
fn r7<A, B, C, D, E, G, H, R>(_: fn(A, B, C, D, E, G, H) -> R) -> usize {
    std::mem::size_of::<R>()
}

/// The bound every command keeps: well below a stack of the bridge thread
/// even when Tauri copies the future a few times.
const MAX_FUTURE_BYTES: usize = 4096;

#[test]
#[allow(clippy::vec_init_then_push)]
fn every_command_leaves_a_small_future_on_the_calling_thread() {
    let mut v: Vec<(&str, usize)> = Vec::new();
    v.push(("cloud_recovery_key_show_once", r3(crate::recovery::cloud_recovery_key_show_once::<tauri::Wry>)));
    v.push(("cloud_recovery_key_reissue", r3(crate::recovery::cloud_recovery_key_reissue::<tauri::Wry>)));
    v.push(("cloud_recovery_status", r1(crate::recovery::cloud_recovery_status::<tauri::Wry>)));
    v.push(("cloud_restore_peek", r2(crate::recovery::cloud_restore_peek)));
    v.push(("cloud_restore_with_words", r7(crate::recovery::cloud_restore_with_words)));
    v.push(("cloud_delete_account", r4(crate::delete::cloud_delete_account::<tauri::Wry>)));
    v.push(("cloud_recovery_alerts", r1(crate::cancel::cloud_recovery_alerts::<tauri::Wry>)));
    v.push(("cloud_recovery_cancel", r2(crate::cancel::cloud_recovery_cancel::<tauri::Wry>)));
    v.push(("cloud_recovery_alert_dismiss", r2(crate::cancel::cloud_recovery_alert_dismiss::<tauri::Wry>)));
    v.push(("cloud_recover_start", r2(crate::request::cloud_recover_start::<tauri::Wry>)));
    v.push(("cloud_recover_poll", r1(crate::request::cloud_recover_poll::<tauri::Wry>)));
    v.push(("cloud_recover_peek", r1(crate::request::cloud_recover_peek)));
    v.push(("cloud_recover_finish", r6(crate::request::cloud_recover_finish)));
    v.push(("cloud_recover_cancel", r1(crate::request::cloud_recover_cancel::<tauri::Wry>)));
    v.push(("cloud_link_start", r1(crate::link::cloud_link_start::<tauri::Wry>)));
    v.push(("cloud_link_poll", r1(crate::link::cloud_link_poll::<tauri::Wry>)));
    v.push(("cloud_link_receive", r3(crate::link::cloud_link_receive::<tauri::Wry>)));
    v.push(("cloud_link_resolve", r2(crate::link::cloud_link_resolve::<tauri::Wry>)));
    v.push(("cloud_link_approve", r2(crate::link::cloud_link_approve::<tauri::Wry>)));
    v.push(("cloud_link_cancel", r1(crate::link::cloud_link_cancel::<tauri::Wry>)));
    v.push(("cloud_usage", r1(crate::sync::cloud_usage::<tauri::Wry>)));
    v.push(("cloud_web_login_inspect", r2(crate::weblogin::cloud_web_login_inspect::<tauri::Wry>)));
    v.push(("cloud_web_login_approve", r2(crate::weblogin::cloud_web_login_approve::<tauri::Wry>)));
    v.push(("cloud_web_login_deny", r2(crate::weblogin::cloud_web_login_deny::<tauri::Wry>)));
    v.push(("cloud_web_confirm", r4(crate::weblogin::cloud_web_confirm::<tauri::Wry>)));
    v.push(("cloud_web_unfreeze", r1(crate::weblogin::cloud_web_unfreeze::<tauri::Wry>)));
    v.push(("cloud_web_sessions", r1(crate::weblogin::cloud_web_sessions::<tauri::Wry>)));
    v.push(("cloud_web_sessions_revoke", r3(crate::weblogin::cloud_web_sessions_revoke::<tauri::Wry>)));
    v.push(("cloud_trustees_status", r1(crate::social::cloud_trustees_status::<tauri::Wry>)));
    v.push(("cloud_contacts_approved", r1(crate::social::cloud_contacts_approved::<tauri::Wry>)));
    v.push(("cloud_trustees_setup", r3(crate::social::cloud_trustees_setup::<tauri::Wry>)));
    v.push(("cloud_trustees_disable", r1(crate::social::cloud_trustees_disable::<tauri::Wry>)));
    v.push(("cloud_trustees_check", r1(crate::social::cloud_trustees_check::<tauri::Wry>)));
    v.push(("cloud_help_lookup", r2(crate::trustee::cloud_help_lookup::<tauri::Wry>)));
    v.push(("cloud_help_confirm", r2(crate::trustee::cloud_help_confirm::<tauri::Wry>)));
    v.push(("cloud_help_close", r1(crate::trustee::cloud_help_close::<tauri::Wry>)));
    v.push(("cloud_status", r1(crate::status::cloud_status::<tauri::Wry>)));
    v.push(("cloud_account_refresh", r1(crate::account::cloud_account_refresh::<tauri::Wry>)));
    v.push(("cloud_limits", r1(crate::account::cloud_limits::<tauri::Wry>)));
    v.push(("cloud_plans", r1(crate::account::cloud_plans::<tauri::Wry>)));
    v.push(("cloud_activate_free", r1(crate::activate::cloud_activate_free::<tauri::Wry>)));
    v.push(("cloud_redeem", r2(crate::activate::cloud_redeem::<tauri::Wry>)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

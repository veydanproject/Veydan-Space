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
fn r6<A, B, C, D, E, G, R>(_: fn(A, B, C, D, E, G) -> R) -> usize {
    std::mem::size_of::<R>()
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn r10<A, B, C, D, E, G, H, I, J, K, R>(_: fn(A, B, C, D, E, G, H, I, J, K) -> R) -> usize {
    std::mem::size_of::<R>()
}

/// The bound every command keeps: well below a stack of the bridge thread
/// even when Tauri copies the future a few times.
const MAX_FUTURE_BYTES: usize = 4096;

#[test]
#[allow(clippy::vec_init_then_push)]
fn every_command_leaves_a_small_future_on_the_calling_thread() {
    let mut v: Vec<(&str, usize)> = Vec::new();
    v.push(("messenger_desktop_notify_get", r2(crate::commands::desktop_notify::messenger_desktop_notify_get)));
    v.push(("messenger_desktop_notify_set", r4(crate::commands::desktop_notify::messenger_desktop_notify_set)));
    v.push(("messenger_desktop_notify_test", r3(crate::commands::desktop_notify::messenger_desktop_notify_test)));
    v.push(("messenger_desktop_notify_keep_running", r2(crate::commands::desktop_notify::messenger_desktop_notify_keep_running)));
    v.push(("messenger_call_start", r4(crate::commands::calls::messenger_call_start)));
    v.push(("messenger_call_accept", r3(crate::commands::calls::messenger_call_accept)));
    v.push(("messenger_call_decline", r2(crate::commands::calls::messenger_call_decline)));
    v.push(("messenger_call_end", r2(crate::commands::calls::messenger_call_end)));
    v.push(("messenger_call_mute", r2(crate::commands::calls::messenger_call_mute)));
    v.push(("messenger_call_set_video", r2(crate::commands::calls::messenger_call_set_video)));
    v.push(("messenger_call_switch_camera", r2(crate::commands::calls::messenger_call_switch_camera)));
    v.push(("messenger_call_list_cameras", r1(crate::commands::calls::messenger_call_list_cameras)));
    #[cfg(desktop)]
    v.push(("messenger_call_list_screens", r1(crate::commands::calls::messenger_call_list_screens)));
    #[cfg(desktop)]
    v.push(("messenger_call_share_screen", r2(crate::commands::calls::messenger_call_share_screen)));
    v.push(("messenger_call_set_video_quality", r2(crate::commands::calls::messenger_call_set_video_quality)));
    v.push(("messenger_call_video_subscribe", r3(crate::commands::calls::messenger_call_video_subscribe)));
    v.push(("messenger_call_video_ack", r2(crate::commands::calls::messenger_call_video_ack)));
    v.push(("messenger_call_video_unsubscribe", r1(crate::commands::calls::messenger_call_video_unsubscribe)));
    v.push(("messenger_call_set_policy", r2(crate::commands::calls::messenger_call_set_policy)));
    v.push(("messenger_call_set_nodes", r2(crate::commands::calls::messenger_call_set_nodes)));
    v.push(("messenger_call_nodes_list", r2(crate::commands::calls::messenger_call_nodes_list)));
    v.push(("messenger_call_nodes_add", r4(crate::commands::calls::messenger_call_nodes_add)));
    v.push(("messenger_call_nodes_remove", r2(crate::commands::calls::messenger_call_nodes_remove)));
    v.push(("messenger_call_nodes_set_trust", r2(crate::commands::calls::messenger_call_nodes_set_trust)));
    v.push(("messenger_call_nodes_refresh", r1(crate::commands::calls::messenger_call_nodes_refresh)));
    v.push(("messenger_call_history", r3(crate::commands::calls::messenger_call_history)));
    v.push(("messenger_call_history_hide", r2(crate::commands::calls::messenger_call_history_hide)));
    v.push(("messenger_call_history_clear", r1(crate::commands::calls::messenger_call_history_clear)));
    v.push(("messenger_calls_missed", r1(crate::commands::calls::messenger_calls_missed)));
    v.push(("messenger_calls_mark_seen", r1(crate::commands::calls::messenger_calls_mark_seen)));
    v.push(("messenger_call_get_state", r1(crate::commands::calls::messenger_call_get_state)));
    v.push(("messenger_call_set_incoming", r2(crate::commands::calls::messenger_call_set_incoming)));
    v.push(("messenger_call_network_changed", r1(crate::commands::calls::messenger_call_network_changed)));
    v.push(("messenger_group_call_start", r4(crate::commands::calls::messenger_group_call_start)));
    v.push(("messenger_group_call_join", r3(crate::commands::calls::messenger_group_call_join)));
    v.push(("messenger_group_call_leave", r1(crate::commands::calls::messenger_group_call_leave)));
    v.push(("messenger_group_call_mute", r2(crate::commands::calls::messenger_group_call_mute)));
    v.push(("messenger_group_call_set_video", r2(crate::commands::calls::messenger_group_call_set_video)));
    v.push(("messenger_group_call_switch_camera", r2(crate::commands::calls::messenger_group_call_switch_camera)));
    v.push(("messenger_group_call_set_layer", r3(crate::commands::calls::messenger_group_call_set_layer)));
    v.push(("messenger_group_call_get_state", r2(crate::commands::calls::messenger_group_call_get_state)));
    v.push(("messenger_group_call_video_subscribe", r3(crate::commands::calls::messenger_group_call_video_subscribe)));
    v.push(("messenger_call_audio_route", r2(crate::commands::call_android::messenger_call_audio_route)));
    v.push(("messenger_call_camera", r2(crate::commands::call_android::messenger_call_camera)));
    v.push(("messenger_call_debug_ring", r2(crate::commands::call_android::messenger_call_debug_ring)));
    v.push(("messenger_report_send", r2(crate::commands::report::messenger_report_send)));
    v.push(("messenger_moderation_requests", r3(crate::commands::moderation::messenger_moderation_requests)));
    v.push(("messenger_moderation_request_act", r3(crate::commands::moderation::messenger_moderation_request_act)));
    v.push(("messenger_moderation_requests_count", r1(crate::commands::moderation::messenger_moderation_requests_count)));
    v.push(("messenger_moderation_dm_settings", r1(crate::commands::moderation::messenger_moderation_dm_settings)));
    v.push(("messenger_moderation_dm_settings_set", r2(crate::commands::moderation::messenger_moderation_dm_settings_set)));
    v.push(("messenger_moderation_group_queue", r5(crate::commands::moderation::messenger_moderation_group_queue)));
    v.push(("messenger_moderation_group_mod", r3(crate::commands::moderation::messenger_moderation_group_mod)));
    v.push(("messenger_moderation_group_log", r4(crate::commands::moderation::messenger_moderation_group_log)));
    v.push(("messenger_moderation_group_settings", r2(crate::commands::moderation::messenger_moderation_group_settings)));
    v.push(("messenger_moderation_group_settings_set", r3(crate::commands::moderation::messenger_moderation_group_settings_set)));
    v.push(("messenger_moderation_group_inherit", r3(crate::commands::moderation::messenger_moderation_group_inherit)));
    v.push(("messenger_moderation_invite_create", r2(crate::commands::moderation::messenger_moderation_invite_create)));
    v.push(("messenger_moderation_invite_list", r1(crate::commands::moderation::messenger_moderation_invite_list)));
    v.push(("messenger_moderation_invite_revoke", r2(crate::commands::moderation::messenger_moderation_invite_revoke)));
    v.push(("messenger_moderation_invite_status", r1(crate::commands::moderation::messenger_moderation_invite_status)));
    v.push(("messenger_contact_link_qr", r2(crate::commands::moderation::messenger_contact_link_qr)));
    v.push(("messenger_moderation_blocked", r1(crate::commands::moderation::messenger_moderation_blocked)));
    v.push(("messenger_push_status", r2(crate::commands::push::messenger_push_status)));
    v.push(("messenger_push_set_enabled", r3(crate::commands::push::messenger_push_set_enabled)));
    v.push(("messenger_push_mark_offered", r1(crate::commands::push::messenger_push_mark_offered)));
    v.push(("messenger_push_set_server", r3(crate::commands::push::messenger_push_set_server)));
    v.push(("messenger_push_set_prefs", r4(crate::commands::push::messenger_push_set_prefs)));
    v.push(("messenger_push_refresh", r2(crate::commands::push::messenger_push_refresh)));
    v.push(("messenger_push_test", r1(crate::commands::push::messenger_push_test)));
    v.push(("messenger_push_tickets", r1(crate::commands::push::messenger_push_tickets)));
    v.push(("messenger_push_ticket_revoke", r2(crate::commands::push::messenger_push_ticket_revoke)));
    v.push(("messenger_push_take_tap", r1(crate::commands::push::messenger_push_take_tap)));
    v.push(("messenger_push_clear", r2(crate::commands::push::messenger_push_clear)));
    v.push(("messenger_push_open_settings", r1(crate::commands::push::messenger_push_open_settings)));
    v.push(("messenger_notify_get", r2(crate::commands::push::messenger_notify_get)));
    v.push(("messenger_notify_set", r4(crate::commands::push::messenger_notify_set)));
    v.push(("messenger_status", r2(crate::commands::messenger_status)));
    v.push(("messenger_set_enabled", r2(crate::commands::messenger_set_enabled)));
    v.push(("messenger_identity_get", r1(crate::commands::messenger_identity_get)));
    v.push(("messenger_identity_create", r2(crate::commands::messenger_identity_create)));
    v.push(("messenger_identity_import", r4(crate::commands::messenger_identity_import)));
    v.push(("messenger_identity_export", r2(crate::commands::messenger_identity_export)));
    v.push(("messenger_identity_delete", r1(crate::commands::messenger_identity_delete)));
    v.push(("messenger_relays_list", r1(crate::commands::messenger_relays_list)));
    v.push(("messenger_relays_add", r3(crate::commands::messenger_relays_add)));
    v.push(("messenger_profile_get", r2(crate::commands::messenger_profile_get)));
    v.push(("messenger_profile_request", r2(crate::commands::messenger_profile_request)));
    v.push(("messenger_profile_own_get", r1(crate::commands::messenger_profile_own_get)));
    v.push(("messenger_profile_own_set", r2(crate::commands::messenger_profile_own_set)));
    v.push(("messenger_social_platforms", r1(crate::commands::messenger_social_platforms)));
    v.push(("messenger_bio_parse", r2(crate::commands::messenger_bio_parse)));
    v.push(("messenger_avatar_prepare", r3(crate::commands::messenger_avatar_prepare)));
    v.push(("messenger_avatar_set", r3(crate::commands::messenger_avatar_set)));
    v.push(("messenger_avatar_remove", r1(crate::commands::messenger_avatar_remove)));
    v.push(("messenger_avatar_cached", r2(crate::commands::messenger_avatar_cached)));
    v.push(("messenger_own_private_get", r1(crate::commands::messenger_own_private_get)));
    v.push(("messenger_own_private_set", r3(crate::commands::messenger_own_private_set)));
    v.push(("messenger_ui_prefs_get", r1(crate::commands::messenger_ui_prefs_get)));
    v.push(("messenger_ui_prefs_set", r3(crate::commands::messenger_ui_prefs_set)));
    v.push(("messenger_contact_private_get", r2(crate::commands::messenger_contact_private_get)));
    v.push(("messenger_card_send", r5(crate::commands::messenger_card_send)));
    v.push(("messenger_card_accept", r2(crate::commands::messenger_card_accept)));
    v.push(("messenger_nip05_verify", r2(crate::commands::messenger_nip05_verify)));
    v.push(("messenger_contacts_list", r1(crate::commands::messenger_contacts_list)));
    v.push(("messenger_contacts_add", r3(crate::commands::messenger_contacts_add)));
    v.push(("messenger_contacts_update", r3(crate::commands::messenger_contacts_update)));
    v.push(("messenger_contacts_remove", r2(crate::commands::messenger_contacts_remove)));
    v.push(("messenger_contacts_set_followed", r3(crate::commands::messenger_contacts_set_followed)));
    v.push(("messenger_chats_list", r2(crate::commands::messenger_chats_list)));
    v.push(("messenger_chat_open", r2(crate::commands::messenger_chat_open)));
    v.push(("messenger_chat_messages", r4(crate::commands::messenger_chat_messages)));
    v.push(("messenger_chat_shared_counts", r2(crate::commands::messenger_chat_shared_counts)));
    v.push(("messenger_chat_shared", r5(crate::commands::messenger_chat_shared)));
    v.push(("messenger_privacy_get", r1(crate::commands::messenger_privacy_get)));
    v.push(("messenger_privacy_set", r3(crate::commands::messenger_privacy_set)));
    v.push(("messenger_presence_list", r1(crate::commands::messenger_presence_list)));
    v.push(("messenger_presence_foreground", r2(crate::commands::messenger_presence_foreground)));
    v.push(("messenger_chat_mark_read", r2(crate::commands::messenger_chat_mark_read)));
    v.push(("messenger_chat_set_pinned", r3(crate::commands::messenger_chat_set_pinned)));
    v.push(("messenger_chat_set_archived", r3(crate::commands::messenger_chat_set_archived)));
    v.push(("messenger_chat_set_muted", r3(crate::commands::messenger_chat_set_muted)));
    v.push(("messenger_chat_delete", r2(crate::commands::messenger_chat_delete)));
    v.push(("messenger_dm_send_text", r5(crate::commands::messenger_dm_send_text)));
    v.push(("messenger_dm_edit", r4(crate::commands::messenger_dm_edit)));
    v.push(("messenger_dm_react", r3(crate::commands::messenger_dm_react)));
    v.push(("messenger_emoji_used", r2(crate::commands::messenger_emoji_used)));
    v.push(("messenger_emoji_top", r2(crate::commands::messenger_emoji_top)));
    v.push(("messenger_dm_delete", r3(crate::commands::messenger_dm_delete)));
    v.push(("messenger_dm_retry", r2(crate::commands::messenger_dm_retry)));
    v.push(("messenger_forward", r6(crate::commands::messenger_forward)));
    v.push(("messenger_dm_relation", r2(crate::commands::messenger_dm_relation)));
    v.push(("messenger_dm_action", r5(crate::commands::messenger_dm_action)));
    v.push(("messenger_groups_list", r1(crate::commands::messenger_groups_list)));
    v.push(("messenger_group_get", r2(crate::commands::messenger_group_get)));
    v.push(("messenger_group_create", r6(crate::commands::messenger_group_create)));
    v.push(("messenger_group_invite", r3(crate::commands::messenger_group_invite)));
    v.push(("messenger_group_invites", r2(crate::commands::messenger_group_invites)));
    v.push(("messenger_group_answer_invite", r3(crate::commands::messenger_group_answer_invite)));
    v.push(("messenger_group_open_link", r3(crate::commands::messenger_group_open_link)));
    v.push(("messenger_group_answer_request", r4(crate::commands::messenger_group_answer_request)));
    v.push(("messenger_group_act", r3(crate::commands::messenger_group_act)));
    v.push(("messenger_group_avatar_set", r4(crate::commands::messenger_group_avatar_set)));
    v.push(("messenger_group_avatar_remove", r2(crate::commands::messenger_group_avatar_remove)));
    v.push(("messenger_group_rotate_link", r2(crate::commands::messenger_group_rotate_link)));
    v.push(("messenger_group_link_qr", r2(crate::commands::messenger_group_link_qr)));
    v.push(("messenger_group_forget", r2(crate::commands::messenger_group_forget)));
    v.push(("messenger_group_history", r4(crate::commands::messenger_group_history)));
    v.push(("messenger_dm_history", r3(crate::commands::messenger_dm_history)));
    v.push(("messenger_dm_blocked", r1(crate::commands::messenger_dm_blocked)));
    v.push(("messenger_media_servers", r1(crate::commands::messenger_media_servers)));
    v.push(("messenger_media_server_put", r2(crate::commands::messenger_media_server_put)));
    v.push(("messenger_media_server_remove", r2(crate::commands::messenger_media_server_remove)));
    v.push(("messenger_media_server_set_enabled", r3(crate::commands::messenger_media_server_set_enabled)));
    v.push(("messenger_media_server_check", r2(crate::commands::messenger_media_server_check)));
    v.push(("messenger_dm_send_file", r10(crate::commands::messenger_dm_send_file)));
    v.push(("messenger_media_import", r4(crate::commands::messenger_media_import)));
    v.push(("messenger_gallery_access", r1(crate::commands::gallery::messenger_gallery_access)));
    v.push(("messenger_gallery_request", r1(crate::commands::gallery::messenger_gallery_request)));
    v.push(("messenger_gallery_recent", r5(crate::commands::gallery::messenger_gallery_recent)));
    v.push(("messenger_gallery_albums", r1(crate::commands::gallery::messenger_gallery_albums)));
    v.push(("messenger_gallery_thumb", r3(crate::commands::gallery::messenger_gallery_thumb)));
    v.push(("messenger_gallery_forget_thumbs", r1(crate::commands::gallery::messenger_gallery_forget_thumbs)));
    v.push(("messenger_gallery_readable", r2(crate::commands::gallery::messenger_gallery_readable)));
    v.push(("messenger_gallery_capture", r2(crate::commands::gallery::messenger_gallery_capture)));
    v.push(("messenger_gallery_pick", r3(crate::commands::gallery::messenger_gallery_pick)));
    v.push(("messenger_gallery_save_picture", r3(crate::commands::gallery::messenger_gallery_save_picture)));
    v.push(("messenger_share_out", r4(crate::commands::share_out::messenger_share_out)));
    v.push(("messenger_media_import_bytes", r4(crate::commands::messenger_media_import_bytes)));
    v.push(("messenger_clipboard_paste", r1(crate::commands::messenger_clipboard_paste)));
    v.push(("messenger_media_download", r3(crate::commands::messenger_media_download)));
    v.push(("messenger_media_foreign_server", r2(crate::commands::messenger_media_foreign_server)));
    v.push(("messenger_media_transfer", r2(crate::commands::messenger_media_transfer)));
    v.push(("messenger_media_transfers", r1(crate::commands::messenger_media_transfers)));
    v.push(("messenger_media_retry_failed", r1(crate::commands::messenger_media_retry_failed)));
    v.push(("messenger_media_pause", r2(crate::commands::messenger_media_pause)));
    v.push(("messenger_media_resume", r2(crate::commands::messenger_media_resume)));
    v.push(("messenger_media_cancel", r2(crate::commands::messenger_media_cancel)));
    v.push(("messenger_media_save_as", r4(crate::commands::messenger_media_save_as)));
    v.push(("messenger_media_data_url", r2(crate::commands::messenger_media_data_url)));
    v.push(("messenger_media_local_path", r2(crate::commands::messenger_media_local_path)));
    v.push(("messenger_media_url", r3(crate::commands::messenger_media_url)));
    v.push(("messenger_open_url", r2(crate::commands::messenger_open_url)));
    v.push(("messenger_links_inspect", r2(crate::commands::messenger_links_inspect)));
    v.push(("messenger_link_incoming", r2(crate::commands::messenger_link_incoming)));
    v.push(("messenger_contact_link", r3(crate::commands::messenger_contact_link)));
    v.push(("messenger_private_link_qr", r2(crate::commands::messenger_private_link_qr)));
    v.push(("messenger_link_preview", r2(crate::commands::messenger_link_preview)));
    v.push(("messenger_media_open", r3(crate::commands::messenger_media_open)));
    v.push(("messenger_dm_send_recording", r3(crate::commands::messenger_dm_send_recording)));
    v.push(("messenger_relays_remove", r2(crate::commands::messenger_relays_remove)));
    v.push(("messenger_relays_set_enabled", r3(crate::commands::messenger_relays_set_enabled)));
    v.push(("messenger_relays_set_silent", r2(crate::commands::messenger_relays_set_silent)));
    v.push(("messenger_manifest_info", r1(crate::commands::messenger_manifest_info)));
    v.push(("messenger_manifest_set_region", r2(crate::commands::messenger_manifest_set_region)));
    v.push(("messenger_servers_use_veydan", r1(crate::commands::messenger_servers_use_veydan)));
    v.push(("messenger_servers_use_own", r1(crate::commands::messenger_servers_use_own)));
    v.push(("messenger_server_remove", r2(crate::commands::messenger_server_remove)));
    v.push(("messenger_server_preview", r2(crate::commands::messenger_server_preview)));
    v.push(("messenger_server_accept", r2(crate::commands::messenger_server_accept)));
    v.push(("messenger_contact_open_private", r4(crate::commands::messenger_contact_open_private)));
    v.push(("messenger_servers_private", r1(crate::commands::messenger_servers_private)));
    v.push(("messenger_manifest_refresh", r1(crate::commands::messenger_manifest_refresh)));
    v.push(("messenger_net_status", r1(crate::commands::messenger_net_status)));
    v.push(("messenger_net_set_mode", r2(crate::commands::messenger_net_set_mode)));
    v.push(("messenger_net_check", r1(crate::commands::messenger_net_check)));
    v.push(("messenger_net_bridge_add", r2(crate::commands::messenger_net_bridge_add)));
    v.push(("messenger_net_bridge_remove", r2(crate::commands::messenger_net_bridge_remove)));
    v.push(("messenger_net_offer_dismiss", r1(crate::commands::messenger_net_offer_dismiss)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

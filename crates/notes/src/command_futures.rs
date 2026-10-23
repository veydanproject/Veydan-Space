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
    v.push(("note_links", r2(crate::links::note_links)));
    v.push(("note_resolve_link", r2(crate::links::note_resolve_link)));
    v.push(("note_backlinks", r2(crate::links::note_backlinks)));
    v.push(("note_related", r2(crate::links::note_related)));
    v.push(("note_entity_notes", r2(crate::links::note_entity_notes)));
    v.push(("note_folder_list", r1(crate::folders::note_folder_list)));
    v.push(("note_folder_create", r4(crate::folders::note_folder_create)));
    v.push(("note_folder_update", r4(crate::folders::note_folder_update)));
    v.push(("note_folder_delete", r2(crate::folders::note_folder_delete)));
    v.push(("note_add_binding", r4(crate::folders::note_add_binding)));
    v.push(("note_remove_binding", r4(crate::folders::note_remove_binding)));
    v.push(("note_add_folder", r3(crate::folders::note_add_folder)));
    v.push(("note_remove_folder", r3(crate::folders::note_remove_folder)));
    v.push(("note_set_folder", r3(crate::folders::note_set_folder)));
    v.push(("sync_conflict_get", r2(crate::sync::commands::sync_conflict_get)));
    v.push(("sync_conflict_resolve", r7(crate::sync::commands::sync_conflict_resolve)));
    v.push(("note_sync_info", r2(crate::sync::commands::note_sync_info)));
    v.push(("sync_attachment_cancel", r3(crate::sync::commands::sync_attachment_cancel)));
    v.push(("note_list", r2(crate::crud::note_list)));
    v.push(("note_get", r2(crate::crud::note_get)));
    v.push(("note_create", r3(crate::crud::note_create)));
    v.push(("note_update", r4(crate::crud::note_update)));
    v.push(("note_delete", r3(crate::crud::note_delete)));
    v.push(("note_delete_many", r2(crate::crud::note_delete_many)));
    v.push(("note_trash_empty", r1(crate::crud::note_trash_empty)));
    v.push(("note_archive", r2(crate::crud::note_archive)));
    v.push(("note_restore", r2(crate::crud::note_restore)));
    v.push(("note_set_tags", r4(crate::crud::note_set_tags)));
    v.push(("note_search", r3(crate::crud::note_search)));
    v.push(("note_sync", r2(crate::crud::note_sync)));
    v.push(("note_reindex", r1(crate::crud::note_reindex)));
    #[cfg(desktop)]
    v.push(("note_open_folder", r2(crate::crud::note_open_folder)));
    #[cfg(desktop)]
    v.push(("note_open_external", r2(crate::crud::note_open_external)));
    v.push(("note_draft_save", r3(crate::crud::note_draft_save)));
    v.push(("note_draft_get", r2(crate::crud::note_draft_get)));
    v.push(("note_draft_discard", r2(crate::crud::note_draft_discard)));
    v.push(("note_history_list", r3(crate::history::note_history_list)));
    v.push(("note_history_get", r2(crate::history::note_history_get)));
    v.push(("note_history_diff", r4(crate::history::note_history_diff)));
    v.push(("note_history_restore", r4(crate::history::note_history_restore)));
    v.push(("note_history_merge", r3(crate::history::note_history_merge)));
    v.push(("notes_attachment_policy_get", r1(crate::settings::notes_attachment_policy_get)));
    v.push(("notes_attachment_policy_set", r2(crate::settings::notes_attachment_policy_set)));
    #[cfg(desktop)]
    v.push(("notes_get_dir", r2(crate::settings::notes_get_dir)));
    #[cfg(desktop)]
    v.push(("notes_set_dir", r4(crate::settings::notes_set_dir)));
    v.push(("note_placeholder_values", r3(crate::templates::note_placeholder_values)));
    v.push(("note_smart_view_list", r1(crate::smart_views::note_smart_view_list)));
    v.push(("note_smart_view_create", r2(crate::smart_views::note_smart_view_create)));
    v.push(("note_smart_view_update", r3(crate::smart_views::note_smart_view_update)));
    v.push(("note_smart_view_delete", r2(crate::smart_views::note_smart_view_delete)));
    #[cfg(desktop)]
    v.push(("note_attachment_add", r2(crate::attachments::note_attachment_add)));
    v.push(("note_attachment_read", r3(crate::attachments::note_attachment_read)));
    #[cfg(desktop)]
    v.push(("clipboard_file_paths", r1(crate::attachments::clipboard_file_paths)));
    v.push(("note_attachment_add_from_path", r4(crate::attachments::note_attachment_add_from_path)));
    v.push(("note_attachment_list", r2(crate::attachments::note_attachment_list)));
    v.push(("note_attachment_fetch", r6(crate::attachments::note_attachment_fetch)));
    v.push(("note_attachment_delete", r3(crate::attachments::note_attachment_delete)));
    #[cfg(desktop)]
    v.push(("note_attachment_open", r3(crate::attachments::note_attachment_open)));
    v.push(("note_attachment_save", r5(crate::attachments::note_attachment_save)));
    #[cfg(desktop)]
    v.push(("note_attachments_gc", r2(crate::attachments::note_attachments_gc)));
    v.push(("note_export", r5(crate::transfer::note_export)));
    v.push(("note_import", r5(crate::transfer::note_import)));
    v.push(("note_open_window", r2(crate::window::note_open_window)));
    v.push(("open_quick_capture", r1(crate::quick_capture::open_quick_capture)));
    v.push(("note_tag_list", r1(crate::tags::note_tag_list)));
    v.push(("note_tag_create", r3(crate::tags::note_tag_create)));
    v.push(("note_tag_delete", r2(crate::tags::note_tag_delete)));
    v.push(("note_tag_update", r4(crate::tags::note_tag_update)));
    v.push(("note_nav", r1(crate::nav::note_nav)));
    v.push(("note_binding_summaries", r2(crate::binding::note_binding_summaries)));
    v.push(("note_entity_search", r3(crate::binding::note_entity_search)));
    let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
    assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
}

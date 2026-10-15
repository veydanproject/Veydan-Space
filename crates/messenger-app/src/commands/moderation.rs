// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Moderation: the commands of the page, forwarded to the facade of the
//! runtime (`messenger_runtime::moderation`), with the types of
//! `ui/src/lib/messenger/generated/moderation.ts`: the queue of requests
//! of strangers and whom I take messages from, the moderation of groups,
//! the invitations. The queue is told by the runtime event `dm.request`
//! (`{peer, pending}`), after which the page reads it again; a pause of
//! the invitations by `dm.invite_paused` (`{until}`).
//!
//! What shows the words of a stranger, or speaks for the user, waits for
//! the lock to open (`runtime_unlocked`); a count and the settings do not,
//! as the count of unread does not.

use super::{map_err, MessengerState};
use messenger_runtime::moderation::{
    BlockedView, DmSettings, GroupModSettings, InvitePreset, InviteStatus, InviteView, LogPage, ModAction, QueueFilter, QueuePage,
    RequestAction, RequestsPage,
};
use veydan_core::CmdResult;

/// The queue of requests, newest first; `before` is the `next_before` of
/// the page shown last, `limit` 50 when not given.
#[tauri::command]
pub async fn messenger_moderation_requests(
    before: Option<i64>,
    limit: Option<u32>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<RequestsPage> {
    messenger.runtime_unlocked().await?.requests(before, limit.unwrap_or(50)).await.map_err(map_err)
}

/// `action`: accept | decline | block. Doing again what is done already
/// is fine.
#[tauri::command]
pub async fn messenger_moderation_request_act(
    peer: String,
    action: RequestAction,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime_unlocked().await?.request_act(&peer, action).await.map_err(map_err)?;
    // An accepted request is a chat with something unread in it.
    super::unread_changed();
    Ok(())
}

#[tauri::command]
pub async fn messenger_moderation_requests_count(messenger: tauri::State<'_, MessengerState>) -> CmdResult<u32> {
    messenger.runtime()?.requests_count().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_dm_settings(messenger: tauri::State<'_, MessengerState>) -> CmdResult<DmSettings> {
    messenger.runtime()?.dm_settings().await.map_err(map_err)
}

/// A shorter term applies at once: the queue is swept before this answers.
#[tauri::command]
pub async fn messenger_moderation_dm_settings_set(
    settings: DmSettings,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.dm_settings_set(settings).await.map_err(map_err)?;
    super::unread_changed();
    Ok(())
}

// ─── Groups: the queue, the acts, the log, the rules of posting ─────────────

#[tauri::command]
pub async fn messenger_moderation_group_queue(
    group: String,
    filter: Option<QueueFilter>,
    before: Option<i64>,
    limit: Option<u32>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<QueuePage> {
    messenger
        .runtime_unlocked()
        .await?
        .group_queue(&group, filter.unwrap_or_default(), before, limit.unwrap_or(50))
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_group_mod(
    group: String,
    action: ModAction,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime_unlocked().await?.group_mod(&group, action).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_group_log(
    group: String,
    before: Option<i64>,
    limit: Option<u32>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<LogPage> {
    messenger.runtime_unlocked().await?.group_log(&group, before, limit.unwrap_or(50)).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_group_settings(
    group: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupModSettings> {
    messenger.runtime()?.group_mod_settings(&group).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_group_settings_set(
    group: String,
    settings: GroupModSettings,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime_unlocked().await?.group_mod_settings_set(&group, settings).await.map_err(map_err)
}

/// Say again, as me, what another moderator (`from`, hex or npub)
/// decided in the group: for the prompt on their demotion. Answers how
/// many notes went out.
#[tauri::command]
pub async fn messenger_moderation_group_inherit(
    group: String,
    from: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<u32> {
    messenger.runtime_unlocked().await?.group_inherit(&group, &from).await.map_err(map_err)
}

// ─── Invitations ────────────────────────────────────────────────────────────

/// An invitation by its preset (`link` | `code` | `qr`); a term, a number
/// of uses or a person given override the preset.
#[tauri::command]
pub async fn messenger_moderation_invite_create(
    preset: InvitePreset,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<InviteView> {
    messenger.runtime_unlocked().await?.invite_create(preset).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_invite_list(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<InviteView>> {
    messenger.runtime_unlocked().await?.invite_list().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_moderation_invite_revoke(id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime_unlocked().await?.invite_revoke(&id).await.map_err(map_err)
}

/// Whether the invitations are paused on this device (somebody guessed
/// too often), until when.
#[tauri::command]
pub async fn messenger_moderation_invite_status(messenger: tauri::State<'_, MessengerState>) -> CmdResult<InviteStatus> {
    messenger.runtime()?.invite_status().await.map_err(map_err)
}

/// The link of an invitation of mine (`id`) as a QR code (SVG); a code
/// has no link (`invite_no_link`).
#[tauri::command]
pub async fn messenger_contact_link_qr(id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<String> {
    messenger.runtime_unlocked().await?.invite_qr(&id).await.map_err(map_err)
}

/// Whom I block, with their npub and the name the app calls them by.
#[tauri::command]
pub async fn messenger_moderation_blocked(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<BlockedView>> {
    messenger.runtime()?.blocked().await.map_err(map_err)
}

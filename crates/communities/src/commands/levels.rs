// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands of the levels and the sections of a channel
//! (internal/communities-wire.md §14): thin calls of the core on the
//! channel's lock, as `channel_act` is.

use super::{bad, detail, from_word, key, read_picked, COVER_DIR};
use crate::state::{map_err, CommunitiesState, Driver};
use crate::views::{ChannelDetail, LevelInput, LevelRequestView, LevelView, NewScopeInput, ScopeSetInput, ScopeView};
use communities_core::levels::ScopeChange;
use communities_core::media::MAX_COVER_INPUT_BYTES;
use communities_core::wire::ScopeLook;
use communities_core::service::Outcome;
use communities_core::{History, LevelCan, SchemeId};
use messenger_core::PubKey;
use veydan_core::CmdResult;

/// What a change of the levels did, carried out by the driver.
async fn changed(d: &Driver, community_id: &str, change: impl std::future::Future<Output = messenger_core::Result<Outcome>>) -> CmdResult<()> {
    let outcome = {
        let lock = d.service.lock_of(community_id).await;
        let _guard = lock.lock().await;
        change.await.map_err(map_err)?
    };
    d.apply(outcome).await;
    Ok(())
}

/// The levels of a channel as the journal and the sealed names give them.
async fn levels_of(d: &Driver, community_id: &str) -> CmdResult<Vec<LevelView>> {
    Ok(detail(d, community_id).await?.levels.unwrap_or_default())
}

#[tauri::command]
pub async fn channel_scopes(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<ScopeView>> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scopes");
        Ok(detail(&state.driver()?, &community_id).await?.scopes.unwrap_or_default())
    })
    .await
}

/// A new section (admin and above; hidden: the owner): its title and look
/// sealed, its keys its own.
#[tauri::command]
pub async fn channel_scope_add(community_id: String, input: NewScopeInput, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scope_add");
        let d = state.driver()?;
        let keys = d.keys()?;
        let scheme = match input.scheme.as_deref() {
            None | Some("sd-v1") => SchemeId::SdV1,
            Some("flat-v1") => SchemeId::FlatV1,
            Some(other) => return Err(bad("channel_invalid", format!("unknown scheme: {other}"))),
        };
        let history: History = from_word(input.history.as_deref().unwrap_or("open"))?;
        let words = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let look = ScopeLook { icon: words(&input.icon), photo: None, color: input.color, order: input.order, note: words(&input.note) };
        let hidden = input.hidden.unwrap_or(false);
        changed(&d, &community_id, async { d.service.scope_add_with(&keys, &community_id, &input.title, look, scheme, history, hidden).await.map(|(_, out)| out) }).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// The title, the look or the hiding of a section changes (the owner).
#[tauri::command]
pub async fn channel_scope_set(community_id: String, scope_id: String, input: ScopeSetInput, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scope_set");
        let d = state.driver()?;
        let keys = d.keys()?;
        let change = scope_change(&input)?;
        changed(&d, &community_id, d.service.scope_set(&keys, &community_id, &scope_id, &change)).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// What `channel_scope_set` takes as the core's change.
fn scope_change(input: &ScopeSetInput) -> CmdResult<ScopeChange> {
    let color = match input.color {
        None => None,
        Some(c) if c < 0 => Some(None),
        Some(c) => Some(Some(u8::try_from(c).map_err(|_| bad("channel_invalid", "scope color"))?)),
    };
    Ok(ScopeChange {
        title: input.title.clone(),
        icon: input.icon.clone(),
        photo: input.no_photo.filter(|n| *n).map(|_| None),
        color,
        order: input.order,
        note: input.note.clone(),
        hidden: input.hidden,
    })
}

/// The sections in this order (the owner): each one's place in `ids`.
#[tauri::command]
pub async fn channel_scopes_order(community_id: String, ids: Vec<String>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scopes_order");
        let d = state.driver()?;
        let keys = d.keys()?;
        changed(&d, &community_id, d.service.scopes_order(&keys, &community_id, &ids)).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// A picked picture (up to `MAX_COVER_INPUT_BYTES`) as the photo of a
/// section (the owner): a square JPEG, sealed on the hosts off the
/// channel's lock, then named by the section's meta.
#[tauri::command]
pub async fn channel_scope_photo_set(
    community_id: String,
    scope_id: String,
    src: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scope_photo_set");
        let d = state.driver()?;
        let keys = d.keys()?;
        let bytes = read_picked(&app, &src, MAX_COVER_INPUT_BYTES, "channel_cover_too_large")?;
        let up = d.service.upload_scope_photo(&keys, &community_id, &bytes).await.map_err(map_err)?;
        // Kept where `channel_scope_photo` reads it: nothing fetched back.
        let dir = d.files.join(COVER_DIR);
        if tokio::fs::create_dir_all(&dir).await.is_ok() {
            let target = dir.join(format!("scope-{}.jpg", up.photo.sha256));
            let part = target.with_extension("part");
            if tokio::fs::write(&part, &up.jpeg).await.is_ok() {
                let _ = tokio::fs::rename(&part, &target).await;
            }
        }
        let change = ScopeChange { photo: Some(Some(up.photo)), ..ScopeChange::default() };
        changed(&d, &community_id, d.service.scope_set(&keys, &community_id, &scope_id, &change)).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// The photo of a section as a `data:` URL; `null`: none, or no host has
/// it now.
#[tauri::command]
pub async fn channel_scope_photo(community_id: String, scope_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Option<String>> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scope_photo");
        let d = state.driver()?;
        d.service.scope_photo_data_url(&community_id, &scope_id, &d.files.join(COVER_DIR)).await.map_err(map_err)
    })
    .await
}

/// A section goes (the owner); `everyone` never.
#[tauri::command]
pub async fn channel_scope_del(community_id: String, scope_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_scope_del");
        let d = state.driver()?;
        let keys = d.keys()?;
        changed(&d, &community_id, d.service.scope_del(&keys, &community_id, &scope_id)).await?;
        detail(&d, &community_id).await
    })
    .await
}

#[tauri::command]
pub async fn channel_levels(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<LevelView>> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_levels");
        levels_of(&state.driver()?, &community_id).await
    })
    .await
}

/// A level made (no `id`: the lowest free, at most 63) or changed.
#[tauri::command]
pub async fn channel_level_set(community_id: String, level: LevelInput, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<LevelView>> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_level_set");
        let d = state.driver()?;
        let keys = d.keys()?;
        let can: LevelCan = level.can.into();
        changed(&d, &community_id, async { d.service.level_set(&keys, &community_id, level.id, &level.name, level.scopes.clone(), can).await.map(|(_, out)| out) }).await?;
        levels_of(&d, &community_id).await
    })
    .await
}

/// A level goes; the base level never.
#[tauri::command]
pub async fn channel_level_del(community_id: String, id: u8, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<LevelView>> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_level_del");
        let d = state.driver()?;
        let keys = d.keys()?;
        changed(&d, &community_id, d.service.level_del(&keys, &community_id, id)).await?;
        levels_of(&d, &community_id).await
    })
    .await
}

/// The levels of a person (moderator and above); the base level is always
/// among them, named or not.
#[tauri::command]
pub async fn channel_set_levels(community_id: String, who: String, levels: Vec<u8>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_set_levels");
        let d = state.driver()?;
        let keys = d.keys()?;
        let who = key(&who)?;
        changed(&d, &community_id, d.service.set_levels(&keys, &community_id, &who, &levels)).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// Asks the moderators for a level.
#[tauri::command]
pub async fn channel_level_request(community_id: String, level: u8, note: Option<String>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_level_request");
        let d = state.driver()?;
        let keys = d.keys()?;
        changed(&d, &community_id, d.service.level_request(&keys, &community_id, level, note.as_deref().unwrap_or(""))).await
    })
    .await
}

/// The requests for a level waiting for me as a moderator.
#[tauri::command]
pub async fn channel_level_requests(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<LevelRequestView>> {
    let _timed = crate::log::Timed::start("channel_level_requests");
    let rows = state.driver()?.service.level_requests(&community_id).await.map_err(map_err)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// A request for a level answered: granted (`set_levels`), or refused with
/// a reason.
#[tauri::command]
pub async fn channel_level_request_act(
    community_id: String,
    requester: String,
    level: u8,
    grant: bool,
    reason: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_level_request_act");
        let d = state.driver()?;
        let keys = d.keys()?;
        let requester: PubKey = key(&requester)?;
        changed(&d, &community_id, d.service.level_request_act(&keys, &community_id, &requester, level, grant, reason.as_deref())).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::as_core;

    /// The rights of a level travel the same both ways.
    #[test]
    fn rights_of_a_level_read_back() {
        let can = LevelCan { comment: false, discuss: true, post: true };
        let view: crate::views::LevelRights = can.into();
        assert_eq!(LevelCan::from(view), can);
        let input: LevelInput = as_core(serde_json::json!({ "name": "Gold", "scopes": [], "can": { "comment": true, "discuss": true, "post": false } })).unwrap();
        assert_eq!(input.id, None);
    }
}

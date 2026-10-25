// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The moderation of a channel and its keeper: the queue, the acts, the
//! log, the rules, the requests and the invitations, the state of the
//! keeper and an epoch now. Adapted from the groups' facade of moderation
//! (crates/messenger/runtime/src/moderation.rs at 1f2ef31d, kept in the
//! core as runtime/moderation.rs until this stage); the queue and the log
//! are told by `channel.queue` and `channel.moderation`, after which the
//! page reads them again.

use super::{as_core, bad, invite_view, key};
use crate::state::{map_err, CommunitiesState, Driver};
use crate::views::{word, ChannelAction, ChannelDetail, InviteStatusView, InviteView, KeeperStatus, TicketsView, LogEntry, LogPage, ModAction, ModSettings, QueueItem, QueuePage, ReaderView, ReadersPage, RequestView, ScheduleView};
use communities_core::db;
use communities_core::OpBody;
use veydan_core::{AppError, CmdResult};

/// A page when the page names none.
const PAGE: u32 = 50;

/// A request answered: an admission to the keeper and a copy to the
/// person, or a refusal.
pub(crate) async fn answer_request(d: &Driver, community_id: &str, requester: &str, approve: bool) -> CmdResult<()> {
    let keys = d.keys()?;
    let who = key(requester)?;
    let outcome = {
        let lock = d.service.lock_of(community_id).await;
        let _guard = lock.lock().await;
        d.service.request_act(&keys, community_id, &who, approve).await.map_err(map_err)?
    };
    d.apply(outcome).await;
    Ok(())
}

/// What pre-moderation holds, one line per author, newest first; with
/// `author`, every message of that author, one line each (`total`: how
/// many of theirs wait), for «approve all from the author».
#[tauri::command]
pub async fn channel_moderation_queue(
    community_id: String,
    author: Option<String>,
    before: Option<i64>,
    limit: Option<u32>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<QueuePage> {
    let _timed = crate::log::Timed::start("channel_moderation_queue");
    let d = state.driver()?;
    let keys = d.keys()?;
    let author = author.as_deref().map(key).transpose()?;
    let limit = limit.unwrap_or(PAGE).clamp(1, 500);
    let (lines, total) = d.service.queue(&keys, &community_id, author.as_ref().map(|a| a.as_hex()), before, limit as usize).await.map_err(map_err)?;
    let next_before = lines.last().filter(|_| lines.len() == limit as usize).map(|l| l.created_at);
    let mut items = Vec::with_capacity(lines.len());
    for line in lines {
        let text = db::content::get(d.service.db(), &line.id).await.map_err(map_err)?.and_then(|m| m.text);
        items.push(QueueItem::of(line, text));
    }
    Ok(QueuePage { items, next_before, total: u32::try_from(total).unwrap_or(u32::MAX) })
}

/// A page of the readers of a channel, for its team only (the owner's
/// decision, 2026-10-25; `ChannelService::readers`): newest first, `query`
/// a start of the hex key. A shard of the snapshot not here is wanted of
/// the hosts by a job started now, and the page says `partial` until it
/// came: the page asks again.
#[tauri::command]
pub async fn channel_readers(
    community_id: String,
    query: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<ReadersPage> {
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_readers");
        let d = state.driver()?;
        let keys = d.keys()?;
        let limit = limit.unwrap_or(READERS_PAGE).clamp(1, communities_core::reader_list::MAX_PAGE as u32);
        let page = d.service.readers(&keys, &community_id, query.as_deref(), offset.unwrap_or(0) as usize, limit as usize).await.map_err(map_err)?;
        if page.partial {
            // The shards it wanted, fetched off this command.
            d.apply(communities_core::service::Outcome::default()).await;
        }
        Ok(ReadersPage {
            readers: page.readers.into_iter().map(ReaderView::from).collect(),
            total: u32::try_from(page.total).unwrap_or(u32::MAX),
            partial: page.partial,
        })
    })
    .await
}

/// Readers a page holds when the page names no number.
const READERS_PAGE: u32 = 100;

/// One act of moderation: hide, unhide, approve, reject, spam, mute,
/// unmute, trust.
#[tauri::command]
pub async fn channel_moderation_mod(community_id: String, action: ModAction, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_moderation_mod");
        let d = state.driver()?;
        let keys = d.keys()?;
        let action: communities_core::ModAction = as_core(action)?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.moderate(&keys, &community_id, action).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        Ok(())
    })
    .await
}

/// What was decided in the channel, newest first.
#[tauri::command]
pub async fn channel_moderation_log(
    community_id: String,
    before: Option<i64>,
    limit: Option<u32>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<LogPage> {
    let _timed = crate::log::Timed::start("channel_moderation_log");
    let d = state.driver()?;
    let keys = d.keys()?;
    let limit = limit.unwrap_or(PAGE).clamp(1, 500);
    let lines = d.service.mod_log(&keys, &community_id, before, limit as usize).await.map_err(map_err)?;
    // A full page may carry more lines of its last second than asked.
    let next_before = lines.last().filter(|_| lines.len() >= limit as usize).map(|l| l.at);
    let mut items = Vec::with_capacity(lines.len());
    for line in lines {
        let mut entry = LogEntry::from(line);
        entry.excerpt = excerpt(&d, &community_id, &entry.target).await;
        items.push(entry);
    }
    Ok(LogPage { items, next_before })
}

/// The longest start of a post's words a line of the log shows.
const EXCERPT_CHARS: usize = 80;

/// The start of the words of a post or a comment of this channel, its first
/// line cut to `EXCERPT_CHARS`; none for a person, a file without words, a
/// removed message, or one of another channel that names the same id.
async fn excerpt(d: &Driver, community_id: &str, target: &str) -> Option<String> {
    if target.len() != 64 {
        return None;
    }
    let row = db::content::get(d.service.db(), target).await.ok().flatten()?;
    if row.community_id != community_id || row.deleted {
        return None;
    }
    let line = row.text?.lines().map(str::trim).find(|l| !l.is_empty())?.to_string();
    Some(match line.char_indices().nth(EXCERPT_CHARS) {
        // Cut on a character; the page ellipsizes what it cannot fit.
        Some((cut, _)) => line[..cut].trim_end().to_string(),
        None => line,
    })
}

/// The rules of the channel as the journal holds them now.
#[tauri::command]
pub async fn channel_moderation_settings(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ModSettings> {
    let _timed = crate::log::Timed::start("channel_moderation_settings");
    let d = state.driver()?;
    let log = d.service.log(&community_id).await.map_err(map_err)?.ok_or_else(|| bad("channel_unknown", "unknown channel"))?;
    let s = log.state();
    Ok(ModSettings {
        post_mode: Some(word(&s.post_mode)),
        comments: Some(word(&s.comments)),
        premod: Some(word(&s.premod)),
        schedule: s.schedule.as_ref().map(|s| ScheduleView { tz: s.tz.clone(), days: s.days.clone(), from: s.from.clone(), to: s.to.clone() }),
        join: Some(word(&s.join)),
        history: Some(word(&s.history)),
    })
}

/// New rules, as one `settings` of the journal; the rules as they are then.
#[tauri::command]
pub async fn channel_moderation_settings_set(
    community_id: String,
    settings: ModSettings,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<ModSettings> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_moderation_settings_set");
        let d = state.driver()?;
        let keys = d.keys()?;
        // A word the journal does not know is refused here, before anything is signed.
        let body: OpBody = as_core(ChannelAction::Settings {
            name: None,
            about: None,
            history: settings.history,
            relays: None,
            storage: None,
            // Posts are by authors, not a setting (wire §3, 2026-10-22).
            post_mode: None,
            premod: settings.premod,
            schedule: settings.schedule,
            comments: settings.comments,
            join: settings.join,
        })?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.act(&keys, &community_id, body).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        channel_moderation_settings(community_id, state).await
    })
    .await
}

/// Says again, as me, what a demoted moderator decided; how many notes went.
#[tauri::command]
pub async fn channel_moderation_inherit(community_id: String, from: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<u32> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_moderation_inherit");
        let d = state.driver()?;
        let keys = d.keys()?;
        let from = key(&from)?;
        let (sent, outcome) = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.inherit_marks(&keys, &community_id, &from).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        Ok(sent)
    })
    .await
}

/// An invitation of mine to a person, against the limits of groups.
#[tauri::command]
pub async fn channel_moderation_invite_create(community_id: String, who: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<InviteView> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_moderation_invite_create");
        let d = state.driver()?;
        let keys = d.keys()?;
        let who = key(&who)?;
        let (invite_id, outcome) = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.invite_create(&keys, &community_id, &who).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        invite_view(&d, &invite_id).await
    })
    .await
}

/// The invitations I made in a channel, newest first.
#[tauri::command]
pub async fn channel_moderation_invite_list(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<InviteView>> {
    let _timed = crate::log::Timed::start("channel_moderation_invite_list");
    let rows = state.driver()?.service.invite_list(&community_id).await.map_err(map_err)?;
    Ok(rows.into_iter().map(InviteView::from).collect())
}

/// Takes back an invitation not answered yet.
#[tauri::command]
pub async fn channel_moderation_invite_revoke(invite_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    let _timed = crate::log::Timed::start("channel_moderation_invite_revoke");
    let d = state.driver()?;
    let outcome = d.service.invite_revoke(&invite_id).await.map_err(map_err)?;
    d.apply(outcome).await;
    Ok(())
}

#[tauri::command]
pub async fn channel_moderation_invite_status(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<InviteStatusView> {
    let _timed = crate::log::Timed::start("channel_moderation_invite_status");
    let status = state.driver()?.service.invite_status(&community_id).await.map_err(map_err)?;
    Ok(status.into())
}

/// Whom the channel bans, hex pubkeys.
#[tauri::command]
pub async fn channel_moderation_blocked(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<String>> {
    let _timed = crate::log::Timed::start("channel_moderation_blocked");
    let d = state.driver()?;
    let log = d.service.log(&community_id).await.map_err(map_err)?.ok_or_else(|| bad("channel_unknown", "unknown channel"))?;
    Ok(log.state().banned.iter().cloned().collect())
}

/// The requests waiting in a channel (for moderators).
#[tauri::command]
pub async fn channel_moderation_requests(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<RequestView>> {
    let _timed = crate::log::Timed::start("channel_moderation_requests");
    let d = state.driver()?;
    let keys = d.keys()?;
    let rows = d.service.requests(&keys, &community_id).await.map_err(map_err)?;
    Ok(rows.into_iter().map(RequestView::from).collect())
}

#[tauri::command]
pub async fn channel_moderation_request_act(
    community_id: String,
    requester: String,
    approve: bool,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_moderation_request_act");
        answer_request(&state.driver()?, &community_id, &requester, approve).await
    })
    .await
}

#[tauri::command]
pub async fn channel_moderation_requests_count(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<u32> {
    let _timed = crate::log::Timed::start("channel_moderation_requests_count");
    let n = state.driver()?.service.requests_count(&community_id).await.map_err(map_err)?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

// ─── The keeper ─────────────────────────────────────────────────────────────

/// What the keeper's last turn left (`ChannelService::keeper_status`):
/// never the keeper's lock, which a turn holds across its uploads, so the
/// page is answered at once whatever the keeper is doing.
async fn keeper_status(d: &Driver, community_id: &str) -> CmdResult<KeeperStatus> {
    let log = d.service.log(community_id).await.map_err(map_err)?;
    let keeper = log.as_ref().and_then(|l| l.state().keeper().map(|k| k.pubkey.as_hex().to_string()));
    let me = d.service.signer().map(|k| k.public_key().to_hex());
    let owner = log.as_ref().is_some_and(|l| me.as_deref() == Some(l.state().owner.as_hex()));
    // A keeper the journal has gone past is on its way out (wire §15): not kept here any more.
    let handing = d.service.keeper_handing(community_id).await.map_err(map_err)?;
    let kept_here = db::keeper::keeper(d.service.db(), community_id).await.map_err(map_err)?.is_some() && !handing;
    let take = d.service.keeper_take_status(community_id).await.map_err(map_err)?;
    let has_root = owner && d.service.holds_root(community_id).await.map_err(map_err)?;
    let next_wake = d.next_wake.read().map(|w| *w).unwrap_or(None).and_then(|w| i64::try_from(w).ok());
    let st = d.service.keeper_status(community_id);
    Ok(KeeperStatus {
        kept_here,
        online: kept_here && d.service.signer().is_some(),
        keeper,
        epoch: st.as_ref().and_then(|s| s.epoch),
        pending_epoch: st.as_ref().and_then(|s| s.pending_epoch),
        last_epoch_at: st.as_ref().and_then(|s| s.last_epoch_at),
        next_wake: next_wake.filter(|_| kept_here),
        readers: st.as_ref().map_or(0, |s| s.readers),
        generations: st.as_ref().map_or(0, |s| s.generations),
        garbage: st.as_ref().map_or(0, |s| s.garbage),
        rotate_asked: st.as_ref().is_some_and(|s| s.rotate_asked),
        background: kept_here && d.background.supported(),
        keep_on: kept_here && d.background.keep_on(community_id),
        background_running: kept_here && d.background.running(),
        owner,
        taking: take.as_ref().map(|t| t.mode.clone()),
        taking_since: take.as_ref().map(|t| t.started_at),
        handing,
        has_root,
        storage_error: st.as_ref().and_then(|s| s.storage_error.clone()).filter(|_| kept_here),
    })
}

/// The keeper of a channel: kept here or not, its epochs, its next turn.
#[tauri::command]
pub async fn channel_keeper_status(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<KeeperStatus> {
    let _timed = crate::log::Timed::start("channel_keeper_status");
    keeper_status(&state.driver()?, &community_id).await
}

/// The owner's switch «Держать хранителя включённым» of a channel kept
/// here, on a phone: its keeper's service runs as long as it is on
/// (internal/communities-stage1.md §4). Kept on this device, per channel.
#[tauri::command]
pub async fn channel_keeper_keep_on(community_id: String, on: bool, state: tauri::State<'_, CommunitiesState>) -> CmdResult<KeeperStatus> {
    let _timed = crate::log::Timed::start("channel_keeper_keep_on");
    let d = state.driver()?;
    if db::keeper::keeper(d.service.db(), &community_id).await.map_err(map_err)?.is_none() {
        return Err(AppError::Other(format!("channel_not_permitted: the keeper of {community_id} is not on this device")));
    }
    d.background.set_keep_on(&community_id, on).map_err(|e| AppError::Other(format!("the switch was not kept: {e}")))?;
    d.background_soon(false);
    keeper_status(&d, &community_id).await
}

/// «Сделать хранителем это устройство» (internal/communities-wire.md §15):
/// the owner takes the keeper over from his other device, which hands its
/// state over; `emergency`: that device is lost, a new generation starts
/// here from the readers of the snapshot.
#[tauri::command]
pub async fn channel_keeper_take(community_id: String, emergency: Option<bool>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<KeeperStatus> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_keeper_take");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.keeper_take(&keys, &community_id, emergency.unwrap_or(false)).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        d.wake_now();
        keeper_status(&d, &community_id).await
    })
    .await
}

/// «Передать канал» (the owner, internal/communities-wire.md §16): the
/// journal names the new owner and he is offered the channel; in force once
/// he accepts.
#[tauri::command]
pub async fn channel_transfer(community_id: String, to: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_transfer");
        let d = state.driver()?;
        let keys = d.keys()?;
        let to = key(&to)?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.transfer(&keys, &community_id, &to).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        super::detail(&d, &community_id).await
    })
    .await
}

/// The answer to «Вам передают канал»: accepted, this device becomes the
/// owner's keeper (the former owner's hands its state over); declined, the
/// former owner is told and nothing changes.
#[tauri::command]
pub async fn channel_transfer_answer(community_id: String, accept: bool, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_transfer_answer");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.transfer_answer(&keys, &community_id, accept).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        d.wake_now();
        super::detail(&d, &community_id).await
    })
    .await
}

/// The tickets I hold as a moderator of a channel (communities-wire §17).
#[tauri::command]
pub async fn channel_tickets(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<TicketsView> {
    let _timed = crate::log::Timed::start("channel_tickets");
    let t = state.driver()?.service.tickets_held(&community_id).await.map_err(map_err)?;
    Ok(TicketsView { held: t.held, max: t.max })
}

/// A moderator asks the keeper for `n` tickets (at most 50 held at once;
/// `request` and `invite` channels only). They come by a note, later.
#[tauri::command]
pub async fn channel_tickets_request(community_id: String, n: u32, state: tauri::State<'_, CommunitiesState>) -> CmdResult<TicketsView> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_tickets_request");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = d.service.tickets_request(&keys, &community_id, n).await.map_err(map_err)?;
        d.apply(outcome).await;
        let t = d.service.tickets_held(&community_id).await.map_err(map_err)?;
        Ok(TicketsView { held: t.held, max: t.max })
    })
    .await
}

/// An epoch now: built here when this device keeps the channel, else
/// asked of the keeper by an operation of the journal (an admin's).
#[tauri::command]
pub async fn channel_rotate_now(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<KeeperStatus> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_rotate_now");
        let d = state.driver()?;
        let keys = d.keys()?;
        // A mark for the keeper's next turn, never its lock: a turn in progress
        // would hold the command for as long as its uploads take.
        match d.service.rotate_now(&community_id).await.map_err(map_err)? {
            true => d.wake_now(),
            false => {
                let outcome = {
                    let lock = d.service.lock_of(&community_id).await;
                    let _guard = lock.lock().await;
                    d.service.act(&keys, &community_id, OpBody::RotateNow { scope_id: None }).await.map_err(map_err)?
                };
                d.apply(outcome).await;
            }
        }
        keeper_status(&d, &community_id).await
    })
    .await
}

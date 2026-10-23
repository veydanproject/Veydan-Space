// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The commands of the channels (internal/communities-stage1.md §6.1):
//! thin calls of `ChannelService`, each on the channel's loc where it
//! changes the channel, with what it led to carried out by the driver.
//! Adapted from the groups' commands of the messenger's runtime
//! (crates/messenger/runtime/src/groups.rs at 1f2ef31d, kept in the core
//! as runtime/groups.rs until this stage).

pub mod levels;
pub mod moderation;

use crate::state::{map_err, take_journal, CommunitiesState, Driver, JOURNAL_WAIT, PREVIEW_WAIT};
use crate::views::{chat_id, ChannelAction, ChannelDetail, ChannelPreview, ChannelView, FeedCursorView, FeedPageView, InviteView, LastPost, NewChannelInput, StoragePlanView, TransferView, UploadView, OUTGOING_DIR};
use communities_core::db::{self, channels::ChannelRow};
use communities_core::link::ChannelLink;
use communities_core::media::{picked_preview, NewFile, NewRecording, Progress, RecordingPlace, Sink, MAX_COVER_INPUT_BYTES, UPLOAD_CANCELLED, UPLOAD_PAUSED};
use communities_core::service::{NewChannel, MEMBERSHIP_JOINED};
use communities_core::transport::FeedCursor;
use communities_core::state::SCOPE_EVERYONE;
use communities_core::{qr, wire, OpBody};
use messenger_core::outbound::Filter;
use messenger_core::{Outbound, PubKey, Scope, TextFormat};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::io::Read;
use veydan_core::{AppError, CmdResult};
use veydan_shell::seams::{ChatPref, MediaUrls};
use veydan_shell::Shell;

/// Every status an invitation is kept with.
const INVITE_STATUSES: [&str; 6] = ["sent", "accepted", "declined", "done", "expired", "cancelled"];

/// The folder of the module the files of messages are fetched into.
const MEDIA_DIR: &str = "media";
/// The covers of channels, kept by the hash of the picture, and the photos
/// of sections (`scope-<hash>.jpg`).
pub(crate) const COVER_DIR: &str = "covers";

pub(crate) fn bad(code: &str, what: impl std::fmt::Display) -> AppError {
    AppError::Other(format!("{code}: {what}"))
}

pub(crate) fn key(s: &str) -> CmdResult<PubKey> {
    PubKey::parse(s.trim()).ok_or_else(|| bad("channel_bad_key", "expected a 64-hex public key"))
}

/// A value of the page as the core's type of the same serde shape.
pub(crate) fn as_core<T: DeserializeOwned>(value: impl serde::Serialize) -> CmdResult<T> {
    serde_json::to_value(value).and_then(serde_json::from_value).map_err(|e| bad("channel_invalid", e))
}

/// A word of the page (`open`, `from_join`, …) as a value of the core.
pub(crate) fn from_word<T: DeserializeOwned>(word: &str) -> CmdResult<T> {
    serde_json::from_value(serde_json::Value::String(word.to_string())).map_err(|_| bad("channel_invalid", format!("unknown word: {word}")))
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// My key, as the core takes it; none before a vault is open.
fn me_key(d: &Driver) -> Option<PubKey> {
    d.service.signer().and_then(|k| PubKey::parse(&k.public_key().to_hex()))
}

/// The newest post of the channel I may read (`ChannelService::last_posts`).
async fn last_post(d: &Driver, row: &ChannelRow) -> CmdResult<Option<LastPost>> {
    let mut lasts = d.service.last_posts(me_key(d).as_ref(), std::slice::from_ref(row)).await.map_err(map_err)?;
    Ok(lasts.remove(&row.id).map(last_of))
}

fn last_of(m: communities_core::db::content::MessageRow) -> LastPost {
    // A voice message or a circle is told by its kind, as a chat's row tells it.
    let media_kind = m
        .media_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .filter(|k| k == "voice" || k == "circle");
    LastPost { has_media: m.media_json.is_some(), media_kind, id: m.id, author: m.author, text: m.text.filter(|_| !m.deleted), at: m.created_at }
}

/// The row of a channel as the list shows it; `unread`, when the list read
/// them all at once.
pub(crate) async fn view_of(d: &Driver, row: ChannelRow, prefs: &HashMap<String, ChatPref>, unread: Option<i64>) -> CmdResult<ChannelView> {
    let db = d.service.db();
    let kept = db::keeper::keeper(db, &row.id).await.map_err(map_err)?.is_some();
    let unread = match unread {
        Some(n) => n,
        None => {
            let me = d.service.signer().map(|k| k.public_key().to_hex()).unwrap_or_default();
            db::content::unread(db, &row.id, &me).await.map_err(map_err)?
        }
    };
    let last = last_post(d, &row).await?;
    let prefs = prefs.get(&chat_id(&row.id));
    Ok(ChannelView::of(row, kept, unread, prefs, last))
}

pub(crate) async fn row(d: &Driver, community_id: &str) -> CmdResult<ChannelRow> {
    db::channels::get(d.service.db(), community_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| bad("channel_unknown", "unknown channel"))
}

pub(crate) async fn detail(d: &Driver, community_id: &str) -> CmdResult<ChannelDetail> {
    let row = row(d, community_id).await?;
    let log = d.service.log(community_id).await.map_err(map_err)?.ok_or_else(|| bad("channel_unknown", "no journal"))?;
    let names = d.service.names(&log).await.map_err(map_err)?;
    let state = log.state().clone();
    let me = d.service.signer().and_then(|k| PubKey::parse(&k.public_key().to_hex()));
    let has_key = match d.service.signer() {
        Some(keys) => d.service.member(&keys).newest_key(community_id, SCOPE_EVERYONE).await.map_err(map_err)?.is_some(),
        None => false,
    };
    let waiting = db::channels::count_pending(d.service.db(), community_id).await.map_err(map_err)?;
    let waiting = u32::try_from(waiting).unwrap_or(u32::MAX);
    // The link is handed on by those in the channel.
    let link = if row.membership == MEMBERSHIP_JOINED { d.service.share_link(community_id).await.ok() } else { None };
    let unread = db::content::unread_by_scope(d.service.db(), community_id, me.as_ref().map_or("", |m| m.as_hex())).await.map_err(map_err)?;
    let view = view_of(d, row, &crate::seams::chat_prefs(d).await, None).await?;
    let mut detail = ChannelDetail::of(view, &state, has_key, waiting, link, &names, me.as_ref(), &unread);
    detail.transfer = d.service.transfer_of(community_id).await.map_err(map_err)?.map(|t| TransferView { direction: t.direction, peer: t.peer, status: t.status, created_at: t.created_at });
    detail.outbox = Some(d.service.outbox_summary(community_id).await.map_err(map_err)?.into());
    Ok(detail)
}

/// What waits for the relays in a channel is tried again now, not at its
/// time (the user pressed «Отправить снова»).
#[tauri::command]
pub async fn channel_outbox_retry(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_outbox_retry");
        let d = state.driver()?;
        d.service.outbox_retry(&community_id).await.map_err(map_err)?;
        detail(&d, &community_id).await
    })
    .await
}

/// Asks the relay of a link for the journal and the card of a channel not
/// here yet, and takes the journal as it comes, page after page down to
/// its `create` (`ChannelService::ask_journal`; a relay hands back 500
/// events a REQ), past the queue of the loop (`state::take_journal`).
async fn fetch_journal(d: &Driver, community_id: &str) -> CmdResult<()> {
    for outcome in take_journal(&d.service, &d.previews, community_id, JOURNAL_WAIT).await? {
        d.apply(outcome).await;
    }
    Ok(())
}

/// Reads at most `max` bytes of a picked file; more is refused.
pub(crate) fn read_picked(app: &tauri::AppHandle, src: &str, max: usize, too_large: &str) -> CmdResult<Vec<u8>> {
    use tauri::Manager;
    let source = app.state::<Shell>().open_source(src)?;
    let file = (source.open)().map_err(AppError::io)?;
    let mut out = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut out).map_err(AppError::io)?;
    if out.len() > max {
        return Err(bad(too_large, "the file is too large"));
    }
    Ok(out)
}

// ─── Channels ───────────────────────────────────────────────────────────────

/// Every channel known here, as the list of chats shows it.
#[tauri::command]
pub async fn channels_list(state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<ChannelView>> {
    let _timed = crate::log::Timed::start("channels_list");
    let d = state.driver()?;
    let prefs = crate::seams::chat_prefs(&d).await;
    let unread = { let me = d.service.signer().map(|k| k.public_key().to_hex()).unwrap_or_default(); db::content::unread_all(d.service.db(), &me).await }.map_err(map_err)?;
    // The keepers and the last posts of all the channels at once: three
    // queries for the list, whatever the number of channels (and the
    // journal of one whose newest post is of a section: `last_posts`).
    let db = d.service.db();
    let rows = db::channels::list(db).await.map_err(map_err)?;
    let kept: std::collections::HashSet<String> = db::keeper::kept(db).await.map_err(map_err)?.into_iter().collect();
    let mut lasts = d.service.last_posts(me_key(&d).as_ref(), &rows).await.map_err(map_err)?;
    let mut views: Vec<ChannelView> = rows
        .into_iter()
        .map(|row| {
            let n = unread.get(&row.id).copied().unwrap_or(0);
            let is_kept = kept.contains(&row.id);
            let last = lasts.remove(&row.id).map(last_of);
            let pref = prefs.get(&chat_id(&row.id)).cloned();
            ChannelView::of(row, is_kept, n, pref.as_ref(), last)
        })
        .collect();
    // By the last post, newest first, as the list of chats shows them.
    views.sort_by(|a, b| b.last_at.cmp(&a.last_at).then_with(|| a.id.cmp(&b.id)));
    Ok(views)
}

#[tauri::command]
pub async fn channel_get(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_get");
        detail(&state.driver()?, &community_id).await
    })
    .await
}

/// A new channel of mine; its keeper runs on this device.
#[tauri::command]
pub async fn channel_create(input: NewChannelInput, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_create");
        let d = state.driver()?;
        let keys = d.keys()?;
        let new = NewChannel {
            name: input.name.trim().to_string(),
            about: input.about.unwrap_or_default(),
            picture: String::new(),
            join: from_word(&input.join)?,
            history: from_word(input.history.as_deref().unwrap_or("open"))?,
            relays: input.relays,
            storage: input.storage,
        };
        if new.name.is_empty() || new.relays.is_empty() || new.storage.is_empty() {
            return Err(bad("channel_invalid", "a name, a relay and a host are needed"));
        }
        let relays = new.relays.iter().filter_map(|r| messenger_core::RelayUrl::parse(r)).collect();
        // The id is the core's to make: what the outbox sends of it before its
        // row is known to the routes goes to the relays of the new channel,
        // and the row is known before the guard goes.
        let (cid, outcome) = {
            let _creating = d.routes.creating(relays).await;
            let made = d.service.create(&keys, new).await.map_err(map_err)?;
            d.refresh_routes().await;
            made
        };
        d.apply(outcome).await;
        detail(&d, &cid).await
    })
    .await
}

/// Where the files of a new channel of mine live, by the account: the
/// free public media of the messenger without Veydan Cloud (as a chat file
/// of such an account, 14 days), the account's own with it.
#[tauri::command]
pub async fn channel_storage_plan(app: tauri::AppHandle) -> CmdResult<StoragePlanView> {
    let _timed = crate::log::Timed::start("channel_storage_plan");
    let days = match veydan_shell::seams::seam::<dyn veydan_shell::seams::CloudPlan, _, _>(&app) {
        Some(plan) => plan.media_retention_days().await.unwrap_or(None),
        None => None,
    };
    Ok(storage_plan(days))
}

/// The plan by how long the account's media keeps a file (`None`: no
/// account of Veydan Cloud; `Some(0)`: no limit).
pub(crate) fn storage_plan(days: Option<u64>) -> StoragePlanView {
    let free_days = veydan_shell::seams::FREE_MEDIA_RETENTION_DAYS;
    let days = days.unwrap_or(free_days);
    StoragePlanView { free: days != 0 && days <= free_days, retention_days: u32::try_from(days).unwrap_or(u32::MAX) }
}

/// An invitation of mine to a person (a moderator and above).
#[tauri::command]
pub async fn channel_invite(community_id: String, who: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<InviteView> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_invite");
        let d = state.driver()?;
        let keys = d.keys()?;
        let who = key(&who)?;
        let (invite_id, outcome) = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.invite(&keys, &community_id, &who).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        invite_view(&d, &invite_id).await
    })
    .await
}

pub(crate) async fn invite_view(d: &Driver, invite_id: &str) -> CmdResult<InviteView> {
    db::channels::invite(d.service.db(), invite_id)
        .await
        .map_err(map_err)?
        .map(InviteView::from)
        .ok_or_else(|| bad("channel_invite_unknown", "unknown invitation"))
}

/// `in`: invitations for me; `out`: those I sent.
#[tauri::command]
pub async fn channel_invites(direction: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<InviteView>> {
    let _timed = crate::log::Timed::start("channel_invites");
    let d = state.driver()?;
    let db = d.service.db();
    db::channels::expire_invites(db, now()).await.map_err(map_err)?;
    let rows = db::channels::invites(db, &direction, &INVITE_STATUSES).await.map_err(map_err)?;
    Ok(rows.into_iter().map(InviteView::from).collect())
}

/// My answer to an invitation that came; a yes fetches the journal of the
/// channel from the relay the invitation names.
#[tauri::command]
pub async fn channel_answer_invite(invite_id: String, accept: bool, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_answer_invite");
        let d = state.driver()?;
        d.keys()?;
        let row = db::channels::invite(d.service.db(), &invite_id).await.map_err(map_err)?.ok_or_else(|| bad("channel_invite_unknown", "unknown invitation"))?;
        let mut route = None;
        if accept {
            let payload: serde_json::Value = serde_json::from_str(&row.payload_json).unwrap_or_default();
            if let Some(relay) = payload["relay"].as_str().and_then(messenger_core::RelayUrl::parse) {
                route = Some(d.routes.hold(&row.community_id, relay));
            }
            if d.service.log(&row.community_id).await.map_err(map_err)?.is_none() {
                fetch_journal(&d, &row.community_id).await?;
            }
        }
        let outcome = d.service.answer_invite(&invite_id, accept).await.map_err(map_err)?;
        if let Some(route) = route {
            route.keep();
        }
        d.apply(outcome).await;
        Ok(())
    })
    .await
}

/// Opens a `veydan://channel/…` link: by its secret a join to the keeper,
/// without one a request to the moderators (or nothing, for a channel by
/// invitation). The journal is fetched from the relay of the link first.
#[tauri::command]
pub async fn channel_open_link(link: String, note: Option<String>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_open_link");
        let d = state.driver()?;
        let keys = d.keys()?;
        // The grammar of links is the messenger's: what it does not take for a
        // channel's is no link of a channel.
        messenger_links::ChannelLink::parse(link.trim()).map_err(|e| bad("channel_bad_link", e.code()))?;
        let parsed = ChannelLink::parse(link.trim()).map_err(map_err)?;
        let cid = parsed.community_id.clone();
        // Held while it is opened; a failure takes it away again.
        let route = d.routes.hold(&cid, parsed.relay.clone());
        // Only the `create` of the link's owner is taken for this id: a
        // journal a forger put first is dropped before it is read.
        d.service.expect_owner(&cid, &parsed.owner).await.map_err(map_err)?;
        if d.service.log(&cid).await.map_err(map_err)?.is_none() {
            fetch_journal(&d, &cid).await?;
        }
        let outcome = {
            let lock = d.service.lock_of(&cid).await;
            let _guard = lock.lock().await;
            d.service.open_link(&keys, link.trim(), note.as_deref().unwrap_or("")).await.map_err(map_err)?
        };
        route.keep();
        d.apply(outcome).await;
        detail(&d, &cid).await
    })
    .await
}

/// The card of the channel of a `veydan://channel/…` link, without joining:
/// asked of the link's relay (routed through the pool as the relays of
/// every channel are) and taken only when the link's owner (`o`) signed
/// it. A relay that does not answer within `PREVIEW_WAIT`, or holds no
/// such card, is `channel_preview_unreachable`.
#[tauri::command]
pub async fn channel_preview_link(link: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelPreview> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_preview_link");
        let d = state.driver()?;
        messenger_links::ChannelLink::parse(link.trim()).map_err(|e| bad("channel_bad_link", e.code()))?;
        let parsed = ChannelLink::parse(link.trim()).map_err(map_err)?;
        let cid = parsed.community_id.clone();
        // As for opening: whatever journal comes for this id later is taken
        // from the link's owner only.
        d.service.expect_owner(&cid, &parsed.owner).await.map_err(map_err)?;
        let here = db::channels::get(d.service.db(), &cid).await.map_err(map_err)?;
        let (_watching, mut cards) = d.previews.watch(&cid);
        // Only while it is looked at: an opening of the same link meanwhile
        // holds the relay on its own.
        let _route = d.routes.hold(&cid, parsed.relay.clone());
        let filter = serde_json::json!({ "kinds": [wire::KIND_CARD], "#d": [&cid], "authors": [parsed.owner.as_hex()] });
        let deadline = tokio::time::Instant::now() + PREVIEW_WAIT;
        // Right after a start the pool may have no relay yet: asked again
        // until it has, within the wait, not refused at once.
        let asked = loop {
            let asked = d.net.send(Outbound::Sync { scope: Scope::Channel { id: cid.clone() }, filter: Filter(filter.clone()), local: vec![] }).await;
            let again = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            if asked.is_ok() || again >= deadline {
                break asked;
            }
            tokio::time::sleep_until(again).await;
        };
        let mut card = None;
        if asked.is_ok() {
            // The relay answered: what it sent is on its way to here already.
            let settle = deadline.min(tokio::time::Instant::now() + std::time::Duration::from_secs(2));
            while let Ok(Some(signed)) = tokio::time::timeout_at(settle, cards.recv()).await {
                // Anybody may put a card under the id: the owner of the link signs the one shown.
                if let Ok(c) = wire::verify_card(&signed, &parsed.owner) {
                    card = Some(c);
                    break;
                }
            }
        }
        match (card, asked) {
            (Some(card), _) => Ok(ChannelPreview::of(card, &parsed, here)),
            (None, Err(e)) => Err(bad("channel_preview_unreachable", e)),
            (None, Ok(_)) => Err(bad("channel_preview_unreachable", "the relay of the link did not answer with the card")),
        }
    })
    .await
}

/// A request to be let in, answered: an admission or a refusal.
#[tauri::command]
pub async fn channel_answer_request(
    community_id: String,
    requester: String,
    approve: bool,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_answer_request");
        let d = state.driver()?;
        moderation::answer_request(&d, &community_id, &requester, approve).await?;
        detail(&d, &community_id).await
    })
    .await
}

/// An operation of the journal (settings, roles, bans, mutes, pins, the
/// handover, disbanding), or `leave`.
#[tauri::command]
pub async fn channel_act(community_id: String, action: ChannelAction, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_act");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            match action {
                ChannelAction::Leave => d.service.leave(&keys, &community_id).await,
                other => d.service.act(&keys, &community_id, as_core::<OpBody>(other)?).await,
            }
            .map_err(map_err)?
        };
        d.apply(outcome).await;
        detail(&d, &community_id).await
    })
    .await
}

/// A picked picture (up to `MAX_COVER_INPUT_BYTES`) as the cover of a
/// channel I manage, made a square JPEG as the picture of a group is.
#[tauri::command]
pub async fn channel_avatar_set(
    community_id: String,
    src: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_avatar_set");
        let d = state.driver()?;
        let keys = d.keys()?;
        let bytes = read_picked(&app, &src, MAX_COVER_INPUT_BYTES, "channel_cover_too_large")?;
        // The hosts are asked off the channel's lock: the events of the channel
        // are heard meanwhile (живой запуск 2026-10-23: 12 s under the lock).
        let cover = d.service.upload_cover(&keys, &community_id, &bytes).await.map_err(map_err)?;
        // Kept where `channel_cover` reads it: the page shows the new cover at
        // once, with nothing fetched back from a host.
        let dir = d.files.join(COVER_DIR);
        if tokio::fs::create_dir_all(&dir).await.is_ok() {
            let target = dir.join(format!("cover-{}.jpg", cover.sha));
            let part = target.with_extension("part");
            if tokio::fs::write(&part, &cover.jpeg).await.is_ok() {
                let _ = tokio::fs::rename(&part, &target).await;
            }
        }
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.apply_cover(&keys, &community_id, cover).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        detail(&d, &community_id).await
    })
    .await
}

#[tauri::command]
pub async fn channel_avatar_remove(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_avatar_remove");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.remove_cover(&keys, &community_id).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        detail(&d, &community_id).await
    })
    .await
}

/// A new link: the old one admits nobody from now on.
#[tauri::command]
pub async fn channel_rotate_link(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ChannelDetail> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_rotate_link");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            let epoch = u32::try_from(row(&d, &community_id).await?.link_epoch).unwrap_or(0) + 1;
            d.service.act(&keys, &community_id, OpBody::RotateLink { link_epoch: epoch }).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        detail(&d, &community_id).await
    })
    .await
}

/// The link of the channel as a QR code (SVG).
#[tauri::command]
pub async fn channel_link_qr(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<String> {
    let _timed = crate::log::Timed::start("channel_link_qr");
    let d = state.driver()?;
    let link = d.service.share_link(&community_id).await.map_err(map_err)?;
    qr::link_svg(&link).map_err(map_err)
}

/// Removes from this device a channel I am no longer in.
#[tauri::command]
pub async fn channel_forget(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_forget");
        let d = state.driver()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.forget(&community_id).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        Ok(())
    })
    .await
}

// ─── The feed ───────────────────────────────────────────────────────────────

/// Where a page starts: the cursor `next` of the page before; else the date
/// alone (`before`, taken for one more release from a page that still
/// sends it, though items of one second at its edge may be skipped).
pub(crate) fn cursor_of(next: Option<FeedCursorView>, before: Option<i64>) -> Option<FeedCursor> {
    match (next, before) {
        (Some(c), _) => Some(FeedCursor { created_at: c.created_at, id: c.id }),
        (None, Some(b)) => Some(FeedCursor { created_at: b, id: String::new() }),
        (None, None) => None,
    }
}

/// A page of posts before `next` (the cursor the page before answered
/// with): the newest of them, oldest first, as a chat reads.
#[tauri::command]
pub async fn channel_history(
    community_id: String,
    limit: Option<u32>,
    next: Option<FeedCursorView>,
    before: Option<i64>,
    scope_id: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<FeedPageView> {
    let _timed = crate::log::Timed::start("channel_history");
    let d = state.driver()?;
    let keys = d.keys()?;
    let cursor = cursor_of(next, before);
    let page = d.service.history_page_in(&keys, &community_id, scope_id.as_deref(), i64::from(limit.unwrap_or(0)), cursor.as_ref()).await.map_err(map_err)?;
    Ok(page.into())
}

/// A page of the comments under a post, oldest first, before `next`.
#[tauri::command]
pub async fn channel_thread(
    community_id: String,
    post_id: String,
    limit: Option<u32>,
    next: Option<FeedCursorView>,
    before: Option<i64>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<FeedPageView> {
    let _timed = crate::log::Timed::start("channel_thread");
    let d = state.driver()?;
    let keys = d.keys()?;
    let cursor = cursor_of(next, before);
    let page = d.service.thread_page(&keys, &community_id, &post_id, i64::from(limit.unwrap_or(0)), cursor.as_ref()).await.map_err(map_err)?;
    Ok(page.into())
}

/// Something written in a channel: what the core did and its id.
async fn written(d: &Driver, community_id: &str, write: impl std::future::Future<Output = messenger_core::Result<(String, communities_core::service::Outcome)>>) -> CmdResult<String> {
    let (id, outcome) = {
        let lock = d.service.lock_of(community_id).await;
        let _guard = lock.lock().await;
        write.await.map_err(map_err)?
    };
    d.apply(outcome).await;
    Ok(id)
}

/// How the words are written, as the messenger's commands take it: `md`
/// for Markdown, anything else (or nothing) plain.
fn format(fmt: Option<&str>) -> TextFormat {
    TextFormat::from_name(fmt)
}

/// A post (`fmt`: `md` when written in Markdown); its id.
#[tauri::command]
pub async fn channel_post(
    community_id: String,
    text: String,
    fmt: Option<String>,
    scope_id: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<String> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_post");
        let d = state.driver()?;
        let keys = d.keys()?;
        written(&d, &community_id, d.service.send_post_in(&keys, &community_id, scope_id.as_deref(), &text, format(fmt.as_deref()))).await
    })
    .await
}

/// A comment under a post (`parent`: a reply in a branch); its id.
#[tauri::command]
pub async fn channel_comment(
    community_id: String,
    post_id: String,
    parent: Option<String>,
    text: String,
    fmt: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<String> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_comment");
        let d = state.driver()?;
        let keys = d.keys()?;
        written(&d, &community_id, d.service.send_comment(&keys, &community_id, &post_id, parent.as_deref(), &text, format(fmt.as_deref()))).await
    })
    .await
}

/// A reaction set or taken back.
#[tauri::command]
pub async fn channel_react(
    community_id: String,
    target: String,
    emoji: String,
    set: bool,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<String> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_react");
        let d = state.driver()?;
        let keys = d.keys()?;
        written(&d, &community_id, d.service.send_reaction(&keys, &community_id, &target, &emoji, set)).await
    })
    .await
}

/// A new text of my post or comment, in `fmt`.
#[tauri::command]
pub async fn channel_edit(
    community_id: String,
    target: String,
    text: String,
    fmt: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<String> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_edit");
        let d = state.driver()?;
        let keys = d.keys()?;
        written(&d, &community_id, d.service.send_edit(&keys, &community_id, &target, &text, format(fmt.as_deref()))).await
    })
    .await
}

/// The blocks of words written in Markdown, as the feed shows them once
/// they are posted (`FeedItemView::rich`): the preview of the post editor.
/// Nothing is read or sent; `None` when they would be shown as plain text.
#[tauri::command]
pub async fn channel_md_preview(text: String) -> CmdResult<Option<Vec<communities_core::transport::MdBlock>>> {
    Ok(communities_core::transport::rich_of(&text))
}

/// A message removed: my own, or one of a lower role as a moderator.
#[tauri::command]
pub async fn channel_delete(community_id: String, target: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<String> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_delete");
        let d = state.driver()?;
        let keys = d.keys()?;
        written(&d, &community_id, d.service.send_delete(&keys, &community_id, &target)).await
    })
    .await
}

/// Pins or unpins a post (a moderator and above).
#[tauri::command]
pub async fn channel_pin(community_id: String, post_id: String, pinned: bool, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let _timed = crate::log::Timed::start("channel_pin");
        let d = state.driver()?;
        let keys = d.keys()?;
        let outcome = {
            let lock = d.service.lock_of(&community_id).await;
            let _guard = lock.lock().await;
            d.service.pin(&keys, &community_id, &post_id, pinned).await.map_err(map_err)?
        };
        d.apply(outcome).await;
        Ok(())
    })
    .await
}

/// The channel is read up to `at` (now when not given).
#[tauri::command]
pub async fn channel_read(community_id: String, at: Option<i64>, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    let _timed = crate::log::Timed::start("channel_read");
    let d = state.driver()?;
    db::content::set_read(d.service.db(), &community_id, at.unwrap_or_else(now)).await.map_err(map_err)?;
    crate::seams::unread_changed(&d);
    Ok(())
}

// ─── Files ──────────────────────────────────────────────────────────────────

/// How a picked file goes, as the page says it (`channel_media_send`
/// `options`), as a chat's file does: a photo as it is (`original`) or
/// made smaller, a video with the frame the page took (`poster`).
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct FileOptions {
    pub original: bool,
    pub poster: Option<communities_core::media::Poster>,
}

/// A picked file as a post, or as a comment under `post_id`: its row at
/// once, the upload in the background, told by `channel.media`. The
/// arguments are the page's, by name (`scopeId`: the section of a post;
/// `batch`: the files picked together, an album, the same for each;
/// `options`: `FileOptions`; `fmt`: `md` when the caption is Markdown).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn channel_media_send(
    community_id: String,
    src: String,
    post_id: Option<String>,
    parent: Option<String>,
    caption: Option<String>,
    fmt: Option<String>,
    scope_id: Option<String>,
    batch: Option<String>,
    options: Option<FileOptions>,
    app: tauri::AppHandle,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<UploadView> {
    let _timed = crate::log::Timed::start("channel_media_send");
    use tauri::Manager;
    let d = state.driver()?;
    let keys = d.keys()?;
    let outgoing = d.files.join(OUTGOING_DIR);
    // A copy `channel_media_import` made (a phone's video, for its frame)
    // goes as it is; a file of the disk too; one a phone hands over by its
    // URI is copied first, as the upload reads it again on a retry. A copy
    // goes under the name the user picked, not its own.
    let (path, name) = match own_copy(&outgoing, &src) {
        Some(copy) => {
            let name = crate::views::upload_name(&copy.to_string_lossy());
            (copy, name)
        }
        None => {
            let source = app.state::<Shell>().open_source(&src)?;
            if std::path::Path::new(&src).is_file() {
                (std::path::PathBuf::from(&src), crate::views::upload_name(&src))
            } else {
                let open = source.open.clone();
                (copy_out(move || open(), &source.name, outgoing.clone()).await?, source.name.clone())
            }
        }
    };
    let FileOptions { original, poster } = options.unwrap_or_default();
    let file = NewFile {
        name: Some(name.as_str()).filter(|n| !n.is_empty()),
        caption: caption.as_deref(),
        scope_id: scope_id.as_deref(),
        post_id: post_id.as_deref(),
        parent: parent.as_deref(),
        batch: batch.as_deref(),
        original,
        poster,
        fmt: format(fmt.as_deref()),
        ..NewFile::at(&path)
    };
    let placed = d.service.media_placeholder_file(&keys, &community_id, file).await;
    let (row, outcome) = placed.map_err(|e| {
        drop_copy(&outgoing, &path);
        map_err(e)
    })?;
    d.apply(outcome).await;
    spawn_send(&d, keys, community_id, row.id.clone(), outgoing);
    Ok(row.into())
}

/// A voice message or a video circle recorded in the composer, as a post
/// (`scopeId`: its section) or as a comment under `post_id` (`parent`: the
/// comment it answers), as a chat's (`messenger_dm_send_recording`): its
/// row at once, the upload in the background, told by `channel.media`,
/// as a picked file's. A reader who may comment records a comment.
#[tauri::command]
pub async fn channel_media_record(
    community_id: String,
    recording: NewRecording,
    post_id: Option<String>,
    parent: Option<String>,
    scope_id: Option<String>,
    state: tauri::State<'_, CommunitiesState>,
) -> CmdResult<UploadView> {
    let _timed = crate::log::Timed::start("channel_media_record");
    let d = state.driver()?;
    // Boxed: the bridge thread of Android carries only the box (`command_futures`).
    Box::pin(async move {
        let keys = d.keys()?;
        let outgoing = d.files.join(OUTGOING_DIR);
        let place = RecordingPlace { scope_id: scope_id.as_deref(), post_id: post_id.as_deref(), parent: parent.as_deref() };
        let (row, outcome) = d.service.media_placeholder_recording(&keys, &community_id, recording, place, &outgoing).await.map_err(map_err)?;
        d.apply(outcome).await;
        spawn_send(&d, keys, community_id, row.id.clone(), outgoing);
        Ok(row.into())
    })
    .await
}

/// The status a page knows a file by, from the word of its transfer
/// (the messenger's `transfer.progress`): `uploading`, `paused`, `failed`,
/// `cancelled`, `sent` of mine; `downloading`, `done` of a message.
fn media_status(direction: &str, status: &str) -> String {
    match (direction, status) {
        ("up", "queued" | "running" | "waiting_retry") => "uploading",
        ("up", "done") => "sent",
        ("down", "queued" | "running" | "waiting_retry") => "downloading",
        (_, other) => other,
    }
    .to_string()
}

/// Tells the page how a file goes, as `channel.media`: what it knew
/// (`status`, `done`, `total`) and the messenger's progress of it, with
/// its stage, speed and time left (`progress`).
fn media_sink(d: &Driver, community_id: &str) -> Sink<impl Fn(Progress) + Send + Sync> {
    let (task, community_id) = (d.clone(), community_id.to_string());
    Sink(move |p: Progress| {
        task.emit(&messenger_core::traits::UiEvent {
            name: communities_core::media::UI_EVENT_CHANNEL_MEDIA.into(),
            payload: serde_json::json!({
                "community_id": community_id,
                "id": p.transfer_id,
                "status": media_status(&p.direction, &p.status),
                "done": p.done_bytes,
                "total": p.total_bytes,
                "error": p.failure_reason,
                "progress": p,
            }),
        });
    })
}

/// Sends the file of a placeholder in the background, its progress and its
/// end told by `channel.media`.
pub(crate) fn spawn_send(d: &Driver, keys: nostr::key::Keys, community_id: String, upload: String, outgoing: std::path::PathBuf) {
    let task = d.clone();
    let id = upload.clone();
    d.uploads.spawn(&id, async move {
        let sink = media_sink(&task, &community_id);
        match task.service.media_send(&keys, &upload, Some(&outgoing), &sink).await {
            Ok((_, outcome)) => {
                // Sent: my post shows the file from where it went (a copy
                // stays while it names it, `sweep_outgoing`).
                task.apply(outcome).await;
            }
            Err(e) => {
                let error = map_err(e).to_string();
                // Given up meanwhile: the row and the copy are gone already,
                // and the page is told so under its own code; a pause waits.
                let status = if error.starts_with(UPLOAD_CANCELLED) {
                    "cancelled"
                } else if error.starts_with(UPLOAD_PAUSED) {
                    "paused"
                } else {
                    eprintln!("communities: a file did not go up: {error}");
                    "failed"
                };
                let error = (status == "failed").then_some(error);
                task.emit(&messenger_core::traits::UiEvent {
                    name: communities_core::media::UI_EVENT_CHANNEL_MEDIA.into(),
                    payload: serde_json::json!({ "community_id": community_id, "id": upload, "status": status, "error": error }),
                });
            }
        }
    });
}

/// A failed or paused file sent again from where it waits (its copy, or
/// the user's own file), on from the chunks the hosts hold; its row is
/// `uploading` again at once.
#[tauri::command]
pub async fn channel_media_retry(upload_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<UploadView> {
    let _timed = crate::log::Timed::start("channel_media_retry");
    let d = state.driver()?;
    let keys = d.keys()?;
    let (row, outcome) = d.service.media_retry(&upload_id).await.map_err(map_err)?;
    d.apply(outcome).await;
    spawn_send(&d, keys, row.community_id.clone(), row.id.clone(), d.files.join(OUTGOING_DIR));
    Ok(row.into())
}

/// Pauses a file of mine on its way (by its upload id) or the file of a
/// message coming down (by the message's id): its requests stop at once,
/// what is done stays; `channel_media_retry` or another fetch goes on.
/// Whether one was running.
#[tauri::command]
pub async fn channel_media_pause(id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<bool> {
    let _timed = crate::log::Timed::start("channel_media_pause");
    Ok(state.driver()?.service.media_pause(&id))
}

/// "Retry now" of a file waiting for its next automatic attempt, mine
/// going up (upload id) or a message's coming down (message id). Whether
/// one was running.
#[tauri::command]
pub async fn channel_media_wake(id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<bool> {
    let _timed = crate::log::Timed::start("channel_media_wake");
    Ok(state.driver()?.service.media_wake(&id))
}

/// Gives up the file of a message coming down: what it fetched goes.
/// Whether one was running.
#[tauri::command]
pub async fn channel_media_fetch_cancel(message_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<bool> {
    let _timed = crate::log::Timed::start("channel_media_fetch_cancel");
    Ok(state.driver()?.service.media_fetch_cancel(&message_id))
}

/// The file of a message, fetched from the hosts of the channel and
/// checked against its hash; the path of the copy here. Its progress is
/// told by `channel.media` under the message's id.
#[tauri::command]
pub async fn channel_media_fetch(community_id: String, message_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<String> {
    let _timed = crate::log::Timed::start("channel_media_fetch");
    let d = state.driver()?;
    // Boxed: the engine's download is a large future (`command_futures`).
    Box::pin(async move {
        let dir = d.files.join(MEDIA_DIR);
        let sink = media_sink(&d, &community_id);
        let path = d.service.media_fetch(&community_id, &message_id, &dir, &sink).await.map_err(map_err)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}

/// Inline preview (`data:` URL) of the file of a message once
/// `channel_media_fetch` brought it here, for a picture, a video or a
/// sound small enough; `null` otherwise (as `messenger_media_data_url`).
/// A `data:` URL and not a path: the webview may not read the module's
/// folder, and the messenger's media server names only its own files.
#[tauri::command]
pub async fn channel_media_url(community_id: String, message_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Option<String>> {
    let _timed = crate::log::Timed::start("channel_media_url");
    let d = state.driver()?;
    d.service.media_data_url(&community_id, &message_id, &d.files.join(MEDIA_DIR)).await.map_err(map_err)
}

/// Where the page plays the file of a message once `channel_media_fetch`
/// brought it here: a video or a sound of any size, from the messenger's
/// server of files, as a chat plays its own (`messenger_media_playable`).
/// `null` while it is not here, for any other file, or without the
/// messenger (the page then takes `channel_media_url`).
#[tauri::command]
pub async fn channel_media_play(community_id: String, message_id: String, app: tauri::AppHandle, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Option<String>> {
    let _timed = crate::log::Timed::start("channel_media_play");
    let d = state.driver()?;
    let Some((path, mime)) = d.service.media_playable(&community_id, &message_id, &d.files.join(MEDIA_DIR)).await.map_err(map_err)? else { return Ok(None) };
    let Some(urls) = veydan_shell::seams::seam::<dyn MediaUrls, _, _>(&app) else { return Ok(None) };
    Ok(urls.url(&path, &mime).await.ok().flatten())
}

/// A file picked to be sent, made a file of the device first when a phone
/// hands it over by its URI (the copy `channel_media_send` then takes as
/// it is), with where the page reads a video to take a frame of it for
/// its readers' preview (`options.poster`), as a chat's composer does.
#[derive(Clone, Debug, serde::Serialize, ts_rs::TS)]
pub struct ImportedFile {
    /// What `channel_media_send` sends.
    pub path: String,
    /// The name the user picked.
    pub name: String,
    /// A video the webview plays, from the messenger's server of files;
    /// `null` for anything else, or without the messenger.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[tauri::command]
pub async fn channel_media_import(src: String, app: tauri::AppHandle, state: tauri::State<'_, CommunitiesState>) -> CmdResult<ImportedFile> {
    let _timed = crate::log::Timed::start("channel_media_import");
    use tauri::Manager;
    let d = state.driver()?;
    let source = app.state::<Shell>().open_source(&src)?;
    let (path, name) = if std::path::Path::new(&src).is_file() {
        (std::path::PathBuf::from(&src), crate::views::upload_name(&src))
    } else {
        let open = source.open.clone();
        (copy_out(move || open(), &source.name, d.files.join(OUTGOING_DIR)).await?, source.name.clone())
    };
    let url = match communities_core::media::playable_by_name(&name).filter(|m| m.starts_with("video/")) {
        Some(mime) => match veydan_shell::seams::seam::<dyn MediaUrls, _, _>(&app) {
            Some(urls) => urls.url(&path, &mime).await.ok().flatten(),
            None => None,
        },
        None => None,
    };
    Ok(ImportedFile { path: path.to_string_lossy().into_owned(), name, url })
}

/// The cover of a channel as a `data:` URL, read from the hosts of the
/// channel and kept on disk by its hash (`ChannelService::cover_data_url`):
/// the messenger's avatar cache fetches only the faces of people, so a
/// cover would stay initials for a reader. `null`: no cover, or no host
/// has it now.
#[tauri::command]
pub async fn channel_cover(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Option<String>> {
    let _timed = crate::log::Timed::start("channel_cover");
    let d = state.driver()?;
    d.service.cover_data_url(&community_id, &d.files.join(COVER_DIR)).await.map_err(map_err)
}

/// A picked file as the tray above the composer shows it before it is
/// sent: a picture as a small square `data:` URL
/// (`communities_core::media::picked_preview`), `null` for anything else.
/// Read and decoded off the runtime's threads: a phone's photo takes a
/// moment.
#[tauri::command]
pub async fn channel_picked_preview(src: String, app: tauri::AppHandle) -> CmdResult<Option<String>> {
    let _timed = crate::log::Timed::start("channel_picked_preview");
    tokio::task::spawn_blocking(move || {
        use tauri::Manager;
        let source = app.state::<Shell>().open_source(&src)?;
        let file = (source.open)().map_err(AppError::io)?;
        picked_preview(file).map_err(AppError::io)
    })
    .await
    .map_err(AppError::io)?
}

/// Gives up a file on its way: its upload ends (the run hears it at once
/// and removes what the hosts hold of it) and its copy goes.
#[tauri::command]
pub async fn channel_media_cancel(upload_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<()> {
    let _timed = crate::log::Timed::start("channel_media_cancel");
    let d = state.driver()?;
    let row = db::content::upload(d.service.db(), &upload_id).await.map_err(map_err)?;
    d.service.media_cancel(&upload_id).await.map_err(map_err)?;
    if let Some(row) = row {
        drop_copy(&d.files.join(OUTGOING_DIR), std::path::Path::new(&row.path));
    }
    Ok(())
}

/// Copies a file handed over by its URI into `dir`, off the async workers:
/// it may be a gigabyte. A copy that broke off is not left behind.
async fn copy_out(open: impl FnOnce() -> std::io::Result<std::fs::File> + Send + 'static, name: &str, dir: std::path::PathBuf) -> CmdResult<std::path::PathBuf> {
    let name = format!("{}-{}", now(), name.replace(['/', '\\'], "_"));
    let copied = tokio::task::spawn_blocking(move || -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(name);
        let mut from = open()?;
        let mut to = std::fs::File::create(&path)?;
        if let Err(e) = std::io::copy(&mut from, &mut to) {
            drop(to);
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(path)
    })
    .await
    .map_err(|e| AppError::Other(format!("channel_invalid: the copy did not finish: {e}")))?;
    copied.map_err(AppError::io)
}

/// How long a copy no row names stays (`sweep_outgoing`): one picked a
/// moment ago may be on its way to a row.
const OUTGOING_KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Removes the copies of the folder of copies that no row names (a file
/// of mine on its way, a post of mine that shows its file from there) and
/// that are older than `OUTGOING_KEEP`, as the messenger sweeps its own.
pub(crate) async fn sweep_outgoing(d: &Driver) {
    let Ok(named) = db::content::named_files(d.service.db()).await else { return };
    let outgoing = d.files.join(OUTGOING_DIR);
    let gone = tokio::task::spawn_blocking(move || {
        let named: std::collections::HashSet<std::path::PathBuf> = named.into_iter().map(std::path::PathBuf::from).collect();
        let Ok(dir) = std::fs::read_dir(&outgoing) else { return 0 };
        let now = std::time::SystemTime::now();
        let mut gone = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let old = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > OUTGOING_KEEP);
            if old && path.is_file() && !named.contains(&path) && std::fs::remove_file(&path).is_ok() {
                gone += 1;
            }
        }
        gone
    })
    .await
    .unwrap_or(0);
    if gone > 0 {
        crate::log::clog!("copies of files no row names removed: {gone}");
    }
}

/// Removes the copy of a file once it is sent or given up; a file of the
/// user, sent from where it lies, is never touched.
/// `src` when it is a copy this module made in its folder of copies
/// `outgoing` (`channel_media_import`: a phone's file, copied so that the
/// page takes a frame of a video from it), as that folder names it. The
/// picker handed over the phone's `content://` URI, not this path, so
/// `open_source`, which reads only what the user picked, refuses it. Only
/// a file right in that folder once links and `..` are resolved: a page
/// cannot name anything else through it.
fn own_copy(outgoing: &std::path::Path, src: &str) -> Option<std::path::PathBuf> {
    let real = std::fs::canonicalize(src).ok()?;
    let dir = std::fs::canonicalize(outgoing).ok()?;
    if real.parent() != Some(dir.as_path()) || !real.is_file() {
        return None;
    }
    Some(outgoing.join(real.file_name()?))
}

fn drop_copy(outgoing: &std::path::Path, path: &std::path::Path) {
    if path.parent() == Some(outgoing) {
        match std::fs::remove_file(path) {
            // The core removed it already: a photo made smaller stands
            // for the copy it was made of.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("communities: a copy of a sent file was not removed: {e}"),
            Ok(()) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Без Veydan Cloud файлы канала — на бесплатных хостах мессенджера,
    /// 14 дней (правило владельца 2026-10-23); тариф, что держит дольше или
    /// без срока, — уже не бесплатный.
    #[test]
    fn the_plan_of_the_storage_is_free_without_a_longer_one() {
        let free = |d| storage_plan(d).free;
        assert_eq!(storage_plan(None), StoragePlanView { free: true, retention_days: 14 });
        assert!(free(Some(14)) && free(Some(7)));
        assert!(!free(Some(0)) && !free(Some(90)));
    }

    /// The cursor a page answers with comes back from the page as it went
    /// out, and is what the next page starts from: two posts of one second
    /// are told apart by their ids, and an old `before` is still taken
    /// (ревью модуля п. 11).
    #[test]
    fn a_page_is_asked_for_with_the_cursor_it_answered_with() {
        let page = communities_core::transport::FeedPage {
            items: vec![],
            next_before: Some(5),
            next: Some(FeedCursor { created_at: 5, id: "b".repeat(64) }),
            asked: false,
        };
        let sent = serde_json::to_value(FeedPageView::from(page)).unwrap();
        assert_eq!(sent["next"], serde_json::json!({ "created_at": 5, "id": "b".repeat(64) }));
        let back: FeedCursorView = serde_json::from_value(sent["next"].clone()).unwrap();
        assert_eq!(cursor_of(Some(back), Some(9)), Some(FeedCursor { created_at: 5, id: "b".repeat(64) }), "the cursor wins over a date");
        assert_eq!(cursor_of(None, Some(9)), Some(FeedCursor { created_at: 9, id: String::new() }));
        assert_eq!(cursor_of(None, None), None);
    }

    /// A copy is made off the async workers into the folder of copies, and
    /// removed from there only: a file of the user stays (ревью модуля п. 8).
    #[tokio::test]
    async fn a_copy_is_made_aside_and_only_a_copy_is_removed() {
        let root = std::env::temp_dir().join(format!("communities-copies-{}-{}", std::process::id(), now()));
        let user = root.join("picked.bin");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&user, [7u8; 4096]).unwrap();
        let outgoing = root.join(OUTGOING_DIR);
        let from = user.clone();
        let copy = copy_out(move || std::fs::File::open(from), "a/b.bin", outgoing.clone()).await.unwrap();
        assert_eq!(copy.parent(), Some(outgoing.as_path()));
        assert!(copy.file_name().unwrap().to_string_lossy().ends_with("a_b.bin"));
        assert_eq!(std::fs::read(&copy).unwrap(), vec![7u8; 4096]);

        drop_copy(&outgoing, &user);
        assert!(user.exists(), "the user's file stays");
        drop_copy(&outgoing, &copy);
        assert!(!copy.exists(), "the copy goes");

        let missing = root.join("gone.bin");
        let failed = copy_out(move || std::fs::File::open(missing), "gone.bin", outgoing.clone()).await;
        assert!(failed.is_err());
        assert_eq!(std::fs::read_dir(&outgoing).unwrap().count(), 0, "nothing is left of a copy that failed");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A phone's video copied by `channel_media_import` (for its frame) is
    /// sent by the path of that copy: it is the module's own file, taken
    /// without the picker's word, under the name the user picked; nothing
    /// else is taken that way — not a file beside the folder, not one
    /// reached through `..`, not the folder itself (живой запуск 2026-10-23:
    /// the video went without its frame).
    #[test]
    fn a_copy_of_mine_is_sent_by_its_path_and_nothing_else_is() {
        let root = std::env::temp_dir().join(format!("communities-own-{}-{}", std::process::id(), now()));
        let outgoing = root.join(OUTGOING_DIR);
        std::fs::create_dir_all(&outgoing).unwrap();
        let copy = outgoing.join("1792736850-207.mp4");
        std::fs::write(&copy, [1u8; 64]).unwrap();
        let beside = root.join("secret.db");
        std::fs::write(&beside, [2u8; 64]).unwrap();

        let got = own_copy(&outgoing, &copy.to_string_lossy()).unwrap();
        assert_eq!(got, copy, "named as the folder names it: a failed placeholder removes it (drop_copy)");
        assert_eq!(crate::views::upload_name(&got.to_string_lossy()), "207.mp4");
        let through = outgoing.join("..").join(OUTGOING_DIR).join("1792736850-207.mp4");
        assert_eq!(own_copy(&outgoing, &through.to_string_lossy()), Some(copy.clone()), "the same file by another spelling");

        assert_eq!(own_copy(&outgoing, &beside.to_string_lossy()), None);
        assert_eq!(own_copy(&outgoing, &outgoing.join("..").join("secret.db").to_string_lossy()), None);
        assert_eq!(own_copy(&outgoing, &outgoing.to_string_lossy()), None);
        assert_eq!(own_copy(&outgoing, &outgoing.join("gone.mp4").to_string_lossy()), None);
        assert_eq!(own_copy(&outgoing, "content://media/picker/0/com.android.providers.media.photopicker/media/1000012345"), None);
        #[cfg(unix)]
        {
            let link = outgoing.join("link.db");
            std::os::unix::fs::symlink(&beside, &link).unwrap();
            assert_eq!(own_copy(&outgoing, &link.to_string_lossy()), None, "a link out of the folder");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// My files on their way into a channel.
#[tauri::command]
pub async fn channel_uploads(community_id: String, state: tauri::State<'_, CommunitiesState>) -> CmdResult<Vec<UploadView>> {
    let _timed = crate::log::Timed::start("channel_uploads");
    let rows = state.driver()?.service.uploads(&community_id).await.map_err(map_err)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// What a command leaves on the thread that calls it. On Android a command
/// is called on the WebView's bridge thread, whose stack is small: Tauri
/// builds the command's future there and moves it a few times before it is
/// spawned, so a large one overflows that stack and the app dies
/// (живой запуск 2026-10-23: `channel_open_link` and
/// `channel_answer_invite`, 19 KB each, SIGSEGV on entering a channel by an
/// invitation, a release build only; posts and reactions were 58 KB). A
/// command whose work is large boxes it (`Box::pin`), so that only the box
/// crosses the bridge.
#[cfg(test)]
mod command_futures {
    use super::*;
    use crate::views::{LevelInput, ModAction, ModSettings, NewScopeInput, ScopeSetInput};
    fn r1<A, R>(_: fn(A) -> R) -> usize { std::mem::size_of::<R>() }
    fn r2<A, B, R>(_: fn(A, B) -> R) -> usize { std::mem::size_of::<R>() }
    fn r3<A, B, C, R>(_: fn(A, B, C) -> R) -> usize { std::mem::size_of::<R>() }
    fn r4<A, B, C, D, R>(_: fn(A, B, C, D) -> R) -> usize { std::mem::size_of::<R>() }
    fn r5<A, B, C, D, E, R>(_: fn(A, B, C, D, E) -> R) -> usize { std::mem::size_of::<R>() }
    fn r6<A, B, C, D, E, G, R>(_: fn(A, B, C, D, E, G) -> R) -> usize { std::mem::size_of::<R>() }
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn r11<A, B, C, D, E, G, H, I, J, K, L, R>(_: fn(A, B, C, D, E, G, H, I, J, K, L) -> R) -> usize { std::mem::size_of::<R>() }
    type S = tauri::State<'static, CommunitiesState>;

    /// The bound every command keeps: well below a stack of the bridge
    /// thread even when Tauri copies the future a few times.
    const MAX_FUTURE_BYTES: usize = 4096;

    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn every_command_leaves_a_small_future_on_the_calling_thread() {
        let mut v: Vec<(&str, usize)> = Vec::new();
        v.push(("channel_outbox_retry", r2::<String, S, _>(channel_outbox_retry)));
        v.push(("channels_list", r1::<S, _>(channels_list)));
        v.push(("channel_get", r2::<String, S, _>(channel_get)));
        v.push(("channel_create", r2::<NewChannelInput, S, _>(channel_create)));
        v.push(("channel_storage_plan", r1::<tauri::AppHandle, _>(channel_storage_plan)));
        v.push(("channel_invite", r3::<String, String, S, _>(channel_invite)));
        v.push(("channel_invites", r2::<String, S, _>(channel_invites)));
        v.push(("channel_answer_invite", r3::<String, bool, S, _>(channel_answer_invite)));
        v.push(("channel_open_link", r3::<String, Option<String>, S, _>(channel_open_link)));
        v.push(("channel_preview_link", r2::<String, S, _>(channel_preview_link)));
        v.push(("channel_answer_request", r4::<String, String, bool, S, _>(channel_answer_request)));
        v.push(("channel_act", r3::<String, ChannelAction, S, _>(channel_act)));
        v.push(("channel_avatar_set", r4::<String, String, tauri::AppHandle, S, _>(channel_avatar_set)));
        v.push(("channel_avatar_remove", r2::<String, S, _>(channel_avatar_remove)));
        v.push(("channel_rotate_link", r2::<String, S, _>(channel_rotate_link)));
        v.push(("channel_link_qr", r2::<String, S, _>(channel_link_qr)));
        v.push(("channel_forget", r2::<String, S, _>(channel_forget)));
        v.push(("channel_history", r6::<String, Option<u32>, Option<FeedCursorView>, Option<i64>, Option<String>, S, _>(channel_history)));
        v.push(("channel_thread", r6::<String, String, Option<u32>, Option<FeedCursorView>, Option<i64>, S, _>(channel_thread)));
        v.push(("channel_post", r5::<String, String, Option<String>, Option<String>, S, _>(channel_post)));
        v.push(("channel_comment", r6::<String, String, Option<String>, String, Option<String>, S, _>(channel_comment)));
        v.push(("channel_react", r5::<String, String, String, bool, S, _>(channel_react)));
        v.push(("channel_edit", r5::<String, String, String, Option<String>, S, _>(channel_edit)));
        v.push(("channel_md_preview", r1::<String, _>(channel_md_preview)));
        v.push(("channel_delete", r3::<String, String, S, _>(channel_delete)));
        v.push(("channel_pin", r4::<String, String, bool, S, _>(channel_pin)));
        v.push(("channel_read", r3::<String, Option<i64>, S, _>(channel_read)));
        v.push(("channel_media_send", r11::<String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<FileOptions>, tauri::AppHandle, S, _>(channel_media_send)));
        v.push(("channel_media_record", r6::<String, NewRecording, Option<String>, Option<String>, Option<String>, S, _>(channel_media_record)));
        v.push(("channel_media_retry", r2::<String, S, _>(channel_media_retry)));
        v.push(("channel_media_fetch", r3::<String, String, S, _>(channel_media_fetch)));
        v.push(("channel_media_url", r3::<String, String, S, _>(channel_media_url)));
        v.push(("channel_cover", r2::<String, S, _>(channel_cover)));
        v.push(("channel_picked_preview", r2::<String, tauri::AppHandle, _>(channel_picked_preview)));
        v.push(("channel_media_cancel", r2::<String, S, _>(channel_media_cancel)));
        v.push(("channel_media_pause", r2::<String, S, _>(channel_media_pause)));
        v.push(("channel_media_wake", r2::<String, S, _>(channel_media_wake)));
        v.push(("channel_media_play", r4::<String, String, tauri::AppHandle, S, _>(channel_media_play)));
        v.push(("channel_media_import", r3::<String, tauri::AppHandle, S, _>(channel_media_import)));
        v.push(("channel_media_fetch_cancel", r2::<String, S, _>(channel_media_fetch_cancel)));
        v.push(("channel_uploads", r2::<String, S, _>(channel_uploads)));
        v.push(("channel_scopes", r2::<String, S, _>(levels::channel_scopes)));
        v.push(("channel_scope_add", r3::<String, NewScopeInput, S, _>(levels::channel_scope_add)));
        v.push(("channel_scope_del", r3::<String, String, S, _>(levels::channel_scope_del)));
        v.push(("channel_scope_set", r4::<String, String, ScopeSetInput, S, _>(levels::channel_scope_set)));
        v.push(("channel_scopes_order", r3::<String, Vec<String>, S, _>(levels::channel_scopes_order)));
        v.push(("channel_scope_photo_set", r5::<String, String, String, tauri::AppHandle, S, _>(levels::channel_scope_photo_set)));
        v.push(("channel_scope_photo", r3::<String, String, S, _>(levels::channel_scope_photo)));
        v.push(("channel_levels", r2::<String, S, _>(levels::channel_levels)));
        v.push(("channel_level_set", r3::<String, LevelInput, S, _>(levels::channel_level_set)));
        v.push(("channel_level_del", r3::<String, u8, S, _>(levels::channel_level_del)));
        v.push(("channel_set_levels", r4::<String, String, Vec<u8>, S, _>(levels::channel_set_levels)));
        v.push(("channel_level_request", r4::<String, u8, Option<String>, S, _>(levels::channel_level_request)));
        v.push(("channel_level_requests", r2::<String, S, _>(levels::channel_level_requests)));
        v.push(("channel_level_request_act", r6::<String, String, u8, bool, Option<String>, S, _>(levels::channel_level_request_act)));
        v.push(("channel_moderation_queue", r5::<String, Option<String>, Option<i64>, Option<u32>, S, _>(moderation::channel_moderation_queue)));
        v.push(("channel_moderation_mod", r3::<String, ModAction, S, _>(moderation::channel_moderation_mod)));
        v.push(("channel_moderation_log", r4::<String, Option<i64>, Option<u32>, S, _>(moderation::channel_moderation_log)));
        v.push(("channel_moderation_settings", r2::<String, S, _>(moderation::channel_moderation_settings)));
        v.push(("channel_moderation_settings_set", r3::<String, ModSettings, S, _>(moderation::channel_moderation_settings_set)));
        v.push(("channel_moderation_inherit", r3::<String, String, S, _>(moderation::channel_moderation_inherit)));
        v.push(("channel_moderation_invite_create", r3::<String, String, S, _>(moderation::channel_moderation_invite_create)));
        v.push(("channel_moderation_invite_list", r2::<String, S, _>(moderation::channel_moderation_invite_list)));
        v.push(("channel_moderation_invite_revoke", r2::<String, S, _>(moderation::channel_moderation_invite_revoke)));
        v.push(("channel_moderation_invite_status", r2::<String, S, _>(moderation::channel_moderation_invite_status)));
        v.push(("channel_moderation_blocked", r2::<String, S, _>(moderation::channel_moderation_blocked)));
        v.push(("channel_moderation_requests", r2::<String, S, _>(moderation::channel_moderation_requests)));
        v.push(("channel_moderation_request_act", r4::<String, String, bool, S, _>(moderation::channel_moderation_request_act)));
        v.push(("channel_moderation_requests_count", r2::<String, S, _>(moderation::channel_moderation_requests_count)));
        v.push(("channel_keeper_status", r2::<String, S, _>(moderation::channel_keeper_status)));
        v.push(("channel_keeper_keep_on", r3::<String, bool, S, _>(moderation::channel_keeper_keep_on)));
        v.push(("channel_keeper_take", r3::<String, Option<bool>, S, _>(moderation::channel_keeper_take)));
        v.push(("channel_transfer", r3::<String, String, S, _>(moderation::channel_transfer)));
        v.push(("channel_transfer_answer", r3::<String, bool, S, _>(moderation::channel_transfer_answer)));
        v.push(("channel_tickets", r2::<String, S, _>(moderation::channel_tickets)));
        v.push(("channel_tickets_request", r3::<String, u32, S, _>(moderation::channel_tickets_request)));
        v.push(("channel_rotate_now", r2::<String, S, _>(moderation::channel_rotate_now)));
        let large: Vec<String> = v.iter().filter(|(_, n)| *n > MAX_FUTURE_BYTES).map(|(c, n)| format!("{c}: {n} bytes")).collect();
        assert!(large.is_empty(), "commands whose futures are too large for the bridge thread of Android (box their work): {large:?}");
    }
}

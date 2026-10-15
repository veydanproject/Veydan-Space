// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The list first (`messenger_dm::snapshot`, internal/messenger-wire.md §3,
//! "Снимок отношений"): a session reads the snapshot of my relations from
//! my relays before it subscribes to the inbox or asks for the history,
//! so a blocked stranger's old messages meet the block and a contact added
//! quietly on my other device is a contact here from the first message.
//! Then it keeps a live subscription on the same events, so the decisions
//! of my other devices come as they are taken; and it publishes this
//! device's list three seconds after a decision, and at the start when the
//! relays are behind it. Without a relay, or in silent mode, nothing is
//! read and the session goes on as before.
//!
//! One path applies the pieces, live or fetched: the pool forwards every
//! event the relays answer, a fetch's among them, and the ingress loop
//! hands each to `RelationsHandler`. The fetch at the start is the way to
//! know that the relays have answered (EOSE) and which events make the
//! set; the start waits for the loop to have applied them, then reads the
//! set once more for what it is (the revision, whether it is whole), which
//! changes nothing twice.

use crate::session::history_catch_up;
use async_trait::async_trait;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_core::{Clock, Context, Effect, EventId, Handler, MetaInbound, Outbound, PubKey, Result, Scope, SubId, Timestamp, Transport};
use messenger_dm::snapshot::{Applied, Known, Piece, KEY_REV};
use messenger_dm::DmService;
use messenger_ingress::{filters, Outbox};
use messenger_store::{events_raw, settings, Store};
use messenger_transport::RelayPool;
use nostr::key::Keys;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Notify};

/// The snapshot was read at the start, or a piece of it was applied:
/// `{"rev", "complete", "changed", "start"}` — `start` for the reading at
/// the start of the session, after which the list is known. A signal
/// only: the page re-reads what it shows.
pub const UI_EVENT_RELATIONS: &str = "dm.relations";

/// How long the start waits for a first relay before it goes on without
/// the list.
pub(crate) const WAIT_FOR_RELAY: Duration = Duration::from_secs(5);
/// How long the relays are given to answer each of the two fetches.
pub(crate) const FETCH_PATIENCE: Duration = Duration::from_secs(5);
/// How long the start waits for the ingress loop to have applied what the
/// fetch brought: a list of thousands takes seconds on a fresh device.
const APPLY_WAIT: Duration = Duration::from_secs(30);
/// A decision taken here is published this long after it: the ones of a
/// moment (accept, then add) go as one revision.
pub(crate) const PUBLISH_AFTER: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(3) };
/// After a failed build the next try waits this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// The pieces of the snapshot as the ingress loop brings them, fetched at
/// the start or live: what another device of mine decided takes effect
/// here without a restart.
pub struct RelationsHandler {
    dm: DmService,
}

impl RelationsHandler {
    pub fn new(dm: DmService) -> Self {
        Self { dm }
    }
}

#[async_trait]
impl Handler<MetaInbound> for RelationsHandler {
    async fn handle(&self, msg: MetaInbound, _ctx: &Context) -> Result<Vec<Effect>> {
        let MetaInbound::Relations { page, created_at, content, .. } = msg else { return Ok(vec![]) };
        // Locked: nothing can be read now; the fetch of the next session
        // brings the newest list anyway.
        let Some(keys) = self.dm.session_keys() else { return Ok(vec![]) };
        let (applied, mut effects) =
            self.dm.apply_relations_snapshot(&keys, &[Piece { page, created_at: created_at.secs(), content }]).await?;
        if applied.rev.is_none() {
            return Ok(effects);
        }
        if applied.settings_changed {
            effects.extend(self.dm.sweep_requests(SystemClock.now().secs()).await?);
        }
        effects.extend(told(&applied, false));
        Ok(effects)
    }
}

/// What the page is told of an applied set.
fn told(applied: &Applied, start: bool) -> Vec<Effect> {
    let mut out = vec![Effect::Emit(UiEvent {
        name: UI_EVENT_RELATIONS.into(),
        payload: serde_json::json!({
            "rev": applied.rev, "complete": applied.complete, "changed": applied.changed.len(), "start": start,
        }),
    })];
    if !applied.changed.is_empty() {
        // A stranger's chat may be gone, a contact may be new.
        out.push(Effect::Emit(UiEvent { name: messenger_dm::UI_EVENT_CHATS_UPDATED.into(), payload: serde_json::json!({}) }));
    }
    out
}

/// The task that opens the inbox of a session, in this order: the list,
/// then the live subscriptions (the snapshot, the inbox), then the
/// history catch-up. A task, not the start itself, so that a start
/// without relays is not held up by the wait for one.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open_inbox(
    store: Store,
    pool: Arc<RelayPool>,
    keys: Keys,
    dm: DmService,
    outbox: Outbox,
    me: PubKey,
    started_at: Timestamp,
    ui: broadcast::Sender<UiEvent>,
) {
    read_list(&store, &pool, &keys, &dm, &outbox, &me, &ui).await;
    // A decision of the last run that was not published (the app closed
    // inside `PUBLISH_AFTER`): the mark outlived the close in the store,
    // but the loop that publishes hears a mark only as it is set, so it is
    // set again, after the fetch, so that the revision continues from what
    // the relays hold. Without a relay the outbox keeps it for one.
    if dm.relations_owed().await.unwrap_or(false) {
        if let Err(e) = dm.mark_relations_owed().await {
            eprintln!("messenger relations: {e}");
        }
    }
    let subscribe = |id: &str, filter| Outbound::Subscribe { id: SubId(id.into()), filter, scope: Scope::Own };
    if let Err(e) = pool.send(subscribe(filters::SUB_RELATIONS, filters::relations_live(&me))).await {
        eprintln!("messenger relations: the list is not followed: {e}");
    }
    let since = Timestamp(started_at.secs() - filters::DM_LIVE_MARGIN_SECS);
    if let Err(e) = pool.send(subscribe(filters::SUB_DM_LIVE, filters::dm_inbox(&me, since))).await {
        let _ = ui.send(UiEvent { name: "error".into(), payload: serde_json::json!({ "family": "inbox", "error": e.to_string() }) });
    }
    history_catch_up(store, pool, me, started_at, ui).await;
}

/// The snapshot from every connected relay, applied, and what this device
/// knows of the list after it. Skipped (the list stays unknown, as before
/// the snapshot) in silent mode, without a relay in `WAIT_FOR_RELAY`, or
/// when the fetch fails. The relays are owed a publication when they hold
/// no list, or an older revision than this device's.
async fn read_list(
    store: &Store,
    pool: &Arc<RelayPool>,
    keys: &Keys,
    dm: &DmService,
    outbox: &Outbox,
    me: &PubKey,
    ui: &broadcast::Sender<UiEvent>,
) {
    let deadline = tokio::time::Instant::now() + WAIT_FOR_RELAY;
    loop {
        if pool.is_silent() {
            return;
        }
        if pool.status().await.relays.iter().any(|r| r.state == RelayState::Connected) {
            let local_rev = local_rev(store).await;
            let (applied, effects) = match fetch_and_apply(store, pool, keys, dm, me).await {
                Ok(read) => read,
                Err(e) => {
                    eprintln!("messenger relations: the list was not read: {e}");
                    return;
                }
            };
            forward(outbox, ui, effects).await;
            dm.set_relations_known(match (applied.rev, applied.complete) {
                (None, _) => Known::No,
                (Some(_), true) => Known::Complete,
                (Some(_), false) => Known::Partial,
            });
            if applied.settings_changed {
                match dm.sweep_requests(SystemClock.now().secs()).await {
                    Ok(fx) => forward(outbox, ui, fx).await,
                    Err(e) => eprintln!("messenger requests: the queue was not swept: {e}"),
                }
            }
            forward(outbox, ui, told(&applied, true)).await;
            if applied.rev.is_none_or(|fetched| fetched < local_rev) {
                if let Err(e) = dm.mark_relations_owed().await {
                    eprintln!("messenger relations: {e}");
                }
            }
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn local_rev(store: &Store) -> u64 {
    settings::get(store, KEY_REV).await.ok().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// The header, then the pages it names, from every connected relay. The
/// relays' answer reaches the ingress loop as well, which applies each
/// piece (`RelationsHandler`) and keeps it as seen: this waits for that,
/// then reads the set as a whole for what it is. What the loop did not
/// apply in time is applied here, and what it did changes nothing twice.
async fn fetch_and_apply(store: &Store, pool: &RelayPool, keys: &Keys, dm: &DmService, me: &PubKey) -> Result<(Applied, Vec<Effect>)> {
    let my_key = keys.public_key();
    let mut found = pool.fetch(&filters::relations_header(me), FETCH_PATIENCE).await?;
    let pages = found.iter().filter_map(|j| Piece::of_json(j, &my_key)).filter_map(|p| pages_named(keys, &p)).max().unwrap_or(1);
    if pages > 1 {
        found.extend(pool.fetch(&filters::relations_pages(me, pages), FETCH_PATIENCE).await?);
    }
    let ids: Vec<EventId> = found.iter().filter_map(|j| EventId::parse(j["id"].as_str()?)).collect();
    let deadline = tokio::time::Instant::now() + APPLY_WAIT;
    while tokio::time::Instant::now() < deadline {
        let mut seen = true;
        for id in &ids {
            if !events_raw::contains(store, id).await.unwrap_or(false) {
                seen = false;
                break;
            }
        }
        if seen {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pieces: Vec<Piece> = found.iter().filter_map(|j| Piece::of_json(j, &my_key)).collect();
    dm.apply_relations_snapshot(keys, &pieces).await
}

/// How many events a header names, when `piece` is a header this device
/// can read; the pages are fetched for the largest among the headers.
fn pages_named(keys: &Keys, piece: &Piece) -> Option<usize> {
    if piece.page != 0 {
        return None;
    }
    let plain = nostr::nips::nip44::decrypt(keys.secret_key(), &keys.public_key(), &piece.content).ok()?;
    let v: serde_json::Value = serde_json::from_str(&plain).ok()?;
    v["pages"].as_u64().map(|n| n as usize)
}

/// The effects of what the fetched list did, as the ingress sink carries
/// those of a live piece: sends through the outbox, the rest to the page.
async fn forward(outbox: &Outbox, ui: &broadcast::Sender<UiEvent>, effects: Vec<Effect>) {
    let mut sends = false;
    for e in effects {
        match e {
            Effect::Send(out) => match outbox.enqueue(out).await {
                Ok(_) => sends = true,
                Err(e) => eprintln!("messenger relations: {e}"),
            },
            Effect::Emit(ev) => {
                let _ = ui.send(ev);
            }
            Effect::Notify(n) => {
                let _ = ui.send(UiEvent { name: "notify".into(), payload: serde_json::to_value(n).unwrap_or_default() });
            }
        }
    }
    if sends {
        outbox.kick();
    }
}

/// While the session runs: a decision taken here (`owed`, rung by the
/// hook the service holds), then quiet for `PUBLISH_AFTER`, then one
/// revision of the whole list through the outbox. Nothing is published
/// when the mark was cleared meanwhile (a stale ring).
pub(crate) async fn publish_loop(dm: DmService, outbox: Outbox, keys: Keys, owed: Arc<Notify>) {
    loop {
        owed.notified().await;
        tokio::time::sleep(PUBLISH_AFTER).await;
        if !dm.relations_owed().await.unwrap_or(false) {
            continue;
        }
        if let Err(e) = publish_once(&dm, &outbox, &keys).await {
            eprintln!("messenger relations: the list was not published: {e}");
            tokio::time::sleep(RETRY_AFTER).await;
            let _ = dm.mark_relations_owed().await;
        }
    }
}

/// One revision of this device's list into the outbox (durable: the pump
/// publishes it, and retries). The mark is cleared before the build, so
/// a decision taken during it marks again; a failure marks again too.
pub(crate) async fn publish_once(dm: &DmService, outbox: &Outbox, keys: &Keys) -> Result<(u64, usize)> {
    dm.relations_settled().await?;
    let published = async {
        let (rev, events) = dm.build_relations_snapshot(keys, SystemClock.now().secs()).await?;
        let n = events.len();
        for event in events {
            outbox.enqueue(Outbound::PublishOwn { event }).await?;
        }
        outbox.kick();
        Ok((rev, n))
    }
    .await;
    if published.is_err() {
        let _ = dm.mark_relations_owed().await;
    }
    published
}

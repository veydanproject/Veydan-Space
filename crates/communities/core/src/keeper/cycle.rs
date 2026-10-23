// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The cycle of a keeper (internal/communities-spec.md §8.3): on the
//! timer (`next_wake`) and on every note or operation, build an epoch
//! when one is due, put **all** its blobs on the hosts, and only once
//! one host (`MIN_HOSTS`) holds every blob publish its event (wire §9);
//! then `published`, the garbage of `retires`, the hosts that lag, and
//! the secret of the link in the feed.
//!
//! A turn does no network itself (живой запуск 2026-10-23: the puts and
//! gets of a turn held the keeper's mutex and the channel's lock, and the
//! loop of the module with them). What the hosts must hold or lose goes
//! out as jobs (`upload::Job`, `Tick::jobs`) run off every lock, and what
//! the relays must take as rows of the outbox (`db::outbox`): the turn
//! reads what they left (`cm_keeper_uploads`, the outbox row gone) and
//! goes on from there. So every step goes on from where a crash left it
//! too: the uploads are rows, the pending epoch is keydist's (`pending()`
//! gives the same bytes after a restart), and its signed event is kept
//! once made, so that a member never sees two versions of one epoch.

use super::upload::{Job, What, Work};
use super::{secs, Keeper, FROM_JOIN_RETENTION_SECS, MIN_HOSTS};
use crate::db::{self, keeper as rows};
use crate::epoch::{sign_epoch, EpochContent};
use crate::log::OpLog;
use crate::op::History;
use crate::state::ChannelState;
use crate::wire;
use keydist::{EpochKey, SchemeId};
use messenger_core::{EventId, MessengerError, Result};
use std::collections::BTreeSet;
use std::sync::Arc;

/// How soon a keeper tries again an epoch whose event could not go out.
pub const RETRY_SECS: u64 = 60;

/// An epoch whose event went out: its key and the signed event, for the
/// member side of this device's own user (no relay, no shard fetched).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedEpoch {
    pub scope_id: String,
    pub key: EpochKey,
    /// The signed event (kind 1392), JSON.
    pub event_json: String,
}

/// What a turn of the cycle did.
#[derive(Debug, Default)]
pub struct Tick {
    /// Epochs whose events went out in this turn.
    pub published: Vec<u32>,
    /// Epochs whose signed events were queued for the relays in this
    /// turn, with their keys: final from then on (never superseded), so
    /// the member side of this device takes them at once
    /// (`ChannelService::hand_to_me`), before the echo of the relay.
    pub published_keys: Vec<PublishedEpoch>,
    /// An epoch built and still waiting for its hosts or its relays.
    pub waiting: Option<u32>,
    /// When to run the cycle again (unix seconds).
    pub next_wake: Option<u64>,
    /// What became of the snapshot of the readers.
    pub snapshot: Option<super::SnapshotTurn>,
    /// The signed snapshot published in this turn (JSON), for the member
    /// side of this device, which holds its shards already.
    pub snapshot_event: Option<String>,
    /// The round trips to the hosts this turn asks for, run off every
    /// lock by whoever ran the turn (`ChannelService::run_job`).
    pub jobs: Vec<Job>,
    /// Pending epochs superseded in this turn, `(scope, epoch)`: a job
    /// that still puts their blobs is cancelled.
    pub burnt: Vec<(String, u32)>,
}

impl Keeper {
    /// One turn of the cycle over every scope.
    pub async fn tick(&mut self, log: &OpLog, now: i64) -> Result<Tick> {
        let state = log.state();
        let mut tick = Tick::default();
        // The scopes of the journal first, then their policies, then who
        // belongs to each (wire §14.2).
        self.sync_scopes(state, now).await?;
        self.sync_policy(state).await?;
        self.sync_members(log, now).await?;
        self.retire_tickets(log, now).await?;
        // The upload keys of an own server (wire §9.1) hold back no epoch.
        if let Err(e) = self.sync_upload_keys(log, now, &mut tick).await {
            tracing::warn!(community = %self.community_id, error = %e, "keeper: the upload keys of the staff were not made");
        }
        // The keys of the staff in the owner's Cloud (wire §9.2) neither.
        if let Err(e) = self.sync_cloud_keys(log, now, &mut tick).await {
            tracing::warn!(community = %self.community_id, error = %e, "keeper: the keys of the staff for the owner's Cloud were not made");
        }
        let scopes: Vec<String> = self.scopes.keys().cloned().collect();
        for scope_id in scopes {
            self.rotate_if_due(&scope_id, now, &mut tick).await?;
            let pending = self.sk(&scope_id)?.pending_epoch();
            if let Some(epoch) = pending {
                if self.publish_pending(&scope_id, log, now, &mut tick).await? {
                    tick.published.push(epoch);
                    if scope_id == crate::state::SCOPE_EVERYONE {
                        // Whom the epoch now published leaves out: what it was built over.
                        self.published_mark = self.built_mark.filter(|(e, _)| *e == epoch).map(|(_, m)| m);
                    }
                } else {
                    tick.waiting = Some(epoch);
                }
            }
            self.catch_up_hosts(&scope_id, state, &mut tick).await?;
            let wake = self.sk(&scope_id)?.next_wake();
            tick.next_wake = min_some(tick.next_wake, wake);
        }
        // A snapshot that fails must not stop the epochs of every turn.
        let turn = match self.snapshot_if_due(log, now, &mut tick).await {
            Ok(t) => t,
            Err(e) => {
                self.note_turn("the snapshot", Some(format!("failed: {e}")), "made again");
                super::SnapshotTurn::NotOut
            }
        };
        match turn {
            super::SnapshotTurn::TooSoon(at) => tick.next_wake = min_some(tick.next_wake, Some(secs(at))),
            super::SnapshotTurn::NotOut | super::SnapshotTurn::Waiting => tick.next_wake = min_some(tick.next_wake, Some(secs(now) + RETRY_SECS)),
            _ => {}
        }
        tick.snapshot = Some(turn);
        tick.snapshot_event = self.snapshot_out.take();
        self.catch_up_snapshot(state, &mut tick).await?;
        // The blobs due to go (the epochs just published retired theirs):
        // one job for all of them.
        self.collect_garbage(now, &mut tick).await?;
        if tick.waiting.is_some() {
            tick.next_wake = min_some(tick.next_wake, Some(secs(now) + RETRY_SECS));
        }
        // The sign of life comes last: it holds back no epoch.
        let alive = self.heartbeat_if_due(state, now).await;
        tick.next_wake = min_some(tick.next_wake, alive);
        Ok(tick)
    }

    /// Says that this keeper runs (wire §18), at most every
    /// `heartbeat::INTERVAL_SECS`: a quiet channel makes no epoch for hours,
    /// and its members took a live keeper for one away (живой запуск
    /// 2026-10-23: «был в сети: 16:20» while it was online). Only the keeper
    /// of the current series; one event, no upload, a row of the
    /// replaceable lane of the outbox (the newer takes the older's place;
    /// «был в сети» on this device is written when a relay took it,
    /// `flush_outbox`). When to say it next.
    async fn heartbeat_if_due(&mut self, state: &ChannelState, now: i64) -> Option<u64> {
        use crate::heartbeat::{sign_heartbeat, Heartbeat, HEARTBEAT_VERSION, INTERVAL_SECS};
        let me = self.keys.public_key().to_hex();
        let series = state.keeper().filter(|k| k.pubkey.as_hex() == me)?.serial;
        if let Some(at) = self.heartbeat_at.filter(|at| now - at < INTERVAL_SECS && *at <= now) {
            return Some(secs(at + INTERVAL_SECS));
        }
        let cid = self.community_id.clone();
        let hb = Heartbeat { v: HEARTBEAT_VERSION, community_id: cid.clone(), at: now, serial: series };
        let queued = match sign_heartbeat(&self.keys, &hb) {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(json) => db::outbox::enqueue_event(&self.env.db, &cid, db::outbox::LANE_REPLACEABLE, wire::KIND_HEARTBEAT, &event.id.to_hex(), None, &json, now).await.map(|_| ()).map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            },
            Err(e) => Err(e.to_string()),
        };
        match queued {
            Ok(()) => {
                self.env.outbox.notify_one();
                self.heartbeat_at = Some(now);
                self.note_turn("the sign of life", None, "queued again");
                Some(secs(now + INTERVAL_SECS))
            }
            Err(e) => {
                self.note_turn("the sign of life", Some(format!("not queued: {e}")), "queued again");
                Some(secs(now) + RETRY_SECS)
            }
        }
    }

    /// Builds an epoch when one is due, at once when one was asked for
    /// urgently; a pending epoch whose event has not gone out is replaced
    /// (`supersede_pending`) when something newer is due, so that a ban
    /// never waits behind an upload; the job that still puts its blobs is
    /// cancelled (`Tick::burnt`).
    async fn rotate_if_due(&mut self, scope_id: &str, now: i64, tick: &mut Tick) -> Result<()> {
        let urgent = self.urgent.remove(scope_id);
        let cid = self.community_id.clone();
        let sk = self.sk(scope_id)?;
        match sk.pending_epoch() {
            Some(epoch) => {
                let newer_due = sk.due_at().is_some_and(|d| d <= secs(now));
                // An event that went out (or may have: queued for the
                // relays) is never replaced: its number is the members' now.
                let sent = rows::event(&self.env.db, &cid, scope_id, epoch).await?.is_some();
                if (urgent || newer_due) && !sent {
                    if let Some(out) = self.sk(scope_id)?.kd_mut().supersede_pending(secs(now)) {
                        tracing::info!(community = %cid, burnt = epoch, epoch = out.key.epoch, "keeper superseded a pending epoch");
                        self.note_build(scope_id);
                        tick.burnt.push((scope_id.to_string(), epoch));
                    }
                    self.save(scope_id).await?;
                    let published = self.sk(scope_id)?.published_head().map(|h| h.epoch);
                    rows::drop_uploads_except(&self.env.db, &cid, scope_id, published).await?;
                } else if urgent {
                    // Asked again once the pending one is out.
                    self.urgent.insert(scope_id.to_string());
                }
            }
            None => {
                if self.sk(scope_id)?.kd_mut().rotate(secs(now), urgent).built().is_some() {
                    self.note_build(scope_id);
                    self.save(scope_id).await?;
                }
            }
        }
        Ok(())
    }

    /// The pending epoch of a scope, in three states and with no network:
    /// fewer than `MIN_HOSTS` hosts hold every blob by the rows → a job
    /// that puts them (`Tick::jobs`); enough hold them and no event row
    /// yet → its signed event is kept as a row and queued for the relays,
    /// in one transaction; the event row is here and the outbox row is
    /// gone (the keeper lane: an epoch no relay takes holds nothing else
    /// back), so a relay took it → `published`, the key to the members of a
    /// flat scope, the blobs it retires (the states are told apart by the
    /// rows alone, after a restart too). The key and the event go to the
    /// member side of this device as soon as the event is queued
    /// (`Tick::published_keys`): nothing changes them from then on, and
    /// the echo of the relay is a repeat. Whether the epoch was published.
    async fn publish_pending(&mut self, scope_id: &str, log: &OpLog, now: i64, tick: &mut Tick) -> Result<bool> {
        let state = log.state();
        let cid = self.community_id.clone();
        let Some(out) = self.sk(scope_id)?.kd().pending().cloned() else { return Ok(false) };
        let epoch = out.key.epoch;
        let hosts = state.storage.clone();
        let have = rows::uploads(&self.env.db, &cid, scope_id, epoch).await?;
        let all: BTreeSet<String> = out.blobs.iter().map(|b| hex::encode(b.sha256)).collect();
        let full: Vec<String> = hosts.iter().filter(|h| all.iter().all(|s| have.contains(&((*h).clone(), s.clone())))).cloned().collect();
        let need = MIN_HOSTS.min(hosts.len()).max(1);
        if full.len() < need {
            tracing::info!(community = %cid, epoch, holding = full.len(), need, "keeper waits for the hosts before the epoch event");
            let blobs = out.blobs.iter().map(|b| (hex::encode(b.sha256), Arc::new(b.bytes.clone()))).collect();
            tick.jobs.push(self.job(Work::Put { what: What::Epoch { scope_id: scope_id.to_string(), epoch }, hosts, blobs }));
            return Ok(false);
        }
        let Some(json) = rows::event(&self.env.db, &cid, scope_id, epoch).await? else {
            // Signed and queued in one transaction: from now on a row of
            // the event with no row of the outbox means a relay took it.
            let content = EpochContent { event: (*out.event).clone(), blobs: full };
            let signed = sign_epoch(&self.keys, &cid, scope_id, &content)?;
            let json = serde_json::to_string(&signed)?;
            rows::put_event_queued(&self.env.db, &cid, scope_id, epoch, &signed.id.to_hex(), &json, now).await?;
            self.env.outbox.notify_one();
            tick.published_keys.push(PublishedEpoch { scope_id: scope_id.to_string(), key: out.key.clone(), event_json: json });
            tracing::info!(community = %cid, epoch, "keeper queued an epoch event");
            return Ok(false);
        };
        let value: serde_json::Value = serde_json::from_str(&json)?;
        let id = value["id"].as_str().and_then(EventId::parse).ok_or_else(|| MessengerError::Invalid("epoch event id".into()))?;
        if db::outbox::state_of(&self.env.db, &cid, db::outbox::LANE_KEEPER, id.as_hex()).await?.is_some() {
            // Queued: the relays have not taken it yet (the worker says why).
            return Ok(false);
        }
        if !self.sk(scope_id)?.kd_mut().published(epoch) {
            return Err(MessengerError::Invalid("keeper: the epoch published is not the pending one".into()));
        }
        self.save(scope_id).await?;
        // A flat epoch reaches its members by their secrets alone.
        if self.sk(scope_id)?.scheme() == SchemeId::FlatV1 {
            self.issue_flat(log, scope_id, now).await?;
        }
        rows::drop_events(&self.env.db, &cid, scope_id).await?;
        rows::drop_uploads_except(&self.env.db, &cid, scope_id, Some(epoch)).await?;
        // The blobs this epoch makes obsolete: at once under an open
        // history (the chain of the events gives every older key), after
        // the owner's retention under `from_join` (a member offline reads
        // its shard of every epoch).
        let due = match state.history {
            History::Open => now,
            History::FromJoin => now + FROM_JOIN_RETENTION_SECS,
        };
        for sha in &out.retires {
            for host in &hosts {
                rows::retire(&self.env.db, &cid, host, &hex::encode(sha), due).await?;
            }
        }
        tracing::info!(community = %cid, epoch, "keeper published an epoch");
        Ok(true)
    }

    /// A job of this keeper.
    pub(crate) fn job(&self, work: Work) -> Job {
        Job { community_id: self.community_id.clone(), keeper: self.keys.clone(), work, cancel: Arc::default() }
    }

    /// The hosts that missed blobs of the newest published epoch get them
    /// from one that holds them (wire §9: the rest catch up): a job.
    async fn catch_up_hosts(&mut self, scope_id: &str, state: &ChannelState, tick: &mut Tick) -> Result<()> {
        let cid = &self.community_id;
        let Some(sk) = self.scopes.get(scope_id) else { return Ok(()) };
        let Some(head) = sk.published_head().map(|h| h.epoch) else { return Ok(()) };
        let blobs: Vec<String> = sk.published_blobs().iter().map(hex::encode).collect();
        if blobs.is_empty() {
            return Ok(());
        }
        let have = rows::uploads(&self.env.db, cid, scope_id, head).await?;
        let full: Vec<String> = state.storage.iter().filter(|h| blobs.iter().all(|s| have.contains(&((*h).clone(), s.clone())))).cloned().collect();
        let lagging: Vec<String> = state.storage.iter().filter(|h| !full.contains(h)).cloned().collect();
        if full.is_empty() || lagging.is_empty() {
            return Ok(());
        }
        tick.jobs.push(self.job(Work::Copy { what: What::Epoch { scope_id: scope_id.to_string(), epoch: head }, from: full, to: lagging, hashes: blobs }));
        Ok(())
    }

    /// The blobs due to be removed: a job; a host that fails keeps its
    /// row for later.
    async fn collect_garbage(&mut self, now: i64, tick: &mut Tick) -> Result<()> {
        let items = rows::retired_due(&self.env.db, &self.community_id, now).await?;
        if !items.is_empty() {
            tick.jobs.push(self.job(Work::Delete { items }));
        }
        Ok(())
    }

    /// Asks for an epoch now (`rotate_now`, a lost key): built, or the
    /// pending one replaced, on the next turn; of one scope, or of every
    /// scope kept here (`None`).
    pub fn rotate_now(&mut self, scope_id: Option<&str>) {
        match scope_id {
            Some(s) => {
                self.urgent.insert(s.to_string());
            }
            None => self.urgent.extend(self.scopes.keys().cloned()),
        }
    }

    /// Blobs waiting to be removed (for a line of the UI and the tests).
    pub async fn garbage(&mut self) -> Result<i64> {
        rows::retired_count(&self.env.db, &self.community_id).await
    }
}

pub(crate) fn min_some(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The snapshot of the readers as the keeper makes it
//! (internal/communities-wire.md §8, §9): the members of keydist's
//! `everyone` in ascending order of their keys, in shards of 1024, each
//! sealed under the key of the newest published epoch and put on the
//! hosts by the rule of an epoch (one host holds all of them
//! before the event goes out), then kind 1393 with the heads of the
//! journal. Only when the readers changed; no more often than once in five
//! minutes in a big channel, `SETTLE_SECS` after the change in a small one
//! (`SMALL_CHANNEL_READERS`).
//!
//! As the epochs (`cycle`), without network in the turn: built and sealed
//! under the keeper, the shards go to the hosts by a job (`upload`), the
//! event into the outbox once enough hosts hold them, and the row of the
//! snapshot is written once a relay took it. The sealed shards live in
//! memory meanwhile (`Keeper::snapshot_pending`), their progress in the
//! rows under the scope `snapshot`; a restart seals them anew (a fresh
//! nonce), as a turn that failed did before.
//!
//! A snapshot never goes out while an epoch is due or waits to go out if
//! anybody was put out since the newest published epoch was built: it
//! would be sealed under a key a reader just put out still holds, and the
//! list of the readers is what the seal keeps from the relays. Otherwise
//! (an epoch held by hosts that refuse its blobs or relays that refuse its
//! event for longer than `RETRY_SECS`, the window of a newcomer) it goes
//! under the newest published epoch: its readers waited for it behind
//! every such epoch. An epoch merely on its way (two turns) is waited for:
//! a snapshot under the epoch before would be replaced a moment later. One sealed
//! under a key that a newer published epoch replaced meanwhile is dropped
//! (its shards retired, its event withdrawn from the outbox if not sent).

use super::cycle::Tick;
use super::upload::{What, Work, SNAPSHOT_SCOPE};
use super::{Keeper, MIN_HOSTS};
use crate::blobs::sha256_hex;
use crate::db::{self, keeper as rows};
use crate::log::OpLog;
use crate::snapshot::{self as snap, Entry, ShardRef, Snapshot, FLAG_READER, MAX_SHARDS, MIN_INTERVAL_SECS, SHARD_ENTRIES, SNAPSHOT_VERSION};
use crate::state::{ChannelState, SCOPE_EVERYONE};
use crate::wire;
use messenger_core::{MessengerError, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The base level (bit 0), which every reader holds whatever the journal
/// says of them (wire §14.1).
pub const LEVELS_STAGE1: u64 = 1;

/// Below this many readers a change of the readers (a join, a put-out)
/// makes a snapshot `SETTLE_SECS` later instead of waiting out
/// `MIN_INTERVAL_SECS`: every member but the keeper places a reader's
/// comment by the newest snapshot, so a newcomer's first comment waited up
/// to five minutes (живой запуск 2026-10-23: «комментарии читателя никто
/// не видит»). The price is the upload: each snapshot puts every shard
/// (`SHARD_ENTRIES` readers each) on every host again, sealed anew, and
/// members fetch the shards they need anew. Under a thousand readers that
/// is one shard of under 46 KB; a big channel, whose readers change all
/// the time, keeps the floor of five minutes.
pub const SMALL_CHANNEL_READERS: usize = 1000;

/// How long a small channel's snapshot waits after the first change of the
/// readers it names: a burst of joins (a link posted somewhere) makes one
/// snapshot, not one each.
pub const SETTLE_SECS: i64 = 20;

/// What a turn of the cycle did about the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotTurn {
    Published { at: i64, members: u64 },
    /// The readers are those of the last snapshot.
    Unchanged,
    /// The readers changed, too soon after the last one: due at this time.
    TooSoon(i64),
    /// An epoch is due or waits to go out; the snapshot follows it.
    AfterEpoch,
    /// More readers than one snapshot names (`MAX_READERS`): none made.
    TooLarge,
    /// Built and sealed: its shards wait for the hosts (a job runs), or
    /// its event for the relays (a row of the outbox).
    Waiting,
    /// It could not be made: again on the next turn.
    NotOut,
}

/// A snapshot built and sealed, on its way to the hosts and the relays.
pub(crate) struct PendingSnapshot {
    digest: String,
    /// The epoch of `everyone` it is sealed under.
    key_epoch: u32,
    entries: Vec<Entry>,
    shards: Vec<(ShardRef, Arc<Vec<u8>>)>,
    hosts: Vec<String>,
    /// Its event, once queued for the relays.
    queued: Option<QueuedSnapshot>,
}

struct QueuedSnapshot {
    id: String,
    at: i64,
    /// The hosts that held every shard when it was signed.
    full: Vec<String>,
}

impl PendingSnapshot {
    fn hashes(&self) -> Vec<String> {
        self.shards.iter().map(|(r, _)| r.sha256.clone()).collect()
    }
}

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel keeper: {what}"))
}

fn list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

impl Keeper {
    /// The readers as keydist holds them, with the date each was let in.
    /// The levels of each are the journal's (wire §14.2: the entry of a
    /// reader carries them), the base level always among them.
    pub(crate) async fn entries(&mut self, state: &ChannelState) -> Result<Vec<Entry>> {
        let since = rows::readers_since(&self.env.db, &self.community_id).await?;
        let made = rows::keeper(&self.env.db, &self.community_id).await?.map_or(0, |r| r.created_at);
        // The leaves of open tickets are nobody yet (wire §17).
        let tickets = self.ticket_members().await?;
        let sk = self.scopes.get(SCOPE_EVERYONE).ok_or_else(|| invalid("unknown scope"))?;
        // keydist's members are keyed by their 32 bytes: in ascending
        // order already, as a shard wants them.
        Ok(sk
            .members()
            .into_iter()
            .filter(|m| !tickets.contains(m))
            .map(|m| {
                let key = hex::encode(m.0);
                let at = since.get(&key).copied().unwrap_or(made).max(0);
                let levels = state.staff.get(&key).map_or(0, |s| s.levels) | LEVELS_STAGE1;
                Entry { pubkey: m.0, flags: FLAG_READER, levels, since: u32::try_from(at).unwrap_or(u32::MAX) }
            })
            .collect())
    }

    /// What the entries of a snapshot are made of, without reading them:
    /// keydist's membership of `everyone` (a reader let in takes a new
    /// leaf, one put out is a revoked leaf, a generation has its number),
    /// the levels of the staff by the journal, the open tickets; and how
    /// many readers there are.
    async fn readers_mark(&mut self, state: &ChannelState) -> Result<(u64, usize)> {
        use std::hash::{Hash, Hasher};
        let tickets = self.ticket_members().await?;
        let sk = self.scopes.get(SCOPE_EVERYONE).ok_or_else(|| invalid("unknown scope"))?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        if let Some(sd) = sk.sd() {
            let g = sd.generation();
            (g.gen, g.next_leaf, g.revoked.len(), sd.state().gens.len()).hash(&mut h);
        }
        sk.members_len().hash(&mut h);
        for (key, s) in &state.staff {
            (key, s.levels).hash(&mut h);
        }
        for t in &tickets {
            t.0.hash(&mut h);
        }
        Ok((h.finish(), sk.members_len().saturating_sub(tickets.len())))
    }

    /// Who is out of `everyone`, cheaply: the revoked leaves of each live
    /// generation. Changes with every reader put out (a ban, a removal, a
    /// leave, a ticket given up) and with every new generation.
    pub(crate) fn revoked_mark(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let sd = self.scopes.get(SCOPE_EVERYONE)?.sd()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for g in &sd.state().gens {
            (g.gen, g.revoked.len()).hash(&mut h);
        }
        Some(h.finish())
    }

    /// An epoch of `everyone` was just built (or a pending one superseded):
    /// whom it leaves out, for `published_mark` once it is out.
    pub(crate) fn note_build(&mut self, scope_id: &str) {
        if scope_id != SCOPE_EVERYONE {
            return;
        }
        let epoch = self.scopes.get(SCOPE_EVERYONE).and_then(|s| s.pending_epoch());
        self.built_mark = epoch.zip(self.revoked_mark());
    }

    /// Whether a snapshot may go under the newest published epoch while
    /// another is due or pending: nobody waits to be put out by an epoch
    /// (keydist's queue) and nobody was put out since the published one was
    /// built, so that everyone who holds its key is still a reader. Not
    /// known after a restart (no mark): the snapshot waits, as it did.
    fn nobody_out_since_published(&self) -> bool {
        let queued = self.scopes.get(SCOPE_EVERYONE).and_then(|s| s.sd()).map_or(1, |sd| sd.state().schedule.queued);
        queued == 0 && self.published_mark.is_some() && self.published_mark == self.revoked_mark()
    }

    /// The snapshot, when one is due: built and sealed, then on its way
    /// (`snapshot_on`); one pending goes on from where it is.
    pub(crate) async fn snapshot_if_due(&mut self, log: &OpLog, now: i64, tick: &mut Tick) -> Result<SnapshotTurn> {
        let state = log.state();
        let cid = self.community_id.clone();
        let sk = self.sk(SCOPE_EVERYONE)?;
        let behind = sk.pending_epoch().is_some() || sk.due_at().is_some();
        // An epoch on its way (its blobs to the hosts, its event to the
        // relays: two turns and a moment) is waited for; one held longer
        // than that by hosts or relays that refuse it is not.
        let fresh = sk.pending_epoch().is_some() && sk.last_epoch_at().is_none_or(|t| super::secs(now) < t + super::RETRY_SECS);
        let Some(key) = sk.published_head().cloned() else { return Ok(SnapshotTurn::AfterEpoch) };
        if let Some(p) = self.snapshot_pending.take() {
            if p.key_epoch == key.epoch {
                return self.snapshot_on(p, log, now, tick).await;
            }
            // Sealed under a key a newer epoch replaced: a reader that
            // epoch put out could open it. Dropped, and sealed anew below.
            tracing::info!(community = %cid, sealed_under = p.key_epoch, published = key.epoch, "keeper dropped a snapshot sealed under an older epoch");
            if let Some(q) = &p.queued {
                db::outbox::withdraw(&self.env.db, &cid, db::outbox::LANE_KEEPER, &q.id).await?;
            }
            self.retire_now(&p.hosts, &p.hashes(), now).await?;
            rows::drop_uploads_except(&self.env.db, &cid, SNAPSHOT_SCOPE, None).await?;
        }
        if behind && (fresh || !self.nobody_out_since_published()) {
            return Ok(SnapshotTurn::AfterEpoch);
        }
        // Every turn comes here (each note, each operation, each epoch):
        // the rows of the readers are read only when the cheap mark says
        // they may have changed, and not before the interval is over
        // (communities-bench, 2026-10-23: 0.6 s a turn at 100 K readers).
        let (mark, readers) = self.readers_mark(state).await?;
        if readers > MAX_SHARDS * SHARD_ENTRIES {
            tracing::warn!(community = %cid, readers, "more than 64 shards of readers: no snapshot (the index blob is stage 4, wire §8)");
            return Ok(SnapshotTurn::TooLarge);
        }
        let last = rows::snapshot(&self.env.db, &cid).await?;
        if let (Some(l), Some((m, d))) = (last.as_ref(), self.snapshot_memo.as_ref()) {
            if *m == mark && *d == l.digest {
                self.readers_moved = None;
                return Ok(SnapshotTurn::Unchanged);
            }
        }
        // The readers may have moved since the last snapshot: when it is
        // due. A small channel waits for the burst to settle, never past
        // the floor of a big one.
        let moved = *self.readers_moved.get_or_insert(now);
        if let Some(l) = last.as_ref() {
            let floor = l.at + MIN_INTERVAL_SECS;
            let due = if readers < self.limits.small_channel { floor.min(moved + SETTLE_SECS) } else { floor };
            if now < due {
                return Ok(SnapshotTurn::TooSoon(due));
            }
        }
        let entries = self.entries(state).await?;
        let mut h = Sha256::new();
        for c in entries.chunks(SHARD_ENTRIES) {
            h.update(snap::encode_shard(c)?);
        }
        let digest = hex::encode(h.finalize());
        self.snapshot_memo = Some((mark, digest.clone()));
        if last.as_ref().is_some_and(|l| l.digest == digest) {
            self.readers_moved = None;
            return Ok(SnapshotTurn::Unchanged);
        }
        let chunks: Vec<&[Entry]> = entries.chunks(SHARD_ENTRIES).collect();
        if chunks.len() > MAX_SHARDS {
            // Not an error of the cycle: epochs and everything else go on.
            tracing::warn!(community = %cid, readers = entries.len(), "more than 64 shards of readers: no snapshot (the index blob is stage 4, wire §8)");
            return Ok(SnapshotTurn::TooLarge);
        }
        let mut shards: Vec<(ShardRef, Arc<Vec<u8>>)> = Vec::with_capacity(chunks.len());
        for c in chunks {
            let bytes = snap::seal_shard(&cid, &key, c)?;
            shards.push((ShardRef { from: hex::encode(c[0].pubkey), sha256: sha256_hex(&bytes) }, Arc::new(bytes)));
        }
        // The rows of a build before (a restart): its shards are not these.
        rows::drop_uploads_except(&self.env.db, &cid, SNAPSHOT_SCOPE, None).await?;
        let pending = PendingSnapshot { digest, key_epoch: key.epoch, entries, shards, hosts: state.storage.clone(), queued: None };
        self.snapshot_on(pending, log, now, tick).await
    }

    /// A pending snapshot, on: its shards to the hosts (a job) until
    /// enough hold them, then its event into the outbox, then, once the
    /// row is gone, published here.
    async fn snapshot_on(&mut self, mut p: PendingSnapshot, log: &OpLog, now: i64, tick: &mut Tick) -> Result<SnapshotTurn> {
        let cid = self.community_id.clone();
        if let Some(q) = &p.queued {
            if db::outbox::state_of(&self.env.db, &cid, db::outbox::LANE_KEEPER, &q.id).await?.is_some() {
                self.snapshot_pending = Some(p);
                return Ok(SnapshotTurn::Waiting);
            }
            return self.snapshot_published(p, now).await;
        }
        let have = rows::uploads(&self.env.db, &cid, SNAPSHOT_SCOPE, 0).await?;
        let hashes = p.hashes();
        let full: Vec<String> = p.hosts.iter().filter(|h| hashes.iter().all(|s| have.contains(&((*h).clone(), s.clone())))).cloned().collect();
        let need = MIN_HOSTS.min(p.hosts.len()).max(1);
        if full.len() < need {
            let blobs = p.shards.iter().map(|(r, b)| (r.sha256.clone(), b.clone())).collect();
            tick.jobs.push(self.job(Work::Put { what: What::Snapshot { digest: p.digest.clone() }, hosts: p.hosts.clone(), blobs }));
            self.snapshot_pending = Some(p);
            return Ok(SnapshotTurn::Waiting);
        }
        let leaves: Vec<[u8; 32]> = hashes.iter().map(|s| hex::decode(s).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("shard hash"))).collect::<Result<_>>()?;
        let last = rows::snapshot(&self.env.db, &cid).await?;
        let at = last.as_ref().map_or(now, |l| now.max(l.at + 1));
        let s = Snapshot {
            v: SNAPSHOT_VERSION,
            community_id: cid.clone(),
            heads: super::issue_heads(log),
            epoch_ref: BTreeMap::from([(SCOPE_EVERYONE.to_string(), p.key_epoch)]),
            members: p.entries.len() as u64,
            at,
            root: hex::encode(snap::merkle_root(&leaves)),
            shards: p.shards.iter().map(|(r, _)| r.clone()).collect(),
        };
        let event = snap::sign_snapshot(&self.keys, &s)?;
        let id = event.id.to_hex();
        let json = serde_json::to_string(&event)?;
        db::outbox::enqueue_event(&self.env.db, &cid, db::outbox::LANE_KEEPER, wire::KIND_SNAPSHOT, &id, None, &json, now).await?;
        self.env.outbox.notify_one();
        // The member side of this device takes it from here (`Tick::snapshot_event`),
        // its shards opened already: no fetch, no wait for the echo before
        // the comments of a new reader are placed.
        for ((r, _), c) in p.shards.iter().zip(p.entries.chunks(SHARD_ENTRIES)) {
            crate::db::snapshots::put_shard(&self.env.db, &cid, &r.from, &r.sha256, p.key_epoch, c).await?;
        }
        self.snapshot_out = Some(json);
        p.queued = Some(QueuedSnapshot { id, at, full });
        self.snapshot_pending = Some(p);
        Ok(SnapshotTurn::Waiting)
    }

    /// A relay took the snapshot: the shards of the one before the last
    /// retired, its row written.
    async fn snapshot_published(&mut self, p: PendingSnapshot, now: i64) -> Result<SnapshotTurn> {
        let cid = self.community_id.clone();
        let Some(q) = p.queued else { return Ok(SnapshotTurn::NotOut) };
        let hashes: Vec<String> = p.shards.iter().map(|(r, _)| r.sha256.clone()).collect();
        // The last two stay (wire §9): the shards of the one before the
        // last go now.
        let last = rows::snapshot(&self.env.db, &cid).await?;
        let (current, prev) = last.as_ref().map(|l| (list(&l.shards_json), list(&l.prev_shards_json))).unwrap_or_default();
        let gone: Vec<String> = prev.into_iter().filter(|p| !current.contains(p) && !hashes.contains(p)).collect();
        self.retire_now(&p.hosts, &gone, now).await?;
        rows::put_snapshot(
            &self.env.db,
            &rows::SnapshotRow {
                community_id: cid.clone(),
                at: q.at,
                digest: p.digest,
                shards_json: serde_json::to_string(&hashes)?,
                hosts_json: serde_json::to_string(&q.full)?,
                prev_shards_json: serde_json::to_string(&current)?,
            },
        )
        .await?;
        rows::drop_uploads_except(&self.env.db, &cid, SNAPSHOT_SCOPE, None).await?;
        self.readers_moved = None;
        let members = p.entries.len() as u64;
        tracing::info!(community = %cid, members, shards = hashes.len(), "keeper published a snapshot of the readers");
        Ok(SnapshotTurn::Published { at: q.at, members })
    }

    async fn retire_now(&mut self, hosts: &[String], hashes: &[String], now: i64) -> Result<()> {
        for host in hosts {
            for sha in hashes {
                rows::retire(&self.env.db, &self.community_id, host, sha, now).await?;
            }
        }
        Ok(())
    }

    /// The hosts that missed the shards of the last snapshot get them
    /// from one that holds them, as the blobs of an epoch catch up: a job.
    pub(crate) async fn catch_up_snapshot(&mut self, state: &ChannelState, tick: &mut Tick) -> Result<()> {
        let Some(row) = rows::snapshot(&self.env.db, &self.community_id).await? else { return Ok(()) };
        let holders = list(&row.hosts_json);
        let lagging: Vec<String> = state.storage.iter().filter(|h| !holders.contains(h)).cloned().collect();
        if lagging.is_empty() || holders.is_empty() {
            return Ok(());
        }
        let hashes = list(&row.shards_json);
        tick.jobs.push(self.job(Work::Copy { what: What::Snapshot { digest: row.digest }, from: holders, to: lagging, hashes }));
        Ok(())
    }
}

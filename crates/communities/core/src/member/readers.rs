// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Who wrote a message, by the snapshot of the readers
//! (internal/communities-wire.md §8, §11.3): staff by the journal, a
//! reader by the shard of the newest snapshot whose range holds the key
//! (opened once and cached in `cm_readers`), banned and muted by the
//! journal whatever the shard says.
//!
//! An author no snapshot here can tell of yet waits (`Wait`): a reader
//! who came after the newest snapshot is in the next one, which the
//! keeper makes soon after a change (`keeper::snapshot`), and a shard
//! whose epoch's key is not here may open when it comes. A shard not in
//! the cache here waits for the hosts (`Unfetched`): fetched off the way
//! of the event by a job (`fetch::Want::Shard`, `ChannelService::run_fetch`),
//! which reads the comments again once it opens. An author the
//! snapshot leaves out although it was made after the message was
//! written is no reader (`Known(None)`).
//!
//! The device that keeps the channel waits for no snapshot: the rows of
//! its keeper say whom it let in and when (`cm_keeper_readers`), and that
//! anybody else is out (живой запуск 2026-10-23: the comment of a reader
//! who had just come reached the owner minutes later, with the next
//! snapshot).

use super::{fetch, key_of, Member};
use crate::blobs::fetch_verified;
use crate::db::{self, keeper as kept, snapshots};
use crate::log::OpLog;
use crate::roles::Role;
use crate::snapshot::{self as snap, Entry, Snapshot, MIN_INTERVAL_SECS};
use crate::state::{ChannelState, Reader, SCOPE_EVERYONE};
use keydist::EpochKey;
use messenger_core::{PubKey, Result};

/// The mark messages waiting for a snapshot are kept under in
/// `cm_pending`.
pub const SNAPSHOT_MARKER: &str = "snapshot";

/// Before the first snapshot of a channel, any author the journal does
/// not ban counts as a reader this long after its `create` (wire §8).
pub const FIRST_SNAPSHOT_WINDOW_SECS: i64 = MIN_INTERVAL_SECS;

/// What the journal and the snapshot say of the author of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Judged now: a reader since then, or nobody the snapshot names
    /// (staff, banned, an outsider: the journal says the rest).
    Known(Option<Reader>),
    /// Not to be told before the next snapshot.
    Wait,
    /// Shard `index` of the newest snapshot here is not in the cache: told
    /// once a job fetched it from the hosts, or by the next snapshot.
    Unfetched(usize),
}

/// The readers known on this device (`Member::known_readers`): hex key
/// and the time of their join at the keeper, in no order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownReaders {
    pub readers: Vec<(String, i64)>,
    /// A shard of the newest snapshot was not here or did not open: some
    /// readers are missing.
    pub partial: bool,
}

/// Why a shard of a snapshot did not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Miss {
    /// Not here: to be had from the hosts (the key of its epoch is here),
    /// or no host gave it now.
    Hosts,
    /// Not to be opened here now: no such shard, the key of its epoch not
    /// here, bytes that do not open or do not fit its range.
    Closed,
}

impl Member {
    /// The newest snapshot of a channel taken here.
    pub async fn latest_snapshot(&self, community_id: &str) -> Result<Option<Snapshot>> {
        match snapshots::latest(&self.env.db, community_id).await? {
            Some((_, json)) => snap::content_of(&json).map(Some),
            None => Ok(None),
        }
    }

    /// What may be said of `author` for a message dated `created_at`.
    pub async fn reader_of(&self, log: &OpLog, author: &PubKey, created_at: i64) -> Result<Standing> {
        let state = log.state();
        let cid = &state.community_id;
        if state.is_banned(author) || state.role_of(author).is_some_and(|r| r >= Role::Author) {
            return Ok(Standing::Known(None));
        }
        // My own messages: I hold the keys, so the keeper let me in; the
        // others judge them by their snapshot.
        if author == &self.me()? && self.has_bundle(cid, SCOPE_EVERYONE).await? {
            let since = db::channels::get(&self.env.db, cid).await?.map_or(0, |r| r.created_at);
            return Ok(Standing::Known(Some(Reader { since })));
        }
        // Kept here: the keeper's rows tell it now, with no snapshot.
        let kept_here = self.kept_here(state).await?;
        if kept_here {
            if let Some(since) = kept::reader_since(&self.env.db, cid, author.as_hex()).await? {
                return Ok(Standing::Known(Some(Reader { since })));
            }
        }
        let Some(s) = self.latest_snapshot(cid).await? else {
            let created = log.created_at().unwrap_or(0);
            // The window of a new channel is the same on every device.
            if created_at <= created + FIRST_SNAPSHOT_WINDOW_SECS {
                return Ok(Standing::Known(Some(Reader { since: created })));
            }
            return Ok(if kept_here { Standing::Known(None) } else { Standing::Wait });
        };
        if kept_here {
            return Ok(Standing::Known(None));
        }
        let outside = if s.at >= created_at { Standing::Known(None) } else { Standing::Wait };
        let Some(index) = s.shard_for(author) else { return Ok(outside) };
        let entries = match self.cached_shard(&s, index).await? {
            Ok(e) => e,
            Err(Miss::Hosts) => return Ok(Standing::Unfetched(index)),
            Err(Miss::Closed) => return Ok(Standing::Wait),
        };
        Ok(match snap::lookup(&entries, author) {
            Some(r) => Standing::Known(Some(r)),
            None => outside,
        })
    }

    /// Whether the keeper of the current series is kept on this device:
    /// by its row, not its lock (a turn of the keeper reads content again
    /// while it holds it, `ChannelService::hand_to_me`). The rows of a
    /// keeper the journal has gone past (wire §15) say nothing any more.
    async fn kept_here(&self, state: &ChannelState) -> Result<bool> {
        let Some(current) = state.keeper() else { return Ok(false) };
        Ok(kept::keeper(&self.env.db, &state.community_id).await?.is_some_and(|r| r.keeper_pubkey == current.pubkey.as_hex()))
    }

    /// What a job must fetch for shard `index` of the snapshot dated `at`
    /// (`fetch::Want::Shard`): none when that snapshot is not the newest
    /// here any more, the shard is in the cache already, or the key of its
    /// epoch is not here (the shard waits for it, `keep`).
    pub(crate) async fn plan_shard(&self, state: &ChannelState, at: i64, index: usize) -> Result<Option<fetch::Plan>> {
        let Some(s) = self.latest_snapshot(&state.community_id).await?.filter(|s| s.at == at) else { return Ok(None) };
        match self.cached_shard(&s, index).await? {
            Err(Miss::Hosts) => {}
            Ok(_) | Err(Miss::Closed) => return Ok(None),
        }
        let Some(raw) = s.shards.get(index).and_then(|r| hex::decode(&r.sha256).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()) else { return Ok(None) };
        Ok(Some(fetch::Plan::Shard { sha256: raw, hosts: state.storage.clone() }))
    }

    /// Shard `index` of the snapshot dated `at` opened from what a job
    /// fetched and put in the cache: true when it is there now (the
    /// comments that waited for it are to be read again), false when it
    /// does not open or fit, or the snapshot is not the newest any more.
    pub(crate) async fn open_fetched_shard(&self, community_id: &str, at: i64, index: usize, bytes: &[u8]) -> Result<bool> {
        let Some(s) = self.latest_snapshot(community_id).await?.filter(|s| s.at == at) else { return Ok(false) };
        Ok(self.open_shard(&s, index, bytes).await?.is_ok())
    }

    /// The readers of shard `index` held here: from the cache when its hash
    /// is the one the snapshot names. `Miss::Hosts` when it is to be had
    /// from the hosts (the key of its epoch is here), never fetched here:
    /// the way of an event waits for no host (`fetch`).
    async fn cached_shard(&self, s: &Snapshot, index: usize) -> Result<std::result::Result<Vec<Entry>, Miss>> {
        let cid = &s.community_id;
        let Some(r) = s.shards.get(index) else { return Ok(Err(Miss::Closed)) };
        if let Some(entries) = snapshots::shard(&self.env.db, cid, &r.from, &r.sha256).await? {
            return Ok(Ok(entries));
        }
        let Some(&epoch) = s.epoch_ref.get(SCOPE_EVERYONE) else { return Ok(Err(Miss::Closed)) };
        if self.key_of_epoch(cid, epoch).await?.is_none() {
            return Ok(Err(Miss::Closed));
        }
        Ok(Err(Miss::Hosts))
    }

    /// The bytes of shard `index` (their hash checked by whoever fetched
    /// them) opened with the key of `epoch_ref`, checked against its range
    /// and put in the cache.
    async fn open_shard(&self, s: &Snapshot, index: usize, bytes: &[u8]) -> Result<std::result::Result<Vec<Entry>, Miss>> {
        let cid = &s.community_id;
        let Some(r) = s.shards.get(index) else { return Ok(Err(Miss::Closed)) };
        let Some(&epoch) = s.epoch_ref.get(SCOPE_EVERYONE) else { return Ok(Err(Miss::Closed)) };
        let Some(key) = self.key_of_epoch(cid, epoch).await? else { return Ok(Err(Miss::Closed)) };
        let entries = match snap::open_shard(cid, &key, bytes, &r.sha256) {
            Ok(e) if s.check_entries(index, &e).is_ok() => e,
            _ => {
                tracing::warn!(community = %cid, "a shard of the snapshot does not open or does not fit its range");
                return Ok(Err(Miss::Closed));
            }
        };
        snapshots::put_shard(&self.env.db, cid, &r.from, &r.sha256, epoch, &entries).await?;
        Ok(Ok(entries))
    }

    /// The readers of shard `index`: from the cache, else fetched from the
    /// hosts (hash checked) and opened. Only off the way of an event (the
    /// keeper recovered, `all_readers`).
    async fn shard(&self, s: &Snapshot, index: usize, hosts: &[String]) -> Result<std::result::Result<Vec<Entry>, Miss>> {
        match self.cached_shard(s, index).await? {
            Err(Miss::Hosts) => {}
            done => return Ok(done),
        }
        let Some(raw) = s.shards.get(index).and_then(|r| hex::decode(&r.sha256).ok()).and_then(|b| <[u8; 32]>::try_from(b).ok()) else { return Ok(Err(Miss::Closed)) };
        let bytes = match fetch_verified(self.env.blobs.as_ref(), hosts, &raw).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(community = %s.community_id, error = %e, "a shard of the snapshot is not to be had now");
                return Ok(Err(Miss::Hosts));
            }
        };
        self.open_shard(s, index, &bytes).await
    }

    /// Every reader the newest snapshot here names, every shard opened (a
    /// device of the owner recovering the keeper, wire §15): `None` when
    /// a shard cannot be had now; empty before the first snapshot.
    pub async fn all_readers(&self, log: &OpLog) -> Result<Option<Vec<Entry>>> {
        let state = log.state();
        let Some(s) = self.latest_snapshot(&state.community_id).await? else { return Ok(Some(Vec::new())) };
        let mut out = Vec::new();
        for index in 0..s.shards.len() {
            match self.shard(&s, index, &state.storage).await? {
                Ok(entries) => out.extend(entries),
                Err(_) => return Ok(None),
            }
        }
        Ok(Some(out))
    }

    /// Every reader known here, for the list the team reads
    /// (`ChannelService::readers`): the keeper's rows when it is kept on
    /// this device (current), else the shards of the newest snapshot in
    /// the cache. A shard not in the cache is left out and wanted of the
    /// hosts by a job (`fetch::Want::Shard`, as a comment would want it:
    /// no round trip here, the page asks again), and the list says it is
    /// `partial`; so is one whose shard does not open here. Empty before
    /// the first snapshot.
    pub async fn known_readers(&self, log: &OpLog) -> Result<KnownReaders> {
        let state = log.state();
        let cid = &state.community_id;
        if self.kept_here(state).await? {
            let rows = kept::readers_since(&self.env.db, cid).await?;
            return Ok(KnownReaders { readers: rows.into_iter().collect(), partial: false });
        }
        let Some(s) = self.latest_snapshot(cid).await? else { return Ok(KnownReaders::default()) };
        let mut out = KnownReaders::default();
        for index in 0..s.shards.len() {
            match self.cached_shard(&s, index).await? {
                Ok(entries) => out.readers.extend(entries.iter().map(|e| (hex::encode(e.pubkey), i64::from(e.since)))),
                Err(Miss::Hosts) => {
                    out.partial = true;
                    self.env.fetches.want(cid, fetch::Want::Shard { at: s.at, index });
                }
                Err(Miss::Closed) => out.partial = true,
            }
        }
        Ok(out)
    }

    async fn key_of_epoch(&self, community_id: &str, epoch: u32) -> Result<Option<EpochKey>> {
        let rows = db::keys::epochs(&self.env.db, community_id, SCOPE_EVERYONE).await?;
        rows.iter().find(|r| r.epoch == i64::from(epoch)).and_then(|r| r.key_hex.as_deref()).map(|k| key_of(epoch, k)).transpose()
    }
}

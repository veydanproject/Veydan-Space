// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What this device holds of the archive and what it must write
//! (`internal/messenger-archive-plan.md` §4.2): every event the runtime
//! received is kept by segment in `msg_archive_events`, a segment with
//! events the storage may lack is dirty, and the handler of the sync
//! entities asks which are due, builds their files and merges the files
//! other devices wrote.
//!
//! **The owner.** The archive is the store's one identity's
//! (`msg_identity`): its public key is the first part of every entity id
//! and sits in the header of every file, so a storage two identities share
//! holds both archives side by side and a device restores its own (D13).
//! Without an identity nothing is due and nothing is queued.
//!
//! **The order of a file.** A segment is append-only on the wire: an event
//! gets its place (`seq`) the first time a file holding it is built here
//! or read from the storage, and keeps it. A build writes the events
//! placed before in their order and appends the others by `(at, id)`; a
//! read of the storage's file puts its events first, in its order, and
//! this device's own after them. So a month that grew is the file the
//! storage had plus new records at the end, whichever device writes it,
//! and only the last chunk of its large file changes. Compaction is the
//! one thing that takes records out of the middle.
//!
//! **Writers.** Every device writes; there is no leader (D3). Before it
//! writes a segment the storage holds, a device merges the storage's file
//! ([`Archive::merge`]): it keeps and replays what it lacked (D7) and
//! uploads only when its union differs from the file. The loser of a race
//! of two writers finds the file lacking its own events and writes them
//! with the next pass: a read that lacks a live event of this device
//! leaves the segment dirty.

use crate::segment::{self, RawRead};
use crate::{
    format_error, stops_replay, too_large, ArchiveMarks, ArchiveScope, ArchiveSink, ArchivedEvent, Body, Month, Replayed,
    Replayer, SegmentId, SegmentMeta, AT_SKEW_SECS, FLUSH_INTERVAL_SECS, MAX_RECORDS, MAX_RECORD_BYTES, MAX_SEGMENT_BYTES,
};
use async_trait::async_trait;
use messenger_core::{envelope, Clock, MessengerError, PubKey, Result, Timestamp};
use messenger_store::archive::{self as repo, EventRow};
use messenger_store::Store;
use std::collections::HashSet;
use std::sync::Arc;

/// Events marked replayed in one transaction.
const REPLAY_BATCH: usize = 500;

/// The archive of this device over the messenger's store.
#[derive(Clone)]
pub struct Archive {
    pub(crate) store: Store,
    pub(crate) clock: Arc<dyn Clock>,
}

/// A segment file ready to upload, and the summary its op carries.
#[derive(Clone, Debug)]
pub struct Built {
    pub bytes: Vec<u8>,
    pub meta: SegmentMeta,
}

/// What a merge with the storage's file did.
#[derive(Clone, Debug)]
pub struct Merged {
    /// The events of the file this device lacked, replayed.
    pub replay: ReplayReport,
    /// The file to upload when this device's union differs from the
    /// storage's (it holds more, the file broke off, or compaction dropped
    /// what the file still has); `None` when the storage holds it all.
    pub upload: Option<Built>,
}

/// What a replay of a segment did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    pub applied: u64,
    /// Had already: by the ingress, or held by the archive before.
    pub duplicate: u64,
    pub ignored: u64,
    /// The handler refused the event; it is left behind with a warning.
    pub failed: u64,
    /// The file broke off; what came before was replayed.
    pub truncated: bool,
}

impl ReplayReport {
    fn count(&mut self, r: Replayed) {
        match r {
            Replayed::Applied => self.applied += 1,
            Replayed::Duplicate => self.duplicate += 1,
            Replayed::Ignored => self.ignored += 1,
        }
    }
}

pub(crate) fn invalid(what: impl std::fmt::Display) -> MessengerError {
    MessengerError::Invalid(format!("archive: {what}"))
}

/// The row of an event: the record is kept as `json`, which is what a
/// file holds of it.
fn row_of(event: &ArchivedEvent, scope: &str, month: &str, json: String) -> EventRow {
    EventRow {
        id: event.id.as_hex().to_string(),
        scope: scope.to_string(),
        month: month.to_string(),
        at: event.at.secs(),
        received_at: event.received_at.secs(),
        kind: i64::from(event.kind),
        msg_id: match &event.body {
            Body::Dm { inbound } => Some(inbound.rumor_id.as_hex().to_string()),
            Body::Wire { .. } => None,
        },
        event_json: json,
    }
}

/// The note of my own devices with the keys of a community's channels
/// (`crates/communities`, wire §6.4): the whole set each time, tens of
/// kilobytes, sent again with every change — the one note that makes an
/// archive big (live run 2026-11-05: 962 of them were 92 % of 18 MB).
const T_OWN_CHANNEL_KEYS: &str = "own.channel_keys";

/// The state a note of my own devices is a snapshot of (plan D17), or
/// `None` for an event that is history and is kept whatever came after
/// it. Of the notes with one key only the newest is worth a file: the
/// keys of a community's channels (by community), the read position of a
/// chat (by chat), the map of emoji, the presence, the profile, the
/// layout — each as a whole. Read from the record as the file holds it.
pub fn supersede_key(event_json: &str) -> Option<String> {
    let event: ArchivedEvent = serde_json::from_str(event_json).ok()?;
    let Body::Dm { inbound } = event.body else { return None };
    if inbound.rumor_kind != envelope::KIND_OWN_RUMOR {
        return None;
    }
    let content: serde_json::Value = serde_json::from_str(&inbound.content).ok()?;
    let t = content.get("t")?.as_str()?;
    let by = |field: &str| content.get(field).and_then(|v| v.as_str()).map(|v| format!("{t}:{v}"));
    match t {
        T_OWN_CHANNEL_KEYS => by("community_id"),
        envelope::T_OWN_READ => by("chat"),
        envelope::T_OWN_EMOJI | envelope::T_OWN_PRESENCE | envelope::T_OWN_PROFILE | envelope::T_OWN_UI => Some(t.to_string()),
        _ => None,
    }
}

/// Why the file of a segment of `scope` holding `size` would be over the
/// bounds of the format, its header counted; `None` when it is not.
pub(crate) fn over_bounds(scope: &ArchiveScope, size: &repo::LiveSize) -> Option<String> {
    let file = segment::header_len(scope) + size.bytes as u64;
    if size.count as usize > MAX_RECORDS {
        Some(format!("{} events", size.count))
    } else if size.longest as usize > MAX_RECORD_BYTES {
        Some(format!("a record of {} bytes", size.longest))
    } else if file > MAX_SEGMENT_BYTES {
        Some(format!("a file of {file} bytes"))
    } else {
        None
    }
}

/// The rows of a file, in its order; a record is kept byte for byte. A
/// record this build does not understand is a format error: the file is
/// not merged into, lest a rewrite lose what it holds.
fn rows_of(raw: &RawRead<'_>) -> Result<Vec<EventRow>> {
    let scope = raw.scope.as_key();
    let month = raw.month.to_string();
    raw.records
        .iter()
        .map(|r| {
            let event: ArchivedEvent = serde_json::from_slice(r).map_err(|e| format_error(format!("record: {e}")))?;
            let json = String::from_utf8(r.to_vec()).map_err(|_| format_error("record is not utf-8"))?;
            Ok(row_of(&event, &scope, &month, json))
        })
        .collect()
}

impl Archive {
    pub fn new(store: Store, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now().secs()
    }

    /// Whose archive this is: the store's identity. `None` before a key
    /// was made or imported.
    pub async fn owner(&self) -> Result<Option<PubKey>> {
        Ok(messenger_store::identity::get(&self.store).await?.and_then(|row| PubKey::parse(&row.pubkey_hex)))
    }

    /// The segments to write at `now`: a dirty open month once
    /// [`FLUSH_INTERVAL_SECS`] passed since it was last written (at once
    /// when it never was), a dirty closed month at once. `force` takes
    /// every dirty segment: the app exits or goes to the background.
    /// Nothing without an identity.
    pub async fn due(&self, now: Timestamp, force: bool) -> Result<Vec<SegmentId>> {
        let Some(owner) = self.owner().await? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for st in repo::dirty(&self.store).await? {
            let (Some(scope), Some(month)) = (ArchiveScope::parse(&st.scope), Month::parse(&st.month)) else {
                continue;
            };
            let due = force
                || month.is_closed_at(now)
                || st.written_at.is_none_or(|at| at + FLUSH_INTERVAL_SECS <= now.secs());
            if due {
                out.push(SegmentId { owner: owner.clone(), scope, month });
            }
        }
        Ok(out)
    }

    /// The file of a segment as this device holds it, compacted first
    /// (D5: every rewrite drops what was deleted), in the order of the
    /// module doc. Places the events not placed yet. A month over the
    /// bounds of the format is refused ([`crate::too_large`]).
    pub async fn build(&self, seg: &SegmentId) -> Result<Built> {
        let (scope, month) = (seg.scope.as_key(), seg.month.to_string());
        repo::drop_dead(&self.store, &scope, &month).await?;
        repo::drop_superseded(&self.store, &scope, &month, &supersede_key).await?;
        // Over the bounds of the format the month is not written: it is
        // told before its file is made in memory.
        let size = repo::live_size(&self.store, &scope, &month).await?;
        if let Some(why) = over_bounds(&seg.scope, &size) {
            return Err(too_large(format!("{}: {why}", seg.entity_id())));
        }
        let rows = repo::order_for_write(&self.store, &scope, &month).await?;
        let bytes = segment::write_records(&seg.owner, &seg.scope, seg.month, rows.iter().map(|r| r.event_json.as_bytes()))?;
        let meta = SegmentMeta {
            owner: seg.owner.as_hex().to_string(),
            scope,
            month,
            count: rows.len() as u64,
            first_at: rows.iter().map(|r| r.at).min().unwrap_or(0),
            last_at: rows.iter().map(|r| r.at).max().unwrap_or(0),
            size: bytes.len() as u64,
            hash: segment::hash(&bytes),
        };
        Ok(Built { bytes, meta })
    }

    /// The months (`yyyy-mm`) this device holds and cannot write, over the
    /// bounds of the format ([`Archive::build`] refuses them): they are in
    /// no backup, and the UI says so. Such a month stays dirty.
    pub async fn unwritable(&self) -> Result<Vec<String>> {
        let mut months: Vec<String> = repo::dirty_sizes(&self.store)
            .await?
            .into_iter()
            .filter(|(scope, _, size)| ArchiveScope::parse(scope).is_some_and(|scope| over_bounds(&scope, size).is_some()))
            .map(|(_, month, _)| month)
            .collect();
        months.dedup();
        Ok(months)
    }

    /// Compaction of one segment (D5), for the monthly pass over closed
    /// segments that deletions touched: how many events were dropped. The
    /// segment is dirty then, and the next collect writes it.
    pub async fn compact(&self, seg: &SegmentId) -> Result<u64> {
        let (scope, month) = (seg.scope.as_key(), seg.month.to_string());
        let dead = repo::drop_dead(&self.store, &scope, &month).await?;
        let superseded = repo::drop_superseded(&self.store, &scope, &month, &supersede_key).await?;
        Ok(dead + superseded)
    }

    /// Merge the storage's file of `seg` into this device's (§4.2, D3,
    /// D7): its events this device lacks are kept and replayed through
    /// `replayer`, then deletions they brought are compacted, and the file
    /// to upload is built when the union differs from the storage's.
    /// When it does not, the segment is clean: the storage holds it all.
    /// A file of another owner, scope or month is refused.
    pub async fn merge(&self, seg: &SegmentId, remote: &[u8], replayer: &dyn Replayer) -> Result<Merged> {
        let raw = segment::read_raw(remote)?;
        if raw.owner != seg.owner || raw.scope != seg.scope || raw.month != seg.month {
            return Err(invalid(format!("the file of {}/{}/{} is not of {}", raw.owner.short(), raw.scope, raw.month, seg.entity_id())));
        }
        let hash = segment::hash(remote);
        let rows = self.absorb(&raw, &hash).await?;
        let mut replay = ReplayReport { truncated: raw.truncated, ..Default::default() };
        self.replay_held(seg, replayer, &mut replay).await?;
        let remote_ids: Vec<String> = rows.into_iter().map(|r| r.id).collect();
        let upload = self.union_after(seg, &remote_ids, &hash, raw.truncated).await?;
        Ok(Merged { replay, upload })
    }

    /// The merge of a version this device holds already (it wrote it, or
    /// read it: [`Archive::holds`]) from what is held, without the file:
    /// what of it is not yet replayed goes through `replayer`, and the
    /// union is built when it differs from that version. `None` when the
    /// version is not held.
    pub async fn merge_held(&self, seg: &SegmentId, hash: &str, replayer: &dyn Replayer) -> Result<Option<Merged>> {
        let (scope, month) = (seg.scope.as_key(), seg.month.to_string());
        let Some(st) = repo::state(&self.store, &scope, &month).await? else { return Ok(None) };
        let count = if st.remote_hash.as_deref() == Some(hash) {
            st.remote_count
        } else if st.written_hash.as_deref() == Some(hash) {
            st.written_count
        } else {
            return Ok(None);
        };
        let mut replay = ReplayReport::default();
        self.replay_held(seg, replayer, &mut replay).await?;
        let file_ids = repo::ids_in_file(&self.store, &scope, &month, count.unwrap_or(0)).await?;
        let upload = self.union_after(seg, &file_ids, hash, false).await?;
        Ok(Some(Merged { replay, upload }))
    }

    /// After a merge: compaction, then the file to upload when the live
    /// events differ from `file_ids` (the storage's file), or the storage
    /// is told to hold it all.
    async fn union_after(&self, seg: &SegmentId, file_ids: &[String], hash: &str, truncated: bool) -> Result<Option<Built>> {
        let (scope, month) = (seg.scope.as_key(), seg.month.to_string());
        repo::drop_dead(&self.store, &scope, &month).await?;
        let file: HashSet<&str> = file_ids.iter().map(String::as_str).collect();
        let held = repo::ids(&self.store, &scope, &month).await?;
        let live: HashSet<&str> = held.iter().filter(|(_, dropped)| !dropped).map(|(id, _)| id.as_str()).collect();
        if truncated || live != file {
            Ok(Some(self.build(seg).await?))
        } else {
            repo::set_written(&self.store, &scope, &month, file_ids.len() as i64, hash, self.now(), false).await?;
            Ok(None)
        }
    }

    /// Whether this device holds the version `hash` of a segment: it
    /// wrote it, or read it from the storage.
    pub async fn holds(&self, seg: &SegmentId, hash: &str) -> Result<bool> {
        let st = repo::state(&self.store, &seg.scope.as_key(), &seg.month.to_string()).await?;
        Ok(st.is_some_and(|st| st.written_hash.as_deref() == Some(hash) || st.remote_hash.as_deref() == Some(hash)))
    }

    /// The file of `meta` is in the storage now, written by this device.
    /// The segment is clean unless what it would write now differs: an
    /// event that came since the build, one that compaction dropped (the
    /// sets are compared through the file they make, never the counts).
    pub async fn written(&self, seg: &SegmentId, meta: &SegmentMeta) -> Result<()> {
        let now = self.build(seg).await?;
        repo::set_written(
            &self.store,
            &seg.scope.as_key(),
            &seg.month.to_string(),
            meta.count as i64,
            &meta.hash,
            self.now(),
            now.meta.hash != meta.hash,
        )
        .await
    }

    /// The storage's file could not be merged (a format this build does
    /// not read): the segment is not due again before the flush interval
    /// passes, so that the file is not downloaded by every cycle.
    pub async fn defer(&self, seg: &SegmentId) -> Result<()> {
        repo::defer(&self.store, &seg.scope.as_key(), &seg.month.to_string(), self.now()).await
    }

    /// Every segment counts as not written: the storage is another than
    /// the one they were written to (a new binding). The written and read
    /// versions are forgotten, every segment with events is dirty and due
    /// by the usual rule; the events and their places in the files stay,
    /// so the files come out as before.
    pub async fn forget_written(&self) -> Result<()> {
        repo::forget_written(&self.store).await
    }

    /// The latest `at` the archive holds of a scope, each capped at the
    /// time it was received plus [`AT_SKEW_SECS`]: where the catch-up
    /// with the relays starts (§4.4, `since` = this − 2 days).
    pub async fn high_water(&self, scope: &ArchiveScope) -> Result<Option<Timestamp>> {
        Ok(repo::high_water(&self.store, &scope.as_key(), AT_SKEW_SECS).await?.map(Timestamp))
    }

    /// The scope is gone here (a group deleted, a server removed): its
    /// live events, segments and queue go; the tombstones of compaction
    /// stay, so that the storage's files, merged again by a group joined
    /// or a server added anew, do not bring back what was deleted.
    /// Returns the entity ids of its segments, newest first, for the
    /// handler to publish their deletion when the scope is gone for good.
    pub async fn remove_scope(&self, scope: &ArchiveScope) -> Result<Vec<String>> {
        let owner = self.owner().await?;
        let key = scope.as_key();
        let months = repo::remove_scope(&self.store, &key).await?;
        let Some(owner) = owner else { return Ok(Vec::new()) };
        Ok(months
            .into_iter()
            .filter_map(|m| Month::parse(&m))
            .map(|month| SegmentId { owner: owner.clone(), scope: scope.clone(), month }.entity_id())
            .collect())
    }

    /// Replay the events of `seg` kept from the storage and not yet
    /// through the ingress, by `(at, id)`; each is marked once the
    /// replayer took it, in batches, so a process killed half way resumes
    /// close to where it stopped (an event taken twice is a duplicate to
    /// the ingress). A refusal of the runtime for now (`NotLoggedIn`
    /// between sessions, a group or a server the device has no row for)
    /// and a failure of the store stop the replay where it is; an event
    /// the handler refuses is left behind with a warning and counted
    /// `failed`, so the rest of the month comes.
    pub(crate) async fn replay_held(&self, seg: &SegmentId, replayer: &dyn Replayer, report: &mut ReplayReport) -> Result<()> {
        let held = repo::unreplayed(&self.store, &seg.scope.as_key(), &seg.month.to_string()).await?;
        let mut batch: Vec<String> = Vec::with_capacity(REPLAY_BATCH.min(held.len()));
        for (id, json) in held {
            match serde_json::from_str::<ArchivedEvent>(&json) {
                Ok(event) => match replayer.replay(event).await {
                    Ok(r) => report.count(r),
                    // The handler could not run, or refuses the scope for
                    // now: what it took is marked, this event and the rest
                    // are not.
                    Err(e) if stops_replay(&e) => {
                        repo::mark_replayed(&self.store, &batch).await?;
                        return Err(e);
                    }
                    Err(e) => {
                        tracing::warn!(id = %id, error = %e, "archive: the handler refused a kept record; left behind");
                        report.failed += 1;
                    }
                },
                Err(e) => {
                    tracing::warn!(id = %id, error = %e, "archive: a kept record does not parse; skipped");
                    report.ignored += 1;
                }
            }
            batch.push(id);
            if batch.len() >= REPLAY_BATCH {
                repo::mark_replayed(&self.store, &batch).await?;
                batch.clear();
            }
        }
        repo::mark_replayed(&self.store, &batch).await
    }

    /// Keep the records of a file read from the storage (the order of the
    /// file, the file as the storage's); returns its rows.
    pub(crate) async fn absorb(&self, raw: &RawRead<'_>, hash: &str) -> Result<Vec<EventRow>> {
        let rows = rows_of(raw)?;
        repo::absorb(&self.store, &raw.scope.as_key(), &raw.month.to_string(), &rows, hash).await?;
        Ok(rows)
    }
}

#[async_trait]
impl ArchiveMarks for Archive {
    /// The runtime's view of `high_water`: a store that does not answer
    /// reads as an empty archive, and the catch-up starts from the
    /// beginning, as without one.
    async fn high_water(&self, scope: &ArchiveScope) -> Option<Timestamp> {
        match Archive::high_water(self, scope).await {
            Ok(mark) => mark,
            Err(e) => {
                tracing::warn!(scope = %scope, error = %e, "archive: the high-water mark was not read");
                None
            }
        }
    }
}

#[async_trait]
impl ArchiveSink for Archive {
    /// Keep an event the runtime received: once by id, its segment dirty.
    async fn record(&self, event: ArchivedEvent) -> Result<()> {
        let (scope, month) = (event.scope.as_key(), Month::of(event.at).to_string());
        let json = serde_json::to_string(&event)?;
        let row = row_of(&event, &scope, &month, json);
        repo::record(&self.store, &row).await?;
        Ok(())
    }
}


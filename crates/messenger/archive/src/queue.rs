// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The order of the restore (`internal/messenger-archive-plan.md` §4.4,
//! §5): every segment an applied op announces that this device does not
//! hold yet is queued, and the handler downloads and replays them one by
//! one, the chat on screen first, then by priority:
//!
//! - `meta` above everything (the snapshot of relations, small, needed
//!   before the list of chats makes sense);
//! - then the newest month first, so the current and the previous month
//!   of every scope come before the first screen and the rest in the
//!   background;
//! - within a month, personal chats before groups.
//!
//! The queue lives in `msg_archive_queue`, by version (the hash of the
//! file): an item is done only once every event of the file went through
//! the ingress, so a process killed in the middle resumes from the events
//! still held (check 6 of §7), without the file again. An item the
//! runtime refuses for now is parked (a group or a server this device has
//! no row for: tried again when one comes), one this build cannot read is
//! refused (tried again by another build); neither holds the restore up.

use crate::collect::{invalid, Archive, ReplayReport};
use crate::{segment, ArchiveScope, Replayer, SegmentId, SegmentMeta};
use messenger_core::Result;
use messenger_store::archive::{self as repo, QueueRow, QUEUE_DONE, QUEUE_FAILED, QUEUE_PARKED, QUEUE_REFUSED};

pub use messenger_store::archive::QueueProgress;

/// A segment to download and replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub entity_id: String,
    pub segment: SegmentId,
    pub count: u64,
    /// Bytes of the plaintext file, for the progress bar.
    pub size: u64,
    /// sha256 of the file the op announced, hex.
    pub hash: String,
}

/// What [`Archive::enqueue`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enqueued {
    /// Queued now: a segment new to the queue, or a newer version of one
    /// in it.
    Queued,
    /// The queue knows this version already (pending, failed, parked,
    /// refused or done): nothing changed.
    Known,
    /// This device holds that very version: it wrote it, or read it from
    /// the storage. With `unreplayed`, events of it still await the
    /// ingress: the item is pending, to be replayed from what is held
    /// without the file; without, nothing is to do.
    Held { unreplayed: bool },
}

/// Above every month of every other scope.
const META_PRIORITY: i64 = 1 << 40;

/// The priority of a segment in the queue: the higher the sooner.
pub fn priority(seg: &SegmentId) -> i64 {
    let month = i64::from(seg.month.year) * 12 + i64::from(seg.month.month) - 1;
    match seg.scope {
        ArchiveScope::Meta => META_PRIORITY + month,
        ArchiveScope::Dm | ArchiveScope::DmServer { .. } => month * 4 + 2,
        ArchiveScope::Group { .. } => month * 4 + 1,
    }
}

fn item_of(row: QueueRow) -> Option<QueueItem> {
    Some(QueueItem {
        segment: SegmentId::parse(&row.entity_id)?,
        entity_id: row.entity_id,
        count: u64::try_from(row.count).unwrap_or(0),
        size: u64::try_from(row.size).unwrap_or(0),
        hash: row.hash,
    })
}

impl Archive {
    /// An op of `messenger_segment` was applied: queue its file unless
    /// this device holds that very version (then only what of it is not
    /// replayed yet is queued, from what is held). A newer version of a
    /// queued segment replaces the older one.
    pub async fn enqueue(&self, entity_id: &str, meta: &SegmentMeta) -> Result<Enqueued> {
        let seg = SegmentId::parse(entity_id).ok_or_else(|| invalid(format!("segment id {entity_id}")))?;
        let (scope, month) = (seg.scope.as_key(), seg.month.to_string());
        let row = QueueRow {
            entity_id: entity_id.to_string(),
            priority: priority(&seg),
            scope: scope.clone(),
            month: month.clone(),
            count: meta.count as i64,
            size: meta.size as i64,
            hash: meta.hash.clone(),
            status: String::new(),
            error: None,
            build: String::new(),
        };
        if self.holds(&seg, &meta.hash).await? {
            if !repo::has_unreplayed(&self.store, &scope, &month).await? {
                return Ok(Enqueued::Held { unreplayed: false });
            }
            repo::upsert_pending(&self.store, &row, self.now(), true).await?;
            return Ok(Enqueued::Held { unreplayed: true });
        }
        Ok(match repo::upsert_pending(&self.store, &row, self.now(), false).await? {
            true => Enqueued::Queued,
            false => Enqueued::Known,
        })
    }

    /// The next segment to replay: one of `open` (the chat on screen)
    /// first, then by [`priority`]. It stays pending until [`done`],
    /// [`failed`], [`park`] or [`refuse`].
    ///
    /// [`done`]: Archive::done
    /// [`failed`]: Archive::failed
    /// [`park`]: Archive::park
    /// [`refuse`]: Archive::refuse
    pub async fn dequeue(&self, open: Option<&ArchiveScope>) -> Result<Option<QueueItem>> {
        let open = open.map(ArchiveScope::as_key);
        Ok(repo::next(&self.store, open.as_deref()).await?.and_then(item_of))
    }

    /// The version `hash` of the segment is replayed whole. A row that
    /// names a newer version by now is left pending.
    pub async fn done(&self, entity_id: &str, hash: &str) -> Result<()> {
        repo::set_status(&self.store, entity_id, hash, QUEUE_DONE, None, "", self.now()).await
    }

    /// The segment could not be had (the storage refused it, the ingress
    /// broke off); [`retry_failed`] puts it back.
    ///
    /// [`retry_failed`]: Archive::retry_failed
    pub async fn failed(&self, entity_id: &str, hash: &str, error: &str) -> Result<()> {
        repo::set_status(&self.store, entity_id, hash, QUEUE_FAILED, Some(error), "", self.now()).await
    }

    /// The runtime refuses the segment for now (`group_unknown`,
    /// `server_unknown`); [`retry_parked`] puts it back when a row of the
    /// group or the server may have come.
    ///
    /// [`retry_parked`]: Archive::retry_parked
    pub async fn park(&self, entity_id: &str, hash: &str, why: &str) -> Result<()> {
        repo::set_status(&self.store, entity_id, hash, QUEUE_PARKED, Some(why), "", self.now()).await
    }

    /// The file is of a format `build` does not read; [`retry_refused`]
    /// puts it back for another build.
    ///
    /// [`retry_refused`]: Archive::retry_refused
    pub async fn refuse(&self, entity_id: &str, hash: &str, error: &str, build: &str) -> Result<()> {
        repo::set_status(&self.store, entity_id, hash, QUEUE_REFUSED, Some(error), build, self.now()).await
    }

    pub async fn retry_failed(&self) -> Result<u64> {
        repo::retry(&self.store, QUEUE_FAILED, self.now()).await
    }

    pub async fn retry_parked(&self) -> Result<u64> {
        repo::retry(&self.store, QUEUE_PARKED, self.now()).await
    }

    /// Segments refused by another build than `build` go back to pending.
    pub async fn retry_refused(&self, build: &str) -> Result<u64> {
        repo::retry_refused(&self.store, build, self.now()).await
    }

    /// The segments the restore does not wait for: parked and refused.
    pub async fn blocked(&self) -> Result<Vec<String>> {
        repo::blocked(&self.store).await
    }

    /// The months (`yyyy-mm`) of the queue refused as over the bounds of
    /// the format ([`crate::too_large`]: the file, a record, the count of
    /// records), read or told by the op: not restored here.
    pub async fn refused_too_large(&self) -> Result<Vec<String>> {
        let mut months: Vec<String> = repo::refused(&self.store)
            .await?
            .into_iter()
            .filter(|(_, _, error)| crate::is_too_large_reason(error))
            .map(|(_, month, _)| month)
            .collect();
        months.dedup();
        Ok(months)
    }

    /// The scopes with a segment still to come (pending, failed or
    /// parked), as their keys.
    pub async fn restoring_scopes(&self) -> Result<Vec<String>> {
        repo::restoring_scopes(&self.store).await
    }

    /// Segments and bytes done and in all, for `archive.progress`.
    pub async fn progress(&self) -> Result<QueueProgress> {
        repo::progress(&self.store).await
    }

    /// Whether `id` is a private server of this device: the segments of
    /// `dm@<id>` are restored only then; the handler parks them before
    /// their file comes down otherwise.
    pub async fn has_server(&self, id: &str) -> Result<bool> {
        Ok(messenger_store::servers::get(&self.store, id).await?.is_some_and(|s| s.is_private()))
    }

    /// The groups the store's identity has a row of: only their secrets
    /// are this archive's to publish (a box of another identity's groups
    /// left by an import of another key is not).
    pub async fn group_ids(&self) -> Result<Vec<String>> {
        Ok(messenger_store::groups::list(&self.store).await?.into_iter().map(|g| g.id).collect())
    }

    /// The scopes forgotten here since the last call — a private server
    /// removed, by hand or by the snapshot of another device; a group
    /// forgotten — taken once: the handler resets its positions of their
    /// segments.
    pub async fn take_forgotten_scopes(&self) -> Result<Vec<ArchiveScope>> {
        Ok(repo::take_forgotten(&self.store).await?.iter().filter_map(|k| ArchiveScope::parse(k)).collect())
    }

    /// The restore is over: forget what was done, so the next one counts
    /// from zero.
    pub async fn clear_done(&self) -> Result<()> {
        repo::clear_done(&self.store).await
    }

    /// Replay a downloaded segment file of `item`: its records are kept
    /// (the device can write and merge the month later), then those it
    /// did not hold go through `replayer` by `(at, id)`. Events it held
    /// already count as duplicates without a call. A file cut short
    /// replays what came before the cut. A file that is not the segment
    /// and version the item names is refused. The caller marks the item
    /// [`done`] once this returns.
    ///
    /// [`done`]: Archive::done
    pub async fn replay_segment(&self, item: &QueueItem, bytes: &[u8], replayer: &dyn Replayer) -> Result<ReplayReport> {
        let raw = segment::read_raw(bytes)?;
        let seg = &item.segment;
        if raw.owner != seg.owner || raw.scope != seg.scope || raw.month != seg.month {
            return Err(invalid(format!("the file is of {}/{}/{}, not of {}", raw.owner.short(), raw.scope, raw.month, item.entity_id)));
        }
        let hash = segment::hash(bytes);
        if hash != item.hash {
            return Err(invalid(format!("the file of {} is not the version its op names", item.entity_id)));
        }
        let rows = self.absorb(&raw, &hash).await?;
        let mut report = ReplayReport { truncated: raw.truncated, ..Default::default() };
        self.replay_held(seg, replayer, &mut report).await?;
        // The rest of the file was held already (an event the ingress
        // recorded, or one a merge replayed before).
        let called = report.applied + report.duplicate + report.ignored + report.failed;
        report.duplicate += (rows.len() as u64).saturating_sub(called);
        Ok(report)
    }

    /// Replay what is held of a version this device holds already
    /// ([`Archive::holds`]): the events of the segment not yet through
    /// the ingress, without the file. `None` when the version is not held
    /// (the file is to be downloaded).
    pub async fn replay_held_of(&self, item: &QueueItem, replayer: &dyn Replayer) -> Result<Option<ReplayReport>> {
        if !self.holds(&item.segment, &item.hash).await? {
            return Ok(None);
        }
        let mut report = ReplayReport::default();
        self.replay_held(&item.segment, replayer, &mut report).await?;
        Ok(Some(report))
    }
}

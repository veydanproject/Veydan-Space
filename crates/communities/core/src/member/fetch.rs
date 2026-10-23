// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The member's round trips to the blob hosts, off the loop and off every
//! lock: an epoch whose key opens by a shard (`Status::Read`, wire §6.2)
//! and a shard of the snapshot of the readers that names the author of a
//! comment (`Standing::Unfetched`, wire §8). The way of an event only
//! notes what it wants (`Fetches::want`) and parks what waits for it (the
//! epoch row without a key, the content in `cm_pending`); whoever runs the
//! service runs the job (`ChannelService::run_fetch`: the module in a task
//! of its own, the testkit inline): what to fetch is read from the rows,
//! the bytes come from the hosts with no lock held, and only what they open
//! is written under the channel's lock (живой запуск 2026-10-23: one epoch
//! or shard fetched inline cost 34–42 s of the loop, every minute, on a
//! phone whose hosts answered slowly).
//!
//! A job that fails is tried again later, `RETRY_FIRST_SECS` doubling up to
//! `RETRY_MAX_SECS`, and given up after `TRIES` (what it waits for comes
//! again with the next issue, note of my own or event); one whose key came
//! another way (an issue, a chain, a note of my own) is forgotten and
//! fetches nothing.

use crate::blobs::{fetch_verified, BlobHosts};
use keydist::shard::ShardIndex;
use messenger_core::{MessengerError, Result};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The first wait after a job that failed; doubled after each failure.
pub const RETRY_FIRST_SECS: i64 = 15;
/// The longest wait between two tries of a job.
pub const RETRY_MAX_SECS: i64 = 600;
/// Tries of a job before it is given up (about 35 minutes of them).
pub const TRIES: u32 = 8;
/// Jobs running at once, in all and of one channel.
pub const AT_ONCE: usize = 4;
pub const AT_ONCE_PER_CHANNEL: usize = 2;

/// What a job fetches.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Want {
    /// The index blob and the shard of my leaf of an epoch of a scope,
    /// which open its key.
    Epoch { scope_id: String, epoch: u32 },
    /// Shard `index` of the snapshot of the readers dated `at`.
    Shard { at: i64, index: usize },
}

/// One round trip of the member's side to the hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchJob {
    pub community_id: String,
    pub want: Want,
}

impl FetchJob {
    /// The key a job is known by (one of a key at a time).
    pub fn key(&self) -> String {
        let cid = &self.community_id;
        match &self.want {
            Want::Epoch { scope_id, epoch } => format!("{cid}:fetch:epoch:{scope_id}:{epoch}"),
            Want::Shard { at, index } => format!("{cid}:fetch:shard:{at}:{index}"),
        }
    }
}

/// What a job fetches, as the rows say when it runs: the hashes and the
/// hosts.
#[derive(Clone, Debug)]
pub(crate) enum Plan {
    /// The index blob, then the shard it names at `shard`.
    Epoch { index: [u8; 32], shard: u32, hosts: Vec<String> },
    /// One blob.
    Shard { sha256: [u8; 32], hosts: Vec<String> },
}

/// What the hosts gave.
#[derive(Clone, Debug)]
pub(crate) enum Got {
    Epoch { index: Vec<u8>, shard: Vec<u8> },
    Shard(Vec<u8>),
}

/// The download of a plan: every GET of a job, with no lock held.
pub(crate) async fn download(blobs: &dyn BlobHosts, plan: &Plan) -> Result<Got> {
    match plan {
        Plan::Epoch { index, shard, hosts } => {
            let index = fetch_verified(blobs, hosts, index).await?;
            let hashes = ShardIndex::from_bytes(&index).map_err(|e| MessengerError::Invalid(format!("keydist: {e}")))?.hashes;
            let hash = hashes.get(*shard as usize).ok_or_else(|| MessengerError::Invalid("channel member: shard number".into()))?;
            let shard = fetch_verified(blobs, hosts, hash).await?;
            Ok(Got::Epoch { index, shard })
        }
        Plan::Shard { sha256, hosts } => Ok(Got::Shard(fetch_verified(blobs, hosts, sha256).await?)),
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Not before (unix seconds).
    next_at: i64,
    /// Tries that failed.
    failed: u32,
    running: bool,
}

/// The jobs wanted, by channel and what they fetch: one of each at a
/// time, each with its time. A leaf (a std mutex, never held across an
/// await), shared by every `Member` of the service (`Env::fetches`).
#[derive(Clone, Default)]
pub struct Fetches(Arc<Mutex<BTreeMap<(String, Want), Entry>>>);

impl Fetches {
    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<(String, Want), Entry>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A job wanted, due at once; one wanted already keeps its time (a
    /// second comment for a shard its hosts did not give asks them no
    /// sooner).
    pub(crate) fn want(&self, community_id: &str, want: Want) {
        self.map().entry((community_id.to_string(), want)).or_insert(Entry { next_at: 0, failed: 0, running: false });
    }

    /// Nothing to fetch any more (the key came another way, the snapshot
    /// is not the newest): forgotten; a job of it running finds nothing to
    /// do when it ends.
    pub(crate) fn forget(&self, community_id: &str, want: &Want) {
        self.map().remove(&(community_id.to_string(), want.clone()));
    }

    /// Every job of a channel forgotten (it is not followed here any more).
    pub(crate) fn forget_channel(&self, community_id: &str) {
        self.map().retain(|(cid, _), _| cid != community_id);
    }

    /// The jobs due at `now`, marked running, within `AT_ONCE` and
    /// `AT_ONCE_PER_CHANNEL` with those running already; epochs before
    /// shards, the newest epoch of a channel first (under an open history
    /// its chain gives the ones before it).
    pub(crate) fn take(&self, now: i64) -> Vec<FetchJob> {
        let mut map = self.map();
        let mut running = map.values().filter(|e| e.running).count();
        let mut per: BTreeMap<String, usize> = BTreeMap::new();
        for (cid, _) in map.iter().filter(|(_, e)| e.running).map(|(k, _)| k) {
            *per.entry(cid.clone()).or_default() += 1;
        }
        let mut due: Vec<(String, Want)> = map.iter().filter(|(_, e)| !e.running && e.next_at <= now).map(|(k, _)| k.clone()).collect();
        due.sort_by(|(c1, w1), (c2, w2)| {
            let rank = |w: &Want| match w {
                Want::Epoch { epoch, .. } => (0, -i64::from(*epoch)),
                Want::Shard { index, .. } => (1, i64::try_from(*index).unwrap_or(i64::MAX)),
            };
            c1.cmp(c2).then(rank(w1).cmp(&rank(w2)))
        });
        let mut jobs = Vec::new();
        for (cid, want) in due {
            if running >= AT_ONCE {
                break;
            }
            let n = per.entry(cid.clone()).or_default();
            if *n >= AT_ONCE_PER_CHANNEL {
                continue;
            }
            *n += 1;
            running += 1;
            if let Some(e) = map.get_mut(&(cid.clone(), want.clone())) {
                e.running = true;
            }
            jobs.push(FetchJob { community_id: cid, want });
        }
        jobs
    }

    /// A job ended with nothing more to do: gone.
    pub(crate) fn done(&self, job: &FetchJob) {
        self.map().remove(&(job.community_id.clone(), job.want.clone()));
    }

    /// A job failed: tried again after its wait, or given up after
    /// `TRIES`. False when given up.
    pub(crate) fn failed(&self, job: &FetchJob, now: i64) -> bool {
        let mut map = self.map();
        let key = (job.community_id.clone(), job.want.clone());
        let Some(e) = map.get_mut(&key) else { return true };
        e.failed += 1;
        e.running = false;
        if e.failed >= TRIES {
            map.remove(&key);
            return false;
        }
        let wait = RETRY_FIRST_SECS.saturating_mul(1 << (e.failed - 1).min(16)).min(RETRY_MAX_SECS);
        e.next_at = now + wait;
        true
    }

    /// When the next job not running is due.
    pub fn wake(&self) -> Option<u64> {
        self.map().values().filter(|e| !e.running).map(|e| e.next_at).min().and_then(|t| u64::try_from(t.max(0)).ok())
    }

    /// The keys of the jobs wanted (`FetchJob::key`), running or not.
    pub fn keys(&self) -> Vec<String> {
        self.map().keys().map(|(cid, want)| FetchJob { community_id: cid.clone(), want: want.clone() }.key()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(n: u32) -> Want {
        Want::Epoch { scope_id: "everyone".into(), epoch: n }
    }

    #[test]
    fn jobs_are_bounded_newest_epoch_first_and_back_off() {
        let f = Fetches::default();
        for n in 1..=3 {
            f.want("a", epoch(n));
        }
        f.want("a", Want::Shard { at: 50, index: 0 });
        f.want("b", epoch(1));
        let jobs = f.take(100);
        assert_eq!(jobs.iter().map(FetchJob::key).collect::<Vec<_>>(), ["a:fetch:epoch:everyone:3", "a:fetch:epoch:everyone:2", "b:fetch:epoch:everyone:1"]);
        assert!(f.take(100).is_empty(), "running, and two of a at once");
        // Wanted again while it runs: nothing changes.
        f.want("a", epoch(3));
        assert!(f.failed(&jobs[0], 100));
        assert_eq!(f.wake(), Some(0), "epoch 1 of a is due already");
        let next = f.take(100);
        assert_eq!(next.iter().map(FetchJob::key).collect::<Vec<_>>(), ["a:fetch:epoch:everyone:1"]);
        f.done(&jobs[1]);
        f.done(&next[0]);
        // The backoff: 15, 30, 60 … 600, then given up.
        let mut at = 100;
        let mut waits = vec![15];
        for _ in 1..TRIES {
            let due = f.map().get(&("a".to_string(), epoch(3))).map(|e| e.next_at).unwrap_or_default();
            at = due;
            let job = f.take(at).into_iter().find(|j| j.want == epoch(3)).unwrap_or_else(|| panic!("epoch 3 due at {at}"));
            if !f.failed(&job, at) {
                break;
            }
            waits.push(f.map().get(&("a".to_string(), epoch(3))).map(|e| e.next_at - at).unwrap_or_default());
        }
        assert_eq!(waits, [15, 30, 60, 120, 240, 480, 600]);
        assert!(!f.keys().contains(&"a:fetch:epoch:everyone:3".to_string()), "given up after {TRIES} tries");
        // A key that came another way: forgotten, its end changes nothing.
        f.forget("b", &epoch(1));
        assert!(f.failed(&jobs[2], at), "a job forgotten is not given up, it is gone");
        assert!(f.keys().iter().all(|k| !k.starts_with("b:")));
    }
}

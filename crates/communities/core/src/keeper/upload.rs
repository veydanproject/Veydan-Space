// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The round trips of a keeper to the blob hosts, off every lock: the
//! cycle (`cycle`, `snapshot`) decides what the hosts must hold or lose
//! and hands it out as a [`Job`] (`Tick::jobs`); whoever runs the turn
//! runs the job on its own (`ChannelService::run_job`: the module in a
//! task of its own, the testkit inline), and the next turn reads what
//! the job left in the rows (`cm_keeper_uploads`, `cm_keeper_retired`,
//! the hosts of the snapshot). Nothing here holds the channel's lock or
//! the keeper's mutex (живой запуск 2026-10-23: a turn held both across
//! every put and get, and the loop with them, 40–96 s a wrap).
//!
//! A job is cancelled (`cancel`) when what it uploads is no longer
//! wanted: a pending epoch superseded (`Tick::burnt`). It stops before
//! its next put; what it put stays in the rows and goes with the next
//! `drop_uploads_except`.

use super::Env;
use crate::blobs::fetch_verified;
use crate::db::keeper as rows;
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The scope the shards of a pending snapshot are counted under in
/// `cm_keeper_uploads` (epoch 0): never a scope of the journal, which is
/// `everyone` or 32 hex characters (wire §1).
pub const SNAPSHOT_SCOPE: &str = "snapshot";

/// What a job puts or copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    /// The blobs of an epoch of a scope.
    Epoch { scope_id: String, epoch: u32 },
    /// The shards of the snapshot of the readers with this digest.
    Snapshot { digest: String },
}

impl What {
    /// The scope and epoch its rows of `cm_keeper_uploads` are under.
    pub(crate) fn rows(&self) -> (&str, u32) {
        match self {
            Self::Epoch { scope_id, epoch } => (scope_id, *epoch),
            Self::Snapshot { .. } => (SNAPSHOT_SCOPE, 0),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Epoch { epoch, .. } => format!("epoch {epoch}"),
            Self::Snapshot { .. } => "the snapshot".into(),
        }
    }
}

/// The work of a job.
#[derive(Clone, Debug)]
pub enum Work {
    /// Blobs on every host of `hosts` that lacks them, by the rows.
    Put { what: What, hosts: Vec<String>, blobs: Vec<(String, Arc<Vec<u8>>)> },
    /// Blobs the hosts `from` hold on each host of `to` that lacks them
    /// (wire §9: the rest catch up).
    Copy { what: What, from: Vec<String>, to: Vec<String>, hashes: Vec<String> },
    /// Blobs removed from the hosts: `(host, sha256)` retired and due.
    Delete { items: Vec<(String, String)> },
    /// The upload keys of members revoked on the hosts of an own server
    /// (wire §9.1): `(host, member hex)`.
    Revoke { items: Vec<(String, String)> },
    /// The keys of the staff in the owner's Cloud (wire §9.2): registered
    /// `(member hex, registration)`, revoked `(member hex, key hex)`.
    Cloud { owner: String, puts: Vec<(String, crate::cloud_keys::DelegatePut)>, revokes: Vec<(String, String)> },
}

/// One round trip of a keeper to the hosts, run off every lock.
#[derive(Clone, Debug)]
pub struct Job {
    pub community_id: String,
    /// The keeper's keys; `Env::uploader` puts the user at work over them.
    pub keeper: Keys,
    pub work: Work,
    /// Set when the job is no longer wanted: it stops before its next put.
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    /// The key one running job of a kind is known by (`ChannelService`
    /// runs one such at a time): the channel, the work and what it is of.
    pub fn key(&self) -> String {
        let cid = &self.community_id;
        match &self.work {
            Work::Put { what, .. } => format!("{cid}:put:{}", what_key(what)),
            Work::Copy { what, .. } => format!("{cid}:copy:{}", what_key(what)),
            Work::Delete { .. } => format!("{cid}:delete"),
            Work::Revoke { .. } => format!("{cid}:revoke"),
            Work::Cloud { .. } => format!("{cid}:cloud"),
        }
    }

}

/// The key of the job that puts the blobs of a pending epoch (`Job::key`).
pub fn put_key(community_id: &str, scope_id: &str, epoch: u32) -> String {
    format!("{community_id}:put:{}", what_key(&What::Epoch { scope_id: scope_id.to_string(), epoch }))
}

fn what_key(what: &What) -> String {
    match what {
        What::Epoch { scope_id, epoch } => format!("{scope_id}:{epoch}"),
        What::Snapshot { digest } => format!("snapshot:{digest}"),
    }
}

/// What a job left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Done {
    /// What the hosts said of a `Put`: `Some(Some(why))`, the first host
    /// that refused a blob; `Some(None)`, every host took what it was
    /// given; `None`, nothing to say (a copy, a delete, a put that ended
    /// before every host was asked).
    pub trouble: Option<Option<String>>,
    pub cancelled: bool,
}

/// Runs a job: every put, get and delete of it, the rows written as it
/// goes, so that a crash or a cancel loses nothing done.
pub async fn run(env: &Env, job: &Job) -> Result<Done> {
    let cid = &job.community_id;
    let short = crate::service::short(cid);
    match &job.work {
        Work::Put { what, hosts, .. } => {
            let (scope_id, epoch) = what.rows();
            let have = rows::uploads(&env.db, cid, scope_id, epoch).await?;
            // A host that needs the key of its server, which this device has
            // not (wire §9.1), is refused here and asked nothing.
            let writable = env.writable(&env.uploader(&job.keeper), hosts, Some(cid)).await;
            // The hosts at once, each its blobs in turn (живой запуск
            // 2026-10-23: three small blobs of a first epoch to two hosts,
            // one host after the other, took 15–26 s of the phone, the
            // whole wait for the owner's key): a host that refuses or hangs
            // holds back none of the others.
            let mut ends: Vec<Option<HostEnd>> = vec![None; hosts.len()];
            for (group, chunk) in hosts.chunks(PUT_HOSTS_AT_ONCE).enumerate() {
                let mut running = tokio::task::JoinSet::new();
                for (i, host) in chunk.iter().enumerate() {
                    let (env, job, host) = (env.clone(), job.clone(), host.clone());
                    let mine: BTreeSet<String> = have.iter().filter(|(h, _)| *h == host).map(|(_, s)| s.clone()).collect();
                    let key = writable.iter().find(|(h, _)| *h == host).map(|(_, k)| k.clone());
                    running.spawn(async move { (group * PUT_HOSTS_AT_ONCE + i, put_to(&env, &job, &host, key, mine).await) });
                }
                while let Some(joined) = running.join_next().await {
                    let (i, end) = joined.map_err(|e| MessengerError::Transport(format!("a put to a host stopped: {e}")))?;
                    ends[i] = Some(end?);
                }
            }
            let ends: Vec<HostEnd> = ends.into_iter().flatten().collect();
            if ends.iter().any(|e| e.cancelled) {
                return Ok(Done { trouble: None, cancelled: true });
            }
            // The first host that refused, in the order of the hosts.
            let refused = ends.iter().find_map(|e| e.refused.clone());
            let full = ends.iter().filter(|e| e.full).count();
            let trouble = (refused.is_some() || full == hosts.len()).then_some(refused);
            Ok(Done { trouble, cancelled: false })
        }
        Work::Copy { what, from, to, hashes } => {
            let (scope_id, epoch) = what.rows();
            let snapshot = matches!(what, What::Snapshot { .. });
            let have = if snapshot { BTreeSet::new() } else { rows::uploads(&env.db, cid, scope_id, epoch).await? };
            let writable = env.writable(&env.uploader(&job.keeper), to, Some(cid)).await;
            for (host, key) in &writable {
                let mut all = true;
                for sha in hashes {
                    if have.contains(&(host.clone(), sha.clone())) {
                        continue;
                    }
                    if job.cancel.load(Ordering::SeqCst) {
                        return Ok(Done { trouble: None, cancelled: true });
                    }
                    let raw: [u8; 32] = hex::decode(sha).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| MessengerError::Invalid("blob hash".into()))?;
                    env.say.mark(Some(format!("keeper {short}: a blob of {} copied to {host}", what.name())));
                    let Ok(bytes) = fetch_verified(env.blobs.as_ref(), from, &raw).await else {
                        all = false;
                        break;
                    };
                    if env.blobs.put_keyed(&env.uploader(&job.keeper), key.as_deref(), host, sha, &bytes).await.is_err() {
                        all = false;
                        break;
                    }
                    if !snapshot {
                        rows::add_upload(&env.db, cid, scope_id, epoch, host, sha).await?;
                    }
                }
                // The snapshot names its holders in its row, not blob by blob.
                if all && snapshot {
                    if let Some(mut row) = rows::snapshot(&env.db, cid).await? {
                        let mut holders: Vec<String> = serde_json::from_str(&row.hosts_json).unwrap_or_default();
                        if !holders.contains(host) {
                            holders.push(host.clone());
                            row.hosts_json = serde_json::to_string(&holders)?;
                            rows::put_snapshot(&env.db, &row).await?;
                        }
                    }
                }
            }
            Ok(Done::default())
        }
        Work::Delete { items } => {
            for (host, sha) in items {
                if job.cancel.load(Ordering::SeqCst) {
                    return Ok(Done { trouble: None, cancelled: true });
                }
                env.say.mark(Some(format!("keeper {short}: a blob removed from {host}")));
                match env.delete_blob(&job.keeper, host, sha).await {
                    Ok(()) => rows::retired_done(&env.db, cid, host, sha).await?,
                    Err(e) => tracing::debug!(community = %cid, host, error = %e, "a blob was not removed yet"),
                }
            }
            Ok(Done::default())
        }
        Work::Cloud { owner, puts, revokes } => {
            let Some(cloud) = env.cloud() else { return Ok(Done::default()) };
            use crate::db::cloud_keys as cloud_rows;
            for (who, put) in puts {
                env.say.mark(Some(format!("keeper {short}: the key of the channel for {} registered with the owner's Cloud", &who[..who.len().min(8)])));
                match cloud.put(put).await {
                    Ok(()) => {
                        let row = cloud_rows::MadeRow { community_id: cid.clone(), who: who.clone(), pubkey: put.pubkey.clone(), owner: owner.clone(), expires_at: put.expires_at, sent: 0 };
                        cloud_rows::put_made(&env.db, &row).await?;
                    }
                    Err(e) => {
                        tracing::info!(community = %cid, error = %e, "the key of the channel not registered with the owner's Cloud yet");
                    }
                }
            }
            for (who, pubkey) in revokes {
                env.say.mark(Some(format!("keeper {short}: the key of the channel for {} revoked in the owner's Cloud", &who[..who.len().min(8)])));
                match cloud.revoke(pubkey).await {
                    Ok(()) => cloud_rows::drop_made(&env.db, cid, who).await?,
                    // Not his any more (forgotten, the account deleted): nothing to revoke.
                    Err(e) if e.to_string().contains("delegate_unknown") || e.to_string().contains("not_activated") => cloud_rows::drop_made(&env.db, cid, who).await?,
                    Err(e) => tracing::info!(community = %cid, error = %e, "the key of the channel not revoked in the owner's Cloud yet"),
                }
            }
            // No word for the keeper block: a Cloud that refuses is no host of
            // the channel refusing; the next turn asks again after RETRY_SECS.
            Ok(Done::default())
        }
        Work::Revoke { items } => {
            let label = crate::upload_keys::label_of(cid);
            for (host, who) in items {
                env.say.mark(Some(format!("keeper {short}: the upload keys of {} revoked on {host}", &who[..who.len().min(8)])));
                // The card of the server gone from this device: nothing
                // revokes them here, they end with their time.
                let Some(app_key) = env.book.app_key(host) else {
                    crate::db::upload_keys::drop_made(&env.db, cid, host, who).await?;
                    continue;
                };
                match env.blobs.revoke(&app_key, host, &label, who).await {
                    Ok(()) => crate::db::upload_keys::drop_made(&env.db, cid, host, who).await?,
                    Err(e) => tracing::info!(community = %cid, host, error = %e, "upload keys not revoked yet"),
                }
            }
            Ok(Done::default())
        }
    }
}

/// Hosts a `Put` sends to at once.
pub const PUT_HOSTS_AT_ONCE: usize = 4;

/// How the blobs of a `Put` went to one host.
#[derive(Clone, Debug, Default)]
struct HostEnd {
    /// It holds every blob of the job.
    full: bool,
    /// It refused one: what it said.
    refused: Option<String>,
    cancelled: bool,
}

/// The blobs of a `Put` job to one host, in turn, those it holds (`have`,
/// by the rows) skipped, a row written as each is taken; the first it
/// refuses ends its turn, the cancel flag is read before each.
/// `key`: `None`, the host needs the key of its server and this device has
/// not it: refused at once; `Some(k)`, the key it carries (`None`: none).
async fn put_to(env: &Env, job: &Job, host: &str, key: Option<Option<String>>, mut have: BTreeSet<String>) -> Result<HostEnd> {
    let Work::Put { what, blobs, .. } = &job.work else { return Ok(HostEnd::default()) };
    let Some(key) = key else {
        return Ok(HostEnd { refused: Some(format!("{host}: {}", crate::upload_keys::NEEDS_KEY)), ..HostEnd::default() });
    };
    let cid = &job.community_id;
    let (scope_id, epoch) = what.rows();
    let short = crate::service::short(cid);
    for (sha, bytes) in blobs {
        if have.contains(sha) {
            continue;
        }
        if job.cancel.load(Ordering::SeqCst) {
            return Ok(HostEnd { cancelled: true, ..HostEnd::default() });
        }
        env.say.mark(Some(format!("keeper {short}: a blob of {} ({} KB) to {host}", what.name(), bytes.len() / 1024)));
        match env.blobs.put_keyed(&env.uploader(&job.keeper), key.as_deref(), host, sha, bytes).await {
            Ok(()) => {
                rows::add_upload(&env.db, cid, scope_id, epoch, host, sha).await?;
                have.insert(sha.clone());
            }
            Err(e) => return Ok(HostEnd { refused: Some(format!("{host} did not take a blob of {}: {e}", what.name())), ..HostEnd::default() }),
        }
    }
    Ok(HostEnd { full: blobs.iter().all(|(s, _)| have.contains(s)), ..HostEnd::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{World, H1, H2};

    fn put_job(cid: &str, hosts: &[&str], n: usize) -> Job {
        let blobs = (0..n)
            .map(|i| {
                let bytes = format!("blob {i}").into_bytes();
                (crate::blobs::sha256_hex(&bytes), Arc::new(bytes))
            })
            .collect();
        let what = What::Epoch { scope_id: crate::state::SCOPE_EVERYONE.into(), epoch: 1 };
        Job { community_id: cid.into(), keeper: Keys::generate(), work: Work::Put { what, hosts: hosts.iter().map(|h| (*h).to_string()).collect(), blobs }, cancel: Arc::default() }
    }

    /// The hosts take the blobs at once, each in turn (живой запуск
    /// 2026-10-23: one host after the other, 15–26 s for a first epoch on a
    /// phone): two slow hosts take about the time of one, and a host that
    /// refuses holds back none of the others.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_hosts_take_the_blobs_at_once_and_a_refusal_holds_back_none() {
        let w = World::new();
        let node = w.node().await;
        let cid = "c".repeat(64);
        // A host that never answers: both hosts were asked while it hangs.
        let gate = Arc::new(tokio::sync::Notify::new());
        *w.hosts.hang.lock().unwrap_or_else(|e| e.into_inner()) = Some(gate.clone());
        let env = node.svc.env.clone();
        let job = put_job(&cid, &[H1, H2], 3);
        let hanging = tokio::spawn(async move { run(&env, &job).await });
        let asked = || w.hosts.signers.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(h, _)| h.clone()).collect::<BTreeSet<_>>();
        for _ in 0..200 {
            if asked().len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(asked().len(), 2, "both hosts asked while the first put hangs");
        *w.hosts.hang.lock().unwrap_or_else(|e| e.into_inner()) = None;
        gate.notify_waiters();
        let done = hanging.await.unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done, Done { trouble: Some(None), cancelled: false });
        assert_eq!((w.hosts.count(H1), w.hosts.count(H2)), (3, 3));

        // Slow hosts: three blobs to each in about the time of three puts.
        let slow = std::time::Duration::from_millis(150);
        *w.hosts.put_delay.lock().unwrap_or_else(|e| e.into_inner()) = Some(slow);
        let cid2 = "d".repeat(64);
        let started = std::time::Instant::now();
        let done = run(&node.svc.env, &put_job(&cid2, &[H1, H2], 3)).await.unwrap_or_else(|e| panic!("{e}"));
        let took = started.elapsed();
        assert_eq!(done.trouble, Some(None));
        assert!(took >= slow * 3 && took < slow * 5, "two hosts in about one host's time (six puts one by one: {:?}): {took:?}", slow * 6);
        let rows = rows::uploads(&node.svc.env.db, &cid2, crate::state::SCOPE_EVERYONE, 1).await.unwrap_or_default();
        assert_eq!(rows.len(), 6, "a row for each blob on each host");

        // H1 refuses: H2 takes them all the same, and the refusal is said.
        *w.hosts.put_delay.lock().unwrap_or_else(|e| e.into_inner()) = None;
        w.hosts.refusing.lock().unwrap_or_else(|e| e.into_inner()).insert(H1.to_string());
        let cid3 = "e".repeat(64);
        let done = run(&node.svc.env, &put_job(&cid3, &[H1, H2], 2)).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(done.trouble.as_ref().is_some_and(|t| t.as_ref().is_some_and(|t| t.starts_with(H1))), "{done:?}");
        let rows = rows::uploads(&node.svc.env.db, &cid3, crate::state::SCOPE_EVERYONE, 1).await.unwrap_or_default();
        assert_eq!(rows.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), [H2, H2], "the other host took both");
    }
}

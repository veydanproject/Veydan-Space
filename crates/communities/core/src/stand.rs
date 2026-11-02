// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the keeper of the stand (`communities-cli keeper`, ROO-397 §5,
//! D4) does past the app, and only in the binaries of `tools`: its limits
//! of joins a minute and of readers from the environment, and readers
//! without a device let in by `Keeper::on_join` with a proof of the link,
//! as `communities-bench` lets them in, on the real relay and hosts: their
//! issues go out as notes and the epochs are built and uploaded for them,
//! nobody reads them.
//!
//! `COMMUNITIES_TRIAL_JOINS_PER_MINUTE` (the app: `JOINS_PER_MINUTE`, 120)
//! and `COMMUNITIES_TRIAL_MAX_READERS` (the app: `MAX_READERS`, 65 536);
//! unset, the keeper keeps the app's.

use crate::keeper::{Joined, Keeper};
use crate::link::LinkSecret;
use crate::service::ChannelService;
use crate::state::SCOPE_EVERYONE;
use crate::wire::{Join, JoinProof};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::{Keys, SecretKey};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;

fn bad(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("stand: {what}"))
}

/// The keeper's limits the stand sets; `None`: the app's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tuning {
    pub joins_per_minute: Option<usize>,
    pub max_readers: Option<usize>,
}

impl Tuning {
    pub fn from_env() -> Self {
        let num = |name: &str| std::env::var(name).ok().and_then(|v| v.trim().replace('_', "").parse::<usize>().ok()).filter(|n| *n > 0);
        Self { joins_per_minute: num("COMMUNITIES_TRIAL_JOINS_PER_MINUTE"), max_readers: num("COMMUNITIES_TRIAL_MAX_READERS") }
    }

    /// Onto a keeper as loaded (a keeper loaded anew takes the app's).
    pub fn apply(&self, k: &mut Keeper) {
        if let Some(n) = self.joins_per_minute {
            k.limits.joins_per_minute = n;
        }
        if let Some(n) = self.max_readers {
            k.limits.max_readers = n;
        }
    }

    /// The limits the keeper holds now: joins a minute, readers.
    pub fn now_of(k: &Keeper) -> (usize, usize) {
        (k.limits.joins_per_minute, k.limits.max_readers)
    }
}

/// The key of synthetic reader `i` of channel `community_id`, the same
/// every time (the cleanup of the stand and a ban derive it again).
pub fn synthetic_keys(community_id: &str, i: u64) -> Result<Keys> {
    let mut h = Sha256::new();
    h.update(b"communities-trial reader");
    h.update(community_id.as_bytes());
    h.update(i.to_le_bytes());
    let raw: [u8; 32] = h.finalize().into();
    let sk = SecretKey::from_slice(&raw).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(Keys::new(sk))
}

pub fn synthetic_pk(community_id: &str, i: u64) -> Result<PubKey> {
    let k = synthetic_keys(community_id, i)?;
    PubKey::parse(&k.public_key().to_hex()).ok_or_else(|| bad("pubkey"))
}

/// The link of the channel as the keeper's device holds it: what a
/// synthetic reader proves in its join.
pub struct Door {
    link_epoch: u32,
    secret: LinkSecret,
}

impl Door {
    pub async fn of(svc: &ChannelService, community_id: &str) -> Result<Self> {
        let log = svc.log(community_id).await?.ok_or_else(|| bad("no journal"))?;
        let link_epoch = log.state().link_epoch;
        let secret = crate::secrets::links(svc.env.secrets.as_ref(), community_id).await?.get(&link_epoch).cloned().ok_or_else(|| bad("no link secret"))?;
        Ok(Self { link_epoch, secret })
    }
}

/// What became of the join of one synthetic reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Synthetic {
    /// Let in now, its issue in the outbox.
    Issued,
    /// A reader already, or put out by the journal: nothing sent.
    Skipped,
    /// The keeper takes no more this minute (or waits for a generation at
    /// the wall): asked again later.
    Busy,
    /// Refused (the channel full, the hosts without paths).
    Refused(&'static str),
}

/// Synthetic reader `i` asks to join by the link at `now`, through the
/// keeper's `on_join` as a join note would after the queue.
pub async fn join(k: &Arc<Mutex<Keeper>>, svc: &ChannelService, door: &Door, community_id: &str, i: u64, now: i64) -> Result<Synthetic> {
    let pk = synthetic_pk(community_id, i)?;
    let log = svc.log(community_id).await?.ok_or_else(|| bad("no journal"))?;
    let mut g = k.lock().await;
    // A refusal would send a note: the one in, or banned, is not asked.
    if g.is_member(&pk) || log.state().is_banned(&pk) {
        return Ok(Synthetic::Skipped);
    }
    let join = Join {
        community_id: community_id.to_string(),
        proof: Some(JoinProof { link_epoch: door.link_epoch, mac: door.secret.join_mac(community_id, &pk, door.link_epoch) }),
        admit_id: None,
        created_at: now,
    };
    Ok(match g.on_join(&log, &pk, &join, now).await? {
        Joined::Issued => Synthetic::Issued,
        Joined::Busy | Joined::Held => Synthetic::Busy,
        Joined::Refused(why) => Synthetic::Refused(why),
    })
}

/// The readers the keeper holds in `everyone`.
pub async fn readers(k: &Arc<Mutex<Keeper>>) -> usize {
    k.lock().await.scope_keeper(SCOPE_EVERYONE).map_or(0, |s| s.members_len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::{History, JoinMode, OpBody};
    use crate::service::NewChannel;
    use crate::testkit::{World, H1, H2};
    use std::sync::atomic::Ordering;

    #[test]
    fn a_synthetic_reader_has_the_same_key_every_time_and_one_of_its_own() {
        let a = synthetic_pk("c1", 7).map(|p| p.as_hex().to_string()).ok();
        assert_eq!(a, synthetic_pk("c1", 7).map(|p| p.as_hex().to_string()).ok());
        assert_ne!(a, synthetic_pk("c1", 8).map(|p| p.as_hex().to_string()).ok());
        assert_ne!(a, synthetic_pk("c2", 7).map(|p| p.as_hex().to_string()).ok());
    }

    /// The keeper of the stand lets synthetic readers in at the pace it
    /// is given, each issue a note on the relay; one in or banned is not
    /// asked again, and the app's limits stay where the stand sets none.
    #[tokio::test]
    async fn synthetic_readers_come_in_at_the_pace_of_the_stand_and_once() {
        let w = World::new();
        let owner = w.node().await;
        let new = NewChannel {
            name: "Stand".into(),
            about: String::new(),
            picture: String::new(),
            join: JoinMode::Open,
            history: History::Open,
            relays: vec!["wss://r1.example".into()],
            storage: vec![H1.into(), H2.into()],
        };
        let (cid, _) = owner.svc.create(&owner.keys, new).await.unwrap_or_else(|e| panic!("create: {e}"));
        w.pump(&[&owner]).await;
        let k = owner.svc.keeper(&cid).await.ok().flatten().unwrap_or_else(|| panic!("kept here"));
        let app = Tuning::now_of(&*k.lock().await);
        Tuning::default().apply(&mut *k.lock().await);
        assert_eq!(Tuning::now_of(&*k.lock().await), app, "no knob: the app's limits");
        Tuning { joins_per_minute: Some(5), max_readers: None }.apply(&mut *k.lock().await);
        assert_eq!(Tuning::now_of(&*k.lock().await), (5, app.1));
        let door = Door::of(&owner.svc, &cid).await.unwrap_or_else(|e| panic!("door: {e}"));
        let before = readers(&k).await;
        let wraps = owner.net.wraps.load(Ordering::SeqCst);
        let now = w.clock.now();
        let mut got = Vec::new();
        for i in 0..7 {
            got.push(join(&k, &owner.svc, &door, &cid, i, now).await.unwrap_or_else(|e| panic!("join {i}: {e}")));
        }
        assert_eq!(got[..5], [Synthetic::Issued, Synthetic::Issued, Synthetic::Issued, Synthetic::Issued, Synthetic::Issued]);
        assert_eq!(got[5..], [Synthetic::Busy, Synthetic::Busy], "5 a minute");
        owner.svc.flush_outbox().await.unwrap_or_else(|e| panic!("outbox: {e}"));
        assert_eq!(readers(&k).await, before + 5);
        assert!(owner.net.wraps.load(Ordering::SeqCst) >= wraps + 5, "every issue is a note on the relay");
        // The next minute the rest come; those in are not asked again.
        w.clock.advance(61);
        let now = w.clock.now();
        assert_eq!(join(&k, &owner.svc, &door, &cid, 0, now).await.ok(), Some(Synthetic::Skipped));
        assert_eq!(join(&k, &owner.svc, &door, &cid, 5, now).await.ok(), Some(Synthetic::Issued));
        // A banned one is not asked (a refusal would be a note).
        owner.svc.act(&owner.keys, &cid, OpBody::Ban { who: synthetic_pk(&cid, 6).unwrap_or_else(|e| panic!("{e}")), reason: None }).await.unwrap_or_else(|e| panic!("ban: {e}"));
        assert_eq!(join(&k, &owner.svc, &door, &cid, 6, now).await.ok(), Some(Synthetic::Skipped));
        assert_eq!(readers(&k).await, before + 6);
    }
}

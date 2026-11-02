// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper of a channel moving to another device (see
//! internal/communities-wire.md §15, internal/communities-spec.md §7.5, §7.6, §7.8):
//! what of it leaves with the rows (the masters of the live generations),
//! how the keeper of the next series begins at the epoch its `keeper`
//! operation named, and the emergency generation a device of the owner
//! starts when the keeper before it is gone.

use super::{kd, policy_for, secs, Keeper, ScopeKeeper, TREE};
use crate::db::keeper as rows;
use crate::log::OpLog;
use crate::snapshot::Entry;
use crate::state::{ChannelState, SCOPE_EVERYONE};
use crate::wire;
use crate::secrets;
use keydist::{Change, FlatKeeper, GenerationMaster, KeyDistribution, MemberId, RootSecret, SchemeId, SdKeeper};
use messenger_core::{PubKey, Result};
use std::collections::BTreeMap;

/// What an emergency keeper is made of (spec §7.6): the new root, the
/// readers of the newest snapshot, the newest generation heard of each
/// scope, the first epoch of the series.
#[derive(Clone, Copy)]
pub(crate) struct Recovery<'a> {
    pub root: &'a RootSecret,
    pub readers: &'a [Entry],
    pub gens: &'a BTreeMap<String, u32>,
    pub from_epoch: u32,
}

impl Keeper {
    /// The masters of the live generations of every tree scope, as the
    /// keeper of the next series needs them (spec §7.5): `(scope, gen,
    /// master)`. The root never leaves: a new owner makes his own (§7.8).
    pub(crate) fn live_masters(&self) -> Vec<(String, u32, [u8; 32])> {
        let mut out = Vec::new();
        for (scope_id, sk) in &self.scopes {
            if let Some(sd) = sk.sd() {
                for g in &sd.state().gens {
                    out.push((scope_id.clone(), g.gen, *g.master.as_bytes()));
                }
            }
        }
        out
    }

    /// The keeper of a new series takes over the state of the one before
    /// (wire §15): every scope numbers its next epoch from `from_epoch`; an
    /// epoch built there and not published is built again under that
    /// number, and a scope without one gets one now, so that the readers
    /// hear the new keeper at once.
    pub(crate) async fn start_series(&mut self, from_epoch: u32, now: i64) -> Result<()> {
        for scope_id in self.scope_ids() {
            let Some(sk) = self.scopes.remove(&scope_id) else { continue };
            let pending = sk.pending_epoch();
            let sk = sk.with_next_epoch(from_epoch).map_err(kd)?;
            let header = sk.sd().map(|sd| sd.state().header());
            self.scopes.insert(scope_id.clone(), sk);
            match header {
                Some(h) => rows::write_changes(&self.env.db, &self.community_id, &scope_id, &[Change::Header(h)]).await?,
                None => self.save(&scope_id).await?,
            }
            match pending {
                Some(e) if e < from_epoch => {
                    self.sk(&scope_id)?.kd_mut().supersede_pending(secs(now));
                    self.save(&scope_id).await?;
                    rows::drop_uploads_except(&self.env.db, &self.community_id, &scope_id, None).await?;
                }
                Some(_) => {}
                None => {
                    self.urgent.insert(scope_id.clone());
                }
            }
        }
        rows::drop_events_all(&self.env.db, &self.community_id).await
    }

    /// A bundle again to every member of every scope (an emergency
    /// generation, a new owner's generation): each reads the next epochs at
    /// once instead of asking. A flat scope before its first epoch has
    /// nothing to give yet.
    pub(crate) async fn reissue_all(&mut self, log: &OpLog, now: i64) -> Result<()> {
        for scope_id in self.scope_ids() {
            self.reissue_scope(log, &scope_id, now).await?;
        }
        Ok(())
    }

    /// A bundle again to every member of one scope (`reissue_all`).
    pub(crate) async fn reissue_scope(&mut self, log: &OpLog, scope_id: &str, now: i64) -> Result<()> {
        let tickets = self.ticket_members().await?;
        let sk = self.sk(scope_id)?;
        if sk.scheme() == SchemeId::FlatV1 && sk.published_head().is_none() {
            return Ok(());
        }
        for m in sk.members().into_iter().filter(|m| !tickets.contains(m)) {
            let Some(who) = PubKey::parse(&hex::encode(m.0)) else { continue };
            // A leaf of a migration past the wall: none (they ask again).
            if let Some(why) = self.walled_for(log.state(), scope_id, &m).await {
                tracing::warn!(community = %self.community_id, scope = %scope_id, who = %who.short(), why, "keeper hands out no new leaf");
                continue;
            }
            let issued = self.sk(scope_id)?.kd_mut().bundle_for(m, secs(now)).map_err(kd)?;
            self.save(scope_id).await?;
            self.send_issue(log, &who, &issued, now).await?;
        }
        Ok(())
    }

    /// A planned generation of every tree scope from the root held here (a
    /// new owner's, spec §7.8): the live one stays for `grace_secs` while
    /// the members take bundles of the new one; after it the former owner,
    /// whose root derived the old one, derives nothing the keeper uses.
    ///
    /// A scope with two generations live (the channel was handed over
    /// again within the grace of the handover before: the generation of
    /// the owner before the last and his successor's) gets an emergency
    /// generation of the root held here instead, at once: keydist holds
    /// two generations, so a third planned one cannot be, and the state
    /// used to leave such a scope as it was — the new owner's keeper then
    /// ran on his predecessor's generation, whose master that predecessor
    /// derives from his root for as long as it is current, banned or not
    /// (ROO-400, the review of 21a70d071). Both masters not held here go
    /// into `retired_masters`; the first epoch of the series carries the
    /// new generation alone, and every member gets a bundle of it
    /// (`reissue_all`, after this).
    pub(crate) async fn migrate_all(&mut self, grace_secs: u64, now: i64) -> Result<()> {
        let root = self.root.clone().ok_or_else(|| messenger_core::MessengerError::SecretMissing(secrets::root_name(&self.community_id)))?;
        for scope_id in self.scope_ids() {
            let Some(sk) = self.scopes.remove(&scope_id) else { continue };
            let ScopeKeeper::Sd(mut sd) = sk else {
                self.scopes.insert(scope_id, sk);
                continue;
            };
            let scope = wire::scope_ref(&self.community_id, &scope_id)?;
            let gen = sd.generation().gen.checked_add(1).ok_or_else(|| kd(keydist::Error::Params("generation number")))?;
            if sd.state().gens.len() > 1 {
                let master = GenerationMaster::derive(&root, &scope, SchemeId::SdV1, gen);
                sd.emergency_regenerate(master, secs(now)).map_err(kd)?;
                let changes = sd.take_changes();
                let header = sd.state().header();
                self.scopes.insert(scope_id.clone(), ScopeKeeper::Sd(sd));
                rows::write_changes(&self.env.db, &self.community_id, &scope_id, &changes).await?;
                rows::write_changes(&self.env.db, &self.community_id, &scope_id, &[Change::Header(header)]).await?;
                // The pending epoch of the state handed over, built under
                // the generations now retired, is dropped with its uploads.
                rows::drop_uploads_except(&self.env.db, &self.community_id, &scope_id, None).await?;
                tracing::warn!(community = %self.community_id, scope = %scope_id, gen, "two generations of roots not held here were live: an emergency generation of the new owner's root replaces both");
                continue;
            }
            // keydist takes the grace of a planned migration from its policy:
            // the owner's 7 days for this one, the usual after it.
            let mut st = sd.into_state();
            let before = st.policy.thresholds.clone();
            st.policy.thresholds.grace_secs = grace_secs;
            let mut sd = SdKeeper::from_state(st).map_err(kd)?;
            let master = GenerationMaster::derive(&root, &scope, SchemeId::SdV1, gen);
            sd.migrate(master, None, secs(now)).map_err(kd)?;
            let changes = sd.take_changes();
            let mut st = sd.into_state();
            st.policy.thresholds = before;
            let sd = SdKeeper::from_state(st).map_err(kd)?;
            let header = sd.state().header();
            self.scopes.insert(scope_id.clone(), ScopeKeeper::Sd(sd));
            rows::write_changes(&self.env.db, &self.community_id, &scope_id, &changes).await?;
            rows::write_changes(&self.env.db, &self.community_id, &scope_id, &[Change::Header(header)]).await?;
        }
        Ok(())
    }

    /// The rows of a keeper recovered on a device of the owner whose keeper
    /// is gone (spec §7.6): per scope an emergency generation from `root`
    /// above the newest this device heard of (`gens`), its epochs from
    /// `from_epoch`, with the readers of the newest snapshot (`readers`;
    /// a section gets those the journal gives it). Written here; the
    /// keeper is then loaded from them.
    pub(crate) async fn write_emergency(env: &super::Env, state: &ChannelState, keys: &nostr::key::Keys, r: &Recovery<'_>, now: i64) -> Result<()> {
        let Recovery { root, readers, gens, from_epoch } = *r;
        let cid = &state.community_id;
        rows::delete_keeper(&env.db, cid).await?;
        rows::put_keeper(&env.db, cid, &keys.public_key().to_hex(), now).await?;
        for r in readers {
            rows::reader_in(&env.db, cid, &hex::encode(r.pubkey), i64::from(r.since)).await?;
        }
        for (scope_id, def) in &state.scopes {
            let members: Vec<MemberId> = readers
                .iter()
                .filter(|r| scope_id == SCOPE_EVERYONE || PubKey::parse(&hex::encode(r.pubkey)).is_some_and(|p| state.in_scope(&p, scope_id)))
                .map(|r| MemberId(r.pubkey))
                .collect();
            let scope = wire::scope_ref(cid, scope_id)?;
            match def.scheme {
                SchemeId::FlatV1 => {
                    let mut st = FlatKeeper::new(scope, policy_for(state, scope_id)).state().clone();
                    st.next_epoch = from_epoch;
                    let mut k = FlatKeeper::from_state(st).map_err(kd)?;
                    for m in members {
                        k.join(m, secs(now)).map_err(kd)?;
                    }
                    rows::put_flat(&env.db, cid, scope_id, &serde_json::to_string(k.state())?).await?;
                }
                _ => {
                    let min_gen = gens.get(scope_id).copied().unwrap_or(0);
                    let master = GenerationMaster::derive(root, &scope, SchemeId::SdV1, min_gen.saturating_add(1));
                    let min_epoch = from_epoch.saturating_sub(1);
                    let (mut sd, _) = SdKeeper::recover(scope, policy_for(state, scope_id), TREE, &members, min_gen, min_epoch, master, secs(now)).map_err(kd)?;
                    let changes = sd.take_changes();
                    rows::write_changes(&env.db, cid, scope_id, &changes).await?;
                }
            }
        }
        secrets::put_keeper_keys(env.secrets.as_ref(), cid, keys).await
    }
}

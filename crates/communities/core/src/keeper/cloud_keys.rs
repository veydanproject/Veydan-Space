// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys of the staff in the owner's Veydan Cloud on the keeper's side
//! (internal/communities-wire.md §9.2, cloud-api.md §17). On the owner's
//! device, when his Cloud account is there: each member of the staff whose
//! quota is the owner's (`set_quota`, the default; not banned, not the
//! owner) gets a key of the channel made from the keeper's, registered
//! with the owner's account (a job, off every lock: the calls to the Cloud
//! take the network) and then handed to him sealed (`channel.upload_keys`,
//! field `cloud`); again when less than `RENEW_SECS` of it is left.
//! Whoever is no staff any more, banned, or uploads into his own quota now
//! has the key revoked on the Cloud (his files stay the owner's) and is
//! told to drop it. A device without the owner's account (another admin's,
//! a locked vault, no Cloud) does nothing either way.

use super::upload::{Job, Work};
use super::{Keeper, Tick};
use crate::cloud_keys::{key_for, proof, DelegatePut, RENEW_SECS, TTL_SECS};
use crate::db::cloud_keys::{self as rows, MadeRow};
use crate::log::OpLog;
use crate::upload_keys::label_of;
use crate::wire::{self, CloudKey, UploadKeys};
use messenger_core::{PubKey, Result};
use std::collections::BTreeSet;

/// The same work is not handed out again sooner than this: a Cloud that
/// refuses is not asked on every turn.
pub const RETRY_SECS: i64 = 600;

impl Keeper {
    /// One turn of the keys of the staff in the owner's Cloud.
    pub(crate) async fn sync_cloud_keys(&mut self, log: &OpLog, now: i64, tick: &mut Tick) -> Result<()> {
        let Some(cloud) = self.env.cloud() else { return Ok(()) };
        let state = log.state();
        let owner = state.owner.as_hex().to_string();
        // Only the owner's own account pays; anything else (no account, a
        // locked vault, another person's device) changes nothing.
        if cloud.account().await.as_deref() != Some(owner.as_str()) {
            return Ok(());
        }
        let cid = self.community_id.clone();
        let wanted: BTreeSet<String> = if state.disbanded {
            BTreeSet::new()
        } else {
            state
                .uploaders()
                .into_iter()
                .filter(|p| *p != state.owner && !state.is_banned(p) && !state.quota_own(p))
                .map(|p| p.as_hex().to_string())
                .collect()
        };
        let made = rows::made(&self.env.db, &cid).await?;
        let label = label_of(&cid);
        let mut puts = Vec::new();
        for who in &wanted {
            let key = key_for(self.keys(), &cid, who);
            let pubkey = key.public_key().to_hex();
            let had = made.iter().find(|r| r.who == *who);
            match had {
                Some(r) if r.pubkey == pubkey && r.owner == owner && r.expires_at - now >= RENEW_SECS => {
                    if r.sent == 0 {
                        let to = PubKey::parse(who).ok_or_else(|| messenger_core::MessengerError::Invalid("member".into()))?;
                        let body = UploadKeys {
                            community_id: cid.clone(),
                            keys: Vec::new(),
                            cloud: Some(CloudKey { owner: owner.clone(), secret: key.secret_key().to_secret_hex(), expires_at: r.expires_at }),
                        };
                        self.send_note(Some(state), &to, wire::T_UPLOAD_KEYS, &body, now).await?;
                        rows::put_made(&self.env.db, &MadeRow { sent: 1, ..r.clone() }).await?;
                        tracing::info!(community = %cid, who = %to.short(), "keeper sent the key of the channel for the owner's Cloud");
                    }
                }
                _ => {
                    let expires_at = now + TTL_SECS;
                    puts.push((who.clone(), DelegatePut { pubkey, label: label.clone(), expires_at, per_day: None, proof: proof(&key, &owner, &label)? }));
                }
            }
        }
        let mut revokes = Vec::new();
        for r in made.iter().filter(|r| !wanted.contains(&r.who)) {
            if r.owner != owner {
                // Registered with an account that is not this one: nothing to revoke here.
                rows::drop_made(&self.env.db, &cid, &r.who).await?;
                continue;
            }
            revokes.push((r.who.clone(), r.pubkey.clone()));
        }
        if puts.is_empty() && revokes.is_empty() {
            return Ok(());
        }
        let signature = format!(
            "{}|{}",
            puts.iter().map(|(w, p)| format!("{w}:{}", p.pubkey)).collect::<Vec<_>>().join(","),
            revokes.iter().map(|(w, _)| w.as_str()).collect::<Vec<_>>().join(",")
        );
        if self.cloud_asked.as_ref().is_some_and(|(at, s)| *s == signature && now - at < RETRY_SECS) {
            return Ok(());
        }
        self.cloud_asked = Some((now, signature));
        // Told to drop the key at once: what he uploads with it is refused
        // from the revocation on, and the device goes on with his own.
        for (who, _) in &revokes {
            let Some(to) = PubKey::parse(who) else { continue };
            let body = UploadKeys { community_id: cid.clone(), keys: Vec::new(), cloud: Some(CloudKey { owner: owner.clone(), secret: String::new(), expires_at: 0 }) };
            self.send_note(Some(state), &to, wire::T_UPLOAD_KEYS, &body, now).await?;
        }
        tick.jobs.push(Job { community_id: cid, keeper: self.keys().clone(), work: Work::Cloud { owner, puts, revokes }, cancel: Default::default() });
        Ok(())
    }

    /// A member asked for his bundle again (a new device): his key of the
    /// channel goes again at the next turn.
    pub(crate) async fn resend_cloud_key(&mut self, who: &PubKey) -> Result<()> {
        rows::unsend(&self.env.db, &self.community_id, who.as_hex()).await
    }
}

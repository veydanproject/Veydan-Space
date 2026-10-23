// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The upload keys of the staff on the keeper's side (internal/communities-wire.md
//! §9.1). A host of `storage[]` that is the owner's own server (this device
//! has the app key of its card, and the host takes upload keys) lets the
//! staff write only with an upload key: the keeper makes one for each
//! author and above (not banned, not the owner, who has the card) from the
//! app key, without asking the server, and sends it sealed to that person
//! (`channel.upload_keys`); again when less than `RENEW_SECS` of it is
//! left. Whoever is no staff any more, or a host that left `storage[]`, has
//! the keys revoked on the host by a job (`Work::Revoke`), off every lock.

use super::upload::{Job, Work};
use super::{Keeper, Tick};
use crate::db::upload_keys as rows;
use crate::log::OpLog;
use crate::upload_keys::{label_of, mint, Gate, RENEW_SECS, TTL_SECS};
use crate::wire::{self, UploadKey, UploadKeys, UPLOAD_KEYS_PER_NOTE};
use messenger_core::{PubKey, Result};
use std::collections::{BTreeMap, BTreeSet};

impl Keeper {
    /// One turn of the upload keys: new ones made and sent, those of
    /// whoever is no staff any more handed to a job to revoke.
    pub(crate) async fn sync_upload_keys(&mut self, log: &OpLog, now: i64, tick: &mut Tick) -> Result<()> {
        let state = log.state();
        let cid = self.community_id.clone();
        let staff: BTreeSet<String> = if state.disbanded {
            BTreeSet::new()
        } else {
            state.uploaders().into_iter().filter(|p| *p != state.owner && !state.is_banned(p)).map(|p| p.as_hex().to_string()).collect()
        };
        let made = rows::made(&self.env.db, &cid).await?;
        let revoke: Vec<(String, String)> =
            made.iter().filter(|r| !staff.contains(&r.who) || !state.storage.contains(&r.host)).map(|r| (r.host.clone(), r.who.clone())).collect();
        let label = label_of(&cid);
        let mut out: BTreeMap<String, Vec<UploadKey>> = BTreeMap::new();
        for host in &state.storage {
            let Some(app_key) = self.env.book.app_key(host) else { continue };
            // A kit before upload keys takes none: nothing to make (its staff
            // write elsewhere, wire §9.1).
            if self.env.book.gate(host, now) != Some(Gate::Key { scoped: true }) {
                continue;
            }
            for who in &staff {
                let had = made.iter().find(|r| r.host == *host && r.who == *who);
                if had.is_some_and(|r| r.expires_at - now >= RENEW_SECS) {
                    continue;
                }
                // Later than any key of before, and than its revocation.
                let iat = now.max(had.map_or(0, |r| r.iat + 1));
                let expires_at = iat + TTL_SECS;
                let key = mint(&app_key, host, &label, who, iat, expires_at);
                rows::put_made(&self.env.db, &rows::MadeRow { community_id: cid.clone(), host: host.clone(), who: who.clone(), iat, expires_at }).await?;
                out.entry(who.clone()).or_default().push(UploadKey { host: host.clone(), key, expires_at });
            }
        }
        for (who, keys) in out {
            let Some(to) = PubKey::parse(&who) else { continue };
            for chunk in keys.chunks(UPLOAD_KEYS_PER_NOTE) {
                let body = UploadKeys { community_id: cid.clone(), keys: chunk.to_vec(), cloud: None };
                self.send_note(Some(state), &to, wire::T_UPLOAD_KEYS, &body, now).await?;
            }
            tracing::info!(community = %cid, who = %to.short(), "keeper sent upload keys of the channel's own server");
        }
        if !revoke.is_empty() {
            tick.jobs.push(Job { community_id: cid, keeper: self.keys().clone(), work: Work::Revoke { items: revoke }, cancel: Default::default() });
        }
        Ok(())
    }

    /// A member asked for his bundle again (a new device, wire §6.4): his
    /// upload keys are made again at the next turn, so that device has them too.
    pub(crate) async fn resend_upload_keys(&mut self, who: &PubKey) -> Result<()> {
        for r in rows::made(&self.env.db, &self.community_id).await?.into_iter().filter(|r| r.who == who.as_hex()) {
            rows::put_made(&self.env.db, &rows::MadeRow { expires_at: 0, ..r }).await?;
        }
        Ok(())
    }
}

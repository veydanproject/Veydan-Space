// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys of a channel's own server on the side of the people who write
//! to it (internal/communities-wire.md §9.1): the app keys of the cards of
//! servers this device has (`set_app_keys`, from the messenger), and the
//! upload keys the keeper gave me as staff (`channel.upload_keys`), kept
//! while I am author or above and dropped when I am not.

use crate::db::cloud_keys as cloud_rows;
use crate::db::upload_keys::{self as rows, GivenRow};
use crate::media::may_upload;
use crate::notes::Opened;
use crate::service::{ChannelService, Outcome};
use crate::upload_keys::{label_of, parse, Given};
use crate::wire::UploadKeys;
use messenger_core::Result;
use nostr::key::Keys;
use std::collections::BTreeMap;

impl ChannelService {
    /// The app keys of the cards of servers of this device's user, `(base
    /// address of the media server, key)`, all of them: an upload to such a
    /// host carries its key (wire §9.1).
    pub fn set_app_keys(&self, keys: &[(String, String)]) {
        self.env.book.set_app_keys(keys);
    }

    /// The owner's Veydan Cloud for the keepers kept here (wire §9.2): the
    /// app lends it once its client is there; `None` takes it away.
    pub fn set_cloud(&self, cloud: Option<std::sync::Arc<dyn crate::cloud_keys::CloudDelegates>>) {
        if let Ok(mut held) = self.env.cloud.write() {
            *held = cloud;
        }
    }

    /// `channel.upload_keys`: from the keeper of the current series, to me
    /// as author or above, of hosts of the channel, for my key.
    pub(crate) async fn on_upload_keys(&self, keys: &Keys, n: &Opened, body: &UploadKeys) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(log) = self.log(&body.community_id).await? else { return Ok(out) };
        let state = log.state();
        let me = Self::me(keys)?;
        let from_keeper = body.check().is_ok() && state.keeper().is_some_and(|k| k.pubkey == n.sender);
        // The word to drop the key of the channel for the owner's Cloud is
        // taken from the keeper whatever my role: it comes as I lose it.
        if from_keeper && body.cloud.as_ref().is_some_and(|c| c.secret.is_empty()) {
            self.drop_cloud_key(&body.community_id).await?;
            tracing::info!(community = %crate::service::short(&body.community_id), "the key of the channel for the owner's Cloud dropped, as the keeper said");
        }
        let ok = from_keeper && may_upload(state, &me);
        if !ok {
            out.note("upload keys not from the keeper, or not for the staff");
            return Ok(out);
        }
        let label = label_of(&body.community_id);
        let now = self.now();
        for k in &body.keys {
            let Some(p) = parse(&k.key) else { continue };
            if !state.storage.contains(&k.host) || p.pubkey != me.as_hex() || p.label != label || p.exp <= now {
                out.note("an upload key not of this channel, host or person");
                continue;
            }
            let row = GivenRow { community_id: body.community_id.clone(), host: k.host.clone(), key: k.key.clone(), expires_at: p.exp.min(k.expires_at), received_at: now };
            rows::put_given(&self.env.db, &row).await?;
        }
        if let Some(c) = body.cloud.as_ref().filter(|c| !c.secret.is_empty()) {
            match nostr::key::SecretKey::from_hex(&c.secret) {
                Ok(sk) if c.owner == state.owner.as_hex() && c.expires_at > now => {
                    let key = Keys::new(sk);
                    crate::secrets::put_cloud_key(self.env.secrets.as_ref(), &body.community_id, &key).await?;
                    let row = cloud_rows::GivenRow { community_id: body.community_id.clone(), pubkey: key.public_key().to_hex(), owner: c.owner.clone(), expires_at: c.expires_at, received_at: now };
                    cloud_rows::put_given(&self.env.db, &row).await?;
                }
                _ => out.note("a key for the owner's Cloud not of this channel's owner, or out of its term"),
            }
        }
        self.refresh_given_keys(keys).await?;
        Ok(out)
    }

    /// The key of a channel for its owner's Cloud given to me: gone, from the
    /// book, the rows and the secret store.
    pub(crate) async fn drop_cloud_key(&self, community_id: &str) -> Result<()> {
        self.env.book.drop_cloud(community_id);
        cloud_rows::drop_given(&self.env.db, community_id).await?;
        crate::secrets::forget_cloud_key(self.env.secrets.as_ref(), community_id).await
    }

    /// The upload keys given to me into the book: those expired go, and
    /// those of a channel where I am no author any more (or banned).
    pub(crate) async fn refresh_given_keys(&self, keys: &Keys) -> Result<()> {
        let now = self.now();
        rows::drop_given(&self.env.db, None, now).await?;
        let me = Self::me(keys)?;
        let mut by_channel: BTreeMap<String, Vec<GivenRow>> = BTreeMap::new();
        for r in rows::given(&self.env.db).await? {
            by_channel.entry(r.community_id.clone()).or_default().push(r);
        }
        let mut book = Vec::new();
        for (cid, rows_of) in by_channel {
            let staff = match self.log(&cid).await? {
                Some(log) => may_upload(log.state(), &me) && !log.state().disbanded,
                None => false,
            };
            if !staff {
                rows::drop_given(&self.env.db, Some(&cid), now).await?;
                tracing::info!(community = %crate::service::short(&cid), "upload keys of the channel dropped: no author here any more");
                continue;
            }
            book.extend(rows_of.into_iter().map(|r| Given { community_id: r.community_id, host: r.host, key: r.key, expires_at: r.expires_at }));
        }
        self.env.book.set_given(book);
        // The keys of channels for their owners' Cloud (wire §9.2): kept
        // while I am of the staff in the owner's quota and the key lives.
        let mut cloud = std::collections::HashMap::new();
        for r in cloud_rows::given(&self.env.db).await? {
            let keep = match self.log(&r.community_id).await? {
                Some(log) => {
                    let s = log.state();
                    may_upload(s, &me) && !s.disbanded && !s.quota_own(&me) && s.owner.as_hex() == r.owner && r.expires_at > now
                }
                None => false,
            };
            let key = if keep { crate::secrets::cloud_key(self.env.secrets.as_ref(), &r.community_id).await? } else { None };
            match key.filter(|k| k.public_key().to_hex() == r.pubkey) {
                Some(k) => {
                    cloud.insert(r.community_id.clone(), (k, r.expires_at));
                }
                None => {
                    self.drop_cloud_key(&r.community_id).await?;
                    tracing::info!(community = %crate::service::short(&r.community_id), "the key of the channel for the owner's Cloud dropped: no staff in his quota here any more");
                }
            }
        }
        self.env.book.set_cloud(cloud);
        Ok(())
    }
}

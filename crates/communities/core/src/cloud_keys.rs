// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys of a channel's staff in the owner's Veydan Cloud
//! (internal/communities-wire.md §9.2, services/cloud/spec/cloud-api.md
//! §17): a member of the staff whose quota is the owner's (`set_quota`,
//! the default) uploads the files of posts to the hosts of the channel
//! with a key of the channel, not his own; the owner's Cloud account has
//! that key as its delegate, so the hosts count and keep those files as
//! the owner's. The keeper on the owner's device makes the key (from its
//! own, for the channel and the person: the same on every turn), proves
//! it to the Cloud, registers it with the owner's account and hands its
//! secret to the person (`channel.upload_keys`, field `cloud`).

use async_trait::async_trait;
use crate::upload_keys::hmac_sha256::hmac;
use messenger_core::{MessengerError, Result};
use nostr::key::{Keys, SecretKey};

/// How long a registration lasts, as the keeper asks it.
pub const TTL_SECS: i64 = 30 * 24 * 3600;
/// A registration with less than this left is made again.
pub const RENEW_SECS: i64 = 15 * 24 * 3600;
/// The kind of the event that proves a key agrees to upload for an owner
/// (cloud-api.md §17.1): never published.
pub const PROOF_KIND: u16 = 27236;
/// Its content.
pub const PROOF_CONTENT: &str = "veydan-cloud-delegate/1";
/// What the media server answers a key its owner revoked, or one out of
/// its term: the key of the channel is dropped, the next try goes with the
/// person's own key.
pub const REFUSALS: [&str; 2] = ["delegate_revoked", "delegate_expired"];

/// One registration the keeper asks of the owner's Cloud.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegatePut {
    /// 64 hex: the key of the channel for one person.
    pub pubkey: String,
    /// The label of the channel (`upload_keys::label_of`).
    pub label: String,
    pub expires_at: i64,
    /// Its own bytes a day within the owner's; none.
    pub per_day: Option<u64>,
    /// The event of [`PROOF_KIND`] the key signed.
    pub proof: serde_json::Value,
}

/// The owner's Veydan Cloud, as the app lends it to the keeper: the
/// account of the user at work on this device, and the calls of §17.2.
#[async_trait]
pub trait CloudDelegates: Send + Sync {
    /// The npub (64 hex) of the Cloud account of the user at work, when he
    /// has one that is not suspended; `None` otherwise (nothing to register
    /// into: the staff upload as themselves).
    async fn account(&self) -> Option<String>;
    /// `PUT /cloud/v1/delegates`.
    async fn put(&self, delegate: &DelegatePut) -> Result<()>;
    /// `DELETE /cloud/v1/delegates?pubkey=` (revoked: what it uploaded stays the owner's).
    async fn revoke(&self, pubkey: &str) -> Result<()>;
}

/// The key of the channel `community_id` for the person `who` (hex), made
/// from the keeper's: HMAC-SHA256 of its secret over
/// `"veydan.channel.cloud-key/1\n" ‖ community_id ‖ "\n" ‖ who` (and a
/// counter, in the vanishing case of a value that is no secret key). Every
/// turn and every device of the same keeper makes the same; a new keeper
/// (another series) makes new ones.
pub fn key_for(keeper: &Keys, community_id: &str, who: &str) -> Keys {
    let secret = keeper.secret_key().to_secret_bytes();
    let mut counter = 0u32;
    loop {
        let msg = if counter == 0 {
            format!("veydan.channel.cloud-key/1\n{community_id}\n{who}")
        } else {
            format!("veydan.channel.cloud-key/1\n{community_id}\n{who}\n{counter}")
        };
        if let Ok(sk) = SecretKey::from_slice(&hmac(&secret, msg.as_bytes())) {
            return Keys::new(sk);
        }
        counter += 1;
    }
}

/// The proof of `key` for `owner` and `label` (cloud-api.md §17.1).
pub fn proof(key: &Keys, owner: &str, label: &str) -> Result<serde_json::Value> {
    use nostr::prelude::*;
    let tags = vec![
        Tag::parse(["p", owner]).map_err(|e| MessengerError::Crypto(e.to_string()))?,
        Tag::parse(["d", label]).map_err(|e| MessengerError::Crypto(e.to_string()))?,
    ];
    let event = EventBuilder::new(Kind::from(PROOF_KIND), PROOF_CONTENT).tags(tags).finalize(key).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(serde_json::to_value(event)?)
}

/// Is `error` (a refusal of a media server) the word of a key of the
/// channel that no longer uploads?
pub fn is_refused(error: &str) -> bool {
    REFUSALS.iter().any(|w| error.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_of_a_person_is_the_keepers_word_and_proves_itself() {
        let keeper = Keys::generate();
        let cid = "ab".repeat(32);
        let (w1, w2) = ("01".repeat(32), "02".repeat(32));
        let k1 = key_for(&keeper, &cid, &w1);
        assert_eq!(k1.public_key(), key_for(&keeper, &cid, &w1).public_key(), "the same every turn");
        assert_ne!(k1.public_key(), key_for(&keeper, &cid, &w2).public_key());
        assert_ne!(k1.public_key(), key_for(&keeper, &"cd".repeat(32), &w1).public_key());
        assert_ne!(k1.public_key(), key_for(&Keys::generate(), &cid, &w1).public_key());
        let owner = "ef".repeat(32);
        let p = proof(&k1, &owner, "abab").unwrap();
        let event: nostr::event::Event = serde_json::from_value(p).unwrap();
        event.verify().unwrap();
        assert_eq!((event.kind.as_u16(), event.content.as_str(), event.pubkey), (PROOF_KIND, PROOF_CONTENT, k1.public_key()));
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(tags, [vec!["p".to_string(), owner], vec!["d".to_string(), "abab".to_string()]]);
        assert!(is_refused(r#"403 {"error":"delegate_revoked"}"#) && is_refused("delegate_expired") && !is_refused("plan_limit"));
    }
}

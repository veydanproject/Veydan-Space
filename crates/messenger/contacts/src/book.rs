// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Private address book + public follow list.

use crate::profile::{ProfileService, ProfileView};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use messenger_store::{contacts as repo, Store};
use nostr::key::Keys;
use nostr::nips::nip02::{Contact, ContactListBuilder};
use nostr::prelude::*;
use serde::{Deserialize, Serialize};

pub use messenger_store::contacts::ContactPatch;

/// A contact joined with whatever profile we know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactView {
    pub pubkey: String,
    pub npub: String,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub followed: bool,
    pub profile: Option<ProfileView>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ContactView {
    pub fn label(&self) -> String {
        self.nickname
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.profile.as_ref().map(|p| p.label()))
            .unwrap_or_else(|| format!("{}…{}", &self.npub[..12], &self.npub[self.npub.len() - 4..]))
    }
}

#[derive(Clone)]
pub struct ContactService {
    store: Store,
    profiles: ProfileService,
}

impl ContactService {
    pub fn new(store: Store, profiles: ProfileService) -> Self {
        Self { store, profiles }
    }

    pub async fn list(&self) -> Result<Vec<ContactView>> {
        let rows = repo::list_active(&self.store).await?;
        let follows = repo::follows(&self.store).await?;
        let keys: Vec<PubKey> = rows.iter().filter_map(|r| PubKey::parse(&r.pubkey)).collect();
        let profiles = self.profiles.get_many(&keys).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let profile = profiles.iter().find(|p| p.pubkey == r.pubkey).cloned();
                ContactView {
                    npub: npub_of(&r.pubkey),
                    followed: follows.contains(&r.pubkey),
                    pubkey: r.pubkey,
                    nickname: r.nickname,
                    note: r.note,
                    profile,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                }
            })
            .collect())
    }

    /// How this person is called here: my nickname for them, else what
    /// their profile says, else a short npub.
    pub async fn label_of(&self, pubkey: &PubKey) -> Result<String> {
        Ok(self.face_of(pubkey).await?.0)
    }

    /// The name as `label_of` gives it, and the address of their picture.
    pub async fn face_of(&self, pubkey: &PubKey) -> Result<(String, Option<String>)> {
        if let Some(c) = self.get(pubkey).await? {
            let picture = c.profile.as_ref().and_then(|p| p.picture.clone());
            return Ok((c.label(), picture));
        }
        if let Some(p) = self.profiles.get(pubkey).await? {
            return Ok((p.label(), p.picture.clone()));
        }
        let npub = PublicKey::from_hex(pubkey.as_hex())
            .ok()
            .and_then(|p| p.to_bech32().ok())
            .unwrap_or_else(|| pubkey.as_hex().to_string());
        Ok((format!("{}…{}", &npub[..12.min(npub.len())], &npub[npub.len().saturating_sub(4)..]), None))
    }

    pub async fn get(&self, pubkey: &PubKey) -> Result<Option<ContactView>> {
        Ok(self.list().await?.into_iter().find(|c| c.pubkey == pubkey.as_hex()))
    }

    pub async fn is_contact(&self, pubkey: &PubKey) -> Result<bool> {
        Ok(repo::get(&self.store, pubkey.as_hex()).await?.map(|r| r.deleted_at.is_none()).unwrap_or(false))
    }

    /// Add by hex or npub. Adding yourself is refused.
    pub async fn add(&self, me: &PubKey, key: &str, nickname: Option<&str>) -> Result<ContactView> {
        let pk = parse_key(key)?;
        if &pk == me {
            return Err(MessengerError::Invalid("that is your own key".into()));
        }
        repo::add(&self.store, pk.as_hex(), nickname.map(str::trim).filter(|s| !s.is_empty())).await?;
        self.get(&pk).await?.ok_or_else(|| MessengerError::Storage("contact vanished".into()))
    }

    pub async fn update(&self, pubkey: &PubKey, patch: &ContactPatch) -> Result<ContactView> {
        repo::update(&self.store, pubkey.as_hex(), patch).await?;
        self.get(pubkey).await?.ok_or_else(|| MessengerError::Storage("contact vanished".into()))
    }

    pub async fn remove(&self, pubkey: &PubKey) -> Result<()> {
        repo::remove(&self.store, pubkey.as_hex()).await
    }

    // ─── Follow list ────────────────────────────────────────────────────────

    pub async fn follows(&self) -> Result<Vec<PubKey>> {
        Ok(repo::follows(&self.store).await?.iter().filter_map(|s| PubKey::parse(s)).collect())
    }

    /// Our own kind 3 arrived (from another device or a replay): replace.
    pub async fn apply_follow_list(&self, follows: &[PubKey]) -> Result<()> {
        let keys: Vec<String> = follows.iter().map(|p| p.as_hex().to_string()).collect();
        repo::replace_follows(&self.store, &keys).await
    }

    pub async fn set_followed(&self, pubkey: &PubKey, followed: bool) -> Result<()> {
        if followed {
            repo::follow(&self.store, pubkey.as_hex()).await
        } else {
            repo::unfollow(&self.store, pubkey.as_hex()).await
        }
    }

    /// Signed kind-3 with the current follow list.
    pub async fn build_follow_list(&self, keys: &Keys) -> Result<WireEvent> {
        let contacts = repo::follows(&self.store)
            .await?
            .iter()
            .filter_map(|s| PublicKey::from_hex(s).ok())
            .map(Contact::new)
            .collect::<Vec<_>>();
        let event = ContactListBuilder::new(contacts)
            .finalize(keys)
            .map_err(|e| MessengerError::Crypto(e.to_string()))?;
        Ok(WireEvent { id: EventId::parse(&event.id.to_hex()).expect("hex id"), json: serde_json::to_value(&event)? })
    }
}

/// hex or npub → PubKey.
pub fn parse_key(input: &str) -> Result<PubKey> {
    let pk = PublicKey::parse(input.trim())
        .map_err(|_| MessengerError::Invalid("expected an npub or a 64-hex public key".into()))?;
    Ok(PubKey::parse(&pk.to_hex()).expect("valid pubkey"))
}

fn npub_of(hex: &str) -> String {
    PublicKey::from_hex(hex).ok().and_then(|p| p.to_bech32().ok()).unwrap_or_else(|| hex.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::Timestamp;

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    #[tokio::test]
    async fn book_lifecycle_with_profiles_and_follows() {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let svc = ContactService::new(store, profiles.clone());
        let me = Keys::generate();
        let bob = Keys::generate();

        assert!(svc.add(&pk(&me), &me.public_key().to_hex(), None).await.is_err(), "no self-contact");
        assert!(svc.add(&pk(&me), "garbage", None).await.is_err());
        let c = svc.add(&pk(&me), &bob.public_key().to_bech32().unwrap(), Some("  Bobby ")).await.unwrap();
        assert_eq!(c.nickname.as_deref(), Some("Bobby"));
        assert_eq!(c.label(), "Bobby");
        assert!(!c.followed);
        assert!(svc.is_contact(&pk(&bob)).await.unwrap());

        profiles.apply_event(&pk(&bob), Timestamp(1), r#"{"name":"bob"}"#).await.unwrap();
        let c = svc.update(&pk(&bob), &ContactPatch { nickname: Some(None), ..Default::default() }).await.unwrap();
        assert_eq!(c.profile.as_ref().unwrap().name.as_deref(), Some("bob"));
        assert_eq!(c.label(), "bob", "falls back to profile name without nickname");

        svc.set_followed(&pk(&bob), true).await.unwrap();
        assert!(svc.get(&pk(&bob)).await.unwrap().unwrap().followed);
        let ev = svc.build_follow_list(&me).await.unwrap();
        let parsed: Event = serde_json::from_value(ev.json).unwrap();
        assert_eq!(parsed.kind.as_u16(), 3);
        assert_eq!(parsed.tags.iter().filter(|t| t.kind() == "p").count(), 1);

        svc.apply_follow_list(&[]).await.unwrap();
        assert!(svc.follows().await.unwrap().is_empty());

        svc.remove(&pk(&bob)).await.unwrap();
        assert!(!svc.is_contact(&pk(&bob)).await.unwrap());
        assert!(svc.list().await.unwrap().is_empty());
    }
}

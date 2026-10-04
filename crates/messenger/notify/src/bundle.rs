// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keys a push handler needs, as the app exports them.
//!
//! The app keeps its secrets behind the vault; a process a push started
//! cannot open it. So the app hands a copy of exactly what opening events
//! takes to the phone's own key store, and this is the shape of that copy.

use messenger_core::{MessengerError, Result};
use messenger_groups::{GroupKey, KeyId};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const BUNDLE_VERSION: u32 = 1;

/// One key of one group; `key_id` is what the `k` tag of an event names.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GroupKeyEntry {
    pub group_id: String,
    pub key_id: String,
    /// Hex of the 32 bytes.
    pub key: String,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct KeyBundle {
    pub v: u32,
    /// Hex of the secret key.
    pub nsec: String,
    #[serde(default)]
    pub groups: Vec<GroupKeyEntry>,
}

impl KeyBundle {
    pub fn new(keys: &Keys, groups: Vec<GroupKeyEntry>) -> Self {
        Self { v: BUNDLE_VERSION, nsec: keys.secret_key().to_secret_hex(), groups }
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let b: Self = serde_json::from_slice(bytes).map_err(|e| MessengerError::Invalid(format!("key bundle: {e}")))?;
        if b.v == 0 || b.v > BUNDLE_VERSION {
            return Err(MessengerError::Invalid(format!("key bundle v{} unsupported", b.v)));
        }
        Ok(b)
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a bundle is always serializable")
    }

    pub fn keys(&self) -> Result<Keys> {
        Keys::parse(&self.nsec).map_err(|_| MessengerError::Invalid("key bundle: secret key".into()))
    }

    pub fn group_key(&self, group_id: &str, key_id: &str) -> Option<GroupKey> {
        self.groups
            .iter()
            .find(|g| g.group_id == group_id && g.key_id == key_id)
            .and_then(|g| hex::decode(&g.key).ok())
            .and_then(|bytes| GroupKey::from_bytes(&bytes).ok())
            // The id of a key is derived from it: a bundle that says otherwise is not trusted.
            .filter(|k| k.id() == KeyId(key_id.to_string()))
    }
}

impl GroupKeyEntry {
    pub fn of(group_id: &str, key: &GroupKey) -> Self {
        Self { group_id: group_id.to_string(), key_id: key.id().0, key: hex::encode(key.as_bytes()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_key_lookup() {
        let keys = Keys::generate();
        let gk = GroupKey::generate().unwrap();
        let bundle = KeyBundle::new(&keys, vec![GroupKeyEntry::of("g1", &gk)]);
        let again = KeyBundle::from_json(&bundle.to_json()).unwrap();
        assert_eq!(again.keys().unwrap().public_key(), keys.public_key());
        assert!(again.group_key("g1", &gk.id().0).is_some());
        assert!(again.group_key("g2", &gk.id().0).is_none());
        assert!(again.group_key("g1", "0000").is_none());
    }

    #[test]
    fn a_key_under_a_wrong_id_is_not_used() {
        let keys = Keys::generate();
        let gk = GroupKey::generate().unwrap();
        let mut entry = GroupKeyEntry::of("g1", &gk);
        entry.key_id = "ffffffffffffffffffffffffffffffff".into();
        let bundle = KeyBundle::new(&keys, vec![entry]);
        assert!(bundle.group_key("g1", "ffffffffffffffffffffffffffffffff").is_none());
    }

    #[test]
    fn versions_from_the_future_are_refused() {
        let keys = Keys::generate();
        let mut bundle = KeyBundle::new(&keys, vec![]);
        bundle.v = BUNDLE_VERSION + 1;
        assert!(KeyBundle::from_json(&bundle.to_json()).is_err());
    }
}

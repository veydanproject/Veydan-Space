// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What the UI is given: when each contact was last seen, and until when it
//! is online. Whether it is online now is the UI's to say, by its clock.

use messenger_core::Result;
use messenger_store::{presence, Store};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PresenceView {
    /// The contact's own key (hex), not its presence key.
    pub peer: String,
    /// The time of its newest beat.
    pub seen_at: i64,
    /// Online while now is before this.
    pub online_until: i64,
}

/// Every contact that told me its key and has beaten since, by peer.
pub async fn snapshot(store: &Store) -> Result<Vec<PresenceView>> {
    Ok(presence::snapshot(store)
        .await?
        .into_iter()
        .map(|(peer, seen_at, online_until)| PresenceView { peer, seen_at, online_until })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_view_names_contacts_not_presence_keys() {
        let s = Store::open_in_memory().await.unwrap();
        assert!(snapshot(&s).await.unwrap().is_empty());
        presence::put_key(&s, "p", Some("aa"), 1).await.unwrap();
        presence::seen(&s, "aa", 100, 180).await.unwrap();
        let v = snapshot(&s).await.unwrap();
        assert_eq!(v, vec![PresenceView { peer: "p".into(), seen_at: 100, online_until: 180 }]);
        assert_eq!(
            serde_json::to_value(&v[0]).unwrap(),
            serde_json::json!({ "peer": "p", "seen_at": 100, "online_until": 180 })
        );
    }
}

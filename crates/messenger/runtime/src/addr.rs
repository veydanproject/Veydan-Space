// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The address of a peer in the runtime's API (internal/messenger-wire.md,
//! «Частный чат»): `<npub|hex>` is the home chat with that person,
//! `<npub|hex>@<server id>` the private chat on that server. A direct chat
//! id (`dm:<hex>`, `dm:<hex>@<server id>`) reads the same, so the page may
//! pass either. Read here once; every command that names a peer takes it,
//! and the host's commands keep their string.

use crate::MessengerRuntime;
use messenger_core::{MessengerError, Result};
use messenger_dm::DmPeer;
use messenger_store::servers;

/// The peer and the server an address names, by its shape alone.
pub fn parse(addr: &str) -> Result<DmPeer> {
    let addr = addr.trim();
    let addr = addr.strip_prefix("dm:").unwrap_or(addr);
    let (key, server) = match addr.split_once('@') {
        Some((key, server)) => (key, Some(server.trim())),
        None => (addr, None),
    };
    let key = messenger_contacts::book::parse_key(key).map_err(|_| {
        MessengerError::Invalid("recipient must be an npub or 64-hex public key".into())
    })?;
    match server {
        Some(id) if !servers::is_id(id) => Err(MessengerError::Invalid("server_unknown".into())),
        server => Ok(DmPeer::on(key, server)),
    }
}

impl MessengerRuntime {
    /// The peer of `addr`, its private server being one this device has
    /// (`server_unknown` otherwise): nothing is written to, or for, a
    /// server that is not mine.
    pub async fn peer_addr(&self, addr: &str) -> Result<DmPeer> {
        let peer = parse(addr)?;
        if let Some(id) = peer.server() {
            if !servers::get(self.store(), id)
                .await?
                .is_some_and(|s| s.is_private())
            {
                return Err(MessengerError::Invalid("server_unknown".into()));
            }
        }
        Ok(peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip19::ToBech32;

    #[test]
    fn an_address_names_the_home_chat_or_a_private_one() {
        let keys = nostr::key::Keys::generate();
        let hex = keys.public_key().to_hex();
        let npub = keys.public_key().to_bech32().unwrap();
        let home = parse(&hex).unwrap();
        assert_eq!((home.key.as_hex(), home.server()), (hex.as_str(), None));
        assert_eq!(parse(&npub).unwrap(), home);
        assert_eq!(
            parse(&format!("dm:{hex}")).unwrap(),
            home,
            "a chat id reads too"
        );
        let private = parse(&format!("{npub}@0123456789abcdef")).unwrap();
        assert_eq!(private.server(), Some("0123456789abcdef"));
        assert_eq!(
            parse(&format!("dm:{hex}@0123456789abcdef")).unwrap(),
            private
        );
        for bad in [
            format!("{hex}@"),
            format!("{hex}@S1"),
            format!("{hex}@0123"),
            "nobody".to_string(),
            "@0123456789abcdef".into(),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    /// The queue of requests holds private chats too: answering one by its
    /// address answers it there, and the home chat with the same person
    /// keeps its relation.
    #[tokio::test]
    async fn a_request_is_answered_in_the_chat_its_address_names() {
        use crate::moderation::RequestAction;
        let dir = tempfile::tempdir().unwrap();
        let cfg = messenger_core::MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, std::sync::Arc::new(messenger_testkit::MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        let s1 = rt
            .server_put_private(&crate::private::PrivateServerInput {
                relay_url: "wss://relay.s1.example".into(),
                key: None,
                media_url: None,
                call_node: None,
                name: None,
                push_mode: None,
            })
            .await
            .unwrap();
        let bob = nostr::key::Keys::generate().public_key().to_hex();
        let on_s1 = format!("{bob}@{s1}");
        rt.request_act(&on_s1, RequestAction::Block).await.unwrap();
        assert!(rt.dm_relation(&on_s1).await.unwrap().blocked, "blocked on S1");
        assert!(!rt.dm_relation(&bob).await.unwrap().blocked, "not at home");
        let e = rt.request_act(&format!("{bob}@{}", "f".repeat(16)), RequestAction::Block).await;
        assert!(matches!(e, Err(MessengerError::Invalid(ref c)) if c == "server_unknown"), "{e:?}");
        rt.shutdown().await;
    }
}

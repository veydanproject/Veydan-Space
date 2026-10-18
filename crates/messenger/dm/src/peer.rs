// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Who a direct chat is with, and where it lives (internal/messenger-wire.md,
//! «Частный чат»): the home set, or one private server. The same person may
//! be talked to in both, and the two chats share nothing — not the
//! messages, not the relationship, not the relays a word goes to.
//!
//! Everything that belongs to one chat takes a `DmPeer`; what belongs to a
//! person (a ban of the network, the address book, presence) takes the key.
//! A `&PubKey` converts into the home chat, so the code of the home chat
//! reads as it did.

use messenger_core::outbound::WireEvent;
use messenger_core::{Outbound, PubKey, RelayUrl, Scope};
use messenger_store::chats::{self, ChatRow};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DmPeer {
    pub key: PubKey,
    /// The private server of the chat (`msg_servers.id`); `None` for the
    /// home set.
    pub server: Option<String>,
}

impl DmPeer {
    /// The home chat with `key`.
    pub fn home(key: PubKey) -> Self {
        Self { key, server: None }
    }

    /// The chat with `key` on `server` (`None`: the home set).
    pub fn on(key: PubKey, server: Option<&str>) -> Self {
        Self {
            key,
            server: server.map(String::from),
        }
    }

    pub fn server(&self) -> Option<&str> {
        self.server.as_deref()
    }

    pub fn is_private(&self) -> bool {
        self.server.is_some()
    }

    /// `dm:<peer>` or `dm:<peer>@<server>`.
    pub fn chat_id(&self) -> String {
        chats::dm_chat_id(self.key.as_hex(), self.server())
    }

    /// The chat a direct chat id names; `None` for any other id.
    pub fn of_chat(chat_id: &str) -> Option<Self> {
        let (peer, server) = chats::parse_dm_chat_id(chat_id)?;
        Some(Self::on(PubKey::parse(peer)?, server))
    }

    /// The chat of a stored row: its peer, on its server.
    pub fn of_row(row: &ChatRow) -> Option<Self> {
        let key = row.peer_pubkey.as_deref().and_then(PubKey::parse)?;
        Some(Self::on(key, row.server_id.as_deref()))
    }

    /// The same person in the chat on `server`.
    pub fn with_key(&self, key: PubKey) -> Self {
        Self {
            key,
            server: self.server.clone(),
        }
    }

    /// How the peer's copy of something of this chat leaves: to the
    /// peer's inbox relays (`hints`) for the home chat, to every relay of
    /// the server for a private one — never anywhere else.
    pub fn to_peer(&self, event: WireEvent, hints: Vec<RelayUrl>) -> Outbound {
        match &self.server {
            None => Outbound::PublishToInbox {
                recipient: self.key.clone(),
                event,
                hint_relays: hints,
            },
            Some(id) => Outbound::PublishScoped {
                scope: Scope::Server { id: id.clone() },
                event,
            },
        }
    }

    /// How my own copy leaves: to my relays of the home set, or to the
    /// server of a private chat, where my other devices on it hear it.
    pub fn to_self(&self, event: WireEvent) -> Outbound {
        to_my_devices(self.server(), event)
    }
}

/// A copy for my other devices about something of `server` (`None`: the
/// home set): my home relays, or that server alone.
pub fn to_my_devices(server: Option<&str>, event: WireEvent) -> Outbound {
    match server {
        None => Outbound::PublishOwn { event },
        Some(id) => Outbound::PublishScoped {
            scope: Scope::Server { id: id.to_string() },
            event,
        },
    }
}

impl From<PubKey> for DmPeer {
    fn from(key: PubKey) -> Self {
        Self::home(key)
    }
}

impl From<&PubKey> for DmPeer {
    fn from(key: &PubKey) -> Self {
        Self::home(key.clone())
    }
}

impl From<&DmPeer> for DmPeer {
    fn from(peer: &DmPeer) -> Self {
        peer.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_id_names_the_peer_and_the_server() {
        let key = PubKey::parse(&"ab".repeat(32)).unwrap();
        let home = DmPeer::home(key.clone());
        let private = DmPeer::on(key.clone(), Some("0123456789abcdef"));
        assert_eq!(home.chat_id(), format!("dm:{}", "ab".repeat(32)));
        assert_eq!(
            private.chat_id(),
            format!("dm:{}@0123456789abcdef", "ab".repeat(32))
        );
        assert_eq!(DmPeer::of_chat(&home.chat_id()), Some(home.clone()));
        assert_eq!(DmPeer::of_chat(&private.chat_id()), Some(private.clone()));
        assert_ne!(home, private, "two chats with one person");
        for bad in [
            "dm:",
            "dm:@s",
            &format!("dm:{}@", "ab".repeat(32)),
            "dm:xyz",
            &format!("group:{}", "ab".repeat(32)),
        ] {
            assert_eq!(DmPeer::of_chat(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_private_chat_goes_to_its_server_alone() {
        let key = PubKey::parse(&"ab".repeat(32)).unwrap();
        let event = WireEvent {
            id: messenger_core::EventId::parse(&"e".repeat(64)).unwrap(),
            json: serde_json::json!({}),
        };
        let private = DmPeer::on(key.clone(), Some("s1"));
        let server = Scope::Server { id: "s1".into() };
        assert!(
            matches!(private.to_peer(event.clone(), vec![]), Outbound::PublishScoped { scope, .. } if scope == server)
        );
        assert!(
            matches!(private.to_self(event.clone()), Outbound::PublishScoped { scope, .. } if scope == server)
        );
        let home = DmPeer::home(key);
        assert!(matches!(
            home.to_peer(event.clone(), vec![]),
            Outbound::PublishToInbox { .. }
        ));
        assert!(matches!(home.to_self(event), Outbound::PublishOwn { .. }));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a link in a message leads to, as the UI shows it on a card.
//!
//! The UI finds in a text what looks like a link and asks here; it never
//! takes a link apart itself. The answer is made of what this device
//! already knows: nothing is fetched to draw a card.

use crate::MessengerRuntime;
use messenger_core::{MessengerError, PubKey, Result};
use messenger_groups::{GroupKind, GroupLink};
use messenger_links::contact::{npub_of, pubkey_of_npub};
use messenger_links::{BridgeLink, CallNodeLink, ContactLink, LinkError, LinkType, Uri};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How many links one call may ask about.
const MAX_LINKS: usize = 64;

/// What I am to a group, as far as this device knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GroupMembership {
    Joined,
    Joining,
    Requested,
    Rejected,
    StaleLink,
    Left,
    Removed,
    Banned,
    Disbanded,
}

impl GroupMembership {
    /// From what the group store writes; `None` for a word it does not write.
    pub fn parse(s: &str) -> Option<Self> {
        use messenger_groups::service::*;
        Some(match s {
            MEMBERSHIP_JOINED => Self::Joined,
            MEMBERSHIP_JOINING => Self::Joining,
            MEMBERSHIP_REQUESTED => Self::Requested,
            MEMBERSHIP_REJECTED => Self::Rejected,
            MEMBERSHIP_STALE => Self::StaleLink,
            MEMBERSHIP_LEFT => Self::Left,
            MEMBERSHIP_REMOVED => Self::Removed,
            MEMBERSHIP_BANNED => Self::Banned,
            MEMBERSHIP_DISBANDED => Self::Disbanded,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LinkGroupKind {
    Public,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkView {
    Group {
        /// The link as this device writes it; cards act on this one.
        link: String,
        group_id: String,
        group_kind: LinkGroupKind,
        name: String,
        relay: String,
        owner: String,
        /// Known only for a group this device has met.
        picture: Option<String>,
        members: Option<u32>,
        /// Empty when this device has never met the group.
        membership: Option<GroupMembership>,
    },
    Contact {
        link: String,
        pubkey: String,
        npub: String,
        /// My name for them, then their own, then what the link says; may be empty.
        name: String,
        picture: Option<String>,
        nip05: Option<String>,
        is_me: bool,
        is_contact: bool,
        blocked: bool,
    },
    /// A VLink bridge to the project's servers that somebody runs
    /// (`veydan://vlink/…`). The kind is `vlink`: `bridge` is kept for
    /// bridges to other networks.
    #[serde(rename = "vlink")]
    Bridge {
        link: String,
        id: String,
        /// `address:port`.
        addr: String,
        /// Already among the bridges added on this device.
        added: bool,
    },
    /// A private call node and, mostly, an invitation to it
    /// (`veydan://call-node/<id>?a=…&t=…`): what `messenger_call_nodes_add`
    /// takes. The card asks before the invitation is spent.
    CallNode {
        /// The link as this device writes it, the token with it.
        link: String,
        id: String,
        /// `address:port` of its control channel.
        addr: String,
        /// The link carries an invitation (a link without one names the
        /// node alone: its key is given by hand).
        has_token: bool,
        /// Already among the nodes of mine on this device: for a link with
        /// an invitation, only when this device holds its own credentials
        /// on the node (an entry of my own list leaves the invitation to
        /// be exchanged).
        added: bool,
    },
    /// A well-formed link of a type a newer client knows.
    Unknown { link_type: String },
    Invalid { code: String },
}

impl LinkView {
    fn invalid(e: LinkError) -> Self {
        Self::Invalid { code: e.code().into() }
    }
}

/// `npub1…` and `nostr:npub1…` name a person as a link does.
fn bare_contact(text: &str) -> Option<ContactLink> {
    let npub = text.strip_prefix("nostr:").unwrap_or(text);
    pubkey_of_npub(npub).map(|pubkey| ContactLink { pubkey, name: String::new(), relays: Vec::new() })
}

impl MessengerRuntime {
    /// One answer per link, in the order asked.
    pub async fn links_inspect(&self, links: &[String]) -> Result<Vec<LinkView>> {
        if links.len() > MAX_LINKS {
            return Err(MessengerError::Invalid("link_too_many".into()));
        }
        let mut out = Vec::with_capacity(links.len());
        for link in links {
            out.push(self.link_inspect(link).await?);
        }
        Ok(out)
    }

    pub async fn link_inspect(&self, link: &str) -> Result<LinkView> {
        let text = link.trim();
        if let Some(contact) = bare_contact(text) {
            return self.contact_card(contact).await;
        }
        let uri = match Uri::parse(text) {
            Ok(u) => u,
            Err(e) => return Ok(LinkView::invalid(e)),
        };
        match uri.link_type() {
            LinkType::Group => match GroupLink::from_uri(&uri) {
                Ok(g) => self.group_card(g).await,
                Err(_) => Ok(LinkView::invalid(LinkError::Param)),
            },
            LinkType::Contact => match ContactLink::from_uri(&uri) {
                Ok(c) => self.contact_card(c).await,
                Err(e) => Ok(LinkView::invalid(e)),
            },
            LinkType::Bridge => match BridgeLink::from_uri(&uri) {
                Ok(b) => {
                    let added = self.net_status().await?.private.iter().any(|p| p.id == b.id);
                    Ok(LinkView::Bridge { link: b.encode(), id: b.id, addr: b.addr.to_string(), added })
                }
                Err(e) => Ok(LinkView::invalid(e)),
            },
            LinkType::CallNode => match CallNodeLink::from_uri(&uri) {
                Ok(n) => {
                    let added = self.call_node_is_mine(&n.id, n.token.is_some()).await?;
                    Ok(LinkView::CallNode { link: n.encode(), id: n.id.clone(), addr: n.addr.to_string(), has_token: n.token.is_some(), added })
                }
                Err(e) => Ok(LinkView::invalid(e)),
            },
            LinkType::Unknown(word) => Ok(LinkView::Unknown { link_type: word.clone() }),
        }
    }

    async fn group_card(&self, link: GroupLink) -> Result<LinkView> {
        let known = match self.session_pubkey().await {
            Some(me) => self.groups().get(&link.group_id, &me).await?,
            None => None,
        };
        let group_kind = if link.kind == GroupKind::Public { LinkGroupKind::Public } else { LinkGroupKind::Private };
        Ok(LinkView::Group {
            link: link.encode(),
            group_id: link.group_id.clone(),
            group_kind,
            // What the group is called now says more than what the link called it.
            name: known.as_ref().map(|g| g.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or(link.name),
            relay: link.relay.as_str().into(),
            owner: link.owner.as_hex().into(),
            picture: known.as_ref().map(|g| g.picture.clone()).filter(|p| !p.is_empty()),
            members: known.as_ref().filter(|g| !g.members.is_empty()).map(|g| g.members.len() as u32),
            membership: known.and_then(|g| GroupMembership::parse(&g.membership)),
        })
    }

    async fn contact_card(&self, link: ContactLink) -> Result<LinkView> {
        let pk = &link.pubkey;
        let is_me = self.session_pubkey().await.as_ref() == Some(pk);
        let contact = self.contacts().get(pk).await?;
        let profile = match &contact {
            Some(c) => c.profile.clone(),
            None => self.profiles().get(pk).await?,
        };
        if profile.is_none() && !is_me {
            // The answer comes as `profile.updated`; no relay is no reason to fail.
            let _ = self.request_profile(pk).await;
        }
        let nickname = contact.as_ref().and_then(|c| c.nickname.clone()).filter(|n| !n.trim().is_empty());
        let own = profile.as_ref().and_then(|p| {
            [&p.display_name, &p.name, &p.nip05].into_iter().flatten().find(|s| !s.trim().is_empty()).cloned()
        });
        Ok(LinkView::Contact {
            link: link.encode()?,
            pubkey: pk.as_hex().into(),
            npub: npub_of(pk).unwrap_or_default(),
            name: nickname.or(own).unwrap_or_else(|| link.name.clone()),
            picture: profile.as_ref().and_then(|p| p.picture.clone()).filter(|p| !p.is_empty()),
            nip05: profile.as_ref().and_then(|p| p.nip05.clone()).filter(|p| !p.is_empty()),
            is_me,
            is_contact: contact.is_some(),
            blocked: self.dm_blocked().await?.iter().any(|b| b == pk.as_hex()),
        })
    }

    /// The link of a person, to share: the name they are known by here and
    /// the relays this device writes to.
    pub async fn contact_link(&self, pubkey: &PubKey) -> Result<String> {
        let name = match self.link_inspect(&npub_of(pubkey).unwrap_or_default()).await? {
            LinkView::Contact { name, .. } => name,
            _ => String::new(),
        };
        let relays = self
            .relays()
            .list()
            .await?
            .into_iter()
            .filter(|r| r.enabled && r.write)
            .filter_map(|r| messenger_core::RelayUrl::parse(&r.url))
            .collect();
        Ok(ContactLink { pubkey: pubkey.clone(), name, relays }.encode()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use nostr::key::Keys;
    use std::sync::Arc;

    async fn runtime(dir: &tempfile::TempDir) -> MessengerRuntime {
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        rt.identity().create("pw").await.unwrap();
        rt.refresh_signer().await.unwrap();
        rt
    }

    #[tokio::test]
    async fn every_text_gets_its_own_kind() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let me = rt.session_pubkey().await.unwrap();
        let bob = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        rt.contact_add(bob.as_hex(), Some("Bobby")).await.unwrap();
        let stranger = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();

        let gid = "0".repeat(62) + "aa";
        let group = format!("veydan://group/{gid}?t=private&r=wss%3A%2F%2Fr.example&o={}&n=Club", "a".repeat(64));
        let asked = vec![
            group.clone(),
            format!("  {group}&future=1 "),
            rt.contact_link(&bob).await.unwrap(),
            format!("nostr:{}", npub_of(&stranger).unwrap()),
            npub_of(&me).unwrap(),
            "veydan://channel/42".to_string(),
            "https://example.com".to_string(),
            group.replace("t=private", "t=public"),
            format!("veydan://contact/{}", bob.as_hex()),
        ];
        let got = rt.links_inspect(&asked).await.unwrap();
        assert_eq!(got.len(), asked.len());

        match &got[0] {
            LinkView::Group { link, group_id, group_kind, name, membership, picture, members, .. } => {
                assert_eq!(group_id, &gid);
                assert_eq!(*group_kind, LinkGroupKind::Private);
                assert_eq!(name, "Club");
                assert_eq!(link, &group, "written the way this device writes it");
                assert!(membership.is_none() && picture.is_none() && members.is_none(), "never met");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(got[1], got[0], "one card for one group, however the link was written");
        match &got[2] {
            LinkView::Contact { pubkey, name, is_contact, is_me, blocked, link, .. } => {
                assert_eq!(pubkey, bob.as_hex());
                assert_eq!(name, "Bobby");
                assert!(*is_contact && !*is_me && !*blocked);
                assert!(link.starts_with("veydan://contact/npub1") && link.contains("n=Bobby"));
            }
            other => panic!("{other:?}"),
        }
        match &got[3] {
            LinkView::Contact { pubkey, name, is_contact, .. } => {
                assert_eq!(pubkey, stranger.as_hex());
                assert!(name.is_empty() && !*is_contact);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&got[4], LinkView::Contact { is_me: true, .. }));
        assert_eq!(got[5], LinkView::Unknown { link_type: "channel".into() });
        assert_eq!(got[6], LinkView::Invalid { code: "link_bad_scheme".into() });
        assert_eq!(got[7], LinkView::Invalid { code: "link_bad_param".into() }, "public without a secret");
        assert_eq!(got[8], LinkView::Invalid { code: "link_bad_id".into() });

        let json = serde_json::to_value(&got[5]).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "unknown", "link_type": "channel" }));

        assert!(rt.links_inspect(&vec![group; MAX_LINKS + 1]).await.is_err());
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_group_this_device_is_in_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let g = rt.group_create(GroupKind::Public, "Open house", "", true).await.unwrap();
        let link = g.link.clone().expect("the owner may share the link");
        match rt.link_inspect(&link).await.unwrap() {
            LinkView::Group { group_id, name, membership, members, link: canonical, .. } => {
                assert_eq!(group_id, g.id);
                assert_eq!(name, "Open house");
                assert_eq!(membership, Some(GroupMembership::Joined));
                assert_eq!(members, Some(1));
                assert_eq!(canonical, link);
            }
            other => panic!("{other:?}"),
        }
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_vlink_link_is_a_bridge_and_the_word_bridge_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let id = "9c5f434def40ebc2879201642bab6c69063578c4a302aa92de5918b3663ffbfc";
        let link = format!("veydan://vlink/{}?a=203.0.113.7%3A443", id.to_uppercase());
        let got = rt.link_inspect(&link).await.unwrap();
        assert_eq!(
            got,
            LinkView::Bridge { link: format!("veydan://vlink/{id}?a=203.0.113.7%3A443"), id: id.into(), addr: "203.0.113.7:443".into(), added: false }
        );
        assert_eq!(serde_json::to_value(&got).unwrap()["kind"], "vlink");
        rt.net_bridge_add(&link).await.unwrap();
        assert!(matches!(rt.link_inspect(&link).await.unwrap(), LinkView::Bridge { added: true, .. }));

        // Kept for bridges to other networks; never released, so no alias.
        let old = format!("veydan://bridge/{id}?a=203.0.113.7%3A443");
        assert_eq!(rt.link_inspect(&old).await.unwrap(), LinkView::Unknown { link_type: "bridge".into() });
        match rt.net_bridge_add(&old).await {
            Err(MessengerError::Invalid(code)) => assert_eq!(code, "net_bridge_link_type"),
            other => panic!("{other:?}"),
        }
        rt.shutdown().await;
    }
}

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
use messenger_links::{BridgeLink, CallNodeLink, ContactLink, LinkError, LinkType, RecoverLink, ServerLink, Uri, WebLink};
use messenger_store::servers;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How many people one link of a private chat lets in (its invitation).
const PRIVATE_LINK_USES: u32 = 5;
/// How long that invitation lets people in.
const PRIVATE_LINK_TTL_SECS: i64 = 30 * 86_400;
/// An invitation with less than this left is not given out again: a new
/// one is made, so a link handed over now still works for a week.
const PRIVATE_LINK_RENEW_SECS: i64 = 7 * 86_400;
/// The setting naming the invitation of the link of my private chat on a
/// server, `<this>@<server id>` (removed with the server).
const KEY_PRIVATE_LINK_INVITE: &str = "invite.private";

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

/// A server as its card shows it, before it is added
/// (internal/messenger-wire.md, «Принятие сервера»): what it has, how
/// phones are woken, whether it is one of the Veydan network's, whether
/// this device has it already. Its app key is not shown; it is in the
/// card, which the dialog hands back to `messenger_server_accept`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServerPreview {
    /// The card as this device writes it (`veydan://server/…`).
    pub link: String,
    /// 16 hex: the id a peer address names (`<npub>@<id>`).
    pub id: String,
    /// What the card calls it; may be empty.
    pub name: String,
    /// `wss://…`
    pub relay: String,
    /// The blob server, `https://…`.
    pub media: Option<String>,
    /// The call node, `address:port#id`.
    pub call_node: Option<String>,
    /// The push server of the server, `https://…`: where the device
    /// registers for pushes through the gateway when the card is accepted.
    pub push_url: Option<String>,
    /// `veydan`: phones are woken through the push of the Veydan network;
    /// `off`: no push.
    pub push: String,
    /// One of my servers already (of the home set or private): accepting
    /// adds nothing.
    pub known: bool,
    /// Its relay is one of the Veydan network's (the manifest in use names
    /// it): «серверы Veydan» rather than a server of somebody's own.
    pub is_veydan: bool,
}

/// A private server of mine as the settings list it; never its app key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PrivateServerView {
    pub id: String,
    pub name: Option<String>,
    pub relay: String,
    pub media: Option<String>,
    pub call_node: Option<String>,
    /// `veydan` | `off`
    pub push: String,
    /// When it was added, unix seconds.
    #[ts(type = "number")]
    pub added_at: i64,
    /// Its private chats on this device: what its removal takes with it.
    #[serde(default)]
    #[ts(type = "number")]
    pub chats: i64,
    /// The groups that live on it (their relay is its relay).
    #[serde(default)]
    #[ts(type = "number")]
    pub groups: i64,
}

impl From<&servers::ServerRow> for PrivateServerView {
    fn from(s: &servers::ServerRow) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            relay: s.relay_url.clone(),
            media: s.media_url.clone(),
            call_node: s.call_node.clone(),
            push: s.push_mode.clone(),
            added_at: s.added_at,
            chats: 0,
            groups: 0,
        }
    }
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
        /// The card of the group's server the link carries (`srv=`): the
        /// server is added first, after asking (`messenger_server_accept`).
        server: Option<ServerPreview>,
        /// The group's relay is a server this device has (of the home set
        /// or private): the link may be opened. Without it and without a
        /// card, opening it is `server_unknown`.
        server_known: bool,
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
        /// The secret of their invitation, when the link carries one: a
        /// request by it (`messenger_dm_action` with `invite`) is let in
        /// without a request.
        invite: Option<String>,
        /// The card of the server the person is talked to on (`srv=`):
        /// the chat is the private one there (`messenger_server_accept`,
        /// then `messenger_contact_open_private`), not the home one.
        server: Option<ServerPreview>,
    },
    /// The card of a server (`veydan://server/…`): what it has and whether
    /// it is Veydan's, before it is added (`messenger_server_accept`).
    Server(ServerPreview),
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
    /// A request of recovery through the trusted people
    /// (`veydan://recover/<npub>?c=<code>`, cloud-recovery.md §8.5): the
    /// card leads to «Помочь другу восстановить доступ» of the module cloud.
    Recover {
        link: String,
        /// The account to recover, 64 hex.
        npub: String,
        /// The check code as shown, `AB7K-2QZD-9MXP`.
        code: String,
        /// My name for the person, when they are a contact; empty otherwise.
        name: String,
    },
    /// A sign-in to the web cabinet of Veydan Cloud (`veydan://web/<id>`,
    /// cloud-api.md §16.2). Approved only from the app's own scanner or a
    /// paste, where the module cloud shows the screen of approval; a card
    /// of it in a chat approves nothing (a link sent by someone else is
    /// how a sign-in is stolen).
    Web {
        link: String,
        /// The pending sign-in, 16 hex.
        id: String,
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
    pubkey_of_npub(npub).map(|pubkey| ContactLink { pubkey, name: String::new(), relays: Vec::new(), invite: None, srv: None })
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
            LinkType::Recover => match RecoverLink::from_uri(&uri) {
                Ok(r) => {
                    let name = self.contacts().get(&r.npub).await?.map(|c| c.label()).unwrap_or_default();
                    Ok(LinkView::Recover { link: r.encode(), npub: r.npub.as_hex().into(), code: r.code, name })
                }
                Err(e) => Ok(LinkView::invalid(e)),
            },
            LinkType::Server => match ServerLink::from_uri(&uri) {
                Ok(s) => Ok(LinkView::Server(self.server_preview_of(&s).await?)),
                Err(e) => Ok(LinkView::invalid(e)),
            },
            LinkType::Web => match WebLink::from_uri(&uri) {
                Ok(w) => Ok(LinkView::Web { link: w.encode(), id: w.id }),
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
            server: match &link.srv {
                Some(s) => Some(self.server_preview_of(s).await?),
                None => None,
            },
            server_known: servers::by_relay_url(self.store(), link.relay.as_str()).await?.is_some(),
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
            invite: link.invite.clone(),
            server: match &link.srv {
                Some(s) => Some(self.server_preview_of(s).await?),
                None => None,
            },
        })
    }

    /// What the card of a server says, with what this device knows of it:
    /// whether it is one of mine already (of the home set or private) and
    /// whether its relay is one of the Veydan network's.
    pub async fn server_preview_of(&self, card: &ServerLink) -> Result<ServerPreview> {
        let relay = servers::normalize(card.relay.as_str());
        let known = servers::by_relay_url(self.store(), &relay).await?.is_some();
        let (manifest, _) = self.relays().current_manifest().await?;
        let is_veydan = manifest.relays.iter().any(|r| servers::normalize(&r.url) == relay);
        Ok(ServerPreview {
            link: card.encode(),
            id: card.id.clone(),
            name: card.name.clone(),
            relay,
            media: card.media.clone(),
            call_node: card.call_node.clone(),
            push_url: card.push_url.clone(),
            push: card.push.clone(),
            known,
            is_veydan,
        })
    }

    /// The link of a person, to share: the name they are known by here and
    /// the relays this device writes to.
    pub async fn contact_link(&self, pubkey: &PubKey) -> Result<String> {
        self.contact_link_with(pubkey, None).await
    }

    /// The same with the secret of an invitation on it (mine, for my own
    /// link: `moderation::invite_create`).
    pub async fn contact_link_with(&self, pubkey: &PubKey, invite: Option<&str>) -> Result<String> {
        self.contact_link_on(pubkey, invite, None).await
    }

    /// The same, of the private chat on private server `server` of mine
    /// (`server_unknown` for one this device does not have): the link
    /// carries the card of that server (`srv=`) and names its relays, never
    /// the home ones, so whoever opens it adds the server and writes to the
    /// person there.
    pub async fn contact_link_on(&self, pubkey: &PubKey, invite: Option<&str>, server: Option<&str>) -> Result<String> {
        let name = match self.link_inspect(&npub_of(pubkey).unwrap_or_default()).await? {
            LinkView::Contact { name, .. } => name,
            _ => String::new(),
        };
        let (relays, srv): (Vec<messenger_core::RelayUrl>, _) = match server {
            Some(id) => {
                let card = self.server_card(id).await?;
                (vec![card.relay.clone()], Some(card))
            }
            // The home set alone: a private server's relay is never named
            // by the home link.
            None => (
                self.relays()
                    .enabled_of(None)
                    .await?
                    .into_iter()
                    .filter(|r| r.write)
                    .filter_map(|r| messenger_core::RelayUrl::parse(&r.url))
                    .collect(),
                None,
            ),
        };
        let mut invite = invite.map(String::from);
        // My own link of a private chat carries an invitation of mine, as
        // the home link of an invitation does: whoever opens it is let
        // straight in on that server, not met as a stranger's request. (The
        // card of the kit, `veydan-server card --contact`, has none: the
        // server cannot make one for a person's chat, so that first message
        // is a request.)
        if invite.is_none() && self.session_pubkey().await.as_ref() == Some(pubkey) {
            if let Some(card) = &srv {
                invite = Some(self.private_link_invite(card).await?);
            }
        }
        Ok(ContactLink { pubkey: pubkey.clone(), name, relays, invite, srv }.encode()?)
    }

    /// My link of the private chat on private server `server` of mine,
    /// with its invitation (`contact_link_on`), as a QR code (SVG): «Мой
    /// QR» for that server. Whoever reads it is asked to add the server
    /// and is let straight into the private chat with me there.
    pub async fn private_link_qr(&self, server: &str) -> Result<String> {
        let me = self.session_pubkey().await.ok_or(MessengerError::NotLoggedIn)?;
        let link = self.contact_link_on(&me, None, Some(server)).await?;
        messenger_groups::qr::link_svg(&link)
    }

    /// The secret of the invitation the link of my private chat on server
    /// `card` carries: one live invitation per server, made again only
    /// when the one before was taken back, used up, or has fewer than
    /// `PRIVATE_LINK_RENEW_SECS` left — asking for the link again does not
    /// pile up invitations — and every one runs out
    /// (`PRIVATE_LINK_TTL_SECS`), so a link that leaked stops letting
    /// people in. Which one is the server's is kept in the settings
    /// (`invite.private@<id>`), which go with the server.
    async fn private_link_invite(&self, card: &ServerLink) -> Result<String> {
        let key = format!("{KEY_PRIVATE_LINK_INVITE}@{}", card.id);
        let now = messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs();
        if let Some(id) = messenger_store::settings::get(self.store(), &key).await? {
            if let Some(held) = self.dm().invite_get(&id).await? {
                let lasts = held.row.expires_at.is_some_and(|e| e - now >= PRIVATE_LINK_RENEW_SECS);
                if held.active(now) && lasts {
                    return Ok(held.row.secret);
                }
            }
        }
        let made = self
            .invite_make(crate::moderation::InvitePreset {
                kind: "link".into(),
                max_uses: Some(PRIVATE_LINK_USES),
                ttl_secs: Some(PRIVATE_LINK_TTL_SECS),
                for_pubkey: None,
                label: card.name.clone(),
            })
            .await?;
        messenger_store::settings::set(self.store(), &key, &made.row.id).await?;
        Ok(made.row.secret)
    }

    /// The card of private server `id` of mine, as a link carries it;
    /// `server_unknown` for an id that is no private server of this device.
    pub async fn server_card(&self, id: &str) -> Result<ServerLink> {
        let s = servers::get(self.store(), id)
            .await?
            .filter(|s| s.is_private())
            .ok_or_else(|| MessengerError::Invalid("server_unknown".into()))?;
        let push_url = servers::push_of(self.store(), &s.id).await?.and_then(|p| p.push_url);
        ServerLink::of_parts(&s.relay_url, s.relay_key.as_deref(), s.media_url.as_deref(), s.call_node.as_deref(), push_url.as_deref(), &s.push_mode, s.name.as_deref())
            .ok_or_else(|| MessengerError::Invalid("server_card_invalid".into()))
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
    async fn a_sign_in_to_the_web_cabinet_is_its_id_alone() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let id = "0123456789abcdef";
        let got = rt
            .links_inspect(&[
                format!(" veydan://web/{id} "),
                format!("veydan://web/{id}?n=Pay%20at%20evil.example"),
                "veydan://web/0123456789ABCDEF".into(),
            ])
            .await
            .unwrap();
        let want = LinkView::Web { link: format!("veydan://web/{id}"), id: id.into() };
        assert_eq!(got[0], want);
        assert_eq!(got[1], want, "a parameter is never shown");
        assert_eq!(got[2], LinkView::Invalid { code: "link_bad_id".into() });
        let json = serde_json::to_value(&got[0]).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "web", "link": format!("veydan://web/{id}"), "id": id }));
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_group_this_device_is_in_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let g = rt.group_create(GroupKind::Public, "Open house", "", true, None).await.unwrap();
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

    /// Stage Э4: the card of a server, alone or on the link of a person or
    /// a group, is shown with whether it is mine already and whether it is
    /// one of the Veydan network's; accepting it adds a private server, and
    /// leaves a server of the home set as it is.
    #[tokio::test]
    async fn a_server_card_says_what_it_is_and_whose() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let (manifest, _) = rt.relays().current_manifest().await.unwrap();
        // A relay of the manifest this device uses: of its home set.
        let in_use: Vec<String> = rt.relays().list().await.unwrap().into_iter().map(|r| r.url).collect();
        let veydan = manifest.relays.iter().map(|r| r.url.clone()).find(|u| in_use.contains(u)).expect("the built-in manifest is in use");
        let s1 = ServerLink {
            key: Some("app-key".into()),
            media: Some("https://media.s1.example".into()),
            push: "veydan".into(),
            name: "S1".into(),
            ..ServerLink::new(messenger_core::RelayUrl::parse("wss://relay.s1.example").unwrap())
        };

        let LinkView::Server(preview) = rt.link_inspect(&s1.encode()).await.unwrap() else { panic!("a server card") };
        assert_eq!(preview.id, servers::id_of("wss://relay.s1.example"));
        assert_eq!((preview.name.as_str(), preview.relay.as_str(), preview.push.as_str()), ("S1", "wss://relay.s1.example", "veydan"));
        assert_eq!(preview.media.as_deref(), Some("https://media.s1.example"));
        assert!(!preview.known && !preview.is_veydan);
        let json = serde_json::to_value(LinkView::Server(preview.clone())).unwrap();
        assert_eq!(json["kind"], "server");
        assert!(json.get("key").is_none(), "the app key is not shown");

        // The same card on the link of a person and of a group.
        let bob = PubKey::parse(&Keys::generate().public_key().to_hex()).unwrap();
        let person = ContactLink { pubkey: bob.clone(), name: "Bob".into(), relays: vec![], invite: None, srv: Some(s1.clone()) };
        match rt.link_inspect(&person.encode().unwrap()).await.unwrap() {
            LinkView::Contact { server: Some(p), link, .. } => {
                assert_eq!(p, preview);
                assert!(link.contains("srv="), "written back with the card");
                assert_eq!(rt.server_preview(&link).await.unwrap(), preview);
            }
            other => panic!("{other:?}"),
        }
        let group = GroupLink {
            group_id: "ab".repeat(32),
            kind: GroupKind::Private,
            relay: s1.relay.clone(),
            name: "On S1".into(),
            owner: bob.clone(),
            managers: vec![],
            secret: None,
            link_epoch: 0,
            srv: Some(s1.clone()),
        };
        match rt.link_inspect(&group.encode()).await.unwrap() {
            LinkView::Group { server: Some(p), server_known, .. } => assert!(p == preview && !server_known),
            other => panic!("{other:?}"),
        }
        let plain = rt.contact_link(&bob).await.unwrap();
        assert!(matches!(rt.server_preview(&plain).await, Err(MessengerError::Invalid(c)) if c == "server_link_none"));

        // Accepted: a private server of mine, with the fields of its card.
        let id = rt.server_accept(&s1.encode()).await.unwrap();
        assert_eq!(id, preview.id);
        let mine = rt.servers_private_view().await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!((mine[0].name.as_deref(), mine[0].push.as_str(), mine[0].media.as_deref()), (Some("S1"), "veydan", Some("https://media.s1.example")));
        assert!(rt.server_preview(&s1.encode()).await.unwrap().known);
        match rt.link_inspect(&group.encode()).await.unwrap() {
            LinkView::Group { server_known, .. } => assert!(server_known, "the group may be opened now"),
            other => panic!("{other:?}"),
        }
        // My link of the private chat there names that server alone.
        let me = rt.session_pubkey().await.unwrap();
        let on_s1 = ContactLink::parse(&rt.contact_link_on(&me, None, Some(&id)).await.unwrap()).unwrap();
        assert_eq!(on_s1.srv.as_ref().map(|s| s.id.as_str()), Some(id.as_str()));
        assert_eq!(on_s1.srv.as_ref().and_then(|s| s.key.as_deref()), Some("app-key"));
        assert_eq!(on_s1.relays.iter().map(|r| r.as_str()).collect::<Vec<_>>(), vec!["wss://relay.s1.example"]);
        let at_home = ContactLink::parse(&rt.contact_link(&me).await.unwrap()).unwrap();
        assert!(at_home.srv.is_none() && !at_home.relays.iter().any(|r| r.as_str().contains("s1.example")), "the home link names no private server");
        assert!(matches!(rt.contact_link_on(&me, None, Some(&"f".repeat(16))).await, Err(MessengerError::Invalid(c)) if c == "server_unknown"));
        // «Мой QR» of that server draws the same link; a server I have not, none.
        assert!(rt.private_link_qr(&id).await.unwrap().contains("<svg"), "a QR of the private link");
        assert!(rt.private_link_qr(&"f".repeat(16)).await.is_err());

        // A relay of the Veydan network: Veydan's, and of my home set here,
        // so accepting it adds nothing and changes nothing.
        let card = ServerLink::new(messenger_core::RelayUrl::parse(&veydan).unwrap());
        let v = rt.server_preview(&card.encode()).await.unwrap();
        assert!(v.is_veydan && v.known, "{v:?}");
        let home_id = rt.server_accept(&card.encode()).await.unwrap();
        assert_eq!(home_id, v.id);
        assert!(!servers::get(rt.store(), &home_id).await.unwrap().unwrap().is_private(), "a home relay never becomes private");
        assert_eq!(rt.servers_private_view().await.unwrap().len(), 1);
        let chat = rt.contact_open_private(bob.as_hex(), &home_id, None).await.unwrap();
        assert_eq!(chat.id, format!("dm:{}", bob.as_hex()), "a card of a home server leads to the home chat");
        rt.shutdown().await;
    }

    /// A card with `pu=` leaves the server with its push server set and the
    /// mode `veydan`; one without it sets nothing.
    #[tokio::test]
    async fn a_card_with_a_push_server_turns_its_push_on() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let card = |relay: &str, pu: Option<&str>, push: &str| ServerLink {
            push_url: pu.map(String::from),
            push: push.into(),
            ..ServerLink::new(messenger_core::RelayUrl::parse(relay).unwrap())
        };

        let with = card("wss://relay.s2.example", Some("https://push.s2.example"), "veydan");
        let preview = rt.server_preview(&with.encode()).await.unwrap();
        assert_eq!(preview.push_url.as_deref(), Some("https://push.s2.example"));
        let id = rt.server_accept(&with.encode()).await.unwrap();
        let push = servers::push_of(rt.store(), &id).await.unwrap().unwrap();
        assert_eq!((push.push_mode.as_str(), push.push_url.as_deref()), ("veydan", Some("https://push.s2.example")));
        // The card written again from the server names its push server too.
        assert_eq!(rt.server_card(&id).await.unwrap().push_url.as_deref(), Some("https://push.s2.example"));
        assert!(rt.server_card(&id).await.unwrap().encode().contains("&pu=https%3A%2F%2Fpush.s2.example&p=veydan"));

        // No pu=: the mode follows the card, no push server is set.
        let without = card("wss://relay.s3.example", None, "veydan");
        let id = rt.server_accept(&without.encode()).await.unwrap();
        let push = servers::push_of(rt.store(), &id).await.unwrap().unwrap();
        assert_eq!(push.push_url, None);

        // p=off with a pu=: nothing is set.
        let off = card("wss://relay.s4.example", Some("https://push.s4.example"), "off");
        let id = rt.server_accept(&off.encode()).await.unwrap();
        let push = servers::push_of(rt.store(), &id).await.unwrap().unwrap();
        assert_eq!((push.push_mode.as_str(), push.push_url), ("off", None));
        rt.shutdown().await;
    }

    /// On a device of its own servers a card of the Veydan network's relay
    /// adds that one relay, as a private server called «Veydan».
    #[tokio::test]
    async fn a_veydan_card_on_a_device_of_its_own_servers() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        rt.relays().add_user("wss://own.example", None).await.unwrap();
        rt.servers_use_own().await.unwrap();
        let (manifest, _) = rt.relays().current_manifest().await.unwrap();
        let veydan = ServerLink { name: "anything".into(), ..ServerLink::new(messenger_core::RelayUrl::parse(&manifest.relays[0].url).unwrap()) };
        let v = rt.server_preview(&veydan.encode()).await.unwrap();
        assert!(v.is_veydan && !v.known, "{v:?}");
        let id = rt.server_accept(&veydan.encode()).await.unwrap();
        let s = servers::get(rt.store(), &id).await.unwrap().unwrap();
        assert!(s.is_private());
        assert_eq!(s.name.as_deref(), Some(crate::private::card::VEYDAN_SERVER_NAME));
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

    /// The link of my private chat on a server, asked for again and again
    /// (every «Поделиться», every QR shown), carries one invitation of
    /// that server while it lives; it runs out in 30 days, and a new one is
    /// made once it was taken back or is near its end. Another server has
    /// its own.
    #[tokio::test]
    async fn the_link_of_a_private_chat_reuses_one_invitation_that_runs_out() {
        let dir = tempfile::tempdir().unwrap();
        let rt = runtime(&dir).await;
        let me = rt.session_pubkey().await.unwrap();
        let put = |relay: &str| crate::private::PrivateServerInput {
            relay_url: relay.into(),
            key: Some("app-key".into()),
            media_url: None,
            call_node: None,
            name: Some("S1".into()),
            push_mode: None,
        };
        let s1 = rt.server_put_private(&put("wss://relay.s1.example")).await.unwrap();
        let s2 = rt.server_put_private(&put("wss://relay.s2.example")).await.unwrap();
        let secret = |link: String| ContactLink::parse(&link).unwrap().invite.expect("an invitation on the link");

        let first = secret(rt.contact_link_on(&me, None, Some(&s1)).await.unwrap());
        for _ in 0..5 {
            assert_eq!(secret(rt.contact_link_on(&me, None, Some(&s1)).await.unwrap()), first, "the same one again");
        }
        let mine = rt.dm().invite_list().await.unwrap();
        assert_eq!(mine.len(), 1, "no pile of invitations");
        let now = messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs();
        let expires = mine[0].row.expires_at.expect("it runs out");
        assert!((expires - now - PRIVATE_LINK_TTL_SECS).abs() < 60, "in 30 days");
        assert_eq!(mine[0].row.max_uses, Some(i64::from(PRIVATE_LINK_USES)));

        // Another server: its own.
        let other = secret(rt.contact_link_on(&me, None, Some(&s2)).await.unwrap());
        assert_ne!(other, first);
        assert_eq!(rt.dm().invite_list().await.unwrap().len(), 2);

        // Taken back: the next link carries a new one.
        rt.dm().invite_revoke(&mine[0].row.id).await.unwrap();
        let renewed = secret(rt.contact_link_on(&me, None, Some(&s1)).await.unwrap());
        assert_ne!(renewed, first);
        // Near its end: a new one too.
        let id = messenger_store::settings::get(rt.store(), &format!("{KEY_PRIVATE_LINK_INVITE}@{s1}")).await.unwrap().unwrap();
        sqlx::query("UPDATE msg_dm_invites SET expires_at = ? WHERE id = ?")
            .bind(now + PRIVATE_LINK_RENEW_SECS - 60)
            .bind(&id)
            .execute(rt.store().pool())
            .await
            .unwrap();
        let fresh = secret(rt.contact_link_on(&me, None, Some(&s1)).await.unwrap());
        assert_ne!(fresh, renewed);
        // A link of the home chat makes none.
        assert!(ContactLink::parse(&rt.contact_link(&me).await.unwrap()).unwrap().invite.is_none());
        assert_eq!(rt.dm().invite_list().await.unwrap().len(), 4);
        rt.shutdown().await;
    }
}

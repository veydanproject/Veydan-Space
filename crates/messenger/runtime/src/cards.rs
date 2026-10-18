// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Contact cards and what of a profile stays off kind 0.
//!
//! A card is made here from a profile this device holds: mine, with my
//! phone only when the user asks for it, or another person's public one,
//! never with a phone. Its picture is the avatar file this device keeps
//! (mine, or the cache of others), made small; nothing is fetched for it.
//! It goes to a direct chat or a group like any message. "Add contact" on
//! a received card adds the person and, when the card is its sender's own
//! and carries a phone, keeps the phone and tells my other devices.
//!
//! My phone and whether my card carries it go to my other devices by
//! `own.profile` on a change and again once a week, from the session's
//! receipt tick (`crate::receipts`).

use crate::MessengerRuntime;
use messenger_contacts::{ContactCard, ProfileView};
use messenger_core::traits::{SystemClock, UiEvent};
use messenger_core::{Clock, Envelope, MessengerError, Outbound, PubKey, Result};
use messenger_dm::wrap::wrap_own;
use messenger_dm::{DmService, MessageView, UI_EVENT_CONTACT_PRIVATE_UPDATED, UI_EVENT_DM_UPDATED};
use messenger_ingress::Outbox;
use messenger_store::dm_routes;
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use serde::Serialize;
use ts_rs::TS;

/// `GroupView::kind` of a group whose members a manager admits; every
/// other kind is read by anyone with the link.
const GROUP_PRIVATE: &str = "private";

/// My phone, as my profile editor shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct OwnPrivateView {
    pub phone: Option<String>,
    /// My card carries the phone unless the user unticks it.
    pub share_phone: bool,
}

/// What I know of a contact privately: the phone it sent me in its own card.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct ContactPrivateView {
    pub pubkey: String,
    pub phone: Option<String>,
}

/// Queue `own.profile` for my other devices when it is owed again (see
/// `DmService::own_profile_due`). `true` when one was queued.
pub(crate) async fn send_own_profile_if_due(dm: &DmService, outbox: &Outbox, keys: &Keys, now: i64) -> Result<bool> {
    let Some(note) = dm.own_profile_due(now).await? else { return Ok(false) };
    queue_own(outbox, keys, &note, now).await?;
    dm.own_profile_sent(now).await?;
    Ok(true)
}

async fn queue_own(outbox: &Outbox, keys: &Keys, note: &Envelope, now: i64) -> Result<()> {
    let event = wrap_own(keys, &note.encode(), now)?;
    outbox.enqueue(Outbound::PublishOwn { event }).await?;
    Ok(())
}

/// A profile nobody published: the key alone.
fn bare_profile(pk: &PubKey) -> ProfileView {
    let npub = nostr::prelude::PublicKey::from_hex(pk.as_hex()).ok().and_then(|p| p.to_bech32().ok()).unwrap_or_else(|| pk.as_hex().into());
    ProfileView {
        pubkey: pk.as_hex().into(),
        npub,
        name: None,
        display_name: None,
        about: None,
        picture: None,
        banner: None,
        website: None,
        nip05: None,
        lud16: None,
        nip05_verified: false,
        event_created_at: 0,
        fetched_at: 0,
        bio: vec![],
        bio_source: None,
        socials: vec![],
        links: vec![],
    }
}

impl MessengerRuntime {
    pub async fn own_private_get(&self) -> Result<OwnPrivateView> {
        let kept = self.dm.own_private().await?;
        Ok(OwnPrivateView { phone: kept.phone, share_phone: kept.share_phone })
    }

    /// Set my phone (any way of writing a number; `None` or empty removes
    /// it) and whether my card carries it by default; my other devices are
    /// told. Error: `phone_invalid`.
    pub async fn own_private_set(&self, phone: Option<&str>, share_phone: bool) -> Result<OwnPrivateView> {
        let (kept, note) = self.dm.set_own_private(phone, share_phone).await?;
        let queued = match self.session_keys().await {
            Ok(keys) => {
                let now = SystemClock.now().secs();
                match queue_own(&self.outbox, &keys, &note, now).await {
                    Ok(()) => {
                        self.dm.own_profile_sent(now).await?;
                        self.outbox.kick();
                        true
                    }
                    Err(e) => {
                        eprintln!("messenger own profile: {e}");
                        false
                    }
                }
            }
            Err(_) => false,
        };
        // Owed: the receipt tick of the next session sends it.
        if !queued {
            self.dm.own_profile_owed().await?;
        }
        Ok(OwnPrivateView { phone: kept.phone, share_phone: kept.share_phone })
    }

    /// The phone `pubkey` (hex or npub) sent me in its own card, if any.
    pub async fn contact_private_get(&self, pubkey: &str) -> Result<ContactPrivateView> {
        let pk = messenger_contacts::book::parse_key(pubkey)?;
        let phone = self.dm.contact_private(pk.as_hex()).await?.and_then(|c| c.phone);
        Ok(ContactPrivateView { pubkey: pk.as_hex().into(), phone })
    }

    /// Send a contact card to `chat`: a person (hex, npub or `dm:<hex>`) or
    /// `group:<id>`. `pubkey` is whose card (hex or npub); `None` or my own
    /// key is mine, with my phone when `include_phone`. Another person's
    /// card is their public profile and never carries a phone. My phone
    /// never goes to a public group, which anyone with its link reads:
    /// `phone_public_group`.
    pub async fn card_send(&self, chat: &str, pubkey: Option<&str>, include_phone: bool) -> Result<MessageView> {
        self.card_send_with_server(chat, pubkey, include_phone, None).await
    }

    /// The same, my own card carrying the card of private server `server`
    /// of mine (`server_unknown` for one this device does not have;
    /// `card_server_not_mine` for the card of somebody else): whoever gets
    /// it may add the server and open a private chat with me there
    /// (internal/messenger-wire.md, §9.4). Sent to a person, it approves
    /// them in that private chat on my side, silently: giving them the
    /// server is asking them there, so their accept, when they open the
    /// chat (`contact_open_private`), opens it on both sides at once.
    ///
    /// Never into a group (`card_server_group`): the card carries the app
    /// key, the only thing that lets anybody in to the server, and a group
    /// is many people (a public one: anybody with its link). A server is
    /// given to a person.
    pub async fn card_send_with_server(&self, chat: &str, pubkey: Option<&str>, include_phone: bool, server: Option<&str>) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let server = server.map(str::trim).filter(|s| !s.is_empty());
        if server.is_some() && chat.trim().strip_prefix("dm:").unwrap_or(chat.trim()).starts_with("group:") {
            return Err(MessengerError::Invalid("card_server_group".into()));
        }
        let server_card = match server {
            Some(id) => {
                let me = keys.public_key().to_hex();
                let whose = pubkey.map(str::trim).filter(|p| !p.is_empty());
                if whose.is_some_and(|p| messenger_contacts::book::parse_key(p).map_or(true, |k| k.as_hex() != me)) {
                    return Err(MessengerError::Invalid("card_server_not_mine".into()));
                }
                Some((id.to_string(), self.server_card(id).await?.encode()))
            }
            None => None,
        };
        let to = chat.strip_prefix("dm:").unwrap_or(chat);
        if let Some(group) = to.strip_prefix("group:") {
            // My card in a group of a private server names that server's
            // relays, as in a private chat there.
            let server = crate::scopes::server_of_group(&self.store, group).await?;
            // No server card here: refused above.
            let card = self.card_of(&keys, pubkey, include_phone, server.as_deref()).await?;
            if card.phone.is_some() {
                let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
                // An unknown group is refused below as such.
                if self.groups().get(group, &me).await?.is_some_and(|g| g.kind != GROUP_PRIVATE) {
                    return Err(MessengerError::Invalid("phone_public_group".into()));
                }
            }
            let (message, out) = {
                let lock = self.groups().lock_of(group).await;
                let _guard = lock.lock().await;
                self.group_driver.renew_stale_key(&keys, group).await?;
                self.groups().prepare_card(&keys, group, &card).await?
            };
            let id = message.id.clone();
            return self.publish_group_message(message, out, &id, true).await;
        }
        // A person, in the home chat or a private one (`<key>@<server>`).
        let peer = self.peer_addr(to).await?;
        let card = self.card_of(&keys, pubkey, include_phone, peer.server()).await?;
        let card = card.with_server(server_card.as_ref().map(|(_, link)| link.as_str()))?;
        let first = self.dm.chat(&peer.chat_id()).await?.is_none();
        let prepared = self.dm.prepare_card(&keys, &peer, &card).await?;
        if first {
            let _ = self.request_profile_on(&peer.key, peer.server()).await;
            let _ = self.resubscribe_meta().await;
        }
        let sent = self.publish_prepared(prepared).await?;
        if let Some((id, _)) = server_card {
            self.approve_on_server(&keys, &peer.key, &id).await?;
        }
        Ok(sent)
    }

    /// I gave `peer` the card of my private server `server`: they are
    /// approved in the private chat there, silently (no signal, and no
    /// request of mine to answer), unless I decided about them there
    /// already (approved, or blocked: a block stands).
    async fn approve_on_server(&self, keys: &Keys, peer: &PubKey, server: &str) -> Result<()> {
        if peer.as_hex() == keys.public_key().to_hex() {
            return Ok(());
        }
        let chat = messenger_dm::DmPeer::on(peer.clone(), Some(server));
        let relation = self.dm.relation(&chat).await?;
        if relation.my_contact == "approved" || relation.blocked {
            return Ok(());
        }
        let result = self.dm.act(keys, &chat, messenger_dm::Action::Request).await?;
        self.after_act(keys, &chat, messenger_dm::Action::Request, result).await?;
        Ok(())
    }

    /// The card of `pubkey` (`None`: mine) as this device can make it. The
    /// relays it names are of the chat it goes to (`server`, `None` for the
    /// home set): my card in a private chat names that server's relays, and
    /// never tells its peer where I am at home.
    async fn card_of(&self, keys: &Keys, pubkey: Option<&str>, include_phone: bool, server: Option<&str>) -> Result<ContactCard> {
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let pk = match pubkey.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => messenger_contacts::book::parse_key(p)?,
            None => me.clone(),
        };
        let mine = pk == me;
        let profile = self.profiles.get(&pk).await?.unwrap_or_else(|| bare_profile(&pk));
        let (relays, phone) = if mine {
            let relays = self.relays.enabled_of(server).await?.into_iter().filter(|r| r.read).map(|r| r.url).collect();
            let phone = if include_phone { self.dm.own_private().await?.phone } else { None };
            (relays, phone)
        } else {
            (dm_routes::for_peer(&self.store, pk.as_hex(), server).await?, None)
        };
        let avatar = match profile.picture.as_deref() {
            Some(url) => self.avatars.local_bytes(url).await,
            None => None,
        };
        let now = SystemClock.now().secs();
        tokio::task::spawn_blocking(move || ContactCard::from_profile(&profile, &relays, phone.as_deref(), avatar.as_deref(), now))
            .await
            .map_err(|e| MessengerError::Io(e.to_string()))?
    }

    /// "Add contact" on a received card: the person joins my contacts, and
    /// when the card is its sender's own and carries a phone, the phone is
    /// kept (`contact_private_get`) and my other devices are told. Returns
    /// the message again. Errors: `card_unknown`, `card_is_me`.
    pub async fn card_accept(&self, message_id: &str) -> Result<MessageView> {
        let keys = self.session_keys().await?;
        let stored = self.dm.stored_card(message_id).await?;
        let pk = PubKey::parse(&stored.card.pubkey).ok_or_else(|| MessengerError::Invalid("card_unknown".into()))?;
        if pk.as_hex() == keys.public_key().to_hex() {
            return Err(MessengerError::Invalid("card_is_me".into()));
        }
        if !self.contacts.is_contact(&pk).await? {
            self.contact_add(pk.as_hex(), None).await?;
        }
        let phone = stored.card.phone.as_deref().filter(|_| stored.incoming && stored.sender == pk.as_hex());
        if let Some(phone) = phone {
            if let Some(note) = self.dm.keep_contact_phone(pk.as_hex(), Some(phone), stored.card.at).await? {
                self.tell_own_devices(&note).await;
                let _ = self.ui.send(UiEvent {
                    name: UI_EVENT_CONTACT_PRIVATE_UPDATED.into(),
                    payload: serde_json::json!({ "pubkey": pk.as_hex() }),
                });
            }
        }
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_DM_UPDATED.into(),
            payload: serde_json::json!({ "chat_id": stored.chat_id, "message_id": message_id }),
        });
        self.dm.message(message_id).await?.ok_or_else(|| MessengerError::Invalid("card_unknown".into()))
    }
}

#[cfg(test)]
mod tests;

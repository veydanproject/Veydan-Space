// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A server reached by its card (internal/messenger-wire.md, «Принятие
//! сервера», stage Э4 of private servers): the card alone
//! (`veydan://server/…`), on the link of a person or a group (`srv=`), or
//! in a contact card sent in a chat. The app shows what will be added and
//! whether it is Veydan's (`server_preview`), and adds it only when the
//! user says so (`server_accept`); then the private chat with the person
//! opens there (`contact_open_private`) or the group's link is opened as
//! any (`group_open_link`).

use super::PrivateServerInput;
use crate::links::{PrivateServerView, ServerPreview};
use crate::MessengerRuntime;
use messenger_core::{MessengerError, Result};
use messenger_dm::{Action, ChatView};
use messenger_groups::GroupLink;
use messenger_links::{ContactLink, LinkType, ServerLink, Uri};
use messenger_store::servers;

/// What a private server added from a card of the Veydan network's own
/// relay is called: the dialog said «серверы Veydan».
pub const VEYDAN_SERVER_NAME: &str = "Veydan";

/// The card of a server a text carries: the card itself, or the `srv=` of
/// the link of a person or a group. `server_link_none` for a link that
/// carries none; the code of the link for one that is no link.
pub fn card_of_text(text: &str) -> Result<ServerLink> {
    let uri = Uri::parse(text.trim())?;
    let card = match uri.link_type() {
        LinkType::Server => Some(ServerLink::from_uri(&uri)?),
        LinkType::Contact => ContactLink::from_uri(&uri)?.srv,
        LinkType::Group => GroupLink::from_uri(&uri)?.srv,
        _ => None,
    };
    card.ok_or_else(|| MessengerError::Invalid("server_link_none".into()))
}

impl MessengerRuntime {
    /// What the server of `link` (a server card, or the link of a person or
    /// a group with `srv=`) is, before anything is added: the dialog of
    /// every path that adds a server shows it.
    pub async fn server_preview(&self, link: &str) -> Result<ServerPreview> {
        self.server_preview_of(&card_of_text(link)?).await
    }

    /// Add the server of `link` (as `server_preview` reads it) after the
    /// user said yes. A server this device has already is left as it is if
    /// it is of the home set (its relay is never made private), and takes
    /// the fields of the newer card if it is private (a card is how a
    /// server's address, node or key change: «Серверы на всех устройствах»).
    /// Otherwise it becomes a private server of mine, here and on my other
    /// devices (`server_put_private`); a card of the Veydan network's own
    /// relay (on a device in the mode of its own servers, or one whose home
    /// set lacks that relay) adds the one relay it names, as a private
    /// server called «Veydan», as the dialog said. The server's id.
    pub async fn server_accept(&self, link: &str) -> Result<String> {
        let card = card_of_text(link)?;
        let preview = self.server_preview_of(&card).await?;
        if let Some(have) = servers::by_relay_url(self.store(), card.relay.as_str()).await? {
            if !have.is_private() {
                return Ok(have.id);
            }
        }
        let name = if preview.is_veydan {
            Some(VEYDAN_SERVER_NAME.to_string())
        } else {
            Some(card.name.clone()).filter(|n| !n.trim().is_empty())
        };
        let id = self
            .server_put_private(&PrivateServerInput {
                relay_url: card.relay.as_str().to_string(),
                key: card.key.clone(),
                media_url: card.media.clone(),
                call_node: card.call_node.clone(),
                name,
                push_mode: Some(card.push.clone()),
            })
            .await?;
        // The card names the push server of the server: a phone registers
        // there through the gateway (`push/gateway.rs`). Without `pu=`, or
        // with `p=off`, the push of the server is left as it is.
        if card.push == servers::PUSH_VEYDAN {
            if let Some(url) = card.push_url.clone() {
                self.server_set_push(&id, Some(url)).await?;
            }
        }
        Ok(id)
    }

    /// Open the private chat with `peer` (hex or npub) on private server
    /// `server` of mine (`server_unknown` otherwise), the one a link with
    /// `srv=` or a card with a server leads to: `dm:<peer>@<server>`.
    ///
    /// The acceptance is mutual by the card. The person who gave it (a
    /// link of theirs, or their card in a chat) asked me to talk to them
    /// there: a card sent in a chat approved me there on their side
    /// already (`card_send_with_server`), and a link may carry an
    /// invitation of theirs (`invite`): a link their app makes for a private
    /// chat always carries one (`contact_link_on` makes it, as the home link
    /// of an invitation has), so the opener is let straight in there. The
    /// link of the kit (`veydan-server card --contact`) has none, the server
    /// being unable to make one for a person: that first message is a
    /// request. Opening the chat is my approval,
    /// told at once (my accept, with the secret when there is one), so
    /// their device answers with theirs and the chat is open on both
    /// sides without a request to answer. A device of theirs that never
    /// approved me (a link without an invitation, of the kit) does not hear
    /// a bare accept: my first text there is a request, as anyone's.
    ///
    /// A chat that is open already, or one I blocked, is only opened. A
    /// server of my home set (the card named a relay that is home here) is
    /// no private chat: the home chat with the person is opened, as a
    /// person's link without a card opens it.
    pub async fn contact_open_private(
        &self,
        peer: &str,
        server: &str,
        invite: Option<&str>,
    ) -> Result<ChatView> {
        let keys = self.session_keys().await?;
        let key = messenger_contacts::book::parse_key(peer)?;
        if servers::get(self.store(), server.trim())
            .await?
            .is_some_and(|s| !s.is_private())
        {
            return self.chat_open(key.as_hex()).await;
        }
        let addr = format!("{}@{}", key.as_hex(), server.trim());
        let chat = self.peer_addr(&addr).await?;
        let relation = self.dm.relation(&chat).await?;
        let open = relation.my_contact == "approved" && relation.peer_signal == "approved";
        if !(open || relation.blocked || relation.banned) {
            match invite.map(str::trim).filter(|i| !i.is_empty()) {
                Some(secret) => {
                    self.dm_request(&addr, Some(secret), None).await?;
                }
                None => {
                    let result = self.dm.request_told(&keys, &chat).await?;
                    self.after_act(&keys, &chat, Action::Request, result)
                        .await?;
                }
            }
        }
        self.chat_open(&addr).await
    }

    /// My private servers as the settings list them, without their keys,
    /// each with the count of its private chats and of its groups (what the
    /// question before its removal names).
    pub async fn servers_private_view(&self) -> Result<Vec<PrivateServerView>> {
        let mut out = Vec::new();
        for row in self.servers_private().await? {
            let (chats, groups) = servers::counts(self.store(), &row.id).await?;
            out.push(PrivateServerView { chats, groups, ..PrivateServerView::from(&row) });
        }
        Ok(out)
    }
}

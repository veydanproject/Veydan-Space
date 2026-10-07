// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Contact cards in a direct chat (`{"v":1,"t":"contact","card":{…}}`,
//! messenger-contacts `card`). The host makes the card; this module sends
//! it like any message and keeps a received one only as `card::validate`
//! reads it, without a phone unless the card is its sender's own. A group
//! keeps its cards the same way (`received_card`).

use crate::service::{DmService, Prepared};
use messenger_contacts::card::{self, ContactCard};
use messenger_core::{Envelope, MessengerError, PubKey, Result};
use messenger_store::messages as repo;
use nostr::key::Keys;

/// A card as it is kept from a message of `author` (hex): checked, and
/// without a phone unless the card is the author's own. `None` when the
/// envelope holds no card worth keeping.
pub fn received_card(envelope: &Envelope, author: &str) -> Option<ContactCard> {
    let card = card::validate(envelope.fields.get("card")?).ok()?;
    Some(if card.pubkey == author { card } else { card.without_phone() })
}

/// A card kept in a message, as `DmService::stored_card` finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredCard {
    pub card: ContactCard,
    pub chat_id: String,
    /// Who sent the message (hex).
    pub sender: String,
    /// The message came from somebody else.
    pub incoming: bool,
}

impl DmService {
    /// Store an outgoing card for `peer` and build its wraps. The card is
    /// sent as it is: the host decides what it holds.
    pub async fn prepare_card(&self, keys: &Keys, peer: &PubKey, card: &ContactCard) -> Result<Prepared> {
        let json = card.to_json();
        let envelope = Envelope::contact(json.clone());
        self.prepare_visible(keys, peer, envelope, repo::CT_CONTACT, None, None, Some(json.to_string())).await
    }

    /// The card a message holds, from a direct chat or a group. Errors:
    /// `card_unknown` (no such message, or not a card, or removed).
    pub async fn stored_card(&self, message_id: &str) -> Result<StoredCard> {
        let unknown = || MessengerError::Invalid("card_unknown".into());
        let row = repo::get(&self.store, message_id).await?.filter(|r| !r.is_hidden && r.deleted_at.is_none()).ok_or_else(unknown)?;
        if row.content_type != repo::CT_CONTACT {
            return Err(unknown());
        }
        let card: ContactCard = row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).ok_or_else(unknown)?;
        Ok(StoredCard { card, chat_id: row.chat_id, incoming: row.direction == repo::DIR_IN, sender: row.sender_pubkey })
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The client's side: make the proof of a message this device holds.
//!
//! A proof is made only of what is checked again before it leaves: a
//! client that sends a proof the server refuses has only told the server
//! that it reported somebody.

use nostr::event::{Event, Kind, UnsignedEvent};
use nostr::key::Keys;
use nostr::nips::nip44::{self as nostr_nip44, v2::ConversationKey};

use crate::nip44::{self, Payload};
use crate::verify::{DM_KINDS, KIND_GIFT_WRAP, KIND_GROUP_MESSAGE, KIND_SEAL};
use crate::wire::Proof;
use crate::Error;

/// The proof of a DM from the gift wrap it came in. `keys` are the keys of
/// the device's user, to whom the wrap is addressed: the peer's copy of a
/// message for a message of the peer, my own copy for a message of mine.
///
/// The wrap is opened as NIP-59 opens it, but the seal is kept: the seal is
/// what its author signed. Then the keys of the seal's payload alone are
/// derived from the conversation key with the payload's nonce, and the
/// conversation key is forgotten.
pub fn dm_proof(keys: &Keys, wrap: &Event) -> Result<Proof, Error> {
    if wrap.kind != Kind::from(KIND_GIFT_WRAP) {
        return Err(Error::NotAWrap);
    }
    wrap.verify().map_err(|_| Error::NotAWrap)?;
    let seal_json = nostr_nip44::decrypt(keys.secret_key(), &wrap.pubkey, &wrap.content)
        .map_err(|_| Error::NotMine)?;
    let seal: Event = serde_json::from_str(&seal_json).map_err(|_| Error::Seal("not an event"))?;
    if seal.kind != Kind::from(KIND_SEAL) {
        return Err(Error::Seal("not a seal"));
    }
    seal.verify().map_err(|_| Error::Seal("signature"))?;

    let payload = Payload::parse(&seal.content).map_err(Error::Payload)?;
    let conversation = ConversationKey::derive(keys.secret_key(), &seal.pubkey)
        .map_err(|_| Error::Seal("author key"))?;
    let conversation: [u8; 32] = conversation
        .as_bytes()
        .try_into()
        .map_err(|_| Error::Seal("conversation key"))?;
    let message_keys = nip44::message_keys(&conversation, &payload.nonce);

    // Open it as the server will, so that a proof that cannot verify never
    // leaves the device.
    let plain = payload.open(&message_keys).map_err(Error::Payload)?;
    let rumor: UnsignedEvent = std::str::from_utf8(&plain)
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or(Error::Rumor("not an event"))?;
    if rumor.pubkey != seal.pubkey {
        return Err(Error::Rumor("not the seal's author"));
    }
    if !DM_KINDS.contains(&rumor.kind.as_u16()) {
        return Err(Error::Rumor("not a message"));
    }
    Ok(Proof::Dm {
        seal,
        keys: message_keys,
    })
}

/// The proof of a group message from the inner event as the group's sealed
/// event held it (`Payload.event` of `messenger_groups::wire`, the value
/// `verify_message` reads): it is signed by its author already and goes as
/// it is.
pub fn group_proof(inner_signed_json: &serde_json::Value) -> Result<Proof, Error> {
    let inner: Event = serde_json::from_value(inner_signed_json.clone())
        .map_err(|_| Error::Group("not an event"))?;
    if inner.kind != Kind::from(KIND_GROUP_MESSAGE) {
        return Err(Error::Group("not a message"));
    }
    inner.verify().map_err(|_| Error::Group("signature"))?;
    if !inner.tags.iter().any(|t| t.kind() == "h") {
        return Err(Error::Group("no group"));
    }
    Ok(Proof::Group { inner })
}

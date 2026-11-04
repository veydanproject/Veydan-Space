// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes of channels on the wire (internal/communities-wire.md §7): a
//! rumor sealed and gift-wrapped by NIP-59, made and opened here with the
//! `nostr` crate the way the messenger's direct messages do it
//! (`messenger-dm::wrap`), without depending on them: a note to a keeper
//! is wrapped to the keeper's own key, which the messenger never holds.
//!
//! Two rumor kinds: 30079 for a note to somebody else (`channel.*`,
//! `keeper.master`), 30078 for a note to my own devices
//! (`own.channel_keys`).

use messenger_core::envelope::{KIND_OWN_RUMOR, KIND_PEER_NOTE_RUMOR};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::nips::nip59::{GiftWrapBuilder, UnwrappedGift};
use nostr::prelude::*;

/// The rumor of a note to somebody else.
pub const KIND_NOTE: u16 = KIND_PEER_NOTE_RUMOR;
/// The rumor of a note to my own devices.
pub const KIND_OWN: u16 = KIND_OWN_RUMOR;
/// The kind of a gift wrap.
pub const KIND_WRAP: u16 = 1059;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn wire(event: Event) -> Result<WireEvent> {
    Ok(WireEvent { id: EventId::parse(&event.id.to_hex()).ok_or_else(|| crypto("event id"))?, json: serde_json::to_value(&event)? })
}

fn rumor(from: &Keys, kind: u16, to: PublicKey, content: &str, created_at: i64) -> UnsignedEvent {
    let mut r = EventBuilder::new(Kind::from(kind), content)
        .tag(Tag::public_key(to))
        .custom_created_at(Timestamp::from_secs(created_at.max(0) as u64))
        .finalize_unsigned(from.public_key());
    r.ensure_id();
    r
}

/// The mark on the outside of the wrap of every note of the channels,
/// `["channel", "1"]`, as the messenger's `pushtags::CHANNEL` and the push
/// plugin's `Keeper.kt` name it.
pub const PUSH_MARK: &str = "channel";

/// A note from `from` to `to`: rumor 30079, sealed by `from`, wrapped to
/// `to`, marked [`PUSH_MARK`] outside. No note of the channels is read as
/// a message, and the recipient's push server pushes an unmarked wrap to
/// its key as a direct message: a join to the keeper's inbox (the owner's
/// key, wire §3 `inbox`) rang "New message" (ROO-633), so did an answer
/// to an invitation to the one who invited (ROO-645). Marked, the push is
/// the keeper's: shown to nobody, the app takes a turn.
pub fn wrap_note(from: &Keys, to: &PubKey, content: &str, created_at: i64) -> Result<WireEvent> {
    let receiver = PublicKey::from_hex(to.as_hex()).map_err(crypto)?;
    let r = rumor(from, KIND_NOTE, receiver, content, created_at);
    marked(GiftWrapBuilder::new(receiver, r), from)
}

fn marked(wrap: GiftWrapBuilder, from: &Keys) -> Result<WireEvent> {
    let wrap = wrap.extra_tags([Tag::parse([PUSH_MARK, "1"]).map_err(crypto)?]);
    wire(wrap.finalize(from).map_err(crypto)?)
}

/// A note to my own devices: rumor 30078, sealed and wrapped to my key,
/// marked as [`wrap_note`]: my push server would push it to my other
/// phone as a direct message.
pub fn wrap_own(keys: &Keys, content: &str, created_at: i64) -> Result<WireEvent> {
    let me = keys.public_key();
    let r = rumor(keys, KIND_OWN, me, content, created_at);
    marked(GiftWrapBuilder::new(me, r), keys)
}

/// A note opened: who sealed it (the seal's signature, which the rumor's
/// author must match), its kind, its content and its date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The id of the rumor, made again from what it holds: the same on
    /// every copy and every wrap of one note (a replay, a second relay,
    /// the door of the messenger), new for a note sent again.
    pub id: String,
    pub sender: PubKey,
    pub kind: u16,
    pub content: String,
    pub created_at: i64,
}

/// Opens a gift wrap with `keys`; `Err` when it is not for them or does
/// not hold together.
pub fn open(keys: &Keys, wrap: &serde_json::Value) -> Result<Opened> {
    let event: Event = serde_json::from_value(wrap.clone()).map_err(|_| crypto("not an event"))?;
    if event.kind.as_u16() != KIND_WRAP {
        return Err(crypto("not a gift wrap"));
    }
    let u = UnwrappedGift::from_gift_wrap(keys, &event).map_err(crypto)?;
    // The seal says who sealed; a rumor that names somebody else is a
    // forgery of its author.
    if u.rumor.pubkey != u.sender {
        return Err(crypto("rumor and seal of different authors"));
    }
    let mut rumor = u.rumor.clone();
    rumor.id = None;
    rumor.ensure_id();
    Ok(Opened {
        id: rumor.id.map(|i| i.to_hex()).unwrap_or_default(),
        sender: PubKey::parse(&u.sender.to_hex()).ok_or_else(|| crypto("sender"))?,
        kind: u.rumor.kind.as_u16(),
        content: u.rumor.content.clone(),
        created_at: u.rumor.created_at.as_secs() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    #[test]
    fn a_note_opens_for_its_receiver_only() {
        let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
        let w = wrap_note(&a, &pk(&b), "{\"v\":1,\"t\":\"channel.leave\"}", 1_760_000_000).unwrap();
        let o = open(&b, &w.json).unwrap();
        assert_eq!((o.sender, o.kind, o.created_at), (pk(&a), KIND_NOTE, 1_760_000_000));
        assert!(o.content.contains("channel.leave"));
        assert!(open(&c, &w.json).is_err());
        assert!(!w.json.to_string().contains("channel.leave"), "nothing of it on the outside");
        let own = wrap_own(&a, "{}", 5).unwrap();
        assert_eq!(open(&a, &own.json).unwrap().kind, KIND_OWN);
    }

    /// Every note of the channels carries the mark of the push outside, to
    /// a keeper or to a person, and to my own devices, and opens as any
    /// note: none is read as a message, and an unmarked one is pushed as a
    /// direct message ("New message" for `channel.invite_reply`, ROO-645).
    #[test]
    fn every_note_is_marked_outside() {
        use crate::wire::{
            T_ADMIT, T_INVITE, T_INVITE_REPLY, T_ISSUE, T_JOIN, T_KEEPER_HANDOVER, T_KEEPER_MASTER, T_LEAVE, T_LEVEL_REQUEST, T_LEVEL_RESOLVE, T_MISSED, T_NOT_LET_IN,
            T_REBUNDLE, T_REJECTED, T_REQUEST, T_TICKET, T_TICKET_REQUEST, T_TICKET_USE, T_TRANSFER_DECLINE, T_TRANSFER_OFFER, T_UPLOAD_KEYS,
        };
        let (a, b) = (Keys::generate(), Keys::generate());
        let mark = |w: &WireEvent| w.json["tags"].as_array().unwrap().iter().filter(|t| t[0] == PUSH_MARK).cloned().collect::<Vec<_>>();
        let all = [
            T_JOIN, T_REQUEST, T_INVITE, T_INVITE_REPLY, T_ADMIT, T_REJECTED, T_ISSUE, T_REBUNDLE, T_NOT_LET_IN, T_MISSED, T_LEAVE, T_KEEPER_MASTER,
            T_LEVEL_REQUEST, T_LEVEL_RESOLVE, T_KEEPER_HANDOVER, T_TRANSFER_OFFER, T_TRANSFER_DECLINE, T_TICKET_REQUEST, T_TICKET, T_TICKET_USE, T_UPLOAD_KEYS,
        ];
        for t in all {
            let content = format!("{{\"v\":1,\"t\":\"{t}\"}}");
            let w = wrap_note(&a, &pk(&b), &content, 1_760_000_000).unwrap();
            assert_eq!(mark(&w), vec![serde_json::json!([PUSH_MARK, "1"])], "{t}");
            assert!(open(&b, &w.json).unwrap().content.contains(t));
            let own = wrap_own(&a, &content, 1_760_000_000).unwrap();
            assert_eq!(mark(&own), vec![serde_json::json!([PUSH_MARK, "1"])], "own {t}");
        }
    }
}

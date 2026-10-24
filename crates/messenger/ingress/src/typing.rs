// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The ephemeral wrap of a «typing» to a person (`KIND_TYPING_WRAP`,
//! internal/messenger-wire.md §3 «Печатает…»): the shape of a gift wrap
//! under a kind the relays never keep. nostr's NIP-59 opens kind 1059
//! alone, so the same three steps are taken here by hand.

use messenger_core::envelope::KIND_TYPING_WRAP;
use nostr::key::Keys;
use nostr::prelude::*;

/// Opens a `KIND_TYPING_WRAP` to me: the sender (the signer of the seal)
/// and the rumor, with the checks of NIP-59 — the outer signature, the
/// seal a signed kind 13, the rumor's author the seal's signer.
pub fn unwrap_typing(keys: &Keys, event: &Event) -> Result<(PublicKey, UnsignedEvent), String> {
    if event.kind.as_u16() != KIND_TYPING_WRAP {
        return Err("not a typing wrap".into());
    }
    event.verify().map_err(|e| e.to_string())?;
    let seal = keys.nip44_decrypt(&event.pubkey, &event.content).map_err(|e| e.to_string())?;
    let seal = Event::from_json(seal).map_err(|e| e.to_string())?;
    if seal.kind != Kind::Seal {
        return Err("not a seal".into());
    }
    seal.verify().map_err(|e| e.to_string())?;
    let rumor = keys.nip44_decrypt(&seal.pubkey, &seal.content).map_err(|e| e.to_string())?;
    let rumor = UnsignedEvent::from_json(rumor).map_err(|e| e.to_string())?;
    if rumor.pubkey != seal.pubkey {
        return Err("rumor author does not match seal signer".into());
    }
    Ok((seal.pubkey, rumor))
}

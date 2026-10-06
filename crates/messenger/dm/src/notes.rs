// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notes between me and a peer: a rumor of `KIND_PEER_NOTE_RUMOR` that is
//! not a message of the chat (docs/messenger-wire.md §3, "Записки
//! собеседнику"). They carry receipts (which of my messages reached the
//! peer, and up to when the peer has read the chat), reactions
//! (`crate::reactions`) and the key a contact beats its presence from
//! (`presence.key`, `messenger_store::presence`).
//!
//! I send notes only to a peer we both chose to talk with (`full_chat`),
//! since even a receipt tells that a device is alive. What comes is kept
//! unless one of us blocked the other: a device of mine may hear a note
//! before it learns that we talk (a new login fetches history in any
//! order), and the note is not fetched again. Kept, it still shows only on
//! my own messages in the chat with whoever sent it, so a stranger's note
//! shows nothing and makes no chat. The chat is never named in a note: it
//! is the chat with whoever sent it.
//!
//! What I owe the peers is taken here too (`take_due_delivered`,
//! `take_due_read`); building the wraps and sending them is the runtime's job.

use crate::relationship::{screen_mode, PeerSignal, ScreenMode};
use crate::service::{updated, DmService};
use crate::own::UI_EVENT_CHAT_RECEIPT;
use messenger_core::envelope::{T_PRESENCE_KEY, T_REACTION, T_RECEIPT_DELIVERED, T_RECEIPT_READ};
use messenger_core::traits::UiEvent;
use messenger_core::{Context, DmInbound, Effect, Envelope, EventId, PubKey, Result};
use messenger_store::messages as repo;
use messenger_store::{chats, presence, receipts, settings};
use nostr::key::{Keys, PublicKey};
use sha2::{Digest, Sha256};

/// A message older than this when it comes gets no delivery receipt:
/// history synced on a new login is not news to the peer.
pub const RECEIPT_WINDOW_SECS: i64 = 7 * 86_400;

/// Whether read receipts go both ways. Off: none are sent, none are shown,
/// and the ones that come are not kept. Delivery receipts do not depend on it.
pub const KEY_READ_RECEIPTS: &str = "privacy.read_receipts";

/// A contact told another presence key, or stopped sharing one:
/// `{"peer": "<hex>"}`. What to watch changed.
pub const UI_EVENT_PRESENCE_KEYS_CHANGED: &str = "presence.keys_changed";

impl DmService {
    /// Whether `peer` and I share presence: a contact in my book, and we
    /// both chose to talk (`notes_allowed`). Only such a peer is told my
    /// presence key, and only its key is watched and shown.
    pub async fn presence_allowed(&self, peer: &PubKey) -> Result<bool> {
        Ok(self.contacts.is_contact(peer).await? && self.notes_allowed(peer).await?)
    }

    /// Whether I send `peer` notes. Blocked either way: never. With the
    /// relationship gate off every chat counts as `full_chat`.
    pub(crate) async fn notes_allowed(&self, peer: &PubKey) -> Result<bool> {
        let r = self.load_relation(peer).await?;
        if r.blocked || r.peer_signal == PeerSignal::Blocked {
            return Ok(false);
        }
        Ok(!self.gate_enabled() || screen_mode(&r) == ScreenMode::FullChat)
    }

    /// Whether one of us blocked the other: then what `peer` notes is dropped.
    async fn blocked_either_way(&self, peer: &PubKey) -> Result<bool> {
        let r = self.load_relation(peer).await?;
        Ok(r.blocked || r.peer_signal == PeerSignal::Blocked)
    }

    pub async fn read_receipts_on(&self) -> Result<bool> {
        settings::get_bool(&self.store, KEY_READ_RECEIPTS, true).await
    }

    /// A note from a peer, or my own copy of one I sent (the other `p` is
    /// the peer then).
    pub(crate) async fn apply_peer_note(&self, msg: &DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        let me = &ctx.my_pubkey;
        let from_me = &msg.sender == me;
        let peer = if from_me {
            match msg.recipients.iter().find(|p| *p != me) {
                Some(p) => p.clone(),
                None => return Ok(vec![]),
            }
        } else {
            msg.sender.clone()
        };
        if self.blocked_either_way(&peer).await? {
            return Ok(vec![]);
        }
        let Ok(envelope) = Envelope::parse(&msg.content) else { return Ok(vec![]) };
        let chat_id = chats::dm_chat_id(peer.as_hex());
        match envelope.t.as_str() {
            // A receipt speaks for the one who sent it; mine are never copied
            // to my devices, and one that were would be about me.
            T_RECEIPT_DELIVERED if !from_me => self.delivered(&chat_id, &peer, &envelope, msg.created_at.secs()).await,
            T_RECEIPT_READ if !from_me => {
                let Some(at) = envelope.fields.get("at").and_then(|v| v.as_i64()) else { return Ok(vec![]) };
                self.peer_read(&chat_id, peer.as_hex(), at, msg.created_at.secs()).await
            }
            // Whoever sent it reacted: the peer, or I on another device (my
            // copy). The use was counted where it was made.
            T_REACTION => {
                let (Some(target), Some(emoji), Some(set)) =
                    (envelope.str_field("target"), envelope.str_field("emoji"), envelope.fields.get("set").and_then(|v| v.as_bool()))
                else {
                    return Ok(vec![]);
                };
                self.apply_reaction(&chat_id, &msg.sender, target, emoji, set, msg.created_at.secs()).await
            }
            // A key speaks for the one who sent it; my devices learn my
            // epoch from `own.presence`, never from a copy.
            T_PRESENCE_KEY if !from_me => self.presence_key(&peer, &envelope, msg.created_at.secs()).await,
            _ => Ok(vec![]),
        }
    }

    /// `peer` told the key it beats from (`null`: it stopped sharing), as
    /// of `since`: when that key began, so a device of the peer that tells
    /// an older key late does not undo a rotation. A key comes with its
    /// proof (`presence_proof`): a contact who knows somebody else's key
    /// cannot tell it as its own. `since` is the peer's own word and counts
    /// no further than a second past the note and my clock.
    ///
    /// Kept from whoever sent it unless one of us blocked the other
    /// (`apply_peer_note`), like every note: the relationship may reach
    /// `full_chat` here only after the key came, and the key is not told
    /// twice. Only the keys of approved contacts are watched and shown
    /// (`presence_allowed`).
    async fn presence_key(&self, peer: &PubKey, envelope: &Envelope, sent_at: i64) -> Result<Vec<Effect>> {
        let Some(said) = envelope.fields.get("since").and_then(|v| v.as_i64()).filter(|s| *s >= 0) else {
            return Ok(vec![]);
        };
        let key = match envelope.fields.get("pubkey") {
            Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(k)) if is_presence_key(k) => {
                let proven = envelope.str_field("proof").is_some_and(|proof| presence_proof_ok(k, peer, said, proof));
                if !proven {
                    return Ok(vec![]);
                }
                Some(k.as_str())
            }
            _ => return Ok(vec![]),
        };
        // A key another contact already beats from is not this one's: it
        // could only be told to watch that contact under another name.
        if let Some(k) = key {
            if presence::peer_of(&self.store, k).await?.is_some_and(|owner| owner != peer.as_hex()) {
                return Ok(vec![]);
            }
        }
        let since = said.min(sent_at.min(self.clock.now().secs()) + 1);
        if !presence::put_key(&self.store, peer.as_hex(), key, since).await? {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent {
            name: UI_EVENT_PRESENCE_KEYS_CHANGED.into(),
            payload: serde_json::json!({ "peer": peer.as_hex() }),
        })])
    }

    async fn delivered(&self, chat_id: &str, peer: &PubKey, envelope: &Envelope, at: i64) -> Result<Vec<Effect>> {
        let Some(ids) = envelope.fields.get("ids").and_then(|v| v.as_array()) else { return Ok(vec![]) };
        let mut effects = Vec::new();
        for id in ids.iter().take(receipts::DELIVERED_IDS_PER_CHAT as usize).filter_map(|v| v.as_str()) {
            if EventId::parse(id).is_none() {
                continue;
            }
            // Kept by id even before the message is here (my other device
            // may hear of it first); a known one must be mine, in this chat.
            let row = repo::get(&self.store, id).await?;
            if let Some(r) = &row {
                if r.chat_id != chat_id || r.direction != repo::DIR_OUT {
                    continue;
                }
            }
            if receipts::mark_delivered(&self.store, id, peer.as_hex(), at).await? {
                if let Some(r) = row.filter(|r| !r.is_hidden) {
                    effects.push(Effect::Emit(updated(chat_id, &r.id)));
                }
            }
        }
        Ok(effects)
    }

    /// The peer (or a member of a group) read the chat up to `at`, said at
    /// `sent_at`. A mark past everything the chat holds says no more than
    /// "all of it": it does not cover what I write later. `sent_at` is the
    /// peer's own word, so it counts no further than my clock: a note dated
    /// in the future cannot mark my next messages read.
    pub async fn peer_read(&self, chat_id: &str, member: &str, at: i64, sent_at: i64) -> Result<Vec<Effect>> {
        if !self.read_receipts_on().await? {
            return Ok(vec![]);
        }
        let newest = repo::last_created_at(&self.store, chat_id).await?;
        let sent_at = sent_at.min(self.clock.now().secs());
        let at = at.min(newest.unwrap_or(0).max(sent_at));
        // Kept even before the chat holds anything; nothing to repaint then.
        if !receipts::raise_peer_read(&self.store, chat_id, member, at).await? || newest.is_none() {
            return Ok(vec![]);
        }
        Ok(vec![Effect::Emit(UiEvent { name: UI_EVENT_CHAT_RECEIPT.into(), payload: serde_json::json!({ "chat_id": chat_id }) })])
    }

    /// The delivery receipts this device owes: by chat, the peer, and the
    /// ids, oldest first. Taken once. A peer I do not send notes to gets
    /// none, and is owed none later. If the take cannot be sorted out, it
    /// is undone, so the next one hands the same ids out.
    pub async fn take_due_delivered(&self, now: i64) -> Result<Vec<(String, String, Vec<String>)>> {
        let taken = receipts::take_due_delivered(&self.store, now, now - RECEIPT_WINDOW_SECS).await?;
        let mut out = Vec::new();
        for (chat_id, ids) in &taken {
            let Some(peer) = chat_id.strip_prefix("dm:").and_then(PubKey::parse) else { continue };
            match self.notes_allowed(&peer).await {
                Ok(true) => out.push((chat_id.clone(), peer.as_hex().to_string(), ids.clone())),
                Ok(false) => {}
                Err(e) => {
                    for (_, ids) in &taken {
                        self.owe_delivered_again(ids).await?;
                    }
                    return Err(e);
                }
            }
        }
        Ok(out)
    }

    /// The delivery receipt for these taken ids could not be queued: the
    /// next take hands them out again.
    pub async fn owe_delivered_again(&self, ids: &[String]) -> Result<()> {
        receipts::owe_delivered_again(&self.store, ids).await
    }

    /// The chats read on this device whose peers have not been told, with
    /// the mark to tell them: direct chats and groups. Taken once.
    /// With read receipts off the flags are taken all the same and nothing
    /// is returned: turning them on later does not tell what was read before.
    /// If the take cannot be sorted out, it is undone.
    pub async fn take_due_read(&self) -> Result<Vec<(String, i64)>> {
        let on = self.read_receipts_on().await?;
        let due = receipts::take_due_read(&self.store).await?;
        if !on {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for (chat_id, at) in &due {
            if let Some(peer) = chat_id.strip_prefix("dm:") {
                let allowed = match PubKey::parse(peer) {
                    Some(pk) => self.notes_allowed(&pk).await,
                    None => Ok(false),
                };
                match allowed {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        for (chat_id, _) in &due {
                            self.owe_read_again(chat_id).await?;
                        }
                        return Err(e);
                    }
                }
            }
            out.push((chat_id.clone(), *at));
        }
        Ok(out)
    }

    /// The read receipt of a taken chat could not be queued: the next take
    /// hands it out again, with the mark of that time.
    pub async fn owe_read_again(&self, chat_id: &str) -> Result<()> {
        receipts::owe_read_again(&self.store, chat_id).await
    }
}

/// What the proof of a presence key starts with; nothing else signs it.
pub const PRESENCE_PROOF_CONTEXT: &[u8] = b"veydan-presence-key-v1";

/// What a presence key signs to say it is `owner`'s as of `since`:
/// SHA-256 of [`PRESENCE_PROOF_CONTEXT`], the owner's 32 key bytes and
/// `since` as eight bytes, little endian.
pub(crate) fn presence_proof_digest(owner: &PubKey, since: i64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(PRESENCE_PROOF_CONTEXT);
    h.update(hex::decode(owner.as_hex()).expect("a PubKey is hex"));
    h.update(since.to_le_bytes());
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&h.finalize());
    digest
}

/// The `proof` of a `presence.key` note: a BIP-340 signature by the
/// presence key over `presence_proof_digest`, 128 hex.
pub fn presence_proof(presence: &Keys, owner: &PubKey, since: i64) -> String {
    presence.sign_schnorr(presence_proof_digest(owner, since)).to_hex()
}

/// Whether `proof` shows that the holder of `presence_pubkey` tells it as
/// `owner`'s key as of `since`.
pub fn presence_proof_ok(presence_pubkey: &str, owner: &PubKey, since: i64, proof: &str) -> bool {
    let Some(key) = PublicKey::from_hex(presence_pubkey).ok().and_then(|p| p.xonly().ok()) else { return false };
    let Some(sig) = hex::decode(proof).ok().and_then(|b| <[u8; 64]>::try_from(b).ok()) else { return false };
    let sig = secp256k1::schnorr::Signature::from_byte_array(sig);
    secp256k1::Secp256k1::verification_only().verify_schnorr(&sig, &presence_proof_digest(owner, since), &key).is_ok()
}

/// A presence key as told: 64 lowercase hex of a point on the curve.
fn is_presence_key(k: &str) -> bool {
    k.len() == 64
        && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && PublicKey::from_hex(k).is_ok_and(|p| p.xonly().is_ok())
}

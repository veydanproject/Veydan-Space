// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Application envelope carried inside encrypted content (DM rumors, group
//! messages). One shape for everything: `{"v":1,"t":"<type>", …fields}`.
//! Unknown `t` values are preserved so newer clients can add types without
//! breaking older readers. See internal/messenger-wire.md §2.

use crate::error::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ENVELOPE_VERSION: u32 = 1;

/// Envelope types this version knows. Unknown ones are carried as-is.
pub const T_TEXT: &str = "text";
/// `{"t":"edit","target":"<rumor id>","text":"…"}` — replace a message's text.
pub const T_EDIT: &str = "edit";
/// `{"t":"delete","target":"<rumor id>"}` — retract a message.
pub const T_DELETE: &str = "delete";
/// `{"t":"control","action":"…"}` — DM relationship signal (stage 5b).
pub const T_CONTROL: &str = "control";
/// `{"t":"media", …}` — encrypted blob reference (stage 6).
pub const T_MEDIA: &str = "media";
/// `{"t":"contact","card":{"pubkey":"<hex>",…}}` — a contact card: a
/// person's key and public profile with a tiny picture, and a phone only
/// in the sender's own card (messenger-contacts `card`).
pub const T_CONTACT: &str = "contact";

/// Kind of the rumor that carries a note from one of my devices to the
/// others. Wrapped to my own key only; a client that knows only kind 14
/// leaves it alone. See internal/messenger-wire.md §3, "Свои устройства".
pub const KIND_OWN_RUMOR: u16 = 30078;
/// `{"t":"own.read","chat":"<chat id>","at":<secs>}` — the chat was read up
/// to the message of the peer with this time.
pub const T_OWN_READ: &str = "own.read";
/// `{"t":"own.hide","chat":"<chat id>","target":"<message id>"}` — the
/// message was removed for me.
pub const T_OWN_HIDE: &str = "own.hide";
/// `{"t":"own.emoji","usage":{"👍":[<count>,<secs>],…}}` — the whole map of
/// the emoji I use, so my other devices and a new one learn it. A reader
/// keeps the larger count and the later time of each, so snapshots agree
/// whatever order they come in.
pub const T_OWN_EMOJI: &str = "own.emoji";
/// `{"t":"own.presence","epoch":N,"since":<secs>,"sharing":true}` — the
/// epoch of my presence key, when it began and whether I share presence at
/// all, so my other devices derive the same key, tell it with the same
/// `since` and follow the switch. A reader keeps the larger epoch; at the
/// same epoch the later `since` and "not sharing" win.
pub const T_OWN_PRESENCE: &str = "own.presence";
/// `{"t":"own.profile","phone":"+…"|null,"share_phone":bool,"at":<secs>}` —
/// what of my profile stays off kind 0: my phone and whether my own card
/// carries it by default. The later `at` wins; sent on a change and again
/// every few days, so a new device learns it.
pub const T_OWN_PROFILE: &str = "own.profile";
/// `{"t":"own.card","pubkey":"<hex>","phone":"+…"|null,"at":<secs>}` — the
/// phone a contact sent me in its own card, or `null` when it was taken
/// back. The later `at` wins per contact.
pub const T_OWN_CARD: &str = "own.card";
/// Every type of a note between my devices starts so.
pub const T_OWN_PREFIX: &str = "own.";

/// Kind of the rumor that carries a note from a peer that is not a message:
/// a receipt, a reaction, a key. A client that knows only kind 14 drops it
/// without a word. See internal/messenger-wire.md §3.
pub const KIND_PEER_NOTE_RUMOR: u16 = 30079;
/// `{"t":"receipt.delivered","ids":["<rumor id>",…]}` — these messages of
/// mine reached one of the peer's devices.
pub const T_RECEIPT_DELIVERED: &str = "receipt.delivered";
/// `{"t":"receipt.read","at":<secs>}` — the peer read every message of mine
/// not later than this time.
pub const T_RECEIPT_READ: &str = "receipt.read";
/// `{"t":"reaction","target":"<rumor id>","emoji":"👍","set":true}` — put a
/// reaction on a message, or take it back with `set: false`. The later
/// `created_at` wins per target, author and emoji.
pub const T_REACTION: &str = "reaction";
/// `{"t":"presence.key","pubkey":"<64 hex>","proof":"<128 hex>","since":<secs>}`
/// — the key my presence beats are signed with, told to an approved
/// contact, with a signature by that key that it is mine as of `since`;
/// `pubkey: null` (no proof) says I stopped sharing. The later `since` wins.
pub const T_PRESENCE_KEY: &str = "presence.key";

/// Every type of the signalling of a call starts so. The invitation is a
/// message (kind 14: it wakes the peer's phone); everything after it is a
/// note (`KIND_PEER_NOTE_RUMOR`). See internal/messenger-wire.md §10.
pub const T_CALL_PREFIX: &str = "call.";
/// `{"t":"call.invite","call_id":"<32 hex>","media":"audio"|"video","sdp":"<offer>","ice":[…]}`
/// — I call you. With `restart: true`, on a note within a call: a new
/// offer for the same call (ICE restart).
pub const T_CALL_INVITE: &str = "call.invite";
/// `{"t":"call.answer","call_id":"…","sdp":"<answer>","ice":[…]}` — I take the call.
pub const T_CALL_ANSWER: &str = "call.answer";
/// `{"t":"call.ice","call_id":"…","ice":[…]}` — candidates found after the
/// offer or the answer left.
pub const T_CALL_ICE: &str = "call.ice";
/// `{"t":"call.decline","call_id":"…","reason":"declined"}` — I will not take it.
pub const T_CALL_DECLINE: &str = "call.decline";
/// `{"t":"call.end","call_id":"…","reason":"ended"|"failed"|"timeout"|"answered_elsewhere"|"superseded","answer":"<rumor id>"?}`
/// — the call is over; `answer` names the one answer of several this end
/// is for. `superseded`: we both called at once and this call lost; it
/// never was (no record of it anywhere).
pub const T_CALL_END: &str = "call.end";
/// `{"t":"call.busy","call_id":"…"}` — I am on another call.
pub const T_CALL_BUSY: &str = "call.busy";
/// `{"t":"call.restart","call_id":"…"}` — the called side lost the
/// connection, or its network changed: the caller is asked for a new
/// offer (`call.invite` with `restart: true`). Only the caller makes
/// offers within a call, so two never cross.
pub const T_CALL_RESTART: &str = "call.restart";
/// `{"t":"call.video","call_id":"…","on":true|false}` — my camera (or my
/// screen) went on or off within the call. No renegotiation goes with it:
/// the video of every call is negotiated from the first offer, and frames
/// simply start or stop (internal/messenger-wire.md §10, "Видео").
pub const T_CALL_VIDEO: &str = "call.video";

/// The signalling of a group call goes as quiet notes of the group,
/// sealed with the group key like a message (`t = msg` of
/// `messenger-groups::wire`), so only the members read it. See
/// internal/messenger-wire.md §10, "Групповые звонки".
///
/// `{"t":"call.start","call_id":"<32 hex>","room_id":"<32 hex>","node":"<addr:port#id>",
/// "key":"<access key>"?,"join_token":"<48 hex>","epoch":1,"secret":"<base64 32 bytes>",
/// "media":"audio"|"video","expires_at":<secs>}` — I made a room on a node
/// for this group; come in. The secret of the first epoch rides inside
/// (the note is under the group key already).
pub const T_CALL_START: &str = "call.start";
/// `{"t":"call.join","call_id":"…","participant":<seat>}` — I am in the
/// room as this seat (the node's participant id), so the others can put a
/// name to it.
pub const T_CALL_JOIN: &str = "call.join";
/// `{"t":"call.leave","call_id":"…","participant":<seat>}` — I left the room.
pub const T_CALL_LEAVE: &str = "call.leave";
/// `{"t":"call.epoch","call_id":"…","epoch":N,"secret":"<base64>","join_token":"…"?}`
/// — a new epoch of the frame keys, made when somebody left the room, a
/// seat never proved who it is, or the group lost a member; whoever is
/// in the room moves its keys to it. `join_token` comes from the creator
/// alone, when it changed the token of the room (a member was removed).
pub const T_CALL_EPOCH: &str = "call.epoch";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub t: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl Envelope {
    pub fn new(t: &str) -> Self {
        Self { v: ENVELOPE_VERSION, t: t.to_string(), fields: Map::new() }
    }

    pub fn text(text: &str) -> Self {
        let mut e = Self::new(T_TEXT);
        e.fields.insert("text".into(), Value::String(text.to_string()));
        e
    }

    pub fn edit(target: &str, text: &str) -> Self {
        Self::new(T_EDIT).with("target", target).with("text", text)
    }

    pub fn delete(target: &str) -> Self {
        Self::new(T_DELETE).with("target", target)
    }

    pub fn control(action: &str) -> Self {
        Self::new(T_CONTROL).with("action", action)
    }

    pub fn own_read(chat_id: &str, at: i64) -> Self {
        Self::new(T_OWN_READ).with("chat", chat_id).with("at", at)
    }

    pub fn own_hide(chat_id: &str, target: &str) -> Self {
        Self::new(T_OWN_HIDE).with("chat", chat_id).with("target", target)
    }

    pub fn receipt_delivered(ids: &[String]) -> Self {
        Self::new(T_RECEIPT_DELIVERED).with("ids", ids.to_vec())
    }

    pub fn receipt_read(at: i64) -> Self {
        Self::new(T_RECEIPT_READ).with("at", at)
    }

    pub fn reaction(target: &str, emoji: &str, set: bool) -> Self {
        Self::new(T_REACTION).with("target", target).with("emoji", emoji).with("set", set)
    }

    /// `key` is `(pubkey, proof)`; `None` goes out as `pubkey: null`: I no
    /// longer share my presence.
    pub fn presence_key(key: Option<(&str, &str)>, since: i64) -> Self {
        let e = Self::new(T_PRESENCE_KEY).with("since", since);
        match key {
            Some((pubkey, proof)) => e.with("pubkey", pubkey).with("proof", proof),
            None => e.with("pubkey", Value::Null),
        }
    }

    pub fn own_presence(epoch: u32, since: i64, sharing: bool) -> Self {
        Self::new(T_OWN_PRESENCE).with("epoch", epoch).with("since", since).with("sharing", sharing)
    }

    /// `usage` is `(emoji, count, last_at)`; an emoji named twice keeps its
    /// last entry.
    pub fn own_emoji(usage: &[(String, i64, i64)]) -> Self {
        let map: Map<String, Value> =
            usage.iter().map(|(emoji, count, at)| (emoji.clone(), Value::from(vec![*count, *at]))).collect();
        Self::new(T_OWN_EMOJI).with("usage", map)
    }

    /// `card` is a checked card as JSON (messenger-contacts `ContactCard`).
    pub fn contact(card: Value) -> Self {
        Self::new(T_CONTACT).with("card", card)
    }

    pub fn own_profile(phone: Option<&str>, share_phone: bool, at: i64) -> Self {
        Self::new(T_OWN_PROFILE).with("phone", phone.map_or(Value::Null, Value::from)).with("share_phone", share_phone).with("at", at)
    }

    pub fn own_card(pubkey: &str, phone: Option<&str>, at: i64) -> Self {
        Self::new(T_OWN_CARD).with("pubkey", pubkey).with("phone", phone.map_or(Value::Null, Value::from)).with("at", at)
    }

    /// `ice` is the list of candidates as the calls crate spells them
    /// (`{"candidate","mid","index"}`); `restart` marks a new offer within
    /// a call and is left out otherwise.
    pub fn call_invite(call_id: &str, media: &str, sdp: &str, ice: Vec<Value>, restart: bool) -> Self {
        let e = Self::new(T_CALL_INVITE).with("call_id", call_id).with("media", media).with("sdp", sdp).with("ice", ice);
        if restart {
            e.with("restart", true)
        } else {
            e
        }
    }

    pub fn call_answer(call_id: &str, sdp: &str, ice: Vec<Value>) -> Self {
        Self::new(T_CALL_ANSWER).with("call_id", call_id).with("sdp", sdp).with("ice", ice)
    }

    pub fn call_ice(call_id: &str, ice: Vec<Value>) -> Self {
        Self::new(T_CALL_ICE).with("call_id", call_id).with("ice", ice)
    }

    pub fn call_decline(call_id: &str, reason: &str) -> Self {
        Self::new(T_CALL_DECLINE).with("call_id", call_id).with("reason", reason)
    }

    /// `answer`: the rumor id of the answer this end is for, when the peer
    /// answered from several devices and only one of them is meant.
    pub fn call_end(call_id: &str, reason: &str, answer: Option<&str>) -> Self {
        let e = Self::new(T_CALL_END).with("call_id", call_id).with("reason", reason);
        match answer {
            Some(id) => e.with("answer", id),
            None => e,
        }
    }

    pub fn call_busy(call_id: &str) -> Self {
        Self::new(T_CALL_BUSY).with("call_id", call_id)
    }

    pub fn call_restart(call_id: &str) -> Self {
        Self::new(T_CALL_RESTART).with("call_id", call_id)
    }

    pub fn call_video(call_id: &str, on: bool) -> Self {
        Self::new(T_CALL_VIDEO).with("call_id", call_id).with("on", on)
    }

    /// `node` is the reference of the call node (`address:port#id`); `key`
    /// the access key of a private node, left out when there is none.
    #[allow(clippy::too_many_arguments)]
    pub fn call_start(
        call_id: &str,
        room_id: &str,
        node: &str,
        key: Option<&str>,
        join_token: &str,
        secret_b64: &str,
        media: &str,
        expires_at: i64,
    ) -> Self {
        let e = Self::new(T_CALL_START)
            .with("call_id", call_id)
            .with("room_id", room_id)
            .with("node", node)
            .with("join_token", join_token)
            .with("epoch", 1u32)
            .with("secret", secret_b64)
            .with("media", media)
            .with("expires_at", expires_at);
        match key {
            Some(k) => e.with("key", k),
            None => e,
        }
    }

    pub fn call_join(call_id: &str, participant: u32) -> Self {
        Self::new(T_CALL_JOIN).with("call_id", call_id).with("participant", participant)
    }

    pub fn call_leave(call_id: &str, participant: u32) -> Self {
        Self::new(T_CALL_LEAVE).with("call_id", call_id).with("participant", participant)
    }

    pub fn call_epoch(call_id: &str, epoch: u32, secret_b64: &str, join_token: Option<&str>) -> Self {
        let e = Self::new(T_CALL_EPOCH).with("call_id", call_id).with("epoch", epoch).with("secret", secret_b64);
        match join_token {
            Some(t) => e.with("join_token", t),
            None => e,
        }
    }

    /// Is this the signalling of a call (`t` starts with `call.`).
    pub fn is_call(&self) -> bool {
        self.t.starts_with(T_CALL_PREFIX)
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// Compact JSON. Field order is `v`, `t`, then the rest as inserted;
    /// readers must not depend on order.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("envelope is always serializable")
    }

    /// Strict parse: must be a JSON object with numeric `v` and string `t`.
    /// Versions above `ENVELOPE_VERSION` are rejected; older ones accepted.
    pub fn parse(content: &str) -> Result<Self> {
        let e: Envelope = serde_json::from_str(content)
            .map_err(|e| MessengerError::Invalid(format!("envelope: {e}")))?;
        if e.v == 0 || e.v > ENVELOPE_VERSION {
            return Err(MessengerError::Invalid(format!("envelope v{} unsupported", e.v)));
        }
        if e.t.is_empty() {
            return Err(MessengerError::Invalid("envelope has an empty type".into()));
        }
        Ok(e)
    }

    /// Plain-text body for `t = text`.
    pub fn as_text(&self) -> Option<&str> {
        if self.t == T_TEXT {
            self.str_field("text")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_golden_vector() {
        let e = Envelope::text("hello");
        assert_eq!(e.encode(), r#"{"v":1,"t":"text","text":"hello"}"#);
        let back = Envelope::parse(r#"{"v":1,"t":"text","text":"hello"}"#).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.as_text(), Some("hello"));
    }

    #[test]
    fn edit_delete_control_golden_vectors() {
        assert_eq!(Envelope::edit("ab", "new").encode(), r#"{"v":1,"t":"edit","target":"ab","text":"new"}"#);
        assert_eq!(Envelope::delete("ab").encode(), r#"{"v":1,"t":"delete","target":"ab"}"#);
        assert_eq!(Envelope::control("dm_accept").encode(), r#"{"v":1,"t":"control","action":"dm_accept"}"#);
        let e = Envelope::parse(r#"{"v":1,"t":"edit","target":"ab","text":"new"}"#).unwrap();
        assert_eq!(e.str_field("target"), Some("ab"));
        assert!(e.as_text().is_none(), "edits are not plain text");
    }

    #[test]
    fn receipt_golden_vectors() {
        let ids = vec!["ab".to_string(), "cd".to_string()];
        assert_eq!(Envelope::receipt_delivered(&ids).encode(), r#"{"v":1,"t":"receipt.delivered","ids":["ab","cd"]}"#);
        assert_eq!(Envelope::receipt_read(1_759_700_000).encode(), r#"{"v":1,"t":"receipt.read","at":1759700000}"#);
        let e = Envelope::parse(r#"{"v":1,"t":"receipt.delivered","ids":["ab","cd"]}"#).unwrap();
        assert_eq!(e, Envelope::receipt_delivered(&ids));
        assert_eq!(e.t, T_RECEIPT_DELIVERED);
        let e = Envelope::parse(r#"{"v":1,"t":"receipt.read","at":1759700000}"#).unwrap();
        assert_eq!(e, Envelope::receipt_read(1_759_700_000));
        assert!(e.as_text().is_none(), "a receipt is not text");
    }

    #[test]
    fn reaction_golden_vectors() {
        // The fields after `t` come sorted (serde_json without
        // `preserve_order`), and an emoji stays UTF-8, never a `\u` escape.
        let put = r#"{"v":1,"t":"reaction","emoji":"👍","set":true,"target":"ab"}"#;
        let off = r#"{"v":1,"t":"reaction","emoji":"❤️","set":false,"target":"ab"}"#;
        assert_eq!(Envelope::reaction("ab", "👍", true).encode(), put);
        assert_eq!(Envelope::reaction("ab", "\u{2764}\u{fe0f}", false).encode(), off);
        let e = Envelope::parse(put).unwrap();
        assert_eq!(e, Envelope::reaction("ab", "👍", true));
        assert_eq!(e.t, T_REACTION);
        assert_eq!(e.str_field("emoji"), Some("👍"));
        assert_eq!(e.fields.get("set").and_then(Value::as_bool), Some(true));
        // Another writer's order and escapes read the same.
        let e = Envelope::parse(r#"{"v":1,"t":"reaction","target":"ab","emoji":"❤️","set":false}"#).unwrap();
        assert_eq!(e, Envelope::reaction("ab", "❤️", false));
        assert!(e.as_text().is_none(), "a reaction is not text");
    }

    #[test]
    fn own_emoji_golden_vector() {
        let usage = vec![("👍".to_string(), 12, 1_759_700_000), ("❤️".to_string(), 3, 1_759_600_000)];
        // Keys sorted by their UTF-8 bytes: U+2764 before U+1F44D.
        let wire = r#"{"v":1,"t":"own.emoji","usage":{"❤️":[3,1759600000],"👍":[12,1759700000]}}"#;
        assert_eq!(Envelope::own_emoji(&usage).encode(), wire);
        let e = Envelope::parse(wire).unwrap();
        assert_eq!(e, Envelope::own_emoji(&usage));
        assert_eq!(e.t, T_OWN_EMOJI);
        assert!(e.t.starts_with(T_OWN_PREFIX));
        assert_eq!(Envelope::own_emoji(&[]).encode(), r#"{"v":1,"t":"own.emoji","usage":{}}"#);
    }

    #[test]
    fn presence_golden_vectors() {
        let (key, proof) = ("ab".repeat(32), "cd".repeat(64));
        let wire = format!(r#"{{"v":1,"t":"presence.key","proof":"{proof}","pubkey":"{key}","since":1759700000}}"#);
        assert_eq!(Envelope::presence_key(Some((&key, &proof)), 1_759_700_000).encode(), wire);
        let e = Envelope::parse(&wire).unwrap();
        assert_eq!(e, Envelope::presence_key(Some((&key, &proof)), 1_759_700_000));
        assert_eq!(e.t, T_PRESENCE_KEY);
        assert_eq!(e.str_field("pubkey"), Some(key.as_str()));
        assert_eq!(e.str_field("proof"), Some(proof.as_str()));
        let off = r#"{"v":1,"t":"presence.key","pubkey":null,"since":1759700001}"#;
        assert_eq!(Envelope::presence_key(None, 1_759_700_001).encode(), off);
        let e = Envelope::parse(off).unwrap();
        assert_eq!(e.fields.get("pubkey"), Some(&Value::Null));
        assert_eq!(e.str_field("pubkey"), None);
        assert!(e.as_text().is_none(), "a key is not text");

        let own = r#"{"v":1,"t":"own.presence","epoch":3,"sharing":false,"since":1759700001}"#;
        assert_eq!(Envelope::own_presence(3, 1_759_700_001, false).encode(), own);
        let e = Envelope::parse(own).unwrap();
        assert_eq!(e, Envelope::own_presence(3, 1_759_700_001, false));
        assert_eq!(e.t, T_OWN_PRESENCE);
        assert!(e.t.starts_with(T_OWN_PREFIX));
        assert_eq!(e.fields.get("epoch").and_then(Value::as_u64), Some(3));
        assert_eq!(e.fields.get("sharing").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn contact_card_golden_vector() {
        let key = "ab".repeat(32);
        let card = serde_json::json!({ "pubkey": key, "name": "Анна", "at": 1_759_700_000 });
        let wire = format!(r#"{{"v":1,"t":"contact","card":{{"at":1759700000,"name":"Анна","pubkey":"{key}"}}}}"#);
        assert_eq!(Envelope::contact(card.clone()).encode(), wire);
        let e = Envelope::parse(&wire).unwrap();
        assert_eq!(e, Envelope::contact(card.clone()));
        assert_eq!(e.t, T_CONTACT);
        assert_eq!(e.fields.get("card"), Some(&card));
        assert!(!e.t.starts_with(T_OWN_PREFIX));
        assert!(e.as_text().is_none(), "a card is not text");
    }

    #[test]
    fn own_profile_and_card_golden_vectors() {
        let with = r#"{"v":1,"t":"own.profile","at":1759700000,"phone":"+79991234567","share_phone":true}"#;
        assert_eq!(Envelope::own_profile(Some("+79991234567"), true, 1_759_700_000).encode(), with);
        let e = Envelope::parse(with).unwrap();
        assert_eq!(e, Envelope::own_profile(Some("+79991234567"), true, 1_759_700_000));
        assert_eq!(e.t, T_OWN_PROFILE);
        assert!(e.t.starts_with(T_OWN_PREFIX));
        assert_eq!(e.str_field("phone"), Some("+79991234567"));
        assert_eq!(e.fields.get("share_phone").and_then(Value::as_bool), Some(true));
        let without = r#"{"v":1,"t":"own.profile","at":1759700001,"phone":null,"share_phone":false}"#;
        assert_eq!(Envelope::own_profile(None, false, 1_759_700_001).encode(), without);
        assert_eq!(Envelope::parse(without).unwrap().fields.get("phone"), Some(&Value::Null));

        let key = "cd".repeat(32);
        let card = format!(r#"{{"v":1,"t":"own.card","at":1759700002,"phone":"+15550001111","pubkey":"{key}"}}"#);
        assert_eq!(Envelope::own_card(&key, Some("+15550001111"), 1_759_700_002).encode(), card);
        let e = Envelope::parse(&card).unwrap();
        assert_eq!(e, Envelope::own_card(&key, Some("+15550001111"), 1_759_700_002));
        assert_eq!(e.t, T_OWN_CARD);
        assert!(e.t.starts_with(T_OWN_PREFIX));
        assert_eq!(e.str_field("pubkey"), Some(key.as_str()));
        let gone = format!(r#"{{"v":1,"t":"own.card","at":1759700003,"phone":null,"pubkey":"{key}"}}"#);
        assert_eq!(Envelope::own_card(&key, None, 1_759_700_003).encode(), gone);
    }

    #[test]
    fn call_golden_vectors() {
        let id = "ab".repeat(16);
        let cand = serde_json::json!({ "candidate": "candidate:1 1 udp 2 203.0.113.7 5000 typ host", "index": 0, "mid": "0" });
        let invite = format!(
            r#"{{"v":1,"t":"call.invite","call_id":"{id}","ice":[{{"candidate":"candidate:1 1 udp 2 203.0.113.7 5000 typ host","index":0,"mid":"0"}}],"media":"audio","sdp":"v=0"}}"#
        );
        assert_eq!(Envelope::call_invite(&id, "audio", "v=0", vec![cand.clone()], false).encode(), invite);
        let e = Envelope::parse(&invite).unwrap();
        assert_eq!(e, Envelope::call_invite(&id, "audio", "v=0", vec![cand.clone()], false));
        assert!(e.is_call());
        assert_eq!(e.str_field("call_id"), Some(id.as_str()));
        assert!(e.fields.get("restart").is_none(), "a first offer says nothing of a restart");
        assert_eq!(
            Envelope::call_invite(&id, "video", "v=1", vec![], true).encode(),
            format!(r#"{{"v":1,"t":"call.invite","call_id":"{id}","ice":[],"media":"video","restart":true,"sdp":"v=1"}}"#)
        );
        assert_eq!(
            Envelope::call_answer(&id, "v=2", vec![cand]).encode(),
            format!(
                r#"{{"v":1,"t":"call.answer","call_id":"{id}","ice":[{{"candidate":"candidate:1 1 udp 2 203.0.113.7 5000 typ host","index":0,"mid":"0"}}],"sdp":"v=2"}}"#
            )
        );
        assert_eq!(Envelope::call_ice(&id, vec![]).encode(), format!(r#"{{"v":1,"t":"call.ice","call_id":"{id}","ice":[]}}"#));
        assert_eq!(
            Envelope::call_decline(&id, "declined").encode(),
            format!(r#"{{"v":1,"t":"call.decline","call_id":"{id}","reason":"declined"}}"#)
        );
        assert_eq!(Envelope::call_end(&id, "ended", None).encode(), format!(r#"{{"v":1,"t":"call.end","call_id":"{id}","reason":"ended"}}"#));
        let answer = "cd".repeat(32);
        assert_eq!(
            Envelope::call_end(&id, "answered_elsewhere", Some(&answer)).encode(),
            format!(r#"{{"v":1,"t":"call.end","answer":"{answer}","call_id":"{id}","reason":"answered_elsewhere"}}"#)
        );
        assert_eq!(Envelope::call_busy(&id).encode(), format!(r#"{{"v":1,"t":"call.busy","call_id":"{id}"}}"#));
        assert_eq!(Envelope::call_restart(&id).encode(), format!(r#"{{"v":1,"t":"call.restart","call_id":"{id}"}}"#));
        assert!(Envelope::call_restart(&id).is_call());
        assert_eq!(Envelope::call_video(&id, true).encode(), format!(r#"{{"v":1,"t":"call.video","call_id":"{id}","on":true}}"#));
        assert_eq!(Envelope::parse(&Envelope::call_video(&id, false).encode()).unwrap().fields.get("on"), Some(&serde_json::json!(false)));
        assert!(!Envelope::text("call.invite").is_call(), "text that names a type is text");
        assert!(Envelope::parse(&invite).unwrap().as_text().is_none(), "a call is not text");
    }

    #[test]
    fn group_call_golden_vectors() {
        let (id, room) = ("ab".repeat(16), "cd".repeat(16));
        let node = format!("203.0.113.7:8443#{}", "ef".repeat(32));
        let start = Envelope::call_start(&id, &room, &node, None, &"12".repeat(24), "c2VjcmV0", "audio", 1_760_043_200);
        assert_eq!(
            start.encode(),
            format!(
                r#"{{"v":1,"t":"call.start","call_id":"{id}","epoch":1,"expires_at":1760043200,"join_token":"{}","media":"audio","node":"{node}","room_id":"{room}","secret":"c2VjcmV0"}}"#,
                "12".repeat(24)
            )
        );
        assert!(start.is_call());
        assert!(start.fields.get("key").is_none(), "no key for a public node");
        let keyed = Envelope::call_start(&id, &room, &node, Some("k1"), "t", "s", "video", 1);
        assert_eq!(keyed.str_field("key"), Some("k1"));
        assert_eq!(Envelope::parse(&keyed.encode()).unwrap(), keyed);
        assert_eq!(Envelope::call_join(&id, 3).encode(), format!(r#"{{"v":1,"t":"call.join","call_id":"{id}","participant":3}}"#));
        assert_eq!(Envelope::call_leave(&id, 3).encode(), format!(r#"{{"v":1,"t":"call.leave","call_id":"{id}","participant":3}}"#));
        assert_eq!(
            Envelope::call_epoch(&id, 2, "bmV4dA==", None).encode(),
            format!(r#"{{"v":1,"t":"call.epoch","call_id":"{id}","epoch":2,"secret":"bmV4dA=="}}"#)
        );
        assert_eq!(
            Envelope::call_epoch(&id, 3, "bmV4dA==", Some("t2")).encode(),
            format!(r#"{{"v":1,"t":"call.epoch","call_id":"{id}","epoch":3,"join_token":"t2","secret":"bmV4dA=="}}"#)
        );
        assert_eq!(Envelope::parse(&Envelope::call_epoch(&id, 2, "x", None).encode()).unwrap().fields.get("epoch"), Some(&serde_json::json!(2)));
    }

    #[test]
    fn unknown_type_is_preserved_and_extra_fields_survive() {
        let e = Envelope::parse(r#"{"v":1,"t":"sticker","pack":"p","id":3,"extra":{"a":1}}"#).unwrap();
        assert_eq!(e.t, "sticker");
        assert_eq!(e.str_field("pack"), Some("p"));
        assert!(e.as_text().is_none());
        assert_eq!(Envelope::parse(&e.encode()).unwrap(), e);
    }

    #[test]
    fn rejects_garbage_and_future_versions() {
        assert!(Envelope::parse("hello plain text").is_err());
        assert!(Envelope::parse(r#"{"t":"text"}"#).is_err(), "missing v");
        assert!(Envelope::parse(r#"{"v":2,"t":"text"}"#).is_err(), "future version");
        assert!(Envelope::parse(r#"{"v":0,"t":"text"}"#).is_err());
        assert!(Envelope::parse(r#"{"v":1,"t":""}"#).is_err());
        assert!(Envelope::parse(r#"[1,2]"#).is_err());
    }
}

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

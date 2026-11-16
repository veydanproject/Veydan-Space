// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The focus event: which chat I am in now, said to my other devices and
//! to nobody else (internal/messenger-wire.md §8, «Фокус»).
//!
//! A device whose chat X is open and in use says so on my relays every
//! half minute, so that another device of mine, a message of X coming,
//! shows nothing: not a sound, not a quiet card. The word is a kind 30315
//! under `d = veydan-focus`, addressable as a beat is (a relay keeps the
//! newest per key), signed by a focus key only my devices can make
//! ([`FocusKeys::derive`]): made from my secret as the presence key is,
//! under a context of its own, so that neither key is the other, and told
//! to no contact. The chat is in the content, NIP-44 v2 under a second key
//! of the same making, the plaintext padded to one size ([`PLAIN_LEN`]),
//! so a relay sees a word of one length whether I am in a chat or in the
//! list of chats.
//!
//! The crate holds the keys, the building, the reading and the opening.
//! When to say it, whose word to believe over this device's own, and what
//! a phone woken by a push does with it are the runtime's and the push
//! handler's (ROO-1275, D2 and D3). Nothing here sends or stores anything.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use hmac::{Hmac, KeyInit, Mac};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::key::Keys;
use messenger_evidence::nip44::{decrypt_v2, OpenError};
use nostr::nips::nip44::v2::{encrypt_to_bytes_with_nonce, ConversationKey};
use nostr::prelude::*;
use serde_json::Value;
use sha2::Sha256;

pub use messenger_core::presence::FOCUS_D;
/// The kind of a focus event: the one of a beat, under another `d`.
pub use messenger_core::presence::KIND_PRESENCE as KIND_FOCUS;

/// The HMAC key of the focus key, which signs the event.
pub const KEY_CONTEXT: &[u8] = b"veydan-focus-v1";
/// The HMAC key of the content key, which seals the chat.
pub const CONTENT_CONTEXT: &[u8] = b"veydan-focus-content-v1";
/// How long a focus event says I am in the chat: its NIP-40 `expiration`
/// is `created_at` plus this, and a reader believes it no longer, whatever
/// the tag says. The same as a beat's, for the same reason: one lost
/// event of two a minute does not end the focus.
pub const FOCUS_TTL_SECS: i64 = 80;
/// The length of the plaintext under the seal, whatever the chat: the
/// JSON padded with spaces to this. Room for the longest chat id of the
/// wire (`group:` and 128 characters, `@` and a server id of 16 hex) with
/// a margin; NIP-44 then pads nothing, and every event's content has one
/// length.
pub const PLAIN_LEN: usize = 256;
/// The epoch of the focus keys: the focus has its own, and moves it on by
/// no word yet (messenger-wire.md §8). Here, not in the runtime, so that a
/// phone's push handler makes the same keys.
pub const FOCUS_EPOCH: u32 = 0;

type HmacSha256 = Hmac<Sha256>;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("focus: {what}"))
}

/// The two keys of the focus of one epoch, the same on every device of
/// the user: the one that signs and the one that seals. Neither is the
/// user's key, the presence key or the other.
pub struct FocusKeys {
    signing: Keys,
    content: ConversationKey,
}

impl FocusKeys {
    /// The keys of `epoch`, from the user's secret alone, which is why a
    /// phone's push handler can make them from its `KeyBundle`.
    ///
    /// The focus key is HMAC-SHA256 with key [`KEY_CONTEXT`] of the 32
    /// secret bytes followed by the epoch as four bytes little endian,
    /// with the counter byte of the presence key should the 32 bytes not
    /// be a scalar (`key::derive`, with [`KEY_CONTEXT`] for its context).
    /// The content key is the same HMAC with key [`CONTENT_CONTEXT`],
    /// taken whole as the conversation key of NIP-44 v2: it is a key of a
    /// MAC and a cipher, not of the curve, so there is nothing a counter
    /// would mend.
    ///
    /// The epoch is the focus's own, not the presence's: the runtime
    /// starts at 0 and moves on by a word of its own, should it ever need
    /// a key the relays have not seen.
    pub fn derive(keys: &Keys, epoch: u32) -> Self {
        let signing = crate::key::derive_with(KEY_CONTEXT, keys, epoch);
        let mut mac =
            HmacSha256::new_from_slice(CONTENT_CONTEXT).expect("hmac takes a key of any length");
        mac.update(keys.secret_key().as_secret_bytes());
        mac.update(&epoch.to_le_bytes());
        let content = ConversationKey::new(mac.finalize().into_bytes().into());
        Self { signing, content }
    }

    /// The key the event is signed by: what a subscription names as the
    /// author, and what the runtime writes down as mine.
    pub fn public_key(&self) -> PublicKey {
        self.signing.public_key()
    }

    /// The signing key whole, for the runtime that publishes.
    pub fn signing(&self) -> &Keys {
        &self.signing
    }
}

/// What a focus event says, once open: the chat I am in, or `None` for
/// the list of chats (in the app, in no chat).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focus {
    pub chat: Option<String>,
}

/// A focus event as it stands on the wire, read but not yet open
/// ([`parse`]): whose, when, for how long, and the sealed content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusEvent {
    /// The focus key that signed it: a focus key, not a user.
    pub author: PubKey,
    pub created_at: i64,
    /// The `expiration` tag, when it parses.
    pub expires_at: Option<i64>,
    /// The content, base64 of a NIP-44 v2 payload.
    pub content: String,
}

impl FocusEvent {
    /// Until when the event counts: its expiration, but no further than
    /// `created_at` plus [`FOCUS_TTL_SECS`], and that when it has none.
    pub fn until(&self) -> i64 {
        let promised = self.created_at.saturating_add(FOCUS_TTL_SECS);
        self.expires_at.map_or(promised, |e| e.min(promised))
    }
}

/// The plaintext of `chat`: `{"chat":"<id>"}` or `{"chat":null}`, padded
/// with spaces to [`PLAIN_LEN`]. A chat id that does not fit is refused;
/// the runtime then says `null`, as if in the list.
fn plaintext(chat: Option<&str>) -> Result<Vec<u8>> {
    let mut plain = serde_json::to_vec(&serde_json::json!({ "chat": chat }))?;
    if plain.len() > PLAIN_LEN {
        return Err(invalid("chat id too long"));
    }
    plain.resize(PLAIN_LEN, b' ');
    Ok(plain)
}

/// The content of an event saying `chat`, sealed under the content key
/// with `nonce`: base64 of the NIP-44 v2 payload. The nonce is drawn for
/// each event by [`build`]; this takes one for the vectors, and must never
/// be given the same nonce twice.
pub fn seal_with_nonce(keys: &FocusKeys, chat: Option<&str>, nonce: [u8; 32]) -> Result<String> {
    let payload =
        encrypt_to_bytes_with_nonce(&keys.content, &plaintext(chat)?, nonce).map_err(crypto)?;
    Ok(B64.encode(payload))
}

/// The content of an event saying `chat`, under a fresh nonce.
pub fn seal(keys: &FocusKeys, chat: Option<&str>) -> Result<String> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(crypto)?;
    seal_with_nonce(keys, chat, nonce)
}

/// The chat out of a content [`seal`] made, under the same keys. Refused
/// when it is not NIP-44 v2, when the keys are not its, when it was
/// touched, when its plaintext is not of the one length every event has,
/// or when it does not say a chat.
pub fn unseal(keys: &FocusKeys, content: &str) -> Result<Focus> {
    let payload = B64
        .decode(content)
        .map_err(|_| invalid("content is not base64"))?;
    // The version byte first, as NIP-44 wants (decrypt_v2 says why).
    let plain = decrypt_v2(&keys.content, &payload).map_err(|e| match e {
        OpenError::Decrypt(e) => crypto(e),
        OpenError::Version(_) | OpenError::Empty => invalid("content is not NIP-44 v2"),
    })?;
    if plain.len() != PLAIN_LEN {
        return Err(invalid("content of another length"));
    }
    let value: Value =
        serde_json::from_slice(&plain).map_err(|_| invalid("content is not JSON"))?;
    let chat = match value.get("chat") {
        Some(Value::Null) => None,
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        _ => return Err(invalid("content names no chat")),
    };
    Ok(Focus { chat })
}

/// A focus event made at `now`, signed by the focus key: kind 30315,
/// `d = veydan-focus`, `expiration` = `now` + [`FOCUS_TTL_SECS`], no `p`
/// and no `silent` (addressed to nobody, there is nobody to wake), the
/// chat sealed in the content.
pub fn build(keys: &FocusKeys, chat: Option<&str>, now: i64) -> Result<WireEvent> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(crypto)?;
    build_with_nonce(keys, chat, now, nonce)
}

/// [`build`] with the nonce given, for the vectors.
pub fn build_with_nonce(
    keys: &FocusKeys,
    chat: Option<&str>,
    now: i64,
    nonce: [u8; 32],
) -> Result<WireEvent> {
    let expiration = (now + FOCUS_TTL_SECS).to_string();
    let event = EventBuilder::new(Kind::from(KIND_FOCUS), seal_with_nonce(keys, chat, nonce)?)
        .tag(Tag::parse(["d", FOCUS_D]).map_err(crypto)?)
        .tag(Tag::parse(["expiration", expiration.as_str()]).map_err(crypto)?)
        .custom_created_at(nostr::types::Timestamp::from_secs(now.max(0) as u64))
        .finalize(&keys.signing)
        .map_err(crypto)?;
    Ok(WireEvent {
        id: EventId::parse(&event.id.to_hex())
            .ok_or_else(|| MessengerError::Crypto("bad event id".into()))?,
        json: serde_json::to_value(&event)?,
    })
}

/// A focus event out of the JSON of an event as a relay gives it: `None`
/// for anything else (an event that does not parse or verify, another
/// kind, a beat or a status of another client under this kind). Whose
/// focus key it is, and whether it is mine, is for [`open`].
pub fn parse(json: &Value) -> Option<FocusEvent> {
    let event: Event = serde_json::from_value(json.clone()).ok()?;
    event.verify().ok()?;
    if event.kind.as_u16() != KIND_FOCUS {
        return None;
    }
    let d = event
        .tags
        .iter()
        .find(|t| t.kind() == "d")
        .and_then(|t| t.as_slice().get(1));
    if d.map(String::as_str) != Some(FOCUS_D) {
        return None;
    }
    let expires_at = event
        .tags
        .iter()
        .find(|t| t.kind() == "expiration")
        .and_then(|t| t.as_slice().get(1))
        .and_then(|s| s.parse::<i64>().ok());
    Some(FocusEvent {
        author: PubKey::parse(&event.pubkey.to_hex())?,
        created_at: event.created_at.as_secs() as i64,
        expires_at,
        content: event.content,
    })
}

/// The chat a focus event of my keys says. Refused when the event is not
/// signed by my focus key (a filter by author should let no other
/// through, but a relay is not trusted to), or when the content does not
/// open ([`unseal`]).
pub fn open(keys: &FocusKeys, event: &FocusEvent) -> Result<Focus> {
    if event.author.as_hex() != keys.public_key().to_hex() {
        return Err(invalid("not signed by my focus key"));
    }
    unseal(keys, &event.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Keys {
        Keys::parse(&"01".repeat(32)).unwrap()
    }

    fn event_of(wire: &WireEvent) -> Event {
        serde_json::from_value(wire.json.clone()).unwrap()
    }

    fn tags_of(event: &Event) -> Vec<Vec<String>> {
        event.tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    #[test]
    fn the_keys_are_the_same_every_time_and_another_for_each_epoch_and_user() {
        let me = Keys::generate();
        let a = FocusKeys::derive(&me, 0);
        let again = FocusKeys::derive(&me, 0);
        assert_eq!(a.public_key(), again.public_key());
        assert_eq!(a.content.as_bytes(), again.content.as_bytes());
        let b = FocusKeys::derive(&me, 1);
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.content.as_bytes(), b.content.as_bytes());
        let other = FocusKeys::derive(&Keys::generate(), 0);
        assert_ne!(
            a.public_key(),
            other.public_key(),
            "another user, another key"
        );
        assert_ne!(a.content.as_bytes(), other.content.as_bytes());
    }

    #[test]
    fn neither_key_is_the_users_the_presences_or_the_other() {
        let me = Keys::generate();
        for epoch in [0, 1, 2, 1000] {
            let f = FocusKeys::derive(&me, epoch);
            assert_ne!(f.public_key(), me.public_key());
            assert_ne!(
                f.signing.secret_key().to_secret_hex(),
                me.secret_key().to_secret_hex()
            );
            let presence = crate::key::derive(&me, epoch);
            assert_ne!(
                f.public_key(),
                presence.public_key(),
                "the presence key is told to contacts"
            );
            assert_ne!(
                f.content.as_bytes(),
                f.signing.secret_key().as_secret_bytes(),
                "the key that signs seals nothing"
            );
            assert_ne!(
                f.content.as_bytes(),
                presence.secret_key().as_secret_bytes()
            );
        }
    }

    /// Computed outside Rust, with OpenSSL 3.5.7:
    ///
    /// ```sh
    /// S=$(printf '\\x01%.0s' $(seq 32))
    /// printf "$S\x00\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-focus-v1
    /// printf "$S\x01\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-focus-v1
    /// printf "$S\x00\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-focus-content-v1
    /// printf "$S\x01\x00\x00\x00" | openssl dgst -sha256 -mac HMAC -macopt key:veydan-focus-content-v1
    /// ```
    ///
    /// that is, HMAC-SHA256 keyed by the ASCII of the context over 32
    /// bytes 0x01 and the epoch as four bytes little endian. Both signing
    /// keys are below the order of the curve, so no counter byte is added.
    #[test]
    fn the_way_the_keys_are_made_is_fixed() {
        let f0 = FocusKeys::derive(&me(), 0);
        assert_eq!(
            f0.signing.secret_key().to_secret_hex(),
            "7d2d58cdc717bd4dd2b125e2660b7cb57fe8a7f42537e1754329e3f97a578cbc"
        );
        assert_eq!(
            hex::encode(f0.content.as_bytes()),
            "71331c2d5a04e312e5c2b0c85224836eee70ab40f7988d48a3dcc8428ae87cb3"
        );
        let f1 = FocusKeys::derive(&me(), 1);
        assert_eq!(
            f1.signing.secret_key().to_secret_hex(),
            "289c0a3ba78af1922ef4e9c04b4530d025dfcffb46aa51540332e8009f5ddd74"
        );
        assert_eq!(
            hex::encode(f1.content.as_bytes()),
            "0804b7b1f9eeb3e684fd60bd5d78ef90e640ca2a8dc99942c28e09d874cfb877"
        );
    }

    #[test]
    fn an_event_is_signed_by_the_focus_key_and_shaped_as_a_beat() {
        let f = FocusKeys::derive(&Keys::generate(), 0);
        let wire = build(&f, Some("dm:abc"), 1_759_700_000).unwrap();
        let event = event_of(&wire);
        event.verify().unwrap();
        assert_eq!(event.id.to_hex(), wire.id.as_hex());
        assert_eq!(event.pubkey, f.public_key());
        assert_eq!(event.kind.as_u16(), 30315);
        assert_eq!(event.created_at.as_secs(), 1_759_700_000);
        assert_eq!(
            tags_of(&event),
            vec![
                vec!["d".to_string(), "veydan-focus".into()],
                vec!["expiration".into(), "1759700080".into()]
            ],
            "no p, no silent"
        );
        assert!(!event.content.is_empty());
    }

    #[test]
    fn the_content_has_one_length_whatever_the_chat() {
        let f = FocusKeys::derive(&Keys::generate(), 0);
        let chats = [
            None,
            Some("dm:abc".to_string()),
            Some(format!("dm:{}", "ab".repeat(32))),
            Some(format!("dm:{}@0123456789abcdef", "ab".repeat(32))),
            Some(format!("group:{}@0123456789abcdef", "g".repeat(128))),
            Some(format!("group:{}", "ы".repeat(100))),
        ];
        let lengths: Vec<usize> = chats
            .iter()
            .map(|c| {
                event_of(&build(&f, c.as_deref(), 1_759_700_000).unwrap())
                    .content
                    .len()
            })
            .collect();
        assert!(lengths.iter().all(|l| *l == lengths[0]), "{lengths:?}");
        // Version, nonce, the two-byte length, the plaintext as is, the MAC.
        assert_eq!(
            B64.decode(seal(&f, None).unwrap()).unwrap().len(),
            1 + 32 + 2 + PLAIN_LEN + 32
        );
    }

    #[test]
    fn a_chat_id_that_does_not_fit_is_refused() {
        let f = FocusKeys::derive(&Keys::generate(), 0);
        let longest = "x".repeat(PLAIN_LEN - r#"{"chat":""}"#.len());
        build(&f, Some(&longest), 1_759_700_000).unwrap();
        let err = build(&f, Some(&format!("{longest}x")), 1_759_700_000).unwrap_err();
        assert!(
            matches!(err, MessengerError::Invalid(ref m) if m.contains("too long")),
            "{err}"
        );
    }

    #[test]
    fn what_one_device_builds_another_of_mine_reads_and_opens() {
        let me = Keys::generate();
        let computer = FocusKeys::derive(&me, 0);
        let phone = FocusKeys::derive(&me, 0);
        for chat in [
            Some(format!("dm:{}", "ab".repeat(32))),
            Some("group:g1".to_string()),
            None,
        ] {
            let wire = build(&computer, chat.as_deref(), 1_759_700_000).unwrap();
            let read = parse(&wire.json).expect("a focus event");
            assert_eq!(read.author.as_hex(), computer.public_key().to_hex());
            assert_eq!(
                (read.created_at, read.expires_at),
                (1_759_700_000, Some(1_759_700_080))
            );
            assert_eq!(read.until(), 1_759_700_080);
            assert_eq!(open(&phone, &read).unwrap(), Focus { chat: chat.clone() });
        }
    }

    #[test]
    fn an_event_counts_no_longer_than_the_ttl_whatever_its_tag_says() {
        let at = |expires_at| FocusEvent {
            author: PubKey::parse(&"aa".repeat(32)).unwrap(),
            created_at: 1_000,
            expires_at,
            content: String::new(),
        };
        assert_eq!(at(Some(1_080)).until(), 1_080);
        assert_eq!(at(Some(1_050)).until(), 1_050, "a shorter promise is kept");
        assert_eq!(at(Some(9_000)).until(), 1_080, "a longer one is cut");
        assert_eq!(at(None).until(), 1_080, "none is read as ours");
    }

    #[test]
    fn nobody_else_opens_it_and_a_touched_one_does_not_open() {
        let me = Keys::generate();
        let mine = FocusKeys::derive(&me, 0);
        let wire = build(&mine, Some("dm:abc"), 1_759_700_000).unwrap();
        let read = parse(&wire.json).unwrap();

        let stranger = FocusKeys::derive(&Keys::generate(), 0);
        assert!(
            matches!(open(&stranger, &read), Err(MessengerError::Invalid(_))),
            "another user's keys: not his key"
        );
        assert!(
            matches!(
                unseal(&stranger, &read.content),
                Err(MessengerError::Crypto(_))
            ),
            "the MAC does not verify"
        );
        let next = FocusKeys::derive(&me, 1);
        assert!(
            matches!(open(&next, &read), Err(MessengerError::Invalid(_))),
            "another epoch is another key"
        );

        let mut touched = read.clone();
        let mut bytes = B64.decode(&touched.content).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        touched.content = B64.encode(bytes);
        assert!(matches!(
            open(&mine, &touched),
            Err(MessengerError::Crypto(_))
        ));
        touched.content = "not base64!".into();
        assert!(matches!(
            open(&mine, &touched),
            Err(MessengerError::Invalid(_))
        ));
    }

    #[test]
    fn a_content_of_another_nip44_version_does_not_open() {
        let me = Keys::generate();
        let mine = FocusKeys::derive(&me, 0);
        let content = seal(&mine, Some("dm:abc")).unwrap();
        let payload = B64.decode(&content).unwrap();
        assert_eq!(payload[0], 2, "the seal is v2");
        for version in [0u8, 1, 3, 255] {
            // The MAC covers nonce and ciphertext only, so the payload
            // would still decrypt: the version must be refused on its own.
            let mut other = payload.clone();
            other[0] = version;
            let other = B64.encode(other);
            assert!(
                matches!(unseal(&mine, &other), Err(MessengerError::Invalid(ref m)) if m.contains("NIP-44")),
                "version {version}"
            );
            // Signed by my own focus key over the changed content: parsed
            // as mine, still not opened.
            let signed = EventBuilder::new(Kind::from(KIND_FOCUS), other)
                .tag(Tag::parse(["d", FOCUS_D]).unwrap())
                .finalize(&mine.signing)
                .unwrap();
            let read = parse(&serde_json::to_value(&signed).unwrap()).expect("mine, well signed");
            assert!(
                matches!(open(&mine, &read), Err(MessengerError::Invalid(ref m)) if m.contains("NIP-44")),
                "version {version}, signed"
            );
        }
        assert_eq!(
            unseal(&mine, &content).unwrap().chat.as_deref(),
            Some("dm:abc")
        );
        assert!(
            matches!(unseal(&mine, ""), Err(MessengerError::Invalid(_))),
            "an empty content"
        );
    }

    #[test]
    fn a_content_of_another_making_does_not_open() {
        let mine = FocusKeys::derive(&Keys::generate(), 0);
        let under_my_key = |plain: &[u8]| {
            B64.encode(encrypt_to_bytes_with_nonce(&mine.content, plain, [7u8; 32]).unwrap())
        };
        // Right JSON, not padded: a length that tells the chat apart.
        let short = under_my_key(br#"{"chat":"dm:abc"}"#);
        assert!(
            matches!(unseal(&mine, &short), Err(MessengerError::Invalid(ref m)) if m.contains("length"))
        );
        let pad = |json: &str| {
            let mut v = json.as_bytes().to_vec();
            v.resize(PLAIN_LEN, b' ');
            v
        };
        assert!(
            matches!(unseal(&mine, &under_my_key(&pad("not json"))), Err(MessengerError::Invalid(ref m)) if m.contains("JSON"))
        );
        for json in [
            r#"{}"#,
            r#"{"chat":5}"#,
            r#"{"chat":""}"#,
            r#"{"chat":["dm:abc"]}"#,
            r#"[]"#,
        ] {
            assert!(
                matches!(unseal(&mine, &under_my_key(&pad(json))), Err(MessengerError::Invalid(ref m)) if m.contains("no chat")),
                "{json}"
            );
        }
        // A field we do not know yet is passed over, as everywhere on the wire.
        assert_eq!(
            unseal(&mine, &under_my_key(&pad(r#"{"chat":"dm:abc","since":5}"#))).unwrap(),
            Focus {
                chat: Some("dm:abc".into())
            }
        );
    }

    #[test]
    fn a_beat_another_kind_or_a_bad_signature_is_no_focus_event() {
        let k = Keys::generate();
        let beat = crate::heartbeat::build(&crate::key::derive(&k, 0), 1_759_700_000).unwrap();
        assert_eq!(parse(&beat.json), None, "a beat of presence");
        let status = EventBuilder::new(Kind::from(KIND_FOCUS), "x")
            .tag(Tag::parse(["d", "music"]).unwrap())
            .finalize(&k)
            .unwrap();
        assert_eq!(
            parse(&serde_json::to_value(&status).unwrap()),
            None,
            "a status of another client"
        );
        let no_d = EventBuilder::new(Kind::from(KIND_FOCUS), "x")
            .finalize(&k)
            .unwrap();
        assert_eq!(parse(&serde_json::to_value(&no_d).unwrap()), None);
        let other_kind = EventBuilder::new(Kind::from(30078u16), "x")
            .tag(Tag::parse(["d", FOCUS_D]).unwrap())
            .finalize(&k)
            .unwrap();
        assert_eq!(parse(&serde_json::to_value(&other_kind).unwrap()), None);

        let f = FocusKeys::derive(&k, 0);
        let wire = build(&f, Some("dm:abc"), 1_759_700_000).unwrap();
        let mut forged = wire.json.clone();
        forged["content"] = Value::String(seal(&f, None).unwrap());
        assert_eq!(parse(&forged), None, "the signature no longer holds");
        assert_eq!(parse(&Value::Null), None);

        let mut no_expiration = EventBuilder::new(Kind::from(KIND_FOCUS), seal(&f, None).unwrap())
            .tag(Tag::parse(["d", FOCUS_D]).unwrap())
            .finalize(&f.signing)
            .unwrap();
        let read = parse(&serde_json::to_value(&no_expiration).unwrap())
            .expect("ours, without an expiration");
        assert_eq!(read.expires_at, None);
        assert_eq!(read.until(), read.created_at + FOCUS_TTL_SECS);
        no_expiration.content.clear();
        assert_eq!(
            parse(&serde_json::to_value(&no_expiration).unwrap()),
            None,
            "touched after signing"
        );
    }

    /// Computed outside Rust, in Python with `cryptography` 43 (the
    /// script is in internal/messenger-wire.md, «Фокус»): the content key
    /// of `the_way_the_keys_are_made_is_fixed`, the message keys by
    /// HKDF-expand with the nonce as info, the plaintext `{"chat":"dm:ab…ab"}`
    /// padded with spaces to 256 bytes behind its two-byte length, ChaCha20,
    /// HMAC-SHA256 over nonce and ciphertext; the id as NIP-01 serializes
    /// the event, with the x-only public key of the focus key.
    #[test]
    fn the_event_is_fixed() {
        let f = FocusKeys::derive(&me(), 0);
        assert_eq!(
            f.public_key().to_hex(),
            "8e69bd12171d1f5ded6cec16e657a956ed8dfca3084afbcbc7cee3fc5d91a8b6"
        );
        let chat = format!("dm:{}", "ab".repeat(32));
        let wire = build_with_nonce(&f, Some(&chat), 1_759_700_000, [0x11; 32]).unwrap();
        let event = event_of(&wire);
        assert_eq!(event.content, "AhERERERERERERERERERERERERERERERERERERERERERXWcKaxJSOgNXCnhF6uL2wasqELov+BZVHDHc1YkwCpepkEGvyR6sGobSvZXtTaxL43EnhBjceAEY+nZEKtKlHzAkd9jQjvpYnunMKiXrxcQeyWSttz8LuvkLBL9OFYjORxTwUtdXWDaiFCE3yav/GT7N96cBdq3sbTCj4xtTg/ijutg5SfXgty2tRpIqnlw2ELjek2c1C968CrbbF8Frvb4Js3MaIRcJKkf7CTnfakmJOJyqh1eoeCxQaUJTQ2dAMBdMVYfAEHhq3VGC2QcyAeOEvPEBrxcYL2I8E8x0kc73KpwEl4tMepdEFNkMLgvZSAe3WtPoPNxv6GW9LbEl0HYlpJmBscu451qN+rSolCSAM248tp431btBoB0IlPxVzCw=");
        assert_eq!(
            wire.id.as_hex(),
            "7bc9765cbb21449a0712ede8bdfde410ebdc8d2a3893649612d02c3b1f4abdf2"
        );
        assert_eq!(
            open(&f, &parse(&wire.json).unwrap())
                .unwrap()
                .chat
                .as_deref(),
            Some(chat.as_str())
        );

        let list = build_with_nonce(&f, None, 1_759_700_000, [0x11; 32]).unwrap();
        assert_eq!(event_of(&list).content, "AhERERERERERERERERERERERERERERERERERERERERERXWcKaxJSOgNXCjRU67Tqg+poUfhuulcXXXOdl8hyS9Xo0gDtiFztWMeQ/NesD+0JojNmxlmeOUNZuDcGa5DkXXFmNpqRzLsa36uNaGSpx5keyWSttz8LuvkLBL9OFYjORxTwUtdXWDaiFCE3yav/GT7N96cBdq3sbTCj4xtTg/ijutg5SfXgty2tRpIqnlw2ELjek2c1C968CrbbF8Frvb4Js3MaIRcJKkf7CTnfakmJOJyqh1eoeCxQaUJTQ2dAMBdMVYfAEHhq3VGC2QcyAeOEvPEBrxcYL2I8E8x0kc73KpwEl4tMepdEFNkMLgvZSAe3WtPoPNxv6GW9LbEl0HYl35NFXDjj+n9cvTZRWoelVk0Pjj9be+aDdenPqGRTcZE=");
        assert_eq!(
            list.id.as_hex(),
            "0874e5896d5882e99cef0b55d246492a4e81ade055fb53c3dfc525b6413863e2"
        );
    }

    #[test]
    fn the_wire_constants_are_fixed() {
        assert_eq!((KIND_FOCUS, FOCUS_D), (30315, "veydan-focus"));
        assert_eq!((FOCUS_TTL_SECS, PLAIN_LEN), (80, 256));
        assert_eq!(FOCUS_TTL_SECS, crate::heartbeat::ONLINE_TTL_SECS);
        assert_eq!(
            (KEY_CONTEXT, CONTENT_CONTEXT),
            (
                b"veydan-focus-v1".as_slice(),
                b"veydan-focus-content-v1".as_slice()
            )
        );
    }
}

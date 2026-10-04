// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What is written on the outside of an event for the sake of a push server.
//!
//! A push server sees events as a relay sees them: sealed. It cannot tell a
//! message from a service event, or a user's own message from somebody
//! else's, or an event of a group from one that only names the group, and
//! would wake the phone for all of them. Three tags on the outside tell it
//! what it needs, and no more.
//!
//! **`silent`** marks what is not worth waking anybody for: the copy of a
//! message kept for the author's other devices, the signals of the
//! protocol (accept, block, edit, delete, keys, operations of a group).
//! A relay learns from it that the event is not a message. It does not learn
//! which of the things above it is.
//!
//! **`vp`** is the mark of the author on a group event. A group event is
//! signed by a key made for that one event, so that nobody outside the
//! group can tell who wrote it. The mark can be recognized only by the
//! holder of the author's mark key, which the author gives to the push
//! server they registered at. To everybody else the mark is a random
//! string, and every group event carries one. What the push server learns:
//! which events of a group were written by its user.
//!
//! **`gp`** is the mark of the group on a group event. The id of a group is
//! no secret: it stands in the open on every event, and anybody can sign an
//! event that names it. The mark can be made only by a holder of the
//! group's key. Members give their push server a key made from the group's
//! key, which opens nothing; with it the server tells an event written by
//! somebody in the group from one written by a stranger, and wakes nobody
//! for the stranger's. What the push server learns: that an event came from
//! a holder of a key its user holds too.
//!
//! The mark is made from the key the group has now. A phone that slept
//! through a change of the key has told its server only the key before, so
//! for a while after a change events carry a second mark, of the key that
//! was replaced ([`GROUP_GRACE_SECS`]).

use hmac::{Hmac, KeyInit, Mac};
use messenger_core::{MessengerError, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use sha2::Sha256;

/// `["silent", "1"]`
pub const SILENT: &str = "silent";
/// `["vp", <mark>]`
pub const AUTHOR: &str = "vp";
/// `["gp", <mark>]`
pub const GROUP: &str = "gp";
/// How long after a change of a group's key events also carry the mark of
/// the key before.
pub const GROUP_GRACE_SECS: i64 = 7 * 24 * 3600;

const CONTEXT: &[u8] = b"veydan-push-author-v1";
const GROUP_CONTEXT: &[u8] = b"veydan-push-group-v1";
/// Hex characters of the mark: 8 bytes.
const LEN: usize = 16;

type HmacSha256 = Hmac<Sha256>;

fn mac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac takes a key of any length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

pub fn silent_tag() -> Result<Tag> {
    Tag::parse([SILENT, "1"]).map_err(crypto)
}

/// The key of the marks, made from the user's key. It cannot be turned back
/// into the user's key, and it signs nothing.
pub fn author_key(keys: &Keys) -> String {
    hex::encode(mac(CONTEXT, keys.secret_key().as_secret_bytes()))
}

/// A mark made with `key_hex` for an event signed by `event_pubkey_hex`.
fn mark(key_hex: &str, event_pubkey_hex: &str) -> Option<String> {
    let key = hex::decode(key_hex).ok()?;
    let pubkey = hex::decode(event_pubkey_hex).ok()?;
    if key.len() != 32 || pubkey.len() != 32 {
        return None;
    }
    let mut out = hex::encode(mac(&key, &pubkey));
    out.truncate(LEN);
    Some(out)
}

/// The mark for an event signed by `event_pubkey` (hex), the key made for
/// that one event.
pub fn author_mark(author_key_hex: &str, event_pubkey_hex: &str) -> Option<String> {
    mark(author_key_hex, event_pubkey_hex)
}

/// The key of a group's marks, made from a key of the group. It cannot be
/// turned back into the group's key, and it opens nothing.
pub fn group_push_key(group_key: &[u8; 32]) -> String {
    hex::encode(mac(group_key, GROUP_CONTEXT))
}

/// The group's mark for an event signed by `event_pubkey` (hex).
pub fn group_mark(push_key_hex: &str, event_pubkey_hex: &str) -> Option<String> {
    mark(push_key_hex, event_pubkey_hex)
}

/// The tag a holder of `group_key` puts on an event that `event_pubkey` signs.
pub fn group_tag(group_key: &[u8; 32], event_pubkey: &PublicKey) -> Result<Tag> {
    let mark = group_mark(&group_push_key(group_key), &event_pubkey.to_hex())
        .ok_or_else(|| MessengerError::Crypto("no mark for this key".into()))?;
    Tag::parse([GROUP, mark.as_str()]).map_err(crypto)
}

/// The tag `author` puts on an event that `event_pubkey` signs.
pub fn author_tag(author: &Keys, event_pubkey: &PublicKey) -> Result<Tag> {
    let mark = author_mark(&author_key(author), &event_pubkey.to_hex())
        .ok_or_else(|| MessengerError::Crypto("no mark for this key".into()))?;
    Tag::parse([AUTHOR, mark.as_str()]).map_err(crypto)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_key_is_the_same_every_time_and_is_not_the_users_key() {
        let keys = Keys::generate();
        let key = author_key(&keys);
        assert_eq!(key, author_key(&keys));
        assert_eq!(key.len(), 64);
        assert_ne!(key, keys.secret_key().to_secret_hex());
        assert_ne!(key, keys.public_key().to_hex());
        assert_ne!(key, author_key(&Keys::generate()));
    }

    #[test]
    fn the_holder_of_the_key_recognizes_the_mark_and_nobody_else_does() {
        let (me, other) = (Keys::generate(), Keys::generate());
        let event = Keys::generate().public_key().to_hex();

        let mine = author_mark(&author_key(&me), &event).unwrap();
        assert_eq!(mine.len(), 16);
        assert_eq!(Some(mine.clone()), author_mark(&author_key(&me), &event));
        assert_ne!(Some(mine), author_mark(&author_key(&other), &event));
    }

    #[test]
    fn marks_of_one_author_do_not_look_alike() {
        let me = author_key(&Keys::generate());
        let a = author_mark(&me, &Keys::generate().public_key().to_hex());
        let b = author_mark(&me, &Keys::generate().public_key().to_hex());
        assert_ne!(a, b);
    }

    #[test]
    fn what_is_not_a_key_gives_no_mark() {
        assert_eq!(author_mark("zz", &"11".repeat(32)), None);
        assert_eq!(author_mark(&"11".repeat(32), "short"), None);
        assert_eq!(author_mark(&"11".repeat(31), &"11".repeat(32)), None);
    }

    /// The push server computes the same mark in its own code, and has this
    /// value in its tests too: if either side changes the way, one of them
    /// fails. The values were computed apart from this code, with openssl.
    #[test]
    fn the_way_the_mark_is_made_is_fixed() {
        let keys = Keys::parse(&"01".repeat(32)).unwrap();
        let key = author_key(&keys);
        assert_eq!(key, "48d060b886c7db36283c9e657795adb5f2dc5c7886eadec18610b91b11b9c8e8");
        assert_eq!(author_mark(&key, &"02".repeat(32)).unwrap(), "20526ea0e3c8f745");
    }

    #[test]
    fn the_groups_mark_is_made_by_holders_of_its_key_only() {
        let (key, other) = ([7u8; 32], [8u8; 32]);
        let event = Keys::generate().public_key();

        let push_key = group_push_key(&key);
        assert_eq!(push_key.len(), 64);
        assert_eq!(push_key, group_push_key(&key));
        assert_ne!(push_key, hex::encode(key), "what the server is given is not the group's key");
        assert_ne!(push_key, group_push_key(&other));

        let mark = group_mark(&push_key, &event.to_hex()).unwrap();
        assert_eq!(mark.len(), 16);
        assert_ne!(Some(mark.clone()), group_mark(&group_push_key(&other), &event.to_hex()));
        assert_ne!(Some(mark.clone()), group_mark(&push_key, &Keys::generate().public_key().to_hex()), "a mark is for one event");
        assert_eq!(group_tag(&key, &event).unwrap().as_slice(), [GROUP.to_string(), mark]);
        assert_eq!(group_mark("zz", &event.to_hex()), None);
    }

    /// As with the author's mark: the push server has these values in its
    /// tests too. Computed apart from this code, with openssl.
    #[test]
    fn the_way_the_groups_mark_is_made_is_fixed() {
        let push_key = group_push_key(&[3u8; 32]);
        assert_eq!(push_key, "c960ddf616adb9a57ac7d0eb04da5e25cf3d139c95d5828addb8f1f0ed50da28");
        assert_eq!(group_mark(&push_key, &"02".repeat(32)).unwrap(), "86a8b9acabf6941d");
    }

    #[test]
    fn the_tag_carries_the_mark() {
        let (me, event) = (Keys::generate(), Keys::generate());
        let tag = author_tag(&me, &event.public_key()).unwrap();
        let expected = author_mark(&author_key(&me), &event.public_key().to_hex()).unwrap();
        assert_eq!(tag.as_slice(), [AUTHOR.to_string(), expected]);
        assert_eq!(silent_tag().unwrap().as_slice(), ["silent".to_string(), "1".to_string()]);
    }
}

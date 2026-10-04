// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What groups put on the wire (docs/messenger-wire.md §4).
//!
//! **On the group relay**: events of kind 9 signed by a throw-away key,
//! tagged with the group (`h`) and the key that opens them (`k`). The relay
//! learns which group an event belongs to and nothing else: not who wrote
//! it, not what it is. Inside, sealed with the group key, is a payload
//! holding an event signed by the real author: a message or an operation.
//!
//! **In direct messages** (NIP-17, like any DM): invitations, requests to
//! join, the welcome with the log and the keys, key deliveries.

use crate::keys::GroupKey;
use crate::op::{KeyId, Op};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::outbound::WireEvent;
use messenger_core::{EventId, MessengerError, PubKey, Result};
use messenger_dm::pushtags;
use nostr::key::Keys;
use nostr::nips::nip44;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};

pub const KIND_GROUP_EVENT: u16 = 9;
/// Inner event kinds. They never reach a relay in the open.
pub const KIND_INNER_MESSAGE: u16 = 9;
pub const KIND_INNER_OP: u16 = 39100;

pub const PAYLOAD_VERSION: u32 = 1;
/// Keys one chain event may carry.
pub const MAX_CHAIN_KEYS: usize = 64;
pub const INVITE_TTL_SECS: i64 = 7 * 24 * 3600;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("group wire: {what}"))
}

/// A secret for one member, readable by them only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretEnvelope {
    pub to: PubKey,
    /// NIP-44 from the author of the operation to `to`.
    pub ct: String,
}

/// The sealed content of a group event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Payload {
    pub v: u32,
    /// `msg` | `op` | `chain`
    pub t: String,
    /// Signed by the real author. Absent in a `chain`.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub event: serde_json::Value,
    /// Operations that bring a key or a link secret: one envelope per
    /// member who is to have it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub envelopes: Vec<SecretEnvelope>,
    /// `chain`: the keys that were current before this one (base64), for
    /// those who came by a newer link. Nothing here is trusted: a key
    /// only opens events, and what is inside them is signed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

/// What a payload turned out to be, authors verified.
#[derive(Clone, Debug)]
pub enum Opened {
    Message(InnerMessage),
    Op { op: Op, signed: serde_json::Value, envelopes: Vec<SecretEnvelope> },
    Chain(Vec<GroupKey>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InnerMessage {
    pub id: EventId,
    pub author: PubKey,
    pub created_at: i64,
    /// Application envelope (`{"v","t",…}`), as in DMs.
    pub content: String,
    pub reply_to: Option<EventId>,
}

fn wire(event: &Event) -> Result<WireEvent> {
    Ok(WireEvent {
        id: EventId::parse(&event.id.to_hex()).ok_or_else(|| invalid("event id"))?,
        json: serde_json::to_value(event)?,
    })
}

fn pk(p: &PublicKey) -> PubKey {
    PubKey::parse(&p.to_hex()).expect("nostr public keys are 64 hex chars")
}

/// A message signed by its author, ready to be sealed.
pub fn sign_message(keys: &Keys, group_id: &str, content: &str, created_at: i64, reply_to: Option<&str>) -> Result<Event> {
    let mut b = EventBuilder::new(Kind::from(KIND_INNER_MESSAGE), content)
        .tag(Tag::parse(["h", group_id]).map_err(crypto)?)
        .custom_created_at(Timestamp::from_secs(created_at.max(0) as u64));
    if let Some(id) = reply_to {
        b = b.tag(Tag::parse(["e", id]).map_err(crypto)?);
    }
    b.finalize(keys).map_err(crypto)
}

/// An operation signed by its author. The signature covers the canonical
/// form, so the id of the operation is covered too.
pub fn sign_op(keys: &Keys, op: &Op) -> Result<Event> {
    if op.author.as_hex() != keys.public_key().to_hex() {
        return Err(invalid("an operation is signed by its author"));
    }
    EventBuilder::new(Kind::from(KIND_INNER_OP), op.canonical())
        .tag(Tag::parse(["h", op.group_id.as_str()]).map_err(crypto)?)
        .custom_created_at(Timestamp::from_secs(op.created_at.max(0) as u64))
        .finalize(keys)
        .map_err(crypto)
}

/// Check a signed operation event and return the operation.
pub fn verify_op(signed: &serde_json::Value, group_id: &str) -> Result<Op> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("operation event"))?;
    event.verify().map_err(|_| invalid("operation signature"))?;
    if event.kind.as_u16() != KIND_INNER_OP {
        return Err(invalid("not an operation"));
    }
    let op: Op = serde_json::from_str(&event.content).map_err(|_| invalid("operation body"))?;
    if op.author.as_hex() != event.pubkey.to_hex() {
        return Err(invalid("operation author is not the signer"));
    }
    if op.group_id != group_id {
        return Err(invalid("operation of another group"));
    }
    // The content must be the canonical form, or ids would differ between devices.
    if op.canonical() != event.content {
        return Err(invalid("operation is not in canonical form"));
    }
    Ok(op)
}

fn verify_message(signed: &serde_json::Value, group_id: &str) -> Result<InnerMessage> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("message event"))?;
    event.verify().map_err(|_| invalid("message signature"))?;
    if event.kind.as_u16() != KIND_INNER_MESSAGE {
        return Err(invalid("not a message"));
    }
    let tag = |k: &str| event.tags.iter().find(|t| t.kind() == k).and_then(|t| t.as_slice().get(1).cloned());
    if tag("h").as_deref() != Some(group_id) {
        return Err(invalid("message of another group"));
    }
    Ok(InnerMessage {
        id: EventId::parse(&event.id.to_hex()).ok_or_else(|| invalid("event id"))?,
        author: pk(&event.pubkey),
        created_at: event.created_at.as_secs() as i64,
        content: event.content.clone(),
        reply_to: tag("e").and_then(|e| EventId::parse(&e)),
    })
}

/// Seal a payload with the group key into an event for the relay, signed
/// by a key that is used once and thrown away.
///
/// `author` signs nothing here. It makes the mark by which the author's push
/// server tells the author's events from the others; `quiet` says the event
/// is not worth a push to anybody. The key makes the mark of the group, by
/// which a push server tells an event of somebody in the group from one
/// that only names it; `grace` is the key replaced not long ago, whose mark
/// is for the phones that have not told their server the new one yet;
/// what is `quiet` wakes no phone and is sealed without it. See
/// `messenger_dm::pushtags`.
pub fn seal(
    group_id: &str,
    key: &GroupKey,
    grace: Option<&GroupKey>,
    payload: &Payload,
    created_at: i64,
    author: &Keys,
    quiet: bool,
) -> Result<WireEvent> {
    let sealed = key.seal(serde_json::to_string(payload)?.as_bytes())?;
    let once = Keys::generate();
    let mut builder = EventBuilder::new(Kind::from(KIND_GROUP_EVENT), sealed)
        .tag(Tag::parse(["h", group_id]).map_err(crypto)?)
        .tag(Tag::parse(["k", key.id().0.as_str()]).map_err(crypto)?)
        .tag(pushtags::author_tag(author, &once.public_key())?)
        .tag(pushtags::group_tag(key.as_bytes(), &once.public_key())?)
        .custom_created_at(Timestamp::from_secs(created_at.max(0) as u64));
    if let Some(before) = grace.filter(|before| *before != key) {
        builder = builder.tag(pushtags::group_tag(before.as_bytes(), &once.public_key())?);
    }
    if quiet {
        builder = builder.tag(pushtags::silent_tag()?);
    }
    wire(&builder.finalize(&once).map_err(crypto)?)
}

pub fn seal_message(
    group_id: &str,
    key: &GroupKey,
    grace: Option<&GroupKey>,
    signed: &Event,
    author: &Keys,
) -> Result<WireEvent> {
    let payload = Payload { v: PAYLOAD_VERSION, t: "msg".into(), event: serde_json::to_value(signed)?, envelopes: vec![], keys: vec![] };
    seal(group_id, key, grace, &payload, signed.created_at.as_secs() as i64, author, false)
}

/// Older keys for the holders of a newer one.
pub fn seal_chain(
    group_id: &str,
    key: &GroupKey,
    older: &[GroupKey],
    created_at: i64,
    author: &Keys,
) -> Result<WireEvent> {
    let payload = Payload {
        v: PAYLOAD_VERSION,
        t: "chain".into(),
        event: serde_json::Value::Null,
        envelopes: vec![],
        keys: encode_keys(older),
    };
    seal(group_id, key, None, &payload, created_at, author, true)
}

pub fn seal_op(
    group_id: &str,
    key: &GroupKey,
    signed: &Event,
    envelopes: Vec<SecretEnvelope>,
    author: &Keys,
) -> Result<WireEvent> {
    let payload = Payload { v: PAYLOAD_VERSION, t: "op".into(), event: serde_json::to_value(signed)?, envelopes, keys: vec![] };
    seal(group_id, key, None, &payload, signed.created_at.as_secs() as i64, author, true)
}

/// Open the content of a group event and verify what is inside.
pub fn open(group_id: &str, key: &GroupKey, ciphertext: &str) -> Result<Opened> {
    let plain = key.open(ciphertext)?;
    let payload: Payload = serde_json::from_slice(&plain).map_err(|_| invalid("payload"))?;
    if payload.v == 0 || payload.v > PAYLOAD_VERSION {
        return Err(invalid("payload version"));
    }
    match payload.t.as_str() {
        "msg" => Ok(Opened::Message(verify_message(&payload.event, group_id)?)),
        "op" => Ok(Opened::Op { op: verify_op(&payload.event, group_id)?, signed: payload.event, envelopes: payload.envelopes }),
        "chain" => Ok(Opened::Chain(decode_keys(&payload.keys).into_iter().take(MAX_CHAIN_KEYS).collect())),
        _ => Err(invalid("payload type")),
    }
}

// ─── Secrets for single members ─────────────────────────────────────────────

pub fn envelope_for(keys: &Keys, to: &PubKey, secret: &[u8]) -> Result<SecretEnvelope> {
    let peer = PublicKey::from_hex(to.as_hex()).map_err(crypto)?;
    let ct = nip44::encrypt(keys.secret_key(), &peer, B64.encode(secret), nip44::Version::V2).map_err(crypto)?;
    Ok(SecretEnvelope { to: to.clone(), ct })
}

/// The envelope addressed to me, opened. `from` is the author of the operation.
pub fn open_envelope(keys: &Keys, from: &PubKey, envelopes: &[SecretEnvelope]) -> Result<Option<Vec<u8>>> {
    let me = keys.public_key().to_hex();
    let Some(mine) = envelopes.iter().find(|e| e.to.as_hex() == me) else { return Ok(None) };
    let peer = PublicKey::from_hex(from.as_hex()).map_err(crypto)?;
    let b64 = nip44::decrypt(keys.secret_key(), &peer, &mine.ct).map_err(crypto)?;
    Ok(Some(B64.decode(b64).map_err(|_| invalid("envelope"))?))
}

/// The key an operation announced must be the key the envelope held.
pub fn check_key(announced: &KeyId, bytes: &[u8]) -> Result<GroupKey> {
    let key = GroupKey::from_bytes(bytes)?;
    if &key.id() != announced {
        return Err(invalid("the delivered key is not the announced one"));
    }
    Ok(key)
}

// ─── Direct messages ────────────────────────────────────────────────────────

pub const T_INVITE: &str = "group.invite";
pub const T_INVITE_REPLY: &str = "group.invite_reply";
pub const T_JOIN_REQUEST: &str = "group.join_request";
pub const T_REJECTED: &str = "group.rejected";
pub const T_WELCOME: &str = "group.welcome";
pub const T_KEYS: &str = "group.keys";

/// What a manager sends to someone they want in the group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub invite_id: String,
    pub group_id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub picture: String,
    pub relay: String,
    pub members: u32,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteReply {
    pub invite_id: String,
    pub group_id: String,
    pub accept: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub group_id: String,
    #[serde(default)]
    pub note: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub group_id: String,
}

/// Everything a newcomer needs: the log (signed operations) and the keys
/// they are entitled to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub group_id: String,
    pub relay: String,
    pub ops: Vec<serde_json::Value>,
    /// Base64 key bytes: all of them when the group shows its history,
    /// the current one otherwise.
    pub keys: Vec<String>,
    /// Public groups: the secret of the current link (base64), so that the
    /// newcomer can share the link too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// Keys for a member who lacks them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyDelivery {
    pub group_id: String,
    pub keys: Vec<String>,
}

pub fn encode_keys(keys: &[GroupKey]) -> Vec<String> {
    keys.iter().map(|k| B64.encode(k.as_bytes())).collect()
}

pub fn decode_keys(list: &[String]) -> Vec<GroupKey> {
    list.iter().filter_map(|s| B64.decode(s).ok()).filter_map(|b| GroupKey::from_bytes(&b).ok()).collect()
}

/// `{"v":1,"t":"group.invite", …fields}`: the DM envelope of a group message.
pub fn encode_secret(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub fn decode_secret(s: &str) -> Option<Vec<u8>> {
    B64.decode(s).ok()
}

pub fn dm_envelope<T: Serialize>(t: &str, body: &T) -> Result<String> {
    let mut e = messenger_core::Envelope::new(t);
    if let serde_json::Value::Object(map) = serde_json::to_value(body)? {
        e.fields = map;
    }
    Ok(e.encode())
}

pub fn dm_body<T: for<'de> Deserialize<'de>>(e: &messenger_core::Envelope) -> Result<T> {
    serde_json::from_value(serde_json::Value::Object(e.fields.clone())).map_err(|_| invalid("direct message body"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::OpBody;

    const GID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn me(k: &Keys) -> PubKey {
        pk(&k.public_key())
    }

    #[test]
    fn relay_sees_neither_author_nor_content() {
        let alice = Keys::generate();
        let key = GroupKey::generate().unwrap();
        let signed = sign_message(&alice, GID, r#"{"v":1,"t":"text","text":"secret words"}"#, 1_700_000_000, None).unwrap();
        let a = seal_message(GID, &key, None, &signed, &alice).unwrap();
        let b = seal_message(GID, &key, None, &signed, &alice).unwrap();
        let text = a.json.to_string();
        assert!(!text.contains(&alice.public_key().to_hex()), "the author is not on the outside");
        assert!(!text.contains("secret words"));
        assert!(!text.contains(&pushtags::author_key(&alice)), "neither is the key of the author's marks");
        assert_ne!(a.json["pubkey"], b.json["pubkey"], "a new throw-away signer every time");
        assert_eq!(a.json["kind"], 9);
        let tags = a.json["tags"].as_array().unwrap();
        assert!(tags.iter().any(|t| t[0] == "h" && t[1] == GID));
        assert!(tags.iter().any(|t| t[0] == "k" && t[1] == key.id().0.as_str()));
        let outer: Event = serde_json::from_value(a.json.clone()).unwrap();
        outer.verify().unwrap();

        match open(GID, &key, a.json["content"].as_str().unwrap()).unwrap() {
            Opened::Message(m) => {
                assert_eq!(m.author, me(&alice));
                assert_eq!(m.created_at, 1_700_000_000);
                assert!(m.content.contains("secret words"));
                assert_eq!(m.id.as_hex(), signed.id.to_hex());
            }
            other => panic!("{other:?}"),
        }
        assert!(open(GID, &GroupKey::generate().unwrap(), a.json["content"].as_str().unwrap()).is_err());
        assert!(open(&"bb".repeat(32), &key, a.json["content"].as_str().unwrap()).is_err(), "sealed for another group");
    }

    #[test]
    fn operations_are_bound_to_their_signer() {
        let alice = Keys::generate();
        let mallory = Keys::generate();
        let key = GroupKey::generate().unwrap();
        let op = Op::new(GID, &me(&alice), vec![crate::op::OpId("p".into())], 5, OpBody::RotateKey).with_key(key.id());
        let signed = sign_op(&alice, &op).unwrap();
        assert!(sign_op(&mallory, &op).is_err(), "nobody signs for someone else");
        let json = serde_json::to_value(&signed).unwrap();
        assert_eq!(verify_op(&json, GID).unwrap(), op);
        assert_eq!(verify_op(&json, GID).unwrap().id(), op.id());
        assert!(verify_op(&json, &"bb".repeat(32)).is_err());

        // Mallory signs an operation that names Alice as the author.
        let forged = EventBuilder::new(Kind::from(KIND_INNER_OP), op.canonical()).finalize(&mallory).unwrap();
        assert!(verify_op(&serde_json::to_value(&forged).unwrap(), GID).is_err());
        // A tampered event does not verify.
        let mut tampered = json.clone();
        tampered["content"] = serde_json::Value::String(op.canonical().replace("rotate_key", "disband"));
        assert!(verify_op(&tampered, GID).is_err());
        // Not canonical: same meaning, different bytes.
        let spaced = EventBuilder::new(Kind::from(KIND_INNER_OP), op.canonical().replace("\"v\":1", "\"v\": 1")).finalize(&alice).unwrap();
        assert!(verify_op(&serde_json::to_value(&spaced).unwrap(), GID).is_err());

        let envelope = envelope_for(&alice, &me(&mallory), key.as_bytes()).unwrap();
        let sealed = seal_op(GID, &key, &signed, vec![envelope], &alice).unwrap();
        match open(GID, &key, sealed.json["content"].as_str().unwrap()).unwrap() {
            Opened::Op { op: got, envelopes, .. } => {
                assert_eq!(got, op);
                assert_eq!(envelopes.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    fn outer_tag(w: &WireEvent, name: &str) -> Option<String> {
        w.json["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t[0] == name)
            .map(|t| t[1].as_str().unwrap().to_string())
    }

    #[test]
    fn a_message_is_worth_a_push_and_what_serves_the_group_is_not() {
        let alice = Keys::generate();
        let key = GroupKey::generate().unwrap();
        let signed = sign_message(&alice, GID, r#"{"v":1,"t":"text","text":"hi"}"#, 1_700_000_000, None).unwrap();
        let message = seal_message(GID, &key, None, &signed, &alice).unwrap();
        assert_eq!(outer_tag(&message, "silent"), None);

        let op = Op::new(GID, &me(&alice), vec![crate::op::OpId("p".into())], 5, OpBody::RotateKey).with_key(key.id());
        let sealed_op = seal_op(GID, &key, &sign_op(&alice, &op).unwrap(), vec![], &alice).unwrap();
        assert_eq!(outer_tag(&sealed_op, "silent").as_deref(), Some("1"));

        let chain = seal_chain(GID, &key, &[GroupKey::generate().unwrap()], 1_700_000_000, &alice).unwrap();
        assert_eq!(outer_tag(&chain, "silent").as_deref(), Some("1"));

        // The tags changed nothing inside.
        assert!(matches!(open(GID, &key, message.json["content"].as_str().unwrap()).unwrap(), Opened::Message(_)));
        assert!(matches!(open(GID, &key, sealed_op.json["content"].as_str().unwrap()).unwrap(), Opened::Op { .. }));
    }

    #[test]
    fn the_author_is_recognized_by_the_holder_of_the_authors_mark_key_only() {
        let (alice, bob) = (Keys::generate(), Keys::generate());
        let key = GroupKey::generate().unwrap();
        let signed = sign_message(&alice, GID, r#"{"v":1,"t":"text","text":"hi"}"#, 1_700_000_000, None).unwrap();
        let sealed = seal_message(GID, &key, None, &signed, &alice).unwrap();

        let mark = outer_tag(&sealed, "vp").unwrap();
        let signer = sealed.json["pubkey"].as_str().unwrap();
        assert_eq!(pushtags::author_mark(&pushtags::author_key(&alice), signer).unwrap(), mark);
        assert_ne!(pushtags::author_mark(&pushtags::author_key(&bob), signer).unwrap(), mark);

        // Every event has a mark, and no two are alike: a mark says nothing
        // to the one who cannot check it.
        let again = seal_message(GID, &key, None, &signed, &alice).unwrap();
        assert_ne!(outer_tag(&again, "vp").unwrap(), mark);
    }

    #[test]
    fn the_group_is_recognized_by_the_holders_of_its_key_only() {
        let alice = Keys::generate();
        let (key, before, stranger) = (GroupKey::generate().unwrap(), GroupKey::generate().unwrap(), GroupKey::generate().unwrap());
        let signed = sign_message(&alice, GID, r#"{"v":1,"t":"text","text":"hi"}"#, 1_700_000_000, None).unwrap();
        let marks = |sealed: &WireEvent| -> Vec<String> {
            let tags = sealed.json["tags"].as_array().unwrap();
            tags.iter().filter(|t| t[0] == "gp").map(|t| t[1].as_str().unwrap().to_string()).collect()
        };
        let mark = |key: &GroupKey, sealed: &WireEvent| {
            pushtags::group_mark(&pushtags::group_push_key(key.as_bytes()), sealed.json["pubkey"].as_str().unwrap()).unwrap()
        };

        let sealed = seal_message(GID, &key, None, &signed, &alice).unwrap();
        assert_eq!(marks(&sealed), vec![mark(&key, &sealed)]);
        assert_ne!(marks(&sealed), vec![mark(&stranger, &sealed)], "naming the group is not enough");
        assert!(!sealed.json.to_string().contains(&pushtags::group_push_key(key.as_bytes())), "the key of the marks stays inside");

        // Not long after a change of the key: the key before marks too.
        let sealed = seal_message(GID, &key, Some(&before), &signed, &alice).unwrap();
        assert_eq!(marks(&sealed), vec![mark(&key, &sealed), mark(&before, &sealed)]);
        let sealed = seal_message(GID, &key, Some(&key), &signed, &alice).unwrap();
        assert_eq!(marks(&sealed).len(), 1, "one key, one mark");

        // What serves the group is marked as well: every event of a group looks the same from outside.
        let op = Op::new(GID, &me(&alice), vec![crate::op::OpId("p".into())], 5, OpBody::RotateKey).with_key(key.id());
        let sealed = seal_op(GID, &key, &sign_op(&alice, &op).unwrap(), vec![], &alice).unwrap();
        assert_eq!(marks(&sealed), vec![mark(&key, &sealed)]);
    }

    #[test]
    fn envelopes_open_for_their_addressee_only() {
        let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
        let key = GroupKey::generate().unwrap();
        let list = vec![envelope_for(&alice, &me(&bob), key.as_bytes()).unwrap()];
        let got = open_envelope(&bob, &me(&alice), &list).unwrap().unwrap();
        assert_eq!(check_key(&key.id(), &got).unwrap(), key);
        assert!(open_envelope(&carol, &me(&alice), &list).unwrap().is_none(), "nothing addressed to carol");
        assert!(open_envelope(&bob, &me(&carol), &list).is_err(), "not from the claimed author");
        assert!(check_key(&GroupKey::generate().unwrap().id(), &got).is_err(), "not the announced key");
    }

    #[test]
    fn direct_message_bodies() {
        let inv = Invite {
            invite_id: "i1".into(),
            group_id: GID.into(),
            name: "Team".into(),
            about: String::new(),
            picture: String::new(),
            relay: "wss://r.example".into(),
            members: 4,
            created_at: 10,
            expires_at: 10 + INVITE_TTL_SECS,
        };
        let text = dm_envelope(T_INVITE, &inv).unwrap();
        let e = messenger_core::Envelope::parse(&text).unwrap();
        assert_eq!(e.t, T_INVITE);
        assert_eq!(dm_body::<Invite>(&e).unwrap(), inv);
        assert!(dm_body::<InviteReply>(&e).is_err());
        let keys = vec![GroupKey::generate().unwrap(), GroupKey::generate().unwrap()];
        let mut encoded = encode_keys(&keys);
        encoded.push("garbage".into());
        assert_eq!(decode_keys(&encoded), keys);
    }
}

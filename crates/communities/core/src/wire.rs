// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/wire.rs at 1f2ef31d, then changed for channels.

//! What channels put on the wire (internal/communities-wire.md).
//!
//! **The journal** (kind 1391, §3): operations in the open, each signed by
//! its author, the canonical form as the content. The relay sees the staff,
//! the bans and the settings: the price of a ban that holds without a key.
//!
//! **The content** (kind 1390, §4): events signed by a throw-away key,
//! tagged with the channel (`h`), the scope (`s`), the stream (`c`), the
//! epoch (`n`, `k`) and the target (`e`). Inside, sealed with the key of
//! the stream in that epoch (keydist), is a payload holding an event
//! (kind 1394) signed by the real author, whose `h`, `s`, `c` must be the
//! outer ones.
//!
//! **The card** (kind 30390, §2): a hint for those outside, taken only
//! from the owner by the journal.
//!
//! **Notes** (§7): NIP-59 rumors with a `t`; their bodies and checks are
//! here, their wrapping is the transport's.

use crate::op::{KeyId, Op, OpId};
use crate::state::{check_relays, check_storage, MAX_ABOUT_CHARS, MAX_NAME_CHARS, MAX_PICTURE_CHARS, SCOPE_EVERYONE};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine as _;
use keydist::{ChannelKey, EpochKey, ScopeRef};
use messenger_core::{EventId, MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Sealed content of a stream (§4).
pub const KIND_CONTENT: u16 = 1390;
/// The open journal (§3).
pub const KIND_JOURNAL: u16 = 1391;
/// An epoch of a scope, signed by the keeper (§5).
pub const KIND_EPOCH: u16 = 1392;
/// A snapshot of the readers, signed by the keeper (§8).
pub const KIND_SNAPSHOT: u16 = 1393;
/// The inner event of sealed content, signed by its author: never on a
/// relay in the open. Its own kind: 9 with `h` is NIP-29.
pub const KIND_INNER: u16 = 1394;
/// The card of a channel, a parameterized replaceable event (§2).
pub const KIND_CARD: u16 = 30390;
/// The sign of life of a keeper, parameterized replaceable, signed by the
/// keeper's key (§18, 2026-10-23).
pub const KIND_HEARTBEAT: u16 = 30391;

pub const PAYLOAD_VERSION: u32 = 1;
pub const CARD_VERSION: u32 = 1;
/// The one stream of stage 1.
pub const CHANNEL_FEED: &str = "feed";
/// Clocks may differ this much (seconds), everywhere on the wire.
pub const SKEW_SECS: i64 = 300;
/// How long an invitation and an admission stand.
pub const INVITE_TTL_SECS: i64 = 7 * 24 * 3600;
/// The label of the bytes of a scope (§1).
pub const SCOPE_LABEL: &[u8] = b"veydan.community.scope.v1";
/// An inscription on the card.
pub const MAX_CODE_CHARS: usize = 32;
/// The note of a request.
pub const MAX_REQUEST_NOTE_CHARS: usize = 500;

const NONCE_LEN: usize = 12;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel wire: {what}"))
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn pk(p: &PublicKey) -> Result<PubKey> {
    PubKey::parse(&p.to_hex()).ok_or_else(|| invalid("public key"))
}

fn tag(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find(|t| t.kind() == name).and_then(|t| t.as_slice().get(1).cloned())
}

fn timestamp(at: i64) -> Timestamp {
    Timestamp::from_secs(at.max(0) as u64)
}

// ─── Ids and keys (§1) ──────────────────────────────────────────────────────

/// The 32 bytes keydist knows a scope by: SHA-256 of the label and the
/// scope id (`everyone`, or 32 hex of stage 2).
pub fn scope_bytes(scope_id: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(SCOPE_LABEL);
    h.update(scope_id.as_bytes());
    h.finalize().into()
}

/// The raw 32 bytes of a community id (64 hex).
pub fn community_bytes(community_id: &str) -> Result<[u8; 32]> {
    if !is_hex(community_id, 64) {
        return Err(invalid("community id"));
    }
    let raw = hex::decode(community_id).map_err(|_| invalid("community id"))?;
    raw.try_into().map_err(|_| invalid("community id"))
}

/// The scope of keydist for a scope of a channel.
pub fn scope_ref(community_id: &str, scope_id: &str) -> Result<ScopeRef> {
    Ok(ScopeRef { community: community_bytes(community_id)?, scope: scope_bytes(scope_id) })
}

/// The scope of the journal whose keydist bytes are these (an `Issued`
/// names its scope by them alone).
pub fn scope_id_of(state: &crate::state::ChannelState, scope: &ScopeRef) -> Option<String> {
    if community_bytes(&state.community_id).ok()? != scope.community {
        return None;
    }
    state.scopes.keys().find(|s| scope_bytes(s) == scope.scope).cloned()
}

/// The key that seals one stream in one epoch (§4): keydist's
/// `channel_key` of the epoch key, the community and the stream's name.
pub fn stream_key(epoch_key: &EpochKey, scope: &ScopeRef, channel_id: &str) -> ChannelKey {
    keydist::channel_key(epoch_key, scope, channel_id.as_bytes())
}

/// The `k` of an epoch: the first 16 bytes of SHA-256 of its key, hex.
pub fn key_id(epoch_key: &EpochKey) -> KeyId {
    KeyId::of(epoch_key.key.as_bytes())
}

// ─── Sealed content (§4) ────────────────────────────────────────────────────

/// The secret of the link of one link epoch (§10), base64url.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPayload {
    pub link_epoch: u32,
    pub secret: String,
}

/// What is sealed into a stream: a message, a note of moderation or the
/// secret of the link (an event of its author), or a shard of a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Payload {
    pub v: u32,
    /// `msg` | `mod` | `link` | `meta` | `snapshot`
    pub t: String,
    /// Signed by the real author (kind 1394): of the message, of the
    /// note, or of the `create`/`rotate_link` whose secret it carries
    /// (§4). Absent in `snapshot`.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub event: serde_json::Value,
    /// `snapshot`: the shard bytes, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<String>,
}

pub const T_MSG: &str = "msg";
pub const T_MOD: &str = "mod";
pub const T_LINK: &str = "link";
pub const T_SNAPSHOT: &str = "snapshot";
/// The sealed name of a level or title of a scope (§14.1, 2026-10-22).
pub const T_META: &str = "meta";

impl Payload {
    pub fn event(t: &str, signed: &Event) -> Result<Self> {
        Ok(Self { v: PAYLOAD_VERSION, t: t.into(), event: serde_json::to_value(signed)?, shard: None })
    }

    pub fn shard(bytes: &[u8]) -> Self {
        Self { v: PAYLOAD_VERSION, t: T_SNAPSHOT.into(), event: serde_json::Value::Null, shard: Some(B64.encode(bytes)) }
    }
}

/// AES-256-GCM with a random nonce: base64 of `nonce ‖ ciphertext ‖ tag`.
pub fn seal_bytes(key: &ChannelKey, plaintext: &[u8]) -> Result<String> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(crypto)?;
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(crypto)?;
    let ct = cipher.encrypt(&Nonce::from(nonce), plaintext).map_err(|_| MessengerError::Crypto("sealing failed".into()))?;
    let mut out = nonce.to_vec();
    out.extend(ct);
    Ok(B64.encode(out))
}

pub fn open_bytes(key: &ChannelKey, sealed: &str) -> Result<Vec<u8>> {
    let raw = B64.decode(sealed.trim()).map_err(|_| MessengerError::Crypto("not base64".into()))?;
    if raw.len() < NONCE_LEN + 16 {
        return Err(MessengerError::Crypto("too short".into()));
    }
    let (nonce, ct) = raw.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| MessengerError::Crypto("nonce".into()))?;
    Aes256Gcm::new_from_slice(&key.0)
        .map_err(crypto)?
        .decrypt(&Nonce::from(nonce), ct)
        .map_err(|_| MessengerError::Crypto("does not open with this key".into()))
}

pub fn seal(key: &ChannelKey, payload: &Payload) -> Result<String> {
    seal_bytes(key, serde_json::to_string(payload)?.as_bytes())
}

pub fn open(key: &ChannelKey, sealed: &str) -> Result<Payload> {
    let plain = open_bytes(key, sealed)?;
    let payload: Payload = serde_json::from_slice(&plain).map_err(|_| invalid("payload"))?;
    if payload.v == 0 || payload.v > PAYLOAD_VERSION {
        return Err(invalid("payload version"));
    }
    match payload.t.as_str() {
        T_MSG | T_MOD | T_LINK | T_META if !payload.event.is_null() => Ok(payload),
        T_SNAPSHOT if payload.shard.is_some() => Ok(payload),
        _ => Err(invalid("payload type")),
    }
}

/// Where a sealed event belongs: the channel, the scope, the stream, and
/// for a comment, a reaction, an edit or a delete, its target (§4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub community_id: String,
    pub scope_id: String,
    pub channel_id: String,
}

impl Place {
    /// The feed of stage 1.
    pub fn feed(community_id: &str) -> Self {
        Self::feed_in(community_id, SCOPE_EVERYONE)
    }

    /// The feed of a section (wire §14.3): `s` its scope, `c` `feed`.
    pub fn feed_in(community_id: &str, scope_id: &str) -> Self {
        Self { community_id: community_id.into(), scope_id: scope_id.into(), channel_id: CHANNEL_FEED.into() }
    }
}

/// The open tags of a sealed event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outer {
    pub place: Place,
    pub epoch: u32,
    pub key_id: KeyId,
    /// The thread of a comment, or the target of a reaction, an edit, a
    /// delete.
    pub target: Option<String>,
    pub created_at: i64,
}

/// Seal a payload under the key of the stream in `epoch_key`'s epoch, into
/// an event signed by a key used once and thrown away.
pub fn seal_content(place: &Place, epoch_key: &EpochKey, target: Option<&str>, payload: &Payload, created_at: i64) -> Result<Event> {
    let scope = scope_ref(&place.community_id, &place.scope_id)?;
    let sealed = seal(&stream_key(epoch_key, &scope, &place.channel_id), payload)?;
    let once = Keys::generate();
    let mut b = EventBuilder::new(Kind::from(KIND_CONTENT), sealed)
        .tag(Tag::parse(["h", place.community_id.as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["s", place.scope_id.as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["c", place.channel_id.as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["n", epoch_key.epoch.to_string().as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["k", key_id(epoch_key).0.as_str()]).map_err(crypto)?)
        .custom_created_at(timestamp(created_at));
    if let Some(t) = target {
        b = b.tag(Tag::parse(["e", t]).map_err(crypto)?);
    }
    b.finalize(&once).map_err(crypto)
}

/// The open tags of a sealed event, the signature checked.
pub fn outer(event: &Event) -> Result<Outer> {
    event.verify().map_err(|_| invalid("signature"))?;
    if event.kind.as_u16() != KIND_CONTENT {
        return Err(invalid("not content"));
    }
    let need = |name: &str| tag(event, name).ok_or_else(|| invalid(name));
    let community_id = need("h")?;
    if !is_hex(&community_id, 64) {
        return Err(invalid("h"));
    }
    let epoch = need("n")?.parse::<u32>().map_err(|_| invalid("n"))?;
    let key_id = need("k")?;
    if !is_hex(&key_id, 32) {
        return Err(invalid("k"));
    }
    let target = tag(event, "e");
    if target.as_deref().is_some_and(|t| !is_hex(t, 64)) {
        return Err(invalid("e"));
    }
    Ok(Outer {
        place: Place { community_id, scope_id: need("s")?, channel_id: need("c")? },
        epoch,
        key_id: KeyId(key_id),
        target,
        created_at: event.created_at.as_secs() as i64,
    })
}

/// Where an inner event points: the post a comment is under, the comment
/// a reply answers, the message a reaction, an edit or a delete is about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    pub thread: Option<String>,
    pub parent: Option<String>,
    pub target: Option<String>,
}

impl Links {
    /// What the outer `e` must say: the thread of a comment, else the
    /// target (§4).
    pub fn outer_target(&self) -> Option<&str> {
        self.thread.as_deref().or(self.target.as_deref())
    }
}

/// An inner event signed by its author (kind 1394): the envelope of the
/// messenger as its content, `h`, `s`, `c` of its place, `e` of its links.
pub fn sign_inner(keys: &Keys, place: &Place, links: &Links, content: &str, created_at: i64) -> Result<Event> {
    let mut b = EventBuilder::new(Kind::from(KIND_INNER), content)
        .tag(Tag::parse(["h", place.community_id.as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["s", place.scope_id.as_str()]).map_err(crypto)?)
        .tag(Tag::parse(["c", place.channel_id.as_str()]).map_err(crypto)?)
        .custom_created_at(timestamp(created_at));
    if let Some(t) = &links.thread {
        b = b.tag(Tag::parse(["e", t.as_str(), "", "root"]).map_err(crypto)?);
    }
    if let Some(p) = &links.parent {
        b = b.tag(Tag::parse(["e", p.as_str(), "", "reply"]).map_err(crypto)?);
    }
    if let Some(t) = &links.target {
        b = b.tag(Tag::parse(["e", t.as_str()]).map_err(crypto)?);
    }
    b.finalize(keys).map_err(crypto)
}

/// A message or a note inside a sealed event, its author verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InnerMessage {
    pub id: EventId,
    pub author: PubKey,
    pub created_at: i64,
    /// Application envelope (`{"v","t",…}`), as in chats.
    pub content: String,
    pub links: Links,
}

impl InnerMessage {
    /// A post: a message under no post.
    pub fn is_post(&self) -> bool {
        self.links.thread.is_none()
    }
}

/// Check the inner event of a payload against the outer tags: the
/// signature, the kind, `h`, `s`, `c` equal to the outer ones, the outer
/// `e` that of its links, the two clocks within `SKEW_SECS`, and an
/// envelope a channel takes (no `receipt.read`: §4).
pub fn verify_inner(signed: &serde_json::Value, outer: &Outer) -> Result<InnerMessage> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("inner event"))?;
    event.verify().map_err(|_| invalid("inner signature"))?;
    if event.kind.as_u16() != KIND_INNER {
        return Err(invalid("inner kind"));
    }
    for (name, want) in [("h", &outer.place.community_id), ("s", &outer.place.scope_id), ("c", &outer.place.channel_id)] {
        if tag(&event, name).as_ref() != Some(want) {
            return Err(invalid("inner tags differ from the outer ones"));
        }
    }
    let mut links = Links::default();
    for t in event.tags.iter().filter(|t| t.kind() == "e") {
        let s = t.as_slice();
        let Some(id) = s.get(1).filter(|id| is_hex(id, 64)).cloned() else { return Err(invalid("e")) };
        let slot = match s.get(3).map(String::as_str) {
            Some("root") => &mut links.thread,
            Some("reply") => &mut links.parent,
            None | Some("") => &mut links.target,
            Some(_) => return Err(invalid("e marker")),
        };
        if slot.replace(id).is_some() {
            return Err(invalid("e twice"));
        }
    }
    if links.parent.is_some() && links.thread.is_none() {
        return Err(invalid("a reply without its post"));
    }
    if links.outer_target() != outer.target.as_deref() {
        return Err(invalid("outer e"));
    }
    let created_at = event.created_at.as_secs() as i64;
    if (created_at - outer.created_at).abs() > SKEW_SECS {
        return Err(invalid("inner and outer times differ"));
    }
    if refused_in_channels(&event.content) {
        return Err(invalid("receipt.read in a channel"));
    }
    Ok(InnerMessage { id: EventId::parse(&event.id.to_hex()).ok_or_else(|| invalid("event id"))?, author: pk(&event.pubkey)?, created_at, content: event.content.clone(), links })
}

/// Envelopes a channel never takes: a read receipt (100 000 readers times
/// one event per post; what was read stays on the device, spec §10.4).
pub fn refused_in_channels(content: &str) -> bool {
    messenger_core::Envelope::parse(content).is_ok_and(|e| e.t == messenger_core::envelope::T_RECEIPT_READ)
}

/// The secret of a link epoch as its author hands it on (§4, §10): an
/// inner event (kind 1394) of the author of the `create` or `rotate_link`
/// that made the link epoch, its content `{link_epoch, secret}`, sealed
/// as `t:"link"`.
pub fn sign_link(keys: &Keys, community_id: &str, link: &LinkPayload, created_at: i64) -> Result<Event> {
    sign_inner(keys, &Place::feed(community_id), &Links::default(), &serde_json::to_string(link)?, created_at)
}

/// The secret in a `t:"link"` and who signed it, the inner event checked
/// against the outer tags as a message is. Whether the signer may hand
/// this link epoch on is the journal's (`OpLog::link_author`).
pub fn verify_link(signed: &serde_json::Value, outer: &Outer) -> Result<(PubKey, LinkPayload)> {
    let m = verify_inner(signed, outer)?;
    if m.links != Links::default() {
        return Err(invalid("a link points nowhere"));
    }
    let link: LinkPayload = serde_json::from_str(&m.content).map_err(|_| invalid("link"))?;
    if B64URL.decode(&link.secret).map(|s| s.len()) != Ok(32) {
        return Err(invalid("link secret"));
    }
    Ok((m.author, link))
}

/// The name of a level or the title of a scope (§14.1, 2026-10-22): what
/// the operation that made it would have said in the open, sealed into the
/// feed of `everyone` by its author, so that only members read it. `op_id`
/// binds it to that operation: taken only from its author, and the newest
/// operation of a level or a scope names it. The meta of a scope carries
/// its look as well (2026-10-23, `ScopeLook`); that of a hidden scope is
/// sealed into the feed of the scope itself, not of `everyone`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub op_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// The title of a scope (`everyone`: may be empty, the page names it),
    /// the name of a level.
    pub name: String,
    #[serde(flatten)]
    pub look: ScopeLook,
}

/// How a section shows as a tile (§14.1, 2026-10-23): every field
/// optional, a meta of before has none of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeLook {
    /// A key of the set of icons of the app, `[a-z0-9-]{1,32}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A square picture, sealed on the hosts of the channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<ScopePhoto>,
    /// One of the ten colours of the palette, 0…9.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
    /// Where the tile stands: the smaller first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// The line under the title on the tile, ≤ 80 characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The photo of a section: a blob on the hosts of the channel (`storage`)
/// by the SHA-256 of its bytes, which are `seal_bytes` of the JPEG under
/// `key` (base64url of 32 random bytes): the hosts hold a picture nobody
/// without the meta reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopePhoto {
    pub sha256: String,
    pub key: String,
}

/// The most colours a section picks from: 0…`MAX_SCOPE_COLOR`.
pub const MAX_SCOPE_COLOR: u8 = 9;
/// The longest key of an icon.
pub const MAX_ICON_CHARS: usize = 32;
/// The longest note of a tile.
pub const MAX_SCOPE_NOTE_CHARS: usize = 80;

impl ScopeLook {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn check(&self) -> Check {
        if let Some(icon) = &self.icon {
            if icon.is_empty() || icon.len() > MAX_ICON_CHARS || !icon.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err("icon");
            }
        }
        if let Some(p) = &self.photo {
            p.check()?;
        }
        if self.color.is_some_and(|c| c > MAX_SCOPE_COLOR) {
            return Err("color");
        }
        if let Some(note) = &self.note {
            check_meta_name(note, MAX_SCOPE_NOTE_CHARS).map_err(|_| "note")?;
        }
        Ok(())
    }
}

impl ScopePhoto {
    pub fn check(&self) -> Check {
        if !is_hex(&self.sha256, 64) || B64URL.decode(&self.key).map(|k| k.len()) != Ok(32) {
            return Err("photo");
        }
        Ok(())
    }

    /// The key the photo is sealed under.
    pub fn sealing_key(&self) -> Result<ChannelKey> {
        let raw: [u8; 32] = B64URL.decode(&self.key).ok().and_then(|k| k.try_into().ok()).ok_or_else(|| invalid("photo key"))?;
        Ok(ChannelKey(raw))
    }
}

impl Meta {
    pub fn check(&self) -> Check {
        if !is_hex(&self.op_id, 64) {
            return Err("op_id");
        }
        let max = match (&self.scope_id, self.level) {
            // `everyone` has no title of its own unless the owner gives it one.
            (Some(s), None) if s == SCOPE_EVERYONE && self.name.is_empty() => return self.look.check(),
            (Some(s), None) if is_hex(s, 32) || s == SCOPE_EVERYONE => crate::state::MAX_SCOPE_TITLE_CHARS,
            (None, Some(l)) if l <= crate::state::MAX_LEVEL_ID && self.look.is_empty() => crate::state::MAX_LEVEL_NAME_CHARS,
            _ => return Err("what"),
        };
        check_meta_name(&self.name, max)?;
        self.look.check()
    }
}

/// A name of a level or a title of a scope: not empty, no control
/// characters, at most `max` characters, trimmed.
pub fn check_meta_name(name: &str, max: usize) -> Check {
    if name.trim().is_empty() || name != name.trim() || name.chars().count() > max || name.chars().any(char::is_control) {
        return Err("name");
    }
    Ok(())
}

/// A `t:"meta"` of its author: an inner event (kind 1394) of the feed of
/// `scope_id` (`everyone`, or a hidden scope the meta is of), its content
/// the `Meta`.
pub fn sign_meta(keys: &Keys, community_id: &str, scope_id: &str, meta: &Meta, created_at: i64) -> Result<Event> {
    sign_inner(keys, &Place::feed_in(community_id, scope_id), &Links::default(), &serde_json::to_string(meta)?, created_at)
}

/// The `Meta` in a `t:"meta"` and who signed it. Whether the signer made
/// the operation it names is the journal's. Sealed in `everyone`, or in
/// the feed of the scope it is the meta of (a hidden one, §14.1).
pub fn verify_meta(signed: &serde_json::Value, outer: &Outer) -> Result<(PubKey, Meta, i64)> {
    let m = verify_inner(signed, outer)?;
    if m.links != Links::default() {
        return Err(invalid("a meta points nowhere"));
    }
    let meta: Meta = serde_json::from_str(&m.content).map_err(|_| invalid("meta"))?;
    meta.check().map_err(invalid)?;
    if outer.place.scope_id != SCOPE_EVERYONE && meta.scope_id.as_deref() != Some(outer.place.scope_id.as_str()) {
        return Err(invalid("a meta is sealed in everyone or in its own scope"));
    }
    Ok((m.author, meta, m.created_at))
}

// ─── The journal (§3) ───────────────────────────────────────────────────────

/// An operation signed by its author: the canonical form as the content,
/// `h` of the channel, `p` of the person it is about.
pub fn sign_op(keys: &Keys, op: &Op) -> Result<Event> {
    if op.author.as_hex() != keys.public_key().to_hex() {
        return Err(invalid("an operation is signed by its author"));
    }
    let mut b = EventBuilder::new(Kind::from(KIND_JOURNAL), op.canonical())
        .tag(Tag::parse(["h", op.community_id.as_str()]).map_err(crypto)?)
        .custom_created_at(timestamp(op.created_at));
    if let Some(who) = op.body.subject() {
        b = b.tag(Tag::parse(["p", who.as_hex()]).map_err(crypto)?);
    }
    b.finalize(keys).map_err(crypto)
}

/// The refusal of `verify_op` for an operation of a version newer than
/// this build reads (`OP_VERSION`): kept, not dropped, until an update.
pub const OP_TOO_NEW: &str = "channel_op_too_new";
/// How many versions past the one this build reads an operation may be
/// and still be kept for an update. Versions are counted one at a time,
/// so a build is never more than a few behind; anything further is not
/// the future but junk, and junk is dropped: one event of some absurd
/// version would otherwise sit parked on every device for good.
pub const AHEAD_WINDOW: u32 = 2;

pub fn is_op_too_new(e: &MessengerError) -> bool {
    matches!(e, MessengerError::Invalid(code) if code == OP_TOO_NEW)
}

/// The newest version of operations this device reads. In tests a device
/// may read less than the build does, to stand in for one not updated yet.
fn reads_up_to() -> u32 {
    #[cfg(test)]
    if let Some(v) = READS_UP_TO.with(|c| c.get()) {
        return v;
    }
    crate::op::OP_VERSION
}

#[cfg(test)]
thread_local! {
    pub(crate) static READS_UP_TO: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// Whether a version is one this device keeps for an update: newer than
/// it reads, within `AHEAD_WINDOW` of that.
fn ahead(v: u64) -> bool {
    let reads = u64::from(reads_up_to());
    v > reads && v <= reads + u64::from(AHEAD_WINDOW)
}

/// Is a signed operation event of a version newer than this device
/// reads, and worth waiting for (`ahead`)? Nothing is verified here.
pub fn is_ahead(signed: &serde_json::Value) -> bool {
    signed
        .get("content")
        .and_then(|c| c.as_str())
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|head| head.get("v").and_then(|v| v.as_u64()))
        .is_some_and(ahead)
}

/// Check a journal event and return the operation (§11.2): the
/// signature, the kind, `h`, the author is the signer, the channel is
/// this one, then the version (a newer one is `OP_TOO_NEW`), the body,
/// and that the content is its canonical form again.
pub fn verify_op(signed: &serde_json::Value, community_id: &str) -> Result<Op> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("operation event"))?;
    event.verify().map_err(|_| invalid("operation signature"))?;
    if event.kind.as_u16() != KIND_JOURNAL {
        return Err(invalid("not an operation"));
    }
    if tag(&event, "h").as_deref() != Some(community_id) {
        return Err(invalid("operation of another channel"));
    }
    let head: serde_json::Value = serde_json::from_str(&event.content).map_err(|_| invalid("operation body"))?;
    let v = head.get("v").and_then(|v| v.as_u64()).ok_or_else(|| invalid("operation version"))?;
    if head.get("author").and_then(|a| a.as_str()).and_then(PubKey::parse).is_none_or(|a| a.as_hex() != event.pubkey.to_hex()) {
        return Err(invalid("operation author is not the signer"));
    }
    if head.get("community_id").and_then(|g| g.as_str()) != Some(community_id) {
        return Err(invalid("operation of another channel"));
    }
    if ahead(v) {
        return Err(MessengerError::Invalid(OP_TOO_NEW.into()));
    }
    if v > u64::from(reads_up_to()) {
        return Err(invalid("operation version past the window"));
    }
    let op: Op = serde_json::from_value(head).map_err(|_| invalid("operation body"))?;
    // The content must be the canonical form, or ids would differ between devices.
    if op.canonical() != event.content {
        return Err(invalid("operation is not in canonical form"));
    }
    // A shape of a later version under an older one (a hidden `scope_add`
    // as `v: 2`) would slip past the builds that park what they cannot
    // read and be applied by them as something else.
    if op.v < op.body.version() {
        return Err(invalid("operation version below its shape"));
    }
    Ok(op)
}

// ─── The card (§2) ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub v: u32,
    pub community_id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub picture: String,
    /// By the last snapshot: an estimate.
    #[serde(default)]
    pub members: u64,
    pub join: crate::op::JoinMode,
    /// An inscription.
    #[serde(default)]
    pub code: String,
    pub relays: Vec<String>,
    pub storage: Vec<String>,
    pub created_at: i64,
}

impl Card {
    /// The limits of groups (§2): a name that shows something, the
    /// lengths, an `https://` picture, 1…8 relays and hosts.
    pub fn check(&self) -> std::result::Result<(), &'static str> {
        if self.v == 0 || self.v > CARD_VERSION {
            return Err("v");
        }
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) || messenger_richtext::normalize(name).is_empty() {
            return Err("name");
        }
        if self.about.chars().count() > MAX_ABOUT_CHARS {
            return Err("about");
        }
        let p = &self.picture;
        if !p.is_empty() && (!p.starts_with("https://") || p.chars().count() > MAX_PICTURE_CHARS) {
            return Err("picture");
        }
        if self.code.chars().count() > MAX_CODE_CHARS || self.code.chars().any(char::is_control) {
            return Err("code");
        }
        check_relays(&self.relays).map_err(|_| "relays")?;
        check_storage(&self.storage).map_err(|_| "storage")?;
        Ok(())
    }
}

pub fn sign_card(keys: &Keys, card: &Card) -> Result<Event> {
    card.check().map_err(invalid)?;
    EventBuilder::new(Kind::from(KIND_CARD), serde_json::to_string(card)?)
        .tag(Tag::parse(["d", card.community_id.as_str()]).map_err(crypto)?)
        .custom_created_at(timestamp(card.created_at))
        .finalize(keys)
        .map_err(crypto)
}

/// Read a card event and take it only from `owner` (the owner by the
/// journal, §2, §11.1): a card of anybody else, the previous owner after a
/// handover included, is dropped however new it is.
pub fn verify_card(signed: &serde_json::Value, owner: &PubKey) -> Result<Card> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("card event"))?;
    event.verify().map_err(|_| invalid("card signature"))?;
    if event.kind.as_u16() != KIND_CARD {
        return Err(invalid("not a card"));
    }
    if event.pubkey.to_hex() != owner.as_hex() {
        return Err(invalid("a card not of the owner"));
    }
    let card: Card = serde_json::from_str(&event.content).map_err(|_| invalid("card body"))?;
    if tag(&event, "d").as_deref() != Some(card.community_id.as_str()) {
        return Err(invalid("card d"));
    }
    card.check().map_err(invalid)?;
    Ok(card)
}

// ─── Notes (§7) ─────────────────────────────────────────────────────────────

pub const T_JOIN: &str = "channel.join";
pub const T_REQUEST: &str = "channel.request";
pub const T_INVITE: &str = "channel.invite";
pub const T_INVITE_REPLY: &str = "channel.invite_reply";
pub const T_ADMIT: &str = "channel.admit";
pub const T_REJECTED: &str = "channel.rejected";
pub const T_ISSUE: &str = "channel.issue";
pub const T_REBUNDLE: &str = "channel.rebundle";
pub const T_MISSED: &str = "channel.missed";
pub const T_LEAVE: &str = "channel.leave";
pub const T_KEEPER_MASTER: &str = "keeper.master";
pub const T_LEVEL_REQUEST: &str = "channel.level_request";
pub const T_LEVEL_RESOLVE: &str = "channel.level_resolve";
/// The state of a keeper to the keeper of the next series (§15, 2026-10-23).
pub const T_KEEPER_HANDOVER: &str = "keeper.handover";
/// A channel offered to a new owner, and the offer declined (§16, 2026-10-23).
pub const T_TRANSFER_OFFER: &str = "channel.transfer_offer";
pub const T_TRANSFER_DECLINE: &str = "channel.transfer_decline";
/// The tickets of the moderators (§17, 2026-10-23).
pub const T_TICKET_REQUEST: &str = "channel.ticket_request";
pub const T_TICKET: &str = "channel.ticket";
pub const T_TICKET_USE: &str = "channel.ticket_use";
/// The upload keys of a channel's own server for one of its staff (wire §9.1).
pub const T_UPLOAD_KEYS: &str = "channel.upload_keys";
/// The most tickets one moderator holds at once (§17).
pub const MAX_TICKETS: u32 = 50;
/// How long a ticket lives (§17): short, a newcomer by a ticket reads from
/// its minting under `from_join`.
pub const TICKET_TTL_SECS: i64 = 24 * 3600;

type Check = std::result::Result<(), &'static str>;

/// How old a note may be (§7): a keeper may sleep for a day and the notes
/// wait for it on the relays; a week is the term of an invitation.
pub const NOTE_MAX_AGE_SECS: i64 = 7 * 24 * 3600;

/// The window of a note (§7): not ahead of my clock by more than the
/// skew, not older than `NOTE_MAX_AGE_SECS` (an admission is judged by
/// its own `expires_at`).
pub fn note_in_window(created_at: i64, now: i64) -> bool {
    created_at <= now + SKEW_SECS && now - created_at <= NOTE_MAX_AGE_SECS
}

fn fresh(created_at: i64, now: i64) -> Check {
    if !note_in_window(created_at, now) {
        return Err("created_at");
    }
    Ok(())
}

fn scope_ok(scope_id: &str) -> bool {
    scope_id == SCOPE_EVERYONE || is_hex(scope_id, 32)
}

/// What a joining member shows by an open link: the MAC of the secret of
/// the link epoch (`link::LinkSecret::join_mac`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinProof {
    pub link_epoch: u32,
    pub mac: String,
}

/// Somebody asks the keeper in: by a link (`proof`), or by an admission
/// a moderator made (`admit_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Join {
    pub community_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<JoinProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admit_id: Option<String>,
    pub created_at: i64,
}

impl Join {
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        match (&self.proof, &self.admit_id) {
            (Some(p), None) if is_hex(&p.mac, 64) => {}
            (None, Some(a)) if is_hex(a, 32) => {}
            _ => return Err("proof"),
        }
        fresh(self.created_at, now)
    }
}

/// A request to the moderators of a channel that takes requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub community_id: String,
    #[serde(default)]
    pub note: String,
    pub created_at: i64,
}

impl Request {
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if self.note.chars().count() > MAX_REQUEST_NOTE_CHARS {
            return Err("note");
        }
        fresh(self.created_at, now)
    }
}

/// What a moderator sends to somebody they want in the channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub invite_id: String,
    pub community_id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub picture: String,
    pub relay: String,
    pub members: u64,
    #[serde(default)]
    pub levels: u64,
    pub created_at: i64,
    pub expires_at: i64,
    /// The owner (the author of `create`): the journal fetched on a yes
    /// is taken only under that genesis (§3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<PubKey>,
}

impl Invite {
    /// Whether an invitation that came is one to keep, by my clock `now`:
    /// ids as this app makes them, a name without controls that still shows
    /// something once cleaned, a description and a picture as the journal
    /// takes them, a relay address, dated no later than a little ahead of
    /// me and good for no more than a week from that date.
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.invite_id, 32) {
            return Err("invite_id");
        }
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) || self.shown_name().is_empty() {
            return Err("name");
        }
        if self.about.chars().count() > MAX_ABOUT_CHARS {
            return Err("about");
        }
        let p = &self.picture;
        if !p.is_empty() && (!p.starts_with("https://") || p.chars().count() > MAX_PICTURE_CHARS) {
            return Err("picture");
        }
        if !self.relay.is_empty() && (self.relay.len() > MAX_PICTURE_CHARS || messenger_core::RelayUrl::parse(&self.relay).is_none()) {
            return Err("relay");
        }
        if self.created_at > now + SKEW_SECS || self.expires_at <= now || self.expires_at > self.created_at + INVITE_TTL_SECS {
            return Err("dates");
        }
        Ok(())
    }

    /// The name as it is shown: trimmed, without bidi marks and the like.
    pub fn shown_name(&self) -> String {
        messenger_richtext::normalize(self.name.trim())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteReply {
    pub invite_id: String,
    pub community_id: String,
    pub accept: bool,
}

impl InviteReply {
    pub fn check(&self) -> Check {
        if !is_hex(&self.invite_id, 32) {
            return Err("invite_id");
        }
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        Ok(())
    }
}

/// A moderator lets somebody in: to the keeper (and a copy to them). The
/// keeper takes it from a moderator by the journal, before it expires,
/// and an `invite_id` once (§7.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admit {
    pub community_id: String,
    pub who: PubKey,
    pub admit_id: String,
    #[serde(default)]
    pub levels: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_id: Option<String>,
    pub expires_at: i64,
    /// The ticket the moderator handed the newcomer while the keeper was
    /// away (§17): the keeper counts it used. Last, and not written when
    /// none: an admission of before reads the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
}

impl Admit {
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if !is_hex(&self.admit_id, 32) {
            return Err("admit_id");
        }
        if self.invite_id.as_deref().is_some_and(|i| !is_hex(i, 32)) {
            return Err("invite_id");
        }
        if self.ticket_id.as_deref().is_some_and(|i| !is_hex(i, 32)) {
            return Err("ticket_id");
        }
        if self.expires_at <= now || self.expires_at > now + INVITE_TTL_SECS + SKEW_SECS {
            return Err("expires_at");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub community_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Rejected {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if self.reason.as_deref().is_some_and(|r| r.chars().count() > MAX_REQUEST_NOTE_CHARS) {
            return Err("reason");
        }
        Ok(())
    }
}

/// A reader asks the moderators for a level (§14.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelRequest {
    pub community_id: String,
    pub level: u8,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub created_at: i64,
}

impl LevelRequest {
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        // The base level is everybody's: nobody asks for it.
        if self.level == crate::state::BASE_LEVEL || self.level > crate::state::MAX_LEVEL_ID {
            return Err("level");
        }
        if self.note.chars().count() > MAX_REQUEST_NOTE_CHARS {
            return Err("note");
        }
        fresh(self.created_at, now)
    }
}

/// A moderator answers a request for a level (§14.4): a grant is the
/// `set_levels` of the journal, this note only tells; a refusal says why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelResolve {
    pub community_id: String,
    pub level: u8,
    pub granted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl LevelResolve {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if self.reason.as_deref().is_some_and(|r| r.chars().count() > MAX_REQUEST_NOTE_CHARS) {
            return Err("reason");
        }
        Ok(())
    }
}

/// What the keeper hands a member: where the channel lives, the heads of
/// the journal it knew, the secret of the link, and keydist's `Issued`
/// (`to_bytes`, base64).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub community_id: String,
    pub relays: Vec<String>,
    pub storage: Vec<String>,
    pub heads: Vec<OpId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkPayload>,
    pub issued: String,
}

impl Issue {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        check_relays(&self.relays).map_err(|_| "relays")?;
        check_storage(&self.storage).map_err(|_| "storage")?;
        if self.heads.is_empty() || self.heads.len() > crate::op::MAX_HEADS || self.heads.iter().any(|h| !is_hex(&h.0, 64)) {
            return Err("heads");
        }
        if self.link.as_ref().is_some_and(|l| B64URL.decode(&l.secret).map(|s| s.len()) != Ok(32)) {
            return Err("link");
        }
        self.issued().map(|_| ()).map_err(|_| "issued")
    }

    /// What the keeper issued.
    pub fn issued(&self) -> std::result::Result<keydist::Issued, keydist::Error> {
        let bytes = keydist::SecretBytes::new(B64.decode(&self.issued).map_err(|_| keydist::Error::Invalid("issued"))?);
        keydist::Issued::from_bytes(&bytes)
    }

    /// `issued` as it travels.
    pub fn encode_issued(issued: &keydist::Issued) -> String {
        B64.encode(issued.to_bytes().as_slice())
    }
}

/// A member asks the keeper for its bundle of generation `gen` again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rebundle {
    pub community_id: String,
    pub scope_id: String,
    pub gen: u32,
}

/// A member asks for the next page of keys it missed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missed {
    pub community_id: String,
    pub scope_id: String,
    pub after_epoch: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leave {
    pub community_id: String,
}

/// The owner hands the keeper the master of a generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperMaster {
    pub community_id: String,
    pub scope_id: String,
    pub gen: u32,
    /// Base64 of 32 bytes.
    pub master: String,
}

impl Rebundle {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) || !scope_ok(&self.scope_id) {
            return Err("scope");
        }
        Ok(())
    }
}

impl Missed {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) || !scope_ok(&self.scope_id) {
            return Err("scope");
        }
        Ok(())
    }
}

impl Leave {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        Ok(())
    }
}

impl KeeperMaster {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) || !scope_ok(&self.scope_id) {
            return Err("scope");
        }
        if B64.decode(&self.master).map(|m| m.len()) != Ok(32) {
            return Err("master");
        }
        Ok(())
    }
}

/// `keeper.handover` (§15): the state of the keeper of the series before
/// `serial`, sealed in a blob on the hosts of the channel with `key`
/// (AES-256-GCM, `nonce ‖ ciphertext ‖ tag`, the seal of the feed); the
/// note itself goes by NIP-59 to the inbox of the series `serial`, signed
/// by the key of the series before it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperHandover {
    pub community_id: String,
    pub serial: u32,
    /// Hex SHA-256 of the sealed blob.
    pub sha256: String,
    pub hosts: Vec<String>,
    /// Base64 of the 32 bytes of the key of the blob.
    pub key: String,
}

impl KeeperHandover {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if !is_hex(&self.sha256, 64) {
            return Err("sha256");
        }
        if self.hosts.is_empty() || self.hosts.len() > 8 || self.hosts.iter().any(|h| !h.starts_with("https://") || h.len() > 256) {
            return Err("hosts");
        }
        if B64.decode(&self.key).map(|k| k.len()) != Ok(32) {
            return Err("key");
        }
        Ok(())
    }
}

/// `channel.transfer_offer` (§16): the owner to the one the journal's
/// `transfer_ownership` names, as a person (no hint of the channel's
/// relays): what to show in «Вам передают канал».
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferOffer {
    pub community_id: String,
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub picture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
}

impl TransferOffer {
    pub fn check(&self, now: i64) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if self.name.chars().count() > 200 || self.about.chars().count() > 2000 || self.picture.len() > 2048 {
            return Err("text");
        }
        fresh(self.created_at, now)?;
        if self.expires_at <= now || self.expires_at > now + INVITE_TTL_SECS + SKEW_SECS {
            return Err("expires_at");
        }
        Ok(())
    }
}

/// `channel.transfer_decline` (§16): the one offered the channel says no.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferDecline {
    pub community_id: String,
}

/// `channel.ticket_request` (§17): a moderator asks the keeper for tickets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketRequest {
    pub community_id: String,
    pub n: u32,
}

impl TicketRequest {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) {
            return Err("community_id");
        }
        if self.n == 0 || self.n > MAX_TICKETS {
            return Err("n");
        }
        Ok(())
    }
}

/// One ticket: an `Issued` of `everyone` for a newcomer not named yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub ticket_id: String,
    /// Base64 of `Issued::to_bytes`.
    pub issued: String,
    pub expires_at: i64,
}

/// `channel.ticket` (§17): the keeper to a moderator, up to
/// `TICKETS_PER_NOTE` a note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tickets {
    pub community_id: String,
    pub tickets: Vec<Ticket>,
}

/// How many tickets one note carries: a bundle is about a kilobyte, a
/// note stays well inside what a relay takes.
pub const TICKETS_PER_NOTE: usize = 10;

impl Tickets {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) || self.tickets.is_empty() || self.tickets.len() > TICKETS_PER_NOTE {
            return Err("tickets");
        }
        if self.tickets.iter().any(|t| !is_hex(&t.ticket_id, 32) || B64.decode(&t.issued).is_err()) {
            return Err("ticket");
        }
        Ok(())
    }
}

/// `channel.ticket_use` (§17): a moderator hands a newcomer a ticket while
/// the keeper is away; the newcomer reads at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketUse {
    pub community_id: String,
    pub ticket_id: String,
    /// Base64 of `Issued::to_bytes`.
    pub issued: String,
}

/// `{"v":1,"t":"channel.join", …fields}`: the body of a note as the
/// content of its rumor.
pub fn note_envelope<T: Serialize>(t: &str, body: &T) -> Result<String> {
    let mut e = messenger_core::Envelope::new(t);
    if let serde_json::Value::Object(map) = serde_json::to_value(body)? {
        e.fields = map;
    }
    Ok(e.encode())
}

pub fn note_body<T: for<'de> Deserialize<'de>>(e: &messenger_core::Envelope) -> Result<T> {
    serde_json::from_value(serde_json::Value::Object(e.fields.clone())).map_err(|_| invalid("note body"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::OpBody;
    use keydist::Key32;

    const CID: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

    fn me(k: &Keys) -> PubKey {
        pk(&k.public_key()).unwrap()
    }

    fn epoch_key(n: u32, byte: u8) -> EpochKey {
        EpochKey { epoch: n, key: Key32([byte; 32]) }
    }

    fn text(t: &str) -> String {
        messenger_core::Envelope::text(t).encode()
    }

    #[test]
    fn seal_and_open() {
        let scope = scope_ref(CID, SCOPE_EVERYONE).unwrap();
        let k = stream_key(&epoch_key(3, 7), &scope, CHANNEL_FEED);
        let link = LinkPayload { link_epoch: 2, secret: B64URL.encode([9u8; 32]) };
        let p = Payload::event(T_LINK, &sign_link(&Keys::generate(), CID, &link, 1_000).unwrap()).unwrap();
        let sealed = seal(&k, &p).unwrap();
        assert_ne!(sealed, seal(&k, &p).unwrap(), "a fresh nonce every time");
        let back = open(&k, &sealed).unwrap();
        assert_eq!((back.t.as_str(), &back.event), (T_LINK, &p.event));
        let unsigned = seal_bytes(&k, br#"{"v":1,"t":"link","link":{"link_epoch":2,"secret":"x"}}"#).unwrap();
        assert!(open(&k, &unsigned).is_err(), "a secret nobody signed");
        let other = stream_key(&epoch_key(4, 8), &scope, CHANNEL_FEED);
        assert!(open(&other, &sealed).is_err(), "another epoch");
        assert!(open(&stream_key(&epoch_key(3, 7), &scope, "talk"), &sealed).is_err(), "another stream");
        let mut raw = B64.decode(&sealed).unwrap();
        raw[15] ^= 1;
        assert!(open(&k, &B64.encode(raw)).is_err());
        assert!(open(&k, "AAAA").is_err());
        // A payload of no known shape.
        let odd = seal_bytes(&k, br#"{"v":1,"t":"chain"}"#).unwrap();
        assert!(open(&k, &odd).is_err());
        let bare = seal_bytes(&k, br#"{"v":1,"t":"msg"}"#).unwrap();
        assert!(open(&k, &bare).is_err(), "a message without its event");
        let shard = open(&k, &seal(&k, &Payload::shard(&[1, 2, 3])).unwrap()).unwrap();
        assert_eq!(B64.decode(shard.shard.unwrap()).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn key_id_and_scope_bytes() {
        let k = epoch_key(1, 1);
        assert_eq!(key_id(&k), KeyId::of(&[1u8; 32]));
        assert_eq!(key_id(&k).0.len(), 32);
        assert_ne!(scope_bytes("everyone"), scope_bytes("Everyone"));
        assert!(scope_ref("short", SCOPE_EVERYONE).is_err());
        assert_eq!(scope_ref(CID, SCOPE_EVERYONE).unwrap().community[31], 0xaa);
    }

    fn post_through(author: &Keys, links: &Links, outer_target: Option<&str>, content: &str, inner_place: &Place) -> Result<InnerMessage> {
        let place = Place::feed(CID);
        let k = epoch_key(5, 3);
        let inner = sign_inner(author, inner_place, links, content, 1_760_000_000)?;
        let event = seal_content(&place, &k, outer_target, &Payload::event(T_MSG, &inner)?, 1_760_000_100)?;
        let o = outer(&event)?;
        assert_eq!((o.epoch, o.key_id.clone()), (5, key_id(&k)));
        let scope = scope_ref(CID, SCOPE_EVERYONE)?;
        let payload = open(&stream_key(&k, &scope, CHANNEL_FEED), &event.content)?;
        verify_inner(&payload.event, &o)
    }

    #[test]
    fn the_relay_sees_the_place_and_never_the_author() {
        let alice = Keys::generate();
        let place = Place::feed(CID);
        let k = epoch_key(5, 3);
        let inner = sign_inner(&alice, &place, &Links::default(), &text("secret words"), 1_760_000_000).unwrap();
        let event = seal_content(&place, &k, None, &Payload::event(T_MSG, &inner).unwrap(), 1_760_000_000).unwrap();
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains(&alice.public_key().to_hex()), "the author is not on the outside");
        assert!(!json.contains("secret words"));
        assert_eq!(event.kind.as_u16(), KIND_CONTENT);
        for (name, want) in [("h", CID.to_string()), ("s", "everyone".into()), ("c", "feed".into()), ("n", "5".into()), ("k", key_id(&k).0)] {
            assert_eq!(tag(&event, name), Some(want), "{name}");
        }
        let m = post_through(&alice, &Links::default(), None, &text("hi"), &place).unwrap();
        assert_eq!(m.author, me(&alice));
        assert!(m.is_post());
    }

    #[test]
    fn outer_and_inner_tags_must_agree() {
        let alice = Keys::generate();
        let feed = Place::feed(CID);
        let post = "ab".repeat(32);
        // A comment: `e` root inside, its post outside.
        let comment = Links { thread: Some(post.clone()), ..Links::default() };
        let m = post_through(&alice, &comment, Some(&post), &text("c"), &feed).unwrap();
        assert_eq!(m.links.thread.as_deref(), Some(post.as_str()));
        assert!(!m.is_post());
        // A reply in a branch.
        let reply = Links { thread: Some(post.clone()), parent: Some("cd".repeat(32)), target: None };
        assert!(post_through(&alice, &reply, Some(&post), &text("r"), &feed).is_ok());
        // A reaction: its target outside.
        let reaction = Links { target: Some(post.clone()), ..Links::default() };
        let env = messenger_core::Envelope::reaction(&post, "👍", true).encode();
        assert!(post_through(&alice, &reaction, Some(&post), &env, &feed).is_ok());
        // Mismatches are refused.
        assert!(post_through(&alice, &comment, None, &text("c"), &feed).is_err(), "outer e missing");
        assert!(post_through(&alice, &comment, Some(&"ef".repeat(32)), &text("c"), &feed).is_err(), "outer e of another post");
        assert!(post_through(&alice, &Links::default(), Some(&post), &text("p"), &feed).is_err(), "a post with an outer e");
        for other in [
            Place { community_id: "bb".repeat(32), ..feed.clone() },
            Place { scope_id: "0123456789abcdef0123456789abcdef".into(), ..feed.clone() },
            Place { channel_id: "talk".into(), ..feed.clone() },
        ] {
            assert!(post_through(&alice, &Links::default(), None, &text("p"), &other).is_err(), "{other:?}");
        }
        let orphan = Links { parent: Some(post.clone()), ..Links::default() };
        assert!(post_through(&alice, &orphan, None, &text("r"), &feed).is_err(), "a reply without its post");
        // A read receipt is never taken in a channel.
        let receipt = messenger_core::Envelope::receipt_read(1).encode();
        assert!(post_through(&alice, &Links::default(), None, &receipt, &feed).is_err());
    }

    /// The meta of a scope (§14.1, 2026-10-23): a meta of before reads as
    /// one without a look; the look has its bounds; a level has none; a
    /// meta is sealed in `everyone` or in the feed of its own scope, never
    /// in another.
    #[test]
    fn a_meta_carries_the_look_of_a_scope_and_is_sealed_where_it_belongs() {
        let gold = "ab".repeat(16);
        let op_id = "01".repeat(32);
        let old: Meta = serde_json::from_str(&format!(r#"{{"op_id":"{op_id}","scope_id":"{gold}","name":"Gold"}}"#)).unwrap();
        assert!(old.look.is_empty() && old.check().is_ok());
        assert_eq!(serde_json::to_string(&old).unwrap(), format!(r#"{{"op_id":"{op_id}","scope_id":"{gold}","name":"Gold"}}"#), "no field it has not");
        let photo = ScopePhoto { sha256: "cd".repeat(32), key: B64URL.encode([5u8; 32]) };
        let look = ScopeLook { icon: Some("star-2".into()), photo: Some(photo.clone()), color: Some(9), order: Some(-3), note: Some("For patrons".into()) };
        let meta = Meta { look: look.clone(), ..old.clone() };
        assert!(meta.check().is_ok());
        let back: Meta = serde_json::from_str(&serde_json::to_string(&meta).unwrap()).unwrap();
        assert_eq!(back, meta);
        assert_eq!(photo.sealing_key().unwrap().0, [5u8; 32]);
        for (bad, what) in [
            (ScopeLook { icon: Some("Star".into()), ..look.clone() }, "icon"),
            (ScopeLook { icon: Some(String::new()), ..look.clone() }, "icon"),
            (ScopeLook { icon: Some("a".repeat(33)), ..look.clone() }, "icon"),
            (ScopeLook { color: Some(10), ..look.clone() }, "color"),
            (ScopeLook { note: Some(" padded ".into()), ..look.clone() }, "note"),
            (ScopeLook { note: Some("n".repeat(81)), ..look.clone() }, "note"),
            (ScopeLook { photo: Some(ScopePhoto { key: "short".into(), ..photo.clone() }), ..look.clone() }, "photo"),
        ] {
            assert_eq!(Meta { look: bad, ..old.clone() }.check(), Err(what));
        }
        let level = Meta { scope_id: None, level: Some(2), look: ScopeLook { color: Some(1), ..ScopeLook::default() }, ..old.clone() };
        assert_eq!(level.check(), Err("what"), "a level has no look");
        // `everyone` may have no title, and a look.
        let everyone = Meta { scope_id: Some(SCOPE_EVERYONE.into()), name: String::new(), look: ScopeLook { color: Some(2), ..ScopeLook::default() }, ..old.clone() };
        assert!(everyone.check().is_ok());
        // Sealed in its own scope it is taken; in another, refused.
        let owner = Keys::generate();
        let k = epoch_key(1, 4);
        let through = |seal_in: &str, meta: &Meta| {
            let place = Place::feed_in(CID, seal_in);
            let inner = sign_meta(&owner, CID, seal_in, meta, 1_760_000_000).unwrap();
            let event = seal_content(&place, &k, None, &Payload::event(T_META, &inner).unwrap(), 1_760_000_000).unwrap();
            verify_meta(&serde_json::to_value(&inner).unwrap(), &outer(&event).unwrap())
        };
        assert_eq!(through(&gold, &meta).unwrap().1, meta);
        assert!(through(SCOPE_EVERYONE, &meta).is_ok());
        assert!(through(&"ef".repeat(16), &meta).is_err(), "the meta of gold sealed in another scope");
    }

    #[test]
    fn inner_and_outer_times_stay_close() {
        let alice = Keys::generate();
        let place = Place::feed(CID);
        let k = epoch_key(1, 1);
        let inner = sign_inner(&alice, &place, &Links::default(), &text("x"), 1_000).unwrap();
        let event = seal_content(&place, &k, None, &Payload::event(T_MSG, &inner).unwrap(), 1_000 + SKEW_SECS + 1).unwrap();
        let o = outer(&event).unwrap();
        assert!(verify_inner(&serde_json::to_value(&inner).unwrap(), &o).is_err());
        // An inner event of another kind is no message of a channel.
        let nine = EventBuilder::new(Kind::from(9u16), text("x")).tag(Tag::parse(["h", CID]).unwrap()).finalize(&alice).unwrap();
        assert!(verify_inner(&serde_json::to_value(&nine).unwrap(), &o).is_err());
    }

    #[test]
    fn operations_are_bound_to_their_signer_and_their_channel() {
        let alice = Keys::generate();
        let mallory = Keys::generate();
        let op = Op::new(CID, &me(&alice), vec![OpId("11".repeat(32))], 5, OpBody::Ban { who: me(&mallory), reason: None });
        let event = sign_op(&alice, &op).unwrap();
        assert_eq!(event.kind.as_u16(), KIND_JOURNAL);
        assert_eq!(tag(&event, "p").as_deref(), Some(me(&mallory).as_hex()));
        let signed = serde_json::to_value(&event).unwrap();
        assert_eq!(verify_op(&signed, CID).unwrap(), op);
        assert!(verify_op(&signed, &"bb".repeat(32)).is_err(), "another channel");
        assert!(sign_op(&mallory, &op).is_err(), "signed by somebody else than the author");
        // Somebody else's name, signed by mallory: refused.
        let forged = EventBuilder::new(Kind::from(KIND_JOURNAL), op.canonical()).tag(Tag::parse(["h", CID]).unwrap()).finalize(&mallory).unwrap();
        assert!(verify_op(&serde_json::to_value(&forged).unwrap(), CID).is_err());
        // Not the canonical form: refused.
        let spaced = op.canonical().replacen(",", ", ", 1);
        let loose = EventBuilder::new(Kind::from(KIND_JOURNAL), spaced).tag(Tag::parse(["h", CID]).unwrap()).finalize(&alice).unwrap();
        assert!(verify_op(&serde_json::to_value(&loose).unwrap(), CID).is_err());
        // Without `h`: refused.
        let untagged = EventBuilder::new(Kind::from(KIND_JOURNAL), op.canonical()).finalize(&alice).unwrap();
        assert!(verify_op(&serde_json::to_value(&untagged).unwrap(), CID).is_err());
    }

    #[test]
    fn a_too_new_operation_is_kept_only_within_the_window() {
        let alice = Keys::generate();
        let ahead_op = |v: u32| {
            let mut op = Op::new(CID, &me(&alice), vec![OpId("11".repeat(32))], 5, OpBody::Disband);
            op.v = v;
            serde_json::to_value(sign_op(&alice, &op).unwrap()).unwrap()
        };
        let reads = crate::op::OP_VERSION;
        for v in reads + 1..=reads + AHEAD_WINDOW {
            assert!(is_op_too_new(&verify_op(&ahead_op(v), CID).unwrap_err()), "v{v}");
            assert!(is_ahead(&ahead_op(v)), "v{v}");
        }
        for v in [reads + AHEAD_WINDOW + 1, 99, u32::MAX] {
            assert!(!is_op_too_new(&verify_op(&ahead_op(v), CID).unwrap_err()), "v{v}");
            assert!(!is_ahead(&ahead_op(v)));
        }
        READS_UP_TO.with(|c| c.set(Some(0)));
        assert!(is_op_too_new(&verify_op(&ahead_op(1), CID).unwrap_err()), "a device not updated keeps it");
        READS_UP_TO.with(|c| c.set(None));
    }

    fn card() -> Card {
        Card {
            v: 1,
            community_id: CID.into(),
            name: "News".into(),
            about: String::new(),
            picture: String::new(),
            members: 10,
            join: crate::op::JoinMode::Open,
            code: "NEWS".into(),
            relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
            storage: vec!["https://s1.example".into()],
            created_at: 1_760_000_000,
        }
    }

    #[test]
    fn a_card_is_taken_from_the_owner_and_within_its_limits() {
        let owner = Keys::generate();
        let other = Keys::generate();
        let signed = serde_json::to_value(sign_card(&owner, &card()).unwrap()).unwrap();
        assert_eq!(verify_card(&signed, &me(&owner)).unwrap(), card());
        assert!(verify_card(&signed, &me(&other)).is_err(), "not the owner by the journal");
        assert!(card().check().is_ok());
        let bad: Vec<(&str, Card)> = vec![
            ("name", Card { name: " ".into(), ..card() }),
            ("name", Card { name: "x".repeat(101), ..card() }),
            ("about", Card { about: "x".repeat(1001), ..card() }),
            ("picture", Card { picture: "http://p.example".into(), ..card() }),
            ("picture", Card { picture: format!("https://{}", "p".repeat(500)), ..card() }),
            ("code", Card { code: "x".repeat(33), ..card() }),
            ("relays", Card { relays: vec![], ..card() }),
            ("relays", Card { relays: (0..9).map(|i| format!("wss://r{i}.example")).collect(), ..card() }),
            ("storage", Card { storage: vec![], ..card() }),
            ("storage", Card { storage: (0..9).map(|i| format!("https://s{i}.example")).collect(), ..card() }),
            ("v", Card { v: 2, ..card() }),
            ("community_id", Card { community_id: "x".into(), ..card() }),
        ];
        for (want, c) in bad {
            assert_eq!(c.check(), Err(want), "{c:?}");
        }
        assert!(sign_card(&owner, &Card { name: String::new(), ..card() }).is_err());
        // A `d` of another channel.
        let mismatched = EventBuilder::new(Kind::from(KIND_CARD), serde_json::to_string(&card()).unwrap())
            .tag(Tag::parse(["d", "bb".repeat(32).as_str()]).unwrap())
            .finalize(&owner)
            .unwrap();
        assert!(verify_card(&serde_json::to_value(&mismatched).unwrap(), &me(&owner)).is_err());
    }

    #[test]
    fn notes_have_their_types_and_checks() {
        let now = 1_760_000_000;
        let join = Join { community_id: CID.into(), proof: Some(JoinProof { link_epoch: 0, mac: "ab".repeat(32) }), admit_id: None, created_at: now };
        assert_eq!(join.check(now), Ok(()));
        assert_eq!(Join { created_at: now - 6 * 24 * 3600, ..join.clone() }.check(now), Ok(()), "a keeper that slept takes it");
        assert_eq!(Join { created_at: now - NOTE_MAX_AGE_SECS - 1, ..join.clone() }.check(now), Err("created_at"), "older than a week");
        assert_eq!(Join { created_at: now + 301, ..join.clone() }.check(now), Err("created_at"), "ahead of the skew");
        assert_eq!(Join { admit_id: Some("cd".repeat(16)), ..join.clone() }.check(now), Err("proof"), "one way in, not two");
        assert_eq!(Join { proof: None, admit_id: Some("cd".repeat(16)), ..join.clone() }.check(now), Ok(()));
        assert_eq!(Join { proof: None, ..join.clone() }.check(now), Err("proof"));
        let e = messenger_core::Envelope::parse(&note_envelope(T_JOIN, &join).unwrap()).unwrap();
        assert_eq!(e.t, T_JOIN);
        assert_eq!(note_body::<Join>(&e).unwrap(), join);

        assert_eq!(Request { community_id: CID.into(), note: "x".repeat(501), created_at: now }.check(now), Err("note"));
        let invite = Invite {
            invite_id: "ab".repeat(16),
            community_id: CID.into(),
            name: "News".into(),
            about: String::new(),
            picture: String::new(),
            relay: "wss://r.example".into(),
            members: 3,
            levels: 0,
            created_at: now,
            expires_at: now + INVITE_TTL_SECS,
            owner: None,
        };
        assert_eq!(invite.check(now), Ok(()));
        assert_eq!(Invite { expires_at: now + INVITE_TTL_SECS + 1, ..invite.clone() }.check(now), Err("dates"));
        assert_eq!(Invite { name: "\u{202e}".into(), ..invite.clone() }.check(now), Err("name"));
        assert_eq!(InviteReply { invite_id: "ab".repeat(16), community_id: CID.into(), accept: true }.check(), Ok(()));
        let admit = Admit { community_id: CID.into(), who: PubKey::parse(&"c".repeat(64)).unwrap(), admit_id: "ef".repeat(16), levels: 1, invite_id: None, expires_at: now + 3600, ticket_id: None };
        assert_eq!(admit.check(now), Ok(()));
        assert_eq!(Admit { expires_at: now, ..admit.clone() }.check(now), Err("expires_at"));
        assert_eq!(Admit { expires_at: now + 8 * 24 * 3600, ..admit.clone() }.check(now), Err("expires_at"));
        assert_eq!(Rejected { community_id: CID.into(), reason: None }.check(), Ok(()));
        assert_eq!(Rebundle { community_id: CID.into(), scope_id: "everyone".into(), gen: 1 }.check(), Ok(()));
        assert_eq!(Missed { community_id: CID.into(), scope_id: "nobody".into(), after_epoch: 1 }.check(), Err("scope"));
        assert_eq!(Leave { community_id: CID.into() }.check(), Ok(()));
        let master = KeeperMaster { community_id: CID.into(), scope_id: "everyone".into(), gen: 2, master: B64.encode([1u8; 32]) };
        assert_eq!(master.check(), Ok(()));
        assert_eq!(KeeperMaster { master: B64.encode([1u8; 31]), ..master }.check(), Err("master"));
        for t in [T_JOIN, T_REQUEST, T_INVITE, T_INVITE_REPLY, T_ADMIT, T_REJECTED, T_ISSUE, T_REBUNDLE, T_MISSED, T_LEAVE] {
            assert!(t.starts_with("channel."), "{t}");
        }
    }

    #[test]
    fn an_issue_carries_what_keydist_issued() {
        use keydist::{GenerationKind, Issued, SchemeId, SecretBytes};
        let issued = Issued {
            scheme: SchemeId::SdV1,
            scope: scope_ref(CID, SCOPE_EVERYONE).unwrap(),
            gen: 1,
            leaf: 7,
            kind: GenerationKind::Initial,
            bundle: SecretBytes::new(vec![]),
            current: Some(epoch_key(4, 9)),
            missed: vec![],
            missed_more: false,
        };
        let issue = Issue {
            community_id: CID.into(),
            relays: vec!["wss://r1.example".into(), "wss://r2.example".into()],
            storage: vec!["https://s1.example".into(), "https://s2.example".into()],
            heads: vec![OpId("ab".repeat(32))],
            link: Some(LinkPayload { link_epoch: 0, secret: B64URL.encode([5u8; 32]) }),
            issued: Issue::encode_issued(&issued),
        };
        assert_eq!(issue.check(), Ok(()));
        assert_eq!(issue.issued().unwrap(), issued);
        assert_eq!(Issue { heads: vec![], ..issue.clone() }.check(), Err("heads"));
        assert_eq!(Issue { issued: "AAAA".into(), ..issue.clone() }.check(), Err("issued"));
        assert_eq!(Issue { link: Some(LinkPayload { link_epoch: 0, secret: "AAAA".into() }), ..issue }.check(), Err("link"));
    }
}

/// One upload key of `channel.upload_keys` (§9.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadKey {
    /// The base address of a host of `storage[]`.
    pub host: String,
    /// `vu1.<label>.<pubkey>.<iat>.<exp>.<mac>`.
    pub key: String,
    pub expires_at: i64,
}

/// `channel.upload_keys` (§9.1): the keeper hands a member of the staff the
/// keys that let him write to the hosts of the owner's own server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadKeys {
    pub community_id: String,
    #[serde(default)]
    pub keys: Vec<UploadKey>,
    /// The key of the channel for the owner's Veydan Cloud (§9.2), or the
    /// word to drop it (`secret` empty). Written only when there is one, so
    /// a note of own servers alone is the note of before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud: Option<CloudKey>,
}

/// The key of a channel for the owner's Veydan Cloud (§9.2): the member
/// uploads the files of posts with it, and the owner's account counts
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudKey {
    /// 64 hex: the npub of the Cloud account it uploads into.
    pub owner: String,
    /// 64 hex: the secret of the key; empty: the member drops his key of
    /// the channel (his quota is his own now, or he is no staff).
    pub secret: String,
    pub expires_at: i64,
}

/// The most keys one note carries: one a host, as many as `storage[]` holds.
pub const UPLOAD_KEYS_PER_NOTE: usize = 8;

impl UploadKeys {
    pub fn check(&self) -> Check {
        if !is_hex(&self.community_id, 64) || (self.keys.is_empty() && self.cloud.is_none()) || self.keys.len() > UPLOAD_KEYS_PER_NOTE {
            return Err("upload keys");
        }
        if let Some(c) = &self.cloud {
            if !is_hex(&c.owner, 64) || !(c.secret.is_empty() || is_hex(&c.secret, 64)) {
                return Err("cloud key");
            }
        }
        if self.keys.iter().any(|k| !k.host.starts_with("https://") || k.host.len() > 500 || crate::upload_keys::parse(&k.key).is_none()) {
            return Err("upload key");
        }
        Ok(())
    }
}

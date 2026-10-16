// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The server's side: check a report and each proof in it with nothing but
//! what the report carries. No key of any user is needed, and none is
//! learned: the keys of a DM proof open the one message they were cut for.

use std::collections::HashSet;

use nostr::event::{Event, Kind, UnsignedEvent};
use nostr::key::PublicKey;
use serde::{Deserialize, Serialize};

use crate::nip44::Payload;
use crate::wire::{Attached, Context, MessageKeys, Proof, Report, Target, REPORT_VERSION};

/// A report attaches at most this many messages.
pub const MAX_MESSAGES: usize = 10;
/// The reporter's own words, in characters.
pub const MAX_COMMENT_CHARS: usize = 2000;
/// A sub-reason is a word of a list, not a text.
pub const MAX_SUB_REASON_CHARS: usize = 64;
/// The note of a message without a proof is a word too.
pub const MAX_NOTE_CHARS: usize = 64;
/// The id of a group, in characters; ours are far shorter.
pub const MAX_GROUP_ID_CHARS: usize = 128;
/// The name of a group a report carries, in characters.
pub const MAX_GROUP_NAME_CHARS: usize = 128;
/// The owner and the admins of a group a report names.
pub const MAX_GROUP_ADMINS: usize = 32;
/// How far in the past a report may be dated: a device without a connection
/// keeps a report and sends it for up to seven days (the outbox of the
/// runtime), so a report is as old as that at most; older is one sent again
/// or made up.
pub const REPORT_PAST_WINDOW_SECS: i64 = 8 * 86_400;
/// How far in the future a report may be dated: the two clocks differ by
/// minutes at most.
pub const REPORT_FUTURE_WINDOW_SECS: i64 = 3_600;
/// How far a seal's time may lie from its rumor's: NIP-59 puts the seal up
/// to two days in the past to hide when a message was sent (nostr 0.45
/// draws it from `0..172800` seconds, `RANGE_RANDOM_TIMESTAMP_TWEAK`).
pub const SEAL_TWEAK_SECS: i64 = 2 * 86_400;
/// Room on top of [`SEAL_TWEAK_SECS`]: a chat may date a message a little
/// after the last one when two fall in one second, and another client may
/// draw its tweak around the time rather than before it.
pub const SEAL_TIME_SLACK_SECS: i64 = 3600;
/// The largest content of a seal or of a group message that is read at
/// all. A message's text is at most 32 KiB (`messenger_dm::service::
/// MAX_TEXT_BYTES`), its envelope a little more, a seal's base64 a third
/// more again; anything far past that is not one of ours.
pub const MAX_CONTENT_BYTES: usize = 256 * 1024;

/// The seal of NIP-59.
pub const KIND_SEAL: u16 = 13;
/// The gift wrap of NIP-59.
pub const KIND_GIFT_WRAP: u16 = 1059;
/// The rumors a proof may carry: a message of NIP-17. The messenger wraps
/// two more kinds (`messenger_dm::wrap`): the notes between two
/// (`KIND_PEER_NOTE_RUMOR`: receipts, reactions, keys, the signalling of
/// calls) and the notes between my own devices (`KIND_OWN_RUMOR`). Neither
/// is something a person wrote to another, so neither is evidence. An
/// attachment is a kind-14 message too (an envelope `t = media`): this
/// messenger does not use the file messages of kind 15.
pub const DM_KINDS: [u16; 1] = [14];
/// A message of a group, inside its sealed event
/// (`messenger_groups::wire::KIND_INNER_MESSAGE`).
pub const KIND_GROUP_MESSAGE: u16 = 9;
/// The fields of an attachment only the sender's device writes
/// (`messenger_dm::media::LOCAL_FIELDS`); a receiver drops them, and so
/// does the copy of the descriptor a report shows.
const LOCAL_MEDIA_FIELDS: [&str; 3] = ["local_path", "transfer_id", "original"];

/// What a server learns from a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    /// Why the report as a whole is refused, a short word; `None` when it
    /// is well formed. A refused report has its messages left unchecked.
    pub refused: Option<String>,
    /// One outcome per attached message, in their order.
    pub messages: Vec<Outcome>,
}

impl Verified {
    /// The report is well formed (whatever its proofs show).
    pub fn accepted(&self) -> bool {
        self.refused.is_none()
    }

    /// The messages proved, in their order.
    pub fn proved(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter().filter_map(|o| match o {
            Outcome::Verified(m) => Some(m),
            Outcome::Unverified(_) => None,
        })
    }
}

/// The check of one attached message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Its author's key wrote it, as shown.
    Verified(Message),
    /// It proves nothing, and the short word says why: `no_proof`,
    /// `duplicate`, `too_large`, `seal_signature`, `seal_kind`,
    /// `seal_author`, `payload_*` ([`crate::nip44::PayloadError`]),
    /// `rumor_utf8`, `rumor_json`, `rumor_id`, `rumor_kind`, `rumor_author`,
    /// `rumor_recipient`, `rumor_time`, `group_context`, `inner_signature`,
    /// `inner_kind`, `inner_author`, `inner_group`, `dm_in_group_report`.
    Unverified(String),
}

/// A message as its author signed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The id of the message: the rumor's of a DM (the same on every copy),
    /// the inner event's of a group message. Computed here, never taken
    /// from the report.
    pub id: String,
    /// Whose key wrote it: the accused's, or the reporter's for a message
    /// of theirs attached as context.
    pub author: String,
    /// 14 for a DM, 9 for a group message.
    pub kind: u16,
    /// When the author dated it, unix seconds.
    pub created_at: i64,
    /// The content as signed: the application envelope
    /// (`{"v":1,"t":"text",…}`), or plain text from a client that writes
    /// no envelope.
    pub content: String,
    /// The tags as signed.
    pub tags: Vec<Vec<String>>,
    /// The attachment the envelope carries (`t = media`): the descriptor
    /// with its servers, chunks and key, so that a moderator can fetch and
    /// open the file. Empty for anything else.
    pub media_json: Vec<serde_json::Value>,
}

/// Check a report at the server's time `now` (unix seconds). Never fails
/// and never panics: whatever does not hold is told in the result.
///
/// The size of the report as a whole is the caller's to bound before it is
/// parsed; here each message is read only up to [`MAX_CONTENT_BYTES`].
pub fn verify(report: &Report, now: i64) -> Verified {
    let refused = |why: &str| Verified {
        refused: Some(why.to_string()),
        messages: Vec::new(),
    };
    if report.v != REPORT_VERSION {
        return refused("version");
    }
    let Some(reporter) = canonical_key(&report.reporter) else {
        return refused("reporter_key");
    };
    let Some(accused) = canonical_key(&report.accused) else {
        return refused("accused_key");
    };
    if reporter == accused {
        return refused("self_report");
    }
    if report.messages.len() > MAX_MESSAGES {
        return refused("too_many_messages");
    }
    if report
        .comment
        .as_deref()
        .is_some_and(|c| c.chars().count() > MAX_COMMENT_CHARS)
    {
        return refused("comment_too_long");
    }
    if report
        .sub_reason
        .as_deref()
        .is_some_and(|s| s.chars().count() > MAX_SUB_REASON_CHARS)
    {
        return refused("sub_reason_too_long");
    }
    if report.messages.iter().any(|m| {
        m.note
            .as_deref()
            .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
    }) {
        return refused("note_too_long");
    }
    if let Context::Group { group_id } = &report.context {
        if group_id.is_empty() || group_id.chars().count() > MAX_GROUP_ID_CHARS {
            return refused("group_id");
        }
    }
    // A report on a group is made in it.
    if report.target == Target::Group && !matches!(report.context, Context::Group { .. }) {
        return refused("target_context");
    }
    if report
        .group_name
        .as_deref()
        .is_some_and(|n| n.chars().count() > MAX_GROUP_NAME_CHARS)
    {
        return refused("group_name_too_long");
    }
    if report.group_admins.len() > MAX_GROUP_ADMINS
        || report
            .group_admins
            .iter()
            .any(|k| canonical_key(k).is_none())
    {
        return refused("group_admins");
    }
    if report.message_id.as_deref().is_some_and(|id| {
        id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return refused("message_id");
    }
    if now
        .checked_sub(report.created_at)
        .is_none_or(|age| !(-REPORT_FUTURE_WINDOW_SECS..=REPORT_PAST_WINDOW_SECS).contains(&age))
    {
        return refused("created_at");
    }

    let parties = Parties { reporter, accused };
    let mut seen = HashSet::new();
    let messages = report
        .messages
        .iter()
        .map(
            |attached| match check(attached, &parties, &report.context, report.target) {
                Ok(message) if !seen.insert(message.id.clone()) => {
                    Outcome::Unverified("duplicate".into())
                }
                Ok(message) => Outcome::Verified(message),
                Err(why) => Outcome::Unverified(why.to_string()),
            },
        )
        .collect();
    Verified {
        refused: None,
        messages,
    }
}

/// The two keys a report is between.
struct Parties {
    reporter: PublicKey,
    accused: PublicKey,
}

impl Parties {
    /// The other one of the two, when `author` is one of them.
    fn other(&self, author: &PublicKey) -> Option<PublicKey> {
        if *author == self.accused {
            Some(self.reporter)
        } else if *author == self.reporter {
            Some(self.accused)
        } else {
            None
        }
    }
}

/// A key in the one spelling a report may use, so that two spellings of
/// one key are never taken for two people.
fn canonical_key(hex: &str) -> Option<PublicKey> {
    let key = PublicKey::from_hex(hex).ok()?;
    (key.to_hex() == hex).then_some(key)
}

fn check(
    attached: &Attached,
    parties: &Parties,
    context: &Context,
    target: Target,
) -> Result<Message, &'static str> {
    match &attached.proof {
        None => Err("no_proof"),
        // A report on a group rests on what was written in the group: a
        // chat of two is no evidence about it.
        Some(Proof::Dm { .. }) if target == Target::Group => Err("dm_in_group_report"),
        // A DM counts in any context: who harasses in a group may go on in
        // a chat of two, and the report is on the person.
        Some(Proof::Dm { seal, keys }) => check_dm(seal, keys, parties),
        Some(Proof::Group { inner }) => match context {
            Context::Group { group_id } => check_group(inner, group_id, parties, target),
            _ => Err("group_context"),
        },
    }
}

fn check_dm(seal: &Event, keys: &MessageKeys, parties: &Parties) -> Result<Message, &'static str> {
    if seal.content.len() > MAX_CONTENT_BYTES {
        return Err("too_large");
    }
    if seal.kind.as_u16() != KIND_SEAL {
        return Err("seal_kind");
    }
    // The id and the signature: the author's key signed exactly this
    // ciphertext, and so the rumor inside it.
    seal.verify().map_err(|_| "seal_signature")?;
    let other = parties.other(&seal.pubkey).ok_or("seal_author")?;

    let plain = Payload::parse(&seal.content)
        .and_then(|p| p.open(keys))
        .map_err(|e| e.code())?;
    let rumor = read_rumor(&plain)?;
    // The seal's signer vouches for the rumor only as its own: NIP-59 lets
    // nobody seal in somebody else's name.
    if rumor.pubkey != seal.pubkey {
        return Err("rumor_author");
    }
    if !DM_KINDS.contains(&rumor.kind.as_u16()) {
        return Err("rumor_kind");
    }
    // Written to the other one of the two: a message the accused sent to
    // somebody else, handed over by that somebody, is not the reporter's
    // to report as a message to them.
    let to_other = rumor.tags.iter().any(|t| {
        t.as_slice().first().map(String::as_str) == Some("p")
            && t.as_slice().get(1) == Some(&other.to_hex())
    });
    if !to_other {
        return Err("rumor_recipient");
    }
    let rumor_at = secs(rumor.created_at);
    let seal_at = secs(seal.created_at);
    if rumor_at
        .checked_sub(seal_at)
        .is_none_or(|d| d.unsigned_abs() > (SEAL_TWEAK_SECS + SEAL_TIME_SLACK_SECS).unsigned_abs())
    {
        return Err("rumor_time");
    }
    Ok(Message {
        id: rumor.compute_id().to_hex(),
        author: rumor.pubkey.to_hex(),
        kind: rumor.kind.as_u16(),
        created_at: rumor_at,
        media_json: media_of(&rumor.content),
        tags: rumor.tags.iter().map(|t| t.as_slice().to_vec()).collect(),
        content: rumor.content,
    })
}

/// The rumor out of the seal's plaintext. A rumor may carry its id; a
/// wrong one is a sign of tampering by whoever made the seal, and is
/// refused rather than ignored.
fn read_rumor(plain: &[u8]) -> Result<UnsignedEvent, &'static str> {
    let text = std::str::from_utf8(plain).map_err(|_| "rumor_utf8")?;
    let rumor: UnsignedEvent = serde_json::from_str(text).map_err(|_| "rumor_json")?;
    rumor.verify_id().map_err(|_| "rumor_id")?;
    Ok(rumor)
}

fn check_group(
    inner: &Event,
    group_id: &str,
    parties: &Parties,
    target: Target,
) -> Result<Message, &'static str> {
    if inner.content.len() > MAX_CONTENT_BYTES {
        return Err("too_large");
    }
    if inner.kind != Kind::from(KIND_GROUP_MESSAGE) {
        return Err("inner_kind");
    }
    inner.verify().map_err(|_| "inner_signature")?;
    // On a person: their messages and the reporter's. On a group: any
    // member's; the signature still names who wrote each.
    if target == Target::Person {
        parties.other(&inner.pubkey).ok_or("inner_author")?;
    }
    // The group is the first `h`, as the reader of the app takes it
    // (`messenger_groups::wire::verify_message`).
    let h = inner
        .tags
        .iter()
        .find(|t| t.kind() == "h")
        .and_then(|t| t.as_slice().get(1));
    if h.map(String::as_str) != Some(group_id) {
        return Err("inner_group");
    }
    Ok(Message {
        id: inner.id.to_hex(),
        author: inner.pubkey.to_hex(),
        kind: KIND_GROUP_MESSAGE,
        created_at: secs(inner.created_at),
        content: inner.content.clone(),
        tags: inner.tags.iter().map(|t| t.as_slice().to_vec()).collect(),
        media_json: media_of(&inner.content),
    })
}

/// The descriptor of the attachment an envelope is (`t = media`,
/// internal/messenger-wire.md §3, «Вложение»), the fields only a sender's
/// device writes taken out. Content that is no envelope has none.
fn media_of(content: &str) -> Vec<serde_json::Value> {
    let Ok(serde_json::Value::Object(mut fields)) =
        serde_json::from_str::<serde_json::Value>(content)
    else {
        return Vec::new();
    };
    if fields.get("t").and_then(|t| t.as_str()) != Some("media") {
        return Vec::new();
    }
    for name in LOCAL_MEDIA_FIELDS {
        fields.remove(name);
    }
    vec![serde_json::Value::Object(fields)]
}

/// A time of an event as the rest of the messenger counts it. A time past
/// `i64` is nobody's real one; it is clamped rather than wrapped, so that
/// no difference of two of them overflows into a pass.
fn secs(at: nostr::types::Timestamp) -> i64 {
    i64::try_from(at.as_secs()).unwrap_or(i64::MAX)
}

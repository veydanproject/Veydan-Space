// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The journal of calls between my devices (internal/messenger-wire.md §3,
//! «Свои устройства»). The signals of a call live a minute (the
//! invitation) or five (the rest) on the relays: a device of mine that was
//! away longer hears nothing of the call. So every device that closes the
//! record of a call between two, or learns more of it, tells my other
//! devices what it has (`own.call`, kept by the relays without a term);
//! and a call taken out of the journal goes out of it on all of them
//! (`own.call_hide`). The notes are what is read here, written and read
//! back; what a note does to the record is the service's.

use crate::call::{Direction, Outcome};
use crate::engine::Media;
use crate::signal::is_call_id;
use messenger_core::envelope::{T_OWN_CALL, T_OWN_CALL_HIDE};
use messenger_core::{Envelope, PubKey};
use messenger_store::calls::{self as repo, CallRow};
use messenger_store::chats;
use serde_json::Value;

/// The most calls one `own.call_hide` names; a longer list is cut.
pub const HIDE_MAX: usize = 500;

/// A call as my other device has it on record, once over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallNote {
    pub call_id: String,
    pub peer: PubKey,
    /// The private server of the chat; `None`: the home chat.
    pub server: Option<String>,
    pub direction: Direction,
    pub media: Media,
    pub started_at: i64,
    pub answered_at: Option<i64>,
    pub ended_at: i64,
    pub outcome: Outcome,
    pub via_relay: bool,
}

impl CallNote {
    /// The note of a record that is over; `None` while it is under way, or
    /// for a record of another shape (a group call).
    pub fn of(row: &CallRow) -> Option<Self> {
        let (peer, server) = chats::parse_dm_chat_id(&row.chat_id)?;
        let server = server.map(String::from);
        if peer != row.peer {
            return None;
        }
        Some(Self {
            call_id: row.call_id.clone(),
            peer: PubKey::parse(&row.peer)?,
            server,
            direction: if row.direction == repo::DIR_OUT { Direction::Out } else { Direction::In },
            media: Media::parse(&row.media)?,
            started_at: row.started_at,
            answered_at: row.answered_at,
            ended_at: row.ended_at.unwrap_or(row.started_at),
            outcome: Outcome::parse(row.outcome.as_deref()?)?,
            via_relay: row.via_relay,
        })
    }

    /// Whether the note is of this record: the same peer, chat and
    /// direction. A note of the id of another record changes nothing of it.
    pub fn is_of(&self, row: &CallRow) -> bool {
        let Some((peer, server)) = chats::parse_dm_chat_id(&row.chat_id) else { return false };
        peer == row.peer && row.peer == self.peer.as_hex() && server == self.server.as_deref() && row.direction == self.direction.as_str()
    }

    pub fn to_envelope(&self) -> Envelope {
        let mut e = Envelope::new(T_OWN_CALL)
            .with("call_id", self.call_id.as_str())
            .with("peer", self.peer.as_hex())
            .with("direction", self.direction.as_str())
            .with("media", self.media.as_str())
            .with("started_at", self.started_at)
            .with("ended_at", self.ended_at)
            .with("outcome", self.outcome.as_str())
            .with("via_relay", self.via_relay);
        if let Some(srv) = &self.server {
            e = e.with("srv", srv.as_str());
        }
        if let Some(at) = self.answered_at {
            e = e.with("answered_at", at);
        }
        e
    }

    /// Read an `own.call`; `None` for anything not well formed (a word of
    /// a newer client this one does not know included).
    pub fn parse(e: &Envelope) -> Option<Self> {
        if e.t != T_OWN_CALL {
            return None;
        }
        let time = |key: &str| e.fields.get(key).and_then(Value::as_i64).filter(|t| *t >= 0);
        let started_at = time("started_at")?;
        let answered_at = match e.fields.get("answered_at") {
            None | Some(Value::Null) => None,
            Some(_) => Some(time("answered_at")?),
        };
        let server = match e.fields.get("srv") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= 128 => Some(s.clone()),
            Some(_) => return None,
        };
        Some(Self {
            call_id: e.str_field("call_id").filter(|id| is_call_id(id))?.to_string(),
            peer: PubKey::parse(e.str_field("peer")?)?,
            server,
            direction: match e.str_field("direction")? {
                repo::DIR_IN => Direction::In,
                repo::DIR_OUT => Direction::Out,
                _ => return None,
            },
            media: Media::parse(e.str_field("media")?)?,
            started_at,
            answered_at,
            ended_at: time("ended_at")?.max(started_at),
            outcome: Outcome::parse(e.str_field("outcome")?)?,
            via_relay: e.fields.get("via_relay").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// Calls taken out of the journal on my other device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HideNote {
    Calls(Vec<String>),
    /// Every call that ended at or before this time.
    AllBefore(i64),
}

impl HideNote {
    pub fn to_envelope(&self) -> Envelope {
        match self {
            Self::Calls(ids) => Envelope::new(T_OWN_CALL_HIDE).with("call_ids", ids.clone()),
            Self::AllBefore(at) => Envelope::new(T_OWN_CALL_HIDE).with("all_before", *at),
        }
    }

    pub fn parse(e: &Envelope) -> Option<Self> {
        if e.t != T_OWN_CALL_HIDE {
            return None;
        }
        if let Some(at) = e.fields.get("all_before").and_then(Value::as_i64) {
            return (at > 0).then_some(Self::AllBefore(at));
        }
        let ids: Vec<String> = e
            .fields
            .get("call_ids")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| is_call_id(id))
            .take(HIDE_MAX)
            .map(String::from)
            .collect();
        (!ids.is_empty()).then_some(Self::Calls(ids))
    }
}

/// How much an outcome says of a call: a device that rang and was not
/// taken writes "missed" while another one took the call or refused it.
pub fn weight(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Missed => 0,
        Outcome::Failed => 1,
        Outcome::Ended | Outcome::Declined | Outcome::Busy => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::new_call_id;

    fn note() -> CallNote {
        CallNote {
            call_id: new_call_id(),
            peer: PubKey::parse(&"b".repeat(64)).unwrap(),
            server: None,
            direction: Direction::Out,
            media: Media::Video,
            started_at: 100,
            answered_at: Some(105),
            ended_at: 165,
            outcome: Outcome::Ended,
            via_relay: true,
        }
    }

    #[test]
    fn a_note_survives_the_wire() {
        for n in [note(), CallNote { server: Some("s1".into()), answered_at: None, outcome: Outcome::Missed, via_relay: false, ..note() }] {
            let wire = n.to_envelope().encode();
            assert_eq!(CallNote::parse(&Envelope::parse(&wire).unwrap()), Some(n), "{wire}");
        }
        for h in [HideNote::Calls(vec![new_call_id(), new_call_id()]), HideNote::AllBefore(1_760_000_000)] {
            let wire = h.to_envelope().encode();
            assert_eq!(HideNote::parse(&Envelope::parse(&wire).unwrap()), Some(h), "{wire}");
        }
    }

    #[test]
    fn a_note_of_another_shape_is_not_read() {
        let good = note().to_envelope();
        let without = |key: &str| {
            let mut e = good.clone();
            e.fields.remove(key);
            e
        };
        for key in ["call_id", "peer", "direction", "media", "started_at", "ended_at", "outcome"] {
            assert_eq!(CallNote::parse(&without(key)), None, "{key}");
        }
        assert_eq!(CallNote::parse(&good.clone().with("outcome", "vanished")), None, "an outcome of a newer client");
        assert_eq!(CallNote::parse(&good.clone().with("call_id", "short")), None);
        assert_eq!(CallNote::parse(&good.clone().with("direction", "sideways")), None);
        assert_eq!(CallNote::parse(&good.clone().with("answered_at", "soon")), None);
        assert_eq!(CallNote::parse(&good.clone().with("srv", 7)), None);
        assert_eq!(CallNote::parse(&good.clone().with("ended_at", 50)).unwrap().ended_at, 100, "no end before the start");
        assert_eq!(CallNote::parse(&Envelope::new("own.calls")), None);

        assert_eq!(HideNote::parse(&Envelope::new(T_OWN_CALL_HIDE)), None);
        assert_eq!(HideNote::parse(&Envelope::new(T_OWN_CALL_HIDE).with("call_ids", vec!["nope"])), None);
        assert_eq!(HideNote::parse(&Envelope::new(T_OWN_CALL_HIDE).with("all_before", 0)), None);
        let many: Vec<String> = (0..HIDE_MAX + 5).map(|_| new_call_id()).collect();
        let Some(HideNote::Calls(ids)) = HideNote::parse(&HideNote::Calls(many).to_envelope()) else { panic!() };
        assert_eq!(ids.len(), HIDE_MAX);
    }

    #[test]
    fn a_record_under_way_or_of_a_group_makes_no_note() {
        let peer = "b".repeat(64);
        let row = CallRow {
            call_id: new_call_id(),
            chat_id: chats::dm_chat_id(&peer, None),
            peer: peer.clone(),
            direction: repo::DIR_IN.into(),
            media: repo::MEDIA_AUDIO.into(),
            started_at: 100,
            answered_at: None,
            ended_at: Some(145),
            outcome: Some(repo::OUTCOME_MISSED.into()),
            via_relay: false,
        };
        let n = CallNote::of(&row).unwrap();
        assert_eq!((n.direction, n.outcome, n.ended_at, n.server), (Direction::In, Outcome::Missed, 145, None));
        assert_eq!(CallNote::of(&CallRow { outcome: None, ..row.clone() }), None);
        assert_eq!(CallNote::of(&CallRow { chat_id: format!("group:{peer}"), ..row.clone() }), None);
        let private = CallNote::of(&CallRow { chat_id: chats::dm_chat_id(&peer, Some("s1")), ..row }).unwrap();
        assert_eq!(private.server.as_deref(), Some("s1"));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The control channel of a room (`ctl`, services/call/spec/protocol.md):
//! what the node says in text, what the client answers, and the binary
//! frames the participants pass each other through the node, which
//! puts the sender's seat in front of each and reads none of it.
//!
//! The types are spelled here rather than taken from `vcall-proto`: the
//! messenger takes no crate from `services/` (scripts/boundaries.sh).

use serde::{Deserialize, Serialize};

/// The sender's seat in front of a relayed frame, big-endian.
pub const FROM_LEN: usize = 4;

/// Which participant's media comes on an m-line, and of what kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: u32,
    /// `audio` or `video`.
    pub kind: String,
    pub mid: String,
}

/// A text frame of the channel. The node sends all but `answer`; the
/// client answers an `offer` with an `answer` of the same `seq`. A `t`
/// this version does not know is dropped unread ([`Message::parse`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Message {
    /// The first message: my seat, and who is in the room.
    Hello { you: u32, participants: Vec<u32> },
    Joined { id: u32 },
    Left { id: u32 },
    /// New m-lines, each sending one participant's stream. An offer
    /// unanswered for a while comes again with the same `seq` and SDP.
    Offer { seq: u32, sdp: String, tracks: Vec<Track> },
    Answer { seq: u32, sdp: String },
    /// Who is speaking now, the loudest first, by the node's audio
    /// levels (a node that counts them; one that does not never says
    /// it). With more participants than the node forwards the sound of,
    /// only theirs comes.
    Speaking { participants: Vec<u32> },
}

impl Message {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("a control message is plain data")
    }
}

/// A relayed frame taken apart: the sender's seat and its bytes.
pub fn from_relayed(frame: &[u8]) -> Option<(u32, &[u8])> {
    let head: [u8; 4] = frame.get(..FROM_LEN)?.try_into().ok()?;
    Some((u32::from_be_bytes(head), &frame[FROM_LEN..]))
}

/// A frame as the node hands it to the others: the seat in front.
pub fn relayed(from: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FROM_LEN + data.len());
    out.extend_from_slice(&from.to_be_bytes());
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_of_the_node_are_read_and_the_answer_written() {
        let hello = Message::parse(r#"{"t":"hello","you":1,"participants":[2,3]}"#).unwrap();
        assert_eq!(hello, Message::Hello { you: 1, participants: vec![2, 3] });
        let offer = Message::parse(r#"{"t":"offer","seq":1,"sdp":"v=0","tracks":[{"id":2,"kind":"audio","mid":"3"}]}"#).unwrap();
        assert_eq!(offer, Message::Offer { seq: 1, sdp: "v=0".into(), tracks: vec![Track { id: 2, kind: "audio".into(), mid: "3".into() }] });
        assert_eq!(Message::Answer { seq: 1, sdp: "v=1".into() }.encode(), r#"{"t":"answer","seq":1,"sdp":"v=1"}"#);
        assert_eq!(Message::parse(r#"{"t":"speaking","participants":[2]}"#), Some(Message::Speaking { participants: vec![2] }));
        assert_eq!(Message::parse(r#"{"t":"wave","id":2}"#), None, "a word of a newer node");
        assert_eq!(Message::parse("not json"), None);
        assert_eq!(from_relayed(&relayed(7, b"bytes")), Some((7, &b"bytes"[..])));
        assert_eq!(from_relayed(b"abc"), None);
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The signalling of a call as it travels inside NIP-17
//! (internal/messenger-wire.md §10): the envelope types `call.*` of
//! `messenger-core` read into one typed [`Signal`], and written back.
//!
//! The invitation goes as a message (kind 14, it wakes the peer's phone);
//! everything else as a note (kind 30079, `wrap_note_as`): quiet, but for
//! the ones that end a ringing (`["call", "0"]` outside; `service::send`),
//! and gone from the relays after `expiration`. Every piece carries the `call_id`,
//! 16 random bytes as hex, made by the caller.

use crate::engine::{IceCandidate, Media};
use messenger_core::envelope::{
    T_CALL_ANSWER, T_CALL_BUSY, T_CALL_DECLINE, T_CALL_END, T_CALL_ICE, T_CALL_INVITE, T_CALL_RESTART, T_CALL_VIDEO,
};
use messenger_core::Envelope;
use serde_json::Value;

/// How long an invitation is good for, by the time inside the rumor: an
/// older one is a missed call, not a ringing phone.
pub const INVITE_TTL_SECS: i64 = 45;
/// `expiration` (NIP-40) on the wraps of an invitation: a relay keeps it
/// no longer than a phone could ring for it.
pub const INVITE_EXPIRATION_SECS: i64 = 60;
/// `expiration` on the wraps of every other piece.
pub const NOTE_EXPIRATION_SECS: i64 = 300;

/// Why a call was declined or ended, as the words go on the wire.
pub mod reason {
    pub const DECLINED: &str = "declined";
    pub const ENDED: &str = "ended";
    pub const FAILED: &str = "failed";
    /// Nobody answered in time (the caller gave up).
    pub const TIMEOUT: &str = "timeout";
    /// The caller took another device's answer; this end is for the
    /// device whose answer it names.
    pub const ANSWERED_ELSEWHERE: &str = "answered_elsewhere";
    /// We both called at once and this call lost: it never was. Whoever
    /// rings for it stops, whoever has it on record forgets it.
    pub const SUPERSEDED: &str = "superseded";
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// `live_restart`: the sender takes a restart of ICE while the way
    /// still works, staying on the call through it (5.1.3+). A client
    /// without the word (5.1.2) moves to `connecting` on such a restart
    /// and gives up when no `Connected` comes, so the peer makes none
    /// while its way is live. Said in the first invitation and in every
    /// answer; a restart offer carries no word of it.
    Invite { call_id: String, media: Media, sdp: String, ice: Vec<IceCandidate>, restart: bool, live_restart: bool },
    Answer { call_id: String, sdp: String, ice: Vec<IceCandidate>, live_restart: bool },
    Ice { call_id: String, ice: Vec<IceCandidate> },
    Decline { call_id: String, reason: String },
    End { call_id: String, reason: String, answer: Option<String> },
    Busy { call_id: String },
    /// The called side asks the caller for a new offer (ICE restart).
    /// `seen` is how many offers of the caller it has taken so far: the
    /// caller drops a request made before its latest offer reached the
    /// called side (that offer answers it). `None` from a client that
    /// does not count (5.1.2), always honoured.
    Restart { call_id: String, seen: Option<u32> },
    /// My video (camera or screen) went on or off within the call.
    Video { call_id: String, on: bool },
}

impl Signal {
    pub fn call_id(&self) -> &str {
        match self {
            Self::Invite { call_id, .. }
            | Self::Answer { call_id, .. }
            | Self::Ice { call_id, .. }
            | Self::Decline { call_id, .. }
            | Self::End { call_id, .. }
            | Self::Busy { call_id }
            | Self::Restart { call_id, .. }
            | Self::Video { call_id, .. } => call_id,
        }
    }

    /// Read a `call.*` envelope. `None` for anything that is not one, or
    /// not well formed: a call id of another shape, an SDP that is empty,
    /// a media word this version does not know.
    pub fn parse(e: &Envelope) -> Option<Self> {
        let call_id = e.str_field("call_id").filter(|id| is_call_id(id))?.to_string();
        let ice = || candidates(e.fields.get("ice"));
        let sdp = || e.str_field("sdp").filter(|s| !s.is_empty() && s.len() <= MAX_SDP_BYTES).map(String::from);
        Some(match e.t.as_str() {
            T_CALL_INVITE => Signal::Invite {
                call_id,
                media: Media::parse(e.str_field("media")?)?,
                sdp: sdp()?,
                ice: ice(),
                restart: e.fields.get("restart").and_then(Value::as_bool).unwrap_or(false),
                live_restart: flag(e, "live_restart"),
            },
            T_CALL_ANSWER => Signal::Answer { call_id, sdp: sdp()?, ice: ice(), live_restart: flag(e, "live_restart") },
            T_CALL_ICE => Signal::Ice { call_id, ice: ice() },
            T_CALL_DECLINE => Signal::Decline { call_id, reason: word(e.str_field("reason"), reason::DECLINED) },
            T_CALL_END => Signal::End {
                call_id,
                reason: word(e.str_field("reason"), reason::ENDED),
                answer: e.str_field("answer").filter(|a| is_hex(a, 64)).map(String::from),
            },
            T_CALL_BUSY => Signal::Busy { call_id },
            T_CALL_RESTART => Signal::Restart {
                call_id,
                seen: e.fields.get("seen").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
            },
            T_CALL_VIDEO => Signal::Video { call_id, on: e.fields.get("on").and_then(Value::as_bool)? },
            _ => return None,
        })
    }

    pub fn to_envelope(&self) -> Envelope {
        match self {
            Self::Invite { call_id, media, sdp, ice, restart, live_restart } => {
                with_flag(Envelope::call_invite(call_id, media.as_str(), sdp, candidates_json(ice), *restart), "live_restart", *live_restart)
            }
            Self::Answer { call_id, sdp, ice, live_restart } => {
                with_flag(Envelope::call_answer(call_id, sdp, candidates_json(ice)), "live_restart", *live_restart)
            }
            Self::Ice { call_id, ice } => Envelope::call_ice(call_id, candidates_json(ice)),
            Self::Decline { call_id, reason } => Envelope::call_decline(call_id, reason),
            Self::End { call_id, reason, answer } => Envelope::call_end(call_id, reason, answer.as_deref()),
            Self::Busy { call_id } => Envelope::call_busy(call_id),
            Self::Restart { call_id, seen } => match seen {
                Some(n) => Envelope::call_restart(call_id).with("seen", *n),
                None => Envelope::call_restart(call_id),
            },
            Self::Video { call_id, on } => Envelope::call_video(call_id, *on),
        }
    }
}

/// An SDP of a call is a few kilobytes; a bigger one is not an SDP.
pub const MAX_SDP_BYTES: usize = 64 * 1024;
/// Candidates in one piece, at most; the rest are dropped unread.
const MAX_CANDIDATES: usize = 64;

/// A new call id: 16 random bytes, hex.
pub fn new_call_id() -> String {
    let mut b = [0u8; 16];
    // A failure of the system's randomness is not something to call with.
    getrandom::fill(&mut b).expect("the system has random bytes");
    hex::encode(b)
}

pub fn is_call_id(s: &str) -> bool {
    is_hex(s, 32)
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// A reason is one word of the wire's own; anything else reads as the
/// default, so an unknown word of a newer client does not become text
/// on the screen.
fn word(s: Option<&str>, default: &str) -> String {
    match s {
        Some(w) if w.len() <= 32 && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') => w.to_string(),
        _ => default.to_string(),
    }
}

/// A boolean field that is `false` when absent (or not a boolean).
fn flag(e: &Envelope, key: &str) -> bool {
    e.fields.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A boolean field written only when it is `true`: absent means `false`,
/// as an older client, which never writes it, says nothing.
fn with_flag(e: Envelope, key: &str, on: bool) -> Envelope {
    if on {
        e.with(key, true)
    } else {
        e
    }
}

fn candidates(v: Option<&Value>) -> Vec<IceCandidate> {
    let Some(Value::Array(items)) = v else { return Vec::new() };
    items
        .iter()
        .take(MAX_CANDIDATES)
        .filter_map(|item| serde_json::from_value::<IceCandidate>(item.clone()).ok())
        .filter(|c| !c.candidate.is_empty() && c.candidate.len() <= 512)
        .collect()
}

fn candidates_json(ice: &[IceCandidate]) -> Vec<Value> {
    ice.iter().map(|c| serde_json::to_value(c).expect("a candidate is plain data")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(n: u16) -> IceCandidate {
        IceCandidate { candidate: format!("candidate:{n} 1 udp 1 203.0.113.7 500{n} typ host"), mid: Some("0".into()), index: Some(0) }
    }

    #[test]
    fn every_signal_survives_the_wire() {
        let id = new_call_id();
        assert!(is_call_id(&id));
        let all = vec![
            Signal::Invite { call_id: id.clone(), media: Media::Video, sdp: "v=0".into(), ice: vec![cand(1), cand(2)], restart: false, live_restart: true },
            Signal::Invite { call_id: id.clone(), media: Media::Audio, sdp: "v=0".into(), ice: vec![], restart: true, live_restart: false },
            Signal::Answer { call_id: id.clone(), sdp: "v=1".into(), ice: vec![cand(3)], live_restart: true },
            Signal::Answer { call_id: id.clone(), sdp: "v=1".into(), ice: vec![], live_restart: false },
            Signal::Ice { call_id: id.clone(), ice: vec![cand(4)] },
            Signal::Decline { call_id: id.clone(), reason: reason::DECLINED.into() },
            Signal::End { call_id: id.clone(), reason: reason::ANSWERED_ELSEWHERE.into(), answer: Some("ab".repeat(32)) },
            Signal::End { call_id: id.clone(), reason: reason::ENDED.into(), answer: None },
            Signal::Busy { call_id: id.clone() },
            Signal::Restart { call_id: id.clone(), seen: None },
            Signal::Restart { call_id: id.clone(), seen: Some(2) },
            Signal::End { call_id: id.clone(), reason: reason::SUPERSEDED.into(), answer: None },
            Signal::Video { call_id: id.clone(), on: true },
            Signal::Video { call_id: id.clone(), on: false },
        ];
        for s in all {
            let wire = s.to_envelope().encode();
            let back = Signal::parse(&Envelope::parse(&wire).unwrap()).unwrap();
            assert_eq!(back, s, "{wire}");
            assert_eq!(back.call_id(), id);
        }
    }

    #[test]
    fn what_is_not_a_call_is_not_read() {
        let id = new_call_id();
        assert!(Signal::parse(&Envelope::text("hi")).is_none());
        assert!(Signal::parse(&Envelope::new("call.invite")).is_none(), "no id");
        assert!(Signal::parse(&Envelope::call_invite("short", "audio", "v=0", vec![], false)).is_none());
        assert!(Signal::parse(&Envelope::call_invite(&id.to_uppercase(), "audio", "v=0", vec![], false)).is_none());
        assert!(Signal::parse(&Envelope::call_invite(&id, "hologram", "v=0", vec![], false)).is_none());
        assert!(Signal::parse(&Envelope::call_invite(&id, "audio", "", vec![], false)).is_none(), "an empty offer");
        assert!(Signal::parse(&Envelope::call_answer(&id, &"x".repeat(MAX_SDP_BYTES + 1), vec![])).is_none());
        // An older client says nothing of live restarts: read as none;
        // and the word is written only when it is said.
        let plain = Signal::parse(&Envelope::call_answer(&id, "v=1", vec![])).unwrap();
        assert_eq!(plain, Signal::Answer { call_id: id.clone(), sdp: "v=1".into(), ice: vec![], live_restart: false });
        assert!(!plain.to_envelope().encode().contains("live_restart"));
        let said = Signal::Answer { call_id: id.clone(), sdp: "v=1".into(), ice: vec![], live_restart: true }.to_envelope();
        assert_eq!(said.fields.get("live_restart"), Some(&Value::Bool(true)));
        assert!(Signal::parse(&Envelope::new("call.wave").with("call_id", id.as_str())).is_none(), "a type of a newer client");
        assert!(Signal::parse(&Envelope::new("call.video").with("call_id", id.as_str())).is_none(), "video without on or off");
        assert!(Signal::parse(&Envelope::new("call.video").with("call_id", id.as_str()).with("on", "yes")).is_none());

        // Candidates that are not candidates are left out; the rest stay.
        let e = Envelope::call_ice(&id, vec![serde_json::json!({ "candidate": "" }), serde_json::json!(7), serde_json::json!({ "candidate": "c" })]);
        assert_eq!(Signal::parse(&e), Some(Signal::Ice { call_id: id.clone(), ice: vec![IceCandidate { candidate: "c".into(), mid: None, index: None }] }));

        // A reason of another shape reads as the default; so does a bad answer id.
        let e = Envelope::call_end(&id, "Hacked <b>", Some("nope"));
        assert_eq!(Signal::parse(&e), Some(Signal::End { call_id: id.clone(), reason: reason::ENDED.into(), answer: None }));
        let e = Envelope::new("call.decline").with("call_id", id.as_str());
        assert_eq!(Signal::parse(&e), Some(Signal::Decline { call_id: id, reason: reason::DECLINED.into() }));
    }
}

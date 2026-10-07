// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What arrived in the push, as VPush sends it
//! (services/push/spec/protocol.md, payload version 2). Everything here came
//! over the network and is checked before it is used.

use messenger_core::{MessengerError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

pub const PAYLOAD_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushKind {
    Dm,
    Group,
}

#[derive(Clone, Debug)]
pub struct PushData {
    pub kind: PushKind,
    /// The outer event as JSON, when the push could carry it.
    pub event: Option<String>,
    pub event_id: Option<String>,
    pub relay: Option<String>,
    pub group_id: Option<String>,
    /// How many events this push stands for; more than one when the
    /// server held the tail of a burst back.
    pub count: u32,
    pub trace: String,
    /// The server says the event carries an invitation to a call (the
    /// outer tag of the wrap, which anyone may put there): the phone
    /// opens it even when it would say nothing of a message.
    pub call: bool,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    v: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    relay: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    count: Option<String>,
    #[serde(default)]
    trace: Option<String>,
    #[serde(default)]
    call: Option<String>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl PushData {
    /// The data map of the push, every value a string, as FCM hands it over.
    pub fn parse(data: &BTreeMap<String, String>) -> Result<Self> {
        let raw: Raw = serde_json::to_value(data)
            .and_then(serde_json::from_value)
            .map_err(|e| MessengerError::Invalid(format!("push: {e}")))?;
        let v: u32 = raw.v.as_deref().unwrap_or("1").parse().unwrap_or(0);
        if v != PAYLOAD_VERSION {
            return Err(MessengerError::Invalid(format!("push v{v}, this code knows v{PAYLOAD_VERSION}")));
        }
        let kind = match raw.kind.as_str() {
            "dm" => PushKind::Dm,
            "group" => PushKind::Group,
            other => return Err(MessengerError::Invalid(format!("push type {other} is not about a message"))),
        };
        let event_id = raw.event_id.filter(|s| hex64(s));
        let group_id = raw.group_id.filter(|s| hex64(s));
        if kind == PushKind::Group && group_id.is_none() {
            return Err(MessengerError::Invalid("group push without a group".into()));
        }
        let relay = raw.relay.filter(|r| r.starts_with("wss://") || r.starts_with("ws://"));
        let count = raw.count.and_then(|c| c.parse().ok()).filter(|c| (1..=9999).contains(c)).unwrap_or(1);
        let trace = raw
            .trace
            .filter(|t| !t.is_empty() && t.len() <= 64 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
            .unwrap_or_else(|| "-".into());
        let call = kind == PushKind::Dm && raw.call.as_deref() == Some("1");
        Ok(Self { kind, event: raw.event.filter(|e| !e.is_empty()), event_id, relay, group_id, count, trace, call })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_dm_push_with_the_event() {
        let p = PushData::parse(&map(&[("v", "2"), ("type", "dm"), ("event", "{}"), ("trace", "abc")])).unwrap();
        assert_eq!(p.kind, PushKind::Dm);
        assert_eq!(p.event.as_deref(), Some("{}"));
        assert_eq!(p.count, 1);
        assert_eq!(p.trace, "abc");
        assert!(!p.call);
    }

    /// The server marks the push of a call; on a direct message only.
    #[test]
    fn a_call_is_marked_on_a_dm_push() {
        assert!(PushData::parse(&map(&[("v", "2"), ("type", "dm"), ("call", "1")])).unwrap().call);
        assert!(!PushData::parse(&map(&[("v", "2"), ("type", "dm"), ("call", "yes")])).unwrap().call);
        let g = "ab".repeat(32);
        assert!(!PushData::parse(&map(&[("v", "2"), ("type", "group"), ("group_id", &g), ("call", "1")])).unwrap().call);
    }

    #[test]
    fn a_group_push_names_its_group_and_may_only_point_at_the_event() {
        let g = "ab".repeat(32);
        let p = PushData::parse(&map(&[
            ("v", "2"),
            ("type", "group"),
            ("group_id", &g),
            ("event_id", &"cd".repeat(32)),
            ("relay", "wss://relay.example"),
            ("count", "3"),
        ]))
        .unwrap();
        assert_eq!(p.kind, PushKind::Group);
        assert_eq!(p.group_id.as_deref(), Some(g.as_str()));
        assert!(p.event.is_none());
        assert!(p.event_id.is_some());
        assert_eq!(p.relay.as_deref(), Some("wss://relay.example"));
        assert_eq!(p.count, 3);
        assert!(PushData::parse(&map(&[("v", "2"), ("type", "group")])).is_err(), "no group");
    }

    #[test]
    fn what_does_not_look_right_is_dropped_or_refused() {
        assert!(PushData::parse(&map(&[("v", "1"), ("type", "dm")])).is_err(), "old version");
        assert!(PushData::parse(&map(&[("v", "2"), ("type", "test")])).is_err(), "not a message");
        let p = PushData::parse(&map(&[
            ("v", "2"),
            ("type", "dm"),
            ("event_id", "zz"),
            ("relay", "http://x"),
            ("count", "0"),
            ("trace", "bad trace!"),
        ]))
        .unwrap();
        assert!(p.event_id.is_none());
        assert!(p.relay.is_none());
        assert_eq!(p.count, 1);
        assert_eq!(p.trace, "-");
    }
}

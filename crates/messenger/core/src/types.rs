// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Plain identifier types. Deliberately independent of any Nostr crate so the
//! contract does not move when the transport implementation changes.

use serde::{Deserialize, Serialize};
use std::fmt;

/// 32-byte x-only public key, lowercase hex (64 chars).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PubKey(String);

impl PubKey {
    /// Accepts a 64-char lowercase/uppercase hex string; normalizes to lowercase.
    pub fn parse(hex: &str) -> Option<Self> {
        let s = hex.trim();
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self(s.to_ascii_lowercase()))
    }

    pub fn as_hex(&self) -> &str {
        &self.0
    }

    /// First 8 hex chars — enough for logs, never for identity decisions.
    pub fn short(&self) -> &str {
        &self.0[..8]
    }
}

impl fmt::Debug for PubKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PubKey({}…)", self.short())
    }
}

impl fmt::Display for PubKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// SHA-256 event id, lowercase hex (64 chars).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    pub fn parse(hex: &str) -> Option<Self> {
        let s = hex.trim();
        if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self(s.to_ascii_lowercase()))
    }

    pub fn as_hex(&self) -> &str {
        &self.0
    }

    pub fn short(&self) -> &str {
        &self.0[..8]
    }
}

impl fmt::Debug for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventId({}…)", self.short())
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `ws://` or `wss://` relay URL, host required, no path or query.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelayUrl(String);

impl RelayUrl {
    /// Structural validation only; no network. Trailing slash is stripped so
    /// `wss://r.example.com` and `wss://r.example.com/` are the same relay.
    pub fn parse(url: &str) -> Option<Self> {
        let s = url.trim().trim_end_matches('/');
        let rest = s
            .strip_prefix("wss://")
            .or_else(|| s.strip_prefix("ws://"))?;
        if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('#') {
            return None;
        }
        let (host, port) = match rest.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
            _ => (rest, None),
        };
        if host.is_empty() {
            return None;
        }
        if let Some(p) = port {
            let n: u32 = p.parse().ok()?;
            if !(1..=65535).contains(&n) {
                return None;
            }
        }
        Some(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unix seconds. All persisted times use this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn secs(self) -> i64 {
        self.0
    }
}

/// Subscription id assigned by the caller; opaque to transport.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubId(pub String);

/// Where a raw event came from. Handlers use it for scoping (private servers)
/// and for the historical/live decision; never for trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventSource {
    Relay { url: RelayUrl },
    Server { id: String },
    /// Fetched by a Negentropy reconciliation, i.e. history, not live.
    Sync { url: RelayUrl },
}

/// A signed Nostr event as received from the wire, before any validation.
///
/// Stage 0 keeps the event as JSON; stage 2 replaces `json` with the
/// transport crate's typed event behind the same field name.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawEvent {
    pub id: EventId,
    pub kind: u16,
    pub pubkey: PubKey,
    pub created_at: Timestamp,
    pub json: serde_json::Value,
    pub source: EventSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX64: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";

    #[test]
    fn pubkey_parses_and_normalizes() {
        let pk = PubKey::parse(&HEX64.to_uppercase()).unwrap();
        assert_eq!(pk.as_hex(), HEX64);
        assert_eq!(pk.short(), "3bf0c63f");
        assert!(PubKey::parse("abc").is_none());
        assert!(PubKey::parse(&"g".repeat(64)).is_none());
    }

    #[test]
    fn relay_url_accepts_only_bare_ws_hosts() {
        assert!(RelayUrl::parse("wss://relay.example.com").is_some());
        assert!(RelayUrl::parse("wss://relay.example.com/").is_some());
        assert!(RelayUrl::parse("ws://127.0.0.1:7777").is_some());
        assert!(RelayUrl::parse("wss://relay.example.com/path").is_none());
        assert!(RelayUrl::parse("wss://relay.example.com?x=1").is_none());
        assert!(RelayUrl::parse("https://relay.example.com").is_none());
        assert!(RelayUrl::parse("wss://").is_none());
        assert!(RelayUrl::parse("wss://host:0").is_none());
        assert!(RelayUrl::parse("wss://host:70000").is_none());
        assert_eq!(
            RelayUrl::parse("wss://relay.example.com/").unwrap().as_str(),
            "wss://relay.example.com"
        );
    }

    #[test]
    fn debug_never_prints_full_keys() {
        let pk = PubKey::parse(HEX64).unwrap();
        let s = format!("{pk:?}");
        assert!(s.contains("3bf0c63f"));
        assert!(!s.contains(HEX64));
    }
}

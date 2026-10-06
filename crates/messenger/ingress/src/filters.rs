// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Relay filters the runtime subscribes with. Kept as JSON (`core::Filter`)
//! so handlers and tests can inspect them without a protocol crate.

use messenger_core::outbound::Filter;
use messenger_core::presence::{KIND_PRESENCE, PRESENCE_D};
use messenger_core::{PubKey, Timestamp};

/// Gift wraps carry a `created_at` tweaked randomly up to two days into the
/// past (NIP-59, `nostr` uses the full 0..2d range), so a live subscription
/// must start two days before "now" or it misses wraps created moments ago.
/// Replays are harmless: `msg_events_raw` dedups them.
pub const DM_LIVE_MARGIN_SECS: i64 = 2 * 24 * 3600 + 600;

pub const SUB_DM_LIVE: &str = "dm-live";

/// Live inbox: every gift wrap addressed to me since `since`.
pub fn dm_inbox(me: &PubKey, since: Timestamp) -> Filter {
    Filter(serde_json::json!({
        "kinds": [1059],
        "#p": [me.as_hex()],
        "since": since.secs(),
    }))
}

/// History window for reconciliation or catch-up.
pub fn dm_inbox_window(me: &PubKey, since: Timestamp, until: Timestamp, limit: usize) -> Filter {
    Filter(serde_json::json!({
        "kinds": [1059],
        "#p": [me.as_hex()],
        "since": since.secs(),
        "until": until.secs(),
        "limit": limit,
    }))
}

pub const SUB_PROFILES: &str = "profiles";
pub const SUB_MY_FOLLOWS: &str = "my-follows";

/// Latest kind-0 of each author (relays return the newest replaceable).
pub fn profiles(authors: &[PubKey]) -> Filter {
    Filter(serde_json::json!({
        "kinds": [0],
        "authors": authors.iter().map(|p| p.as_hex()).collect::<Vec<_>>(),
    }))
}

/// One author's profile, for on-demand lookups.
pub fn profile_of(author: &PubKey) -> Filter {
    Filter(serde_json::json!({ "kinds": [0], "authors": [author.as_hex()], "limit": 1 }))
}

/// My own follow list (kind 3), newest only.
pub fn my_follows(me: &PubKey) -> Filter {
    Filter(serde_json::json!({ "kinds": [3], "authors": [me.as_hex()], "limit": 1 }))
}

pub const SUB_DM_RELAYS: &str = "dm-relays";
/// Scope name of the DM history cursor in `msg_sync_cursors`.
pub const CURSOR_DM_INBOX: &str = "dm-inbox";
/// Upper bound of one history catch-up request.
pub const DM_HISTORY_LIMIT: usize = 2000;

/// Inbox relay lists (kind 10050) of the given authors.
pub fn dm_relays(authors: &[PubKey]) -> Filter {
    Filter(serde_json::json!({
        "kinds": [10050],
        "authors": authors.iter().map(|p| p.as_hex()).collect::<Vec<_>>(),
    }))
}

/// History catch-up: gift wraps for me since `since` (wire time), newest
/// `DM_HISTORY_LIMIT` at most. Used by `Outbound::Sync`.
pub fn dm_history(me: &PubKey, since: Timestamp) -> Filter {
    Filter(serde_json::json!({
        "kinds": [1059],
        "#p": [me.as_hex()],
        "since": since.secs().max(0),
        "limit": DM_HISTORY_LIMIT,
    }))
}

pub const SUB_GROUPS: &str = "groups";
pub const CURSOR_GROUPS: &str = "groups";
/// Group events carry their real time; the margin covers clock drift.
pub const GROUP_LIVE_MARGIN_SECS: i64 = 600;

/// Events of the groups I am in (kind 9, tag `h`).
pub fn group_events(group_ids: &[String], since: Option<Timestamp>) -> Filter {
    let mut f = serde_json::json!({ "kinds": [9], "#h": group_ids });
    if let Some(s) = since {
        f["since"] = serde_json::json!(s.secs());
    }
    Filter(f)
}

pub const SUB_PRESENCE: &str = "presence";

/// Presence beats of the given presence keys (not user keys: a contact
/// beats from a key only its approved contacts know).
pub fn presence_of(authors: &[PubKey]) -> Filter {
    Filter(serde_json::json!({
        "kinds": [KIND_PRESENCE],
        "authors": authors.iter().map(|p| p.as_hex()).collect::<Vec<_>>(),
        "#d": [PRESENCE_D],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_filter_asks_for_our_beats_of_these_keys() {
        let a = PubKey::parse(&"ab".repeat(32)).unwrap();
        let b = PubKey::parse(&"cd".repeat(32)).unwrap();
        let f = presence_of(&[a.clone(), b.clone()]);
        assert_eq!(
            f.0,
            serde_json::json!({ "kinds": [30315], "authors": [a.as_hex(), b.as_hex()], "#d": ["veydan"] })
        );
        assert_eq!(SUB_PRESENCE, "presence");
    }

    #[test]
    fn dm_filters_target_gift_wraps_for_me() {
        let me = PubKey::parse(&"ab".repeat(32)).unwrap();
        let f = dm_inbox(&me, Timestamp(100));
        assert_eq!(f.0["kinds"], serde_json::json!([1059]));
        assert_eq!(f.0["#p"][0], me.as_hex());
        assert_eq!(f.0["since"], 100);
        let w = dm_inbox_window(&me, Timestamp(1), Timestamp(2), 50);
        assert_eq!(w.0["until"], 2);
        assert_eq!(w.0["limit"], 50);
    }
}


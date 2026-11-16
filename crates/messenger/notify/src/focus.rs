// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whether another device of mine is reading a chat now: what a phone woken
//! by a push asks before it shows a message of the chat (ROO-1275, D3).
//!
//! A device of mine in a chat says so on my relays every half minute, by a
//! focus event signed with a key only my devices make
//! (`messenger_presence::focus`). The phone has no runtime to hear it: the
//! process may have been dead for hours. So the handler asks the relays of
//! my home set for it, once, the way the app went when it last ran
//! (`net::apply_saved`), and shows nothing of a chat that is read live
//! there. The relays learn that a phone of mine woke at that moment; that
//! is the price the owner accepted (variant 2 of ROO-1279).
//!
//! The question is short: the message rings late by at most [`BUDGET`]
//! when the relays are slow, and rings as before when they do not answer,
//! when nothing opens, or on any trouble.

use crate::net;
use messenger_core::presence::KEY_FOCUS_SENT;
use messenger_core::RelayUrl;
use messenger_presence::elsewhere::{live, FOCUS_AHEAD_SECS};
use messenger_presence::focus::{self, FocusKeys, KIND_FOCUS};
use messenger_presence::{FOCUS_D, FOCUS_EPOCH, FOCUS_TTL_SECS};
use messenger_store::{relays, settings, Store};
use messenger_transport::RelayConfig;
use nostr::key::Keys;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the relays are given to take the connection.
pub const CONNECT: Duration = Duration::from_secs(3);
/// How long the question waits for the answer.
pub const PATIENCE: Duration = Duration::from_secs(2);
/// Connection and question together: the message waits no longer.
pub const BUDGET: Duration = Duration::from_secs(4);

/// The relays of my home set this phone reads from: where my devices say
/// their focus (`Scope::Own`), with their gate keys.
async fn home(store: &Store) -> messenger_core::Result<Vec<RelayConfig>> {
    Ok(relays::list(store)
        .await?
        .into_iter()
        .filter(|r| r.enabled && r.read && r.server_id.is_none())
        .filter_map(|r| Some(RelayConfig { url: RelayUrl::parse(&r.url)?, read: true, write: false, api_key: r.auth_secret, server_id: None }))
        .collect())
}

/// Whether another device of mine is reading `chat` at `now` (unix
/// seconds), by what my home relays hold of my focus. `keys` are mine.
/// False in silent mode, when no relay answers in time and on any trouble.
pub async fn read_elsewhere(store: &Store, keys: &Keys, chat: &str, now: i64) -> bool {
    // In silent mode the phone reaches no relay for this either, and an
    // unreadable switch counts as on: the message shows as before.
    if !matches!(settings::get_bool(store, relays::KEY_SILENT, false).await, Ok(false)) {
        return false;
    }
    let focus = FocusKeys::derive(keys, FOCUS_EPOCH);
    let events = match tokio::time::timeout(BUDGET, ask(store, &focus)).await {
        Ok(Ok(events)) => events,
        Ok(Err(e)) => {
            eprintln!("messenger notify: focus not asked: {e}");
            return false;
        }
        Err(_) => return false,
    };
    let sent = settings::get(store, KEY_FOCUS_SENT).await.ok().flatten().unwrap_or_default();
    reading(&focus, &events, &sent, chat, now)
}

/// The focus events my home relays hold: one question, within [`BUDGET`].
async fn ask(store: &Store, focus: &FocusKeys) -> messenger_core::Result<Vec<serde_json::Value>> {
    let home = home(store).await?;
    if home.is_empty() {
        return Ok(Vec::new());
    }
    messenger_transport::ensure_crypto_provider();
    // The way the app was using when it last ran. Unreadable settings mean
    // the direct way, as for the event of the push (`fetch`).
    let _ = net::apply_saved(store).await;
    let started = Instant::now();
    let client = Client::builder().websocket_transport(messenger_transport::BridgedTransport::default()).build();
    for config in &home {
        let _ = client.add_relay(config.connect_url().as_str()).await;
    }
    // The question goes to the relays that took the connection, so that
    // one out of reach holds nobody up.
    let connected = client.try_connect().timeout(CONNECT).await;
    let filter = Filter::new().kind(Kind::from(KIND_FOCUS)).author(focus.public_key()).identifier(FOCUS_D);
    let targets: HashMap<nostr::types::RelayUrl, Filter> = connected.success.keys().map(|url| (url.clone(), filter.clone())).collect();
    let mut found = Vec::new();
    if !targets.is_empty() {
        let patience = PATIENCE.min(BUDGET.saturating_sub(started.elapsed()));
        let asked = client.fetch_events(targets).timeout(patience);
        if let Ok(Ok(events)) = tokio::time::timeout(patience, asked).await {
            found = events.into_iter().map(|e| serde_json::to_value(&e).expect("an event serializes")).collect();
        }
    }
    client.disconnect().await;
    Ok(found)
}

/// Whether `events`, as the relays gave them, say that another device of
/// mine reads `chat` at `now`: the newest of them that this device did not
/// send (`sent`, the ids of `focus.sent`) and that is not dated far ahead
/// opens with my keys, names `chat` and is live. Read as the runtime reads
/// the focus (`messenger_presence::elsewhere`).
pub fn reading(keys: &FocusKeys, events: &[serde_json::Value], sent: &str, chat: &str, now: i64) -> bool {
    let mine = |id: &str| sent.split(' ').any(|s| s == id);
    let newest = events
        .iter()
        .filter(|json| !json.get("id").and_then(|v| v.as_str()).is_some_and(mine))
        .filter_map(focus::parse)
        .filter(|e| e.created_at <= now + FOCUS_AHEAD_SECS)
        .max_by_key(|e| e.created_at);
    let Some(event) = newest else { return false };
    let Ok(said) = focus::open(keys, &event) else { return false };
    let until = event.until().min(event.created_at + FOCUS_TTL_SECS);
    said.chat.as_deref() == Some(chat) && live(event.created_at, until, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(keys: &FocusKeys, chat: Option<&str>, at: i64) -> serde_json::Value {
        focus::build(keys, chat, at).unwrap().json
    }

    #[test]
    fn a_live_event_of_the_chat_says_it_is_read_elsewhere() {
        let me = Keys::generate();
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        let now = 1_000_000;
        let x = event(&keys, Some("dm:x"), now - 10);
        assert!(reading(&keys, std::slice::from_ref(&x), "", "dm:x", now));
        assert!(!reading(&keys, std::slice::from_ref(&x), "", "dm:y", now), "another chat");
        assert!(!reading(&keys, &[], "", "dm:x", now), "nothing on the relays");
        assert!(!reading(&keys, std::slice::from_ref(&x), "", "dm:x", now - 10 + FOCUS_TTL_SECS), "gone at its end");
        assert!(!reading(&keys, &[event(&keys, None, now - 5)], "", "dm:x", now), "in the list");
        assert!(!reading(&keys, &[event(&keys, Some("dm:x"), now + FOCUS_AHEAD_SECS + 1)], "", "dm:x", now), "dated far ahead");
        assert!(reading(&keys, &[event(&keys, Some("dm:x"), now + FOCUS_AHEAD_SECS)], "", "dm:x", now), "a clock a little fast");
    }

    #[test]
    fn the_newest_counts_and_this_phones_own_does_not() {
        let me = Keys::generate();
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        let now = 1_000_000;
        let x = event(&keys, Some("dm:x"), now - 20);
        let left = event(&keys, None, now - 5);
        assert!(!reading(&keys, &[x.clone(), left.clone()], "", "dm:x", now), "left the chat since");
        // The newest is this phone's own, said before its process went:
        // the computer's older word counts.
        let id = left["id"].as_str().unwrap();
        assert!(reading(&keys, &[x.clone(), left.clone()], &format!("other {id}"), "dm:x", now));
        let own_x = x["id"].as_str().unwrap();
        assert!(!reading(&keys, std::slice::from_ref(&x), own_x, "dm:x", now), "the phone's own word");
    }

    #[test]
    fn what_is_not_mine_or_not_whole_says_nothing() {
        let me = Keys::generate();
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        let now = 1_000_000;
        let stranger = FocusKeys::derive(&Keys::generate(), FOCUS_EPOCH);
        assert!(!reading(&keys, &[event(&stranger, Some("dm:x"), now)], "", "dm:x", now), "another user's");
        let next = FocusKeys::derive(&me, FOCUS_EPOCH + 1);
        assert!(!reading(&keys, &[event(&next, Some("dm:x"), now)], "", "dm:x", now), "another epoch");
        let mut touched = event(&keys, Some("dm:x"), now);
        touched["content"] = serde_json::json!("AAAA");
        assert!(!reading(&keys, &[touched], "", "dm:x", now), "does not verify");
    }

    /// Through a relay as the phone asks it: the computer's word is heard,
    /// a relay out of reach holds nothing up, and no relay rings as before.
    #[tokio::test]
    async fn the_phone_asks_my_home_relays() {
        use messenger_core::{traits::Clock, traits::SystemClock, MessengerConfig};
        use nostr_sdk::local_relay::LocalRelay;

        let dir = tempfile::tempdir().unwrap();
        let config = MessengerConfig::new(dir.path().join("messenger"));
        let store = Store::open(&config).await.unwrap();
        let me = Keys::generate();
        let now = SystemClock.now().secs();
        assert!(!read_elsewhere(&store, &me, "dm:x", now).await, "no relays");

        let relay = LocalRelay::builder().build();
        relay.run().await.unwrap();
        let url = relay.url().await.as_str_without_trailing_slash().to_string();
        for (url, server) in [(url.as_str(), None), ("ws://127.0.0.1:1", None), ("ws://127.0.0.1:2", Some("private"))] {
            let row = relays::RelayUpsert {
                url: url.into(),
                relay_id: None,
                source: relays::SOURCE_USER.into(),
                regions: vec![],
                read: true,
                write: true,
                enabled: true,
                auth_type: None,
                auth_secret: None,
                server_id: server.map(str::to_string),
            };
            relays::upsert(&store, &row, true).await.unwrap();
        }
        assert!(!read_elsewhere(&store, &me, "dm:x", now).await, "nothing said yet");

        // The computer says it reads dm:x.
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        let computer = Client::default();
        computer.add_relay(url.as_str()).await.unwrap();
        computer.connect().await;
        let said: Event = serde_json::from_value(event(&keys, Some("dm:x"), now - 2)).unwrap();
        computer.send_event(&said).await.unwrap();

        let started = Instant::now();
        assert!(read_elsewhere(&store, &me, "dm:x", now).await, "read on the computer");
        assert!(started.elapsed() <= BUDGET + Duration::from_millis(500), "within the budget");
        assert!(!read_elsewhere(&store, &me, "dm:y", now).await, "another chat");
        assert!(!read_elsewhere(&store, &Keys::generate(), "dm:x", now).await, "another user");

        // Left for the list: the relay keeps the newest.
        let left: Event = serde_json::from_value(event(&keys, None, now)).unwrap();
        computer.send_event(&left).await.unwrap();
        assert!(!read_elsewhere(&store, &me, "dm:x", now).await, "left the chat");
        computer.disconnect().await;
        store.close().await;
    }

    /// In silent mode the phone asks no relay: not a connection, so not a
    /// question either.
    #[tokio::test]
    async fn silent_mode_asks_nothing() {
        use messenger_core::MessengerConfig;

        let dir = tempfile::tempdir().unwrap();
        let config = MessengerConfig::new(dir.path().join("messenger"));
        let store = Store::open(&config).await.unwrap();
        let me = Keys::generate();
        // A relay that only counts who comes.
        let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let row = relays::RelayUpsert {
            url: format!("ws://{}", relay.local_addr().unwrap()),
            relay_id: None,
            source: relays::SOURCE_USER.into(),
            regions: vec![],
            read: true,
            write: true,
            enabled: true,
            auth_type: None,
            auth_secret: None,
            server_id: None,
        };
        relays::upsert(&store, &row, true).await.unwrap();

        settings::set_bool(&store, relays::KEY_SILENT, true).await.unwrap();
        assert!(!read_elsewhere(&store, &me, "dm:x", 1_000_000).await);
        let came = tokio::time::timeout(Duration::from_millis(200), relay.accept()).await.is_ok();
        assert!(!came, "no connection in silent mode");

        // The same relay is reached once silent mode is off.
        settings::set_bool(&store, relays::KEY_SILENT, false).await.unwrap();
        let (asked, came) = tokio::join!(read_elsewhere(&store, &me, "dm:x", 1_000_000), async {
            tokio::time::timeout(BUDGET, relay.accept()).await.is_ok()
        });
        assert!(!asked);
        assert!(came, "reached with silent mode off");
        store.close().await;
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! «Load earlier» in a direct chat (internal/messenger-wire.md, «Приём DM»,
//! «Раньше с сервера»).
//!
//! The first catch-up of a device takes the newest `DM_HISTORY_LIMIT`
//! wraps of the whole inbox, and nothing older ever came after. The inbox
//! is one stream of gift wraps (kind 1059, `#p` me) for every chat
//! together, and a relay cannot tell one chat's wraps from another's: so
//! the past is read by pages of the inbox of the chat's server (the home
//! set, or the private server of a private chat), going back by the time
//! on the outside of the wraps, until the chat asked for has something
//! older than what it shows or the relays have nothing older. Every wrap
//! that comes goes through the ingress as history, the same way as the
//! catch-up's: the wraps of the other chats land in theirs, gates,
//! receipts, controls and the DMs of the groups apply as always.
//!
//! Where the reading stands is kept per server (`msg_settings`,
//! `dm.history`, `dm.history@<server id>`, which goes with the server):
//! `floor` — every wrap of the server at or after
//! it is on this device, as far as its relays know — and whether the
//! relays had nothing under it (`end`). Both hold for the relays and the
//! key they were read with; another set of relays or another key starts
//! again from the oldest wrap held.

use crate::session::dm_local_set;
use crate::MessengerRuntime;
use messenger_core::{EventId, MessengerError, Result, Timestamp};
use messenger_ingress::filters;
use messenger_store::{chats, events_raw, messages, settings, Store};
use serde::Serialize;
use ts_rs::TS;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Wraps asked for in one page: strfry's default cap of a filter's limit.
pub const DM_HISTORY_PAGE: usize = 500;
/// Pages that bring wraps this device lacked, read at most by one ask; the
/// next ask goes on from where it stopped.
const DM_HISTORY_PAGES: usize = 4;
/// Pages read at most by one ask, those of wraps already here among them:
/// the first ask of a server starts at the top and reads again what the
/// catch-up brought.
const DM_HISTORY_PAGES_MAX: usize = 16;
/// How long the relays of the server may take to answer a page.
const PAGE_PATIENCE: Duration = Duration::from_secs(20);
/// How long an ask waits for a first relay of the server.
const WAIT_FOR_RELAY: Duration = Duration::from_secs(20);
/// How often the store is asked whether the wraps of a page are in, how
/// many asks in a row without one more mean the rest will not be (a wrap
/// whose handler failed is not kept), and how long a page may take.
const SETTLE_POLL: Duration = Duration::from_millis(250);
const SETTLE_STILL: u32 = 8;
const SETTLE_MAX: Duration = Duration::from_secs(60);
/// The first ask of a server starts at now and this much more: a relay
/// keeps no wrap from the future, but clocks differ.
const CLOCK_SLACK_SECS: i64 = 600;

/// What an ask for the past of a direct chat came to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DmHistoryPage {
    /// Messages of the chat older than `before` there are on this device
    /// now (0: nothing older came). The page reads them as usual.
    pub older: u32,
    /// The relays of the server have nothing older than what this device
    /// holds: asking again brings nothing (until another set of relays).
    pub end: bool,
}

/// One ask at a time per server: two would read the same pages. And the
/// size of a page (`DM_HISTORY_PAGE`; a test makes it small).
#[derive(Clone)]
pub(crate) struct DmHistoryState {
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    page: Arc<AtomicUsize>,
}

impl Default for DmHistoryState {
    fn default() -> Self {
        Self { locks: Arc::default(), page: Arc::new(AtomicUsize::new(DM_HISTORY_PAGE)) }
    }
}

impl DmHistoryState {
    async fn of(&self, key: &str) -> Arc<Mutex<()>> {
        self.locks.lock().await.entry(key.to_string()).or_default().clone()
    }

    fn page(&self) -> usize {
        self.page.load(Ordering::Relaxed).max(1)
    }

    #[cfg(test)]
    pub(crate) fn set_page(&self, size: usize) {
        self.page.store(size, Ordering::Relaxed);
    }
}

/// `dm.history`, and `dm.history@<id>` of a private server: a setting
/// named so goes when the server is removed (`servers::remove`).
fn key_of(server: Option<&str>) -> String {
    crate::scopes::sub_id("dm.history", server)
}

/// Where the reading of a server stands, when it was read with these
/// relays and this key (`whom`).
async fn reach(store: &Store, server: Option<&str>, whom: &str) -> Option<(i64, bool)> {
    let value = settings::get(store, &key_of(server)).await.ok()??;
    let mut parts = value.splitn(3, ' ');
    let floor = parts.next()?.parse().ok()?;
    let end = parts.next()? == "1";
    (parts.next()? == whom).then_some((floor, end))
}

async fn note_reach(store: &Store, server: Option<&str>, whom: &str, floor: i64, end: bool) {
    let value = format!("{floor} {} {whom}", if end { 1 } else { 0 });
    if let Err(e) = settings::set(store, &key_of(server), &value).await {
        eprintln!("messenger dm history: where the reading stands was not kept: {e}");
    }
}

/// Waits until the ingress has kept the wraps `ids` (it keeps a wrap once
/// it is applied), or until none more come in a while.
async fn settle(store: &Store, ids: &[String]) {
    let mut left: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let deadline = tokio::time::Instant::now() + SETTLE_MAX;
    let mut still = 0;
    while !left.is_empty() && still < SETTLE_STILL && tokio::time::Instant::now() < deadline {
        let before = left.len();
        let mut kept = Vec::new();
        for id in &left {
            let Some(event_id) = EventId::parse(id) else {
                kept.push(*id);
                continue;
            };
            if events_raw::contains(store, &event_id).await.unwrap_or(false) {
                kept.push(*id);
            }
        }
        for id in kept {
            left.remove(id);
        }
        if left.is_empty() {
            break;
        }
        still = if left.len() < before { 0 } else { still + 1 };
        tokio::time::sleep(SETTLE_POLL).await;
    }
}

impl MessengerRuntime {
    /// «Load earlier» in the direct chat `chat` (`dm:<hex>`, or
    /// `dm:<hex>@<server id>` of a private chat; a peer's address reads
    /// too): the wraps of the chat's server older than what this device
    /// holds, page by page from its relays, until the chat has messages
    /// older than `before` (the oldest one shown; now, for a chat that
    /// shows none) or the relays have nothing older. One ask per server at
    /// a time; another waits for it and then finds what it brought.
    pub async fn dm_history(&self, chat: &str, before: i64) -> Result<DmHistoryPage> {
        let keys = self.session_keys().await?;
        let me = messenger_core::PubKey::parse(&keys.public_key().to_hex()).ok_or(MessengerError::NotLoggedIn)?;
        let peer = self.peer_addr(chat).await?;
        let server = peer.server().map(str::to_string);
        let chat_id = chats::dm_chat_id(peer.key.as_hex(), server.as_deref());
        let lock = self.dm_history_state.of(&key_of(server.as_deref())).await;
        let _one = lock.lock().await;

        let pool = self.relays.pool().await;
        let mut relays: Vec<String> = pool
            .configured()
            .into_iter()
            .filter(|r| r.server_id.as_deref() == server.as_deref())
            .map(|r| r.url.as_str().to_string())
            .collect();
        relays.sort();
        let whom = format!("{}|{}", me.as_hex(), relays.join(","));
        let stood = reach(&self.store, server.as_deref(), &whom).await;
        let end = stood.is_some_and(|(_, end)| end);
        // What an ask that went before this one brought, or a page of its
        // own the window has not read yet.
        let there = older(&self.store, &chat_id, before).await?;
        if there > 0 || end {
            return Ok(DmHistoryPage { older: there, end });
        }

        let scope = crate::scopes::of_server(server.as_deref());
        let deadline = tokio::time::Instant::now() + WAIT_FOR_RELAY;
        while pool.connected_in(&scope).await == 0 || pool.is_silent() {
            if tokio::time::Instant::now() >= deadline {
                return Err(MessengerError::Transport("no relay connected".into()));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        // Every wrap at or after the floor is here. The first ask starts at
        // the top: the oldest wrap held says nothing of what lies between
        // it and the catch-up (a welcome to myself fetched by its id, the
        // window of one relay cut where another's was not), and a floor too
        // low would pass the gap over for good; a page of wraps already
        // here costs its bytes and not the budget of the ask.
        let mut floor = stood.map_or(now_secs() + CLOCK_SLACK_SECS, |(floor, _)| floor);
        let held: HashSet<String> =
            dm_local_set(&self.store, server.as_deref(), 0).await.into_iter().map(|i| i.id.as_hex().to_string()).collect();
        let size = self.dm_history_state.page();
        let mut fresh_pages = 0;
        for _ in 0..DM_HISTORY_PAGES_MAX {
            let filter = filters::dm_inbox_window(&me, Timestamp(0), Timestamp(floor - 1), size);
            let asked = pool.connected_in(&scope).await;
            let pages = pool.fetch_page(&filter, &scope, PAGE_PATIENCE).await?;
            // The floor goes down to where the slowest relay's page ended,
            // so no relay's wraps are passed over: a full page may have
            // more of its oldest second, which is asked again.
            let mut next: Option<i64> = None;
            let mut ids = Vec::new();
            for page in &pages {
                let Some(oldest) = page.oldest else { continue };
                let reached = if page.ids.len() >= size && oldest + 1 < floor { oldest + 1 } else { oldest };
                next = Some(next.map_or(reached, |n: i64| n.max(reached)));
                ids.extend(page.ids.iter().cloned());
            }
            settle(&self.store, &ids).await;
            let Some(next) = next else {
                // The end, when every relay asked said so: one that did not
                // answer in time may still hold more.
                let end = pages.len() >= asked;
                if end {
                    note_reach(&self.store, server.as_deref(), &whom, floor, true).await;
                }
                let there = older(&self.store, &chat_id, before).await?;
                return Ok(DmHistoryPage { older: there, end });
            };
            floor = next.min(floor - 1);
            note_reach(&self.store, server.as_deref(), &whom, floor, false).await;
            let there = older(&self.store, &chat_id, before).await?;
            if there > 0 {
                return Ok(DmHistoryPage { older: there, end: false });
            }
            if ids.iter().any(|id| !held.contains(id)) {
                fresh_pages += 1;
                if fresh_pages == DM_HISTORY_PAGES {
                    break;
                }
            }
        }
        Ok(DmHistoryPage { older: 0, end: false })
    }
}

/// Messages of `chat_id` older than `before` that the window would show.
async fn older(store: &Store, chat_id: &str, before: i64) -> Result<u32> {
    messages::count_before(store, chat_id, before).await.map(|n| n.clamp(0, u32::MAX as i64) as u32)
}

fn now_secs() -> i64 {
    use messenger_core::Clock;
    messenger_core::traits::SystemClock.now().secs()
}

#[cfg(test)]
mod tests;

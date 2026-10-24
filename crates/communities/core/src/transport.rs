// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyFor-Perimeter-1.0.1

//! What the service asks of the relays of its channels
//! (internal/communities-stage1.md §7, row 1.3; internal/communities-wire.md
//! §3–§8): the live subscriptions of every channel this device follows,
//! the catch-up after a start from where each kind of event was heard
//! last (`cm_sync`), and the pages of the feed and of a thread that the
//! database does not hold yet.
//!
//! Everything goes out as `Outbound::Subscribe` / `Outbound::Sync` with
//! `Scope::Channel`: which relays that is, is the host's to resolve (the
//! relays of the channel, `ChannelService::relays`). What the relays send
//! back comes in through `on_event` like everything else.

use crate::db::{self, channels::ChannelRow, content::MessageRow};
use crate::roles::Role;
use crate::service::{ChannelService, MEMBERSHIP_DISBANDED, MEMBERSHIP_JOINED, MEMBERSHIP_LEFT};
use crate::state::SCOPE_EVERYONE;
use crate::wire;
use messenger_core::outbound::{Filter, SyncItem};
use messenger_core::{Envelope, EventId, MessengerError, Outbound, PubKey, Result, Scope, SubId, TextFormat, Timestamp};
use messenger_richtext::md;
/// A block of a Markdown text, as the messenger's chats have it.
pub use messenger_richtext::md::MdBlock;

/// The blocks of words written in Markdown, as a page of the feed gets them
/// (`FeedItem::rich`); `None` when they are too long or too many pieces to
/// be shown as Markdown, and are shown as plain text.
pub fn rich_of(text: &str) -> Option<Vec<MdBlock>> {
    md::parse(text)
}
use nostr::key::Keys;
use serde::{Deserialize, Serialize};

/// Live subscriptions start this far back: the clocks of the writers.
pub const LIVE_MARGIN_SECS: i64 = 600;
/// NIP-59 dates a gift wrap up to two days back: notes are asked for from
/// that far before what was heard.
pub const WRAP_MARGIN_SECS: i64 = 2 * 24 * 3600;
/// The notes to a key paged in a run are paged again by a later round of
/// it only this long after (`catch_up_round`): what came meanwhile, the
/// live subscription brought.
pub const NOTES_RECATCH_SECS: i64 = 600;
/// The route of my own relays: a `Scope::Channel` of this id, which no
/// channel has, is resolved by the host to the relays the notes to me are
/// listened to on outside every channel (the module's `own_notes`). Their
/// past is paged with the catch-up, as a channel's notes are: a live
/// subscription that brought it came back with two days of wraps every
/// time it went out, ahead of everything live (живой запуск 2026-10-23).
pub const OWN_ROUTE: &str = "";
/// A page of the feed or of a thread when the caller names none.
pub const PAGE: i64 = 20;
/// The same page of a channel is asked of the relays again only this long
/// after.
pub const PAGE_REASK_SECS: i64 = 60;
/// How many of the newest sealed events held are named to the relays as
/// held when a page is asked for.
pub const LOCAL_ITEMS: i64 = 2000;
/// The largest page the relays are asked for.
pub const MAX_PAGE: i64 = 200;

/// The most events one page of a catch-up asks for: below what relays
/// cap a page at (strfry 500), so that a page that comes back full is
/// known to be cut, and the next one is asked for.
pub const CATCH_UP_LIMIT: i64 = 200;
/// A round of catch-up that came back short counts as whole only this
/// long after it started: a run that ended before its pages were read
/// leaves the cursor where it was.
pub const ROUND_SETTLE_SECS: i64 = 60;

/// The most events a round of catch-up of one part keeps as counted
/// (`PageSeen`): past it a copy counts again.
const MAX_PAGE_SEEN: usize = 100_000;

/// What a round of catch-up of a part has counted toward its pages: the
/// ids, and the `until` of the page asked for last, whose oldest second
/// comes again in it (`until` takes it in) and counts there once, as the
/// relay counts it (`again`).
#[derive(Debug, Default)]
pub(crate) struct PageSeen {
    ids: std::collections::HashSet<String>,
    until: Option<i64>,
    again: std::collections::HashSet<String>,
}

fn catch_up_limit() -> i64 {
    #[cfg(test)]
    if let Some(n) = CATCH_UP_LIMIT_TEST.with(|c| c.get()) {
        return n;
    }
    CATCH_UP_LIMIT
}

#[cfg(test)]
thread_local! {
    pub(crate) static CATCH_UP_LIMIT_TEST: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

pub const PART_JOURNAL: &str = "journal";
pub const PART_CARD: &str = "card";
pub const PART_EPOCHS: &str = "epochs";
pub const PART_SNAPSHOTS: &str = "snapshots";
pub const PART_FEED: &str = "feed";
pub const PART_NOTES: &str = "notes";
pub const PART_HEARTBEAT: &str = "heartbeat";
/// «Typing» under the posts (`wire::KIND_TYPING`): live only, never
/// paged, no cursor (`part_of` says nothing of it).
pub const PART_TYPING: &str = "typing";
/// How far back the live subscription of «typing» looks: a relay keeps
/// none, the margin only covers the clocks.
const TYPING_MARGIN_SECS: i64 = 60;

/// The longest subscription id a relay takes (NIP-01; strfry's
/// `MAX_SUBID_SIZE`): a longer one is answered `CLOSED` («bad req: invalid
/// subscription id length») after the pool sent the REQ, so the pool
/// counts it as taken and nothing of it ever comes.
pub const MAX_SUB_ID: usize = 64;

/// How many hex digits of the SHA-256 of a channel's id name it in the
/// id of a subscription, and of a key in that of its notes.
const SUB_TAG_HEX: usize = 24;
const SUB_KEY_HEX: usize = 20;

/// The id of a subscription of a channel: one per kind of event,
/// `channel:<tag>:<part>`, `<tag>` the first hex digits of the SHA-256 of
/// the channel's id (empty for none: the notes to me outside every
/// channel, `channel::notes`). Never longer than `MAX_SUB_ID`: with the
/// channel's id itself it was 77 to 142 bytes, strfry closed every one,
/// and nothing of a channel came live, only by the catch-up (живой запуск
/// 2026-10-23: a reader's comment never reached the owner's phone). A
/// digest, not a prefix of the id: whoever makes a channel chooses its
/// id, and one made to share another's prefix would take its place.
pub fn sub_id(community_id: &str, part: &str) -> SubId {
    let tag = if community_id.is_empty() { String::new() } else { crate::blobs::sha256_hex(community_id.as_bytes())[..SUB_TAG_HEX].to_string() };
    SubId(format!("channel:{tag}:{part}"))
}

fn scope(community_id: &str) -> Scope {
    Scope::Channel { id: community_id.to_string() }
}

/// The filters of a channel by what this device is in it: the journal,
/// the card, the epochs and the snapshots always; the feed only for a
/// member. The notes go by `note_filter`, one key each.
fn filters(community_id: &str, member: bool) -> Vec<(&'static str, serde_json::Value)> {
    let mut out = vec![
        (PART_JOURNAL, serde_json::json!({ "kinds": [wire::KIND_JOURNAL], "#h": [community_id] })),
        (PART_CARD, serde_json::json!({ "kinds": [wire::KIND_CARD], "#d": [community_id] })),
        (PART_EPOCHS, serde_json::json!({ "kinds": [wire::KIND_EPOCH], "#h": [community_id] })),
        (PART_SNAPSHOTS, serde_json::json!({ "kinds": [wire::KIND_SNAPSHOT], "#h": [community_id] })),
        (PART_HEARTBEAT, serde_json::json!({ "kinds": [wire::KIND_HEARTBEAT], "#d": [community_id] })),
    ];
    if member {
        out.push((PART_FEED, serde_json::json!({ "kinds": [wire::KIND_CONTENT], "#h": [community_id], "#c": [wire::CHANNEL_FEED] })));
    }
    out
}

/// The notes to one key (stage 1: on the relays of the channel, wire §7).
/// One key a filter: a relay with vguard serves a `#p` only to the
/// connection signed in as that key.
fn note_filter(key: &str) -> serde_json::Value {
    serde_json::json!({ "kinds": [crate::notes::KIND_WRAP], "#p": [key] })
}

/// One request of the notes to `key` for each set of relays (a channel's,
/// as its row keeps them): the channels on the same relays share it.
fn notes_key(key: &str, relays_json: &str) -> (String, Vec<String>) {
    let mut relays: Vec<String> = serde_json::from_str(relays_json).unwrap_or_default();
    relays.sort();
    relays.dedup();
    (key.to_string(), relays)
}

/// What `PageSeen` of a part of a channel is kept under.
fn seen_key(community_id: &str, part: &str) -> String {
    format!("{community_id}\n{part}")
}

/// A relay as the pool names it, so that a journal's `wss://r/` and the
/// pool's `wss://r` are one.
fn same_relay(url: &str) -> String {
    messenger_core::RelayUrl::parse(url).map_or_else(|| url.to_string(), |u| u.as_str().to_string())
}

/// The id of the subscription of the notes to `key` in a channel: mine
/// under `notes`, a keeper's own key under `notes:<digest of the key>`.
fn notes_sub(community_id: &str, key: &str, mine: bool) -> SubId {
    if mine {
        sub_id(community_id, PART_NOTES)
    } else {
        sub_id(community_id, &format!("{PART_NOTES}:{}", &crate::blobs::sha256_hex(key.as_bytes())[..SUB_KEY_HEX]))
    }
}

fn with_since(mut f: serde_json::Value, since: Option<i64>) -> Filter {
    if let Some(s) = since {
        f["since"] = serde_json::json!(s.max(0));
    }
    Filter(f)
}

/// The part of a channel an event belongs to, and the channel: what its
/// cursor in `cm_sync` is kept under.
pub(crate) fn part_of(signed: &serde_json::Value) -> Option<(String, String)> {
    let kind = u16::try_from(signed["kind"].as_u64()?).ok()?;
    let tag = |name: &str| signed["tags"].as_array()?.iter().find(|t| t[0] == name).and_then(|t| t[1].as_str()).map(str::to_string);
    Some(match kind {
        wire::KIND_JOURNAL => (tag("h")?, PART_JOURNAL.into()),
        wire::KIND_CARD => (tag("d")?, PART_CARD.into()),
        wire::KIND_EPOCH => (tag("h")?, PART_EPOCHS.into()),
        wire::KIND_SNAPSHOT => (tag("h")?, PART_SNAPSHOTS.into()),
        wire::KIND_HEARTBEAT => (tag("d")?, PART_HEARTBEAT.into()),
        wire::KIND_CONTENT => (tag("h")?, PART_FEED.into()),
        crate::notes::KIND_WRAP => (String::new(), format!("{PART_NOTES}:{}", tag("p")?)),
        _ => return None,
    })
}

/// A post or a comment as a page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedItem {
    pub id: String,
    pub community_id: String,
    /// Of a comment: its post. Empty for a post.
    pub post_id: String,
    pub parent_id: Option<String>,
    pub author: String,
    pub created_at: i64,
    pub text: Option<String>,
    /// `md` when `text` (a caption too) is written in Markdown, as the
    /// envelope in force says (`fmt`; the edit's, once edited); `None`
    /// for plain text.
    pub fmt: Option<String>,
    /// Of a Markdown text: its blocks, as a chat's message has them
    /// (`md::parse`); `None` when plain, or too long to be shown as
    /// Markdown, which is then shown as plain text.
    pub rich: Option<Vec<MdBlock>>,
    /// The media envelope (`t: media`), when it is one.
    pub media_json: Option<String>,
    /// `ok` | `pending` | `approved` | `rejected` | `released`.
    pub review: String,
    pub hidden: bool,
    pub deleted: bool,
    pub pinned: bool,
    /// Emoji, how many set it, and whether I am one of them.
    pub reactions: Vec<(String, i64, bool)>,
    /// Of a post: how many comments are here.
    pub comments: i64,
    /// The scope (section) it is in (wire §14.3).
    pub scope_id: String,
    /// When the edit in force was written (its author's clock); none
    /// for a message never edited.
    pub edited_at: Option<i64>,
    /// Mine, still in the outbox: no relay has it yet (`channel.outbox`
    /// tells when it went).
    pub pending: bool,
}

/// Where the next page starts: the last item of this one, by date and id
/// (`history_page`, `thread_page`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedCursor {
    pub created_at: i64,
    pub id: String,
}

/// A page and whether the relays were asked for more: what they send
/// comes in as `channel.message` and a second call shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedPage {
    /// Oldest first, the feed and a thread alike: the newest at the bottom.
    pub items: Vec<FeedItem>,
    /// What to pass as `before` for the next page: the oldest date of
    /// this one (of a thread, the oldest comment of it).
    pub next_before: Option<i64>,
    /// The same with the id of that item: what `history_page` and
    /// `thread_page` take, so that items of one second are not skipped.
    pub next: Option<FeedCursor>,
    /// The relays were asked for the page and it came back (the ask is
    /// awaited): what it brought comes as `channel.message` a moment
    /// later, once taken.
    pub asked: bool,
}

impl ChannelService {
    /// The channels this device listens to: every one it knows of but
    /// those it left and those disbanded, and every one it keeps.
    async fn followed(&self) -> Result<Vec<(ChannelRow, bool)>> {
        let kept = self.kept().await?;
        let mut out = Vec::new();
        for row in db::channels::list(&self.env.db).await? {
            let keeps = kept.contains(&row.id);
            if !keeps && (row.membership == MEMBERSHIP_LEFT || row.membership == MEMBERSHIP_DISBANDED) {
                continue;
            }
            out.push((row, keeps));
        }
        Ok(out)
    }

    /// The keys notes of a channel come to on this device: mine, and the
    /// keeper's own key when it is kept here and its journal names that
    /// key as the inbox (journals before `inbox`, wire §3), each in a
    /// filter of its own: a relay with vguard serves a `#p` only to the
    /// connection signed in as that key (the trial of 1.6, problem 5).
    async fn inboxes(&self, keys: &Keys, community_id: &str) -> Result<Vec<String>> {
        let mut out = vec![Self::me(keys)?.as_hex().to_string()];
        if let Some(k) = self.keeper_pubkey(community_id).await? {
            let named = self.log(community_id).await?.and_then(|l| l.state().keeper().map(|r| r.inbox().clone()));
            if named.as_ref() == Some(&k) {
                out.push(k.as_hex().to_string());
            }
        }
        Ok(out)
    }

    /// The live subscriptions of every channel followed here, from a
    /// little before now.
    pub async fn subscriptions(&self, keys: &Keys) -> Result<Vec<Outbound>> {
        let now = self.now();
        let mut out = Vec::new();
        let mut notes_asked = std::collections::HashSet::new();
        for (row, keeps) in self.followed().await? {
            let inboxes = self.inboxes(keys, &row.id).await?;
            let member = row.membership == MEMBERSHIP_JOINED || keeps;
            for (part, f) in filters(&row.id, member) {
                out.push(Outbound::Subscribe { id: sub_id(&row.id, part), filter: with_since(f, Some(now - LIVE_MARGIN_SECS)), scope: scope(&row.id) });
            }
            // The «typing» of the members, live alone: not in `filters`,
            // which the catch-up and the pages of the relays walk too.
            if member {
                let f = serde_json::json!({ "kinds": [wire::KIND_TYPING], "#h": [row.id.clone()] });
                out.push(Outbound::Subscribe { id: sub_id(&row.id, PART_TYPING), filter: with_since(f, Some(now - TYPING_MARGIN_SECS)), scope: scope(&row.id) });
            }
            for (i, key) in inboxes.iter().enumerate() {
                // The notes to a key once for the relays that bring them,
                // not once a channel, and new ones only (`limit` 0): the
                // catch-up brings what came before. Every channel asking for
                // two days of them again had each round hand the module
                // thousands of the messenger's wraps (живой запуск
                // 2026-10-23: ~2 000 events a round, minutes, a join last).
                if !notes_asked.insert(notes_key(key, &row.relays_json)) {
                    continue;
                }
                let mut filter = with_since(note_filter(key), Some(now - WRAP_MARGIN_SECS));
                filter.0["limit"] = serde_json::json!(0);
                out.push(Outbound::Subscribe { id: notes_sub(&row.id, key, i == 0), filter, scope: scope(&row.id) });
            }
        }
        Ok(out)
    }

    /// The catch-up of every channel followed here: each kind of event
    /// from a little before the newest one heard (all of it when none
    /// was), the journal reconciled against the operations held here.
    pub async fn catch_up(&self, keys: &Keys) -> Result<Vec<Outbound>> {
        self.catch_up_in(keys, false, &[], true).await
    }

    /// The catch-up of a round of a host whose live subscriptions stay up
    /// between its rounds (the module): the notes to a key on a set of
    /// relays paged in this run less than `NOTES_RECATCH_SECS` ago are not
    /// paged again. A gift wrap is dated up to two days back, so a round
    /// pages two days of every wrap to the key (the direct messages of the
    /// messenger among them), and every round did (живой запуск
    /// 2026-10-23: thousands of events a round, long on a phone), though
    /// the live subscription had brought what came since. `forget_caught`
    /// when that may not hold: a relay connected anew, a round that did
    /// not go out. `own`: the relays the notes to me are listened to on
    /// outside the channels (`OWN_ROUTE`); those of them no channel
    /// followed here is on have the notes to me paged too.
    pub async fn catch_up_round(&self, keys: &Keys, own: &[String]) -> Result<Vec<Outbound>> {
        self.catch_up_in(keys, true, own, true).await
    }

    /// The notes of `catch_up_round` alone, by its rule: what a wake of the
    /// keeper asks for when the live subscriptions are up (a note that came
    /// while the phone slept, at most once in `NOTES_RECATCH_SECS`); the
    /// journals, cards, epochs and posts are the subscriptions' to bring.
    pub async fn catch_up_notes(&self, keys: &Keys, own: &[String]) -> Result<Vec<Outbound>> {
        self.catch_up_in(keys, true, own, false).await
    }

    /// The notes of every key are paged again by the next round.
    pub fn forget_caught(&self) {
        if let Ok(mut caught) = self.notes_caught.lock() {
            caught.clear();
        }
    }

    async fn catch_up_in(&self, keys: &Keys, fresh: bool, own: &[String], parts: bool) -> Result<Vec<Outbound>> {
        let mut out = Vec::new();
        let mut notes_rounds: std::collections::BTreeMap<String, Option<i64>> = std::collections::BTreeMap::new();
        let mut notes_asked = std::collections::HashSet::new();
        let mut channels_on: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (row, keeps) in self.followed().await? {
            let inboxes = self.inboxes(keys, &row.id).await?;
            let member = row.membership == MEMBERSHIP_JOINED || keeps;
            channels_on.extend(serde_json::from_str::<Vec<String>>(&row.relays_json).unwrap_or_default().iter().map(|r| same_relay(r)));
            // The notes to a key: one round for the key, its pages asked of
            // every channel that brings them (`heard`), newest first and
            // `limit` at a time, until a page comes back short (the trial
            // of 1.6, problem 6: strfry hands 500 events a REQ, and a keeper
            // back after a day had more).
            for key in &inboxes {
                let asked = notes_key(key, &row.relays_json);
                if !notes_asked.insert(asked.clone()) {
                    continue;
                }
                if let Some(page) = self.notes_page(key, asked, fresh, scope(&row.id), &mut notes_rounds).await? {
                    out.push(page);
                }
            }
            if !parts {
                continue;
            }
            for (part, f) in filters(&row.id, member) {
                let since = self.open_round(&row.id, part, LIVE_MARGIN_SECS).await?;
                let local = if part == PART_JOURNAL {
                    db::channels::op_items(&self.env.db, &row.id)
                        .await?
                        .into_iter()
                        .filter_map(|(id, at)| Some(SyncItem { id: EventId::parse(&id)?, created_at: Timestamp(at) }))
                        .collect()
                } else {
                    vec![]
                };
                let mut filter = with_since(f, since);
                filter.0["limit"] = serde_json::json!(catch_up_limit());
                out.push(Outbound::Sync { scope: scope(&row.id), filter, local });
            }
        }
        // The notes to me on my own relays that no channel's page reaches
        // (an invitation from a channel not known here, the copies of my
        // other devices), in the same round of the notes to me: the relays
        // of a channel hand the same wraps to its page.
        let mut own_only: Vec<String> = own.iter().map(|r| same_relay(r)).filter(|r| !channels_on.contains(r)).collect();
        own_only.sort();
        own_only.dedup();
        if !own_only.is_empty() {
            let me = Self::me(keys)?.as_hex().to_string();
            let asked = (me.clone(), own_only);
            if let Some(page) = self.notes_page(&me, asked, fresh, scope(OWN_ROUTE), &mut notes_rounds).await? {
                out.push(page);
            }
        }
        Ok(out)
    }

    /// A page of the notes to `key` on the relays `asked` names, of the
    /// round of the notes to the key (one for every set of relays); none
    /// when `fresh` and the set was paged in this run not long ago.
    async fn notes_page(&self, key: &str, asked: (String, Vec<String>), fresh: bool, scope: Scope, rounds: &mut std::collections::BTreeMap<String, Option<i64>>) -> Result<Option<Outbound>> {
        if fresh {
            let now = self.now();
            let Ok(mut caught) = self.notes_caught.lock() else { return Ok(None) };
            if caught.get(&asked).is_some_and(|at| now - at < NOTES_RECATCH_SECS && *at <= now) {
                return Ok(None);
            }
            caught.insert(asked, now);
        }
        let part = format!("{PART_NOTES}:{key}");
        let since = match rounds.get(&part) {
            Some(s) => *s,
            None => {
                let s = self.open_round("", &part, WRAP_MARGIN_SECS).await?;
                rounds.insert(part, s);
                s
            }
        };
        let mut filter = with_since(note_filter(key), since);
        filter.0["limit"] = serde_json::json!(catch_up_limit());
        Ok(Some(Outbound::Sync { scope, filter, local: vec![] }))
    }

    /// A new round of catch-up of a part of a channel; from where. The
    /// round before moves the cursor only if it came back short of its
    /// limit (whole) and had time to be read. `margin`: how far before the
    /// newest event heard the round starts (the clocks of the writers;
    /// a gift wrap dated back two days).
    async fn open_round(&self, community_id: &str, part: &str, margin: i64) -> Result<Option<i64>> {
        let now = self.now();
        if let Some(r) = db::channels::round(&self.env.db, community_id, part).await? {
            if r.round_since.is_some() && r.page_count < catch_up_limit() && now - r.round_started >= ROUND_SETTLE_SECS {
                db::channels::round_done(&self.env.db, community_id, part).await?;
            }
        }
        let since = db::channels::heard_at(&self.env.db, community_id, part).await?.map(|a| a - margin);
        db::channels::round_start(&self.env.db, community_id, part, since.unwrap_or(0).max(0), now).await?;
        if let Ok(mut seen) = self.page_seen.lock() {
            seen.remove(&seen_key(community_id, part));
        }
        Ok(since)
    }

    /// Whether an event of a page counts toward the page of its round: the
    /// first copy of it does, a copy of one counted already does not (the
    /// same page from another relay of the channel, or from another channel
    /// on the same relays), but for one of the oldest second of the page
    /// asked for last, which that page brings again.
    fn counts(&self, community_id: &str, part: &str, signed: &serde_json::Value, at: i64) -> bool {
        let Some(id) = signed["id"].as_str() else { return true };
        let Ok(mut seen) = self.page_seen.lock() else { return true };
        let round = seen.entry(seen_key(community_id, part)).or_default();
        if round.ids.contains(id) {
            return round.until == Some(at) && round.again.insert(id.to_string());
        }
        if round.ids.len() < MAX_PAGE_SEEN {
            round.ids.insert(id.to_string());
        }
        true
    }

    /// The page before `until` is asked for in this round of the part.
    fn page_asked(&self, community_id: &str, part: &str, until: i64) {
        if let Ok(mut seen) = self.page_seen.lock() {
            let round = seen.entry(seen_key(community_id, part)).or_default();
            round.until = Some(until);
            round.again.clear();
        }
    }

    /// Asks the relays of a channel not followed here yet (a link opened)
    /// for its journal and its card: a round of pages, as the catch-up, so
    /// that a journal longer than a relay hands back for one REQ comes
    /// down to its `create` (the trial of 1.6, problem 10).
    pub async fn ask_journal(&self, community_id: &str) -> Result<()> {
        for (part, f) in filters(community_id, false).into_iter().filter(|(p, _)| *p == PART_JOURNAL || *p == PART_CARD) {
            let since = self.open_round(community_id, part, LIVE_MARGIN_SECS).await?;
            let mut filter = with_since(f, since);
            filter.0["limit"] = serde_json::json!(catch_up_limit());
            self.env.net.send(Outbound::Sync { scope: scope(community_id), filter, local: vec![] }).await?;
        }
        Ok(())
    }

    /// Sends the catch-up and the live subscriptions: what a host does at
    /// a start and on every `resubscribe`.
    pub async fn listen(&self, keys: &Keys) -> Result<()> {
        for out in self.catch_up(keys).await?.into_iter().chain(self.subscriptions(keys).await?) {
            if let Err(e) = self.env.net.send(out).await {
                tracing::debug!(error = %e, "a request of the channels did not go out");
            }
        }
        Ok(())
    }

    /// The relays of the channels followed here, each once: what the host
    /// puts into its pool for `Scope::Channel`.
    pub async fn relays(&self) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for (row, _) in self.followed().await? {
            for r in serde_json::from_str::<Vec<String>>(&row.relays_json).unwrap_or_default() {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    /// An event of a part was heard (`live`: not from a catch-up). A part
    /// of a channel, and the notes to a key, only count it in its round
    /// (`open_round` moves the cursor once a round came back whole), and a
    /// page of the catch-up that came back full asks for the page before
    /// it: a relay that caps a page must not lose the rest. The ask is
    /// noted (`ask_later`) and sent off the way of the events: the pool
    /// awaits a `Sync` to its end, and the page of the notes to me asked of
    /// every channel followed held the loop of the owner's phone 26–56 s
    /// after every channel made (живой запуск 2026-10-23, «an event of kind
    /// 1059 held the loop 56 s»).
    pub(crate) async fn heard(&self, signed: &serde_json::Value, live: bool) -> Result<()> {
        let Some((cid, part)) = part_of(signed) else { return Ok(()) };
        let Some(at) = signed["created_at"].as_i64() else { return Ok(()) };
        self.mark(Some(format!("where the catch-up of {} stands", part.get(..14).unwrap_or(&part))));
        // A date far ahead would make the next catch-up start past what
        // is still to come.
        let at = at.min(self.now() + wire::SKEW_SECS);
        let limit = catch_up_limit();
        if let Some(key) = part.strip_prefix(PART_NOTES).and_then(|k| k.strip_prefix(':')) {
            // Heard live with no round of pages open (the notes to me on my
            // own relays, `own_notes` of the module): nothing older waits,
            // the cursor moves at once.
            if live && db::channels::round(&self.env.db, &cid, &part).await?.is_none_or(|r| r.round_since.is_none()) {
                return db::channels::heard(&self.env.db, &cid, &part, at).await;
            }
            if !live && !self.counts(&cid, &part, signed, at) {
                return Ok(());
            }
            // A full page of the notes to a key: the page before it, once
            // for every set of relays of the channels followed (the notes
            // of each come on its relays). Asked of every channel, the
            // channels on the same relays brought the same page each, and
            // every `limit` of those copies asked for it of all of them
            // again: the dev app pulled 95 GB of the same wraps from
            // eu-1-relay-1 in a night, and strfry, holding what it had not
            // sent yet, was killed for its memory (2026-10-23).
            if let Some((since, until)) = db::channels::round_heard(&self.env.db, &cid, &part, at, !live, limit).await? {
                self.mark(Some(format!("the next page of the notes to {} wanted of every set of relays followed", key.get(..8).unwrap_or(key))));
                self.page_asked(&cid, &part, until);
                let mut asked = std::collections::HashSet::new();
                for (row, _) in self.followed().await? {
                    if !asked.insert(notes_key(key, &row.relays_json)) {
                        continue;
                    }
                    let mut filter = with_since(note_filter(key), Some(since));
                    filter.0["until"] = serde_json::json!(until);
                    filter.0["limit"] = serde_json::json!(limit);
                    self.ask_later(Outbound::Sync { scope: scope(&row.id), filter, local: vec![] });
                }
            }
            return Ok(());
        }
        if !live && !self.counts(&cid, &part, signed, at) {
            return Ok(());
        }
        if let Some((since, until)) = db::channels::round_heard(&self.env.db, &cid, &part, at, !live, limit).await? {
            self.page_asked(&cid, &part, until);
            let Some((_, f)) = filters(&cid, true).into_iter().find(|(p, _)| *p == part) else { return Ok(()) };
            let mut filter = with_since(f, Some(since));
            filter.0["until"] = serde_json::json!(until);
            filter.0["limit"] = serde_json::json!(limit);
            self.ask_later(Outbound::Sync { scope: scope(&cid), filter, local: vec![] });
        }
        Ok(())
    }

    /// Whom `me` sees a channel as: a moderator sees everything (`None`),
    /// anybody else what is released, and their own.
    async fn viewer(&self, keys: &Keys, community_id: &str) -> Result<Option<String>> {
        let me = Self::me(keys)?;
        let state = self.state(community_id).await?;
        Ok(if state.role_of(&me).is_some_and(Role::is_moderator) { None } else { Some(me.as_hex().to_string()) })
    }

    /// The rows as a page shows them to `me` (hex), whose reactions are marked.
    async fn items(&self, community_id: &str, rows: Vec<MessageRow>, me: &str) -> Result<Vec<FeedItem>> {
        let pinned = self.state(community_id).await?.pinned;
        let pending = self.outbox_pending(community_id).await?;
        // The reactions and the counts of comments of the whole page in two asks, not two a row.
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        let posts: Vec<&str> = rows.iter().filter(|r| r.is_post()).map(|r| r.id.as_str()).collect();
        let mut reactions = db::content::reactions_of(&self.env.db, &ids, me).await?;
        let counts = db::content::count_comments_of(&self.env.db, community_id, &posts).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let markdown = !r.deleted && Envelope::parse(&r.envelope_json).is_ok_and(|e| e.format() == TextFormat::Markdown);
            out.push(FeedItem {
                fmt: markdown.then(|| TextFormat::Markdown.name().to_string()),
                rich: if markdown { r.text.as_deref().and_then(rich_of) } else { None },
                reactions: reactions.remove(&r.id).unwrap_or_default(),
                comments: if r.is_post() { counts.get(&r.id).copied().unwrap_or(0) } else { 0 },
                pinned: pinned.contains(&r.id),
                hidden: r.mod_state == db::content::MOD_HIDDEN,
                pending: pending.contains(&r.id),
                id: r.id,
                community_id: r.community_id,
                post_id: r.post_id,
                parent_id: r.parent_id,
                author: r.author,
                created_at: r.created_at,
                text: r.text,
                media_json: r.media_json,
                review: r.review,
                deleted: r.deleted,
                scope_id: r.scope_id,
                edited_at: (r.edited_at > 0).then_some(r.edited_at),
            });
        }
        Ok(out)
    }

    /// The scopes of a channel I hold a key of.
    pub(crate) async fn scopes_keyed(&self, keys: &Keys, state: &crate::state::ChannelState) -> Result<std::collections::BTreeSet<String>> {
        let member = self.member(keys);
        let mut out = std::collections::BTreeSet::new();
        for scope_id in state.scopes.keys() {
            if member.newest_key(&state.community_id, scope_id).await?.is_some() {
                out.insert(scope_id.clone());
            }
        }
        Ok(out)
    }

    /// The newest page of the feed of one section, asked of the relays
    /// (2026-10-23): for a section I was just let into, whose events came
    /// while it was not mine and were dropped (wire §14.5) — its posts and
    /// the meta of a hidden one (§14.1).
    pub(crate) async fn ask_scope_feed(&self, community_id: &str, scope_id: &str) -> Result<bool> {
        let f = serde_json::json!({ "kinds": [wire::KIND_CONTENT], "#h": [community_id], "#s": [scope_id], "#c": [wire::CHANNEL_FEED] });
        self.ask_page(community_id, &format!("{PART_FEED}:in:{scope_id}"), f, None, MAX_PAGE).await
    }

    /// Asks the relays of a channel for one page of the past, off the
    /// caller: the pool waits on each relay (20 s a relay), and a page
    /// awaited on that froze the channel as it opened (живой запуск
    /// 2026-10-23); what the relays send comes through `on_event` as
    /// `channel.message`. The same page (`what`, from the same `before`) of
    /// a channel goes at most once in `PAGE_REASK_SECS` (a channel opened
    /// again and again asks once; a page that came back short of posts
    /// asks on from where it got to), and the events held are named as
    /// `local`, so that the relays hand back none of them again. `true`: a
    /// fetch was started.
    async fn ask_page(&self, community_id: &str, what: &str, mut filter: serde_json::Value, before: Option<i64>, limit: i64) -> Result<bool> {
        let now = self.now();
        let key = format!("{community_id}:{what}:{}", before.map_or_else(String::new, |b| b.to_string()));
        if let Ok(mut asked) = self.pages_asked.lock() {
            if asked.get(&key).is_some_and(|at| now - at < PAGE_REASK_SECS && *at <= now) {
                return Ok(false);
            }
            asked.insert(key, now);
        }
        filter["until"] = serde_json::json!(before.unwrap_or(now));
        filter["limit"] = serde_json::json!(limit.clamp(1, MAX_PAGE));
        let local = db::content::outer_items(&self.env.db, community_id, LOCAL_ITEMS)
            .await?
            .into_iter()
            .filter_map(|(id, at)| Some(SyncItem { id: EventId::parse(&id)?, created_at: Timestamp(at) }))
            .collect();
        let (net, cid) = (self.env.net.clone(), community_id.to_string());
        tokio::spawn(async move {
            if let Err(e) = net.send(Outbound::Sync { scope: scope(&cid), filter: Filter(filter), local }).await {
                tracing::debug!(community = %cid, error = %e, "a page was not asked for");
            }
        });
        // The ask reaches the pool before the page is answered.
        tokio::task::yield_now().await;
        Ok(true)
    }

    /// A page of the feed, the newest `limit` posts strictly older than
    /// `before`, oldest first as a chat reads (the page above goes before
    /// it, `next` is its oldest post):
    /// from the database, and when it holds fewer than `limit` posts the
    /// relays are asked for the stream before the oldest event held here
    /// (wire §4: `#h`, `#c`, `until`, `limit`); comments come in the same
    /// stream, so a page of posts may take more than one ask. By the date
    /// alone: posts of the same second at the edge of a page are skipped,
    /// `history_page` with the cursor `next` does not skip them.
    pub async fn history(&self, keys: &Keys, community_id: &str, limit: i64, before: Option<i64>) -> Result<FeedPage> {
        let cursor = before.map(|b| FeedCursor { created_at: b, id: String::new() });
        self.history_page(keys, community_id, limit, cursor.as_ref()).await
    }

    /// A page of the feed before `before` in the order `(created_at, id)`.
    pub async fn history_page(&self, keys: &Keys, community_id: &str, limit: i64, before: Option<&FeedCursor>) -> Result<FeedPage> {
        self.history_page_in(keys, community_id, None, limit, before).await
    }

    /// The scopes a page of `me` shows (wire §14.3): those the journal
    /// gives me (every scope to a moderator), of them `only` when named.
    async fn shown_scopes(&self, keys: &Keys, community_id: &str, only: Option<&str>) -> Result<Vec<String>> {
        let state = self.state(community_id).await?;
        let mine = state.scopes_of(&Self::me(keys)?);
        Ok(match only {
            Some(s) => mine.into_iter().filter(|m| m == s).collect(),
            None => mine.into_iter().collect(),
        })
    }

    /// The newest post of each channel of `rows` that `me` may read: what
    /// its row in the list of chats shows, and the list is ordered by
    /// (owner 2026-10-23: by the last post, never by what was opened). A
    /// moderator and above (`my_role` of the row) sees every post; a
    /// reader what is released, his own, and only in the sections the
    /// journal gives him: a post of a section he lost stays in the
    /// database and is neither shown nor the date of the row. The journal
    /// is read only for a channel whose newest post is of another section
    /// than `everyone`. No key (`me` none): what the database holds.
    pub async fn last_posts(&self, me: Option<&PubKey>, rows: &[ChannelRow]) -> Result<std::collections::HashMap<String, MessageRow>> {
        let me_hex = me.map(|m| m.as_hex().to_string()).unwrap_or_default();
        let sees_all = |r: &ChannelRow| r.my_role.as_deref().and_then(Role::parse).is_some_and(Role::is_moderator);
        let seeing_all: Vec<String> = rows.iter().filter(|r| sees_all(r)).map(|r| r.id.clone()).collect();
        let mut out = db::content::last_posts(&self.env.db, &me_hex, &seeing_all).await?;
        let Some(me) = me else { return Ok(out) };
        for row in rows.iter().filter(|r| !sees_all(r)) {
            let Some(last) = out.get(&row.id) else { continue };
            if last.scope_id == SCOPE_EVERYONE {
                continue;
            }
            let Some(log) = self.log(&row.id).await? else { continue };
            let scopes: Vec<String> = log.state().scopes_of(me).into_iter().collect();
            if scopes.contains(&last.scope_id) {
                continue;
            }
            match db::content::last_post_in(&self.env.db, &row.id, Some(&me_hex), &scopes).await? {
                Some(m) => out.insert(row.id.clone(), m),
                None => out.remove(&row.id),
            };
        }
        Ok(out)
    }

    /// `history_page` of one section (`scope_id`), or of every scope I
    /// belong to (none): what a section the journal does not give me holds
    /// is never shown.
    pub async fn history_page_in(&self, keys: &Keys, community_id: &str, scope_id: Option<&str>, limit: i64, before: Option<&FeedCursor>) -> Result<FeedPage> {
        let limit = if limit <= 0 { PAGE } else { limit.min(MAX_PAGE) };
        let viewer = self.viewer(keys, community_id).await?;
        let key = before.map(|c| (c.created_at, c.id.as_str()));
        let scopes = self.shown_scopes(keys, community_id, scope_id).await?;
        let rows = db::content::feed_in(&self.env.db, community_id, key, limit, viewer.as_deref(), Some(&scopes)).await?;
        let at = before.map(|c| c.created_at);
        let asked = if (rows.len() as i64) < limit {
            let f = serde_json::json!({ "kinds": [wire::KIND_CONTENT], "#h": [community_id], "#c": [wire::CHANNEL_FEED] });
            let until = db::content::oldest_in_feed(&self.env.db, community_id, at).await?.or(at);
            let what = format!("{PART_FEED}:{}", scope_id.unwrap_or(""));
            self.ask_page(community_id, &what, f, until, limit).await?
        } else {
            false
        };
        let next = rows.last().map(|r| FeedCursor { created_at: r.created_at, id: r.id.clone() });
        // The database gives the newest first; a chat reads down to the newest.
        let rows: Vec<MessageRow> = rows.into_iter().rev().collect();
        Ok(FeedPage { next_before: next.as_ref().map(|c| c.created_at), next, items: self.items(community_id, rows, Self::me(keys)?.as_hex()).await?, asked })
    }

    /// A page of the comments under a post: the newest `limit` older than
    /// `before`, oldest first; the relays are asked for it (`#e` = the
    /// post) when the database holds fewer. By the date alone, as
    /// `history`; `thread_page` takes the cursor.
    pub async fn thread(&self, keys: &Keys, community_id: &str, post_id: &str, limit: i64, before: Option<i64>) -> Result<FeedPage> {
        let cursor = before.map(|b| FeedCursor { created_at: b, id: String::new() });
        self.thread_page(keys, community_id, post_id, limit, cursor.as_ref()).await
    }

    /// A page of a thread before `before` in the order `(created_at, id)`.
    pub async fn thread_page(&self, keys: &Keys, community_id: &str, post_id: &str, limit: i64, before: Option<&FeedCursor>) -> Result<FeedPage> {
        let limit = if limit <= 0 { PAGE } else { limit.min(MAX_PAGE) };
        if post_id.len() != 64 || !post_id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(MessengerError::Invalid("channel: post id".into()));
        }
        let viewer = self.viewer(keys, community_id).await?;
        let key = before.map(|c| (c.created_at, c.id.as_str()));
        let scopes = self.shown_scopes(keys, community_id, None).await?;
        let rows = db::content::thread_in(&self.env.db, community_id, post_id, key, limit, viewer.as_deref(), Some(&scopes)).await?;
        let asked = if (rows.len() as i64) < limit {
            let f = serde_json::json!({ "kinds": [wire::KIND_CONTENT], "#h": [community_id], "#e": [post_id] });
            let what = format!("thread:{post_id}");
            self.ask_page(community_id, &what, f, rows.first().map(|r| r.created_at).or(before.map(|c| c.created_at)), limit).await?
        } else {
            false
        };
        let next = rows.first().map(|r| FeedCursor { created_at: r.created_at, id: r.id.clone() });
        Ok(FeedPage { next_before: next.as_ref().map(|c| c.created_at), next, items: self.items(community_id, rows, Self::me(keys)?.as_hex()).await?, asked })
    }
}

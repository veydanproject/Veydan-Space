// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The snapshot of my relations: the list first (internal/messenger-wire.md
//! §3, "Снимок отношений").
//!
//! A new device, and one back from a long time off, gets its history mixed
//! with the copies of my signals: a stranger I blocked could land in a chat
//! before my block comes, and a decision without a signal (a request that
//! ran out, a contact added quietly) never comes at all. So every device
//! publishes the whole list as addressable events of my key on my relays
//! (kind 30078, `d = veydan.relations.v1` for the header, `.<n>` for the
//! pages), NIP-44 to myself: a relay keeps the newest under each `d`, which
//! is the list and nothing older. A device reads it before it subscribes to
//! anything, and keeps a live subscription so the decisions of my other
//! devices reach it as they are taken.
//!
//! One publication is one revision and the full set: a header and every
//! page with that `rev`. The receiver applies entries one by one, by their
//! `decided_at` (`merge_decision`): a snapshot never rolls a device back,
//! and the stricter of two decisions of one second wins on both. It counts
//! the list as known in full only when every page of the header's revision
//! came. Here: what one revision looks like, how it is built from this
//! device's rows, and how one is applied; the relays, the timing and the
//! live subscription are the runtime's.

use crate::relationship::{DmFrom, MyContact, PeerSignal};
use crate::requests::{KEY_DM_FROM, KEY_KEEP_DAYS};
use crate::service::DmService;
use messenger_core::outbound::WireEvent;
use messenger_core::relations::{relations_d, relations_page, KIND_RELATIONS, RELATIONS_MAX_PAGES, RELATIONS_PEERS_PER_PAGE};
use messenger_core::{Effect, EventId, MessengerError, PubKey, Result};
use messenger_store::chats::{self, Pref, PrefsRow};
use messenger_store::dm_relations::{self, RelationRow};
use messenger_store::settings;
use nostr::nips::nip44;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub mod servers;
pub use servers::{PRIVATE_PEERS_MAX, SERVERS_MAX, SERVERS_REMOVED_MAX, UI_EVENT_SERVERS_UPDATED};

/// The layout of the plain text; a header or a page of another version is
/// not read.
pub const SNAPSHOT_VERSION: u32 = 1;

/// The revision this device published last, or the newest it saw on a
/// relay; the next publication is one more.
pub const KEY_REV: &str = "relations.rev";
/// The newest `created_at` of a snapshot event seen or made: the next one
/// is dated after it, so a device whose clock runs behind still wins the
/// relay (which keeps the newest `created_at` under a `d`).
pub const KEY_SEEN_CREATED_AT: &str = "relations.seen_created_at";
/// Set when a decision was taken here and the list was not published
/// since: a device closed before it could publish owes it at the next
/// start.
pub const KEY_OWED: &str = "relations.owed";
/// When the privacy settings (`dm_from`, `keep_days`) were last set here,
/// my clock: a snapshot's settings apply only when they are newer.
pub const KEY_PRIVACY_DECIDED_AT: &str = "privacy.decided_at";
/// Set once this device published the list with the wills of the peers
/// (`Header::wills`): a device that held them before it learned to write
/// them owes the list once (`DmService::owe_wills_once`).
pub const KEY_WILLS_PUBLISHED: &str = "relations.wills_published";
/// The header this device published last, `<rev> <created_at> <id>`: a
/// header of another device of no newer revision that the relay keeps over
/// it (newer, or as new with the smaller id) took its `d`, and my revision
/// is lost (`DmService::apply_relations_snapshot`).
pub const KEY_PUBLISHED: &str = "relations.published";

/// What this device knows of my list after the fetch at the start of a
/// session: nothing (no snapshot on the relays, or none reached), the
/// peers of an incomplete set, or the whole list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    No,
    Partial,
    Complete,
}

/// Shared by the clones of the service: what the runtime learned of the
/// list, and the mark of a decision not yet published, with the hook the
/// runtime hangs on it to hear of one.
pub struct Shared {
    owed: AtomicBool,
    known: AtomicU8,
    hook: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Rumor ids of accepts whose secret was refused lately
    /// (`crate::invites::REFUSED_REMEMBERED`), oldest first.
    refused: Mutex<VecDeque<String>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self { owed: AtomicBool::new(false), known: AtomicU8::new(0), hook: RwLock::new(None), refused: Mutex::new(VecDeque::new()) }
    }
}

impl Shared {
    /// Note a refused accept by its rumor id: whether it is new here (its
    /// copy from another relay is not).
    pub(crate) fn remember_refused(&self, rumor_id: &str) -> bool {
        let mut refused = self.refused.lock().unwrap_or_else(|e| e.into_inner());
        if refused.iter().any(|id| id == rumor_id) {
            return false;
        }
        if refused.len() >= crate::invites::REFUSED_REMEMBERED {
            refused.pop_front();
        }
        refused.push_back(rumor_id.to_string());
        true
    }
}

/// One event of the snapshot as a relay gave it, before it is read: the
/// header (`page` 0) or a page. `id` is the id of the event, when known:
/// without it a revision of another device is not told from my own.
#[derive(Clone, Debug)]
pub struct Piece {
    pub id: Option<EventId>,
    pub page: usize,
    pub created_at: i64,
    pub content: String,
}

impl Piece {
    /// A fetched event (its signature checked by the transport) when it is
    /// of my key, our kind and one of our `d`.
    pub fn of_json(json: &serde_json::Value, me: &PublicKey) -> Option<Piece> {
        let e: Event = serde_json::from_value(json.clone()).ok()?;
        if e.kind.as_u16() != KIND_RELATIONS || &e.pubkey != me {
            return None;
        }
        let page = relations_page(&e.tags.identifier()?)?;
        Some(Piece { id: EventId::parse(&e.id.to_hex()), page, created_at: e.created_at.as_secs() as i64, content: e.content })
    }
}

/// The header this device published last (`KEY_PUBLISHED`).
struct Published {
    rev: u64,
    created_at: i64,
    id: EventId,
}

impl Published {
    /// A header of another device of no newer revision than mine was built
    /// without it (a build is one more than every revision it saw), and the
    /// relay keeps it over mine when it is newer, or as new with the smaller
    /// id (NIP-01): two devices that built in the same second, and only the
    /// one whose header went is owed, so the two never take turns.
    fn lost_to(&self, header: &Piece, rev: u64) -> bool {
        let Some(id) = &header.id else { return false };
        id != &self.id
            && rev <= self.rev
            && (header.created_at > self.created_at || (header.created_at == self.created_at && id.as_hex() < self.id.as_hex()))
    }
}

/// What applying a set of pieces did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The newest revision among the pieces; `None` when none could be
    /// read, or when every one is older than the list this device holds.
    pub rev: Option<u64>,
    /// The header of that revision and every page it names came.
    pub complete: bool,
    /// Whose relation changed here.
    pub changed: Vec<PubKey>,
    /// The privacy settings of the header were newer than the ones here.
    pub settings_changed: bool,
    /// A choice of a chat (no notifications, pinned, archived) was taken,
    /// or a chat went with a removed server.
    pub chats_changed: bool,
    /// A private server was taken or removed here: the running session
    /// listens to it, or stops (internal/messenger-wire.md, «Серверы на
    /// всех устройствах»).
    pub servers_changed: bool,
    /// Groups of mine this device is not in: the runtime fetches their
    /// welcomes to myself.
    pub groups_wanted: Vec<GroupWanted>,
}

/// `[hex, my_contact, blocked, decided_at]`, compact: `my_contact` is one
/// letter (`a` approved, `d` declined, `x` expired, `n` none — a block, a
/// removal or an unblock; a `none` nobody decided is not written).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry(String, String, u8, i64);

fn letter(m: MyContact) -> &'static str {
    match m {
        MyContact::None => "n",
        MyContact::Approved => "a",
        MyContact::Declined => "d",
        MyContact::Expired => "x",
    }
}

fn contact_of(letter: &str) -> Option<MyContact> {
    Some(match letter {
        "n" => MyContact::None,
        "a" => MyContact::Approved,
        "d" => MyContact::Declined,
        "x" => MyContact::Expired,
        _ => return None,
    })
}

/// What the peer signalled last, one letter (`a` approved, `b` blocked,
/// `l` left, `d` declined, `r` revoked, `n` none).
fn will_letter(p: PeerSignal) -> &'static str {
    match p {
        PeerSignal::None => "n",
        PeerSignal::Approved => "a",
        PeerSignal::Blocked => "b",
        PeerSignal::Left => "l",
        PeerSignal::Declined => "d",
        PeerSignal::Revoked => "r",
    }
}

fn signal_of(letter: &str) -> Option<PeerSignal> {
    Some(match letter {
        "n" => PeerSignal::None,
        "a" => PeerSignal::Approved,
        "b" => PeerSignal::Blocked,
        "l" => PeerSignal::Left,
        "d" => PeerSignal::Declined,
        "r" => PeerSignal::Revoked,
        _ => return None,
    })
}

/// `[index, peer_signal, last_signal_at, was_ever_mutual]`: the will of
/// the peer of entry `index` of the same piece, counted over `peers` and
/// then `private_peers`. Apart from the entries, whose arrays an older
/// build reads at their length; only a peer with a will to tell has one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Will(usize, String, i64, u8);

/// The peer's will by `(hex, server_id)`, of the rows that have one.
type Wills = HashMap<(String, String), (PeerSignal, i64, bool)>;

fn wills_of(rows: &[RelationRow], into: &mut Wills) {
    for r in rows {
        let p = PeerSignal::parse(&r.peer_signal).unwrap_or(PeerSignal::None);
        if p != PeerSignal::None || r.was_ever_mutual {
            into.insert((r.peer_pubkey.clone(), r.server_id.clone()), (p, r.last_signal_at, r.was_ever_mutual));
        }
    }
}

/// The wills of one piece's entries, by their index there.
fn piece_wills(peers: &[Entry], private_peers: &[serde_json::Value], wills: &Wills) -> Vec<serde_json::Value> {
    let home = peers.iter().map(|e| Some((e.0.clone(), String::new())));
    let private = private_peers
        .iter()
        .map(|v| serde_json::from_value::<servers::PrivateEntry>(v.clone()).ok().map(|e| (e.0, e.1)));
    home.chain(private)
        .enumerate()
        .filter_map(|(i, key)| {
            let (p, at, mutual) = wills.get(&key?)?;
            serde_json::to_value(Will(i, will_letter(*p).into(), *at, u8::from(*mutual))).ok()
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    v: u32,
    rev: u64,
    /// When it was built (the clock of that device): for the eye.
    at: i64,
    /// How many events the revision has, the header among them.
    pages: usize,
    complete: bool,
    dm_from: String,
    keep_days: u32,
    /// When the settings were set, on the device that set them: they
    /// apply where that is newer. Not `at`: a device that never touched
    /// them publishes what it has, and must not undo a newer choice.
    #[serde(default)]
    settings_at: i64,
    /// My invitations (`crate::invites::invites_for_snapshot`): read entry
    /// by entry, so one this build cannot read costs nothing else.
    #[serde(default)]
    invites: Vec<serde_json::Value>,
    /// The choices of my chats (`chat_entries`): an older build leaves
    /// them out, and absence is no word.
    #[serde(default)]
    chats: Vec<ChatEntry>,
    /// My deleted direct chats, `[chat_id, cleared_at, peer_upto]`
    /// (`chats::tombstones`), the newest deletion first.
    #[serde(default)]
    deleted: Vec<(String, i64, i64)>,
    /// My private servers (`servers::ServerEntry`); `None` from a build
    /// that writes none, which is not the word "no servers".
    #[serde(default)]
    servers: Option<Vec<serde_json::Value>>,
    /// My removed private servers, `[id, removed_at]`.
    #[serde(default)]
    servers_removed: Vec<serde_json::Value>,
    /// My decisions in private chats (`servers::PrivateEntry`), apart from
    /// `peers`, which an older build reads as the home chats.
    #[serde(default)]
    private_peers: Vec<serde_json::Value>,
    /// The wills of the peers of this header's entries (`Will`); an older
    /// build skips them, and absence is no word.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wills: Vec<serde_json::Value>,
    /// The groups I am in, `[group_id, wrap_id, server_id]`: the gift wrap
    /// of my newest welcome to myself, which a new device fetches by its
    /// id (`GroupWanted`); `server_id` empty for the home set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    groups: Vec<serde_json::Value>,
    peers: Vec<Entry>,
}

/// The groups the header names at most, the latest touched first.
pub const GROUPS_MAX: usize = 200;

/// A group of mine another device is in and this one is not: the welcome
/// to myself that brings it, to be fetched by its id from my inbox (the
/// home set) or from the group's private server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupWanted {
    pub group_id: String,
    pub wrap_id: EventId,
    pub server: Option<String>,
}

/// `messenger_groups::service::MEMBERSHIP_JOINED`, which this crate does
/// not see.
const JOINED: &str = "joined";

/// `[chat_id, muted, pinned, archived]`: each choice by the time it was
/// made (my clock), positive when on, negative when off, 0 when never.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ChatEntry(String, i64, i64, i64);

impl ChatEntry {
    fn of(&self, pref: Pref) -> i64 {
        match pref {
            Pref::Muted => self.1,
            Pref::Pinned => self.2,
            Pref::Archived => self.3,
        }
    }
}

/// A choice that is off is carried this long after it was made: my other
/// devices hear of it by then, and the list does not grow with every chat
/// ever muted and let speak again.
const CHAT_OFF_KEPT_SECS: i64 = 30 * 86_400;

/// The entries of my chats: every one with a choice on first, then the
/// ones with a choice lately turned off, each newest first (`rows` come
/// so). What does not fit the header is cut from the end.
fn chat_entries(rows: &[PrefsRow], now: i64) -> Vec<ChatEntry> {
    let (mut on, mut off) = (Vec::new(), Vec::new());
    for r in rows {
        let signed = |pref| match r.get(pref) {
            (_, at) if at <= 0 => 0,
            (true, at) => at,
            (false, at) if now - at <= CHAT_OFF_KEPT_SECS => -at,
            _ => 0,
        };
        let e = ChatEntry(r.chat_id.clone(), signed(Pref::Muted), signed(Pref::Pinned), signed(Pref::Archived));
        if Pref::ALL.iter().any(|&p| e.of(p) > 0) {
            on.push(e);
        } else if Pref::ALL.iter().any(|&p| e.of(p) != 0) {
            off.push(e);
        }
    }
    on.extend(off);
    on
}

/// `dm:<hex>`, `dm:<hex>@<server id>` (a private chat) or `group:<hex>`, as
/// this build names chats. A private chat's choices and tombstone travel
/// like a home one's; a device that does not have its server keeps them
/// by the id until the chat comes.
fn chat_id_ok(id: &str) -> bool {
    let hex64 = |h: &str| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if let Some(group) = id.strip_prefix("group:") {
        return hex64(group);
    }
    match messenger_store::chats::parse_dm_chat_id(id) {
        Some((peer, None)) => hex64(peer),
        Some((peer, Some(server))) => hex64(peer) && messenger_store::servers::is_id(server),
        None => false,
    }
}

/// The plain text of the header is kept under this: NIP-44 v2 takes
/// 65535 bytes, its entries (`HEADER_ENTRIES`) take up to some 27 KB of
/// it, and what is still over is cut by `fit_header`.
const HEADER_MAX_BYTES: usize = 60_000;

#[derive(Debug, Serialize, Deserialize)]
struct Page {
    v: u32,
    rev: u64,
    peers: Vec<Entry>,
    /// The relations of private chats that did not fit the header
    /// (`servers::PrivateEntry`); an older build reads the page without
    /// them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    private_peers: Vec<serde_json::Value>,
    /// The wills of the peers of this page's entries (`Will`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wills: Vec<serde_json::Value>,
}

/// NIP-44 v2 takes 65535 bytes of plain text: a page over this sheds its
/// wills from the end before it is sealed (a page of private relations
/// comes near it).
const PAGE_MAX_BYTES: usize = 64_000;

fn crypto(e: impl std::fmt::Display) -> MessengerError {
    MessengerError::Crypto(e.to_string())
}

async fn get_i64(store: &messenger_store::Store, key: &str) -> Result<i64> {
    Ok(settings::get(store, key).await?.and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// The entries of a revision in their order: every block first, then the
/// approved, then the declined, the expired and the removed, each newest
/// decision first (`rows` come newest first). A `none` nobody decided (a
/// stranger who wrote, a peer this device never touched) is no decision
/// and is left out; a `none` with a time is one — a contact removed, a
/// block lifted — and the one carrier of a removal that sent no signal (a
/// quiet add undone). A device that reads the list treats a peer it does
/// not name by what it has.
fn entries(rows: &[RelationRow], me: &str) -> Vec<Entry> {
    ordered(rows, me).into_iter().map(|(r, m)| Entry(r.peer_pubkey.clone(), letter(m).into(), u8::from(r.blocked), r.decided_at)).collect()
}

/// The rows that are decisions, in the order of a list: the blocks, the
/// approved, the rest, each as `rows` come (newest first). The home list
/// (`entries`) and the list of private chats (`servers::private_entries`)
/// both.
fn ordered<'a>(rows: &'a [RelationRow], me: &str) -> Vec<(&'a RelationRow, MyContact)> {
    let (mut blocks, mut approved, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    for r in rows {
        if r.peer_pubkey == me {
            continue;
        }
        let m = MyContact::parse(&r.my_contact).unwrap_or(MyContact::None);
        if !r.blocked && m == MyContact::None && r.decided_at <= 0 {
            continue;
        }
        if r.blocked {
            blocks.push((r, m));
        } else if m == MyContact::Approved {
            approved.push((r, m));
        } else {
            rest.push((r, m));
        }
    }
    blocks.extend(approved);
    blocks.extend(rest);
    blocks
}

/// The entries the header carries at most, of `peers` and `private_peers`
/// together: the rest of its room is the chats', the deletions', the
/// servers' and the invitations'. 250 entries of either list take up to
/// some 27 KB of the 60.
const HEADER_ENTRIES: usize = 250;

/// One page of entries: of the home list and of the list of private
/// chats.
type PageEntries = (Vec<Entry>, Vec<serde_json::Value>);

/// The entries cut into pages: the home ones first, then the ones of
/// private chats, `HEADER_ENTRIES` on the header and
/// `RELATIONS_PEERS_PER_PAGE` on every page after it; one page at least,
/// `RELATIONS_MAX_PAGES` at most. Whether every home entry found a place
/// (the list of private chats never says the list is whole).
fn paginate(home: Vec<Entry>, private: Vec<serde_json::Value>) -> (Vec<PageEntries>, bool) {
    let home_count = home.len();
    let mut pages: Vec<PageEntries> = vec![(Vec::new(), Vec::new())];
    let room = |page: usize| if page == 0 { HEADER_ENTRIES } else { RELATIONS_PEERS_PER_PAGE };
    let mut placed_home = 0;
    let add = |pages: &mut Vec<PageEntries>| -> Option<usize> {
        let last = pages.len() - 1;
        if pages[last].0.len() + pages[last].1.len() < room(last) {
            return Some(last);
        }
        if pages.len() == RELATIONS_MAX_PAGES {
            return None;
        }
        pages.push((Vec::new(), Vec::new()));
        Some(pages.len() - 1)
    };
    for e in home {
        let Some(n) = add(&mut pages) else { break };
        pages[n].0.push(e);
        placed_home += 1;
    }
    for e in private {
        let Some(n) = add(&mut pages) else { break };
        pages[n].1.push(e);
    }
    (pages, placed_home == home_count)
}

/// The header cut to `HEADER_MAX_BYTES`, when it is over: whole entries
/// from the end of the invitations (the ones that ended are last), then
/// of the servers, then of the relations of private chats; only after
/// them the removals of servers, the deletions and the choices of chats —
/// a deletion or a choice left out stays undone on my other devices,
/// where a server left out is only not brought. What was cut is logged.
fn fit_header(header: &mut Header) -> Result<String> {
    let mut plain = serde_json::to_string(&*header)?;
    let mut cut = [0usize; 8];
    while plain.len() > HEADER_MAX_BYTES {
        let which = if header.invites.pop().is_some() {
            0
        } else if header.wills.pop().is_some() {
            6
        } else if header.groups.pop().is_some() {
            7
        } else if header.servers.as_mut().and_then(Vec::pop).is_some() {
            1
        } else if header.private_peers.pop().is_some() {
            2
        } else if header.servers_removed.pop().is_some() {
            3
        } else if header.deleted.pop().is_some() {
            4
        } else if header.chats.pop().is_some() {
            5
        } else {
            break;
        };
        cut[which] += 1;
        plain = serde_json::to_string(&*header)?;
    }
    if cut.iter().any(|n| *n > 0) {
        let [invites, servers, private, removed, deleted, chats, wills, groups] = cut;
        eprintln!(
            "messenger relations: the header was over {HEADER_MAX_BYTES} bytes; cut {invites} invitations, {wills} wills of peers, \
             {groups} groups, \
             {servers} servers, {private} relations of private chats, {removed} removals of servers, {deleted} deletions, \
             {chats} choices of chats"
        );
    }
    Ok(plain)
}

impl DmService {
    /// The keys of the running session, when the host unlocked them.
    pub fn session_keys(&self) -> Option<Keys> {
        self.signer.read().unwrap().clone()
    }

    /// What this device knows of my list (set by the runtime after the
    /// fetch at the start of a session).
    pub fn relations_known(&self) -> Known {
        match self.relations.known.load(Ordering::SeqCst) {
            2 => Known::Complete,
            1 => Known::Partial,
            _ => Known::No,
        }
    }

    pub fn set_relations_known(&self, known: Known) {
        let v = match known {
            Known::No => 0,
            Known::Partial => 1,
            Known::Complete => 2,
        };
        self.relations.known.store(v, Ordering::SeqCst);
    }

    /// What the runtime wants called when a decision is taken here.
    pub fn set_relations_hook(&self, hook: Option<Arc<dyn Fn() + Send + Sync>>) {
        *self.relations.hook.write().unwrap() = hook;
    }

    /// A decision was taken here (an action of mine, a request that ran
    /// out, the settings), or a copy of one came: the list on the relays is
    /// behind this device. Kept in the store too, so a device closed before
    /// it published owes it at the next start. A copy marks as well: the
    /// device that decided may be an older build that publishes nothing.
    pub async fn mark_relations_owed(&self) -> Result<()> {
        settings::set(&self.store, KEY_OWED, "1").await?;
        self.relations.owed.store(true, Ordering::SeqCst);
        let hook = self.relations.hook.read().unwrap().clone();
        if let Some(h) = hook {
            h();
        }
        Ok(())
    }

    /// Whether the relays are behind this device.
    pub async fn relations_owed(&self) -> Result<bool> {
        Ok(self.relations.owed.load(Ordering::SeqCst) || settings::get(&self.store, KEY_OWED).await?.is_some())
    }

    /// The header this device published last (`KEY_PUBLISHED`).
    async fn published_header(&self) -> Result<Option<Published>> {
        let Some(v) = settings::get(&self.store, KEY_PUBLISHED).await? else { return Ok(None) };
        let mut it = v.split(' ');
        let (Some(rev), Some(created_at), Some(id)) = (it.next(), it.next(), it.next()) else { return Ok(None) };
        let (Ok(rev), Ok(created_at), Some(id)) = (rev.parse(), created_at.parse(), EventId::parse(id)) else { return Ok(None) };
        Ok(Some(Published { rev, created_at, id }))
    }

    /// The list is being published: a decision taken from here on marks
    /// again. Cleared before the build, so nothing taken during it is lost.
    pub async fn relations_settled(&self) -> Result<()> {
        self.relations.owed.store(false, Ordering::SeqCst);
        settings::delete(&self.store, KEY_OWED).await
    }

    /// The events of a new revision of the snapshot, from this device's
    /// rows and settings: the header first, then the pages, every one
    /// dated after whatever this device saw (`KEY_SEEN_CREATED_AT`), so the
    /// relays take them over the last. Returns the revision with them; the
    /// next one continues from it.
    pub async fn build_relations_snapshot(&self, keys: &Keys, now: i64) -> Result<(u64, Vec<WireEvent>)> {
        let rev = (get_i64(&self.store, KEY_REV).await?.max(0) as u64).saturating_add(1);
        let created_at = now.max(get_i64(&self.store, KEY_SEEN_CREATED_AT).await?.saturating_add(1));
        let me = keys.public_key();
        let rows = dm_relations::decided_rows(&self.store).await?;
        let private_rows = dm_relations::decided_private_rows(&self.store).await?;
        let mut wills = Wills::new();
        wills_of(&rows, &mut wills);
        wills_of(&private_rows, &mut wills);
        let private = servers::private_entries(&private_rows, &me.to_hex());
        let (pages, complete) = paginate(entries(&rows, &me.to_hex()), private);
        let s = self.dm_settings().await?;
        let settings_at = get_i64(&self.store, KEY_PRIVACY_DECIDED_AT).await?;
        let invites = self.invites_for_snapshot(now).await?;
        let chats = chat_entries(&chats::prefs(&self.store).await?, now);
        let deleted = chats::tombstones(&self.store).await?;
        let (my_servers, servers_removed) = servers::entries(&self.store).await?;
        let groups = self.group_entries().await?;
        let mut events = Vec::with_capacity(pages.len());
        let count = pages.len();
        for (n, (peers, private_peers)) in pages.into_iter().enumerate() {
            let piece_wills = piece_wills(&peers, &private_peers, &wills);
            let plain = if n == 0 {
                let mut header = Header {
                    v: SNAPSHOT_VERSION,
                    rev,
                    at: now,
                    pages: count,
                    complete,
                    dm_from: s.dm_from.clone(),
                    keep_days: s.keep_days,
                    settings_at,
                    invites: invites.clone(),
                    chats: chats.clone(),
                    deleted: deleted.clone(),
                    servers: Some(my_servers.clone()),
                    servers_removed: servers_removed.clone(),
                    private_peers,
                    wills: piece_wills,
                    groups: groups.clone(),
                    peers,
                };
                fit_header(&mut header)?
            } else {
                let mut page = Page { v: SNAPSHOT_VERSION, rev, peers, private_peers, wills: piece_wills };
                let mut plain = serde_json::to_string(&page)?;
                while plain.len() > PAGE_MAX_BYTES && page.wills.pop().is_some() {
                    plain = serde_json::to_string(&page)?;
                }
                plain
            };
            let content = nip44::encrypt(keys.secret_key(), &me, plain, nip44::Version::V2).map_err(crypto)?;
            let event = EventBuilder::new(Kind::from(KIND_RELATIONS), content)
                .tag(Tag::identifier(relations_d(n)))
                .custom_created_at(Timestamp::from_secs(created_at.max(0) as u64))
                .finalize(keys)
                .map_err(crypto)?;
            events.push(WireEvent {
                id: EventId::parse(&event.id.to_hex()).ok_or_else(|| crypto("bad event id"))?,
                json: serde_json::to_value(&event)?,
            });
        }
        if let Some(header) = events.first() {
            settings::set(&self.store, KEY_PUBLISHED, &format!("{rev} {created_at} {}", header.id.as_hex())).await?;
        }
        settings::set(&self.store, KEY_WILLS_PUBLISHED, "1").await?;
        settings::set(&self.store, KEY_SEEN_CREATED_AT, &created_at.to_string()).await?;
        settings::set(&self.store, KEY_REV, &rev.to_string()).await?;
        Ok((rev, events))
    }

    /// The wills of one piece, each read on its own: one this build cannot
    /// read, of an entry that is not there, of my own key or of a private
    /// server this device lacks is skipped alone.
    async fn apply_wills(
        &self,
        me: &str,
        peers: &[Entry],
        private_peers: &[serde_json::Value],
        wills: &[serde_json::Value],
    ) -> Result<(Vec<PubKey>, Vec<Effect>)> {
        let (mut changed, mut effects) = (Vec::new(), Vec::new());
        if wills.is_empty() {
            return Ok((changed, effects));
        }
        let mine: HashSet<String> = messenger_store::servers::private(&self.store).await?.into_iter().map(|s| s.id).collect();
        for v in wills {
            let Ok(Will(i, letter, at, mutual)) = serde_json::from_value::<Will>(v.clone()) else { continue };
            let Some(signal) = signal_of(&letter) else { continue };
            let (hex, server) = match peers.get(i) {
                Some(e) => (e.0.clone(), None),
                None => match private_peers.get(i - peers.len()).and_then(|v| serde_json::from_value::<servers::PrivateEntry>(v.clone()).ok()) {
                    Some(e) if mine.contains(&e.1) => (e.0, Some(e.1)),
                    _ => continue,
                },
            };
            let Some(peer) = PubKey::parse(&hex).filter(|p| p.as_hex() != me) else { continue };
            if let Some(fx) = self.apply_will_copy(&crate::peer::DmPeer::on(peer.clone(), server.as_deref()), signal, at, mutual != 0).await? {
                changed.push(peer);
                effects.extend(fx);
            }
        }
        Ok((changed, effects))
    }

    /// The groups I am in whose welcome to myself this device knows, the
    /// latest touched first, `GROUPS_MAX` at most.
    async fn group_entries(&self) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        for g in messenger_store::groups::list(&self.store).await? {
            if g.membership != JOINED {
                continue;
            }
            let Some(wrap) = messenger_store::groups::self_welcome(&self.store, &g.id).await? else { continue };
            let server = messenger_store::servers::of_group(&self.store, &g.id).await?.unwrap_or_default();
            out.push(serde_json::json!([g.id, wrap, server]));
            if out.len() == GROUPS_MAX {
                break;
            }
        }
        Ok(out)
    }

    /// The groups of a header this device is not in, each read on its own:
    /// one of a private server this device lacks is left out (its welcome
    /// would be held anyway), as is one this build cannot read.
    async fn groups_wanted(&self, entries: &[serde_json::Value]) -> Result<Vec<GroupWanted>> {
        let mine: HashSet<String> = messenger_store::servers::private(&self.store).await?.into_iter().map(|s| s.id).collect();
        let mut out = Vec::new();
        for v in entries {
            let Ok((group_id, wrap, server)) = serde_json::from_value::<(String, String, String)>(v.clone()) else { continue };
            let Some(wrap_id) = EventId::parse(&wrap) else { continue };
            if !chat_id_ok(&format!("group:{group_id}")) || (!server.is_empty() && !mine.contains(&server)) {
                continue;
            }
            let here = messenger_store::groups::get(&self.store, &group_id).await?;
            if here.is_some_and(|g| g.membership == JOINED) {
                continue;
            }
            out.push(GroupWanted { group_id, wrap_id, server: Some(server).filter(|s| !s.is_empty()) });
        }
        Ok(out)
    }

    /// A device that held the wills of its peers before this build could
    /// write them owes the list once, or a new device of mine still reads
    /// them from the history alone.
    pub async fn owe_wills_once(&self) -> Result<()> {
        if settings::get(&self.store, KEY_WILLS_PUBLISHED).await?.is_some() {
            return Ok(());
        }
        let mut wills = Wills::new();
        wills_of(&dm_relations::decided_rows(&self.store).await?, &mut wills);
        wills_of(&dm_relations::decided_private_rows(&self.store).await?, &mut wills);
        if wills.is_empty() {
            return Ok(());
        }
        self.mark_relations_owed().await
    }

    /// Pieces of the snapshot, from a fetch or the live subscription: the
    /// newest revision among them is the list; its entries are applied one
    /// by one (`apply_decision_copy`), its settings when newer than the
    /// ones here, and the set is complete when the header of that revision
    /// and every page it names are among the pieces. A piece of an older
    /// revision than the pieces beside it, or than the list this device
    /// already took (`KEY_REV`), is left alone: the header is the newest
    /// word on its own list, and a page a relay still holds under its `d`
    /// from a revision the newer one outgrew comes alone through the live
    /// subscription, after the newer header was taken. The effects are
    /// what the decisions did here (a chat gone, a held text shown, the
    /// book); a copy marks nothing as owed, unless it took the place of the
/// revision this device published (`Published::lost_to`).
    pub async fn apply_relations_snapshot(&self, keys: &Keys, pieces: &[Piece]) -> Result<(Applied, Vec<Effect>)> {
        let me = keys.public_key();
        let mut headers: Vec<(i64, Header)> = Vec::new();
        let mut pages: Vec<(usize, Page)> = Vec::new();
        let mut seen_at = 0i64;
        let mut lost = false;
        let published = self.published_header().await?;
        for p in pieces {
            seen_at = seen_at.max(p.created_at);
            let Ok(plain) = nip44::decrypt(keys.secret_key(), &me, &p.content) else { continue };
            if p.page == 0 {
                match serde_json::from_str::<Header>(&plain) {
                    Ok(h) if h.v == SNAPSHOT_VERSION => {
                        lost |= published.as_ref().is_some_and(|mine| mine.lost_to(p, h.rev));
                        headers.push((p.created_at, h));
                    }
                    _ => {}
                }
            } else {
                match serde_json::from_str::<Page>(&plain) {
                    Ok(g) if g.v == SNAPSHOT_VERSION => pages.push((p.page, g)),
                    _ => {}
                }
            }
        }
        // Even one this device cannot read holds its `d` on the relay: the
        // next publication is dated after it.
        if seen_at > get_i64(&self.store, KEY_SEEN_CREATED_AT).await? {
            settings::set(&self.store, KEY_SEEN_CREATED_AT, &seen_at.to_string()).await?;
        }
        // Another device built a revision without mine, and the relay keeps
        // it over mine: what I decided is on no relay. What it says is taken
        // below as any copy, and the union goes out as the next revision.
        if lost {
            self.mark_relations_owed().await?;
        }
        let rev = headers.iter().map(|(_, h)| h.rev).chain(pages.iter().map(|(_, g)| g.rev)).max();
        let Some(rev) = rev else { return Ok((Applied::default(), vec![])) };
        if rev < get_i64(&self.store, KEY_REV).await?.max(0) as u64 {
            return Ok((Applied::default(), vec![]));
        }
        let header = headers.iter().filter(|(_, h)| h.rev == rev).max_by_key(|(at, _)| *at).map(|(_, h)| h);
        let complete = header.is_some_and(|h| {
            h.complete && (1..h.pages.clamp(1, RELATIONS_MAX_PAGES)).all(|n| pages.iter().any(|(p, g)| *p == n && g.rev == rev))
        });
        let mut effects = Vec::new();
        let mut changed = Vec::new();
        let mut done: HashSet<&str> = HashSet::new();
        let of_rev = header.into_iter().flat_map(|h| h.peers.iter()).chain(pages.iter().filter(|(_, g)| g.rev == rev).flat_map(|(_, g)| g.peers.iter()));
        for Entry(hex, m, blocked, decided_at) in of_rev {
            if !done.insert(hex.as_str()) {
                continue;
            }
            let (Some(peer), Some(my_contact)) = (PubKey::parse(hex), contact_of(m)) else { continue };
            if peer.as_hex() == me.to_hex() {
                continue;
            }
            if let Some(fx) = self.apply_decision_copy(&crate::peer::DmPeer::home(peer.clone()), my_contact, *blocked != 0, *decided_at).await? {
                changed.push(peer);
                effects.extend(fx);
            }
        }
        let mut settings_changed = false;
        let mut chats_changed = false;
        let mut servers_changed = false;
        let mut groups_wanted = Vec::new();
        if let Some(h) = header {
            // My private servers, before the relations of their chats: a
            // removal newer than the server here takes it and its chats, a
            // server this device lacks comes (absence is no word).
            let s = self.apply_server_entries(h).await?;
            servers_changed = s.changed;
            chats_changed |= s.chats_gone;
            // After the servers: a group of a server that just came is
            // wanted too.
            groups_wanted = self.groups_wanted(&h.groups).await?;
            // A list that carries wills is of a build that writes them:
            // what this device takes from it owes no publication once more
            // (`owe_wills_once`); a will that changes here owes its own.
            if !h.wills.is_empty() {
                settings::set(&self.store, KEY_WILLS_PUBLISHED, "1").await?;
            }
            if s.owed {
                // An older build published over my servers: say them again.
                self.mark_relations_owed().await?;
            }
            // The relations of private chats: of the header, then of the
            // pages of its revision.
            let private: Vec<serde_json::Value> = h
                .private_peers
                .iter()
                .chain(pages.iter().filter(|(_, g)| g.rev == rev).flat_map(|(_, g)| g.private_peers.iter()))
                .cloned()
                .collect();
            let (private_changed, fx) = self.apply_private_entries(&me.to_hex(), &private).await?;
            changed.extend(private_changed);
            effects.extend(fx);
            // By union and the later revocation: nothing here is undone by
            // a list that lacks an invitation (absence is no word).
            self.apply_invite_entries(&h.invites).await?;
            // Choice by choice, the newer wins; a chat left out keeps
            // what it has.
            for e in &h.chats {
                if !chat_id_ok(&e.0) {
                    continue;
                }
                for pref in Pref::ALL {
                    let t = e.of(pref);
                    if t != 0 && chats::merge_pref(&self.store, &e.0, pref, t > 0, t.saturating_abs()).await? {
                        chats_changed = true;
                    }
                }
            }
            // A chat deleted there goes here, up to what it held then.
            let now = self.clock.now().secs();
            for (id, cleared_at, peer_upto) in &h.deleted {
                if id.starts_with("dm:") && chat_id_ok(id) && chats::delete_copy(&self.store, id, *cleared_at, *peer_upto, now).await? {
                    chats_changed = true;
                }
            }
            if h.settings_at > get_i64(&self.store, KEY_PRIVACY_DECIDED_AT).await? {
                if let Some(from) = DmFrom::parse(&h.dm_from) {
                    settings::set(&self.store, KEY_DM_FROM, from.as_str()).await?;
                    settings::set(&self.store, KEY_KEEP_DAYS, &h.keep_days.to_string()).await?;
                    settings::set(&self.store, KEY_PRIVACY_DECIDED_AT, &h.settings_at.to_string()).await?;
                    settings_changed = true;
                }
            }
        }
        // The wills of the peers, after my decisions and my servers: an
        // accept older than my removal is read against it
        // (`apply_will_copy`). A page that came alone brings its own.
        let of_rev = header
            .map(|h| (&h.peers, &h.private_peers, &h.wills))
            .into_iter()
            .chain(pages.iter().filter(|(_, g)| g.rev == rev).map(|(_, g)| (&g.peers, &g.private_peers, &g.wills)));
        for (peers, private_peers, wills) in of_rev {
            let (peers_changed, fx) = self.apply_wills(&me.to_hex(), peers, private_peers, wills).await?;
            for p in peers_changed {
                if !changed.contains(&p) {
                    changed.push(p);
                }
            }
            effects.extend(fx);
        }
        if rev > get_i64(&self.store, KEY_REV).await?.max(0) as u64 {
            settings::set(&self.store, KEY_REV, &rev.to_string()).await?;
        }
        Ok((Applied { rev: Some(rev), complete, changed, settings_changed, chats_changed, servers_changed, groups_wanted }, effects))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relationship::{Action, Relationship};
    use crate::requests::DmSettings;
    use messenger_contacts::{ContactService, ProfileService};
    use messenger_core::traits::SystemClock;
    use messenger_core::Clock;
    use messenger_store::Store;

    async fn device(keys: &Keys) -> DmService {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let dm = DmService::new(store, contacts, profiles, Arc::new(SystemClock));
        dm.set_signer(Some(keys.clone()));
        dm
    }

    fn peer(n: usize) -> PubKey {
        PubKey::parse(&format!("{n:064x}")).unwrap()
    }

    fn pieces(events: &[WireEvent], me: &PublicKey) -> Vec<Piece> {
        events.iter().map(|e| Piece::of_json(&e.json, me).expect("a piece of mine")).collect()
    }

    fn now() -> i64 {
        SystemClock.now().secs()
    }

    /// A row of a decision, as a device holds it.
    async fn decided(dm: &DmService, p: &PubKey, m: MyContact, blocked: bool, decided_at: i64) {
        let r = Relationship { my_contact: m, blocked, decided_at, ..Relationship::default() };
        dm.save_relation(p, &r).await.unwrap();
    }

    async fn relation(dm: &DmService, p: &PubKey) -> (MyContact, bool, i64) {
        let r = dm.load_relation(p).await.unwrap();
        (r.my_contact, r.blocked, r.decided_at)
    }

    #[tokio::test]
    async fn the_list_of_one_device_becomes_the_list_of_the_other() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let (x, y, z, w) = (peer(1), peer(2), peer(3), peer(4));
        a.act(&keys, &x, Action::Block).await.unwrap();
        a.act(&keys, &y, Action::Request).await.unwrap();
        decided(&a, &z, MyContact::Declined, false, 1_700_000_000).await;
        decided(&a, &w, MyContact::Expired, false, 1_700_000_001).await;
        a.dm_settings_set(DmSettings { dm_from: "contacts".into(), keep_days: 3 }).await.unwrap();
        assert!(a.relations_owed().await.unwrap(), "decisions were taken");

        a.relations_settled().await.unwrap();
        let (rev, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!((rev, events.len()), (1, 1), "four peers fit the header");
        let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
        assert_eq!(ev.kind.as_u16(), 30078);
        assert_eq!(ev.tags.identifier().as_deref(), Some("veydan.relations.v1"));
        assert!(ev.verify().is_ok());
        assert!(!ev.content.contains(x.as_hex()), "the relay sees no key of my list");
        let plain = nip44::decrypt(keys.secret_key(), &keys.public_key(), &ev.content).unwrap();
        let h: Header = serde_json::from_str(&plain).unwrap();
        assert_eq!((h.v, h.rev, h.pages, h.complete, h.dm_from.as_str(), h.keep_days), (1, 1, 1, true, "contacts", 3));
        assert!(h.settings_at > 0);
        let order: Vec<&str> = h.peers.iter().map(|e| e.1.as_str()).collect();
        assert_eq!(order, vec!["n", "a", "x", "d"], "the block first, then the approved, then the rest newest first");
        assert_eq!((h.peers[0].0.as_str(), h.peers[0].2), (x.as_hex(), 1));
        assert!(!a.relations_owed().await.unwrap(), "nothing new since the build");

        let (applied, effects) = b.apply_relations_snapshot(&keys, &pieces(&events, &keys.public_key())).await.unwrap();
        assert_eq!((applied.rev, applied.complete, applied.settings_changed), (Some(1), true, true));
        assert_eq!(applied.changed.len(), 4);
        assert!(!effects.is_empty(), "the decisions tell the page");
        for p in [&x, &y, &z, &w] {
            assert_eq!(relation(&b, p).await, relation(&a, p).await, "{}", p.as_hex());
        }
        assert_eq!(b.dm_settings().await.unwrap(), DmSettings { dm_from: "contacts".into(), keep_days: 3 });
        assert!(b.contacts.is_contact(&y).await.unwrap(), "the book follows the approval");
        assert!(!b.relations_owed().await.unwrap(), "a copy is no decision");
        assert_eq!(settings::get(b.store(), KEY_REV).await.unwrap().as_deref(), Some("1"), "the next publication continues from it");

        // The same set again changes nothing.
        let (again, effects) = b.apply_relations_snapshot(&keys, &pieces(&events, &keys.public_key())).await.unwrap();
        assert!(again.changed.is_empty() && effects.is_empty());
        // The author applying its own list changes nothing either.
        let (own, _) = a.apply_relations_snapshot(&keys, &pieces(&events, &keys.public_key())).await.unwrap();
        assert!(own.changed.is_empty() && !own.settings_changed);
    }

    #[tokio::test]
    async fn twelve_hundred_peers_are_three_pages_of_one_revision() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        for n in 1..=1200 {
            decided(&a, &peer(n), MyContact::Approved, false, 1_700_000_000 + n as i64).await;
        }
        let (rev, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!((rev, events.len()), (1, 3));
        let ds: Vec<String> = events
            .iter()
            .map(|e| serde_json::from_value::<Event>(e.json.clone()).unwrap().tags.identifier().unwrap())
            .collect();
        assert_eq!(ds, vec!["veydan.relations.v1", "veydan.relations.v1.1", "veydan.relations.v1.2"]);
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &keys.public_key())).await.unwrap();
        assert_eq!((applied.rev, applied.complete, applied.changed.len()), (Some(1), true, 1200));
        assert_eq!(dm_relations::decided_rows(b.store()).await.unwrap().len(), 1200);
        assert_eq!(relation(&b, &peer(1200)).await, (MyContact::Approved, false, 1_700_001_200));

        // Without one page the rest still applies, but the list is not
        // known in full.
        let c = device(&keys).await;
        let mut short = pieces(&events, &keys.public_key());
        short.remove(1);
        let (applied, _) = c.apply_relations_snapshot(&keys, &short).await.unwrap();
        assert_eq!((applied.rev, applied.complete, applied.changed.len()), (Some(1), false, 700));
    }

    #[tokio::test]
    async fn the_newest_revision_is_the_list_and_older_pages_are_left_alone() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let me = keys.public_key();
        for n in 1..=600 {
            decided(&a, &peer(n), MyContact::Approved, false, 1_700_000_000 + n as i64).await;
        }
        let (_, rev1) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!(rev1.len(), 2);
        // Then the rows of everyone but the first are no decision any more
        // (nobody's time on them): one page, a newer revision.
        for n in 2..=600 {
            decided(&a, &peer(n), MyContact::None, false, 0).await;
        }
        let (rev, rev2) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!((rev, rev2.len()), (2, 1));

        // A fresh device gets the new header and the old page the relay
        // still holds under `.1`: the page is of an older revision.
        let mut mixed = pieces(&rev2, &me);
        mixed.extend(pieces(&rev1[1..], &me));
        let (applied, _) = b.apply_relations_snapshot(&keys, &mixed).await.unwrap();
        assert_eq!((applied.rev, applied.complete, applied.changed.len()), (Some(2), true, 1));
        assert_eq!(relation(&b, &peer(1)).await.0, MyContact::Approved);
        assert_eq!(relation(&b, &peer(501)).await, (MyContact::None, false, 0), "not taken from the old page");
        // The same old page alone, as the live subscription brings it after
        // the header was taken: older than the list here, left alone.
        let (applied, effects) = b.apply_relations_snapshot(&keys, &pieces(&rev1[1..], &me)).await.unwrap();
        assert_eq!((applied, effects.len()), (Applied::default(), 0), "older than revision 2");
        assert_eq!(relation(&b, &peer(501)).await, (MyContact::None, false, 0));
        assert_eq!(settings::get(b.store(), KEY_REV).await.unwrap().as_deref(), Some("2"));

        // Two headers: the newest revision wins, whatever the order.
        let c = device(&keys).await;
        let mut both = pieces(&rev1[..1], &me);
        both.extend(pieces(&rev2, &me));
        let (applied, _) = c.apply_relations_snapshot(&keys, &both).await.unwrap();
        assert_eq!((applied.rev, applied.changed.len()), (Some(2), 1));
        assert_eq!(relation(&c, &peer(2)).await, (MyContact::None, false, 0), "the older header is not the list");
        // A page alone names its revision and applies.
        let d = device(&keys).await;
        let (applied, _) = d.apply_relations_snapshot(&keys, &pieces(&rev1[1..], &me)).await.unwrap();
        assert_eq!((applied.rev, applied.complete, applied.changed.len()), (Some(1), false, 600 - HEADER_ENTRIES));
    }

    #[tokio::test]
    async fn a_removal_without_a_signal_and_an_unblock_reach_the_other_device() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let me = keys.public_key();
        let (quiet, shut) = (peer(1), peer(2));
        a.act(&keys, &quiet, Action::Request).await.unwrap();
        a.act(&keys, &shut, Action::Block).await.unwrap();
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert_eq!(relation(&b, &quiet).await.0, MyContact::Approved);
        assert!(b.contacts.is_contact(&quiet).await.unwrap());
        assert!(relation(&b, &shut).await.1);

        // Undone on A: the quiet add sends no signal (nobody learns they
        // were ever added), so the list is its only way to B.
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let removed = a.act(&keys, &quiet, Action::Remove).await.unwrap();
        assert!(removed.outbounds.is_empty(), "a quiet add is undone quietly");
        a.act(&keys, &shut, Action::Unblock).await.unwrap();
        let (rev, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!(rev, 2);
        let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
        let plain = nip44::decrypt(keys.secret_key(), &me, &ev.content).unwrap();
        let h: Header = serde_json::from_str(&plain).unwrap();
        let mut letters: Vec<(&str, u8)> = h.peers.iter().map(|e| (e.1.as_str(), e.2)).collect();
        letters.sort();
        assert_eq!(letters, vec![("n", 0), ("n", 0)], "both are decisions of mine: none, no block, with their time");

        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert_eq!(applied.changed.len(), 2);
        assert_eq!(relation(&b, &quiet).await.0, MyContact::None, "removed here too");
        assert!(!b.contacts.is_contact(&quiet).await.unwrap(), "and out of the book");
        assert!(!relation(&b, &shut).await.1, "unblocked here too");
    }

    #[tokio::test]
    async fn an_entry_stands_by_its_time_and_the_stricter_of_one_second_wins() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let me = keys.public_key();
        let (older, same_block, same_yes, newer) = (peer(1), peer(2), peer(3), peer(4));
        decided(&a, &older, MyContact::Declined, false, 40).await;
        decided(&a, &same_block, MyContact::Approved, true, 50).await;
        decided(&a, &same_yes, MyContact::Approved, false, 50).await;
        decided(&a, &newer, MyContact::Expired, false, 70).await;
        decided(&b, &older, MyContact::Approved, false, 60).await;
        decided(&b, &same_block, MyContact::Approved, false, 50).await;
        decided(&b, &same_yes, MyContact::Declined, false, 50).await;
        decided(&b, &newer, MyContact::Approved, false, 60).await;
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert_eq!(applied.changed, vec![same_block.clone(), newer.clone()]);
        assert_eq!(relation(&b, &older).await, (MyContact::Approved, false, 60), "the older decision loses");
        assert_eq!(relation(&b, &same_block).await, (MyContact::Approved, true, 50), "the block of the same second wins");
        assert_eq!(relation(&b, &same_yes).await, (MyContact::Declined, false, 50), "a no is stricter than a yes");
        assert_eq!(relation(&b, &newer).await, (MyContact::Expired, false, 70), "the copy keeps its time, nothing is bumped");
    }

    #[tokio::test]
    async fn the_settings_apply_by_their_own_time_not_the_snapshots() {
        let keys = Keys::generate();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let me = keys.public_key();
        // B chose; A never touched the settings and publishes later.
        b.dm_settings_set(DmSettings { dm_from: "contacts".into(), keep_days: 30 }).await.unwrap();
        decided(&a, &peer(1), MyContact::Approved, false, 10).await;
        let (_, events) = a.build_relations_snapshot(&keys, now() + 100).await.unwrap();
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(!applied.settings_changed);
        assert_eq!(b.dm_settings().await.unwrap(), DmSettings { dm_from: "contacts".into(), keep_days: 30 });
        // B's list carries its choice to A.
        let (_, events) = b.build_relations_snapshot(&keys, now()).await.unwrap();
        let (applied, _) = a.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(applied.settings_changed);
        assert_eq!(a.dm_settings().await.unwrap(), DmSettings { dm_from: "contacts".into(), keep_days: 30 });
        // A header of another version, or not mine, is not read.
        let stranger = Keys::generate();
        let (_, events) = a.build_relations_snapshot(&stranger, now()).await.unwrap();
        assert!(pieces_of(&events, &me).is_empty(), "signed by another key");
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &stranger.public_key())).await.unwrap();
        assert_eq!(applied, Applied::default(), "cannot be read with my key");
    }

    fn pieces_of(events: &[WireEvent], me: &PublicKey) -> Vec<Piece> {
        events.iter().filter_map(|e| Piece::of_json(&e.json, me)).collect()
    }

    /// Muted on the computer, muted on the phone: a chat the phone has,
    /// and one it does not have yet, which is made muted. Each choice goes
    /// by its own time, and one turned off travels too.
    #[tokio::test]
    async fn the_choices_of_my_chats_reach_my_other_device() {
        let keys = Keys::generate();
        let me = keys.public_key();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let (here, later) = (chats::dm_chat_id(peer(1).as_hex(), None), chats::dm_chat_id(peer(2).as_hex(), None));
        let group = format!("group:{}", "ab".repeat(32));
        chats::ensure_dm(b.store(), peer(1).as_hex(), None).await.unwrap();
        a.set_muted(&here, true).await.unwrap();
        a.set_muted(&later, true).await.unwrap();
        a.set_pinned(&group, true).await.unwrap();
        assert!(a.relations_owed().await.unwrap(), "a choice is published");

        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let (applied, effects) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(applied.chats_changed && effects.is_empty());
        assert!(chats::get(b.store(), &here).await.unwrap().unwrap().muted);
        assert!(chats::is_muted(b.store(), &later).await.unwrap(), "before the chat is here");
        assert!(chats::ensure_dm(b.store(), peer(2).as_hex(), None).await.unwrap().muted, "and when it comes");
        assert!(!b.relations_owed().await.unwrap(), "a copy is no decision");
        // The same list again changes nothing.
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(!applied.chats_changed);

        // Let speak again on the phone, in the same second: a choice made
        // here goes after the one it replaces, and the computer follows.
        b.set_muted(&here, false).await.unwrap();
        let (_, events) = b.build_relations_snapshot(&keys, now()).await.unwrap();
        let (applied, _) = a.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(applied.chats_changed);
        assert!(!chats::is_muted(a.store(), &here).await.unwrap());
        assert!(chats::is_muted(a.store(), &later).await.unwrap(), "the other choices stand");

        // What is not a chat of this build is passed over.
        assert!(chat_id_ok(&here) && chat_id_ok(&group));
        assert!(chat_id_ok(&format!("{here}@0123456789abcdef")), "a private chat");
        for bad in [
            "dm:",
            "dm:AB",
            "group:../x",
            &format!("chan:{}", "ab".repeat(32)),
            &format!("{here}@"),
            &format!("{here}@S1"),
            &format!("{here}@0123456789abcdef0"),
            &format!("{group}@0123456789abcdef"),
        ] {
            assert!(!chat_id_ok(bad), "{bad}");
        }
    }

    /// A private chat's choices and its deletion travel as a home chat's,
    /// and the deletion takes that chat alone, not the home chat with the
    /// same person.
    #[tokio::test]
    async fn a_private_chat_is_chosen_and_deleted_on_every_device() {
        let keys = Keys::generate();
        let me = keys.public_key();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let server = "0123456789abcdef";
        let (home, private) = (chats::dm_chat_id(peer(1).as_hex(), None), chats::dm_chat_id(peer(1).as_hex(), Some(server)));
        for d in [&a, &b] {
            chats::ensure_dm(d.store(), peer(1).as_hex(), None).await.unwrap();
            chats::ensure_dm(d.store(), peer(1).as_hex(), Some(server)).await.unwrap();
            for chat in [&home, &private] {
                chats::touch(d.store(), chat, 100, Some("hi"), false).await.unwrap();
            }
        }
        a.set_pinned(&private, true).await.unwrap();
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(chats::get(b.store(), &private).await.unwrap().unwrap().pinned);
        assert!(!chats::get(b.store(), &home).await.unwrap().unwrap().pinned);

        a.delete_chat_by_me(&private).await.unwrap();
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert!(applied.chats_changed);
        assert!(chats::get(b.store(), &private).await.unwrap().is_none(), "the private chat goes");
        assert!(chats::get(b.store(), &home).await.unwrap().is_some(), "the home one stays");
        assert!(chats::tombstone(b.store(), &private).await.unwrap().0 > 0);
    }

    #[test]
    fn a_choice_turned_off_long_ago_is_left_out() {
        let now = 100 * 86_400;
        let row = |id: &str, muted: bool, muted_at: i64, pinned_at: i64| PrefsRow {
            chat_id: id.into(),
            muted,
            muted_at,
            pinned: true,
            pinned_at,
            ..Default::default()
        };
        let rows = [row("off-new", false, now - 10, 0), row("on", true, now - 20, 0), row("off-old", false, now - CHAT_OFF_KEPT_SECS - 1, 0), row("pin", false, 0, 5)];
        let ids: Vec<_> = chat_entries(&rows, now).into_iter().map(|e| (e.0, e.1, e.2)).collect();
        assert_eq!(ids, vec![("on".into(), now - 20, 0), ("pin".into(), 0, 5), ("off-new".into(), -(now - 10), 0)], "on first, then off");
    }

    #[tokio::test]
    async fn each_revision_is_dated_after_the_last_one_seen() {
        let keys = Keys::generate();
        let a = device(&keys).await;
        let me = keys.public_key();
        let ahead = now() + 3600;
        let (_, events) = a.build_relations_snapshot(&keys, ahead).await.unwrap();
        let at = |e: &WireEvent| serde_json::from_value::<Event>(e.json.clone()).unwrap().created_at.as_secs() as i64;
        assert_eq!(at(&events[0]), ahead);
        // Another device of mine with a clock ahead published: the next
        // revision of this one is still newer at the relay.
        let b = device(&keys).await;
        b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        let (rev, events) = b.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!((rev, at(&events[0])), (2, ahead + 1));
    }

    /// A permanent QR on a sign gathers hundreds of uses: its entry carries
    /// the newest hundred, so it pushes no other invitation out of the
    /// header. When page 0 is full of peers and the lists are full, what
    /// does not fit is cut from the end: the ones that ended first, whole,
    /// and every one that still admits stays.
    #[tokio::test]
    async fn a_well_used_invitation_does_not_push_the_others_out_of_the_header() {
        use crate::invites::{InviteKind, InviteNew, SNAPSHOT_INVITES, SNAPSHOT_USES_PER_INVITE};
        use messenger_store::dm_invites;
        let keys = Keys::generate();
        let me = keys.public_key();
        let a = device(&keys).await;
        let make = |secret: String, kind: InviteKind, max_uses: Option<u32>| InviteNew { secret, kind, label: String::new(), expires_at: None, max_uses, for_pubkey: None };
        let sign = a.invite_create(make("AAECAwQFBgcICQoLDA0O_w".into(), InviteKind::Qr, None)).await.unwrap();
        for n in 1..=800 {
            dm_invites::add_use(a.store(), &sign.row.id, &format!("{n:064x}"), now() - 1000 + n).await.unwrap();
        }
        let pulled = a.invite_create(make("BBECAwQFBgcICQoLDA0O_w".into(), InviteKind::Link, Some(1))).await.unwrap();
        a.invite_revoke(&pulled.row.id).await.unwrap();
        let spent = a.invite_create(make("CCECAwQFBgcICQoLDA0O_w".into(), InviteKind::Link, Some(1))).await.unwrap();
        dm_invites::add_use(a.store(), &spent.row.id, peer(1).as_hex(), now()).await.unwrap();

        let header = |events: &[WireEvent]| -> Header {
            let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
            let plain = nip44::decrypt(keys.secret_key(), &me, &ev.content).unwrap();
            assert!(plain.len() <= HEADER_MAX_BYTES, "{} bytes", plain.len());
            serde_json::from_str(&plain).unwrap()
        };
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let h = header(&events);
        let ids: Vec<&str> = h.invites.iter().map(|e| e[0].as_str().unwrap()).collect();
        assert_eq!(ids, [sign.row.id.as_str(), spent.row.id.as_str(), pulled.row.id.as_str()], "all three, the ended behind");
        assert_eq!(h.invites[0][5].as_array().unwrap().len(), SNAPSHOT_USES_PER_INVITE, "the newest hundred uses");
        let b = device(&keys).await;
        b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert_eq!(b.invite_get(&sign.row.id).await.unwrap().unwrap().used_by.len(), SNAPSHOT_USES_PER_INVITE);
        assert!(b.invite_get(&pulled.row.id).await.unwrap().unwrap().revoked());
        assert!(b.invite_get(&spent.row.id).await.unwrap().unwrap().used_up());

        // A full page of peers, fifty that admit (the sign among them) and
        // fifty that ended: the header keeps every one that admits, the
        // ended behind them.
        for n in 1..=RELATIONS_PEERS_PER_PAGE {
            decided(&a, &peer(n), MyContact::Approved, false, 1_700_000_000 + n as i64).await;
        }
        let mut open = vec![sign.row.id.clone()];
        for n in 0..SNAPSHOT_INVITES - 1 {
            open.push(a.invite_create(make(format!("{n:022}"), InviteKind::Link, None)).await.unwrap().row.id);
        }
        for n in 0..SNAPSHOT_INVITES - 2 {
            let gone = a.invite_create(make(format!("x{n:021}"), InviteKind::Link, None)).await.unwrap();
            a.invite_revoke(&gone.row.id).await.unwrap();
        }
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let h = header(&events);
        assert_eq!(h.peers.len(), HEADER_ENTRIES, "the rest on page 1");
        let ids: Vec<&str> = h.invites.iter().map(|e| e[0].as_str().unwrap()).collect();
        assert!(open.iter().all(|id| ids.contains(&id.as_str())), "every one that admits travels: {} of {}", ids.len(), open.len());
        // The header's entries are capped (`HEADER_ENTRIES`): with its
        // page 0 full, every invitation has room now. The ended ones come
        // last, the first cut when it is over (`fit_header`).
        let ended = ids.len() - open.len();
        assert_eq!(ended, SNAPSHOT_INVITES, "the ended travel too: {ended}");
        assert!(ids[..open.len()].iter().all(|id| open.iter().any(|o| o == id)), "the ones that admit first");
        assert_eq!(h.invites.iter().find(|e| e[0].as_str() == Some(sign.row.id.as_str())).unwrap()[5].as_array().unwrap().len(), SNAPSHOT_USES_PER_INVITE);
    }

    /// A row of a decision of mine with the peer's will beside it.
    async fn willed(dm: &DmService, p: &PubKey, m: MyContact, signal: PeerSignal, at: i64, mutual: bool) {
        let r = Relationship {
            my_contact: m,
            decided_at: at - 100,
            peer_signal: signal,
            last_signal_at: at,
            was_ever_mutual: mutual,
            ..Relationship::default()
        };
        dm.save_relation(p, &r).await.unwrap();
    }

    /// The owner's case: a new device read my list (my approvals) but its
    /// history did not reach back to the peers' accepts, so every older
    /// contact read as "request sent". The peer's will travels beside my
    /// decision now: the chat opens in full there, mutual, and nothing is
    /// owed for a copy.
    #[tokio::test]
    async fn the_peers_will_reaches_my_new_device() {
        use crate::relationship::{screen_mode, ScreenMode};
        let keys = Keys::generate();
        let me = keys.public_key();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let (x, y, z, w) = (peer(1), peer(2), peer(3), peer(4));
        willed(&a, &x, MyContact::Approved, PeerSignal::Approved, 1_700_000_000, true).await;
        willed(&a, &y, MyContact::Approved, PeerSignal::Left, 1_700_000_100, true).await;
        // No will to tell: no entry of it.
        decided(&a, &z, MyContact::Approved, false, 1_700_000_200).await;
        // A request of theirs I declined: my no cleared their accept.
        willed(&a, &w, MyContact::Declined, PeerSignal::None, 1_700_000_300, false).await;

        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
        let h: Header = serde_json::from_str(&nip44::decrypt(keys.secret_key(), &me, &ev.content).unwrap()).unwrap();
        assert_eq!(h.wills.len(), 2, "{:?}", h.wills);
        let at = |hex: &str| h.peers.iter().position(|e| e.0 == hex).unwrap();
        assert!(h.wills.contains(&serde_json::json!([at(x.as_hex()), "a", 1_700_000_000, 1])));
        assert!(h.wills.contains(&serde_json::json!([at(y.as_hex()), "l", 1_700_000_100, 1])));

        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        let rx = b.load_relation(&x).await.unwrap();
        assert_eq!(screen_mode(&rx), ScreenMode::FullChat);
        assert!(rx.was_ever_mutual);
        assert_eq!(rx.last_signal_at, 1_700_000_000);
        assert_eq!(screen_mode(&b.load_relation(&y).await.unwrap()), ScreenMode::RemovedByPeer);
        assert_eq!(screen_mode(&b.load_relation(&z).await.unwrap()), ScreenMode::RequestSent, "nothing known of them");
        assert!(applied.changed.contains(&x) && applied.changed.contains(&y));
        assert!(!b.relations_owed().await.unwrap(), "a copy is no decision");

        // A page carries the wills of its own entries.
        for n in 10..10 + HEADER_ENTRIES {
            willed(&a, &peer(n), MyContact::Approved, PeerSignal::Approved, 1_600_000_000, true).await;
        }
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        assert_eq!(events.len(), 2);
        let c = device(&keys).await;
        c.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        for n in [1, 10, 9 + HEADER_ENTRIES] {
            assert_eq!(screen_mode(&c.load_relation(&peer(n)).await.unwrap()), ScreenMode::FullChat, "peer {n}");
        }
    }

    /// The will of the peer is merged as their signal is: a newer word
    /// here stands, and an accept older than my removal taken here is not
    /// a request again (it was for the relation I ended).
    #[tokio::test]
    async fn an_older_will_does_not_undo_what_this_device_knows() {
        use crate::relationship::{screen_mode, ScreenMode};
        let keys = Keys::generate();
        let me = keys.public_key();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let (x, y) = (peer(1), peer(2));
        willed(&a, &x, MyContact::Approved, PeerSignal::Approved, 1_700_000_000, true).await;
        willed(&a, &y, MyContact::Approved, PeerSignal::Approved, 1_700_000_000, true).await;
        // Here: x left later; y was removed by me after their accept.
        willed(&b, &x, MyContact::Approved, PeerSignal::Left, 1_700_000_500, true).await;
        let removed = Relationship { decided_at: 1_700_000_900, ..Relationship::default() };
        b.save_relation(&y, &removed).await.unwrap();

        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        let rx = b.load_relation(&x).await.unwrap();
        assert_eq!((rx.peer_signal, rx.last_signal_at), (PeerSignal::Left, 1_700_000_500));
        let ry = b.load_relation(&y).await.unwrap();
        assert_eq!(ry.my_contact, MyContact::None, "my newer removal stands");
        assert_ne!(screen_mode(&ry), ScreenMode::RequestReceived, "their old accept is no request: {ry:?}");
        assert!(ry.was_ever_mutual, "we were contacts once");
    }

    /// A device that held the wills before this build owes the list once;
    /// the peer's will changing here owes it again; a device with nothing
    /// to tell owes nothing.
    #[tokio::test]
    async fn the_wills_are_published_once_and_when_they_change() {
        let keys = Keys::generate();
        let a = device(&keys).await;
        a.owe_wills_once().await.unwrap();
        assert!(!a.relations_owed().await.unwrap(), "nothing to tell");
        let x = peer(1);
        willed(&a, &x, MyContact::Approved, PeerSignal::Approved, 1_700_000_000, true).await;
        a.owe_wills_once().await.unwrap();
        assert!(a.relations_owed().await.unwrap(), "held before this build");
        a.relations_settled().await.unwrap();
        a.build_relations_snapshot(&keys, now()).await.unwrap();
        a.owe_wills_once().await.unwrap();
        assert!(!a.relations_owed().await.unwrap(), "once");

        // Their removal comes: the list owes their new will.
        let chat = crate::peer::DmPeer::home(x.clone()).chat_id();
        a.on_control(&chat, &crate::peer::DmPeer::home(x.clone()), false, Some("contact_removed"), now(), false, false).await.unwrap();
        assert_eq!(a.load_relation(&x).await.unwrap().peer_signal, PeerSignal::Left);
        assert!(a.relations_owed().await.unwrap(), "their will changed");
    }

    async fn group_row(dm: &DmService, n: usize, membership: &str) -> String {
        let id = format!("{:064x}", 0xabc0 + n);
        let row = messenger_store::groups::GroupRow {
            id: id.clone(),
            kind: "private".into(),
            name: format!("G{n}"),
            about: String::new(),
            picture: String::new(),
            relay_url: "wss://relay.example".into(),
            owner: String::new(),
            membership: membership.into(),
            my_role: None,
            members: 2,
            link_epoch: 0,
            created_at: 0,
            updated_at: 0,
            needs_update: false,
        };
        messenger_store::groups::upsert(dm.store(), &row).await.unwrap();
        id
    }

    /// The groups I am in travel by the welcome to myself each names; the
    /// device that reads the list wants the ones it is not in, and one it
    /// left or never heard of a welcome for is no word.
    #[tokio::test]
    async fn my_groups_travel_by_their_welcomes() {
        let keys = Keys::generate();
        let me = keys.public_key();
        let (a, b) = (device(&keys).await, device(&keys).await);
        let wrap = |n: u64| format!("{n:064x}");
        let (g1, g2, g3, g4) = (group_row(&a, 1, "joined").await, group_row(&a, 2, "joined").await, group_row(&a, 3, "left").await, group_row(&a, 4, "joined").await);
        for (g, n) in [(&g1, 1), (&g2, 2), (&g3, 3)] {
            messenger_store::groups::set_self_welcome(a.store(), g, &wrap(n)).await.unwrap();
        }
        let (_, events) = a.build_relations_snapshot(&keys, now()).await.unwrap();
        let ev: Event = serde_json::from_value(events[0].json.clone()).unwrap();
        let h: Header = serde_json::from_str(&nip44::decrypt(keys.secret_key(), &me, &ev.content).unwrap()).unwrap();
        let named: HashSet<String> = h.groups.iter().map(|e| e[0].as_str().unwrap().to_string()).collect();
        assert_eq!(named, HashSet::from([g1.clone(), g2.clone()]), "joined, with a welcome: {:?}", h.groups);
        assert!(h.groups.contains(&serde_json::json!([g1, wrap(1), ""])));
        let _ = (g3, g4);

        // The other device is in g2 already.
        group_row(&b, 2, "joined").await;
        let (applied, _) = b.apply_relations_snapshot(&keys, &pieces(&events, &me)).await.unwrap();
        assert_eq!(
            applied.groups_wanted,
            vec![GroupWanted { group_id: g1.clone(), wrap_id: EventId::parse(&wrap(1)).unwrap(), server: None }]
        );
        assert!(!b.relations_owed().await.unwrap(), "a copy is no decision");
    }

    fn home(n: usize) -> Vec<Entry> {
        (0..n).map(|i| Entry(format!("{i:064x}"), "a".into(), 0, 1_700_000_000)).collect()
    }

    fn private(n: usize) -> Vec<serde_json::Value> {
        (0..n).map(|i| serde_json::json!([format!("{i:064x}"), "0123456789abcdef", "a", 0, 1_700_000_000])).collect()
    }

    /// The header carries `HEADER_ENTRIES` of both lists, every page after
    /// it `RELATIONS_PEERS_PER_PAGE`: home first, then the private chats.
    /// The list is whole when every home entry found a place.
    #[test]
    fn the_relations_of_private_chats_go_on_the_pages_after_the_home_ones() {
        let (pages, complete) = paginate(home(600), private(300));
        let sizes: Vec<(usize, usize)> = pages.iter().map(|(h, p)| (h.len(), p.len())).collect();
        assert_eq!(sizes, [(250, 0), (350, 150), (0, 150)]);
        assert!(complete);
        let (pages, complete) = paginate(Vec::new(), Vec::new());
        assert_eq!((pages.len(), complete), (1, true), "one header, empty");
        let (pages, complete) = paginate(home(10), private(5));
        assert_eq!((pages.len(), pages[0].0.len(), pages[0].1.len(), complete), (1, 10, 5, true));
        // More than the pages hold: the private ones go first, and only a
        // home entry left out says the list is not whole.
        let room = HEADER_ENTRIES + (RELATIONS_MAX_PAGES - 1) * RELATIONS_PEERS_PER_PAGE;
        let (pages, complete) = paginate(home(room - 10), private(300));
        assert_eq!((pages.len(), complete), (RELATIONS_MAX_PAGES, true));
        assert_eq!(pages.iter().map(|(_, p)| p.len()).sum::<usize>(), 10);
        let (_, complete) = paginate(home(room + 1), Vec::new());
        assert!(!complete);
    }

    /// Over the bytes of a header, the invitations go first, then the
    /// servers, then the relations of private chats; the removals of
    /// servers, the deletions and the choices of chats only after them.
    #[test]
    fn a_header_over_its_bytes_is_cut_in_its_order() {
        let header = |chats: usize, private_peers: usize| Header {
            v: SNAPSHOT_VERSION,
            rev: 1,
            at: 1,
            pages: 1,
            complete: true,
            dm_from: "everyone".into(),
            keep_days: 30,
            settings_at: 0,
            invites: (0..20).map(|i| serde_json::json!([format!("{i:064x}"), "x".repeat(200)])).collect(),
            chats: (0..chats).map(|i| ChatEntry(format!("dm:{i:064x}"), 1_700_000_000, 0, 0)).collect(),
            deleted: (0..100).map(|i| (format!("dm:{:064x}", i + 50_000), 1_700_000_000, 1_700_000_000)).collect(),
            servers: Some((0..32).map(|i| serde_json::json!({ "id": format!("{i:016x}"), "relay_url": "x".repeat(300) })).collect()),
            servers_removed: (0..64).map(|i| serde_json::json!([format!("{i:016x}"), 1_700_000_000])).collect(),
            private_peers: private(private_peers),
            wills: Vec::new(),
            groups: Vec::new(),
            peers: home(HEADER_ENTRIES),
        };
        // Fits: nothing is cut.
        let mut h = header(10, 0);
        fit_header(&mut h).unwrap();
        assert_eq!((h.invites.len(), h.servers.as_ref().unwrap().len(), h.chats.len()), (20, 32, 10));
        // Over by the choices of chats: the invitations, the servers and
        // the private relations go, the choices, the deletions and the
        // removals stay whole.
        let mut h = header(250, 100);
        let plain = fit_header(&mut h).unwrap();
        assert!(plain.len() <= HEADER_MAX_BYTES, "{}", plain.len());
        assert_eq!((h.chats.len(), h.deleted.len(), h.servers_removed.len()), (250, 100, 64));
        assert!(h.invites.is_empty(), "the invitations first");
        assert!(h.servers.as_ref().unwrap().len() < 32, "then the servers");
        assert_eq!(h.peers.len(), HEADER_ENTRIES, "the peers are never cut");
        // A header nothing can be cut of any more is left as it is.
        let mut h = header(5000, 0);
        let plain = fit_header(&mut h).unwrap();
        assert!(h.invites.is_empty() && h.servers.as_ref().unwrap().is_empty() && h.deleted.is_empty());
        assert!(plain.len() <= HEADER_MAX_BYTES, "the choices of chats last: {} left", h.chats.len());
    }
}

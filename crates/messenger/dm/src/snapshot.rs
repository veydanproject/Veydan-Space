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

use crate::relationship::{DmFrom, MyContact};
use crate::requests::{KEY_DM_FROM, KEY_KEEP_DAYS};
use crate::service::DmService;
use messenger_core::outbound::WireEvent;
use messenger_core::relations::{relations_d, relations_page, KIND_RELATIONS, RELATIONS_MAX_PAGES, RELATIONS_PEERS_PER_PAGE};
use messenger_core::{Effect, EventId, MessengerError, PubKey, Result};
use messenger_store::dm_relations::{self, RelationRow};
use messenger_store::settings;
use nostr::nips::nip44;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

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
/// header (`page` 0) or a page.
#[derive(Clone, Debug)]
pub struct Piece {
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
        Some(Piece { page, created_at: e.created_at.as_secs() as i64, content: e.content })
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
    peers: Vec<Entry>,
}

/// The plain text of the header is kept under this: NIP-44 v2 takes
/// 65535 bytes, the peers of page 0 take up to 45 KB of it, and the
/// invitations are cut from the end to fit (the ones that ended go first).
const HEADER_MAX_BYTES: usize = 60_000;

#[derive(Debug, Serialize, Deserialize)]
struct Page {
    v: u32,
    rev: u64,
    peers: Vec<Entry>,
}

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
    let (mut blocks, mut approved, mut rest) = (Vec::new(), Vec::new(), Vec::new());
    for r in rows {
        if r.peer_pubkey == me {
            continue;
        }
        let m = MyContact::parse(&r.my_contact).unwrap_or(MyContact::None);
        if !r.blocked && m == MyContact::None && r.decided_at <= 0 {
            continue;
        }
        let e = Entry(r.peer_pubkey.clone(), letter(m).into(), u8::from(r.blocked), r.decided_at);
        if r.blocked {
            blocks.push(e);
        } else if m == MyContact::Approved {
            approved.push(e);
        } else {
            rest.push(e);
        }
    }
    blocks.extend(approved);
    blocks.extend(rest);
    blocks
}

/// The entries cut into pages; one page at least, `RELATIONS_MAX_PAGES` at
/// most, and whether every entry found a place.
fn paginate(entries: Vec<Entry>) -> (Vec<Vec<Entry>>, bool) {
    let mut pages: Vec<Vec<Entry>> = entries.chunks(RELATIONS_PEERS_PER_PAGE).map(<[Entry]>::to_vec).collect();
    if pages.is_empty() {
        pages.push(Vec::new());
    }
    let complete = pages.len() <= RELATIONS_MAX_PAGES;
    pages.truncate(RELATIONS_MAX_PAGES);
    (pages, complete)
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
        let (pages, complete) = paginate(entries(&rows, &me.to_hex()));
        let s = self.dm_settings().await?;
        let settings_at = get_i64(&self.store, KEY_PRIVACY_DECIDED_AT).await?;
        let invites = self.invites_for_snapshot(now).await?;
        let mut events = Vec::with_capacity(pages.len());
        let count = pages.len();
        for (n, peers) in pages.into_iter().enumerate() {
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
                    peers,
                };
                let mut plain = serde_json::to_string(&header)?;
                while plain.len() > HEADER_MAX_BYTES && header.invites.pop().is_some() {
                    plain = serde_json::to_string(&header)?;
                }
                plain
            } else {
                serde_json::to_string(&Page { v: SNAPSHOT_VERSION, rev, peers })?
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
        settings::set(&self.store, KEY_REV, &rev.to_string()).await?;
        settings::set(&self.store, KEY_SEEN_CREATED_AT, &created_at.to_string()).await?;
        Ok((rev, events))
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
    /// book); a copy marks nothing as owed.
    pub async fn apply_relations_snapshot(&self, keys: &Keys, pieces: &[Piece]) -> Result<(Applied, Vec<Effect>)> {
        let me = keys.public_key();
        let mut headers: Vec<(i64, Header)> = Vec::new();
        let mut pages: Vec<(usize, Page)> = Vec::new();
        let mut seen_at = 0i64;
        for p in pieces {
            seen_at = seen_at.max(p.created_at);
            let Ok(plain) = nip44::decrypt(keys.secret_key(), &me, &p.content) else { continue };
            if p.page == 0 {
                match serde_json::from_str::<Header>(&plain) {
                    Ok(h) if h.v == SNAPSHOT_VERSION => headers.push((p.created_at, h)),
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
            if let Some(fx) = self.apply_decision_copy(&peer, my_contact, *blocked != 0, *decided_at).await? {
                changed.push(peer);
                effects.extend(fx);
            }
        }
        let mut settings_changed = false;
        if let Some(h) = header {
            // By union and the later revocation: nothing here is undone by
            // a list that lacks an invitation (absence is no word).
            self.apply_invite_entries(&h.invites).await?;
            if h.settings_at > get_i64(&self.store, KEY_PRIVACY_DECIDED_AT).await? {
                if let Some(from) = DmFrom::parse(&h.dm_from) {
                    settings::set(&self.store, KEY_DM_FROM, from.as_str()).await?;
                    settings::set(&self.store, KEY_KEEP_DAYS, &h.keep_days.to_string()).await?;
                    settings::set(&self.store, KEY_PRIVACY_DECIDED_AT, &h.settings_at.to_string()).await?;
                    settings_changed = true;
                }
            }
        }
        if rev > get_i64(&self.store, KEY_REV).await?.max(0) as u64 {
            settings::set(&self.store, KEY_REV, &rev.to_string()).await?;
        }
        Ok((Applied { rev: Some(rev), complete, changed, settings_changed }, effects))
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
        assert_eq!((applied.rev, applied.complete, applied.changed.len()), (Some(1), false, 100));
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

        // Page 0 full of peers, fifty that admit (the sign among them) and
        // fifty that ended: the header keeps every one that admits and cuts
        // the ended from the end.
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
        assert_eq!(h.peers.len(), RELATIONS_PEERS_PER_PAGE);
        let ids: Vec<&str> = h.invites.iter().map(|e| e[0].as_str().unwrap()).collect();
        assert!(open.iter().all(|id| ids.contains(&id.as_str())), "every one that admits travels: {} of {}", ids.len(), open.len());
        let ended = ids.len() - open.len();
        assert!(ended < SNAPSHOT_INVITES, "the ended were cut to fit: {ended} travel");
        assert_eq!(h.invites.iter().find(|e| e[0].as_str() == Some(sign.row.id.as_str())).unwrap()[5].as_array().unwrap().len(), SNAPSHOT_USES_PER_INVITE);
    }
}

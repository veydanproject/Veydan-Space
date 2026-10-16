// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Invitations to write to me without a request: one secret with a term,
//! a number of uses and, when it is for one person, their key. A link, a
//! code and a QR are presets of the same thing (the facade of the runtime
//! chooses the secret and the defaults); here is what an invitation
//! admits and what it costs to guess one.
//!
//! The peer comes in with `dm_accept` carrying the secret
//! (`Envelope::control_invite`). The secret is looked up by its sha256,
//! the one name every device of mine and the snapshot of my relations
//! give it. The uses are a set of keys: the same key again is the same
//! use (a replay, a copy from another device), and two of my devices that
//! each let somebody in agree by union (internal/messenger-wire.md §3,
//! "Приглашения").
//!
//! Guessing: one budget per receiver, not per key, since an attempt names
//! no invitation and a counter per key would not slow a fresh key down.
//! Ten wrong secrets within an hour, from anybody, and every invitation
//! pauses for an hour: a secret is not checked at all, and the signal
//! takes the ordinary road of a stranger's. A million codes at ten an
//! hour is eleven years; a link's 16 bytes are not guessed at all. Each
//! device keeps its own budget and pause: whichever of them answers, the
//! peer is in.

use crate::service::DmService;
use messenger_core::traits::UiEvent;
use messenger_core::{Effect, MessengerError, PubKey, Result};
use messenger_store::dm_invites::{self, InviteRow};
use messenger_store::settings;
use sha2::{Digest, Sha256};

/// `{"until": <secs>}` — somebody is guessing: the invitations are paused
/// until then. A signal to the page, which shows a line of it.
pub const UI_EVENT_DM_INVITE_PAUSED: &str = "dm.invite_paused";

/// `<wrong>,<window start>`: how many wrong secrets came since the start
/// of the running hour.
pub const KEY_INVITE_FAILS: &str = "invites.fails";
/// Until when the invitations are paused; absent or past: not paused.
pub const KEY_INVITE_PAUSED_UNTIL: &str = "invites.paused_until";

/// Wrong secrets within `INVITE_FAIL_WINDOW_SECS` that pause the invitations.
pub const INVITE_FAILS_PER_WINDOW: u32 = 10;
pub const INVITE_FAIL_WINDOW_SECS: i64 = 3600;
/// How long the pause lasts.
pub const INVITE_PAUSE_SECS: i64 = 3600;

/// A label longer than this is cut: it is for my eye in the list.
pub const MAX_INVITE_LABEL_CHARS: usize = 100;

/// The snapshot carries this many invitations that still admit, newest
/// first, and as many that ended but are worth a word (below).
pub const SNAPSHOT_INVITES: usize = 50;
/// An invitation that was revoked, or used up, travels in the snapshot
/// this long after it, so that my other devices stop admitting by it;
/// one that ran out needs no word (their clocks say so).
pub const SNAPSHOT_INVITE_TOLD_SECS: i64 = 30 * 86_400;
/// An entry of the snapshot carries this many uses at most, the newest:
/// a use is some 80 bytes, a permanent QR on a sign gathers hundreds,
/// and the header has 60 000 bytes for every invitation together
/// (`crate::snapshot::HEADER_MAX_BYTES`). What is cut is the other
/// devices' count of a well-used invitation without a limit, and the
/// word that a key came in by it long ago; a limit above this many is
/// kept loosely between devices (no preset has one).
pub const SNAPSHOT_USES_PER_INVITE: usize = 100;
/// Accepts with a secret that were refused lately, by rumor id: the
/// copy of one from another relay is the same try, not a second one.
/// Nothing of a refused wrap is stored, so the memory is this ring, per
/// process.
pub const REFUSED_REMEMBERED: usize = 64;

/// What an invitation is: the presets of the facade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteKind {
    Link,
    Code,
    Qr,
}

impl InviteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InviteKind::Link => "link",
            InviteKind::Code => "code",
            InviteKind::Qr => "qr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "link" => Some(InviteKind::Link),
            "code" => Some(InviteKind::Code),
            "qr" => Some(InviteKind::Qr),
            _ => None,
        }
    }
}

/// The id of an invitation: hex sha256 of its secret.
pub fn invite_id(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// An invitation to make, as the facade decided it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteNew {
    pub secret: String,
    pub kind: InviteKind,
    pub label: String,
    pub expires_at: Option<i64>,
    pub max_uses: Option<u32>,
    pub for_pubkey: Option<PubKey>,
}

/// An invitation with who used it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub row: InviteRow,
    /// `(hex, at)`, in the order they came.
    pub used_by: Vec<(String, i64)>,
}

impl Invite {
    pub fn revoked(&self) -> bool {
        self.row.revoked_at.is_some()
    }

    pub fn expired(&self, now: i64) -> bool {
        self.row.expires_at.is_some_and(|e| e <= now)
    }

    pub fn used_up(&self) -> bool {
        self.row.max_uses.is_some_and(|m| self.used_by.len() as i64 >= m)
    }

    /// Whether it still lets somebody new in.
    pub fn active(&self, now: i64) -> bool {
        !self.revoked() && !self.expired(now) && !self.used_up()
    }

    /// Whether it lets `peer` in now: it is active, or `peer` came in by it
    /// already; and it is theirs when it is for one. A key in `used_by` is
    /// in whatever the term, a revocation or the number say by now: their
    /// use is a fact recorded by me or my other device, and their accept
    /// again (a copy from another relay, the copy my other device reads as
    /// history after the snapshot told it of the use) is the same coming
    /// in, never a guess.
    pub fn admits(&self, peer: &PubKey, now: i64) -> bool {
        if self.row.for_pubkey.as_deref().is_some_and(|f| f != peer.as_hex()) {
            return false;
        }
        self.used_by.iter().any(|(p, _)| p == peer.as_hex()) || self.active(now)
    }
}

/// `<count>,<window start>` of `KEY_INVITE_FAILS`.
fn parse_fails(v: Option<&str>) -> (u32, i64) {
    v.and_then(|s| {
        let (c, w) = s.split_once(',')?;
        Some((c.parse().ok()?, w.parse().ok()?))
    })
    .unwrap_or((0, 0))
}

impl DmService {
    async fn invite_with_uses(&self, row: InviteRow) -> Result<Invite> {
        let used_by = dm_invites::uses(&self.store, &row.id).await?;
        Ok(Invite { row, used_by })
    }

    /// Make an invitation, dated now. The same secret made twice is the
    /// same invitation (its id is the secret's). Decided here: the
    /// snapshot of my relations owes the relays a word.
    pub async fn invite_create(&self, new: InviteNew) -> Result<Invite> {
        if new.secret.is_empty() || new.secret.chars().count() > messenger_core::envelope::MAX_INVITE_CHARS {
            return Err(MessengerError::Invalid("invite_secret".into()));
        }
        let now = self.clock.now().secs();
        let label: String = new.label.chars().filter(|c| !c.is_control()).take(MAX_INVITE_LABEL_CHARS).collect::<String>().trim().to_string();
        let row = InviteRow {
            id: invite_id(&new.secret),
            secret: new.secret,
            kind: new.kind.as_str().into(),
            label,
            created_at: now,
            expires_at: new.expires_at,
            max_uses: new.max_uses.map(i64::from),
            for_pubkey: new.for_pubkey.map(|p| p.as_hex().to_string()),
            revoked_at: None,
        };
        dm_invites::create(&self.store, &row).await?;
        // A code shown every quarter of an hour would pile up: what ran out
        // a month ago is forgotten with its uses.
        dm_invites::prune_expired(&self.store, now - SNAPSHOT_INVITE_TOLD_SECS).await?;
        self.mark_relations_owed().await?;
        let row = dm_invites::get(&self.store, &row.id).await?.ok_or_else(|| MessengerError::Storage("invite vanished".into()))?;
        self.invite_with_uses(row).await
    }

    pub async fn invite_get(&self, id: &str) -> Result<Option<Invite>> {
        match dm_invites::get(&self.store, id).await? {
            Some(row) => Ok(Some(self.invite_with_uses(row).await?)),
            None => Ok(None),
        }
    }

    /// Every invitation: the ones that still admit first, newest first,
    /// then the rest, newest first.
    pub async fn invite_list(&self) -> Result<Vec<Invite>> {
        let now = self.clock.now().secs();
        let mut out = Vec::new();
        for row in dm_invites::list(&self.store).await? {
            out.push(self.invite_with_uses(row).await?);
        }
        out.sort_by_key(|i| (!i.active(now), std::cmp::Reverse(i.row.created_at)));
        Ok(out)
    }

    /// Revoke it here and, by the snapshot, everywhere. One revoked
    /// already stays as it is; an unknown id is refused (`invite_unknown`).
    pub async fn invite_revoke(&self, id: &str) -> Result<()> {
        if dm_invites::get(&self.store, id).await?.is_none() {
            return Err(MessengerError::Invalid("invite_unknown".into()));
        }
        if dm_invites::revoke(&self.store, id, self.clock.now().secs()).await? {
            self.mark_relations_owed().await?;
        }
        Ok(())
    }

    /// Until when the invitations are paused on this device (somebody
    /// guessed too often); `None` when they are not.
    pub async fn invite_paused_until(&self) -> Result<Option<i64>> {
        let until: i64 = settings::get(&self.store, KEY_INVITE_PAUSED_UNTIL).await?.and_then(|v| v.parse().ok()).unwrap_or(0);
        Ok((until > self.clock.now().secs()).then_some(until))
    }

    /// `peer` came with `secret` on their accept, the wrap `rumor_id`: the
    /// id of the invitation that lets them in, with their use recorded,
    /// or `None`. A wrong secret costs the budget, and the tenth within
    /// the hour pauses the invitations (the event goes to the page); while
    /// paused nothing is looked up. The same wrap refused again (its copy
    /// from another relay) costs nothing more. A blocked key, or one the
    /// network bans, is let in by nothing, and its right secret is no guess.
    pub async fn redeem_invite(&self, peer: &PubKey, secret: &str, rumor_id: &str) -> Result<(Option<String>, Vec<Effect>)> {
        let now = self.clock.now().secs();
        if self.invite_paused_until().await?.is_some() {
            return Ok((None, vec![]));
        }
        let found = self.invite_get(&invite_id(secret)).await?;
        if let Some(invite) = found.filter(|i| i.admits(peer, now)) {
            if self.load_relation(peer).await?.blocked || self.is_banned(peer).await? {
                return Ok((None, vec![]));
            }
            if dm_invites::add_use(&self.store, &invite.row.id, peer.as_hex(), now).await? {
                self.mark_relations_owed().await?;
            }
            return Ok((Some(invite.row.id), vec![]));
        }
        if !self.relations.remember_refused(rumor_id) {
            return Ok((None, vec![]));
        }
        Ok((None, self.invite_failed(now).await?))
    }

    /// One wrong secret: counted within the running hour; the tenth pauses.
    async fn invite_failed(&self, now: i64) -> Result<Vec<Effect>> {
        let (count, since) = parse_fails(settings::get(&self.store, KEY_INVITE_FAILS).await?.as_deref());
        let (count, since) = if now - since >= INVITE_FAIL_WINDOW_SECS { (1, now) } else { (count + 1, since) };
        if count >= INVITE_FAILS_PER_WINDOW {
            let until = now + INVITE_PAUSE_SECS;
            settings::set(&self.store, KEY_INVITE_PAUSED_UNTIL, &until.to_string()).await?;
            settings::delete(&self.store, KEY_INVITE_FAILS).await?;
            return Ok(vec![Effect::Emit(UiEvent {
                name: UI_EVENT_DM_INVITE_PAUSED.into(),
                payload: serde_json::json!({ "until": until }),
            })]);
        }
        settings::set(&self.store, KEY_INVITE_FAILS, &format!("{count},{since}")).await?;
        Ok(vec![])
    }

    /// The invitations as the header of the snapshot carries them
    /// (`[id, secret, kind, expires_at, max_uses, used_by, for,
    /// revoked_at, created_at, label]`): the ones that admit, newest
    /// first, then the ones that ended lately (revoked, or used up) and
    /// are not past their term, so that a device that still holds them
    /// as open hears of it. `SNAPSHOT_INVITES` of each at most.
    pub async fn invites_for_snapshot(&self, now: i64) -> Result<Vec<serde_json::Value>> {
        let mut active = Vec::new();
        let mut ended = Vec::new();
        for i in self.invite_list().await? {
            if i.expired(now) {
                continue;
            }
            if i.active(now) {
                active.push(i);
            } else {
                let ended_at = i.row.revoked_at.unwrap_or_else(|| i.used_by.iter().map(|(_, at)| *at).max().unwrap_or(i.row.created_at));
                if now - ended_at < SNAPSHOT_INVITE_TOLD_SECS {
                    ended.push(i);
                }
            }
        }
        active.truncate(SNAPSHOT_INVITES);
        ended.truncate(SNAPSHOT_INVITES);
        Ok(active.into_iter().chain(ended).map(|i| entry_of(&i)).collect())
    }

    /// The invitations of a snapshot of mine, merged here: an unknown one
    /// is taken, a known one gains the uses it lacked and the later
    /// revocation. Entries this build cannot read are skipped. Returns
    /// whether anything changed here.
    pub async fn apply_invite_entries(&self, entries: &[serde_json::Value]) -> Result<bool> {
        let mut changed = false;
        for e in entries {
            let Some((row, uses)) = row_of_entry(e) else { continue };
            changed |= dm_invites::merge(&self.store, &row, &uses).await?;
        }
        Ok(changed)
    }
}

/// The entry with the newest `SNAPSHOT_USES_PER_INVITE` uses (`used_by`
/// is in the order they came).
fn entry_of(i: &Invite) -> serde_json::Value {
    let r = &i.row;
    let from = i.used_by.len().saturating_sub(SNAPSHOT_USES_PER_INVITE);
    serde_json::json!([
        r.id,
        r.secret,
        r.kind,
        r.expires_at,
        r.max_uses,
        i.used_by[from..].iter().map(|(p, at)| serde_json::json!([p, at])).collect::<Vec<_>>(),
        r.for_pubkey,
        r.revoked_at,
        r.created_at,
        r.label,
    ])
}

/// An entry as `entry_of` writes it, checked: a secret or an id of another
/// shape, a kind this build does not know, a use that is no key, and the
/// entry is left alone. `created_at` and `label` may be missing (a shorter
/// writer): the entry is dated by nothing and named by nothing.
fn row_of_entry(e: &serde_json::Value) -> Option<(InviteRow, Vec<(String, i64)>)> {
    let a = e.as_array()?;
    if a.len() < 8 {
        return None;
    }
    let id = a[0].as_str()?;
    let secret = a[1].as_str()?;
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || invite_id(secret) != id {
        return None;
    }
    let kind = InviteKind::parse(a[2].as_str()?)?;
    let expires_at = a[3].as_i64();
    let max_uses = a[4].as_i64().filter(|m| *m >= 0);
    let mut uses = Vec::new();
    for u in a[5].as_array()? {
        let pair = u.as_array()?;
        let peer = pair.first()?.as_str()?;
        PubKey::parse(peer)?;
        uses.push((peer.to_string(), pair.get(1).and_then(|v| v.as_i64()).unwrap_or(0)));
    }
    let for_pubkey = match a[6].as_str() {
        Some(f) => Some(PubKey::parse(f)?.as_hex().to_string()),
        None => None,
    };
    let revoked_at = a[7].as_i64();
    let created_at = a.get(8).and_then(|v| v.as_i64()).unwrap_or(0);
    let label: String = a.get(9).and_then(|v| v.as_str()).unwrap_or_default().chars().take(MAX_INVITE_LABEL_CHARS).collect();
    Some((
        InviteRow {
            id: id.into(),
            secret: secret.into(),
            kind: kind.as_str().into(),
            label,
            created_at,
            expires_at,
            max_uses,
            for_pubkey,
            revoked_at,
        },
        uses,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_contacts::{ContactService, ProfileService};
    use messenger_core::{Clock, Timestamp};
    use messenger_store::Store;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0.load(Ordering::SeqCst))
        }
    }

    async fn device() -> (DmService, Arc<TestClock>) {
        let store = Store::open_in_memory().await.unwrap();
        let profiles = ProfileService::new(store.clone());
        let contacts = ContactService::new(store.clone(), profiles.clone());
        let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
        (DmService::new(store, contacts, profiles, clock.clone()), clock)
    }

    fn peer(n: u8) -> PubKey {
        PubKey::parse(&format!("{n:02x}").repeat(32)).unwrap()
    }

    fn new(secret: &str, max_uses: Option<u32>, ttl: Option<i64>, for_pubkey: Option<PubKey>) -> InviteNew {
        InviteNew {
            secret: secret.into(),
            kind: InviteKind::Code,
            label: " for Bob \u{7}".into(),
            expires_at: ttl.map(|t| 1_000_000 + t),
            max_uses,
            for_pubkey,
        }
    }

    /// The table of what admits: by the invitation's facts and the time.
    #[test]
    fn what_an_invitation_admits() {
        let now = 1_000_000;
        let base = Invite {
            row: InviteRow {
                id: invite_id("123456"),
                secret: "123456".into(),
                kind: "code".into(),
                label: String::new(),
                created_at: now - 10,
                expires_at: Some(now + 100),
                max_uses: Some(1),
                for_pubkey: None,
                revoked_at: None,
            },
            used_by: vec![],
        };
        let with = |f: &dyn Fn(&mut Invite)| {
            let mut i = base.clone();
            f(&mut i);
            i
        };
        assert!(base.admits(&peer(1), now) && base.active(now), "fresh");
        assert!(!with(&|i| i.row.expires_at = Some(now)).admits(&peer(1), now), "ran out this second");
        assert!(with(&|i| i.row.expires_at = None).admits(&peer(1), now + 1_000_000), "permanent");
        assert!(!with(&|i| i.row.revoked_at = Some(now - 1)).admits(&peer(1), now), "revoked");
        let used = with(&|i| i.used_by = vec![(peer(2).as_hex().into(), now - 5)]);
        assert!(!used.admits(&peer(1), now) && !used.active(now), "used up by somebody else");
        assert!(used.admits(&peer(2), now), "the one who used it comes in again (a replay is no guess)");
        // Their use is a fact: the accept of a key let in once (its copy from
        // another relay, the history my other device reads) is in past the
        // term and past a revocation; a key not in is not.
        let late = with(&|i| {
            i.row.expires_at = Some(now - 1);
            i.used_by = vec![(peer(2).as_hex().into(), now - 5)];
        });
        assert!(late.admits(&peer(2), now) && !late.admits(&peer(1), now) && !late.active(now), "ran out, used by them before");
        let pulled = with(&|i| {
            i.row.revoked_at = Some(now - 1);
            i.row.max_uses = None;
            i.used_by = vec![(peer(2).as_hex().into(), now - 5)];
        });
        assert!(pulled.admits(&peer(2), now) && !pulled.admits(&peer(1), now), "revoked, used by them before");
        assert!(with(&|i| {
            i.row.max_uses = None;
            i.used_by = vec![(peer(2).as_hex().into(), now - 5)];
        })
        .admits(&peer(1), now), "without a limit");
        let mine = with(&|i| i.row.for_pubkey = Some(peer(3).as_hex().into()));
        assert!(mine.admits(&peer(3), now) && !mine.admits(&peer(1), now), "for one person");
    }

    /// A peer's accept with a secret, each a wrap of its own.
    async fn try_in(dm: &DmService, who: u8, secret: &str) -> (Option<String>, Vec<Effect>) {
        static WRAP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = WRAP.fetch_add(1, Ordering::SeqCst);
        dm.redeem_invite(&peer(who), secret, &format!("wrap{n}")).await.unwrap()
    }

    async fn fails(dm: &DmService) -> Option<String> {
        settings::get(dm.store(), KEY_INVITE_FAILS).await.unwrap()
    }

    #[tokio::test]
    async fn a_right_secret_lets_in_once_per_key_and_a_wrong_one_costs_the_budget() {
        let (dm, clock) = device().await;
        let made = dm.invite_create(new("123456", Some(1), Some(3600), None)).await.unwrap();
        assert_eq!((made.row.label.as_str(), made.row.kind.as_str(), made.row.max_uses), ("for Bob", "code", Some(1)));
        assert!(dm.relations_owed().await.unwrap(), "the snapshot carries it");
        assert_eq!(dm.invite_create(new("123456", Some(5), None, None)).await.unwrap().row.max_uses, Some(1), "the same secret is the same invitation");

        let (id, fx) = try_in(&dm, 1, "123456").await;
        assert_eq!(id.as_deref(), Some(made.row.id.as_str()));
        assert!(fx.is_empty());
        assert_eq!(try_in(&dm, 1, "123456").await.0, id, "again, the same key: the same coming in");
        assert_eq!(dm.redeem_invite(&peer(2), "123456", "carol's wrap").await.unwrap().0, None, "used up");
        assert_eq!(dm.invite_get(&made.row.id).await.unwrap().unwrap().used_by.len(), 1);
        assert_eq!(fails(&dm).await.as_deref(), Some("1,1000000"), "the second key's try was a guess");
        // The same wrap again (another relay): the same try, not a second one.
        let copy = dm.redeem_invite(&peer(2), "123456", "carol's wrap").await.unwrap();
        assert!(copy.0.is_none() && copy.1.is_empty());
        assert_eq!(fails(&dm).await.as_deref(), Some("1,1000000"), "a copy of a refused wrap costs nothing");
        // Nine more wrong ones within the hour: the tenth pauses for an hour.
        for n in 2..INVITE_FAILS_PER_WINDOW {
            clock.0.fetch_add(60, Ordering::SeqCst);
            let (id, fx) = try_in(&dm, n as u8, "000000").await;
            assert!(id.is_none() && fx.is_empty(), "{n}");
        }
        let (_, fx) = try_in(&dm, 9, "000000").await;
        let now = clock.now().secs();
        assert_eq!(fx.len(), 1);
        match &fx[0] {
            Effect::Emit(ev) => assert_eq!((ev.name.as_str(), ev.payload["until"].as_i64()), (UI_EVENT_DM_INVITE_PAUSED, Some(now + INVITE_PAUSE_SECS))),
            other => panic!("{other:?}"),
        }
        assert_eq!(dm.invite_paused_until().await.unwrap(), Some(now + INVITE_PAUSE_SECS));
        assert_eq!(fails(&dm).await, None, "the pause starts the count over");
        // Paused: even the right secret is not looked at, and a wrong one
        // neither counts nor makes the pause longer.
        let fresh = dm.invite_create(new("654321", Some(1), None, None)).await.unwrap();
        assert_eq!(try_in(&dm, 4, "654321").await.0, None);
        assert!(dm.invite_get(&fresh.row.id).await.unwrap().unwrap().used_by.is_empty());
        clock.0.fetch_add(60, Ordering::SeqCst);
        let wrong = try_in(&dm, 5, "000000").await;
        assert!(wrong.0.is_none() && wrong.1.is_empty());
        assert_eq!(fails(&dm).await, None, "not counted while paused");
        assert_eq!(dm.invite_paused_until().await.unwrap(), Some(now + INVITE_PAUSE_SECS), "the pause stands as it was");
        // The pause is over: the budget starts afresh.
        clock.0.fetch_add(INVITE_PAUSE_SECS, Ordering::SeqCst);
        assert_eq!(dm.invite_paused_until().await.unwrap(), None);
        assert!(try_in(&dm, 4, "654321").await.0.is_some());
        assert!(try_in(&dm, 5, "999999").await.1.is_empty());
        let since = clock.now().secs();
        assert_eq!(fails(&dm).await, Some(format!("1,{since}")));

        // The window: nine wrong ones within the hour of the first, then an
        // hour passes — the next one starts a window of its own, no pause.
        for n in 2..INVITE_FAILS_PER_WINDOW {
            clock.0.fetch_add(1, Ordering::SeqCst);
            assert!(try_in(&dm, n as u8, "999999").await.1.is_empty(), "{n}");
        }
        assert_eq!(fails(&dm).await, Some(format!("{},{since}", INVITE_FAILS_PER_WINDOW - 1)));
        clock.0.fetch_add(INVITE_FAIL_WINDOW_SECS, Ordering::SeqCst);
        let tenth = try_in(&dm, 9, "999999").await;
        assert!(tenth.0.is_none() && tenth.1.is_empty(), "the tenth, an hour on: no pause");
        assert_eq!(dm.invite_paused_until().await.unwrap(), None);
        assert_eq!(fails(&dm).await, Some(format!("1,{}", clock.now().secs())), "a window of its own");
    }

    #[tokio::test]
    async fn the_list_puts_the_open_ones_first_and_a_revocation_is_owed_once() {
        let (dm, clock) = device().await;
        let old = dm.invite_create(new("111111", Some(1), Some(10), None)).await.unwrap();
        clock.0.fetch_add(5, Ordering::SeqCst);
        let spent = dm.invite_create(new("222222", Some(1), None, None)).await.unwrap();
        clock.0.fetch_add(5, Ordering::SeqCst);
        let open = dm.invite_create(new("333333", None, None, Some(peer(7)))).await.unwrap();
        try_in(&dm, 1, "222222").await;
        clock.0.fetch_add(5, Ordering::SeqCst);
        let list = dm.invite_list().await.unwrap();
        let got: Vec<&str> = list.iter().map(|i| i.row.id.as_str()).collect();
        assert_eq!(got, [open.row.id.as_str(), spent.row.id.as_str(), old.row.id.as_str()], "open first, then spent, then the one that ran out");
        assert!(list[0].active(clock.now().secs()) && !list[1].active(clock.now().secs()) && !list[2].active(clock.now().secs()));

        dm.relations_settled().await.unwrap();
        dm.invite_revoke(&open.row.id).await.unwrap();
        assert!(dm.relations_owed().await.unwrap());
        dm.relations_settled().await.unwrap();
        dm.invite_revoke(&open.row.id).await.unwrap();
        assert!(!dm.relations_owed().await.unwrap(), "revoked already: nothing new");
        assert!(dm.invite_revoke("nope").await.unwrap_err().to_string().contains("invite_unknown"));
        assert_eq!(try_in(&dm, 7, "333333").await.0, None, "revoked admits nobody new");

        // The snapshot: the ones that ended lately travel, newest first;
        // the one that ran out does not.
        let entries = dm.invites_for_snapshot(clock.now().secs()).await.unwrap();
        let named: Vec<&str> = entries.iter().map(|e| e[0].as_str().unwrap()).collect();
        assert_eq!(named, [open.row.id.as_str(), spent.row.id.as_str()]);
        assert_eq!(entries[1][5][0][0].as_str(), Some(peer(1).as_hex()), "the uses travel");
        assert_eq!(entries[0][7].as_i64(), Some(clock.now().secs()), "the revocation travels");
        assert_eq!(entries[0][6].as_str(), Some(peer(7).as_hex()));

        // Another device takes them: by union and the later revocation.
        let (other, _) = device().await;
        assert!(other.apply_invite_entries(&entries).await.unwrap());
        assert!(!other.apply_invite_entries(&entries).await.unwrap(), "the same again changes nothing");
        let there = other.invite_get(&spent.row.id).await.unwrap().unwrap();
        assert_eq!((there.used_by.len(), there.row.secret.as_str(), there.row.label.as_str()), (1, "222222", "for Bob"));
        assert!(other.invite_get(&open.row.id).await.unwrap().unwrap().revoked());
        // Junk among the entries is skipped: a wrong id, an unknown kind, a use that is no key.
        let mut junk = entries.clone();
        junk[1][0] = serde_json::json!("ab".repeat(32));
        junk.push(serde_json::json!([invite_id("x"), "x", "ticket", null, null, [], null, null]));
        junk.push(serde_json::json!([invite_id("y"), "y", "link", null, null, [["nobody", 1]], null, null]));
        junk.push(serde_json::json!("not an entry"));
        assert!(!other.apply_invite_entries(&junk).await.unwrap());
        assert_eq!(other.invite_list().await.unwrap().len(), 2);
    }

    /// A permanent QR on a sign gathers hundreds of uses: the entry carries
    /// the newest `SNAPSHOT_USES_PER_INVITE`, and a device that takes it
    /// knows those; the other device's own uses are kept beside them.
    #[tokio::test]
    async fn a_well_used_invitation_travels_with_its_newest_uses() {
        let (dm, clock) = device().await;
        let sign = dm.invite_create(InviteNew { kind: InviteKind::Qr, ..new("AAECAwQFBgcICQoLDA0O_w", None, None, None) }).await.unwrap();
        for n in 1..=(SNAPSHOT_USES_PER_INVITE as i64 * 8) {
            let who = PubKey::parse(&format!("{n:064x}")).unwrap();
            dm_invites::add_use(dm.store(), &sign.row.id, who.as_hex(), 1_000_000 + n).await.unwrap();
        }
        clock.0.fetch_add(10_000, Ordering::SeqCst);
        let entries = dm.invites_for_snapshot(clock.now().secs()).await.unwrap();
        let uses = entries[0][5].as_array().unwrap();
        assert_eq!(uses.len(), SNAPSHOT_USES_PER_INVITE);
        assert_eq!(uses[0][1].as_i64(), Some(1_000_000 + SNAPSHOT_USES_PER_INVITE as i64 * 7 + 1), "the oldest carried is the first of the newest hundred");
        assert_eq!(uses.last().unwrap()[0].as_str(), Some(format!("{:064x}", SNAPSHOT_USES_PER_INVITE * 8).as_str()), "the newest is carried");

        let (other, _) = device().await;
        dm_invites::create(other.store(), &sign.row).await.unwrap();
        dm_invites::add_use(other.store(), &sign.row.id, peer(1).as_hex(), 1_000_000).await.unwrap();
        assert!(other.apply_invite_entries(&entries).await.unwrap());
        assert_eq!(other.invite_get(&sign.row.id).await.unwrap().unwrap().used_by.len(), SNAPSHOT_USES_PER_INVITE + 1, "the union with its own");
    }
}

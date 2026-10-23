// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Epochs on the member's side (internal/communities-wire.md §5, §6).
//!
//! An epoch event (kind 1392) is keydist's `EpochEvent` with the hosts of
//! its blobs, signed by the key of a keeper series. A member takes it only
//! if the journal gives that key the range the epoch falls into, the
//! numbers follow (`prev_epoch < epoch ≤ prev_epoch + 16` everywhere: a
//! superseded epoch burns its number, so a gap is normal inside a series
//! too), it is not dated ahead, and it does not contradict an epoch
//! already taken. An emergency generation is told by `trees[]`: a newer
//! generation is there and the one before it is gone. Only then is it keydist's
//! `Verified<EpochEvent>`, which the keys of the member (`MemberKeys`,
//! keydist's `MemberState` per scope) act on.
//!
//! What a member keeps (`cm_epochs`, `cm_bundles`) is the database's;
//! here is the state and the rules.

use crate::state::ChannelState;
use crate::wire::{self, KIND_EPOCH, SKEW_SECS};
use keydist::{ChannelKey, EpochEvent, EpochKey, Issued, MemberState, ScopeRef, Status, Verified};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The widest gap between `prev_epoch` and `epoch` (wire §5): wide enough
/// for the numbers `supersede_pending` burns, narrow enough that a stolen
/// keeper key cannot run the numbers out in one event.
pub const MAX_GAP: u32 = 16;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel epoch: {what}"))
}

/// The content of an epoch event: keydist's event and the hosts that hold
/// its blobs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochContent {
    #[serde(flatten)]
    pub event: EpochEvent,
    pub blobs: Vec<String>,
}

/// The event of an epoch, signed by the keeper's key, tagged with the
/// channel, the scope and the epoch.
pub fn sign_epoch(keeper: &Keys, community_id: &str, scope_id: &str, content: &EpochContent) -> Result<Event> {
    let tag = |t: [&str; 2]| Tag::parse(t).map_err(|e| invalid(&e.to_string()));
    EventBuilder::new(Kind::from(KIND_EPOCH), serde_json::to_string(content)?)
        .tag(tag(["h", community_id])?)
        .tag(tag(["s", scope_id])?)
        .tag(tag(["n", content.event.epoch.to_string().as_str()])?)
        .custom_created_at(Timestamp::from_secs(content.event.at))
        .finalize(keeper)
        .map_err(|e| MessengerError::Crypto(e.to_string()))
}

/// An epoch event whose signature is checked, before the rules: who
/// signed it, what it says, and the digest that tells two versions of
/// one epoch apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    pub signer: PubKey,
    pub content: EpochContent,
    pub digest: [u8; 32],
}

/// Read an epoch event of a scope of a channel: the signature, the kind,
/// `h`, `s`, `n` agreeing with the content and its scope bytes (§1).
pub fn read_epoch(signed: &serde_json::Value, community_id: &str, scope_id: &str) -> Result<Signed> {
    let event: Event = serde_json::from_value(signed.clone()).map_err(|_| invalid("event"))?;
    event.verify().map_err(|_| invalid("signature"))?;
    if event.kind.as_u16() != KIND_EPOCH {
        return Err(invalid("kind"));
    }
    let tag = |name: &str| event.tags.iter().find(|t| t.kind() == name).and_then(|t| t.as_slice().get(1).cloned());
    let content: EpochContent = serde_json::from_str(&event.content).map_err(|_| invalid("content"))?;
    let scope = wire::scope_ref(community_id, scope_id)?;
    if tag("h").as_deref() != Some(community_id) || tag("s").as_deref() != Some(scope_id) || content.event.scope() != scope {
        return Err(invalid("scope"));
    }
    if tag("n") != Some(content.event.epoch.to_string()) {
        return Err(invalid("n"));
    }
    Ok(Signed {
        signer: PubKey::parse(&event.pubkey.to_hex()).ok_or_else(|| invalid("signer"))?,
        digest: Sha256::digest(event.content.as_bytes()).into(),
        content,
    })
}

/// What became of an epoch event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Taken; `current` says whether it is the newest now, `emergency`
    /// whether it starts an emergency generation (by `trees[]`; the old
    /// bundles and keys go, wire §6.5).
    Accepted { event: Verified<EpochEvent>, current: bool, emergency: bool },
    /// The same event again.
    Duplicate,
    /// Another event for an epoch already taken: ignored, logged, and the
    /// member asks for its bundle again (`channel.rebundle`).
    Conflict,
    /// Ahead of the epochs taken (past `current + MAX_GAP`, or its
    /// `prev_epoch` not taken): kept aside and tried again when the epochs
    /// between come.
    Ahead,
    /// Refused, and why.
    Refused(&'static str),
}

/// What a member has taken of the epochs of one scope: for each epoch the
/// digest of its event, the current epoch, and the newest generation the
/// events carried.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    pub seen: BTreeMap<u32, String>,
    pub current: Option<u32>,
    pub newest_gen: Option<u32>,
}

impl Acceptance {
    /// The rules of §5, in order. `now`: my clock, seconds.
    pub fn accept(&mut self, state: &ChannelState, ev: Signed, now: i64) -> Verdict {
        let e = &ev.content.event;
        // The series whose key signed it, and whose range holds the epoch:
        // former keepers are read for the history, and none of them
        // publishes in a range not its own. Before anything else, so that
        // nobody but a keeper makes a member doubt an epoch it holds.
        let Some(series) = state.keepers.iter().filter(|k| k.pubkey == ev.signer).find(|k| {
            let end = state.range_end(k.serial);
            k.from_epoch <= e.epoch && end.is_none_or(|end| e.epoch < end)
        }) else {
            return Verdict::Refused("not the keeper of this range");
        };
        let digest = hex::encode(ev.digest);
        match self.seen.get(&e.epoch) {
            Some(d) if *d == digest => return Verdict::Duplicate,
            Some(_) => return Verdict::Conflict,
            None => {}
        }
        let at = i64::try_from(e.at).unwrap_or(i64::MAX);
        if at > now + SKEW_SECS {
            return Verdict::Refused("dated ahead");
        }
        let starts_series = e.epoch == series.from_epoch;
        // The first epochs of a later series in a scope (wire §15,
        // 2026-10-23): their predecessor is the former keeper's, and the
        // series begins at one number for every scope, so a section whose
        // numbers lagged jumps to it. The journal placed `from_epoch`; past
        // it the gap is bounded as anywhere (a number the new keeper burnt).
        let opens_series = series.serial > 0 && e.prev_epoch.is_some_and(|p| p < series.from_epoch) && e.epoch - series.from_epoch <= MAX_GAP;
        let emergency = self.emergency(e);
        match e.prev_epoch {
            // The first epoch of a series may know no predecessor (a new
            // keeper); any other names one.
            None if starts_series => {}
            // So may the first epoch of an emergency generation (keydist
            // `recover` chains nothing of the lost state), but no further
            // past the current epoch than any other gap.
            None if emergency => {
                if self.current.is_some_and(|c| e.epoch <= c || e.epoch - c > MAX_GAP) {
                    return Verdict::Refused("a gap in the epochs");
                }
            }
            None => return Verdict::Refused("no prev_epoch"),
            Some(prev) => {
                if e.epoch <= prev {
                    return Verdict::Refused("prev_epoch not before epoch");
                }
                if e.epoch - prev > MAX_GAP && !opens_series {
                    return Verdict::Refused("a gap in the epochs");
                }
            }
        }
        // Forward, the gap is bounded by the current epoch too, and the
        // event follows one taken here: checked against its own
        // `prev_epoch` only, one event of a stolen or broken keeper key
        // (epoch 4e9, prev 4e9 - 1) would move `current` for good and
        // refuse all live content after it. What runs ahead waits for the
        // epochs between (`Ahead`). The first epoch of a series is placed
        // by the journal.
        if let (Some(c), false) = (self.current, starts_series || opens_series) {
            let follows = e.prev_epoch.is_none_or(|p| p == c || self.seen.contains_key(&p));
            if e.epoch > c && (e.epoch - c > MAX_GAP || !follows) {
                return Verdict::Ahead;
            }
        }
        // Only the current epoch moves forward; older ones are taken in
        // the catch-up.
        let current = self.current.is_none_or(|c| e.epoch > c);
        if current {
            self.current = Some(e.epoch);
        }
        if let Some(g) = e.newest_gen() {
            self.newest_gen = Some(self.newest_gen.map_or(g, |n| n.max(g)));
        }
        self.seen.insert(e.epoch, digest);
        Verdict::Accepted { event: Verified::after_signature_check(ev.content.event), current, emergency }
    }

    /// An event that starts an emergency generation: its newest generation
    /// is newer than any taken before, and the one before is no longer in
    /// it (a planned migration carries both through its grace).
    pub fn emergency(&self, e: &EpochEvent) -> bool {
        match (self.newest_gen, e.newest_gen()) {
            (Some(before), Some(now)) => now > before && e.tree(before).is_none(),
            _ => false,
        }
    }

    /// Whether content sealed under `epoch` is taken live (§4): no older
    /// than the epoch before the current one, so that a banned member with
    /// an old key writes nothing back in time. Content fetched in the
    /// catch-up of history is not live.
    pub fn live_content_ok(&self, epoch: u32) -> bool {
        self.current.is_none_or(|c| epoch >= c.saturating_sub(1))
    }
}

/// The keys of one member, per scope: keydist's `MemberState` and what
/// was accepted of the epochs.
#[derive(Clone, Debug, Default)]
pub struct MemberKeys {
    scopes: BTreeMap<String, (MemberState, Acceptance)>,
}

impl MemberKeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// The keys of a scope as they were kept.
    pub fn restore(&mut self, scope_id: &str, state: MemberState, acceptance: Acceptance) {
        self.scopes.insert(scope_id.to_string(), (state, acceptance));
    }

    pub fn state(&self, scope_id: &str) -> Option<&MemberState> {
        self.scopes.get(scope_id).map(|(s, _)| s)
    }

    pub fn acceptance(&self, scope_id: &str) -> Option<&Acceptance> {
        self.scopes.get(scope_id).map(|(_, a)| a)
    }

    fn entry(&mut self, scope: ScopeRef, scope_id: &str) -> &mut (MemberState, Acceptance) {
        self.scopes.entry(scope_id.to_string()).or_insert_with(|| (MemberState::new(scope), Acceptance::default()))
    }

    /// What `channel.issue` brought (§6.1).
    pub fn take_issued(&mut self, community_id: &str, scope_id: &str, issued: &Issued) -> Result<()> {
        let scope = wire::scope_ref(community_id, scope_id)?;
        let (state, _) = self.entry(scope, scope_id);
        state.accept(issued).map_err(|e| invalid(&e.to_string()))
    }

    /// An epoch event, by the rules of §5, and what to do with it next
    /// (§6.2): fetch and open a shard, ask for a bundle, or nothing.
    pub fn on_epoch(&mut self, channel: &ChannelState, community_id: &str, scope_id: &str, ev: Signed, now: i64) -> Result<(Verdict, Option<Status>)> {
        let scope = wire::scope_ref(community_id, scope_id)?;
        let (state, acceptance) = self.entry(scope, scope_id);
        let verdict = acceptance.accept(channel, ev, now);
        let status = match &verdict {
            Verdict::Accepted { event, .. } => Some(state.status(event)),
            _ => None,
        };
        Ok((verdict, status))
    }

    /// Opens an epoch with a shard (`Status::Read`) and keeps its key.
    pub fn open(&mut self, scope_id: &str, ev: &Verified<EpochEvent>, shard: &[u8], witness: &[u8]) -> Result<EpochKey> {
        let (state, _) = self.scopes.get_mut(scope_id).ok_or_else(|| invalid("unknown scope"))?;
        state.open(ev, shard, witness).map_err(|e| invalid(&e.to_string()))
    }

    /// Under an open history: the key before, from an event whose key is held.
    pub fn follow_chain(&mut self, scope_id: &str, ev: &Verified<EpochEvent>) -> Result<EpochKey> {
        let (state, _) = self.scopes.get_mut(scope_id).ok_or_else(|| invalid("unknown scope"))?;
        state.follow_chain(ev).map_err(|e| invalid(&e.to_string()))
    }

    /// The key that seals `channel_id` in `epoch`, if held.
    pub fn stream_key(&self, scope_id: &str, epoch: u32, channel_id: &str) -> Option<ChannelKey> {
        self.state(scope_id)?.channel_key(epoch, channel_id.as_bytes())
    }

    /// The epoch a `k` names, among the keys held: what opens an event.
    pub fn epoch_of_key_id(&self, scope_id: &str, key_id: &crate::op::KeyId) -> Option<&EpochKey> {
        self.state(scope_id)?.keys.values().find(|k| &wire::key_id(k) == key_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::OpBody;
    use crate::state::tests::{create, op, pk};
    use keydist::{History, SchemeId, TreeIndex};

    const CID: &str = crate::state::tests::CHANNEL;

    fn as_pk(k: &Keys) -> PubKey {
        PubKey::parse(&k.public_key().to_hex()).unwrap()
    }

    fn tree(gen: u32) -> TreeIndex {
        TreeIndex { gen, depth: 24, leaves: 1024, count: 1, index: None, root: None }
    }

    fn event(epoch: u32, prev: Option<u32>, at: u64, gens: &[u32]) -> EpochEvent {
        let scope = wire::scope_ref(CID, "everyone").unwrap();
        EpochEvent {
            v: 1,
            scheme: SchemeId::SdV1,
            community_id: scope.community,
            scope_id: scope.scope,
            epoch,
            prev_epoch: prev,
            at,
            history: History::Open,
            trees: gens.iter().map(|g| tree(*g)).collect(),
            chain: None,
        }
    }

    fn signed(keeper: &Keys, e: EpochEvent) -> Signed {
        let content = EpochContent { event: e, blobs: vec!["https://s1.example".into()] };
        let ev = sign_epoch(keeper, CID, "everyone", &content).unwrap();
        read_epoch(&serde_json::to_value(&ev).unwrap(), CID, "everyone").unwrap()
    }

    /// A channel whose first keeper is `k0` (epochs 1…19) and second `k1`
    /// (from 20).
    fn channel(k0: &Keys, k1: &Keys) -> ChannelState {
        let o = pk("0");
        let mut c = create(&o);
        if let OpBody::Create { keeper, .. } = &mut c.body {
            keeper.pubkey = as_pk(k0);
        }
        let mut s = ChannelState::genesis(&c).unwrap();
        s.apply(&op(&o, OpBody::Keeper { pubkey: as_pk(k1), serial: 1, from_epoch: 20, inbox: None })).unwrap();
        s
    }

    fn accepted(v: &Verdict) -> bool {
        matches!(v, Verdict::Accepted { .. })
    }

    /// Every rule of wire §5, one row each.
    #[test]
    fn the_rules_of_acceptance() {
        let (k0, k1, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let c = channel(&k0, &k1);
        let now = 10_000;
        let mut a = Acceptance::default();
        // The first epoch of the first series knows no predecessor.
        assert!(accepted(&a.accept(&c, signed(&k0, event(1, None, 100, &[1])), now)));
        assert_eq!(a.current, Some(1));
        // The next one names it.
        assert!(accepted(&a.accept(&c, signed(&k0, event(2, Some(1), 200, &[1])), now)));
        // The same again: a duplicate; another event for it: a conflict.
        assert_eq!(a.accept(&c, signed(&k0, event(2, Some(1), 200, &[1])), now), Verdict::Duplicate);
        assert_eq!(a.accept(&c, signed(&k0, event(2, Some(1), 201, &[1])), now), Verdict::Conflict);
        // Not the keeper of this range: a stranger, or a keeper outside its own.
        assert_eq!(a.accept(&c, signed(&stranger, event(3, Some(2), 300, &[1])), now), Verdict::Refused("not the keeper of this range"));
        assert_eq!(a.accept(&c, signed(&k1, event(3, Some(2), 300, &[1])), now), Verdict::Refused("not the keeper of this range"), "the new keeper before its range");
        assert_eq!(a.accept(&c, signed(&k0, event(20, Some(19), 300, &[1])), now), Verdict::Refused("not the keeper of this range"), "the old keeper past its range");
        // Dated ahead of my clock past the skew.
        assert_eq!(a.accept(&c, signed(&k0, event(3, Some(2), (now + SKEW_SECS + 1) as u64, &[1])), now), Verdict::Refused("dated ahead"));
        assert!(accepted(&a.accept(&c, signed(&k0, event(3, Some(2), (now + SKEW_SECS) as u64, &[1])), now)), "within the skew");
        // Inside a series a gap of up to 16 (a superseded epoch burns its
        // number), none wider; `prev_epoch` not before: refused.
        assert!(accepted(&a.clone().accept(&c, signed(&k0, event(5, Some(3), 400, &[1])), now)), "one number burnt");
        assert!(accepted(&a.clone().accept(&c, signed(&k0, event(19, Some(3), 400, &[1])), now)), "16 inside a series");
        assert_eq!(a.clone().accept(&c, signed(&k0, event(19, Some(2), 400, &[1])), now), Verdict::Refused("a gap in the epochs"), "17 inside a series");
        assert_eq!(a.accept(&c, signed(&k0, event(5, Some(5), 400, &[1])), now), Verdict::Refused("prev_epoch not before epoch"));
        assert_eq!(a.accept(&c, signed(&k0, event(4, Some(5), 400, &[1])), now), Verdict::Refused("prev_epoch not before epoch"));
        assert_eq!(a.accept(&c, signed(&k0, event(5, None, 400, &[1])), now), Verdict::Refused("no prev_epoch"));
        // At the boundary of a series (wire §15, 2026-10-23): the journal
        // places its first number, so the gap from the series before is not
        // bounded (a section whose numbers lagged jumps to it); past that
        // number at most 16, as anywhere (the new keeper burnt some).
        let mut b = a.clone();
        assert!(accepted(&b.clone().accept(&c, signed(&k1, event(20, Some(3), 500, &[1])), now)), "a lagging scope jumps to the series");
        assert!(accepted(&b.clone().accept(&c, signed(&k1, event(36, Some(3), 500, &[1])), now)), "16 burnt past its first number");
        assert_eq!(b.clone().accept(&c, signed(&k1, event(37, Some(3), 500, &[1])), now), Verdict::Refused("a gap in the epochs"), "17 past it");
        assert_eq!(b.clone().accept(&c, signed(&k0, event(21, Some(3), 500, &[1])), now), Verdict::Refused("not the keeper of this range"), "the former keeper publishes no more");
        assert!(accepted(&a.accept(&c, signed(&k1, event(20, Some(4), 500, &[1])), now)), "16");
        assert!(accepted(&b.accept(&c, signed(&k1, event(20, None, 500, &[1])), now)), "a new keeper may start afresh");
        // An emergency generation is told by `trees[]`: a newer generation,
        // the one before gone; a planned migration keeps both.
        let mut e = Acceptance::default();
        assert!(matches!(e.accept(&c, signed(&k0, event(1, None, 100, &[1])), now), Verdict::Accepted { emergency: false, .. }));
        assert!(matches!(e.clone().accept(&c, signed(&k0, event(3, Some(1), 200, &[1, 2])), now), Verdict::Accepted { emergency: false, .. }), "planned");
        // A recovered keeper chains nothing: its first epoch names no
        // predecessor, within the gap of the current one.
        assert_eq!(e.clone().accept(&c, signed(&k0, event(3, None, 200, &[1])), now), Verdict::Refused("no prev_epoch"), "not an emergency");
        assert_eq!(e.clone().accept(&c, signed(&k0, event(18, None, 200, &[2])), now), Verdict::Refused("a gap in the epochs"));
        assert!(matches!(e.clone().accept(&c, signed(&k0, event(3, None, 200, &[2])), now), Verdict::Accepted { emergency: true, .. }));
        assert!(matches!(e.accept(&c, signed(&k0, event(3, Some(1), 200, &[2])), now), Verdict::Accepted { emergency: true, .. }), "emergency");
        assert_eq!(e.newest_gen, Some(2));
    }

    /// After `supersede_pending` the keeper publishes `n + 2` over `n`:
    /// taken, and the burnt number is never asked for.
    #[test]
    fn the_gap_rule_after_a_supersede() {
        let (k0, k1) = (Keys::generate(), Keys::generate());
        let c = channel(&k0, &k1);
        let mut a = Acceptance::default();
        assert!(accepted(&a.accept(&c, signed(&k0, event(1, None, 100, &[1])), 1_000)));
        assert!(matches!(a.accept(&c, signed(&k0, event(3, Some(1), 200, &[1])), 1_000), Verdict::Accepted { current: true, .. }));
        assert_eq!(a.current, Some(3));
        assert!(!a.seen.contains_key(&2));
        // Sixteen burnt numbers in a row are still one event.
        assert!(accepted(&a.accept(&c, signed(&k0, event(19, Some(3), 300, &[1])), 1_000)));
    }


    /// Forward, an epoch is bounded by the current one as well as by its
    /// own `prev_epoch`, and follows one taken here; what runs ahead
    /// waits (`Ahead`) and comes in once the epochs between are taken.
    #[test]
    fn an_epoch_far_ahead_of_the_current_one_waits() {
        let (k0, k1) = (Keys::generate(), Keys::generate());
        let c = channel(&k0, &k1);
        let mut a = Acceptance::default();
        assert!(accepted(&a.accept(&c, signed(&k0, event(1, None, 100, &[1])), 1_000)));
        assert_eq!(a.accept(&c, signed(&k0, event(19, Some(18), 200, &[1])), 1_000), Verdict::Ahead, "18 past the current one");
        assert_eq!(a.accept(&c, signed(&k0, event(3, Some(2), 200, &[1])), 1_000), Verdict::Ahead, "2 not taken");
        assert_eq!(a.current, Some(1));
        assert!(accepted(&a.accept(&c, signed(&k0, event(2, Some(1), 200, &[1])), 1_000)));
        assert!(accepted(&a.accept(&c, signed(&k0, event(3, Some(2), 200, &[1])), 1_000)), "once 2 is here");
        // A keeper key of the current series, stolen: one event far ahead
        // moves nothing.
        let mut b = Acceptance::default();
        assert!(accepted(&b.accept(&c, signed(&k1, event(20, None, 100, &[1])), 1_000)));
        assert_eq!(b.accept(&c, signed(&k1, event(4_000_000_000, Some(3_999_999_999), 200, &[1])), 1_000), Verdict::Ahead);
        assert_eq!(b.current, Some(20));
        assert!(b.live_content_ok(20));
    }
    #[test]
    fn only_the_current_epoch_moves_forward() {
        let (k0, k1) = (Keys::generate(), Keys::generate());
        let c = channel(&k0, &k1);
        let mut a = Acceptance::default();
        assert!(matches!(a.accept(&c, signed(&k0, event(5, Some(4), 100, &[1])), 1_000), Verdict::Accepted { current: true, .. }));
        assert!(matches!(a.accept(&c, signed(&k0, event(3, Some(2), 90, &[1])), 1_000), Verdict::Accepted { current: false, .. }), "taken in the catch-up");
        assert_eq!(a.current, Some(5));
        assert!(matches!(a.accept(&c, signed(&k0, event(6, Some(5), 110, &[1])), 1_000), Verdict::Accepted { current: true, .. }));
        // Live content: the current epoch and the one before only.
        assert!(a.live_content_ok(6) && a.live_content_ok(5) && !a.live_content_ok(4));
        assert!(Acceptance::default().live_content_ok(1), "nothing taken yet: nothing to compare");
    }

    #[test]
    fn an_epoch_event_names_its_scope_and_number_in_the_tags() {
        let k0 = Keys::generate();
        let content = EpochContent { event: event(7, Some(6), 1, &[1]), blobs: vec!["https://s1.example".into()] };
        let ev = serde_json::to_value(sign_epoch(&k0, CID, "everyone", &content).unwrap()).unwrap();
        let read = read_epoch(&ev, CID, "everyone").unwrap();
        assert_eq!(read.content, content);
        assert_eq!(read.signer, as_pk(&k0));
        assert!(read_epoch(&ev, &"bb".repeat(32), "everyone").is_err(), "another channel");
        assert!(read_epoch(&ev, CID, "0123456789abcdef0123456789abcdef").is_err(), "another scope");
        // `n` that lies about the number.
        let lying = EventBuilder::new(Kind::from(KIND_EPOCH), serde_json::to_string(&content).unwrap())
            .tag(Tag::parse(["h", CID]).unwrap())
            .tag(Tag::parse(["s", "everyone"]).unwrap())
            .tag(Tag::parse(["n", "8"]).unwrap())
            .finalize(&k0)
            .unwrap();
        assert!(read_epoch(&serde_json::to_value(&lying).unwrap(), CID, "everyone").is_err());
        // The content keeps the fields of keydist and adds `blobs`.
        let json: serde_json::Value = serde_json::from_str(&serde_json::to_string(&content).unwrap()).unwrap();
        assert_eq!(json["scheme"], "sd-v1");
        assert_eq!(json["blobs"][0], "https://s1.example");
        assert_eq!(json["community_id"], CID);
    }

    #[test]
    fn member_keys_take_what_the_keeper_issued() {
        use keydist::{GenerationKind, Key32, SecretBytes};
        let (k0, k1) = (Keys::generate(), Keys::generate());
        let c = channel(&k0, &k1);
        let mut keys = MemberKeys::new();
        let scope = wire::scope_ref(CID, "everyone").unwrap();
        let current = EpochKey { epoch: 4, key: Key32([3u8; 32]) };
        let issued = Issued {
            scheme: SchemeId::SdV1,
            scope,
            gen: 1,
            leaf: 0,
            kind: GenerationKind::Initial,
            bundle: SecretBytes::new(vec![]),
            current: Some(current.clone()),
            missed: vec![],
            missed_more: false,
        };
        keys.take_issued(CID, "everyone", &issued).unwrap();
        assert_eq!(keys.state("everyone").unwrap().key(4), Some(&current));
        assert!(keys.stream_key("everyone", 4, "feed").is_some());
        assert!(keys.stream_key("everyone", 5, "feed").is_none());
        assert_eq!(keys.epoch_of_key_id("everyone", &wire::key_id(&current)), Some(&current));
        // A conflicting key for the same epoch is refused.
        let conflicting = Issued { current: Some(EpochKey { epoch: 4, key: Key32([9u8; 32]) }), ..issued };
        assert!(keys.take_issued(CID, "everyone", &conflicting).is_err());
        // An event of an epoch: taken by the rules; no bundle held, so a
        // bundle is asked for.
        let (verdict, status) = keys.on_epoch(&c, CID, "everyone", signed(&k0, event(5, Some(4), 10, &[1])), 1_000).unwrap();
        assert!(accepted(&verdict));
        assert_eq!(status, Some(Status::NeedBundle { gen: 1 }));
        let (verdict, status) = keys.on_epoch(&c, CID, "everyone", signed(&k1, event(6, Some(5), 10, &[1])), 1_000).unwrap();
        assert_eq!((verdict, status), (Verdict::Refused("not the keeper of this range"), None));
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The member's side of the keys (internal/communities-wire.md §6,
//! internal/communities-spec.md §9): what `channel.issue` brought, the
//! epoch events by the rules of §5, the shards that open them, the chain
//! of an open history, the sealed events that wait for their key, and the
//! note to my other devices that carries all of it.
//!
//! The state is the database's (`cm_bundles`, `cm_epochs`, `cm_pending`):
//! `epoch::MemberKeys` is put together from it for every call and every
//! key that opens is written back at once.

use crate::db::{self, channels::PendingRow, keys::EpochRow};
use crate::epoch::{read_epoch, MemberKeys, Verdict};
use crate::keeper::Env;
use crate::log::OpLog;
use crate::op::{History, OpBody};
use crate::secrets;
use crate::state::{ChannelState, SCOPE_EVERYONE};
use crate::wire::{self, Issue, LinkPayload, Missed, Rebundle};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use keydist::{Bundle, EpochEvent, EpochKey, GenerationKind, Issued, Key32, SecretBytes, Status, Verified};
use messenger_core::{MessengerError, PubKey, Result};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};

pub mod fetch;
pub mod readers;

use fetch::Want;

/// The note to my own devices (rumor 30078, wire §6.4).
pub const T_OWN_KEYS: &str = "own.channel_keys";
/// The keys of epochs one such note carries at most: under an open
/// history the chain gives the rest, under `from_join` the keeper's
/// missed pages do.
pub const OWN_KEYS_MAX: usize = 64;

/// The mark an epoch event that ran ahead waits under in `cm_pending`.
fn ahead_marker(scope_id: &str) -> String {
    format!("epoch-ahead:{scope_id}")
}

/// Where I stand in a channel the epochs put me out of.
pub const MEMBERSHIP_OUT: &str = "left";

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel member: {what}"))
}

fn kd(e: keydist::Error) -> MessengerError {
    MessengerError::Invalid(format!("keydist: {e}"))
}

/// One key in `own.channel_keys`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnEpoch {
    pub epoch: u32,
    /// Hex.
    pub key: String,
}

/// `own.channel_keys` (wire §6.4): my bundle, my keys and the link, for
/// my other devices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnKeys {
    pub community_id: String,
    pub scope_id: String,
    pub gen: u32,
    /// `Issued::to_bytes` of the newest bundle, base64, without keys.
    pub issued: String,
    pub epochs: Vec<OwnEpoch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkPayload>,
    /// The other scopes I belong to (wire §14.5): the same for each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<OwnScopeKeys>,
    /// The owner's root secret, for the owner's other devices (wire §15,
    /// 2026-10-23): a device of the owner takes the keeper over (or
    /// recovers it) only with the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<OwnRoot>,
}

/// The root secret in `own.channel_keys`, with the serial of the keeper
/// series it was made for: of two, the newer series' stays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnRoot {
    pub serial: u32,
    /// Base64 of 32 bytes.
    pub secret: String,
}

/// The keys of one more scope in `own.channel_keys`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnScopeKeys {
    pub scope_id: String,
    pub gen: u32,
    pub issued: String,
    pub epochs: Vec<OwnEpoch>,
}

/// What a call of the member's side did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// Keys that opened (from a shard, the chain, a note).
    pub keys: Vec<EpochKey>,
    /// Sealed events that waited for one of those keys, taken out of
    /// `cm_pending` to be read again.
    pub pending: Vec<PendingRow>,
    /// The epochs put me out of the channel.
    pub out: bool,
    /// Sections whose newest epoch leaves me out while I stay in the
    /// channel: «раздел закрыт» (wire §14.5).
    pub closed: Vec<String>,
    /// What was asked of the keeper (`channel.rebundle`, `channel.missed`).
    pub asked: Vec<&'static str>,
    /// What became of an epoch event.
    pub verdict: Option<Verdict>,
    /// The link of the channel came.
    pub link: Option<LinkPayload>,
}

/// The member's side of one person (all channels).
#[derive(Clone)]
pub struct Member {
    env: Env,
    keys: Keys,
}

fn key_of(epoch: u32, hex_key: &str) -> Result<EpochKey> {
    let raw: [u8; 32] = hex::decode(hex_key).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("epoch key"))?;
    Ok(EpochKey { epoch, key: Key32(raw) })
}

impl Member {
    pub fn new(env: Env, keys: Keys) -> Self {
        Self { env, keys }
    }

    fn me(&self) -> Result<PubKey> {
        PubKey::parse(&self.keys.public_key().to_hex()).ok_or_else(|| invalid("my key"))
    }

    async fn load(&self, community_id: &str, scope_id: &str) -> Result<MemberKeys> {
        let scope = wire::scope_ref(community_id, scope_id)?;
        let (state, acceptance) = db::keys::load_member(&self.env.db, community_id, scope_id, scope).await?;
        let mut keys = MemberKeys::new();
        keys.restore(scope_id, state, acceptance);
        Ok(keys)
    }

    /// Writes a key that opened and takes out what waited for it.
    async fn keep(&self, community_id: &str, scope_id: &str, k: &EpochKey, out: &mut Outcome) -> Result<()> {
        // Whatever brought it, a fetch of it is nothing to do any more.
        self.env.fetches.forget(community_id, &Want::Epoch { scope_id: scope_id.to_string(), epoch: k.epoch });
        // A key held already (a note heard again by the next round of
        // listening) opens nothing new and is not counted as one.
        if !db::keys::set_key(&self.env.db, community_id, scope_id, k).await? {
            return Ok(());
        }
        let id = wire::key_id(k);
        out.pending.extend(db::channels::take_pending(&self.env.db, community_id, &id.0).await?);
        // A key of `everyone` may be the `epoch_ref` of the snapshot held
        // here: a shard that did not open may now, and the comments that
        // waited for it are read again rather than at the next snapshot
        // (those it still cannot place wait again, under `MAX_PENDING`).
        if scope_id == SCOPE_EVERYONE {
            out.pending.extend(db::channels::take_pending(&self.env.db, community_id, readers::SNAPSHOT_MARKER).await?);
        }
        out.keys.push(k.clone());
        Ok(())
    }

    /// A note to the keeper of the channel (the current series): a row of
    /// the outbox, sent by the worker off this call (`crate::outbox`).
    async fn to_keeper<T: Serialize>(&self, state: &ChannelState, t: &str, body: &T, now: i64) -> Result<()> {
        let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?.inbox().clone();
        crate::outbox::queue_note(&self.env, &state.community_id, Some(&keeper), &state.relays, t, body, 0, None, now).await?;
        Ok(())
    }

    /// `channel.rebundle` for the bundle of `gen` (wire §7).
    pub async fn ask_rebundle(&self, state: &ChannelState, scope_id: &str, gen: u32, now: i64) -> Result<()> {
        let body = Rebundle { community_id: state.community_id.clone(), scope_id: scope_id.to_string(), gen };
        self.to_keeper(state, wire::T_REBUNDLE, &body, now).await
    }

    // ─── channel.issue ──────────────────────────────────────────────────────

    /// A `channel.issue`, taken only from the keeper of the current series
    /// by the journal (wire §11.6): the bundle, the keys, the link; then
    /// the epochs that waited for a bundle are opened, the next page of
    /// missed keys asked for, and my other devices told.
    pub async fn on_issue(&self, log: &OpLog, from: &PubKey, issue: &Issue, now: i64) -> Result<Outcome> {
        let state = log.state();
        if issue.community_id != state.community_id {
            return Err(invalid("another channel"));
        }
        if state.keeper().map(|k| &k.pubkey) != Some(from) {
            return Err(invalid("an issue not from the keeper"));
        }
        issue.check().map_err(invalid)?;
        let issued = issue.issued().map_err(kd)?;
        // The scope it is of, by its bytes (wire §14.5): a scope the journal
        // here does not know yet is not taken (the keeper sends again).
        let scope_id = wire::scope_id_of(state, &issued.scope).ok_or_else(|| invalid("issued for a scope the journal does not know"))?;
        let mut out = Outcome::default();
        let short = state.community_id.get(..8).unwrap_or(&state.community_id);
        let step = |what: &str| self.env.say.mark(Some(format!("{} for {short}: {what}", wire::T_ISSUE)));
        step("the bundle and the keys taken");
        // Whether the issue brings anything: a bundle not held, a key not
        // held, or I was not in yet. One heard again (a note of the next
        // round of listening, an echo of my own keeper's) brings nothing,
        // and my other devices are not told again (живой запуск
        // 2026-10-23: every echo sent `own.channel_keys` once more).
        let joined = db::channels::get(&self.env.db, &state.community_id).await?.is_some_and(|r| r.membership == crate::service::MEMBERSHIP_JOINED);
        let new_bundle = !issued.bundle.is_empty() && db::keys::bundles(&self.env.db, &state.community_id, &scope_id).await?.iter().all(|(gen, bytes, _)| *gen != i64::from(issued.gen) || bytes.as_slice() != issued.bundle.as_ref());
        self.take_issued(state, &scope_id, &issued, &mut out).await?;
        let fresh = !joined || new_bundle || !out.keys.is_empty();
        if let Some(link) = &issue.link {
            step("the secret of the link kept");
            secrets::put_link_trusted(self.env.secrets.as_ref(), &state.community_id, link.link_epoch, &crate::link::LinkSecret::decode(&link.secret)?).await?;
            out.link = Some(link.clone());
        }
        if issued.missed_more {
            if let Some(last) = issued.missed.last() {
                step("the next page of missed keys asked for");
                let body = Missed { community_id: state.community_id.clone(), scope_id: scope_id.clone(), after_epoch: last.epoch };
                self.to_keeper(state, wire::T_MISSED, &body, now).await?;
                out.asked.push(wire::T_MISSED);
            }
        }
        step("the epochs that waited for a bundle");
        self.open_waiting(state, &scope_id, &mut out).await?;
        if fresh {
            step("my other devices told");
            self.send_own_keys(state, now).await?;
        }
        Ok(out)
    }

    /// A ticket a moderator handed me while the keeper was away (wire §17):
    /// its `Issued` of `everyone` taken as the keeper's would be; the
    /// keeper's own issue, when it comes, takes its place.
    pub(crate) async fn take_ticket(&self, state: &ChannelState, issued: &Issued, now: i64) -> Result<Outcome> {
        if issued.scope != wire::scope_ref(&state.community_id, SCOPE_EVERYONE)? {
            return Err(invalid("a ticket of another scope"));
        }
        let mut out = Outcome::default();
        self.take_issued(state, SCOPE_EVERYONE, issued, &mut out).await?;
        self.open_waiting(state, SCOPE_EVERYONE, &mut out).await?;
        self.send_own_keys(state, now).await?;
        Ok(out)
    }

    async fn take_issued(&self, state: &ChannelState, scope_id: &str, issued: &Issued, out: &mut Outcome) -> Result<()> {
        let cid = &state.community_id;
        if issued.scope != wire::scope_ref(cid, scope_id)? {
            return Err(invalid("issued for another scope"));
        }
        let mut keys = self.load(cid, scope_id).await?;
        keys.take_issued(cid, scope_id, issued)?;
        if !issued.bundle.is_empty() {
            let emergency = matches!(issued.kind, GenerationKind::Emergency);
            db::keys::put_bundle(&self.env.db, cid, scope_id, issued.gen, &issued.bundle, emergency).await?;
        }
        for k in issued.current.iter().chain(issued.missed.iter()) {
            self.keep(cid, scope_id, k, out).await?;
        }
        Ok(())
    }

    // ─── Epoch events ───────────────────────────────────────────────────────

    /// An epoch event (kind 1392), then those it lets through that ran
    /// ahead of it (`Verdict::Ahead`, kept in `cm_pending`): relays hand
    /// the newest first, so after a while offline the epochs come back in
    /// the wrong order and the near ones open the way for the far ones.
    ///
    /// Under an open history the chain of a newer epoch gives every key
    /// before it, and the keeper drops the blobs of an epoch as soon as the
    /// next one is out: of the epochs that come back together, only the
    /// newest opens by its shard (the others would cost a GET per host that
    /// finds nothing, communities-bench 2026-10-23).
    pub async fn on_epoch(&self, log: &OpLog, signed: &serde_json::Value, now: i64) -> Result<Outcome> {
        let cid = &log.state().community_id;
        let open = log.state().history == History::Open;
        let first_scope = signed["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "s")).and_then(|t| t[1].as_str()).unwrap_or(SCOPE_EVERYONE);
        let first_epoch = signed["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "n")).and_then(|t| t[1].as_str()).and_then(|n| n.parse::<i64>().ok()).unwrap_or(i64::MAX);
        let later = open && db::channels::has_pending_after(&self.env.db, cid, &ahead_marker(first_scope), first_epoch).await?;
        let mut out = self.on_epoch_one(log, signed, now, later).await?;
        let mut moved = matches!(out.verdict, Some(Verdict::Accepted { current: true, .. }));
        while moved {
            moved = false;
            let scope_id = signed["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "s")).and_then(|t| t[1].as_str()).unwrap_or(SCOPE_EVERYONE);
            let mut parked = db::channels::take_pending(&self.env.db, cid, &ahead_marker(scope_id)).await?;
            parked.sort_by_key(|r| r.epoch);
            let last = parked.len();
            for (i, row) in parked.into_iter().enumerate() {
                let value: serde_json::Value = serde_json::from_str(&row.event_json)?;
                let o = self.on_epoch_one(log, &value, now, open && i + 1 < last).await?;
                moved |= matches!(o.verdict, Some(Verdict::Accepted { current: true, .. }));
                out.keys.extend(o.keys);
                out.pending.extend(o.pending);
                out.out |= o.out;
                out.asked.extend(o.asked);
            }
        }
        Ok(out)
    }

    /// The key of an epoch the keeper kept on this device has just
    /// published, with its signed event: the key kept at once, then the
    /// event taken as a relay would hand it. The key is held, so no shard
    /// is fetched, and the row the key made gets its event (`put_epoch`
    /// fills a row without one); the echo of the relay is then a
    /// `Duplicate`. Hand over only what was published: content sealed
    /// under an epoch nobody else has heard of is read by nobody.
    pub async fn take_own_epoch(&self, log: &OpLog, scope_id: &str, key: &EpochKey, signed: &serde_json::Value, now: i64) -> Result<Outcome> {
        let mut out = Outcome::default();
        self.keep(&log.state().community_id, scope_id, key, &mut out).await?;
        let o = self.on_epoch(log, signed, now).await?;
        out.keys.extend(o.keys);
        out.pending.extend(o.pending);
        out.out |= o.out;
        out.closed.extend(o.closed);
        out.asked.extend(o.asked);
        out.verdict = o.verdict;
        Ok(out)
    }

    /// One epoch event: the rules of §5, then what the status says
    /// (§6.2) — open it with a shard, ask for a bundle, or leave.
    /// `by_chain`: a newer epoch of an open history came with it, whose
    /// chain gives this key; no shard is fetched.
    async fn on_epoch_one(&self, log: &OpLog, signed: &serde_json::Value, now: i64, by_chain: bool) -> Result<Outcome> {
        let state = log.state();
        let cid = &state.community_id;
        let scope_id = signed["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "s")).and_then(|t| t[1].as_str()).unwrap_or(SCOPE_EVERYONE).to_string();
        let ev = read_epoch(signed, cid, &scope_id)?;
        let digest = hex::encode(ev.digest);
        let newest_gen = ev.content.event.newest_gen();
        let ev_epoch = ev.content.event.epoch;
        let mut keys = self.load(cid, &scope_id).await?;
        let (verdict, status) = keys.on_epoch(state, cid, &scope_id, ev, now)?;
        let mut out = Outcome { verdict: Some(verdict.clone()), ..Outcome::default() };
        match &verdict {
            Verdict::Accepted { event, current, .. } => {
                let row = EpochRow {
                    community_id: cid.clone(),
                    scope_id: scope_id.clone(),
                    epoch: i64::from(event.epoch),
                    digest,
                    gen: newest_gen.map(i64::from),
                    key_hex: None,
                    event_json: serde_json::to_string(signed)?,
                    at: i64::try_from(event.at).unwrap_or(i64::MAX),
                };
                db::keys::put_epoch(&self.env.db, &row).await?;
                let held = keys.state(&scope_id).is_some_and(|s| s.key(event.epoch).is_some());
                // A flat epoch gives no key by its event (the keeper's issue
                // to each member does): `Out` of it says nothing.
                let flat = event.scheme == keydist::SchemeId::FlatV1;
                match status {
                    _ if held || flat => {}
                    // Out of a section, not of the channel: «раздел закрыт»
                    // (wire §14.5); `everyone` and the membership stay.
                    Some(Status::Out) if *current && scope_id != SCOPE_EVERYONE => {
                        if keys.state(&scope_id).is_some_and(|s| !s.bundles.is_empty()) {
                            tracing::info!(community = %cid, scope = %scope_id, "a section is closed to me");
                            out.closed.push(scope_id.clone());
                        }
                    }
                    Some(Status::Read { .. }) if by_chain => {}
                    // Its shard is fetched off this call (`fetch`, run by
                    // `ChannelService::run_fetch`): the row waits without
                    // its key, what is sealed under it in `cm_pending`.
                    Some(Status::Read { .. }) => {
                        self.env.fetches.want(cid, Want::Epoch { scope_id: scope_id.clone(), epoch: event.epoch });
                        tracing::debug!(community = %cid, epoch = event.epoch, "an epoch waits for its shard");
                    }
                    // Only a member asks: one who holds a bundle of an older
                    // generation. A device without any waits for a note of
                    // mine or asks by itself (`ChannelService::ensure_keys`);
                    // somebody who only reads the relay asks nothing.
                    Some(Status::NeedBundle { gen }) if keys.state(&scope_id).is_some_and(|s| !s.bundles.is_empty()) => {
                        self.ask_rebundle(state, &scope_id, gen, now).await?;
                        out.asked.push(wire::T_REBUNDLE);
                    }
                    Some(Status::Out) if *current => {
                        // The newest epoch leaves me out: banned, removed,
                        // or gone from another device.
                        out.out = true;
                        db::channels::set_membership(&self.env.db, cid, MEMBERSHIP_OUT).await?;
                        // Why, unless this device knows already: the journal says
                        // ban or removal; otherwise I left on another device.
                        if db::channels::get(&self.env.db, cid).await?.is_some_and(|r| r.out_reason.is_none()) {
                            let me = self.me()?;
                            let why = log
                                .ordered()
                                .filter(|o| log.rejection(&o.id()).is_none())
                                .filter_map(|o| match &o.body {
                                    OpBody::Ban { who, .. } if *who == me => Some(crate::service::OUT_BANNED),
                                    OpBody::Remove { who } if *who == me => Some(crate::service::OUT_REMOVED),
                                    OpBody::Unban { who, .. } if *who == me => Some(crate::service::OUT_LEFT),
                                    _ => None,
                                })
                                .last()
                                .unwrap_or(crate::service::OUT_LEFT);
                            db::channels::set_out_reason(&self.env.db, cid, Some(why)).await?;
                        }
                    }
                    _ => {}
                }
            }
            Verdict::Conflict => {
                tracing::warn!(community = %cid, "another event for an epoch already taken: ignored, the bundle asked for again");
                let gen = keys.state(&scope_id).and_then(|s| s.bundles.last().map(|b| b.gen)).unwrap_or(1);
                self.ask_rebundle(state, &scope_id, gen, now).await?;
                out.asked.push(wire::T_REBUNDLE);
            }
            Verdict::Duplicate => {}
            Verdict::Ahead => {
                let row = PendingRow {
                    event_id: signed["id"].as_str().unwrap_or_default().to_string(),
                    community_id: cid.clone(),
                    scope_id: scope_id.clone(),
                    epoch: i64::from(ev_epoch),
                    key_id: ahead_marker(&scope_id),
                    event_json: serde_json::to_string(signed)?,
                    created_at: signed["created_at"].as_i64().unwrap_or(now),
                    received_at: now,
                };
                db::channels::add_pending(&self.env.db, &row).await?;
            }
            Verdict::Refused(why) => tracing::info!(community = %cid, why, "an epoch event refused"),
        }
        if state.history == History::Open {
            self.follow_chains(cid, &scope_id, &mut out).await?;
        }
        Ok(out)
    }

    /// The event of an epoch whose row waits for its key, and the hosts of
    /// its blobs; none when the key is held or the row is gone.
    async fn waiting_epoch(&self, state: &ChannelState, scope_id: &str, epoch: u32) -> Result<Option<(Verified<EpochEvent>, Vec<String>)>> {
        let cid = &state.community_id;
        let rows = db::keys::epochs(&self.env.db, cid, scope_id).await?;
        let Some(row) = rows.into_iter().find(|r| r.epoch == i64::from(epoch) && r.key_hex.is_none() && !r.event_json.is_empty()) else { return Ok(None) };
        let value: serde_json::Value = serde_json::from_str(&row.event_json)?;
        let ev = read_epoch(&value, cid, scope_id)?;
        let hosts = if ev.content.blobs.is_empty() { state.storage.clone() } else { ev.content.blobs.clone() };
        Ok(Some((Verified::after_signature_check(ev.content.event), hosts)))
    }

    /// What a job must fetch to open an epoch (`fetch::Want::Epoch`), by
    /// the rows now: the index blob of the generation of my bundle and the
    /// shard of my leaf. None when there is nothing to fetch any more (the
    /// key came another way, the status is no longer `Read`).
    pub(crate) async fn plan_epoch(&self, state: &ChannelState, scope_id: &str, epoch: u32) -> Result<Option<fetch::Plan>> {
        let Some((ev, hosts)) = self.waiting_epoch(state, scope_id, epoch).await? else { return Ok(None) };
        let keys = self.load(&state.community_id, scope_id).await?;
        let Some(Status::Read { gen, shard }) = keys.state(scope_id).map(|m| m.status(&ev)) else { return Ok(None) };
        let tree = ev.tree(gen).ok_or_else(|| invalid("generation not in the event"))?;
        // Stage 1 reads by the index blob; a tree with more shards than an
        // index holds names a Merkle root, whose proofs a host serves by
        // no hash the event gives.
        let index = tree.index.ok_or_else(|| invalid("an epoch without an index blob"))?;
        Ok(Some(fetch::Plan::Epoch { index, shard, hosts }))
    }

    /// An epoch opened with what a job fetched (the witness and the shard,
    /// each checked by its hash), its key kept, unless it came another way
    /// meanwhile; then the chain and the epochs that still wait.
    pub(crate) async fn open_fetched(&self, state: &ChannelState, scope_id: &str, epoch: u32, index: &[u8], shard: &[u8], out: &mut Outcome) -> Result<()> {
        let Some((ev, _)) = self.waiting_epoch(state, scope_id, epoch).await? else { return Ok(()) };
        let mut keys = self.load(&state.community_id, scope_id).await?;
        let k = keys.open(scope_id, &ev, shard, index)?;
        self.keep(&state.community_id, scope_id, &k, out).await?;
        self.open_waiting(state, scope_id, out).await
    }

    /// Under an open history: every key held opens the one before it from
    /// the chain of its event, as far back as the events here go (wire
    /// §6.3: one event per epoch).
    async fn follow_chains(&self, community_id: &str, scope_id: &str, out: &mut Outcome) -> Result<()> {
        loop {
            let rows = db::keys::epochs(&self.env.db, community_id, scope_id).await?;
            let held: std::collections::BTreeSet<i64> = rows.iter().filter(|r| r.key_hex.is_some()).map(|r| r.epoch).collect();
            let mut keys = self.load(community_id, scope_id).await?;
            let mut progress = false;
            for row in rows.iter().rev().filter(|r| r.key_hex.is_some() && !r.event_json.is_empty()) {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.event_json) else { continue };
                let Ok(ev) = read_epoch(&value, community_id, scope_id) else { continue };
                let Some(prev) = ev.content.event.prev_epoch else { continue };
                if held.contains(&i64::from(prev)) || ev.content.event.chain.is_none() {
                    continue;
                }
                let verified = Verified::after_signature_check(ev.content.event);
                match keys.follow_chain(scope_id, &verified) {
                    Ok(k) => {
                        self.keep(community_id, scope_id, &k, out).await?;
                        progress = true;
                        break;
                    }
                    Err(e) => tracing::debug!(community = %community_id, epoch = row.epoch, error = %e, "a chain did not open"),
                }
            }
            if !progress {
                return Ok(());
            }
        }
    }

    /// Epochs whose events came before the bundle that opens them (a
    /// newcomer, a new generation, `from_join` catching up): what the keys
    /// held open now (the chain), the rest wanted of the hosts off this
    /// call (`fetch`; nothing here waits for them). Under an open history
    /// only the newest such epoch: its chain gives the ones before it.
    pub async fn open_waiting(&self, state: &ChannelState, scope_id: &str, out: &mut Outcome) -> Result<()> {
        let cid = &state.community_id;
        let open = state.history == History::Open;
        if open {
            self.follow_chains(cid, scope_id, out).await?;
        }
        let rows = db::keys::epochs(&self.env.db, cid, scope_id).await?;
        let keys = self.load(cid, scope_id).await?;
        let Some(member) = keys.state(scope_id) else { return Ok(()) };
        for row in rows.iter().rev().filter(|r| r.key_hex.is_none() && !r.event_json.is_empty()) {
            let value: serde_json::Value = serde_json::from_str(&row.event_json)?;
            let ev = read_epoch(&value, cid, scope_id)?;
            let verified = Verified::after_signature_check(ev.content.event);
            if let Status::Read { .. } = member.status(&verified) {
                self.env.fetches.want(cid, Want::Epoch { scope_id: scope_id.to_string(), epoch: verified.epoch });
                if open {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Whether I hold a bundle of the channel (a new device holds none
    /// until a note of my own or a rebundle).
    pub async fn has_bundle(&self, community_id: &str, scope_id: &str) -> Result<bool> {
        Ok(!db::keys::bundles(&self.env.db, community_id, scope_id).await?.is_empty())
    }

    // ─── My other devices ───────────────────────────────────────────────────

    /// `own.channel_keys` to my other devices: the newest bundle, the
    /// newest keys and the link (wire §6.4).
    pub async fn send_own_keys(&self, state: &ChannelState, now: i64) -> Result<()> {
        let cid = &state.community_id;
        let Some(mut body) = self.own_keys(state, SCOPE_EVERYONE).await? else { return Ok(()) };
        for scope_id in state.scopes.keys().filter(|s| *s != SCOPE_EVERYONE) {
            if let Some(o) = self.own_keys(state, scope_id).await? {
                body.scopes.push(OwnScopeKeys { scope_id: o.scope_id, gen: o.gen, issued: o.issued, epochs: o.epochs });
            }
        }
        // The owner's other devices hold the root too: any of them may take
        // the keeper over (wire §15).
        if state.owner == self.me()? {
            if let Some(root) = secrets::root(self.env.secrets.as_ref(), cid).await? {
                let serial = secrets::root_serial(self.env.secrets.as_ref(), cid).await?;
                body.root = Some(OwnRoot { serial, secret: B64.encode(root.as_bytes()) });
            }
        }
        // To my own inbox by the outbox, off this call.
        crate::outbox::queue_note(&self.env, cid, None, &[], T_OWN_KEYS, &body, 0, None, now).await?;
        tracing::debug!(community = %cid, "own.channel_keys queued");
        Ok(())
    }

    async fn own_keys(&self, state: &ChannelState, scope_id: &str) -> Result<Option<OwnKeys>> {
        let cid = &state.community_id;
        let bundles = db::keys::bundles(&self.env.db, cid, scope_id).await?;
        let rows = db::keys::epochs(&self.env.db, cid, scope_id).await?;
        let mut epochs: Vec<OwnEpoch> =
            rows.iter().rev().filter_map(|r| Some(OwnEpoch { epoch: u32::try_from(r.epoch).ok()?, key: r.key_hex.clone()? })).take(OWN_KEYS_MAX).collect();
        epochs.reverse();
        let issued = match bundles.last() {
            Some((_, bytes, emergency)) => {
                let b = Bundle::from_bytes(bytes).map_err(kd)?;
                Issued {
                    scheme: b.scheme,
                    scope: b.scope,
                    gen: b.gen,
                    leaf: b.leaf,
                    kind: if *emergency { GenerationKind::Emergency } else { GenerationKind::Initial },
                    bundle: SecretBytes::new(bytes.clone()),
                    current: None,
                    missed: vec![],
                    missed_more: false,
                }
            }
            // A flat scope has no bundle: its keys are all there is.
            None if state.scopes.get(scope_id).is_some_and(|d| d.scheme == keydist::SchemeId::FlatV1) && !epochs.is_empty() => Issued {
                scheme: keydist::SchemeId::FlatV1,
                scope: wire::scope_ref(cid, scope_id)?,
                gen: 0,
                leaf: 0,
                kind: GenerationKind::Initial,
                bundle: SecretBytes::default(),
                current: None,
                missed: vec![],
                missed_more: false,
            },
            None => return Ok(None),
        };
        let gen = issued.gen;
        let held = secrets::links(self.env.secrets.as_ref(), cid).await?;
        let link = held.get(&state.link_epoch).map(|s| LinkPayload { link_epoch: state.link_epoch, secret: s.encode() });
        Ok(Some(OwnKeys {
            community_id: cid.clone(),
            scope_id: scope_id.to_string(),
            gen,
            issued: Issue::encode_issued(&issued),
            epochs,
            link,
            scopes: Vec::new(),
            root: None,
        }))
    }

    /// `own.channel_keys` from another device of mine (the seal is mine:
    /// the caller checked it): the bundle, the keys, the link taken as if
    /// the keeper had issued them.
    pub async fn on_own_keys(&self, state: &ChannelState, from: &PubKey, body: &OwnKeys) -> Result<Outcome> {
        if from != &self.me()? || body.community_id != state.community_id {
            return Err(invalid("own keys not mine or of another channel"));
        }
        let mut out = Outcome::default();
        self.take_own_scope(state, &body.scope_id, body.gen, &body.issued, &body.epochs, &mut out).await?;
        if let Some(link) = &body.link {
            secrets::put_link_trusted(self.env.secrets.as_ref(), &state.community_id, link.link_epoch, &crate::link::LinkSecret::decode(&link.secret)?).await?;
            out.link = Some(link.clone());
        }
        // The root of my channel from my other device: kept unless the one
        // here is of a newer series (wire §15).
        if let Some(r) = body.root.as_ref().filter(|_| state.owner == *from) {
            self.take_own_root(&state.community_id, r).await?;
        }
        // The sections: one that does not take (a scope this journal does
        // not know yet) holds back none of the others.
        for s in &body.scopes {
            if let Err(e) = self.take_own_scope(state, &s.scope_id, s.gen, &s.issued, &s.epochs, &mut out).await {
                tracing::info!(community = %state.community_id, scope = %s.scope_id, error = %e, "keys of a section from my other device not taken");
            }
        }
        Ok(out)
    }

    /// A root secret from another device of mine: kept when this device
    /// holds none, or one of an older series.
    async fn take_own_root(&self, community_id: &str, r: &OwnRoot) -> Result<()> {
        let raw: [u8; 32] = B64.decode(&r.secret).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("root"))?;
        let held = secrets::root(self.env.secrets.as_ref(), community_id).await?;
        let serial = secrets::root_serial(self.env.secrets.as_ref(), community_id).await?;
        if held.is_some() && r.serial <= serial {
            return Ok(());
        }
        secrets::put_root_of(self.env.secrets.as_ref(), community_id, &keydist::RootSecret::from_bytes(raw), r.serial).await?;
        tracing::info!(community = %community_id, serial = r.serial, "the root of my channel came from my other device");
        Ok(())
    }

    /// The keys of one scope from another device of mine.
    async fn take_own_scope(&self, state: &ChannelState, scope_id: &str, gen: u32, issued: &str, epochs: &[OwnEpoch], out: &mut Outcome) -> Result<()> {
        let bytes = SecretBytes::new(B64.decode(issued).map_err(|_| invalid("issued"))?);
        let mut issued = Issued::from_bytes(&bytes).map_err(kd)?;
        if issued.gen != gen {
            return Err(invalid("gen"));
        }
        issued.missed = epochs.iter().map(|e| key_of(e.epoch, &e.key)).collect::<Result<Vec<_>>>()?;
        issued.missed.sort_by_key(|k| k.epoch);
        self.take_issued(state, scope_id, &issued, out).await?;
        self.open_waiting(state, scope_id, out).await
    }

    /// The key that opens a sealed event of `epoch` with `k`, if held.
    pub async fn key_for(&self, community_id: &str, scope_id: &str, epoch: u32, key_id: &str) -> Result<Option<EpochKey>> {
        let rows = db::keys::epochs(&self.env.db, community_id, scope_id).await?;
        let Some(row) = rows.iter().find(|r| r.epoch == i64::from(epoch)) else { return Ok(None) };
        let Some(k) = row.key_hex.as_deref() else { return Ok(None) };
        let key = key_of(epoch, k)?;
        Ok((wire::key_id(&key).0 == key_id).then_some(key))
    }

    /// The newest key held: what new content is sealed with.
    pub async fn newest_key(&self, community_id: &str, scope_id: &str) -> Result<Option<EpochKey>> {
        let rows = db::keys::epochs(&self.env.db, community_id, scope_id).await?;
        rows.iter().rev().find_map(|r| r.key_hex.as_deref().map(|k| (r.epoch, k.to_string()))).map(|(e, k)| key_of(u32::try_from(e).map_err(|_| invalid("epoch"))?, &k)).transpose()
    }

    /// The epoch that `live` content may be sealed under at least (wire
    /// §4: no older than the current one less one).
    pub async fn live_ok(&self, community_id: &str, scope_id: &str, epoch: u32) -> Result<bool> {
        let keys = self.load(community_id, scope_id).await?;
        Ok(keys.acceptance(scope_id).is_none_or(|a| a.live_content_ok(epoch)))
    }
}


// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper moving between the owner's devices (see
//! internal/communities-wire.md §15, internal/communities-spec.md §7.5, §7.6).
//!
//! - Taking it over (`keeper_take`, on device B): a new keeper key, the
//!   `keeper` operation of the next series from the epoch after the newest
//!   heard here, and a wait for the state of the keeper before it.
//! - Handing it over (`hand_over`, on device A, as soon as the journal names
//!   a later series): the rows of the keeper and the masters of its live
//!   generations, sealed with a fresh key into a blob on the hosts of the
//!   channel; `keeper.handover` with that key to the inbox of the new
//!   series; then the keeper's rows and key go from A. Until the blob and
//!   the note are out A keeps its rows and tries again on its turns, but
//!   never acts as the keeper again.
//! - B takes the rows (`on_keeper_handover`), begins its series at its
//!   `from_epoch` (every scope) and publishes the next epoch: the readers
//!   go on with the bundles they hold.
//! - A gone (`keeper_take` with `emergency`): B makes a new root, an
//!   emergency generation per scope with the readers of the newest snapshot
//!   and hands each a bundle; the readers drop the old ones (`Issued.kind`).

use crate::db::{self, handover::TakeRow};
use crate::keeper::Keeper;
use crate::log::OpLog;
use crate::notes::{self, Opened};
use crate::op::OpBody;
use crate::secrets;
use crate::service::{ChannelService, Outcome, UI_EVENT_CHANNEL_UPDATED};
use crate::wire::{self, KeeperHandover};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use keydist::{ChannelKey, GenerationMaster, RootSecret};
use messenger_core::{MessengerError, Outbound, PubKey, RelayUrl, Result};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Why an emergency cannot begin: the readers of the newest snapshot here
/// are not to be had (its shards not on the hosts, or sealed under a key
/// of an epoch this device does not hold).
pub const NO_READERS: &str = "the readers of the newest snapshot cannot be had or opened here";

/// Why a take of the keeper waits: none of the current keeper's epochs is
/// here (the channel not caught up from its relays). The app words it as
/// `channel_not_here_yet`.
pub const EPOCHS_NOT_HERE: &str = "the epochs of the channel are not here yet";

/// A series cannot begin: the epoch numbers would run out.
pub const EPOCHS_EXHAUSTED: &str = "the epoch numbers of the channel are exhausted";

/// The epochs a new series must have room for past its first one.
const SERIES_ROOM: u32 = 1 << 20;
/// How often `keeper_take_at_head` looks at what came, and how long it
/// must stay the same.
const HEAD_POLL: std::time::Duration = std::time::Duration::from_millis(250);
const HEAD_QUIET: std::time::Duration = std::time::Duration::from_secs(2);

/// The version of the blob of a handover.
pub const DUMP_VERSION: u8 = 1;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

/// What the blob of `keeper.handover` holds: the keeper's rows by table,
/// the masters of the live generations and the secrets of the link.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dump {
    pub v: u8,
    pub community_id: String,
    pub tables: BTreeMap<String, db::handover::TableRows>,
    pub masters: Vec<DumpMaster>,
    /// Link epoch → `LinkSecret::encode`.
    pub links: BTreeMap<u32, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DumpMaster {
    pub scope_id: String,
    pub gen: u32,
    /// Base64 of 32 bytes.
    pub master: String,
}

impl ChannelService {
    /// Whether a keeper kept here belongs to a series the journal has gone
    /// past.
    pub(crate) async fn superseded(&self, community_id: &str, log: &OpLog) -> Result<bool> {
        let Some(row) = db::keeper::keeper(&self.env.db, community_id).await? else { return Ok(false) };
        Ok(log.state().keeper().is_some_and(|k| k.pubkey.as_hex() != row.keeper_pubkey))
    }

    /// The take this device waits for (none: not taking the keeper).
    pub async fn keeper_take_status(&self, community_id: &str) -> Result<Option<TakeRow>> {
        db::handover::take(&self.env.db, community_id).await
    }

    /// Whether this device holds the root secret of the channel (the owner's
    /// devices do, by their notes of keys): an emergency keeper needs it.
    pub async fn holds_root(&self, community_id: &str) -> Result<bool> {
        Ok(secrets::root(self.env.secrets.as_ref(), community_id).await?.is_some())
    }

    /// Whether the keeper kept here waits to hand its state over.
    pub async fn keeper_handing(&self, community_id: &str) -> Result<bool> {
        match self.log(community_id).await? {
            Some(log) => self.superseded(community_id, &log).await,
            None => Ok(false),
        }
    }

    /// The first epoch of a new keeper series: past every epoch heard here
    /// in any scope, with one to spare for an epoch the keeper before may
    /// publish before it hears of the series (wire §15). Refused, never
    /// wrapped, when the numbers run out (a keeper key that signed an epoch
    /// near the end of the numbers).
    pub(crate) async fn next_series_epoch(&self, community_id: &str, after: u32) -> Result<u32> {
        // Every epoch heard counts, not only those whose keys are held: a
        // series that began at an epoch already published would be a
        // conflict to every reader holding it.
        let newest = self.newest_epoch_known(community_id).await?;
        let past = newest.checked_add(2).ok_or_else(|| invalid(EPOCHS_EXHAUSTED))?;
        let next = after.checked_add(1).ok_or_else(|| invalid(EPOCHS_EXHAUSTED))?;
        // Room for the epochs of the series itself (keydist adds one per
        // epoch and does not wrap).
        let from = past.max(next);
        if from > u32::MAX - SERIES_ROOM {
            return Err(invalid(EPOCHS_EXHAUSTED));
        }
        Ok(from)
    }

    // ─── Device B: taking the keeper over ───────────────────────────────────

    /// «Сделать хранителем это устройство» (spec §7.5): the owner on a
    /// device that does not keep the channel. `emergency`: the keeper
    /// before is gone (a lost phone, spec §7.6): no state is waited for, a
    /// new generation is started here at once; asked for again while a
    /// handover waits, it gives up the wait. `keeper_take_at_head` brings
    /// the channel to its head from the relays first; this one takes what
    /// is here, and is refused when none of the current keeper's epochs is.
    pub async fn keeper_take(&self, keys: &Keys, community_id: &str, emergency: bool) -> Result<Outcome> {
        self.keeper_take_with(keys, community_id, emergency, false).await
    }

    /// `keeper_take`; `force`: the owner's word that the current keeper
    /// series never published an epoch (a keeper lost before its first
    /// turn), so that none heard is no reason to wait.
    pub async fn keeper_take_with(&self, keys: &Keys, community_id: &str, emergency: bool, force: bool) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let state = log.state().clone();
        if state.owner != me || state.disbanded {
            return Err(invalid("not permitted"));
        }
        let take = db::handover::take(&self.env.db, community_id).await?;
        if self.keeper(community_id).await?.is_some() {
            // An emergency that failed after its keeper was written and
            // before its row went: the keeper is here, the row says nothing more.
            if take.is_some() {
                db::handover::drop_take(&self.env.db, community_id).await?;
                return Ok(Outcome::default());
            }
            return Err(invalid("this device keeps the channel"));
        }
        let held_root = secrets::root(self.env.secrets.as_ref(), community_id).await?.is_some();
        if let Some(take) = take {
            let current = state.keeper().is_some_and(|k| k.pubkey.as_hex() == take.keeper_pubkey && i64::from(k.serial) == take.serial);
            if current {
                if !emergency {
                    return Ok(Outcome::default());
                }
                // An emergency asked again: it gives up a handover's wait,
                // and it resumes an emergency of its own that failed
                // half-way (its `keeper` operation is out, the row says
                // emergency, no keeper is installed here).
                let readers = self.emergency_readers(keys, &log).await?;
                return self.take_emergency(keys, &log, take, readers).await;
            }
            // The journal went past the series this row waits for (another
            // device took the keeper since), or never named it: the row and
            // its key are of nothing.
            tracing::info!(community = %community_id, serial = take.serial, "a take of the keeper the journal no longer names: dropped");
            db::handover::drop_take(&self.env.db, community_id).await?;
            secrets::forget_keeper_keys(self.env.secrets.as_ref(), community_id).await?;
        }
        let last = state.keeper().ok_or_else(|| invalid("no keeper"))?;
        // The series is numbered past the epochs heard here: a device that
        // heard none of the keeper's (a fresh folder of the owner that has
        // the journal alone) would begin at an epoch the keeper published,
        // and every reader holding that one would take the new one for a
        // conflict (live run 2026-10-30). `keeper_take_at_head` catches the
        // channel up first; `force` is the owner's word that there is none.
        if !force && self.newest_epoch_known(community_id).await? < last.from_epoch {
            return Err(invalid(EPOCHS_NOT_HERE));
        }
        // An emergency whose readers cannot be had would fail after its
        // `keeper` operation went out, the series announced and kept by no
        // one: the snapshot is opened first, nothing written before.
        let readers = if emergency { Some(self.emergency_readers(keys, &log).await?) } else { None };
        let serial = last.serial.checked_add(1).ok_or_else(|| invalid("keeper"))?;
        let from_epoch = self.next_series_epoch(community_id, last.from_epoch).await?;
        let kk = Keys::generate();
        let pubkey = Self::me(&kk)?;
        secrets::put_keeper_keys(self.env.secrets.as_ref(), community_id, &kk).await?;
        let take = TakeRow {
            community_id: community_id.to_string(),
            keeper_pubkey: pubkey.as_hex().to_string(),
            serial: i64::from(serial),
            from_epoch: i64::from(from_epoch),
            mode: if emergency { db::handover::MODE_EMERGENCY } else { db::handover::MODE_HANDOVER }.into(),
            started_at: self.now(),
        };
        // The wait first: the keeper before may answer before the operation
        // is even back here.
        db::handover::put_take(&self.env.db, &take).await?;
        let body = OpBody::Keeper { pubkey, serial, from_epoch, inbox: Some(me) };
        let mut out = match self.act(keys, community_id, body).await {
            Ok(out) => out,
            Err(e) => {
                db::handover::drop_take(&self.env.db, community_id).await?;
                secrets::forget_keeper_keys(self.env.secrets.as_ref(), community_id).await?;
                return Err(e);
            }
        };
        tracing::info!(community = %community_id, serial, from_epoch, emergency, root = held_root, "this device takes the keeper over");
        if let Some(readers) = readers {
            let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
            out.merge(self.take_emergency(keys, &log, take, readers).await?);
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "keeper": "taking" }));
        Ok(out)
    }

    /// The readers of the new generation of an emergency: those of the
    /// newest snapshot (`NO_READERS` when it cannot be had or opened), less
    /// whom the journal put out after it. The snapshot may be older than a
    /// removal or a ban (a small channel's comes some seconds after a
    /// change, and the keeper may be lost in between: an e2e of 2026-10-30,
    /// the one removed read the emergency keeper's posts). "After" is by the
    /// journal's ancestry: an operation that is no ancestor of the heads the
    /// snapshot names (one concurrent with it too) is after it; by the
    /// clocks only when one of those heads is not here. Operations the
    /// journal rejected (a demoted moderator's) put nobody out.
    pub(crate) async fn emergency_readers(&self, keys: &Keys, log: &OpLog) -> Result<Vec<crate::snapshot::Entry>> {
        let state = log.state();
        let member = self.member(keys);
        let mut readers = member.all_readers(log).await?.ok_or_else(|| invalid(NO_READERS))?;
        let snapshot = member.latest_snapshot(&state.community_id).await?;
        // What the snapshot was made after: its heads and every operation
        // they descend from. One concurrent with it (on no path to its
        // heads) is after it, wherever the order of the journal puts it.
        let before = snapshot.as_ref().and_then(|s| {
            let mut seen: std::collections::HashSet<crate::op::OpId> = std::collections::HashSet::new();
            let mut todo: Vec<crate::op::OpId> = s.heads.clone();
            while let Some(id) = todo.pop() {
                if seen.contains(&id) {
                    continue;
                }
                let op = log.get(&id)?;
                todo.extend(op.parents.iter().cloned());
                seen.insert(id);
            }
            Some(seen)
        });
        let mut out_after: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out_at: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for op in log.ordered() {
            if log.rejection(&op.id()).is_some() {
                continue;
            }
            if let OpBody::Ban { who, .. } | OpBody::Remove { who } = &op.body {
                let who = who.as_hex().to_string();
                match &before {
                    Some(b) if b.contains(&op.id()) => {}
                    Some(_) => {
                        out_after.insert(who);
                    }
                    None => {
                        let t = out_at.entry(who).or_insert(op.created_at);
                        *t = (*t).max(op.created_at);
                    }
                }
            }
        }
        readers.retain(|r| {
            let who = hex::encode(r.pubkey);
            !state.banned.contains(&who) && !out_after.contains(&who) && out_at.get(&who).is_none_or(|t| i64::from(r.since) > *t)
        });
        Ok(readers)
    }

    /// The keeper before is gone: a new root (the lost device held the old
    /// one), an emergency generation per scope from `readers`
    /// (`emergency_readers`), a bundle to each of them. A run that failed
    /// after the keeper was written is taken up from there.
    async fn take_emergency(&self, keys: &Keys, log: &OpLog, take: TakeRow, mut readers: Vec<crate::snapshot::Entry>) -> Result<Outcome> {
        let state = log.state();
        let cid = state.community_id.clone();
        let me = Self::me(keys)?;
        let kk = secrets::keeper_keys(self.env.secrets.as_ref(), &cid).await?.ok_or_else(|| MessengerError::SecretMissing(secrets::keeper_name(&cid)))?;
        let mine: [u8; 32] = hex::decode(me.as_hex()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("my key"))?;
        if !readers.iter().any(|r| r.pubkey == mine) {
            readers.push(crate::snapshot::Entry { pubkey: mine, flags: crate::snapshot::FLAG_READER, levels: 1, since: u32::try_from(self.now()).unwrap_or(0) });
        }
        let serial = u32::try_from(take.serial).map_err(|_| invalid("keeper"))?;
        let from_epoch = u32::try_from(take.from_epoch).map_err(|_| invalid("keeper"))?;
        let now = self.now();
        let written = db::keeper::keeper(&self.env.db, &cid).await?.is_some_and(|r| r.keeper_pubkey == take.keeper_pubkey);
        if !written {
            let mut gens = BTreeMap::new();
            for scope_id in state.scopes.keys() {
                gens.insert(scope_id.clone(), db::keys::newest_gen(&self.env.db, &cid, scope_id).await?);
            }
            let root = RootSecret::generate();
            let recovery = crate::keeper::Recovery { root: &root, readers: &readers, gens: &gens, from_epoch };
            Keeper::write_emergency(&self.env, state, &kk, &recovery, now).await?;
            secrets::put_root_of(self.env.secrets.as_ref(), &cid, &root, serial).await?;
        }
        db::handover::drop_take(&self.env.db, &cid).await?;
        let k = self.install_keeper(&cid, log).await?;
        k.lock().await.reissue_all(log, now).await?;
        // Its first epoch is the turn's, asked for now (no network here).
        self.ask_turn();
        // My other devices take the new root by my next note of keys.
        if let Err(e) = self.member(keys).send_own_keys(state, now).await {
            tracing::info!(community = %cid, error = %e, "the new root did not reach my other devices yet");
        }
        tracing::warn!(community = %cid, serial, from_epoch, readers = readers.len(), "the keeper was recovered here: an emergency generation");
        let mut out = Outcome { wake: Some(u64::try_from(now).unwrap_or(0)), ..Outcome::default() };
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "keeper": "here" }));
        Ok(out)
    }

    /// The keeper of the rows here, loaded and put in `keepers`.
    pub(crate) async fn install_keeper(&self, community_id: &str, log: &OpLog) -> Result<Arc<Mutex<Keeper>>> {
        let k = Keeper::load(self.env.clone(), log.state(), self.now()).await?;
        let pk = k.pubkey()?;
        let k = Arc::new(Mutex::new(k));
        self.keepers.lock().await.insert(community_id.to_string(), k.clone());
        self.remember_keeper(community_id, pk.as_hex());
        Ok(k)
    }

    /// `keeper.handover` (wire §15): taken only by the device that waits
    /// for exactly that series, from the key of the series before it.
    pub(crate) async fn on_keeper_handover(&self, n: &Opened, body: &KeeperHandover) -> Result<Outcome> {
        let mut out = Outcome::default();
        let cid = body.community_id.clone();
        let Some(take) = db::handover::take(&self.env.db, &cid).await? else {
            out.note("a handover nobody here waits for");
            return Ok(out);
        };
        let Some(log) = self.log(&cid).await? else { return Ok(out) };
        let state = log.state();
        let mine = state.keepers.iter().find(|k| i64::from(k.serial) == take.serial);
        let before = state.keepers.iter().find(|k| i64::from(k.serial) + 1 == take.serial);
        let ok = body.check().is_ok()
            && i64::from(body.serial) == take.serial
            && mine.is_some_and(|k| k.pubkey.as_hex() == take.keeper_pubkey)
            && before.is_some_and(|k| k.pubkey == n.sender);
        if !ok {
            out.note("a handover not from the keeper before mine, or not for my series");
            return Ok(out);
        }
        let raw: [u8; 32] = hex::decode(&body.sha256).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("sha256"))?;
        let sealed = crate::blobs::fetch_verified(self.env.blobs.as_ref(), &body.hosts, &raw).await?;
        let key: [u8; 32] = B64.decode(&body.key).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("key"))?;
        let plain = wire::open_bytes(&ChannelKey(key), &B64.encode(&sealed))?;
        let dump: Dump = serde_json::from_slice(&plain)?;
        if dump.community_id != cid || dump.v != DUMP_VERSION {
            out.note("a handover of another channel or version");
            return Ok(out);
        }
        self.take_dump(&cid, &dump, &take).await?;
        let from_epoch = u32::try_from(take.from_epoch).map_err(|_| invalid("keeper"))?;
        let now = self.now();
        let k = self.install_keeper(&cid, &log).await?;
        let mut k = k.lock().await;
        // A new owner's keeper (wire §16): a generation of his root in
        // every scope, before the first epoch of the series carries it.
        let transfer = take.mode == db::handover::MODE_TRANSFER;
        if transfer {
            k.migrate_all(crate::transfer::TRANSFER_GRACE_SECS, now).await?;
        }
        k.start_series(from_epoch, now).await?;
        if transfer {
            k.reissue_all(&log, now).await?;
        }
        let queue = self.drain_keeper(&mut k, &log, now).await?;
        drop(k);
        // The blob served its turn: removed by the next turn's job of the
        // hosts (a host lets only who put it remove it, which is my user
        // key when the keeper before was my device: `Env::delete_blob`).
        for host in &body.hosts {
            db::keeper::retire(&self.env.db, &cid, host, &body.sha256, now).await?;
        }
        tracing::info!(community = %cid, serial = take.serial, from_epoch, "the keeper came over to this device");
        // The first epoch of the series is the turn's, asked for now: run
        // here it held the channel's lock across its uploads.
        self.ask_turn();
        out.wake = Some(queue.map_or(u64::try_from(now).unwrap_or(0), |q| q.min(u64::try_from(now).unwrap_or(0))));
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "keeper": "here" }));
        Ok(out)
    }

    /// The rows, the masters and the secrets of the link of a dump, here.
    async fn take_dump(&self, cid: &str, dump: &Dump, take: &TakeRow) -> Result<()> {
        db::handover::import(&self.env.db, cid, &dump.tables).await?;
        db::keeper::put_keeper(&self.env.db, cid, &take.keeper_pubkey, self.now()).await?;
        for m in &dump.masters {
            let raw: [u8; 32] = B64.decode(&m.master).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("master"))?;
            secrets::put_master(self.env.secrets.as_ref(), cid, &m.scope_id, m.gen, &GenerationMaster::from_bytes(raw)).await?;
        }
        for (epoch, s) in &dump.links {
            secrets::put_link_trusted(self.env.secrets.as_ref(), cid, *epoch, &crate::link::LinkSecret::decode(s)?).await?;
        }
        db::handover::drop_take(&self.env.db, cid).await
    }

    // ─── Device A: handing the keeper over ──────────────────────────────────

    /// The keeper kept here, of a series the journal has gone past: its
    /// state to the inbox of the newest series, then gone from here.
    /// `false` when there was nothing to hand over.
    pub async fn hand_over(&self, community_id: &str) -> Result<bool> {
        let Some(log) = self.log(community_id).await? else { return Ok(false) };
        if !self.superseded(community_id, &log).await? {
            return Ok(false);
        }
        let state = log.state();
        let next = state.keeper().ok_or_else(|| invalid("no keeper"))?.clone();
        // Out of the map first: from here on nothing runs it.
        let held = self.keepers.lock().await.remove(community_id);
        let k = match held {
            Some(k) => k,
            None => Arc::new(Mutex::new(Keeper::load(self.env.clone(), state, self.now()).await?)),
        };
        let k = k.lock().await;
        let kkeys = k.keys().clone();
        let masters = k
            .live_masters()
            .into_iter()
            .map(|(scope_id, gen, m)| DumpMaster { scope_id, gen, master: B64.encode(m) })
            .collect();
        drop(k);
        let links = secrets::links(self.env.secrets.as_ref(), community_id).await?.into_iter().map(|(e, s)| (e, s.encode())).collect();
        let dump = Dump { v: DUMP_VERSION, community_id: community_id.to_string(), tables: db::handover::export(&self.env.db, community_id).await?, masters, links };
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| MessengerError::Crypto(e.to_string()))?;
        let sealed = B64.decode(wire::seal_bytes(&ChannelKey(key), &serde_json::to_vec(&dump)?)?).map_err(|_| invalid("sealed"))?;
        let sha = crate::blobs::sha256_hex(&sealed);
        let mut hosts = Vec::new();
        let uploader = self.env.uploader(&kkeys);
        for (host, key) in &self.env.writable(&uploader, &state.storage, Some(community_id)).await {
            match self.env.blobs.put_keyed(&uploader, key.as_deref(), host, &sha, &sealed).await {
                Ok(()) => hosts.push(host.clone()),
                Err(e) => tracing::info!(community = %community_id, host, error = %e, "a host did not take the state of the keeper"),
            }
        }
        if hosts.is_empty() {
            return Err(MessengerError::Transport("err.not_delivered: no host took the state of the keeper".into()));
        }
        let body = KeeperHandover { community_id: community_id.to_string(), serial: next.serial, sha256: sha, hosts, key: B64.encode(key) };
        let content = wire::note_envelope(wire::T_KEEPER_HANDOVER, &body)?;
        let event = notes::wrap_note(&kkeys, next.inbox(), &content, self.now())?;
        let hint_relays = state.relays.iter().filter_map(|r| RelayUrl::parse(r)).collect();
        let ack = self.env.net.send(Outbound::PublishToInbox { recipient: next.inbox().clone(), event, hint_relays }).await?;
        if ack.accepted_by.is_empty() {
            return Err(MessengerError::Transport("err.not_delivered: the handover note".into()));
        }
        // Gone from here: the rows, the keeper's key; the root stays only
        // with its owner (a former owner's is no longer the channel's).
        db::keeper::delete_keeper(&self.env.db, community_id).await?;
        secrets::forget_keeper_keys(self.env.secrets.as_ref(), community_id).await?;
        let me: Option<PubKey> = self.signer().and_then(|s| PubKey::parse(&s.public_key().to_hex()));
        if me.is_some_and(|m| m != state.owner) {
            secrets::forget_root(self.env.secrets.as_ref(), community_id).await?;
        }
        if let Ok(mut s) = self.statuses.lock() {
            s.remove(community_id);
        }
        if let Ok(mut m) = self.keeper_pks.lock() {
            m.remove(community_id);
        }
        tracing::info!(community = %community_id, serial = next.serial, "the keeper was handed over to the next series");
        Ok(true)
    }
}

impl ChannelService {
    /// An epoch event heard of a channel: its number counts toward where a
    /// new keeper series must begin, whatever the member side makes of it
    /// (taken, a conflict, out of its range), kept across a restart
    /// (`cm_epochs_heard`). Counted only when its signature is good, its
    /// signer is a keeper the journal names, it is within that keeper's
    /// range (a superseded series up to the next one's first epoch), and it
    /// follows what is known (its `prev_epoch` known, at most `MAX_GAP`
    /// past): one event of a stolen keeper key cannot move the numbers far.
    pub(crate) async fn note_epoch_heard(&self, community_id: &str, log: &OpLog, signed: &serde_json::Value) -> Result<()> {
        let scope = signed["tags"].as_array().and_then(|t| t.iter().find(|t| t[0] == "s")).and_then(|t| t[1].as_str()).map(str::to_string);
        let Some(scope) = scope else { return Ok(()) };
        let Ok(ev) = crate::epoch::read_epoch(signed, community_id, &scope) else { return Ok(()) };
        let state = log.state();
        let e = &ev.content.event;
        let in_range = state.keepers.iter().filter(|k| k.pubkey == ev.signer).any(|k| k.from_epoch <= e.epoch && state.range_end(k.serial).is_none_or(|end| e.epoch <= end));
        if !in_range {
            return Ok(());
        }
        let known = self.newest_epoch_known(community_id).await?;
        let follows = e.prev_epoch.is_none_or(|p| p <= known) && e.epoch <= known.saturating_add(crate::epoch::MAX_GAP).max(state.keeper().map_or(0, |k| k.from_epoch));
        if e.epoch > known && !follows {
            return Ok(());
        }
        db::keys::note_epoch_heard(&self.env.db, community_id, e.epoch).await
    }

    /// The newest epoch of the channel this device knows of: of the epoch
    /// events its member side took (`cm_epochs`), and of every epoch event
    /// of a keeper it heard, refused ones too (`cm_epochs_heard`).
    pub async fn newest_epoch_known(&self, community_id: &str) -> Result<u32> {
        let held = db::keys::newest_epoch(&self.env.db, community_id).await?;
        let heard = db::keys::epoch_heard(&self.env.db, community_id).await?;
        Ok(held.max(heard))
    }

    /// Asks the relays of a channel for what brings it to its head: its
    /// journal, its card, its epoch events and its snapshots, each a page
    /// of the catch-up (`ask_journal` for the journal and the card). The
    /// pool hands what it fetched to the channel's stream before this
    /// returns; the caller waits for that stream to be read.
    pub async fn ask_head(&self, community_id: &str) -> Result<()> {
        self.ask_journal(community_id).await?;
        self.ask_parts(community_id, &[crate::transport::PART_EPOCHS, crate::transport::PART_SNAPSHOTS]).await
    }

    /// `ask_head`; whether a relay answered every page asked.
    pub(crate) async fn ask_head_answered(&self, community_id: &str) -> Result<bool> {
        let journal = self.ask_journal_answered(community_id).await?;
        let parts = self.ask_parts_answered(community_id, &[crate::transport::PART_EPOCHS, crate::transport::PART_SNAPSHOTS]).await?;
        Ok(journal && parts)
    }

    /// What this device holds of a channel's head: a catch-up that changes
    /// none of it brought nothing new.
    async fn head_of(&self, keys: &Keys, community_id: &str) -> Result<(u64, Option<u32>, u32, Option<i64>)> {
        let log = self.log(community_id).await?;
        let snapshot = self.member(keys).latest_snapshot(community_id).await?.map(|s| s.at);
        Ok((log.as_ref().map_or(0, |l| l.state().version), log.as_ref().and_then(|l| l.state().keeper().map(|k| k.serial)), self.newest_epoch_known(community_id).await?, snapshot))
    }

    /// Brings a channel to its head from the relays: asked, the catch-up
    /// read to its end (`settle`), the pages it wanted asked, again until a
    /// round changes nothing here, at most `wait`. `Ok(true)` when that is
    /// known: every round answered by a relay and read to its end. A host
    /// that gives no way into its stream of events (`settle` none) is
    /// watched instead until what it holds stays the same for a while:
    /// `Ok(false)`, the head likely but not known. `EPOCHS_NOT_HERE` when
    /// the channel did not come to its head in `wait`.
    pub(crate) async fn bring_to_head(&self, keys: &Keys, community_id: &str, wait: std::time::Duration) -> Result<bool> {
        let started = std::time::Instant::now();
        let mut before: Option<(u64, Option<u32>, u32, Option<i64>)> = None;
        loop {
            let answered = self.ask_head_answered(community_id).await?;
            let left = wait.saturating_sub(started.elapsed());
            match self.settle(left).await {
                None => return self.watch_head(keys, community_id, started, wait).await.map(|()| false),
                Some(true) => {
                    // What the pages read wanted (the page before a full one).
                    let asked = self.run_asks().await;
                    let head = self.head_of(keys, community_id).await?;
                    if head.0 > 0 && asked == 0 && before.as_ref() == Some(&head) {
                        return Ok(answered);
                    }
                    before = Some(head);
                }
                Some(false) => {}
            }
            if started.elapsed() >= wait {
                return Err(invalid(EPOCHS_NOT_HERE));
            }
        }
    }

    /// The head watched until what this device holds of it stays the same
    /// for `HEAD_QUIET` (a host that gives no way into its stream).
    async fn watch_head(&self, keys: &Keys, community_id: &str, started: std::time::Instant, wait: std::time::Duration) -> Result<()> {
        let mut asked_at = std::time::Instant::now();
        let mut last = self.head_of(keys, community_id).await?;
        let mut same_since = std::time::Instant::now();
        loop {
            tokio::time::sleep(HEAD_POLL).await;
            let now = self.head_of(keys, community_id).await?;
            if now != last {
                last = now;
                same_since = std::time::Instant::now();
            }
            if last.0 > 0 && same_since.elapsed() >= HEAD_QUIET {
                return Ok(());
            }
            if started.elapsed() >= wait {
                return Err(invalid(EPOCHS_NOT_HERE));
            }
            // A page of the catch-up that brought the journal lets the next
            // ask bring what waited for it.
            if asked_at.elapsed() >= HEAD_QUIET * 2 {
                self.ask_head(community_id).await?;
                asked_at = std::time::Instant::now();
            }
        }
    }

    /// `keeper_take_with` of a channel brought to its head from the relays
    /// first (`bring_to_head`: the journal, the epochs, the snapshots) —
    /// the events are read by whoever runs the service, so no lock of the
    /// channel is held meanwhile. The app's «Сделать хранителем это
    /// устройство» and the bots share it: a series numbered from a partial
    /// view clashes with epochs the keeper before published.
    /// `EPOCHS_NOT_HERE` when the channel did not come to its head.
    ///
    /// `force` is the owner's word that the current series published
    /// nothing; it is also taken without his word when that is known: the
    /// head reached and read to its end with every page answered, and no
    /// epoch heard of the current series (a keeper lost before its first
    /// turn, the owner's phone that took it while the others were offline).
    /// The epochs come by the newest page whatever was heard before, so
    /// one the series published would be on it. Without it every other
    /// device of the owner was refused for good.
    pub async fn keeper_take_at_head(&self, keys: &Keys, community_id: &str, emergency: bool, force: bool, wait: std::time::Duration) -> Result<Outcome> {
        let me = Self::me(keys)?;
        self.expect_owner(community_id, &me).await?;
        let known = self.bring_to_head(keys, community_id, wait).await?;
        let lock = self.lock_of(community_id).await;
        let _g = lock.lock().await;
        let empty = known && self.series_published_nothing(community_id).await?;
        if empty && !force {
            tracing::info!(community = %community_id, "the current keeper series published no epoch: taken past it");
        }
        self.keeper_take_with(keys, community_id, emergency, force || empty).await
    }

    /// No epoch heard of the current keeper series.
    async fn series_published_nothing(&self, community_id: &str) -> Result<bool> {
        let Some(log) = self.log(community_id).await? else { return Ok(false) };
        let Some(from) = log.state().keeper().map(|k| k.from_epoch) else { return Ok(false) };
        Ok(self.newest_epoch_known(community_id).await? < from)
    }
}

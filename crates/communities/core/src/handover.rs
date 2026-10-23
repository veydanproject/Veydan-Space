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
    /// publish before it hears of the series (wire §15).
    pub(crate) async fn next_series_epoch(&self, community_id: &str, after: u32) -> Result<u32> {
        let newest = db::keys::newest_epoch(&self.env.db, community_id).await?;
        Ok(newest.saturating_add(2).max(after.saturating_add(1)))
    }

    // ─── Device B: taking the keeper over ───────────────────────────────────

    /// «Сделать хранителем это устройство» (spec §7.5): the owner on a
    /// device that does not keep the channel. `emergency`: the keeper
    /// before is gone (a lost phone, spec §7.6): no state is waited for, a
    /// new generation is started here at once; asked for again while a
    /// handover waits, it gives up the wait.
    pub async fn keeper_take(&self, keys: &Keys, community_id: &str, emergency: bool) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let state = log.state().clone();
        if state.owner != me || state.disbanded {
            return Err(invalid("not permitted"));
        }
        if self.keeper(community_id).await?.is_some() {
            return Err(invalid("this device keeps the channel"));
        }
        let held_root = secrets::root(self.env.secrets.as_ref(), community_id).await?.is_some();
        if let Some(take) = db::handover::take(&self.env.db, community_id).await? {
            if !emergency || take.mode == db::handover::MODE_EMERGENCY {
                return Ok(Outcome::default());
            }
            return self.take_emergency(keys, &log, take).await;
        }
        let last = state.keeper().ok_or_else(|| invalid("no keeper"))?;
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
        if emergency {
            let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
            out.merge(self.take_emergency(keys, &log, take).await?);
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id, "keeper": "taking" }));
        Ok(out)
    }

    /// The keeper before is gone: a new root (the lost device held the old
    /// one), an emergency generation per scope from the readers of the
    /// newest snapshot, a bundle to each of them.
    async fn take_emergency(&self, keys: &Keys, log: &OpLog, take: TakeRow) -> Result<Outcome> {
        let state = log.state();
        let cid = state.community_id.clone();
        let me = Self::me(keys)?;
        let kk = secrets::keeper_keys(self.env.secrets.as_ref(), &cid).await?.ok_or_else(|| MessengerError::SecretMissing(secrets::keeper_name(&cid)))?;
        let mut readers = self.member(keys).all_readers(log).await?.ok_or_else(|| invalid("the journal of the channel is not here yet"))?;
        let mine: [u8; 32] = hex::decode(me.as_hex()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("my key"))?;
        if !readers.iter().any(|r| r.pubkey == mine) {
            readers.push(crate::snapshot::Entry { pubkey: mine, flags: crate::snapshot::FLAG_READER, levels: 1, since: u32::try_from(self.now()).unwrap_or(0) });
        }
        let mut gens = BTreeMap::new();
        for scope_id in state.scopes.keys() {
            gens.insert(scope_id.clone(), db::keys::newest_gen(&self.env.db, &cid, scope_id).await?);
        }
        let serial = u32::try_from(take.serial).map_err(|_| invalid("keeper"))?;
        let from_epoch = u32::try_from(take.from_epoch).map_err(|_| invalid("keeper"))?;
        let root = RootSecret::generate();
        let now = self.now();
        let recovery = crate::keeper::Recovery { root: &root, readers: &readers, gens: &gens, from_epoch };
        Keeper::write_emergency(&self.env, state, &kk, &recovery, now).await?;
        secrets::put_root_of(self.env.secrets.as_ref(), &cid, &root, serial).await?;
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

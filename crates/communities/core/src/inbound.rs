// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/inbound.rs at 1f2ef31d, then changed for channels.

//! What arrives: the events of the channels (journal 1391, epochs 1392,
//! content 1390, snapshots 1393, card 30390) and the
//! notes (NIP-59) to me or to a keeper kept here, with the checks of
//! internal/communities-wire.md §11.
//!
//! Relays deliver in any order and more than once. Everything here can
//! be replayed: an operation whose parents are not here waits in the
//! journal, sealed content whose key is not here waits in `cm_pending`
//! (7 days) and is read again when the key comes, an epoch whose bundle
//! is not here waits in `cm_epochs` until it comes.

use crate::db::{self, channels::PendingRow, content::MessageRow};
use crate::keeper::{Joined, Keeper};
use crate::log::OpLog;
use crate::marks::{self, ModNote, REVIEW_OK, REVIEW_PENDING};
use crate::member::readers::{Standing::Known, Standing::Unfetched, Standing::Wait, SNAPSHOT_MARKER};
use crate::member::{OwnKeys, T_OWN_KEYS};
use messenger_core::outbound::Filter;
use messenger_core::{Outbound, Scope};
use crate::notes::{self, Opened, KIND_OWN};
use crate::op::OpBody;
use crate::service::{
    short, ChannelService, Outcome, MEMBERSHIP_JOINED, MEMBERSHIP_JOINING, MEMBERSHIP_REJECTED, UI_EVENT_CHANNEL_INVITE, UI_EVENT_CHANNEL_MESSAGE, UI_EVENT_CHANNEL_TYPING,
    UI_EVENT_CHANNEL_REQUEST, UI_EVENT_CHANNEL_UPDATED,
};
use crate::state::{rules_at, Content, Reader, SCOPE_EVERYONE};
use crate::wire::{self, Admit, Invite, InviteReply, Issue, Join, KeeperMaster, Leave, Missed, Rebundle, Rejected, Request};
use crate::{link::LinkSecret, roles::Role};
use messenger_core::envelope::{T_DELETE, T_EDIT, T_MEDIA, T_REACTION, T_TEXT, T_TYPING};
use messenger_core::{Envelope, MessengerError, PubKey, Result};
use nostr::key::Keys;
use nostr::prelude::Event;

fn tag(signed: &serde_json::Value, name: &str) -> Option<String> {
    signed["tags"].as_array()?.iter().find(|t| t[0] == name).and_then(|t| t[1].as_str()).map(str::to_string)
}

/// One short line of what became of an epoch event on the member's side
/// (`epoch 3 of 1a2b3c4d accepted`), never the event itself; none for one
/// taken already.
fn epoch_line(v: &crate::epoch::Verdict, signed: &serde_json::Value, cid: &str) -> Option<String> {
    use crate::epoch::Verdict;
    let n = match v {
        Verdict::Accepted { event, .. } => event.epoch.to_string(),
        _ => signed["content"]
            .as_str()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
            .and_then(|c| c["event"]["epoch"].as_u64())
            .map_or_else(|| "?".to_string(), |e| e.to_string()),
    };
    let what = match v {
        Verdict::Duplicate => return None,
        Verdict::Accepted { emergency: true, .. } => "accepted, an emergency generation".to_string(),
        Verdict::Accepted { .. } => "accepted".to_string(),
        Verdict::Conflict => "another event for an epoch taken: its bundle is asked for again".to_string(),
        Verdict::Ahead => "ahead of the epochs here: kept aside".to_string(),
        Verdict::Refused(why) => format!("refused, {why}"),
    };
    Some(format!("epoch {n} of {}: {what}", short(cid)))
}

/// The notes to a keeper (wire §7): heard on the inbox its journal names.
const KEEPER_NOTES: [&str; 7] = [wire::T_JOIN, wire::T_ADMIT, wire::T_REBUNDLE, wire::T_MISSED, wire::T_LEAVE, wire::T_KEEPER_MASTER, wire::T_TICKET_REQUEST];
/// The most notes that wait in the queue of a keeper (`cm_keeper_inbox`):
/// a week of joins of a busy channel; past it a joiner sends again.
pub const MAX_QUEUED_NOTES: i64 = 20_000;
/// How many notes of the queue are read at once.
const QUEUE_BATCH: i64 = 200;

/// The mark a secret of a link waits under for its `rotate_link`.
fn link_marker(link_epoch: u32) -> String {
    format!("link:{link_epoch}")
}

/// The envelope of a file (`t: media`) with a new caption written in
/// `format` (an empty caption takes it away, plain text drops `fmt`); none
/// for something that is not the envelope of a file.
fn media_with_caption(json: &str, caption: &str, format: messenger_core::TextFormat) -> Option<String> {
    let mut env = Envelope::parse(json).ok().filter(|e| e.t == T_MEDIA)?;
    env.fields.remove(messenger_core::envelope::FIELD_FMT);
    if caption.trim().is_empty() {
        env.fields.remove("caption");
        return Some(env.encode());
    }
    env.fields.insert("caption".into(), serde_json::Value::String(caption.to_string()));
    Some(env.with_format(format).encode())
}

/// The channel a note is about, if it is one of the channels' (`channel.*`,
/// `keeper.master`, `own.channel_keys`): the door of the messenger's
/// direct messages hands it here.
pub fn channel_of_note(content: &str) -> Option<String> {
    let e = Envelope::parse(content).ok()?;
    let ours = e.t.starts_with("channel.") || e.t == wire::T_KEEPER_MASTER || e.t == wire::T_KEEPER_HANDOVER || e.t == T_OWN_KEYS;
    ours.then(|| e.str_field("community_id").map(str::to_string)).flatten()
}

impl ChannelService {
    // ─── Events of the relays ───────────────────────────────────────────────

    /// An event of a channel. `live`: it came now (not from a sync of the
    /// past): old epochs are refused for live content only (wire §4).
    pub async fn on_event(&self, keys: &Keys, signed: &serde_json::Value, live: bool) -> Result<Outcome> {
        // A marker of `settle`: the events before it were read.
        if self.settle_heard(signed) {
            return Ok(Outcome::default());
        }
        // How far each kind was heard, for the catch-up after a start; a
        // forged event moves nothing but where the next one asks from. An
        // epoch or a snapshot of a channel whose journal is not here yet is
        // dropped (it cannot be checked): it moves no cursor either, so that
        // the next round asks for it again (a device restored on a fresh
        // folder heard the epochs before the journal and never again).
        let kind = signed["kind"].as_u64().and_then(|k| u16::try_from(k).ok());
        let before_journal = match kind {
            Some(wire::KIND_EPOCH) | Some(wire::KIND_SNAPSHOT) => match tag(signed, "h") {
                Some(cid) => self.log(&cid).await?.is_none(),
                None => false,
            },
            _ => false,
        };
        if !before_journal {
            self.heard(signed, live).await?;
        }
        let out = match signed["kind"].as_u64().and_then(|k| u16::try_from(k).ok()) {
            Some(wire::KIND_JOURNAL) => self.on_journal(keys, signed).await?,
            Some(wire::KIND_EPOCH) => self.on_epoch_event(keys, signed).await?,
            Some(wire::KIND_CONTENT) => self.on_content(keys, signed, live).await?,
            Some(wire::KIND_TYPING) => self.on_typing(keys, signed).await?,
            Some(wire::KIND_CARD) => self.on_card(signed).await?,
            Some(wire::KIND_SNAPSHOT) => self.on_snapshot(keys, signed).await?,
            Some(wire::KIND_HEARTBEAT) => self.on_heartbeat(signed).await?,
            Some(notes::KIND_WRAP) => self.on_wrap_heard(keys, signed, live).await?.unwrap_or_default(),
            _ => Outcome::default(),
        };
        Ok(out)
    }

    /// A «typing» under a post (`wire::KIND_TYPING`): opened like content,
    /// under the key of its section; a word for the screen alone, kept
    /// nowhere and never parked (one without its key here is nothing, as
    /// is my own, a banned author's, or one under no post).
    async fn on_typing(&self, keys: &Keys, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Ok(event) = serde_json::from_value::<Event>(signed.clone()) else { return Ok(out) };
        let Ok(outer) = wire::outer(&event) else { return Ok(out) };
        let cid = outer.place.community_id.clone();
        let (scope_id, channel_id) = (outer.place.scope_id.clone(), outer.place.channel_id.clone());
        let Some(log) = self.log(&cid).await? else { return Ok(out) };
        let Some(key) = self.member(keys).key_for(&cid, &scope_id, outer.epoch, &outer.key_id.0).await? else { return Ok(out) };
        let scope = wire::scope_ref(&cid, &scope_id)?;
        let Ok(payload) = wire::open(&wire::stream_key(&key, &scope, &channel_id), &event.content) else { return Ok(out) };
        if payload.t != wire::T_MSG {
            return Ok(out);
        }
        let Ok(m) = wire::verify_inner(&payload.event, &outer) else { return Ok(out) };
        let Ok(env) = Envelope::parse(&m.content) else { return Ok(out) };
        if env.t != T_TYPING || m.author == Self::me(keys)? || log.state().is_banned(&m.author) {
            return Ok(out);
        }
        let Some(post_id) = m.links.thread.clone() else { return Ok(out) };
        out.event(
            UI_EVENT_CHANNEL_TYPING,
            serde_json::json!({ "community_id": cid, "post_id": post_id, "author": m.author.as_hex(), "until": m.created_at + env.typing_ttl() }),
        );
        Ok(out)
    }

    /// A comment parked for shard `index` of the newest snapshot, which is
    /// not in the cache here: a job fetches it off this event
    /// (`fetch::Want::Shard`, `run_fetch`), and reads the comments that
    /// wait for it again once it opens.
    async fn shard_missed(&self, community_id: &str, index: usize) -> Result<()> {
        if let Some((at, _)) = db::snapshots::latest(&self.env.db, community_id).await? {
            self.env.fetches.want(community_id, crate::member::fetch::Want::Shard { at, index });
        }
        Ok(())
    }

    /// An operation of the journal: kept, applied in the order of
    /// `OpLog`, and handed to the keeper when it is here.
    async fn on_journal(&self, keys: &Keys, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(cid) = tag(signed, "h") else { return Ok(out) };
        self.mark(Some(format!("channel {}: an operation of the journal", short(&cid))));
        let op = match wire::verify_op(signed, &cid) {
            Ok(op) => op,
            Err(e) if wire::is_op_too_new(&e) => {
                db::channels::set_needs_update(&self.env.db, &cid, true).await?;
                out.note("an operation newer than this build");
                return Ok(out);
            }
            Err(e) => {
                out.note(format!("operation refused: {e}"));
                return Ok(out);
            }
        };
        // An operation dated past my clock would move the head of the
        // journal there for good: timed mutes would count as lifted and
        // every later operation be dated after it. Refused at receipt; a
        // later catch-up takes it once its date has come.
        if op.created_at > self.now() + wire::SKEW_SECS {
            out.note("an operation dated ahead");
            return Ok(out);
        }
        if !self.store_op(&cid, &op, signed).await? {
            return Ok(out);
        }
        let Some(log) = self.log(&cid).await? else {
            out.note("an operation before the journal's create");
            return Ok(out);
        };
        // A secret of the link that came before its `rotate_link`.
        let waiting = db::channels::take_pending(&self.env.db, &cid, &link_marker(log.state().link_epoch)).await?;
        self.read_pending(keys, waiting, &mut out).await?;
        let me = Self::me(keys)?;
        let id = op.id();
        let applied = log.contains(&id) && log.rejection(&id).is_none();
        let membership = match &op.body {
            OpBody::Ban { who, .. } | OpBody::Remove { who } if applied && who == &me => Some(crate::service::MEMBERSHIP_LEFT),
            _ => None,
        };
        self.refresh_row(log.state(), &me, membership).await?;
        let why = match &op.body {
            OpBody::Ban { who, .. } if applied && who == &me => Some(crate::service::OUT_BANNED),
            OpBody::Remove { who } if applied && who == &me => Some(crate::service::OUT_REMOVED),
            _ => None,
        };
        if why.is_some() {
            db::channels::set_out_reason(&self.env.db, &cid, why).await?;
        }
        // Invitations that came before the journal: borne out by it, or dropped.
        let state = log.state();
        db::channels::verify_invites(&self.env.db, &cid, |peer| PubKey::parse(peer).is_some_and(|p| state.role_of(&p).is_some_and(Role::is_moderator))).await?;
        // The force of the marks follows the journal (a demoted moderator's
        // hides lapse, a lifted ban shows again).
        self.refresh_marks(&log).await?;
        // A new keeper series names where the notes to the keeper go: a join
        // of mine that waits goes there at once, not hours later at its
        // next time (живой запуск 2026-10-23: a join sent to the inbox of a
        // journal before `inbox` reached nobody).
        if applied && matches!(op.body, OpBody::Keeper { .. }) && db::channels::joining_due(&self.env.db, &cid, self.now()).await? {
            self.say(&format!("{} for {}: the keeper is heard at another inbox now, sent there at once", wire::T_JOIN, crate::service::short(&cid)));
            out.wake = Some(u64::try_from(self.now()).unwrap_or(0));
        }
        // A later keeper series: the keeper kept here hands its state over
        // (wire §15); a failure is tried again on the next turn.
        if self.superseded(&cid, &log).await? {
            self.mark(Some(format!("channel {}: the keeper handed over to the next series (its state to the hosts, its note)", short(&cid))));
            if let Err(e) = self.hand_over(&cid).await {
                tracing::warn!(community = %cid, error = %e, "the keeper was not handed over yet");
                out.wake = u64::try_from(self.now()).ok().map(|t| t + crate::keeper::RETRY_SECS);
            }
        }
        if let Some(k) = self.keeper(&cid).await? {
            self.mark(Some(format!("channel {}: an operation of the journal waits for the keeper", short(&cid))));
            let mut k = k.lock().await;
            self.mark(Some(format!("channel {}: the keeper told of the operations", short(&cid))));
            // Every operation the journal holds now, not only this one: an
            // operation that waited for its parents is applied with them.
            for o in log.ordered() {
                k.on_op(&log, o, self.now()).await?;
            }
            // Its turn (an epoch for a ban, the sections put in step) is
            // the loop's, right after this event: run here it held the
            // loop for its hosts' round trips (живой запуск 2026-10-23,
            // the section's epoch heard seven minutes after `scope_add`).
            out.wake = Some(u64::try_from(self.now()).unwrap_or(0));
            self.ask_turn();
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid }));
        Ok(out)
    }

    /// A snapshot of the readers (wire §8): from the keeper of the current
    /// series, newer than the one here. Kept with the shards it still
    /// names; heads of the journal it names that are not here are asked
    /// for; messages that waited for a snapshot are read again.
    pub(crate) async fn on_snapshot(&self, keys: &Keys, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(cid) = tag(signed, "h") else { return Ok(out) };
        self.mark(Some(format!("channel {}: a snapshot", short(&cid))));
        let Some(log) = self.log(&cid).await? else {
            out.note("a snapshot before the journal");
            return Ok(out);
        };
        let known = db::snapshots::latest(&self.env.db, &cid).await?.map(|(at, _)| at);
        // The one held, or one before it, heard again by the next round of
        // listening (the relays hand every snapshot): nothing to take and
        // nothing to say. Nothing is taken without the checks below.
        let at = signed["content"].as_str().and_then(|c| serde_json::from_str::<crate::snapshot::Snapshot>(c).ok()).map(|s| s.at);
        if known.is_some_and(|k| at.is_some_and(|at| at <= k)) {
            return Ok(out);
        }
        let s = match crate::snapshot::verify_snapshot(signed, log.state(), known, self.now()) {
            Ok(s) => s,
            Err(e) => {
                out.note(format!("snapshot refused: {e}"));
                return Ok(out);
            }
        };
        db::snapshots::put_snapshot(&self.env.db, &cid, s.at, &serde_json::to_string(signed)?).await?;
        let froms: Vec<String> = s.shards.iter().map(|r| r.from.clone()).collect();
        db::snapshots::retain_shards(&self.env.db, &cid, &froms).await?;
        if let Some(mut row) = db::channels::get(&self.env.db, &cid).await? {
            row.members = i64::try_from(s.members).unwrap_or(i64::MAX);
            db::channels::upsert(&self.env.db, &row).await?;
        }
        // The snapshot never overrides the journal: heads it names that
        // are not here are fetched (wire §3), off the way of the event
        // (`ask_later`): the pool awaits a `Sync` to its end, 20 s a relay.
        let missing: Vec<String> = s.heads.iter().filter(|h| !log.contains(h)).map(|h| h.0.clone()).collect();
        if !missing.is_empty() {
            self.ask_later(Outbound::Sync { scope: Scope::Channel { id: cid.clone() }, filter: Filter(serde_json::json!({ "kinds": [wire::KIND_JOURNAL], "ids": missing })), local: vec![] });
        }
        self.mark(Some(format!("channel {}: what waited for a snapshot read again", short(&cid))));
        let waiting = db::channels::take_pending(&self.env.db, &cid, SNAPSHOT_MARKER).await?;
        self.read_pending(keys, waiting, &mut out).await?;
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "members": s.members }));
        Ok(out)
    }

    /// The keeper's sign of life (wire §18): from the keeper of the current
    /// series, newer than the one held. «Был в сети» of every member, and
    /// the rule of the tickets (§17), read it with the epochs. The one held,
    /// or an older one heard again by the next round, is silent.
    async fn on_heartbeat(&self, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(cid) = tag(signed, "d") else { return Ok(out) };
        let Some(log) = self.log(&cid).await? else { return Ok(out) };
        let known = db::channels::keeper_seen(&self.env.db, &cid).await?;
        let at = signed["created_at"].as_i64();
        if known.is_some_and(|k| at.is_some_and(|at| at <= k)) {
            return Ok(out);
        }
        match crate::heartbeat::verify_heartbeat(signed, log.state(), known, self.now()) {
            Ok(hb) => {
                db::channels::put_keeper_seen(&self.env.db, &cid, hb.at, hb.serial).await?;
                out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid }));
            }
            Err(e) => out.note(format!("heartbeat refused: {e}")),
        }
        Ok(out)
    }

    /// An epoch event, on the member's side.
    async fn on_epoch_event(&self, keys: &Keys, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(cid) = tag(signed, "h") else { return Ok(out) };
        self.mark(Some(format!("channel {}: an epoch event", short(&cid))));
        let Some(log) = self.log(&cid).await? else {
            out.note("an epoch before the journal");
            return Ok(out);
        };
        self.note_epoch_heard(&cid, &log, signed).await?;
        let m = self.member(keys).on_epoch(&log, signed, self.now()).await?;
        if !m.keys.is_empty() {
            self.mark(Some(format!("channel {}: the secrets of links owed, by the new key", short(&cid))));
            self.publish_owed_links(keys).await?;
            self.mark(Some(format!("channel {}: what I wrote sealed again under the new key", short(&cid))));
            if let Err(e) = self.reseal(keys, &log).await {
                tracing::warn!(community = %cid, error = %e, "what I wrote under the epoch before was not sealed again");
            }
        }
        if m.out {
            out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "membership": crate::service::MEMBERSHIP_LEFT }));
        }
        // A section closed to me: the channel stays mine (wire §14.5).
        for scope_id in &m.closed {
            out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "closed_scope": scope_id }));
        }
        // An epoch taken already, heard again by the next round: silent.
        if let Some(v) = m.verdict.as_ref() {
            if let Some(line) = epoch_line(v, signed, &cid) {
                out.note(line);
            }
        }
        self.mark(Some(format!("channel {}: what waited for the epoch read again", short(&cid))));
        self.read_pending(keys, m.pending, &mut out).await?;
        Ok(out)
    }

    /// What I wrote under an older epoch after a newer one began (my device
    /// had not heard it yet: the trial of 1.6, run h), sealed again under
    /// the newest key held, the same inner event (its id made again from
    /// what it holds): under `from_join` a reader let in just before that
    /// epoch holds its key and none before it. Those who hold both keep
    /// it once (the inner id).
    pub(crate) async fn reseal(&self, keys: &Keys, log: &OpLog) -> Result<()> {
        // Each scope of `from_join` on its own (wire §14.3): the keys of one
        // say nothing of another.
        let scopes: Vec<String> =
            log.state().scopes.iter().filter(|(_, d)| d.history == crate::op::History::FromJoin).map(|(s, _)| s.clone()).collect();
        for scope_id in scopes {
            self.reseal_scope(keys, log, &scope_id).await?;
        }
        Ok(())
    }

    async fn reseal_scope(&self, keys: &Keys, log: &OpLog, scope_id: &str) -> Result<()> {
        let state = log.state();
        let cid = &state.community_id;
        let Some(key) = self.member(keys).newest_key(cid, scope_id).await? else { return Ok(()) };
        let rows = db::keys::epochs(&self.env.db, cid, scope_id).await?;
        let Some(began) = rows.iter().find(|r| r.epoch == i64::from(key.epoch)).map(|r| r.at) else { return Ok(()) };
        let me = Self::me(keys)?;
        let place = wire::Place::feed_in(cid, scope_id);
        for row in db::content::written_behind(&self.env.db, cid, scope_id, me.as_hex(), i64::from(key.epoch), began - wire::SKEW_SECS).await? {
            let links = if row.post_id.is_empty() { wire::Links::default() } else { wire::Links { thread: Some(row.post_id.clone()), parent: row.parent_id.clone(), target: None } };
            let inner = wire::sign_inner(keys, &place, &links, &row.envelope_json, row.created_at)?;
            if inner.id.to_hex() != row.id {
                continue;
            }
            let payload = wire::Payload::event(wire::T_MSG, &inner)?;
            let outer = wire::seal_content(&place, &key, links.outer_target(), &payload, row.created_at)?;
            self.enqueue(cid, db::outbox::LANE_CHANNEL, wire::KIND_CONTENT, &outer, Some(&row.id)).await?;
            db::content::set_epoch(&self.env.db, &row.id, i64::from(key.epoch)).await?;
        }
        Ok(())
    }

    /// Sealed events that waited for a key that came: read again, as
    /// history (not live).
    pub(crate) async fn read_pending(&self, keys: &Keys, rows: Vec<PendingRow>, out: &mut Outcome) -> Result<()> {
        for row in rows {
            let value: serde_json::Value = serde_json::from_str(&row.event_json)?;
            out.merge(self.on_content(keys, &value, false).await?);
        }
        Ok(())
    }

    /// Sealed content (kind 1390): the outer tags, the key named by `k`
    /// (or a wait in `cm_pending`), the payload, the inner event and the
    /// rules of the journal at its time.
    async fn on_content(&self, keys: &Keys, signed: &serde_json::Value, live: bool) -> Result<Outcome> {
        let mut out = Outcome::default();
        let event: Event = serde_json::from_value(signed.clone()).map_err(|_| MessengerError::Invalid("content event".into()))?;
        let outer = match wire::outer(&event) {
            Ok(o) => o,
            Err(e) => {
                out.note(format!("content refused: {e}"));
                return Ok(out);
            }
        };
        let cid = outer.place.community_id.clone();
        let (scope_id, channel_id) = (outer.place.scope_id.clone(), outer.place.channel_id.clone());
        self.mark(Some(format!("channel {}: a sealed event", short(&cid))));
        if channel_id != wire::CHANNEL_FEED {
            out.note("content of a stream of a later stage");
            return Ok(out);
        }
        let member = self.member(keys);
        let Some(key) = member.key_for(&cid, &scope_id, outer.epoch, &outer.key_id.0).await? else {
            // Content of a section the journal gives me not (or a scope it
            // does not know) waits for no key: it would only crowd out what
            // does (wire §14.5). A journal not here yet says nothing.
            if let Some(log) = self.log(&cid).await? {
                let me = Self::me(keys)?;
                if scope_id != SCOPE_EVERYONE && !log.state().in_scope(&me, &scope_id) {
                    out.note("content of a section not mine: dropped");
                    return Ok(out);
                }
            }
            if !self.may_park(&cid, &scope_id, outer.epoch).await? {
                out.note("content for no channel followed here, or past the epochs, or too much waiting: dropped");
                return Ok(out);
            }
            let row = PendingRow {
                event_id: event.id.to_hex(),
                community_id: cid.clone(),
                scope_id: scope_id.clone(),
                epoch: i64::from(outer.epoch),
                key_id: outer.key_id.0.clone(),
                event_json: serde_json::to_string(signed)?,
                created_at: outer.created_at,
                received_at: self.now(),
            };
            db::channels::add_pending(&self.env.db, &row).await?;
            out.note("content waits for its key");
            return Ok(out);
        };
        if live && !member.live_ok(&cid, &scope_id, outer.epoch).await? {
            out.note("live content under an old epoch");
            return Ok(out);
        }
        let Some(log) = self.log(&cid).await? else { return Ok(out) };
        let scope = wire::scope_ref(&cid, &scope_id)?;
        let payload = match wire::open(&wire::stream_key(&key, &scope, &channel_id), &event.content) {
            Ok(p) => p,
            Err(e) => {
                out.note(format!("content does not open: {e}"));
                return Ok(out);
            }
        };
        match payload.t.as_str() {
            wire::T_LINK => match wire::verify_link(&payload.event, &outer) {
                Ok((author, link)) => match log.link_author(link.link_epoch) {
                    // Only the author of the operation that made the link
                    // epoch hands its secret on: any reader holds the key
                    // of the epoch and could put a secret of their own
                    // first (wire §4).
                    Some(by) if by == &author => {
                        crate::secrets::put_link_trusted(self.env.secrets.as_ref(), &cid, link.link_epoch, &LinkSecret::decode(&link.secret)?).await?;
                        // By its rows, not its lock: content is read again
                        // while the keeper is held (`hand_to_me`).
                        if self.keeper(&cid).await?.is_some() {
                            crate::keeper::take_link(&self.env.db, self.env.secrets.as_ref(), &cid, &link).await?;
                        }
                    }
                    Some(_) => out.note("a link secret not from the author of its link epoch"),
                    // The `rotate_link` has not come yet: the secret waits
                    // for it, as content waits for its key.
                    None => self.park(signed, &outer, &event, &link_marker(link.link_epoch), &mut out).await?,
                },
                Err(e) => out.note(format!("link refused: {e}")),
            },
            wire::T_MSG => match wire::verify_inner(&payload.event, &outer) {
                // Readers are not in the journal: the snapshot says who
                // they are (wire §8); one it cannot tell yet waits for the
                // next snapshot.
                Ok(m) => match member.reader_of(&log, &m.author, m.created_at).await? {
                    Known(reader) => self.on_message(&log, m, reader, outer.epoch, &scope_id, &event.id.to_hex(), &mut out).await?,
                    Wait => self.park(signed, &outer, &event, SNAPSHOT_MARKER, &mut out).await?,
                    // The shard is not here: fetched off this event, the
                    // comment read again once it opens (not only at the
                    // next snapshot).
                    Unfetched(index) => {
                        self.park(signed, &outer, &event, SNAPSHOT_MARKER, &mut out).await?;
                        self.shard_missed(&cid, index).await?;
                    }
                },
                Err(e) => out.note(format!("inner refused: {e}")),
            },
            wire::T_MOD => match wire::verify_inner(&payload.event, &outer) {
                Ok(m) => self.on_mod(&log, m, &mut out).await?,
                Err(e) => out.note(format!("inner refused: {e}")),
            },
            // A name of a level or a title of a scope: kept by the operation
            // it names; whether that operation is its signer's is asked
            // whenever the names are read (`ChannelService::names`), so it
            // may come before the operation.
            wire::T_META => match wire::verify_meta(&payload.event, &outer) {
                Ok((author, meta, at)) => {
                    if self.keep_meta(&cid, &author, &meta, at).await? {
                        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid }));
                    }
                }
                Err(e) => out.note(format!("meta refused: {e}")),
            },
            _ => out.note("snapshot shard: sub-stage 1.3"),
        }
        Ok(out)
    }

    /// Of a message by an author who is not in now (`out_now`), banned or
    /// removed by the journal, written before the journal put them out
    /// last: the reader its author was then, by the key of its epoch (wire
    /// §4, 2026-10-22 for a ban, 2026-10-30 for a removal). Bound to that
    /// last time put out, not to the message's own date: sealed under the
    /// epoch current then or the one before it (one put out holding older
    /// keys writes back no further than the last two epochs before), dated
    /// before it, not before its epoch began, and — after an earlier time
    /// put out — under an epoch that began after it (the time out is no
    /// time in). A key that no snapshot names and no operation put out is
    /// still judged at the head: holding the key of an epoch is no proof of
    /// having been in. Judged at the head instead, a device that came after
    /// the removal (a new device, a folder restored empty) dropped every
    /// old message of a removed author that the devices there at the time
    /// keep.
    async fn before_out(&self, log: &OpLog, author: &PubKey, out_now: bool, written: i64, epoch: u32, scope_id: &str) -> Result<Option<Reader>> {
        if !out_now {
            return Ok(None);
        }
        let state = log.state();
        let applied = || log.ordered().filter(|o| log.rejection(&o.id()).is_none());
        let put_out = |o: &&crate::op::Op| matches!(&o.body, OpBody::Ban { who, .. } | OpBody::Remove { who } if who == author);
        let outs: Vec<i64> = applied().filter(put_out).map(|o| o.created_at).collect();
        let Some(last_out) = outs.iter().copied().max() else { return Ok(None) };
        if written >= last_out {
            return Ok(None);
        }
        let rows = db::keys::epochs(&self.env.db, &state.community_id, scope_id).await?;
        let Some(own) = rows.iter().find(|r| r.epoch == i64::from(epoch)) else { return Ok(None) };
        if written < own.at - wire::SKEW_SECS {
            return Ok(None);
        }
        let current_at = |t: i64| rows.iter().filter(|r| r.at <= t).map(|r| r.epoch).max().unwrap_or(0);
        let e = i64::from(epoch);
        if e < current_at(written) - 1 || e < current_at(last_out) - 1 {
            return Ok(None);
        }
        // Put out before too: in again only under an epoch begun since.
        if let Some(earlier) = outs.iter().copied().filter(|t| *t <= written).max() {
            if own.at <= earlier {
                return Ok(None);
            }
        }
        Ok(Some(Reader { since: own.at.min(written) }))
    }

    /// A message: who is in, banned and muted by the journal at max(its
    /// date, the head here) (`rules_at`), the rules of the channel at its
    /// date, never past my clock (`Rules::closed_written`); `reader`: what
    /// the snapshot says of its author (none for staff and an outsider).
    #[allow(clippy::too_many_arguments)]
    async fn on_message(&self, log: &OpLog, m: wire::InnerMessage, reader: Option<Reader>, epoch: u32, scope_id: &str, outer_id: &str, out: &mut Outcome) -> Result<()> {
        let state = log.state();
        let cid = state.community_id.clone();
        let Ok(env) = Envelope::parse(&m.content) else { return Ok(()) };
        let written = m.created_at.min(self.now());
        // Written before its author was put out (wire §4: a ban 2026-10-22,
        // a removal 2026-10-30): judged at its date on every device, the
        // one that heard it then and the one that came after alike; that
        // its author was in shows the key of its epoch, one of the last two
        // before they were put out (the trial of 1.6, problem 8; the review
        // of 2026-10-30). What the head says of them stays what a
        // premoderation, an edit and a deletion are judged by.
        let head = m.created_at.max(Self::head_at(log, self.now()));
        let snapshot_reader = reader;
        let out_now = rules_at(log, &m.author, snapshot_reader.as_ref(), head).standing == crate::state::Standing::Outsider;
        let before_out = self.before_out(log, &m.author, out_now, written, epoch, scope_id).await?;
        let at = if before_out.is_some() { written } else { head };
        let reader = before_out.or(reader);
        let rules = rules_at(log, &m.author, reader.as_ref(), at);
        let then = rules_at(log, &m.author, reader.as_ref(), written);
        let kind = if m.is_post() { Content::Post } else { Content::Comment };
        let author = m.author.as_hex().to_string();
        // The message an edit, a deletion or a reaction changes: told with
        // the event, since the event's own id names no row.
        let target = matches!(env.t.as_str(), T_REACTION | T_EDIT | T_DELETE).then(|| env.str_field("target").map(str::to_string)).flatten();
        match env.t.as_str() {
            T_TEXT | T_MEDIA => {
                if let Some(closed) = rules.closed_written(&then, kind, written) {
                    out.note(format!("message refused: {closed:?}"));
                    return Ok(());
                }
                // A reader's post stands only in a scope a posting level
                // gives them (2026-10-23): holding the key of a section is
                // reading it, not writing in it.
                if kind == Content::Post && !rules.post_scope_ok(&then, scope_id) {
                    out.note("a post in a scope its levels do not let post in");
                    return Ok(());
                }
                // A comment stands in the scope of its post (wire §14.3): sealed
                // elsewhere it would show a section's thread to those outside it.
                if let Some(post) = m.links.thread.as_deref() {
                    if db::content::get(&self.env.db, post).await?.is_some_and(|p| p.scope_id != scope_id) {
                        out.note("a comment out of the scope of its post");
                        return Ok(());
                    }
                }
                let row = MessageRow {
                    id: m.id.as_hex().to_string(),
                    community_id: cid.clone(),
                    post_id: m.links.thread.clone().unwrap_or_default(),
                    parent_id: m.links.parent.clone(),
                    author,
                    created_at: m.created_at,
                    epoch: i64::from(epoch),
                    envelope_json: m.content.clone(),
                    text: if env.t == T_MEDIA { env.str_field("caption") } else { env.as_text() }.map(str::to_string),
                    media_json: (env.t == T_MEDIA).then(|| m.content.clone()),
                    version: m.id.as_hex().to_string(),
                    review: if rules.subject_to_premod() || (before_out.is_some() && premod_now(log, &m.author, before_out.as_ref(), head)) { REVIEW_PENDING } else { REVIEW_OK }.into(),
                    mod_state: db::content::MOD_SHOWN.into(),
                    deleted: false,
                    received_at: self.now(),
                    scope_id: scope_id.to_string(),
                    outer_id: Some(outer_id.to_string()),
                    edited_at: 0,
                };
                // A message heard again (every round of listening, every page
                // asked of the relays) changes nothing and says nothing.
                if !db::content::insert(&self.env.db, &row).await? {
                    return Ok(());
                }
            }
            T_REACTION => {
                if rules.closed_written(&then, Content::Comment, written).is_some() {
                    return Ok(());
                }
                let (Some(target), Some(emoji)) = (env.str_field("target"), env.str_field("emoji")) else { return Ok(()) };
                if m.links.target.as_deref() != Some(target) {
                    return Ok(());
                }
                let set = env.fields.get("set").and_then(serde_json::Value::as_bool).unwrap_or(true);
                if !db::content::set_reaction(&self.env.db, &cid, target, &author, emoji, set, m.created_at).await? {
                    return Ok(());
                }
            }
            T_EDIT => {
                let (Some(target), Some(text)) = (env.str_field("target"), env.str_field("text")) else { return Ok(()) };
                let Some(row) = db::content::get(&self.env.db, target).await? else { return Ok(()) };
                // One who is out now edits nothing, whatever the date says.
                if out_now || row.author != author || row.scope_id != scope_id || m.links.target.as_deref() != Some(target) {
                    return Ok(());
                }
                // An edit of a file is a new caption (wire §4, 2026-10-23):
                // its attachments never change; the caption goes into its
                // envelope too, with the format of the edit, and an empty
                // one takes the caption away.
                let media = match row.media_json.as_deref() {
                    Some(json) => match media_with_caption(json, text, env.format()) {
                        Some(m) => Some(m),
                        None => return Ok(()),
                    },
                    None => None,
                };
                let text = if media.is_some() { Some(text).filter(|t| !t.trim().is_empty()) } else { Some(text) };
                if !db::content::edit(&self.env.db, target, &m.content, text, media.as_deref(), m.id.as_hex(), m.created_at).await? {
                    return Ok(());
                }
            }
            T_DELETE => {
                let Some(target) = env.str_field("target") else { return Ok(()) };
                let Some(row) = db::content::get(&self.env.db, target).await? else { return Ok(()) };
                let Some(target_author) = PubKey::parse(&row.author) else { return Ok(()) };
                // By what the head says of the one who deletes: one put out
                // deletes nothing back-dated, not even their own.
                if row.scope_id != scope_id || m.links.target.as_deref() != Some(target) || !state.can_delete_message(&m.author, snapshot_reader.as_ref(), &target_author) {
                    return Ok(());
                }
                if row.deleted {
                    return Ok(());
                }
                db::content::mark_deleted(&self.env.db, target).await?;
            }
            _ => return Ok(()),
        }
        let mut ev = serde_json::json!({ "community_id": cid, "id": m.id.as_hex() });
        if let Some(target) = target {
            ev["target"] = target.into();
        }
        out.event(UI_EVENT_CHANNEL_MESSAGE, ev);
        Ok(())
    }

    /// A note of moderation (`cmod.*`): its marks kept, each in force or
    /// not by the journal now (`marks::active`), and what they say of the
    /// messages they name.
    async fn on_mod(&self, log: &OpLog, m: wire::InnerMessage, out: &mut Outcome) -> Result<()> {
        let state = log.state();
        let cid = state.community_id.clone();
        let Ok(env) = Envelope::parse(&m.content) else { return Ok(()) };
        let Ok(Some(note)) = ModNote::parse(&env) else { return Ok(()) };
        let mut fresh = false;
        for mut row in note.rows(&cid, m.id.as_hex(), &m.author, m.created_at) {
            let target = if row.target_id.is_empty() { None } else { db::content::get(&self.env.db, &row.target_id).await? };
            let thread_author = match &target {
                Some(t) if !t.is_post() => db::content::get(&self.env.db, &t.post_id).await?.map(|p| p.author),
                _ => None,
            };
            row.active = marks::active(&row, state, target.as_ref().map(|t| t.author.as_str()), thread_author.as_deref());
            // A note heard again keeps its marks as they are: silent.
            if !db::marks::insert_if_new(&self.env.db, &row).await? {
                continue;
            }
            fresh = true;
            if let Some(t) = target {
                self.resolve(log, &t).await?;
            }
        }
        if !fresh {
            return Ok(());
        }
        out.event(UI_EVENT_CHANNEL_MESSAGE, serde_json::json!({ "community_id": cid, "mod": m.id.as_hex() }));
        Ok(())
    }

    /// What the marks about a message add up to now.
    pub(crate) async fn resolve(&self, log: &OpLog, msg: &MessageRow) -> Result<()> {
        let mut all = db::marks::by_target(&self.env.db, &msg.community_id, &msg.id).await?;
        all.extend(db::marks::by_author_range(&self.env.db, &msg.community_id, &msg.author, msg.created_at).await?);
        let held = msg.review != REVIEW_OK;
        let subject = PubKey::parse(&msg.author).is_some_and(|a| log.state().subject_to_premod(&a, Some(&Reader { since: 0 })));
        let r = marks::resolve(&all, msg.created_at, &msg.version, held, subject);
        let mod_state = if r.hidden { db::content::MOD_HIDDEN } else { db::content::MOD_SHOWN };
        db::content::set_resolution(&self.env.db, &msg.id, r.review.as_str(), mod_state).await?;
        Ok(())
    }

    /// The card of a channel: from the owner by the journal only.
    async fn on_card(&self, signed: &serde_json::Value) -> Result<Outcome> {
        let mut out = Outcome::default();
        let Some(cid) = tag(signed, "d") else { return Ok(out) };
        let Some(log) = self.log(&cid).await? else { return Ok(out) };
        match wire::verify_card(signed, &log.state().owner) {
            Ok(card) => {
                // The readers the card counts are for the one who looks
                // from outside; a member holds the keeper's snapshot, which
                // counts them, and a card heard again by a catch-up (an
                // old one says 0) takes nothing away from it: the snapshot's
                // count is written back instead, which puts right a row an
                // old card wrote 0 into (живой запуск 2026-10-24:
                // «читателей: 0» after a restart of the app).
                let members = self.snapshot_members(&cid).await?.unwrap_or(card.members);
                if let Some(mut row) = db::channels::get(&self.env.db, &cid).await? {
                    if row.members != i64::try_from(members).unwrap_or(i64::MAX) {
                        row.members = i64::try_from(members).unwrap_or(i64::MAX);
                        db::channels::upsert(&self.env.db, &row).await?;
                    }
                }
            }
            Err(e) => out.note(format!("card refused: {e}")),
        }
        Ok(out)
    }

    // ─── Notes ──────────────────────────────────────────────────────────────

    /// A gift wrap heard live: opened with the key of a keeper kept here
    /// (a note to the keeper) or with mine. `None` when it is for neither.
    pub async fn on_wrap(&self, keys: &Keys, wrap: &serde_json::Value) -> Result<Option<Outcome>> {
        self.on_wrap_heard(keys, wrap, true).await
    }

    /// A gift wrap, live or from a catch-up (`live`: a note to the keeper
    /// is taken at once; one of a catch-up at the keeper's next turn, when
    /// the pages are in, oldest first).
    pub(crate) async fn on_wrap_heard(&self, keys: &Keys, wrap: &serde_json::Value, live: bool) -> Result<Option<Outcome>> {
        // Only a keeper whose own key the wrap names may open it: every wrap
        // to me (each direct message, receipt, presence of the messenger)
        // waited for the lock of every keeper here, which a turn held
        // across its uploads then, and held the loop with it (живой запуск
        // 2026-10-23: «an event of kind 1059 holds the loop for 45 s»).
        let wrap_id = wrap["id"].as_str().unwrap_or_default().to_string();
        if self.foreign_wraps.lock().is_ok_and(|f| f.contains(&wrap_id)) {
            return Ok(None);
        }
        let to = tag(wrap, "p").unwrap_or_default();
        self.mark(Some(format!("a gift wrap to {}: the keepers kept here", to.get(..8).unwrap_or(&to))));
        for cid in self.kept().await? {
            let Some(k) = self.keeper(&cid).await? else { continue };
            let pk = self.keeper_pks.lock().ok().and_then(|m| m.get(&cid).cloned());
            if pk.as_deref() != Some(to.as_str()) {
                continue;
            }
            self.mark(Some(format!("a gift wrap to the keeper of {}: waits for the keeper", short(&cid))));
            let kkeys = k.lock().await.keys().clone();
            self.mark(Some(format!("a gift wrap to the keeper of {}: opened", short(&cid))));
            if let Ok(opened) = notes::open(&kkeys, wrap) {
                // Each wrap once: a relay that replays a leaver's join (no
                // rotated link since) would have the keeper let them in again.
                let id = wrap["id"].as_str().unwrap_or_default();
                let now = self.now();
                let before = now - wire::NOTE_MAX_AGE_SECS - crate::transport::WRAP_MARGIN_SECS;
                if !db::keeper::take_wrap(&self.env.db, &cid, id, now, before).await? {
                    return Ok(Some(Outcome::default()));
                }
                return self.on_note_heard(keys, &opened, Some(&cid), live).await.map(Some);
            }
        }
        self.mark(Some(format!("a gift wrap to {}: opened by my key", to.get(..8).unwrap_or(&to))));
        match notes::open(keys, wrap) {
            Ok(opened) => {
                // Not a note of the channels: never opened again here.
                if channel_of_note(&opened.content).is_none() {
                    if let Ok(mut f) = self.foreign_wraps.lock() {
                        f.insert(&wrap_id);
                    }
                }
                self.on_note_heard(keys, &opened, None, live).await.map(Some)
            }
            Err(e) => {
                if let Ok(mut f) = self.foreign_wraps.lock() {
                    f.insert(&wrap_id);
                }
                // Asked for by `#p` of my key or of a keeper's: one that none
                // of them opens is said, never dropped unseen (живой запуск
                // 2026-10-23: joins that left no trace).
                let to = tag(wrap, "p").unwrap_or_default();
                let id = wrap["id"].as_str().unwrap_or_default();
                self.say(&format!("a gift wrap {} to {} not opened by my key or a keeper's: {e}", id.get(..8).unwrap_or(id), to.get(..8).unwrap_or(&to)));
                Ok(None)
            }
        }
    }

    /// A note opened (here, or by the messenger's door of direct
    /// messages): `keeper_of` names the channel whose keeper key opened
    /// it. The checks of wire §11.6: the date in the window, the sender by
    /// the journal.
    pub async fn on_note(&self, keys: &Keys, n: &Opened, keeper_of: Option<&str>) -> Result<Outcome> {
        self.on_note_heard(keys, n, keeper_of, true).await
    }

    async fn on_note_heard(&self, keys: &Keys, n: &Opened, keeper_of: Option<&str>, live: bool) -> Result<Outcome> {
        let mut out = Outcome::default();
        // A peer note of the messenger (a receipt, a presence) comes on the
        // same `#p`: only what reads as a note of the channels is ours.
        let Ok(env) = Envelope::parse(&n.content) else { return Ok(out) };
        let ours = env.t.starts_with("channel.") || KEEPER_NOTES.contains(&env.t.as_str());
        let now = self.now();
        if n.kind == KIND_OWN {
            if env.t == T_OWN_KEYS || env.t == wire::T_INVITE {
                return self.on_own_note(keys, n, &env).await;
            }
            return Ok(out);
        }
        // The window of §7: a week back, the skew ahead; an admission is
        // judged by its own term (`Admit::check`).
        let in_window = if env.t == wire::T_ADMIT { n.created_at <= now + wire::SKEW_SECS } else { wire::note_in_window(n.created_at, now) };
        if !in_window {
            out.note(format!("{}: out of the time window", env.t));
            return Ok(out);
        }
        let Some(cid) = env.str_field("community_id").map(str::to_string) else {
            if ours {
                self.say(&format!("{} from {}: dropped, it names no channel", env.t, n.sender.short()));
            }
            return Ok(out);
        };
        self.mark(Some(format!("{} from {} for {}", env.t, n.sender.short(), short(&cid))));
        if let Some(kept) = keeper_of {
            if kept != cid {
                return Ok(out);
            }
            return self.on_keeper_note(n, &cid, live).await;
        }
        let log = self.log(&cid).await?;
        let me = Self::me(keys)?;
        // A note to the keeper of a channel kept here that names my key as
        // its inbox (wire §3, §7: the keeper of the app hears on my
        // connection); the copy of an admission of somebody else is the
        // keeper's too, mine is the one that names me.
        if KEEPER_NOTES.contains(&env.t.as_str()) {
            let to_keeper = log.as_ref().and_then(|l| l.state().keeper()).is_some_and(|k| k.inbox() == &me);
            let my_copy = env.t == wire::T_ADMIT && wire::note_body::<Admit>(&env).is_ok_and(|a| a.who == me);
            if to_keeper && !my_copy && self.keeper(&cid).await?.is_some() {
                return self.on_keeper_note(n, &cid, live).await;
            }
            // A keeper on its way here (wire §15): its notes wait in its
            // queue, taken once its state has come.
            if to_keeper && !my_copy && db::handover::take(&self.env.db, &cid).await?.is_some() {
                return self.on_keeper_note(n, &cid, false).await;
            }
            // Not taken here: said, whatever the reason (живой запуск
            // 2026-10-23: a join heard and dropped left no trace).
            if !my_copy {
                let inbox = log.as_ref().and_then(|l| l.state().keeper().map(|k| k.inbox().clone()));
                let why = match inbox {
                    None if log.is_none() => "the journal of the channel is not here",
                    None => "the journal names no keeper",
                    Some(inbox) if inbox != me => "the journal names another inbox of its keeper",
                    Some(_) => "its keeper is not kept on this device",
                };
                self.say(&format!("{} from {} for {}: not taken here, {why}", env.t, n.sender.short(), short(&cid)));
            }
        }
        match env.t.as_str() {
            wire::T_ISSUE => {
                let Some(log) = log else {
                    self.say(&format!("{} from {} for {}: dropped, the journal of the channel is not here", env.t, n.sender.short(), short(&cid)));
                    return Ok(out);
                };
                let issue: Issue = wire::note_body(&env)?;
                let was = db::channels::get(&self.env.db, &cid).await?.map(|r| r.membership);
                let held = self.scopes_keyed(keys, log.state()).await?;
                let m = self.member(keys).on_issue(&log, &n.sender, &issue, now).await?;
                let step = |what: &str| self.mark(Some(format!("{} from {} for {}: {what}", env.t, n.sender.short(), short(&cid))));
                step("the row of the channel");
                if was.as_deref() != Some(MEMBERSHIP_JOINED) || !m.keys.is_empty() {
                    self.say(&format!("{} from {} for {}: {} key(s) of the channel taken", env.t, n.sender.short(), short(&cid), m.keys.len()));
                }
                db::channels::joining_done(&self.env.db, &cid).await?;
                self.refresh_row(log.state(), &me, Some(MEMBERSHIP_JOINED)).await?;
                // The same issue heard again by the next round of listening
                // (no key new here, joined already) changes nothing: no
                // round more, no event (живой запуск 2026-10-22).
                let fresh = was.as_deref() != Some(MEMBERSHIP_JOINED) || !m.keys.is_empty();
                step("what waited for the keys read again");
                self.read_pending(keys, m.pending, &mut out).await?;
                // A section I was let into just now: what was sealed there
                // before (its posts, the meta of a hidden one) came while it
                // was not mine and was dropped; its feed is asked again.
                for scope_id in self.scopes_keyed(keys, log.state()).await?.difference(&held) {
                    if scope_id != SCOPE_EVERYONE {
                        step("the feed of a section new to me asked for");
                        self.ask_scope_feed(&cid, scope_id).await?;
                    }
                }
                step("the secrets of links owed");
                self.publish_owed_links(keys).await?;
                if fresh {
                    out.resubscribe = true;
                    out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "membership": MEMBERSHIP_JOINED }));
                }
            }
            wire::T_ADMIT => {
                // The copy of an admission: I ask the keeper in with it.
                let Some(log) = log else { return Ok(out) };
                let admit: Admit = wire::note_body(&env)?;
                let state = log.state();
                let moderator = state.role_of(&n.sender).is_some_and(Role::is_moderator) && !state.is_banned(&n.sender);
                if admit.who != me || !moderator || admit.check(now).is_err() {
                    return Ok(out);
                }
                // The same copy heard again (a subscription made anew brings
                // two days of wraps back, a catch-up pages them) changes
                // nothing while the join it led to waits for the keeper's
                // issue, or once that join went out in this run: one join an
                // admission, not one a replay (живой запуск 2026-10-24: an
                // agent sent the keeper a join every five seconds and was
                // issued no key each time). Not by my membership: a ticket of
                // the moderator (§17) has me in before the keeper heard of me,
                // and the join is what tells it. After a start, the catch-up
                // hands one admission once more: one join, issued again.
                let waits = db::channels::joining(&self.env.db)
                    .await?
                    .into_iter()
                    .find(|j| j.community_id == cid)
                    .and_then(|j| serde_json::from_str::<Join>(&j.join_json).ok())
                    .is_some_and(|j| j.admit_id.as_deref() == Some(admit.admit_id.as_str()));
                if waits {
                    self.say(&format!("{} from {} for {}: heard again, my join waits for the keeper", env.t, n.sender.short(), short(&cid)));
                    return Ok(out);
                }
                if !self.admits_joined.lock().map(|mut a| a.first(&admit.admit_id)).unwrap_or(true) {
                    self.say(&format!("{} from {} for {}: heard again, joined by it already", env.t, n.sender.short(), short(&cid)));
                    return Ok(out);
                }
                let keeper = state.keeper().ok_or_else(|| MessengerError::Invalid("no keeper".into()))?.inbox().clone();
                let join = Join { community_id: cid.clone(), proof: None, admit_id: Some(admit.admit_id.clone()), created_at: now };
                self.send_join(state, &keeper, &join).await?;
                self.refresh_row(state, &me, Some(MEMBERSHIP_JOINING)).await?;
            }
            wire::T_NOT_LET_IN => {
                let Some(log) = log else { return Ok(out) };
                let body: wire::NotLetIn = wire::note_body(&env)?;
                let keeper = log.state().keeper().is_some_and(|k| k.pubkey == n.sender);
                if body.check().is_ok() && body.community_id == cid && keeper {
                    self.say(&format!("{} from {} for {}: never let in as a reader, invite first", env.t, n.sender.short(), short(&cid)));
                    if let Ok(mut r) = self.not_let_in.lock() {
                        r.insert(cid.clone());
                    }
                    out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "keeper": crate::service::NEVER_LET_IN }));
                }
            }
            wire::T_REJECTED => {
                let Some(log) = log else { return Ok(out) };
                let body: Rejected = wire::note_body(&env)?;
                let state = log.state();
                let keeper = state.keeper().is_some_and(|k| k.pubkey == n.sender);
                let moderator = state.role_of(&n.sender).is_some_and(Role::is_moderator);
                if body.check().is_ok() && (keeper || moderator) {
                    self.say(&format!("{} from {} for {}: the join was refused", env.t, n.sender.short(), short(&cid)));
                    db::channels::joining_done(&self.env.db, &cid).await?;
                    self.refresh_row(state, &me, Some(MEMBERSHIP_REJECTED)).await?;
                    out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid, "membership": MEMBERSHIP_REJECTED }));
                }
            }
            wire::T_REQUEST => {
                let Some(log) = log else { return Ok(out) };
                let body: Request = wire::note_body(&env)?;
                let state = log.state();
                if body.check(now).is_err() || !state.role_of(&me).is_some_and(Role::is_moderator) || state.is_banned(&n.sender) {
                    return Ok(out);
                }
                let waiting = db::channels::requests(&self.env.db, &cid, "in", "pending").await?;
                if waiting.len() >= crate::service::MAX_REQUESTS && !waiting.iter().any(|r| r.requester == n.sender.as_hex()) {
                    out.note("too many requests");
                    return Ok(out);
                }
                let row = db::channels::RequestRow {
                    community_id: cid.clone(),
                    requester: n.sender.as_hex().to_string(),
                    direction: "in".into(),
                    status: "pending".into(),
                    payload_json: serde_json::to_string(&body)?,
                    created_at: n.created_at,
                    updated_at: now,
                };
                db::channels::put_request(&self.env.db, &row).await?;
                out.event(UI_EVENT_CHANNEL_REQUEST, serde_json::json!({ "community_id": cid, "requester": n.sender.as_hex() }));
            }
            wire::T_TICKET => {
                let body: wire::Tickets = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_tickets(keys, n, &body).await?);
                }
            }
            wire::T_UPLOAD_KEYS => {
                let body: wire::UploadKeys = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_upload_keys(keys, n, &body).await?);
                }
            }
            wire::T_TICKET_USE => {
                let body: wire::TicketUse = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_ticket_use(keys, n, &body).await?);
                }
            }
            wire::T_TRANSFER_OFFER => {
                let body: wire::TransferOffer = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_transfer_offer(keys, n, &body).await?);
                }
            }
            wire::T_TRANSFER_DECLINE => {
                let body: wire::TransferDecline = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_transfer_decline(n, &body).await?);
                }
            }
            wire::T_KEEPER_HANDOVER => {
                let body: wire::KeeperHandover = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_keeper_handover(n, &body).await?);
                }
            }
            wire::T_LEVEL_REQUEST => {
                let Some(log) = log else { return Ok(out) };
                let body: wire::LevelRequest = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_level_request(&me, log.state(), &n.sender, n.created_at, &body).await?);
                }
            }
            wire::T_LEVEL_RESOLVE => {
                let Some(log) = log else { return Ok(out) };
                let body: wire::LevelResolve = wire::note_body(&env)?;
                if body.community_id == cid {
                    out.merge(self.on_level_resolve(&me, log.state(), &n.sender, &body).await?);
                }
            }
            wire::T_INVITE => {
                let body: Invite = wire::note_body(&env)?;
                if body.check(now).is_err() {
                    return Ok(out);
                }
                // A journal here says whether the inviter may invite; one
                // not here yet is checked when it comes (the keeper takes
                // only an admission of a moderator anyway).
                if let Some(log) = &log {
                    if !log.state().role_of(&n.sender).is_some_and(Role::is_moderator) {
                        return Ok(out);
                    }
                }
                // Anybody can send one naming any channel: the limits of §7 on
                // what one sender and everybody may have open here.
                db::channels::expire_invites(&self.env.db, now).await?;
                let open = db::channels::invites(&self.env.db, "in", &["received"]).await?;
                let from_sender = open.iter().filter(|i| i.peer == n.sender.as_hex()).count();
                if open.len() >= crate::service::MAX_OPEN_INVITES || from_sender >= crate::service::MAX_INVITES_PER_PERSON {
                    out.note("too many invitations");
                    return Ok(out);
                }
                self.keep_invite(&body, "in", &n.sender, "received", log.is_some()).await?;
                out.event(UI_EVENT_CHANNEL_INVITE, serde_json::json!({ "community_id": cid, "invite_id": body.invite_id }));
            }
            wire::T_INVITE_REPLY => {
                let Some(log) = log else { return Ok(out) };
                let body: InviteReply = wire::note_body(&env)?;
                let Some(row) = db::channels::invite(&self.env.db, &body.invite_id).await? else { return Ok(out) };
                // The invitation is of this channel: `admit` below uses the journal
                // of the envelope's channel, so a reply naming another one would
                // let somebody invited to one channel into another.
                let same = row.community_id == cid && body.community_id == cid;
                if !same || body.check().is_err() || row.direction != "out" || row.peer != n.sender.as_hex() || row.status != "sent" || row.expires_at <= now {
                    return Ok(out);
                }
                if body.accept {
                    self.admit(keys, log.state(), &n.sender, Some(body.invite_id.clone())).await?;
                    db::channels::set_invite_status(&self.env.db, &body.invite_id, "done").await?;
                } else {
                    db::channels::set_invite_status(&self.env.db, &body.invite_id, "declined").await?;
                }
                out.event(UI_EVENT_CHANNEL_INVITE, serde_json::json!({ "community_id": cid, "invite_id": body.invite_id }));
            }
            _ => {}
        }
        Ok(out)
    }

    /// A note to the keeper of `cid`, kept here: once by its rumor (the
    /// copies of one note on the channel's relays, on mine and through the
    /// door of the messenger), into the keeper's queue, taken at the
    /// keeper's next turn, asked for now: right after this event when it
    /// came live; from a catch-up, when the pages that come newest first
    /// are in and the queue is taken oldest first (the trial of 1.6,
    /// problem 6). Never taken here: the turn's hosts round trips held
    /// the loop 40–96 s on every wrap (живой запуск 2026-10-23, «an event
    /// of kind 1059 holds the loop for 83 s»), and a note heard already
    /// asks for no turn at all.
    async fn on_keeper_note(&self, n: &Opened, cid: &str, live: bool) -> Result<Outcome> {
        let mut out = Outcome::default();
        let now = self.now();
        let before = now - wire::NOTE_MAX_AGE_SECS - crate::transport::WRAP_MARGIN_SECS;
        let note_id = if n.id.is_empty() { crate::blobs::sha256_hex(format!("{}:{}:{}", n.sender.as_hex(), n.created_at, n.content).as_bytes()) } else { n.id.clone() };
        let t = Envelope::parse(&n.content).map(|e| e.t).unwrap_or_default();
        let what = format!("{t} from {} for {}", n.sender.short(), short(cid));
        self.mark(Some(format!("{what}: to the keeper's queue")));
        if !db::keeper::take_wrap(&self.env.db, cid, &note_id, now, before).await? {
            // The copies of one note (the channel's relays and mine, two
            // subscriptions): the second said live only, as a catch-up
            // hears days of notes again.
            if live {
                self.say(&format!("{what}: heard already"));
            }
            return Ok(out);
        }
        self.say(&format!("{what}: to the keeper{}", if live { "" } else { ", from a catch-up: taken at its next turn" }));
        let note = db::keeper::QueuedNote { note_id, sender: n.sender.as_hex().to_string(), content: n.content.clone(), created_at: n.created_at };
        if !db::keeper::queue_note(&self.env.db, cid, &note, now, MAX_QUEUED_NOTES).await? {
            self.say(&format!("{what}: dropped, the keeper's queue is full (its sender sends it again)"));
            return Ok(out);
        }
        out.wake = u64::try_from(now).ok();
        if live {
            self.ask_turn();
        }
        Ok(out)
    }

    /// The keeper takes the notes of its queue, oldest first. A join it
    /// has no room for this minute stays in the queue, with the joins
    /// after it; the rest goes on. When to come back for what stays.
    pub(crate) async fn drain_keeper(&self, k: &mut Keeper, log: &OpLog, now: i64) -> Result<Option<u64>> {
        let cid = k.community_id().to_string();
        let mut kept: i64 = 0;
        let mut joins_wait = false;
        loop {
            let batch = db::keeper::queued_notes(&self.env.db, &cid, kept, QUEUE_BATCH).await?;
            if batch.is_empty() {
                break;
            }
            for q in batch {
                let env = Envelope::parse(&q.content).ok();
                if joins_wait && env.as_ref().is_some_and(|e| e.t == wire::T_JOIN) {
                    kept += 1;
                    continue;
                }
                let what = || format!("{} from {} for {}", env.as_ref().map_or("a note", |e| e.t.as_str()), q.sender.get(..8).unwrap_or(&q.sender), short(&cid));
                self.mark(Some(format!("keeper {}: {}", short(&cid), what())));
                match self.keeper_note(k, log, &q, env.as_ref(), now).await {
                    Ok(Some(Joined::Busy)) => {
                        self.say(&format!("{}: the keeper is busy with joins, taken in a minute", what()));
                        joins_wait = true;
                        kept += 1;
                        continue;
                    }
                    Ok(Some(Joined::Issued)) => self.say(&format!("{}: taken, a reader of the channel", what())),
                    Ok(Some(Joined::Held)) => self.say(&format!("{}: waits for the admission it names", what())),
                    Ok(Some(Joined::Refused(why))) => self.say(&format!("{}: refused, {why}", what())),
                    Ok(None) => {}
                    // The database of this device failed: the note stays in
                    // the queue for the next turn, its sender's resend may
                    // be hours away.
                    Err(e @ MessengerError::Storage(_)) => {
                        self.say(&format!("{}: not taken yet, again at the next turn: {e}", what()));
                        joins_wait = true;
                        kept += 1;
                        continue;
                    }
                    Err(e) => self.say(&format!("{}: not taken, {e}", what())),
                }
                db::keeper::unqueue_note(&self.env.db, &cid, &q.note_id).await?;
            }
        }
        Ok(joins_wait.then(|| u64::try_from(now + 60).unwrap_or(0)))
    }

    /// One note of the keeper's queue; what became of a join.
    async fn keeper_note(&self, k: &mut Keeper, log: &OpLog, q: &db::keeper::QueuedNote, env: Option<&Envelope>, now: i64) -> Result<Option<Joined>> {
        let (Some(env), Some(sender)) = (env, PubKey::parse(&q.sender)) else {
            self.say(&format!("a note to the keeper of {}: dropped, it does not read", short(k.community_id())));
            return Ok(None);
        };
        // The window of §7 again: a note may have waited in the queue.
        let in_window = if env.t == wire::T_ADMIT { q.created_at <= now + wire::SKEW_SECS } else { wire::note_in_window(q.created_at, now) };
        if !in_window || env.str_field("community_id") != Some(k.community_id()) {
            let why = if in_window { "it names another channel" } else { "out of the time window" };
            self.say(&format!("{} from {} for {}: dropped, {why}", env.t, sender.short(), short(k.community_id())));
            return Ok(None);
        }
        match env.t.as_str() {
            wire::T_JOIN => {
                let body: Join = wire::note_body(env)?;
                let joined = k.on_join(log, &sender, &body, now).await?;
                tracing::debug!(community = %k.community_id(), ?joined, "join");
                return Ok(Some(joined));
            }
            wire::T_ADMIT => {
                let body: Admit = wire::note_body(env)?;
                k.on_admit(log, &sender, &body, now).await?;
            }
            wire::T_REBUNDLE => {
                let body: Rebundle = wire::note_body(env)?;
                k.on_rebundle(log, &sender, &body, now).await?;
            }
            wire::T_MISSED => {
                let body: Missed = wire::note_body(env)?;
                k.on_missed(log, &sender, &body, now).await?;
            }
            wire::T_LEAVE => {
                let body: Leave = wire::note_body(env)?;
                if body.check().is_ok() {
                    k.on_leave(&sender, now).await?;
                }
            }
            wire::T_KEEPER_MASTER => {
                let body: KeeperMaster = wire::note_body(env)?;
                k.on_master(log, &sender, &body).await?;
            }
            wire::T_TICKET_REQUEST => {
                let body: wire::TicketRequest = wire::note_body(env)?;
                k.on_ticket_request(log, &sender, &body, now).await?;
            }
            _ => {}
        }
        Ok(None)
    }

    /// A note of my own: my keys of a channel from another device of
    /// mine, or the copy of an invitation I made.
    async fn on_own_note(&self, keys: &Keys, n: &Opened, env: &Envelope) -> Result<Outcome> {
        let mut out = Outcome::default();
        let me = Self::me(keys)?;
        if n.sender != me {
            return Ok(out);
        }
        if env.t == wire::T_INVITE {
            let body: Invite = wire::note_body(env)?;
            if body.check(self.now()).is_ok() {
                self.keep_invite(&body, "out", &me, "sent", true).await?;
            }
            return Ok(out);
        }
        let body: OwnKeys = wire::note_body(env)?;
        let Some(log) = self.log(&body.community_id).await? else { return Ok(out) };
        let was = db::channels::get(&self.env.db, &body.community_id).await?.map(|r| r.membership);
        self.mark(Some(format!("{} for {}: the keys of my other device", env.t, short(&body.community_id))));
        let m = self.member(keys).on_own_keys(log.state(), &n.sender, &body).await?;
        self.refresh_row(log.state(), &me, Some(MEMBERSHIP_JOINED)).await?;
        // Heard again (no key new here, joined already): nothing changed.
        let fresh = was.as_deref() != Some(MEMBERSHIP_JOINED) || !m.keys.is_empty();
        self.mark(Some(format!("{} for {}: what waited for the keys read again, the secrets of links owed", env.t, short(&body.community_id))));
        self.read_pending(keys, m.pending, &mut out).await?;
        self.publish_owed_links(keys).await?;
        if fresh {
            out.resubscribe = true;
            out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": body.community_id }));
        }
        Ok(out)
    }

    /// Whether a sealed event may wait in `cm_pending`: anybody can sign
    /// kind 1390 with any `h`, `n` and `k`, so only for a channel I follow
    /// (not left, not disbanded, or kept here), under an epoch the rules
    /// of §5 could still take (`current + MAX_GAP`), and at most
    /// `MAX_PENDING` of a channel.
    async fn may_park(&self, community_id: &str, scope_id: &str, epoch: u32) -> Result<bool> {
        let followed = db::channels::get(&self.env.db, community_id)
            .await?
            .is_some_and(|r| r.membership != crate::service::MEMBERSHIP_LEFT && r.membership != crate::service::MEMBERSHIP_DISBANDED)
            || db::keeper::keeper(&self.env.db, community_id).await?.is_some();
        if !followed {
            return Ok(false);
        }
        let current = db::keys::current_epoch(&self.env.db, community_id, scope_id).await?;
        if current.is_some_and(|c| epoch > c.saturating_add(crate::epoch::MAX_GAP)) {
            return Ok(false);
        }
        Ok(db::channels::count_pending(&self.env.db, community_id).await? < db::channels::MAX_PENDING)
    }

    /// Keeps a sealed event aside under `marker` (in `cm_pending`, the 7
    /// days of §4) until what it waits for comes and `take_pending` with
    /// the same marker gives it back.
    pub(crate) async fn park(&self, signed: &serde_json::Value, outer: &wire::Outer, event: &Event, marker: &str, out: &mut Outcome) -> Result<()> {
        if !self.may_park(&outer.place.community_id, &outer.place.scope_id, outer.epoch).await? {
            out.note("content for no channel followed here, or past the epochs, or too much waiting: dropped");
            return Ok(());
        }
        let row = PendingRow {
            event_id: event.id.to_hex(),
            community_id: outer.place.community_id.clone(),
            scope_id: outer.place.scope_id.clone(),
            epoch: i64::from(outer.epoch),
            key_id: marker.to_string(),
            event_json: serde_json::to_string(signed)?,
            created_at: outer.created_at,
            received_at: self.now(),
        };
        db::channels::add_pending(&self.env.db, &row).await?;
        out.note(format!("content waits: {marker}"));
        Ok(())
    }

    async fn keep_invite(&self, body: &Invite, direction: &str, peer: &PubKey, status: &str, verified: bool) -> Result<()> {
        let row = db::channels::InviteRow {
            invite_id: body.invite_id.clone(),
            community_id: body.community_id.clone(),
            direction: direction.into(),
            peer: peer.as_hex().to_string(),
            status: status.into(),
            payload_json: serde_json::to_string(body)?,
            created_at: body.created_at,
            expires_at: body.expires_at,
            updated_at: self.now(),
            verified,
        };
        db::channels::insert_invite(&self.env.db, &row).await?;
        Ok(())
    }
}

/// Whether the premoderation in force at the head holds a message of one
/// put out since, judged as the member they were (`Rules::subject_to_premod`
/// takes nobody outside): what they wrote before comes to a device after
/// them through the queue as their new words would.
fn premod_now(log: &OpLog, author: &PubKey, reader: Option<&Reader>, head: i64) -> bool {
    let mut rules = rules_at(log, author, reader, head);
    rules.standing = crate::state::Standing::Member;
    rules.subject_to_premod()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_door_takes_the_notes_of_channels_only() {
        let cid = "ab".repeat(32);
        let join = wire::note_envelope(wire::T_JOIN, &serde_json::json!({ "community_id": cid })).unwrap();
        assert_eq!(channel_of_note(&join), Some(cid.clone()));
        let own = wire::note_envelope(T_OWN_KEYS, &serde_json::json!({ "community_id": cid })).unwrap();
        assert_eq!(channel_of_note(&own), Some(cid));
        assert_eq!(channel_of_note(&Envelope::text("hi").encode()), None);
        assert_eq!(channel_of_note("not json"), None);
    }
}

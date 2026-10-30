// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/service.rs at 1f2ef31d, then changed for channels.

//! Channels on a device: the journal and the keys in `communities.db`,
//! what the user does, what goes out. Everything that arrives is in
//! `inbound`.
//!
//! Unlike the groups' service this one sends by itself (`Transport`): a
//! keeper must know that its blobs are on the hosts and that its epoch
//! event reached a relay before it calls the epoch published, so the
//! order of what goes out is the service's, not the host's. What the UI
//! should hear comes back as events (`Outcome`).

use crate::blobs::BlobHosts;
use crate::db::{self, channels::ChannelRow, Db};
use crate::keeper::{Env, Job, Keeper};
use crate::link::{ChannelLink, LinkSecret};
use crate::log::{Insert, OpLog};
use crate::marks::ModNote;
use crate::member::fetch::{self, FetchJob, Got, Want};
use crate::member::Member;
use crate::op::{History, JoinMode, KeeperRef, Op, OpBody};
use crate::roles::{permits, Action, Role};
use crate::secrets;
use crate::state::{rules_at, ChannelState, Content, Reader, SCOPE_EVERYONE};
use crate::wire::{self, Admit, Card, Invite, InviteReply, Issue, Join, JoinProof, Leave, LinkPayload, Links, Payload, Place, Rejected, Request};
use keydist::{RootSecret, SchemeId};
use messenger_core::traits::UiEvent;
use messenger_core::{Clock, Envelope, MessengerError, Outbound, PubKey, RelayUrl, Result, Scope, SecretStore, TextFormat, Transport};
use nostr::key::Keys;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;

pub const UI_EVENT_CHANNEL_UPDATED: &str = "channel.updated";
pub const UI_EVENT_CHANNEL_MESSAGE: &str = "channel.message";
/// Somebody is writing a comment under a post: `{"community_id",
/// "post_id", "author": "<hex>", "until": <secs>}` — shown as «… печатает»
/// under the comments until `until`, kept nowhere.
pub const UI_EVENT_CHANNEL_TYPING: &str = "channel.typing";
pub const UI_EVENT_CHANNEL_INVITE: &str = "channel.invite";
/// What the keeper says to one the journal names (staff) who asks for a
/// bundle and was never let in as a reader (`channel.not_let_in`).
pub const NEVER_LET_IN: &str = "never let in: invite first";
pub const UI_EVENT_CHANNEL_REQUEST: &str = "channel.request";
pub use crate::outbox::UI_EVENT_CHANNEL_OUTBOX;

/// A channel whose journal came here, without me in it.
pub const MEMBERSHIP_NONE: &str = "none";
pub const MEMBERSHIP_JOINED: &str = "joined";
/// Asked the keeper in; the bundle has not come.
pub const MEMBERSHIP_JOINING: &str = "joining";
/// Asked the moderators by request.
pub const MEMBERSHIP_REQUESTED: &str = "requested";
pub const MEMBERSHIP_REJECTED: &str = "rejected";
pub const MEMBERSHIP_LEFT: &str = crate::member::MEMBERSHIP_OUT;
pub const MEMBERSHIP_DISBANDED: &str = "disbanded";

/// Why I am out of a channel (`ChannelRow::out_reason`): I left (here or
/// on another device), a moderator removed or banned me, the owner
/// disbanded it.
pub const OUT_LEFT: &str = "left";
pub const OUT_REMOVED: &str = "removed";
pub const OUT_BANNED: &str = "banned";
pub const OUT_DISBANDED: &str = "disbanded";

/// A `channel.join` without an answer is sent again after this, then
/// after twice as long each time, up to `JOIN_RETRY_MAX_SECS` apart, for
/// the week a note lives (wire §7).
pub const JOIN_RETRY_SECS: i64 = 600;
pub const JOIN_RETRY_MAX_SECS: i64 = 6 * 3600;

/// A keeper held elsewhere when the loop comes for its turn (a command's
/// turn and its uploads): tried again this much later, not waited for.
pub const BUSY_KEEPER_SECS: u64 = 5;

/// The requests to the relays the events wanted kept until they are sent
/// (`ChannelService::take_asks`).
pub const MAX_ASKS: usize = 1024;

/// The limits of invitations and requests, those of groups (wire §7).
pub const MAX_OPEN_INVITES: usize = 50;
pub const MAX_INVITES_PER_CHANNEL: usize = 4;
pub const MAX_INVITES_PER_PERSON: usize = 10;
pub const MAX_REQUESTS: usize = 20;

/// Operations of a channel kept while they wait for their parents (or,
/// before its `create` is here, at all): a bound on what anybody can
/// make every member keep.
pub const MAX_WAITING_OPS: usize = 1024;

fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel: {what}"))
}

/// What an action or an arrival leads to, for the host.
#[derive(Debug, Default)]
pub struct Outcome {
    pub events: Vec<UiEvent>,
    /// The set of channels to listen to changed.
    pub resubscribe: bool,
    /// When a keeper wants its cycle run again (unix seconds).
    pub wake: Option<u64>,
    /// For the log of whoever runs the service.
    pub notes: Vec<String>,
    /// What was put into the outbox and waits for the relays, by the id
    /// the page shows (`channel.outbox` tells when it went, or failed).
    pub pending: Vec<String>,
}

impl Outcome {
    pub(crate) fn merge(&mut self, mut other: Outcome) {
        self.events.append(&mut other.events);
        self.resubscribe |= other.resubscribe;
        self.notes.append(&mut other.notes);
        self.pending.append(&mut other.pending);
        self.wake = match (self.wake, other.wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    pub(crate) fn event(&mut self, name: &str, payload: serde_json::Value) {
        self.events.push(UiEvent { name: name.into(), payload });
    }

    pub(crate) fn note(&mut self, line: impl Into<String>) {
        self.notes.push(line.into());
    }
}

/// The first eight of a channel's id, for the log.
pub(crate) fn short(community_id: &str) -> &str {
    community_id.get(..8).unwrap_or(community_id)
}

/// What names my join to the keeper of a channel in the outbox.
fn join_ref(community_id: &str) -> String {
    format!("join:{community_id}")
}

/// The line of a note sent: to whom, and which relays took it; none is
/// said plainly, as the joiner then waits for nothing.
pub(crate) fn sent_line(t: &str, community_id: &str, to: &PubKey, ack: &messenger_core::Ack) -> String {
    let took: Vec<&str> = ack.accepted_by.iter().map(RelayUrl::as_str).collect();
    let refused: Vec<String> = ack.rejected_by.iter().map(|(r, why)| format!("{} ({why})", r.as_str())).collect();
    match (took.is_empty(), refused.is_empty()) {
        (false, _) => format!("{t} to {} for {}: taken by {}", to.short(), short(community_id), took.join(", ")),
        (true, true) => format!("{t} to {} for {}: sent, no relay answered yet", to.short(), short(community_id)),
        (true, false) => format!("{t} to {} for {}: no relay took it ({})", to.short(), short(community_id), refused.join(", ")),
    }
}

/// What a new channel is made with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewChannel {
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub picture: String,
    pub join: JoinMode,
    #[serde(default)]
    pub history: History,
    pub relays: Vec<String>,
    pub storage: Vec<String>,
}

/// A keeper kept here, as its last turn left it: read without waiting
/// for the keeper (`storage_error` follows the jobs of the hosts as they
/// end, `ChannelService::job_done`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperStatus {
    /// The newest epoch published.
    pub epoch: Option<u32>,
    /// An epoch built and waiting for its hosts or relays.
    pub pending_epoch: Option<u32>,
    /// When the newest epoch was built, unix seconds.
    pub last_epoch_at: Option<i64>,
    pub readers: u32,
    pub generations: u32,
    /// Blobs waiting to be removed from the hosts.
    pub garbage: i64,
    /// An epoch asked for now and not built yet.
    pub rotate_asked: bool,
    /// What a blob host said when it last refused a blob of the keeper
    /// (`<host> did not take …: <its answer>`), until every host takes
    /// them again: «Хранилище отказало: …» in the keeper block.
    pub storage_error: Option<String>,
    /// When this was taken, unix seconds.
    pub at: i64,
}

/// Channels on this device: the member's side for every channel I read,
/// the keeper for those I keep.
///
/// # The locks, in the order they are taken
///
/// - The lock of a channel (`lock_of`) first: a command or an event of
///   one channel at a time. Under it: `logs` (the journal in memory), and
///   the keeper's mutex only to tell it an operation (`act_op`, the
///   journal in `inbound`) or in `create`. Never the other way round.
/// - The keeper's mutex (`keepers[cid]`): a turn (`tick_one`) takes it
///   without the channel's lock (`try_lock`: held elsewhere, the turn
///   comes again), and takes `logs` never while it holds the keeper
///   (`hand_to_me` reads content with the keeper held: that way must not
///   lead back to a lock, which is why `take_link` is lock-free).
/// - `keepers`, `locks`, `logs`, `fetches` are tokio mutexes held only to
///   look something up. `statuses`, `troubles`, `uploading`,
///   `rotate_asked`, `keeper_pks`, `transfers`, `foreign_wraps`, `admits_joined`,
///   `notes_caught`, `pages_asked`, `page_seen` are leaves: std mutexes, never held
///   across an await.
/// - `flushing` (the outbox worker) is taken by nobody who holds a
///   channel's lock or a keeper.
/// - `Env::fetches` (what the member's side wants of the hosts) and
///   `asks` (what it wants of the relays) are leaves too: noted on the way
///   of an event, taken by `take_fetches` and `take_asks`.
/// - No network under the channel's lock or the keeper's mutex, and none
///   on the way of an event (`on_event`, `on_wrap`, `on_note`): what the
///   relays must take is a row of the outbox, what the hosts must hold or
///   lose is a job (`keeper::upload`), what the member's side must fetch
///   (an epoch's index and shard, a shard of a snapshot) is a fetch
///   (`member::fetch`, `run_fetch`: the GETs with no lock held, then the
///   channel's lock only around what they open), what the relays must be
///   asked for (the page before a full page of a catch-up, the heads of
///   the journal a snapshot names) is an ask (`ask_later`, `send_asks`:
///   the pool awaits a `Sync` to its end). What is left, each once
///   in the life of a keeper series: the handover to the next series
///   (`hand_over`: its state to the hosts and one note), in the keeper's
///   task and on the journal operation that names the next keeper; the
///   state taken over by the next keeper (`on_keeper_handover`: one GET
///   of it, on its note); the reads of every shard when a device of the
///   owner recovers the keeper (`take_emergency`, a command).
/// - The step of each piece of work is marked (`mark`): within the loop of
///   the events and the keepers' turn of the module (`keeper::Steps`) the
///   step of that task, so that one that holds it long is said with it.
#[derive(Clone)]
pub struct ChannelService {
    pub(crate) env: Env,
    pub(crate) clock: Arc<dyn Clock>,
    signer: Arc<RwLock<Option<Keys>>>,
    pub(crate) keepers: Arc<Mutex<HashMap<String, Arc<Mutex<Keeper>>>>>,
    /// The key of each keeper in `keepers`, read without its lock: a keeper
    /// whose series the journal has gone past is not handed out (wire §15).
    pub(crate) keeper_pks: Arc<std::sync::Mutex<HashMap<String, String>>>,
    locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// The journal of each channel, verified and ordered once: rebuilt
    /// only when it is not here, kept in step by `store_op`.
    logs: Arc<Mutex<HashMap<String, OpLog>>>,
    /// The run of each file of mine on its way, by its upload id: a pause
    /// or a cancel reaches its requests at once (messenger-media `Control`).
    pub(crate) transfers: Arc<std::sync::Mutex<HashMap<String, messenger_media::control::Control>>>,
    /// The chunk requests of every file of the channels at once, as the
    /// messenger's media service shares them among its transfers.
    pub(crate) chunk_slots: Arc<tokio::sync::Semaphore>,
    /// The waits before the automatic attempts of a file (the messenger's).
    pub(crate) retry_delays: Arc<Vec<std::time::Duration>>,
    /// Photos made smaller at once (messenger-media `PHOTO_SLOTS`).
    pub(crate) photo_slots: Arc<tokio::sync::Semaphore>,
    /// Channels an epoch was asked for now (`rotate_now`): taken by the
    /// next turn of their keeper, never waiting for its lock.
    rotate_asked: Arc<std::sync::Mutex<HashMap<String, Vec<Option<String>>>>>,
    /// The markers of a catch-up read (`settle`): the way into the host's
    /// stream and those on their way.
    pub(crate) settle: Arc<crate::settle::Settle>,
    /// The channels whose keeper said I was never let in as a reader
    /// (`channel.not_let_in`): asking it for a bundle is no use until an
    /// invitation lets me in (`refused_by_keeper`).
    pub(crate) not_let_in: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// What each keeper kept here looked like after its last turn.
    pub(crate) statuses: Arc<std::sync::Mutex<HashMap<String, KeeperStatus>>>,
    /// What a blob host said when it last refused a blob of each keeper
    /// kept here (`job_done`), none once every host takes them again:
    /// `KeeperStatus::storage_error`.
    pub(crate) troubles: Arc<std::sync::Mutex<HashMap<String, Option<String>>>>,
    /// The jobs of the hosts running now, by `Job::key`, each with its
    /// cancel flag: a turn hands out no second job of the same key, and a
    /// pending epoch superseded cancels the job that puts its blobs.
    pub(crate) uploading: Arc<std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
    /// One fetch of a cover at a time: two pages that ask for it at once
    /// would write the same `.part` (a file of a message has the engine's
    /// own lock of its folder).
    pub(crate) fetches: Arc<Mutex<HashMap<std::path::PathBuf, Arc<Mutex<()>>>>>,
    /// Gift wraps to me that hold no note of the channels (the direct
    /// messages, receipts and presence of the messenger on the same `#p`),
    /// by id: every round of listening hands two days of them again, on
    /// every relay, and each was opened anew (two decryptions) on the
    /// one loop of the module, minutes of it while joins waited behind.
    pub(crate) foreign_wraps: Arc<std::sync::Mutex<ForeignWraps>>,
    /// The admissions I sent the keeper a join by in this run, by
    /// `admit_id`: the copy of one heard again (a subscription made anew,
    /// a catch-up) sends no second join.
    pub(crate) admits_joined: Arc<std::sync::Mutex<ForeignWraps>>,
    /// The notes to a key on a set of relays, when a round of this run
    /// last paged them (`catch_up_round`).
    pub(crate) notes_caught: Arc<std::sync::Mutex<NotesCaught>>,
    /// One `flush_outbox` at a time: two senders would interleave the
    /// rows of a channel.
    pub(crate) flushing: Arc<Mutex<()>>,
    /// A command asked for a turn of the keepers now (`Outcome::wake`):
    /// what the testkit's pump reads, as the loop reads the outcome.
    pub(crate) turn_asked: Arc<std::sync::atomic::AtomicBool>,
    /// When a page of the feed or of a thread last asked the relays for
    /// more, by channel and ask (`transport::history_page_in`).
    pub(crate) pages_asked: Arc<std::sync::Mutex<HashMap<String, i64>>>,
    /// The events a round of catch-up of a part has counted, by part
    /// (`transport::heard`): a copy of one, from another relay or from
    /// another channel on the same relays, fills no page.
    pub(crate) page_seen: Arc<std::sync::Mutex<HashMap<String, crate::transport::PageSeen>>>,
    /// What the relays are to be asked for that an event wanted: the page
    /// before a full page of a catch-up (`heard`), the heads of the journal
    /// a snapshot names that are not here (`on_snapshot`). Sent by whoever
    /// runs the service off the way of the events (`take_asks`,
    /// `send_asks`), never on it: the pool awaits a `Sync` to its end, up
    /// to 20 s a relay, and a page of the notes to me asked of every
    /// channel followed held the loop 26–56 s after every channel made
    /// (живой запуск 2026-10-23).
    pub(crate) asks: Arc<std::sync::Mutex<Vec<Outbound>>>,
}

/// When the notes to a key on a set of relays were paged last, by them.
pub(crate) type NotesCaught = HashMap<(String, Vec<String>), i64>;

/// What a turn of the keepers left: when to come again, the jobs of the
/// hosts to run off every lock (`ChannelService::run_job`), and the
/// fetches of the member's side due now (`ChannelService::run_fetch`).
#[derive(Debug, Default)]
pub struct Turned {
    pub wake: Option<u64>,
    pub jobs: Vec<Job>,
    pub fetches: Vec<FetchJob>,
}

fn word<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn random_hex(n: usize) -> Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| MessengerError::Crypto(e.to_string()))?;
    Ok(hex::encode(b))
}

impl ChannelService {
    pub fn new(db: Db, secrets: Arc<dyn SecretStore>, net: Arc<dyn Transport>, blobs: Arc<dyn BlobHosts>, clock: Arc<dyn Clock>) -> Self {
        let signer = Arc::new(RwLock::new(None));
        Self {
            env: Env {
                db,
                secrets,
                net,
                blobs,
                payer: signer.clone(),
                say: crate::keeper::Say::default(),
                outbox: Arc::default(),
                fetches: Default::default(),
                book: Arc::default(),
                clock: clock.clone(),
                cloud: Arc::default(),
            },
            clock,
            signer,
            keepers: Arc::new(Mutex::new(HashMap::new())),
            keeper_pks: Arc::default(),
            locks: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(HashMap::new())),
            transfers: Arc::default(),
            chunk_slots: Arc::new(tokio::sync::Semaphore::new(messenger_media::control::CHUNK_SLOTS)),
            retry_delays: Arc::new(messenger_media::service::RETRY_DELAYS.to_vec()),
            photo_slots: Arc::new(tokio::sync::Semaphore::new(messenger_media::prepare::PHOTO_SLOTS)),
            rotate_asked: Arc::default(),
            settle: Arc::default(),
            not_let_in: Arc::default(),
            statuses: Arc::default(),
            troubles: Arc::default(),
            uploading: Arc::default(),
            fetches: Arc::default(),
            foreign_wraps: Arc::default(),
            admits_joined: Arc::default(),
            notes_caught: Arc::default(),
            flushing: Arc::default(),
            turn_asked: Arc::default(),
            pages_asked: Arc::default(),
            page_seen: Arc::default(),
            asks: Arc::default(),
        }
    }

    /// Woken whenever a row goes into the outbox: whoever sends them
    /// (`flush_outbox`) waits on it between rows.
    pub fn outbox_notify(&self) -> Arc<tokio::sync::Notify> {
        self.env.outbox.clone()
    }

    /// A command wants a turn of the keepers now: what the loop hears as
    /// `Outcome::wake`, noted here for whoever runs turns without outcomes.
    pub(crate) fn ask_turn(&self) {
        self.turn_asked.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether a turn was asked for since the last call.
    pub fn take_turn_asked(&self) -> bool {
        self.turn_asked.swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// The user's key (none while logged out: nothing is done then).
    pub fn set_signer(&self, keys: Option<Keys>) {
        if let Ok(mut s) = self.signer.write() {
            *s = keys;
        }
    }

    /// Where the lines of the way of a note to the keeper go: the log of
    /// the app (`communities: …` on its terminal); none: `tracing`.
    pub fn set_say(&self, to: Option<crate::keeper::SayFn>) {
        self.env.say.set(to);
    }

    pub fn say(&self, line: &str) {
        self.env.say.line(line);
    }

    /// The state of one thing on the terminal (`Say::state`): a trouble said
    /// once until it changes, its end once.
    pub fn say_state(&self, what: &str, trouble: Option<&str>, fine: &str) {
        self.env.say.state(what, trouble, fine);
    }

    /// The step of the keepers' cycle running now, for the line of a loop
    /// held long (`communities: … holds the loop`).
    pub fn doing(&self) -> Option<String> {
        self.env.say.doing()
    }

    pub fn mark(&self, what: Option<String>) {
        self.env.say.mark(what);
    }

    pub fn signer(&self) -> Option<Keys> {
        self.signer.read().ok().and_then(|s| s.clone())
    }

    pub fn db(&self) -> &Db {
        &self.env.db
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now().secs()
    }

    /// The head of the journal, never past my clock and its skew: an
    /// operation stored before the bound at receipt (or a clock that ran
    /// ahead) must not date every later act and judgement past it.
    pub(crate) fn head_at(log: &OpLog, now: i64) -> i64 {
        log.head_time().min(now + wire::SKEW_SECS)
    }

    pub fn me(keys: &Keys) -> Result<PubKey> {
        PubKey::parse(&keys.public_key().to_hex()).ok_or_else(|| invalid("my key"))
    }

    pub fn member(&self, keys: &Keys) -> Member {
        Member::new(self.env.clone(), keys.clone())
    }

    /// One channel at a time: the journal is rebuilt on every operation.
    pub async fn lock_of(&self, community_id: &str) -> Arc<Mutex<()>> {
        self.locks.lock().await.entry(community_id.to_string()).or_default().clone()
    }

    /// The journal of a channel, from the signed operations kept here:
    /// verified and ordered once, then kept in memory (`store_op` keeps it
    /// in step), not rebuilt on every event, page and tick.
    pub async fn log(&self, community_id: &str) -> Result<Option<OpLog>> {
        let mut logs = self.logs.lock().await;
        if let Some(l) = logs.get(community_id) {
            return Ok(Some(l.clone()));
        }
        let built = self.build_log(community_id).await?;
        if let Some(l) = &built {
            logs.insert(community_id.to_string(), l.clone());
        }
        Ok(built)
    }

    pub(crate) async fn build_log(&self, community_id: &str) -> Result<Option<OpLog>> {
        let mut ops = Vec::new();
        for json in db::channels::ops(&self.env.db, community_id).await? {
            let value: serde_json::Value = serde_json::from_str(&json)?;
            if let Ok(op) = wire::verify_op(&value, community_id) {
                ops.push(op);
            }
        }
        if ops.is_empty() {
            return Ok(None);
        }
        match OpLog::from_ops(ops) {
            Ok(log) => Ok(Some(log)),
            // Operations that came before their `create` (a relay answers
            // newest first): no journal yet, they wait for it.
            Err(crate::state::Rejection::NotCreated) => Ok(None),
            Err(e) => Err(invalid(&format!("journal: {e:?}"))),
        }
    }

    /// Forgets the journal held in memory (its rows went).
    pub(crate) async fn drop_log(&self, community_id: &str) {
        self.logs.lock().await.remove(community_id);
    }

    /// Keeps a signed operation (verified by the caller) and the journal
    /// in memory. Refused, so never kept: a second `create`, or one not
    /// by the owner named (wire §3); an operation whose parents are here
    /// by somebody who was never staff of the journal; an operation that
    /// waits for its parents beyond `MAX_WAITING_OPS`. Rows from anybody
    /// would grow every member's disk and work for good.
    pub(crate) async fn store_op(&self, community_id: &str, op: &Op, signed: &serde_json::Value) -> Result<bool> {
        let mut logs = self.logs.lock().await;
        if !logs.contains_key(community_id) {
            if let Some(l) = self.build_log(community_id).await? {
                logs.insert(community_id.to_string(), l);
            }
        }
        let log = logs.get_mut(community_id);
        match (&op.body, &log) {
            // The id of a channel is not bound to its `create`: anybody can
            // sign one under it. One genesis per channel, never a second,
            // and only by the owner a link or an invitation named (wire §3).
            (OpBody::Create { .. }, Some(_)) => return Ok(false),
            (OpBody::Create { .. }, None) => {
                if db::channels::genesis_owner(&self.env.db, community_id).await?.is_some_and(|o| o != op.author.as_hex()) {
                    return Ok(false);
                }
            }
            (_, None) => {
                if db::channels::count_ops(&self.env.db, community_id).await? >= MAX_WAITING_OPS as i64 {
                    return Ok(false);
                }
            }
            (_, Some(l)) => {
                if l.contains(&op.id()) {
                    return Ok(false);
                }
                let parents_here = op.parents.iter().all(|p| l.contains(p));
                if (parents_here && !l.may_author(&op.author)) || (!parents_here && l.waiting_len() >= MAX_WAITING_OPS) {
                    tracing::debug!(community = %community_id, "an operation by somebody never staff, or too many waiting: not kept");
                    return Ok(false);
                }
            }
        }
        let stored = db::channels::insert_op(&self.env.db, community_id, &op.id().0, &serde_json::to_string(signed)?).await?;
        if let (true, Some(l)) = (stored, log) {
            l.insert(op.clone());
        }
        Ok(stored)
    }

    /// The owner a link (`o`) or an invitation I accept names: from now on
    /// the only author whose `create` is taken for this id. A journal held
    /// under the `create` of somebody else (put first by a forger) is
    /// dropped and asked for again; one I keep stays as it is.
    pub async fn expect_owner(&self, community_id: &str, owner: &PubKey) -> Result<()> {
        if db::keeper::keeper(&self.env.db, community_id).await?.is_some() {
            return Ok(());
        }
        db::channels::set_genesis_owner(&self.env.db, community_id, owner.as_hex()).await?;
        let Some(log) = self.log(community_id).await? else { return Ok(()) };
        if log.creator() == Some(owner) {
            return Ok(());
        }
        tracing::warn!(community = %community_id, "a journal under a create not by the owner of the link: dropped and asked for again");
        db::channels::drop_journal(&self.env.db, community_id).await?;
        self.drop_log(community_id).await;
        let filter = messenger_core::outbound::Filter(serde_json::json!({ "kinds": [wire::KIND_JOURNAL], "#h": [community_id] }));
        if let Err(e) = self.env.net.send(Outbound::Sync { scope: Scope::Channel { id: community_id.to_string() }, filter, local: vec![] }).await {
            tracing::debug!(community = %community_id, error = %e, "the journal was not asked for again");
        }
        Ok(())
    }

    /// The row a list shows of a channel, from its journal.
    pub(crate) async fn refresh_row(&self, state: &ChannelState, me: &PubKey, membership: Option<&str>) -> Result<()> {
        let old = db::channels::get(&self.env.db, &state.community_id).await?;
        let membership = membership.map(str::to_string).or_else(|| old.as_ref().map(|o| o.membership.clone())).unwrap_or_else(|| MEMBERSHIP_NONE.to_string());
        let membership = if state.disbanded { MEMBERSHIP_DISBANDED.to_string() } else { membership };
        // An old issue or note of my own keys heard again (a catch-up of
        // two days of notes) does not undo a ban of the journal.
        let membership = if membership == MEMBERSHIP_JOINED && state.is_banned(me) { MEMBERSHIP_LEFT.to_string() } else { membership };
        // The readers: the keeper's snapshot held here counts them; before
        // the first, what a card said (a row a card of 0 wrote over is
        // put right by the next refresh — живой запуск 2026-10-24).
        let members = match self.snapshot_members(&state.community_id).await? {
            Some(n) => i64::try_from(n).unwrap_or(i64::MAX),
            None => old.as_ref().map_or(0, |o| o.members),
        };
        let row = ChannelRow {
            id: state.community_id.clone(),
            name: state.name.clone(),
            about: state.about.clone(),
            picture: state.picture.clone(),
            relays_json: serde_json::to_string(&state.relays)?,
            storage_json: serde_json::to_string(&state.storage)?,
            owner: state.owner.as_hex().to_string(),
            join_mode: word(&state.join),
            membership,
            my_role: state.role_of(me).map(|r| r.as_str().to_string()),
            members,
            link_epoch: i64::from(state.link_epoch),
            created_at: old.as_ref().map_or(self.now(), |o| o.created_at),
            updated_at: self.now(),
            needs_update: false,
            out_reason: old.as_ref().and_then(|o| o.out_reason.clone()),
            keeper_last_seen: None,
        };
        let joined = row.membership == MEMBERSHIP_JOINED;
        db::channels::upsert(&self.env.db, &row).await?;
        // Why I am out follows where I stand: none once in again.
        if state.disbanded {
            db::channels::set_out_reason(&self.env.db, &state.community_id, Some(OUT_DISBANDED)).await?;
        } else if joined {
            db::channels::set_out_reason(&self.env.db, &state.community_id, None).await?;
        }
        db::channels::replace_from_state(&self.env.db, state).await
    }

    /// A note from me to `to` by NIP-59 (wire §7): a row of the outbox
    /// (lane `inbox`, signer `me`), wrapped by the worker at every send
    /// with my keys and sent off this call, again after a failure. To the
    /// keeper or a member (`state`): the channel's relays as the hint,
    /// where it listens, and behind the operations of the channel queued
    /// before it (an admission after the `set_role` that lets me admit).
    /// To a person (`state` none): no hint, so that the pool delivers it
    /// to their own inbox relays as a direct message; one who follows no
    /// channel on the channel's relays would never hear it there.
    /// `ref_id` names the row while it waits (`db::outbox::has_ref`).
    pub(crate) async fn send_note<T: Serialize>(&self, state: Option<&ChannelState>, community_id: &str, to: &PubKey, t: &str, body: &T, ref_id: Option<&str>) -> Result<()> {
        let (hint, after_seq) = match state {
            Some(s) => (s.relays.clone(), db::outbox::last_channel_seq(&self.env.db, community_id).await?),
            None => (Vec::new(), 0),
        };
        crate::outbox::queue_note(&self.env, community_id, Some(to), &hint, t, body, after_seq, ref_id, self.now()).await?;
        Ok(())
    }

    /// The same note to my other devices (my own inbox), by the outbox.
    pub(crate) async fn copy_to_me<T: Serialize>(&self, community_id: &str, t: &str, body: &T) -> Result<()> {
        crate::outbox::queue_note(&self.env, community_id, None, &[], t, body, 0, None, self.now()).await?;
        Ok(())
    }

    // ─── The keeper ─────────────────────────────────────────────────────────

    /// The keeper of a channel kept on this device, loaded once; none when
    /// the journal names a later series than the one kept here (wire §15):
    /// that keeper only hands its state over (`hand_over`), it acts no more.
    pub async fn keeper(&self, community_id: &str) -> Result<Option<Arc<Mutex<Keeper>>>> {
        let log = self.log(community_id).await?;
        let current = log.as_ref().and_then(|l| l.state().keeper().map(|k| k.pubkey.as_hex().to_string()));
        let mut map = self.keepers.lock().await;
        if let Some(k) = map.get(community_id) {
            let mine = self.keeper_pks.lock().ok().and_then(|m| m.get(community_id).cloned());
            if current.is_some() && mine.is_some() && mine != current {
                return Ok(None);
            }
            return Ok(Some(k.clone()));
        }
        let Some(row) = db::keeper::keeper(&self.env.db, community_id).await? else { return Ok(None) };
        let Some(log) = log else { return Ok(None) };
        if current.as_deref() != Some(row.keeper_pubkey.as_str()) {
            return Ok(None);
        }
        let k = Arc::new(Mutex::new(Keeper::load(self.env.clone(), log.state(), self.now()).await?));
        map.insert(community_id.to_string(), k.clone());
        self.remember_keeper(community_id, &row.keeper_pubkey);
        Ok(Some(k))
    }

    /// Notes the key of a keeper put in `keepers`.
    pub(crate) fn remember_keeper(&self, community_id: &str, pubkey: &str) {
        if let Ok(mut m) = self.keeper_pks.lock() {
            m.insert(community_id.to_string(), pubkey.to_string());
        }
    }

    /// A turn of a keeper (held by the caller): an epoch asked for by
    /// `rotate_now` first, then the cycle (no network: what it asks of
    /// the hosts comes back in `Tick::jobs`, for `run_job`), then its
    /// status kept. A pending epoch the cycle superseded (`Tick::burnt`)
    /// cancels the job that still puts its blobs.
    pub(crate) async fn keeper_tick(&self, k: &mut Keeper, log: &OpLog, now: i64) -> Result<crate::keeper::Tick> {
        let cid = k.community_id().to_string();
        let asked = self.rotate_asked.lock().ok().and_then(|mut r| r.remove(&cid)).unwrap_or_default();
        for scope_id in asked {
            k.rotate_now(scope_id.as_deref());
        }
        let tick = k.tick(log, now).await?;
        for (scope_id, epoch) in &tick.burnt {
            let key = crate::keeper::upload::put_key(&cid, scope_id, *epoch);
            if let Some(cancel) = self.uploading.lock().ok().and_then(|u| u.get(&key).cloned()) {
                cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                self.say(&format!("keeper {}: the blobs of epoch {epoch} are no longer wanted, their upload stops", short(&cid)));
            }
        }
        match self.signer() {
            Some(keys) => self.hand_to_me(&keys, k, log, Some(&tick)).await,
            // Nobody at work here: nothing kept for later.
            None => drop(k.take_issued_to_me()),
        }
        let st = k.scope_keeper(SCOPE_EVERYONE);
        let status = KeeperStatus {
            epoch: st.and_then(|s| s.published_head().map(|h| h.epoch)),
            pending_epoch: st.and_then(|s| s.pending_epoch()),
            last_epoch_at: st.and_then(|s| s.last_epoch_at()).and_then(|t| i64::try_from(t).ok()),
            readers: st.map_or(0, |s| u32::try_from(s.members_len()).unwrap_or(u32::MAX)),
            generations: st.map_or(0, |s| u32::try_from(s.generations()).unwrap_or(u32::MAX)),
            garbage: k.garbage().await?,
            rotate_asked: false,
            storage_error: self.troubles.lock().ok().and_then(|t| t.get(&cid).cloned().flatten()),
            at: now,
        };
        if let Ok(mut s) = self.statuses.lock() {
            s.insert(cid, status);
        }
        Ok(tick)
    }

    /// The keys of the jobs of the hosts running now (`Job::key`).
    pub fn jobs_running(&self) -> Vec<String> {
        self.uploading.lock().map(|u| u.keys().cloned().collect()).unwrap_or_default()
    }

    /// The jobs of a turn that no job of the same key runs for already:
    /// those are put in `uploading` and handed back, the rest dropped (the
    /// next turn asks again if they are still wanted).
    pub(crate) fn take_jobs(&self, jobs: Vec<Job>) -> Vec<Job> {
        let Ok(mut running) = self.uploading.lock() else { return Vec::new() };
        jobs.into_iter()
            .filter(|job| {
                let key = job.key();
                if running.contains_key(&key) {
                    return false;
                }
                running.insert(key, job.cancel.clone());
                true
            })
            .collect()
    }

    /// Runs a job of the hosts (`keeper::upload::run`) off every lock,
    /// then takes what it left: the trouble of the hosts for the page and
    /// the terminal, the job out of `uploading`, and a turn of the keeper
    /// asked for (`ask_turn`; the module wakes its keeper task itself).
    /// Never fails: a job that could not run is said, and the next turn
    /// asks for it again.
    pub async fn run_job(&self, job: Job) {
        let key = job.key();
        let cid = job.community_id.clone();
        let done = crate::keeper::upload::run(&self.env, &job).await;
        self.mark(None);
        if let Ok(mut running) = self.uploading.lock() {
            running.remove(&key);
        }
        match done {
            Ok(done) => {
                if let Some(trouble) = done.trouble {
                    self.note_storage(&cid, &key, trouble);
                }
                if done.cancelled {
                    tracing::info!(community = %cid, key, "a job of the hosts stopped: no longer wanted");
                }
            }
            Err(e) => self.say(&format!("keeper {}: a job of the hosts failed, again at the next turn: {e}", short(&cid))),
        }
        self.ask_turn();
    }

    /// `run_job` of each job a turn handed out (`Turned::jobs`, in the
    /// registry already), one after the other: the testkit and the tools.
    pub async fn run_jobs(&self, jobs: Vec<Job>) -> usize {
        let n = jobs.len();
        for job in jobs {
            self.run_job(job).await;
        }
        n
    }

    /// The fetches of the member's side due now (`member::fetch`: an
    /// epoch to open, a shard of a snapshot), marked running, within
    /// `fetch::AT_ONCE`: whoever runs the service runs each with
    /// `run_fetch`, off the loop of the events.
    pub fn take_fetches(&self) -> Vec<FetchJob> {
        self.env.fetches.take(self.now())
    }

    /// A request to the relays an event wanted (`asks`): noted, sent off
    /// the way of the events.
    pub(crate) fn ask_later(&self, out: Outbound) {
        if let Ok(mut asks) = self.asks.lock() {
            // A host that never sends them keeps no more than this: the next
            // round of listening asks again.
            if asks.len() < MAX_ASKS {
                asks.push(out);
            }
        }
    }

    /// The requests to the relays the events wanted since the last call:
    /// whoever runs the service sends them (`send_asks`) off its loop.
    pub fn take_asks(&self) -> Vec<Outbound> {
        self.asks.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default()
    }

    /// Sends requests taken by `take_asks`, one after the other (a `Sync`
    /// is awaited to its end); what they bring comes as events. One that
    /// does not go out is left: the next round of listening asks again.
    /// How many went.
    pub async fn send_asks(&self, asks: Vec<Outbound>) -> usize {
        let n = asks.len();
        for ask in asks {
            if let Err(e) = self.env.net.send(ask).await {
                tracing::debug!(error = %e, "a request the events wanted did not go out");
            }
        }
        n
    }

    /// `send_asks` of every request wanted, until none is (the testkit and
    /// the tools, inline); how many went.
    pub async fn run_asks(&self) -> usize {
        let mut sent = 0;
        loop {
            let asks = self.take_asks();
            if asks.is_empty() {
                return sent;
            }
            sent += self.send_asks(asks).await;
        }
    }

    /// The keys of the fetches wanted now, running or not (`FetchJob::key`).
    pub fn fetches_wanted(&self) -> Vec<String> {
        self.env.fetches.keys()
    }

    /// When the next fetch that waits is due.
    pub fn fetches_wake(&self) -> Option<u64> {
        self.env.fetches.wake()
    }

    /// Runs a fetch of the member's side: what to fetch read from the rows,
    /// the bytes from the hosts with no lock held, then what they open
    /// under the channel's lock only (the key kept, the epochs it opens by
    /// the chain, the content that waited for it read again; a shard put in
    /// the cache and the comments that waited for it read again). One that
    /// failed is tried again at its time (`fetch::RETRY_FIRST_SECS`
    /// doubling) and given up after `fetch::TRIES`; one with nothing to do
    /// any more (the key came another way) ends without a GET. Never fails:
    /// what it read is in the outcome, for the page, with its next time.
    pub async fn run_fetch(&self, keys: &Keys, job: FetchJob) -> Outcome {
        let mut out = Outcome::default();
        let cid = job.community_id.clone();
        let what = match &job.want {
            Want::Epoch { epoch, .. } => format!("epoch {epoch}"),
            Want::Shard { index, .. } => format!("shard {index} of the snapshot"),
        };
        let said = format!("channel {}: {what}", short(&cid));
        let trouble = match self.fetch_and_open(keys, &job, &mut out).await {
            Ok(true) => {
                self.env.fetches.done(&job);
                None
            }
            Ok(false) => Some("not to be had from the hosts now".to_string()),
            Err(e) => Some(e.to_string()),
        };
        self.mark(None);
        if let Some(t) = &trouble {
            if self.env.fetches.failed(&job, self.now()) {
                tracing::debug!(community = %cid, what, trouble = %t, "a fetch failed, again later");
            } else {
                self.env.say.line(&format!("{said}: given up after {} tries, waits for the next key or event: {t}", fetch::TRIES));
            }
        }
        out.wake = [out.wake, self.env.fetches.wake()].into_iter().flatten().min();
        out
    }

    /// `run_fetch` of every fetch due, until none is (the testkit and the
    /// tools, one after the other); how many ran.
    pub async fn run_fetches(&self, keys: &Keys) -> usize {
        let mut ran = 0;
        loop {
            let jobs = self.take_fetches();
            if jobs.is_empty() {
                return ran;
            }
            for job in jobs {
                self.run_fetch(keys, job).await;
                ran += 1;
            }
        }
    }

    /// The work of `run_fetch`: true when settled (opened, or nothing to
    /// fetch any more), false when the hosts did not give it.
    async fn fetch_and_open(&self, keys: &Keys, job: &FetchJob, out: &mut Outcome) -> Result<bool> {
        let cid = &job.community_id;
        let Some(log) = self.log(cid).await? else { return Ok(true) };
        let member = self.member(keys);
        let plan = match &job.want {
            Want::Epoch { scope_id, epoch } => member.plan_epoch(log.state(), scope_id, *epoch).await?,
            Want::Shard { at, index } => member.plan_shard(log.state(), *at, *index).await?,
        };
        let Some(plan) = plan else { return Ok(true) };
        self.mark(Some(format!("channel {}: a fetch from the hosts", short(cid))));
        // No lock held: a host that answers slowly holds nothing else.
        let got = match fetch::download(self.env.blobs.as_ref(), &plan).await {
            Ok(got) => got,
            Err(e) => {
                tracing::debug!(community = %cid, error = %e, "a fetch from the hosts failed");
                return Ok(false);
            }
        };
        // What the bytes open, as an event of the channel would change it.
        let lock = self.lock_of(cid).await;
        let _g = lock.lock().await;
        let Some(log) = self.log(cid).await? else { return Ok(true) };
        match (&job.want, got) {
            (Want::Epoch { scope_id, epoch }, Got::Epoch { index, shard }) => {
                let mut m = crate::member::Outcome::default();
                member.open_fetched(log.state(), scope_id, *epoch, &index, &shard, &mut m).await?;
                if !m.keys.is_empty() {
                    out.note(format!("epoch {epoch} of {}: opened by its shard", short(cid)));
                    self.publish_owed_links(keys).await?;
                    if let Err(e) = self.reseal(keys, &log).await {
                        tracing::warn!(community = %cid, error = %e, "what I wrote under the epoch before was not sealed again");
                    }
                    out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid }));
                }
                self.read_pending(keys, m.pending, out).await?;
            }
            (Want::Shard { at, index }, Got::Shard(bytes)) => {
                let opened = member.open_fetched_shard(cid, *at, *index, &bytes).await?;
                if opened {
                    let waiting = db::channels::take_pending(&self.env.db, cid, crate::member::readers::SNAPSHOT_MARKER).await?;
                    self.read_pending(keys, waiting, out).await?;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// A host refused a blob of a keeper kept here (`Some`: which, and
    /// what it said), or every host took what it was given (`None`): said
    /// on the terminal once a change (живой запуск 2026-10-23: the owner
    /// saw nothing of his keeper), shown in the keeper block of the page.
    /// Said once a change of each job (`job`: its key), not of the keeper:
    /// two jobs refused side by side are two troubles, not one that flips.
    fn note_storage(&self, community_id: &str, job: &str, trouble: Option<String>) {
        self.env.say.state_of(job, &format!("keeper {}: the blob hosts", short(community_id)), trouble.as_deref(), "every host takes the blobs again");
        if let Ok(mut t) = self.troubles.lock() {
            t.insert(community_id.to_string(), trouble.clone());
        }
        if let Ok(mut s) = self.statuses.lock() {
            if let Some(st) = s.get_mut(community_id) {
                st.storage_error = trouble;
            }
        }
    }

    /// What the keeper kept here made for this device's own user, taken by
    /// its member side at once, with no relay and no host: the issues to
    /// me (`Keeper::take_issued_to_me`), the keys of the epochs `tick`
    /// published in the scopes I belong to, and the snapshot it published
    /// (its shards put in the cache of this device by the keeper). The
    /// notes and events still go out (my other devices take them there),
    /// and their echo here is a repeat that changes nothing. Called with
    /// the keeper held: nothing on this way waits for it. A trouble is
    /// said, not returned: the echo of the relay is still the way round.
    pub(crate) async fn hand_to_me(&self, keys: &Keys, k: &mut Keeper, log: &OpLog, tick: Option<&crate::keeper::Tick>) {
        let cid = k.community_id().to_string();
        let me = Self::me(keys);
        let issues: Vec<Issue> = k.take_issued_to_me().into_iter().filter(|(to, _)| me.as_ref().is_ok_and(|me| to == me)).map(|(_, i)| i).collect();
        let epochs: Vec<crate::keeper::PublishedEpoch> = tick
            .map(|t| t.published_keys.iter().filter(|p| me.as_ref().is_ok_and(|me| k.in_scope(me, &p.scope_id))).cloned().collect())
            .unwrap_or_default();
        let snapshot = tick.and_then(|t| t.snapshot_event.clone());
        if issues.is_empty() && epochs.is_empty() && snapshot.is_none() {
            return;
        }
        let keeper = k.pubkey();
        let done = async {
            let me = me?;
            let member = self.member(keys);
            let mut out = Outcome::default();
            for issue in &issues {
                let m = member.on_issue(log, &keeper.clone()?, issue, self.now()).await?;
                db::channels::joining_done(&self.env.db, &cid).await?;
                self.refresh_row(log.state(), &me, Some(MEMBERSHIP_JOINED)).await?;
                self.read_pending(keys, m.pending, &mut out).await?;
            }
            let mut new_keys = false;
            for p in &epochs {
                let signed: serde_json::Value = serde_json::from_str(&p.event_json)?;
                let m = member.take_own_epoch(log, &p.scope_id, &p.key, &signed, self.now()).await?;
                new_keys |= !m.keys.is_empty();
                self.read_pending(keys, m.pending, &mut out).await?;
            }
            // The snapshot after the keys: its `epoch_ref` is the newest of them.
            if let Some(json) = &snapshot {
                let signed: serde_json::Value = serde_json::from_str(json)?;
                out.merge(self.on_snapshot(keys, &signed).await?);
            }
            // As an epoch heard from a relay: what I wrote under the one
            // before, sealed again; the secrets of links owed, out.
            if new_keys {
                if let Err(e) = self.reseal(keys, log).await {
                    tracing::warn!(community = %cid, error = %e, "what I wrote under the epoch before was not sealed again");
                }
            }
            self.publish_owed_links(keys).await
        }
        .await;
        let trouble = done.err().map(|e| format!("not taken here, the relay brings them: {e}"));
        self.env.say.state(&format!("keeper {}: my own keys", short(&cid)), trouble.as_deref(), "taken here again");
    }

    /// Asks the keeper kept here for an epoch now: a mark taken by its
    /// next turn, so the caller never waits for a turn in progress.
    /// `false`: not kept here.
    pub async fn rotate_now(&self, community_id: &str) -> Result<bool> {
        self.rotate_scope_now(community_id, Some(SCOPE_EVERYONE)).await
    }

    /// `rotate_now` of one scope, or of every scope kept here (`None`).
    pub async fn rotate_scope_now(&self, community_id: &str, scope_id: Option<&str>) -> Result<bool> {
        if db::keeper::keeper(&self.env.db, community_id).await?.is_none() {
            return Ok(false);
        }
        if let Ok(mut r) = self.rotate_asked.lock() {
            r.entry(community_id.to_string()).or_default().push(scope_id.map(str::to_string));
        }
        Ok(true)
    }

    /// The keeper of a channel as its last turn left it (none before its
    /// first turn here, or when not kept here); never waits for it.
    pub fn keeper_status(&self, community_id: &str) -> Option<KeeperStatus> {
        let asked = self.rotate_asked.lock().map(|r| r.contains_key(community_id)).unwrap_or(false);
        self.statuses.lock().ok()?.get(community_id).cloned().map(|s| KeeperStatus { rotate_asked: asked, ..s })
    }

    /// Drops the keepers held in memory: the next call loads them from
    /// their rows (a restart of the app, in the tests).
    pub async fn unload_keepers(&self) {
        self.keepers.lock().await.clear();
    }

    /// A turn of the keepers, its jobs and the outbox settled: turns, the
    /// jobs of the hosts run inline, passes of the worker, by turns, until
    /// a round sends nothing and runs no job (at most eight: an epoch is
    /// three turns now, blobs → event queued → published, and a snapshot
    /// after it three more). What one call meant before the outbox and
    /// the jobs (an epoch on the relay, an issue to a reader), for the
    /// tests and the tools; the module runs `turn`, the jobs and
    /// `flush_outbox` each on its own. When to come again.
    pub async fn tick(&self) -> Result<Option<u64>> {
        let mut wake = None;
        for _ in 0..8 {
            let turned = self.turn().await?;
            wake = turned.wake;
            let mut ran = self.run_jobs(turned.jobs).await;
            if let Some(keys) = self.signer() {
                for job in turned.fetches {
                    self.run_fetch(&keys, job).await;
                    ran += 1;
                }
                ran += self.run_fetches(&keys).await;
            }
            // What the events and the snapshot of the turn wanted of the relays.
            self.run_asks().await;
            if self.flush_outbox().await?.sent == 0 && ran == 0 {
                break;
            }
        }
        Ok(wake)
    }

    /// One turn of the cycle of every channel kept here; when to come
    /// again, and the jobs of the hosts the keepers ask for, which the
    /// caller runs off every lock (`run_job`). Each step and each channel
    /// on its own: one that fails (a relay a join cannot reach, a bad row,
    /// a keeper whose hosts are down) is logged and tried again
    /// `RETRY_SECS` later, and holds back no epoch or ban of any other
    /// channel.
    pub async fn turn(&self) -> Result<Turned> {
        let mut jobs: Vec<Job> = Vec::new();
        let mut wake: Option<u64> = None;
        let retry = u64::try_from(self.now()).unwrap_or(0) + crate::keeper::RETRY_SECS;
        let merge = |wake: Option<u64>, at: Option<u64>| match (wake, at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(keys) = self.signer() {
            self.mark(Some("my joins sent again".into()));
            match self.resend_joins().await {
                Ok(w) => {
                    wake = merge(wake, w);
                    self.env.say.state("joins", None, "sent again");
                }
                Err(e) => {
                    self.env.say.state("joins", Some(&format!("not sent again: {e}")), "sent again");
                    wake = merge(wake, Some(retry));
                }
            }
            self.mark(Some("the upload keys given to me".into()));
            if let Err(e) = self.refresh_given_keys(&keys).await {
                self.env.say.state("upload keys", Some(&format!("not read: {e}")), "read again");
            }
            self.mark(Some("the secrets of links".into()));
            match self.publish_owed_links(&keys).await {
                Ok(()) => self.env.say.state("secrets of links", None, "out again"),
                Err(e) => {
                    self.env.say.state("secrets of links", Some(&format!("did not go out: {e}")), "out again");
                    wake = merge(wake, Some(retry));
                }
            }
        }
        let signer = self.signer();
        for cid in db::keeper::kept(&self.env.db).await? {
            // Said on the terminal (живой запуск 2026-10-23: a keeper that
            // failed every turn said it to `tracing` alone, and looked dead).
            let what = format!("keeper {}: the turn", short(&cid));
            match self.tick_one(&cid, signer.as_ref()).await {
                Ok((w, j)) => {
                    wake = merge(wake, w);
                    jobs.extend(j);
                    self.env.say.state(&what, None, "runs again");
                }
                Err(e) => {
                    self.env.say.state(&what, Some(&format!("failed, again in {} s: {e}", crate::keeper::RETRY_SECS)), "runs again");
                    wake = merge(wake, Some(retry));
                }
            }
        }
        self.mark(None);
        // The fetches of the member's side due now (one that failed, at its
        // time): handed out with the jobs, run off every lock by the caller.
        let fetches = if self.signer().is_some() { self.take_fetches() } else { Vec::new() };
        wake = merge(wake, self.env.fetches.wake());
        Ok(Turned { wake, jobs, fetches })
    }

    /// The turn of one keeper: the channel's lock only for what changes
    /// the journal (a handover, the inbox named), dropped before the
    /// keeper's mutex; nothing of the network under either (the cycle
    /// hands out jobs and queues rows). When to come again, and the jobs.
    async fn tick_one(&self, cid: &str, signer: Option<&Keys>) -> Result<(Option<u64>, Vec<Job>)> {
        let step = |what: &str| self.mark(Some(format!("keeper {}: {what}", short(cid))));
        step("waits for the channel's lock");
        let lock = self.lock_of(cid).await;
        let log = {
            let _g = lock.lock().await;
            step("its inbox");
            let Some(mut log) = self.log(cid).await? else { return Ok((None, vec![])) };
            // A keeper the journal has gone past hands its state over and
            // stops (wire §15); one that could not yet tries again on the
            // next turn.
            if self.superseded(cid, &log).await? {
                let retry = u64::try_from(self.now()).unwrap_or(0) + crate::keeper::RETRY_SECS;
                return match self.hand_over(cid).await {
                    Ok(_) => Ok((None, vec![])),
                    Err(e) => {
                        self.env.say.state(&format!("keeper {}: the handover", short(cid)), Some(&format!("not yet, again later: {e}")), "done");
                        Ok((Some(retry), vec![]))
                    }
                };
            }
            if let Some(keys) = signer {
                let what = format!("keeper {}: its inbox", short(cid));
                match self.name_the_inbox(keys, cid, &log).await {
                    Ok(true) => {
                        self.env.say.state(&what, None, "named");
                        let Some(named) = self.log(cid).await? else { return Ok((None, vec![])) };
                        log = named;
                    }
                    Ok(false) => self.env.say.state(&what, None, "named"),
                    Err(e) => self.env.say.state(&what, Some(&format!("not named, again at the next turn: {e}")), "named"),
                }
            }
            log
        };
        let Some(k) = self.keeper(cid).await? else { return Ok((None, vec![])) };
        let now = self.now();
        // A keeper held elsewhere (a command that tells it an operation)
        // is not waited for: its turn comes again in a few seconds.
        let held = format!("keeper {}: held elsewhere", short(cid));
        let Ok(mut k) = k.try_lock() else {
            self.env.say.state(&held, Some(&format!("its turn waits, again every {BUSY_KEEPER_SECS} s")), "its turn runs again");
            return Ok((Some(u64::try_from(now).unwrap_or(0) + BUSY_KEEPER_SECS), vec![]));
        };
        self.env.say.state(&held, None, "its turn runs again");
        // Every operation of the journal the keeper has not acted on (a
        // ban while it was held elsewhere, one applied with its parents
        // late): `on_op` takes each once.
        step("the operations of the journal");
        for o in log.ordered() {
            k.on_op(&log, o, now).await?;
        }
        // The notes that wait first (a catch-up, joins over the minute's
        // room), then the cycle.
        step("the notes that wait");
        let queue = self.drain_keeper(&mut k, &log, now).await?;
        // The cycle waits while operations of the channel are still in the
        // outbox: the epoch event of a new series goes after its `keeper`
        // operation (wire §15), and any epoch after the operations it
        // answers. The worker wakes the turn once they are on the relay.
        let waits = format!("keeper {}: the journal", short(cid));
        if self.outbox_holds_channel(cid).await? {
            self.env.say.state(&waits, Some("operations of the channel wait for the relays; the cycle after them"), "on the relay");
            self.env.outbox.notify_one();
            let again = u64::try_from(now).unwrap_or(0) + BUSY_KEEPER_SECS;
            return Ok((Some(queue.map_or(again, |q| q.min(again))), vec![]));
        }
        self.env.say.state(&waits, None, "on the relay");
        step("the cycle (epochs, snapshots)");
        let mut t = self.keeper_tick(&mut k, &log, now).await?;
        drop(k);
        let jobs = self.take_jobs(std::mem::take(&mut t.jobs));
        let wake = match (queue, t.next_wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Ok((wake, jobs))
    }

    /// A journal from before `inbox` (wire §3) has every note to the keeper
    /// sent to the keeper's own key, which the keeper of the app never
    /// hears: it listens on the owner's connection, and a relay with vguard
    /// serves a `#p` only to the connection signed in as it (живой запуск
    /// 2026-10-23: no join of a channel made that morning reached its
    /// keeper, from a phone or a bot). The owner's device that keeps the
    /// channel names the owner the inbox once, by a `keeper` of the same key
    /// in the next series: the epochs keep their signer and numbers, and the
    /// joiners send to the owner from then on. True when it was written.
    pub(crate) async fn name_the_inbox(&self, keys: &Keys, cid: &str, log: &OpLog) -> Result<bool> {
        let me = Self::me(keys)?;
        let state = log.state();
        let Some(last) = state.keeper() else { return Ok(false) };
        if last.inbox.is_some() || state.owner != me || state.disbanded {
            return Ok(false);
        }
        if self.keeper_pubkey(cid).await?.as_ref() != Some(&last.pubkey) || db::handover::take(&self.env.db, cid).await?.is_some() {
            return Ok(false);
        }
        let serial = last.serial.checked_add(1).ok_or_else(|| invalid("keeper"))?;
        let from_epoch = self.next_series_epoch(cid, last.from_epoch).await?;
        let pubkey = last.pubkey.clone();
        self.act_op(keys, cid, OpBody::Keeper { pubkey, serial, from_epoch, inbox: Some(me.clone()) }).await?;
        self.say(&format!("keeper {}: the journal named no inbox of the keeper; the notes to it go to {} from now (series {serial} from epoch {from_epoch})", short(cid), me.short()));
        Ok(true)
    }

    // ─── Making a channel ───────────────────────────────────────────────────

    /// A new channel: its id, the root secret and the keeper's key (in the
    /// secret store), `create` in the journal, the card, the secret of the
    /// link, the owner let in as the first reader and the first epoch.
    pub async fn create(&self, keys: &Keys, new: NewChannel) -> Result<(String, Outcome)> {
        self.create_naming(keys, new, true).await
    }

    /// `create`; `inbox` false: a journal as it was made before `inbox`
    /// (wire §3), for the tests of such a channel.
    pub(crate) async fn create_naming(&self, keys: &Keys, new: NewChannel, inbox: bool) -> Result<(String, Outcome)> {
        let me = Self::me(keys)?;
        let now = self.now();
        let cid = random_hex(32)?;
        let keeper_keys = Keys::generate();
        let keeper_pk = Self::me(&keeper_keys)?;
        let body = OpBody::Create {
            join: new.join,
            name: new.name.clone(),
            about: new.about.clone(),
            picture: new.picture.clone(),
            history: new.history,
            relays: new.relays.clone(),
            storage: new.storage.clone(),
            scheme: SchemeId::SdV1,
            // The keeper of the app hears its notes on my connection (wire §3, §7).
            keeper: KeeperRef { pubkey: keeper_pk.clone(), serial: 0, from_epoch: 1, inbox: inbox.then(|| me.clone()) },
            link_epoch: 0,
        };
        let op = Op::new(&cid, &me, vec![], now, body);
        let log = OpLog::create(op.clone()).map_err(|e| invalid(&format!("create: {e:?}")))?;
        let signed = wire::sign_op(keys, &op)?;
        // Held to the end: a turn of the loop in the meantime waits for it
        // and then finds this keeper in the map, never a second one loaded
        // from its rows.
        let lock = self.lock_of(&cid).await;
        let _guard = lock.lock().await;
        self.store_op(&cid, &op, &serde_json::to_value(&signed)?).await?;
        let k = Keeper::create(self.env.clone(), log.state(), keeper_keys, RootSecret::generate(), now).await?;
        let k = Arc::new(Mutex::new(k));
        self.keepers.lock().await.insert(cid.clone(), k.clone());
        self.remember_keeper(&cid, keeper_pk.as_hex());
        // The journal and the card go to the relays from the outbox, the
        // journal first: the channel is here at once, pending on its page
        // until a relay takes them (`channel_forget` removes one that never
        // gets there).
        self.enqueue(&cid, db::outbox::LANE_CHANNEL, wire::KIND_JOURNAL, &signed, Some(&op.id().0)).await?;
        self.enqueue(&cid, db::outbox::LANE_REPLACEABLE, wire::KIND_CARD, &wire::sign_card(keys, &self.card_of(log.state(), now, 0))?, None).await?;
        secrets::put_link(self.env.secrets.as_ref(), &cid, 0, &LinkSecret::generate()?).await?;
        // The secret of the first link goes into the feed by my hand once
        // a key of the channel is here: the keeper's issue to me brings it.
        db::channels::owe_link(&self.env.db, &cid, 0, 0).await?;
        self.refresh_row(log.state(), &me, Some(MEMBERSHIP_JOINED)).await?;
        // The first reader; my bundle here at once, not by the echo of my
        // own issue (queued behind the journal, for my other devices). The
        // first epoch is the keeper's turn, asked for now: its key comes to
        // me by that turn (`keeper_tick`, `hand_to_me`).
        {
            let mut k = k.lock().await;
            k.admit_reader(&log, &me, now).await?;
            self.hand_to_me(keys, &mut k, &log, None).await;
        }
        self.ask_turn();
        let mut out = Outcome { resubscribe: true, wake: Some(u64::try_from(now).unwrap_or(0)), pending: vec![op.id().0], ..Outcome::default() };
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": cid }));
        Ok((cid, out))
    }

    /// The card of a channel as its owner publishes it; `members`: the
    /// readers as the keeper's latest snapshot counted them
    /// (`members_known`), what the one who looks from outside is told.
    pub(crate) fn card_of(&self, state: &ChannelState, now: i64, members: u64) -> Card {
        Card {
            v: wire::CARD_VERSION,
            community_id: state.community_id.clone(),
            name: state.name.clone(),
            about: state.about.clone(),
            picture: state.picture.clone(),
            members,
            join: state.join,
            code: String::new(),
            relays: state.relays.clone(),
            storage: state.storage.clone(),
            created_at: now,
        }
    }

    /// The readers of a channel as the keeper's latest snapshot heard here
    /// counted them (0 before the first): what the card says of them.
    pub(crate) async fn members_known(&self, community_id: &str) -> Result<u64> {
        Ok(db::channels::get(&self.env.db, community_id).await?.map_or(0, |r| u64::try_from(r.members).unwrap_or(0)))
    }

    /// The readers the keeper's latest snapshot held here counts; none
    /// before the first snapshot (then the row keeps what a card said).
    pub(crate) async fn snapshot_members(&self, community_id: &str) -> Result<Option<u64>> {
        match db::snapshots::latest(&self.env.db, community_id).await? {
            Some((_, json)) => Ok(crate::snapshot::content_of(&json).ok().map(|s| s.members)),
            None => Ok(None),
        }
    }

    // ─── The journal ────────────────────────────────────────────────────────

    /// An operation of mine (kind 1391): refused here when the journal
    /// would refuse it (`permits` and the rules of `apply`), else signed,
    /// applied, queued for the relays (`outbox`) and told to the keeper
    /// kept here, whose turn is asked for now.
    pub async fn act(&self, keys: &Keys, community_id: &str, body: OpBody) -> Result<Outcome> {
        self.act_op(keys, community_id, body).await.map(|(_, out)| out)
    }

    /// `act`, with the operation made.
    pub(crate) async fn act_op(&self, keys: &Keys, community_id: &str, body: OpBody) -> Result<(Op, Outcome)> {
        let me = Self::me(keys)?;
        // A keeper of the app hears its notes on my connection: unless the
        // operation names another inbox (a bot keeper, stage 3), mine.
        let body = match body {
            OpBody::Keeper { pubkey, serial, from_epoch, inbox: None } => OpBody::Keeper { pubkey, serial, from_epoch, inbox: Some(me.clone()) },
            other => other,
        };
        let now = self.now();
        let mut log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let op = log.next(&me, now.max(Self::head_at(&log, now)), body);
        log.check(&op).map_err(|e| invalid(&format!("{e:?}")))?;
        // The author of `rotate_link` hands its secret on in the feed: one
        // without a key of the channel would leave a link nobody can use.
        if matches!(op.body, OpBody::RotateLink { .. }) && self.member(keys).newest_key(community_id, SCOPE_EVERYONE).await?.is_none() {
            return Err(invalid("no key of the channel to hand the new link on"));
        }
        let signed = wire::sign_op(keys, &op)?;
        self.store_op(community_id, &op, &serde_json::to_value(&signed)?).await?;
        if !matches!(log.insert(op.clone()), Insert::Applied) {
            return Err(invalid("the journal did not take my operation"));
        }
        // Queued before the keeper hears of it: a note it answers with (the
        // issue of a scope the operation made) follows it in the outbox,
        // and a reader hears the operation first (wire §14.5).
        self.enqueue(community_id, db::outbox::LANE_CHANNEL, wire::KIND_JOURNAL, &signed, Some(&op.id().0)).await?;
        let mut out = Outcome { pending: vec![op.id().0], ..Outcome::default() };
        self.refresh_row(log.state(), &me, None).await?;
        if let Some(k) = self.keeper(community_id).await? {
            k.lock().await.on_op(&log, &op, now).await?;
        }
        match &op.body {
            OpBody::RotateLink { link_epoch } => {
                // The author of `rotate_link` hands the new secret to every
                // member in the feed (wire §10).
                let secret = LinkSecret::generate()?;
                secrets::put_link(self.env.secrets.as_ref(), community_id, *link_epoch, &secret).await?;
                let link = LinkPayload { link_epoch: *link_epoch, secret: secret.encode() };
                if let Some(k) = self.keeper(community_id).await? {
                    k.lock().await.take_link(&link).await?;
                }
                // Sealed under the key held now, the secret would be read by
                // a reader put out a moment ago, still inside the window of
                // the revoke: it waits for the next epoch, which the keeper
                // builds at once for a `rotate_link` (`Keeper::on_op`).
                let held = self.member(keys).newest_key(community_id, SCOPE_EVERYONE).await?.map_or(0, |k| k.epoch);
                db::channels::owe_link(&self.env.db, community_id, *link_epoch, held).await?;
                self.publish_owed_links(keys).await?;
            }
            OpBody::Settings { .. } if log.state().owner == me => {
                let card = wire::sign_card(keys, &self.card_of(log.state(), now, self.members_known(community_id).await?))?;
                self.enqueue(community_id, db::outbox::LANE_REPLACEABLE, wire::KIND_CARD, &card, None).await?;
            }
            _ => {}
        }
        // The keeper's turn (an epoch for a ban, the sections put in step)
        // is the loop's, now: not run here under the channel's lock.
        if self.keeper(community_id).await?.is_some() {
            out.wake = Some(u64::try_from(now).unwrap_or(0));
            self.ask_turn();
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id }));
        Ok((op, out))
    }

    /// The secrets of links I made and have not put in the feed yet, each
    /// signed by me and sealed under the newest key I hold (wire §10);
    /// one whose channel I hold no key of (newer than the epoch it waits
    /// past) waits for it. A link epoch the journal says is not mine (or no
    /// longer the current one) is dropped.
    pub async fn publish_owed_links(&self, keys: &Keys) -> Result<()> {
        let me = Self::me(keys)?;
        for (cid, epoch, after) in db::channels::links_owed(&self.env.db).await? {
            // One channel's trouble (no journal yet, a relay down) holds
            // back no other's link.
            if let Err(e) = self.publish_owed_link(keys, &me, &cid, epoch, after).await {
                tracing::warn!(community = %cid, error = %e, "the secret of the link did not go out: kept for later");
            }
        }
        // The names of levels and titles of scopes wait for a key the same
        // way, and go out wherever the secrets of links do.
        self.publish_owed_metas(keys).await
    }

    async fn publish_owed_link(&self, keys: &Keys, me: &PubKey, cid: &str, epoch: i64, after: i64) -> Result<()> {
        let Ok(link_epoch) = u32::try_from(epoch) else { return Ok(()) };
        let Some(log) = self.log(cid).await? else { return Ok(()) };
        if log.link_author(link_epoch) != Some(me) || link_epoch != log.state().link_epoch {
            return db::channels::link_paid(&self.env.db, cid, epoch).await;
        }
        let Some(key) = self.member(keys).newest_key(cid, SCOPE_EVERYONE).await?.filter(|k| i64::from(k.epoch) > after) else { return Ok(()) };
        let Some(secret) = secrets::links(self.env.secrets.as_ref(), cid).await?.get(&link_epoch).cloned() else {
            return db::channels::link_paid(&self.env.db, cid, epoch).await;
        };
        let link = LinkPayload { link_epoch, secret: secret.encode() };
        let now = self.now();
        let inner = wire::sign_link(keys, cid, &link, now)?;
        let event = wire::seal_content(&Place::feed(cid), &key, None, &Payload::event(wire::T_LINK, &inner)?, now)?;
        self.enqueue(cid, db::outbox::LANE_CHANNEL, wire::KIND_CONTENT, &event, Some(&inner.id.to_hex())).await?;
        db::channels::link_paid(&self.env.db, cid, epoch).await
    }

    /// A `channel.join` to the keeper, kept to be sent again while no
    /// issue comes (wire §7: the keeper may be asleep, and a note lost).
    /// The row of the outbox is named `join:<channel>` while it waits.
    pub(crate) async fn send_join(&self, state: &ChannelState, keeper: &PubKey, join: &Join) -> Result<()> {
        let cid = &join.community_id;
        self.send_note(Some(state), cid, keeper, wire::T_JOIN, join, Some(&join_ref(cid))).await?;
        self.say(&format!("{} to {} for {}: queued", wire::T_JOIN, keeper.short(), short(cid)));
        let now = self.now();
        db::channels::put_joining(&self.env.db, cid, &serde_json::to_string(join)?, now, now + JOIN_RETRY_SECS).await
    }

    /// The joins whose time came, sent again with a fresh date, each time
    /// twice as late; given up after the week a note lives. One still in
    /// the outbox (no relay took it yet) is not queued twice: the worker
    /// sends it again by itself. When to come back.
    async fn resend_joins(&self) -> Result<Option<u64>> {
        let now = self.now();
        let mut wake: Option<u64> = None;
        for row in db::channels::joining(&self.env.db).await? {
            if now - row.first_at > wire::NOTE_MAX_AGE_SECS {
                db::channels::joining_done(&self.env.db, &row.community_id).await?;
                continue;
            }
            let mut next = row.next_at;
            if row.next_at <= now {
                // A row that does not read is dropped; a join that does not
                // go out is tried again at its next time: neither stops the
                // others, nor the keepers' turn after them.
                let Ok(mut join) = serde_json::from_str::<Join>(&row.join_json) else {
                    tracing::warn!(community = %row.community_id, "a join that does not read: dropped");
                    db::channels::joining_done(&self.env.db, &row.community_id).await?;
                    continue;
                };
                let Some(log) = self.log(&row.community_id).await? else { continue };
                let Some(keeper) = log.state().keeper().map(|k| k.inbox().clone()) else { continue };
                if db::outbox::has_ref(&self.env.db, &row.community_id, &join_ref(&row.community_id)).await? {
                    self.say(&format!("{} to the keeper of {}: still in the outbox, not queued again", wire::T_JOIN, short(&row.community_id)));
                } else {
                    join.created_at = now;
                    match self.send_note(Some(log.state()), &row.community_id, &keeper, wire::T_JOIN, &join, Some(&join_ref(&row.community_id))).await {
                        Ok(()) => self.say(&format!("{} to {} for {}: queued again", wire::T_JOIN, keeper.short(), short(&row.community_id))),
                        Err(e) => self.say(&format!("{} to the keeper of {}: not queued again, later: {e}", wire::T_JOIN, short(&row.community_id))),
                    }
                }
                let shift = u32::try_from(row.attempts.clamp(0, 16)).unwrap_or(16);
                next = now + JOIN_RETRY_SECS.saturating_mul(1i64 << shift).min(JOIN_RETRY_MAX_SECS);
                db::channels::joining_sent(&self.env.db, &row.community_id, next).await?;
            }
            let at = u64::try_from(next).unwrap_or(0);
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        Ok(wake)
    }

    // ─── Content ────────────────────────────────────────────────────────────

    /// What the journal and the keys say of me: whether I may write
    /// `content` now. Readers are not in the journal; until the snapshot
    /// of sub-stage 1.3 a member who holds a bundle counts as a reader
    /// since the channel came here.
    pub(crate) async fn may_write(&self, keys: &Keys, state: &ChannelState, content: Content) -> Result<()> {
        let me = Self::me(keys)?;
        let reader = self.reader_of(state).await?;
        state.can_write_at(&me, reader.as_ref(), content, self.now()).map_err(|c| invalid(&format!("{c:?}")))
    }

    async fn reader_of(&self, state: &ChannelState) -> Result<Option<Reader>> {
        let row = db::channels::get(&self.env.db, &state.community_id).await?;
        Ok(row.filter(|r| r.membership == MEMBERSHIP_JOINED).map(|r| Reader { since: r.created_at }))
    }

    /// Seals an envelope of mine into the feed of `everyone` (outer 1390,
    /// inner 1394, wire §4) under the newest key I hold, queues it for the
    /// relays and takes it in here as it would come back (the echo of the
    /// relay is then one more copy, kept once by the inner id).
    pub(crate) async fn seal_and_send(&self, keys: &Keys, community_id: &str, links: Links, envelope: Envelope, t: &str) -> Result<(String, Outcome)> {
        self.seal_and_send_in(keys, community_id, SCOPE_EVERYONE, links, envelope, t).await
    }

    /// `seal_and_send` into the feed of a scope (wire §14.3), under the
    /// newest key of that scope I hold.
    pub(crate) async fn seal_and_send_in(&self, keys: &Keys, community_id: &str, scope_id: &str, links: Links, envelope: Envelope, t: &str) -> Result<(String, Outcome)> {
        let key = self.member(keys).newest_key(community_id, scope_id).await?.ok_or_else(|| {
            if scope_id == SCOPE_EVERYONE {
                invalid("no key of the channel yet")
            } else {
                invalid("no key of the section yet")
            }
        })?;
        let place = Place::feed_in(community_id, scope_id);
        let now = self.now();
        let inner = wire::sign_inner(keys, &place, &links, &envelope.encode(), now)?;
        let payload = Payload::event(t, &inner)?;
        let outer = wire::seal_content(&place, &key, links.outer_target(), &payload, now)?;
        let id = inner.id.to_hex();
        self.enqueue(community_id, db::outbox::LANE_CHANNEL, wire::KIND_CONTENT, &outer, Some(&id)).await?;
        let mut out = self.on_event(keys, &serde_json::to_value(&outer)?, true).await?;
        out.pending.push(id.clone());
        Ok((id, out))
    }

    /// The scope a new post goes into (none: `everyone`): one of the
    /// journal that I belong to (wire §14.1: moderators and above belong
    /// to every scope).
    pub(crate) fn scope_to_write(state: &ChannelState, me: &PubKey, scope_id: Option<&str>) -> Result<String> {
        let scope_id = scope_id.unwrap_or(SCOPE_EVERYONE);
        if !state.scopes.contains_key(scope_id) {
            return Err(invalid("unknown scope"));
        }
        if !state.in_scope(me, scope_id) {
            return Err(invalid("not member of scope"));
        }
        Ok(scope_id.to_string())
    }

    /// The scope of a message here (a comment, a reaction, an edit, a
    /// delete go where their target is); `everyone` when it is not here.
    async fn scope_of_message(&self, id: &str) -> Result<String> {
        Ok(db::content::get(&self.env.db, id).await?.map_or_else(|| SCOPE_EVERYONE.to_string(), |r| r.scope_id))
    }

    /// A post in the feed: by `post_mode` (everyone, or authors and above).
    /// `fmt`: the words are Markdown, as a chat's text says it (the
    /// messenger's `fmt` of the envelope; the wire of a channel carries
    /// the envelope as it is).
    pub async fn send_post(&self, keys: &Keys, community_id: &str, text: &str, fmt: TextFormat) -> Result<(String, Outcome)> {
        self.send_post_in(keys, community_id, None, text, fmt).await
    }

    /// A post in the feed of a section (none: `everyone`), sealed with the
    /// key of its scope (wire §14.3).
    pub async fn send_post_in(&self, keys: &Keys, community_id: &str, scope_id: Option<&str>, text: &str, fmt: TextFormat) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        let scope_id = Self::scope_to_write(&state, &me, scope_id)?;
        self.may_write(keys, &state, Content::Post).await?;
        // A reader posts by a level only into the scopes of the levels that
        // let post (2026-10-23); everybody else would drop it.
        if !state.post_scopes_of(&me).contains(&scope_id) {
            return Err(invalid("no post in this scope"));
        }
        self.seal_and_send_in(keys, community_id, &scope_id, Links::default(), Envelope::text(text).with_format(fmt), wire::T_MSG).await
    }

    /// A comment under a post (`parent`: a branch under a comment), in the
    /// scope of the post.
    pub async fn send_comment(&self, keys: &Keys, community_id: &str, post_id: &str, parent: Option<&str>, text: &str, fmt: TextFormat) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let scope_id = Self::scope_to_write(&state, &Self::me(keys)?, Some(&self.scope_of_message(post_id).await?))?;
        self.may_write(keys, &state, Content::Comment).await?;
        let links = Links { thread: Some(post_id.to_string()), parent: parent.map(str::to_string), target: None };
        self.seal_and_send_in(keys, community_id, &scope_id, links, Envelope::text(text).with_format(fmt), wire::T_MSG).await
    }

    /// A reaction to a post or a comment (`e` on the outside: wire §4).
    /// «I am writing a comment under `post_id`»: sealed under the key of
    /// the post's section like a comment, under the ephemeral kind
    /// (`wire::KIND_TYPING`), sent to the relays at once, never through
    /// the outbox (a «typing» that waited is nothing), and kept nowhere.
    /// A bot's word (2026-10-24); the caller says it again every few
    /// seconds while it writes.
    pub async fn send_typing(&self, keys: &Keys, community_id: &str, post_id: &str) -> Result<()> {
        let scope_id = self.scope_of_message(post_id).await?;
        let key = self.member(keys).newest_key(community_id, &scope_id).await?.ok_or_else(|| invalid("no key of the section yet"))?;
        let place = Place::feed_in(community_id, &scope_id);
        let now = self.now();
        let links = Links { thread: Some(post_id.to_string()), parent: None, target: None };
        let inner = wire::sign_inner(keys, &place, &links, &Envelope::typing(messenger_core::envelope::TYPING_TTL_SECS).encode(), now)?;
        let payload = Payload::event(wire::T_MSG, &inner)?;
        let outer = wire::seal_content_kind(&place, &key, links.outer_target(), &payload, now, wire::KIND_TYPING)?;
        let id = messenger_core::EventId::parse(&outer.id.to_hex()).ok_or_else(|| invalid("event id"))?;
        let event = messenger_core::outbound::WireEvent { id, json: serde_json::to_value(&outer)? };
        self.env.net.send(Outbound::PublishScoped { scope: Scope::Channel { id: community_id.to_string() }, event }).await?;
        Ok(())
    }

    pub async fn send_reaction(&self, keys: &Keys, community_id: &str, target: &str, emoji: &str, set: bool) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let scope_id = Self::scope_to_write(&state, &Self::me(keys)?, Some(&self.scope_of_message(target).await?))?;
        self.may_write(keys, &state, Content::Comment).await?;
        let links = Links { target: Some(target.to_string()), ..Links::default() };
        self.seal_and_send_in(keys, community_id, &scope_id, links, Envelope::reaction(target, emoji, set), wire::T_MSG).await
    }

    /// A new text of my message, in `fmt` (an edit may turn Markdown on or off).
    pub async fn send_edit(&self, keys: &Keys, community_id: &str, target: &str, text: &str, fmt: TextFormat) -> Result<(String, Outcome)> {
        let row = db::content::get(&self.env.db, target).await?.ok_or_else(|| invalid("unknown message"))?;
        if row.author != Self::me(keys)?.as_hex() {
            return Err(invalid("only the author edits"));
        }
        let links = Links { target: Some(target.to_string()), ..Links::default() };
        self.seal_and_send_in(keys, community_id, &row.scope_id, links, Envelope::edit(target, text).with_format(fmt), wire::T_MSG).await
    }

    /// A message removed: my own, or one of a lower role as a moderator.
    pub async fn send_delete(&self, keys: &Keys, community_id: &str, target: &str) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let row = db::content::get(&self.env.db, target).await?.ok_or_else(|| invalid("unknown message"))?;
        let author = PubKey::parse(&row.author).ok_or_else(|| invalid("author"))?;
        let reader = self.reader_of(&state).await?;
        if !state.can_delete_message(&Self::me(keys)?, reader.as_ref(), &author) {
            return Err(invalid("not permitted"));
        }
        let links = Links { target: Some(target.to_string()), ..Links::default() };
        self.seal_and_send_in(keys, community_id, &row.scope_id, links, Envelope::delete(target), wire::T_MSG).await
    }

    /// Pins or unpins a post: an operation of the journal (moderator+).
    pub async fn pin(&self, keys: &Keys, community_id: &str, post_id: &str, pinned: bool) -> Result<Outcome> {
        self.act(keys, community_id, OpBody::Pin { post_id: post_id.to_string(), pinned }).await
    }

    /// A note of moderation (`cmod.*`, sealed as `t:"mod"`): a moderator
    /// and above, or the author of a post about the comments under it.
    pub async fn send_mod(&self, keys: &Keys, community_id: &str, note: &ModNote) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        let moderator = state.role_of(&me).is_some_and(Role::is_moderator);
        if !moderator {
            let mut own_thread = !note.targets().is_empty();
            for t in note.targets() {
                let Some(row) = db::content::get(&self.env.db, t).await? else { return Err(invalid("unknown message")) };
                let post = db::content::get(&self.env.db, &row.post_id).await?;
                own_thread &= !row.is_post() && post.is_some_and(|p| p.author == me.as_hex());
            }
            if !own_thread {
                return Err(invalid("not permitted"));
            }
        }
        self.seal_and_send(keys, community_id, Links::default(), note.envelope(), wire::T_MOD).await
    }

    pub(crate) async fn state(&self, community_id: &str) -> Result<ChannelState> {
        Ok(self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?.state().clone())
    }

    // ─── Getting in ─────────────────────────────────────────────────────────

    /// The link of a channel to hand on (wire §10): the relay, the owner,
    /// and the secret of the current link epoch when I hold it.
    pub async fn share_link(&self, community_id: &str) -> Result<String> {
        let state = self.state(community_id).await?;
        let relay = state.relays.first().and_then(|r| RelayUrl::parse(r)).ok_or_else(|| invalid("no relay"))?;
        let held = secrets::links(self.env.secrets.as_ref(), community_id).await?;
        let link = ChannelLink {
            community_id: community_id.to_string(),
            relay,
            owner: state.owner.clone(),
            secret: held.get(&state.link_epoch).cloned(),
            link_epoch: state.link_epoch,
            invite_id: None,
        };
        Ok(link.encode())
    }

    /// Opens a link of a channel whose journal is here (the relay of the
    /// link gave it by `#h`): by the secret, `channel.join` with a proof
    /// to the keeper; without one, a request to the moderators (`request`)
    /// or nothing (`invite`).
    pub async fn open_link(&self, keys: &Keys, link: &str, note: &str) -> Result<Outcome> {
        let me = Self::me(keys)?;
        let link = ChannelLink::parse(link)?;
        self.expect_owner(&link.community_id, &link.owner).await?;
        let log = self.log(&link.community_id).await?.ok_or_else(|| invalid("the journal of the channel is not here yet"))?;
        let state = log.state();
        if state.owner != link.owner {
            return Err(invalid("the link names another owner"));
        }
        let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?.inbox().clone();
        let mut out = Outcome { resubscribe: true, ..Outcome::default() };
        match (&link.secret, state.join) {
            (Some(secret), JoinMode::Open) => {
                secrets::put_link(self.env.secrets.as_ref(), &link.community_id, link.link_epoch, secret).await?;
                let join = Join {
                    community_id: link.community_id.clone(),
                    proof: Some(JoinProof { link_epoch: link.link_epoch, mac: secret.join_mac(&link.community_id, &me, link.link_epoch) }),
                    admit_id: None,
                    created_at: self.now(),
                };
                self.send_join(state, &keeper, &join).await?;
                self.refresh_row(state, &me, Some(MEMBERSHIP_JOINING)).await?;
            }
            (_, JoinMode::Request) => {
                let request = Request { community_id: link.community_id.clone(), note: note.to_string(), created_at: self.now() };
                request.check(self.now()).map_err(invalid)?;
                for m in state.moderators() {
                    self.send_note(Some(state), &link.community_id, &m, wire::T_REQUEST, &request, None).await?;
                }
                let row = db::channels::RequestRow {
                    community_id: link.community_id.clone(),
                    requester: me.as_hex().to_string(),
                    direction: "out".into(),
                    status: "pending".into(),
                    payload_json: serde_json::to_string(&request)?,
                    created_at: self.now(),
                    updated_at: self.now(),
                };
                db::channels::put_request(&self.env.db, &row).await?;
                self.refresh_row(state, &me, Some(MEMBERSHIP_REQUESTED)).await?;
            }
            (None, JoinMode::Open) => return Err(invalid("a link without its secret opens the card only")),
            // The link of a channel I am invited to is the invitation taken
            // (a bot writes «here is the link» to the one it invited, and the
            // link is what they open; 2026-10-24).
            (_, JoinMode::Invite) => {
                let invited = db::channels::invites(&self.env.db, "in", &["received"]).await?.into_iter().find(|i| i.community_id == link.community_id);
                return match invited {
                    Some(i) => self.answer_invite(&i.invite_id, true).await,
                    None => Err(invalid("this channel takes invitations only")),
                };
            }
        }
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": link.community_id }));
        Ok(out)
    }

    /// An admission of mine as a moderator (wire §7.1): to the keeper,
    /// and a copy to the person, who then asks the keeper in with it.
    pub(crate) async fn admit(&self, keys: &Keys, state: &ChannelState, who: &PubKey, invite_id: Option<String>) -> Result<Admit> {
        let me = Self::me(keys)?;
        if !state.role_of(&me).is_some_and(|r| permits(r, Action::Admit)) {
            return Err(invalid("not permitted"));
        }
        let mut admit = Admit {
            community_id: state.community_id.clone(),
            who: who.clone(),
            admit_id: random_hex(16)?,
            levels: 0,
            invite_id,
            expires_at: self.now() + wire::INVITE_TTL_SECS,
            ticket_id: None,
        };
        // The keeper away: the newcomer reads by a ticket of mine at once (wire §17).
        if let Err(e) = self.ticket_for(state, &mut admit).await {
            tracing::info!(community = %state.community_id, error = %e, "no ticket handed over");
        }
        let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?.inbox().clone();
        self.send_note(Some(state), &state.community_id, &keeper, wire::T_ADMIT, &admit, None).await?;
        self.send_note(None, &state.community_id, who, wire::T_ADMIT, &admit, None).await?;
        Ok(admit)
    }

    /// A request answered yes: an admission.
    pub async fn approve_request(&self, keys: &Keys, community_id: &str, requester: &PubKey) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        self.admit(keys, &state, requester, None).await?;
        self.answered_request(community_id, requester, "approved").await
    }

    /// A request answered no: `channel.rejected` to the person.
    pub async fn reject_request(&self, keys: &Keys, community_id: &str, requester: &PubKey) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if !state.role_of(&me).is_some_and(|r| permits(r, Action::Admit)) {
            return Err(invalid("not permitted"));
        }
        let body = Rejected { community_id: community_id.to_string(), reason: None };
        self.send_note(None, community_id, requester, wire::T_REJECTED, &body, None).await?;
        self.answered_request(community_id, requester, "rejected").await
    }

    async fn answered_request(&self, community_id: &str, requester: &PubKey, status: &str) -> Result<Outcome> {
        if let Some(mut r) = db::channels::request(&self.env.db, community_id, requester.as_hex(), "in").await? {
            r.status = status.to_string();
            db::channels::put_request(&self.env.db, &r).await?;
        }
        let mut out = Outcome::default();
        out.event(UI_EVENT_CHANNEL_REQUEST, serde_json::json!({ "community_id": community_id }));
        Ok(out)
    }

    /// What the keeper said, if it refused me a bundle: `NEVER_LET_IN` until
    /// a bundle of the channel is here (an invitation let me in).
    pub async fn refused_by_keeper(&self, community_id: &str) -> Result<Option<String>> {
        let said = self.not_let_in.lock().map(|r| r.contains(community_id)).unwrap_or(false);
        if !said {
            return Ok(None);
        }
        if !db::keys::bundles(&self.env.db, community_id, SCOPE_EVERYONE).await?.is_empty() {
            if let Ok(mut r) = self.not_let_in.lock() {
                r.remove(community_id);
            }
            return Ok(None);
        }
        Ok(Some(NEVER_LET_IN.to_string()))
    }

    /// Why an invitation is not made: the person is in the channel already.
    pub const ALREADY_MEMBER: &str = "already a member";

    /// An invitation of mine as a moderator (wire §7, the limits of
    /// groups): to the person and a copy to my devices.
    pub async fn invite(&self, keys: &Keys, community_id: &str, who: &PubKey) -> Result<(String, Outcome)> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if !state.role_of(&me).is_some_and(|r| permits(r, Action::Invite)) {
            return Err(invalid("not permitted"));
        }
        if state.is_banned(who) {
            return Err(invalid("banned"));
        }
        // One in the channel already needs no invitation: refused as such,
        // not as the cap (live run 2026-10-30: bots restored on empty folders
        // were invited again and met «too many open invitations»). In, for an
        // invitation, is what the keeper let in: a reader the journal did not
        // put out since. A role in the journal is not: staff named before any
        // invitation (a migration made agent bots authors first) hold no
        // reader row and no key until invited and admitted, their role kept.
        // Refused only where that is known now, the keeper's own rows (a
        // leave takes them out at once); a snapshot may name one who left
        // after it, and only frees the place of their invitation under the cap.
        let log = self.log(community_id).await?.ok_or_else(|| invalid("unknown channel"))?;
        let member = self.member(keys);
        let kept = member.kept_here(&state).await?;
        let readers = member.known_readers(&log).await?;
        let reader_in = |p: &PubKey| {
            readers.readers.iter().find(|(r, _)| r == p.as_hex()).is_some_and(|(_, since)| rules_at(&log, p, Some(&Reader { since: *since }), i64::MAX).standing != crate::state::Standing::Outsider)
        };
        if kept && reader_in(who) {
            return Err(invalid(Self::ALREADY_MEMBER));
        }
        let is_member = |hex: &str| PubKey::parse(hex).is_some_and(|p| reader_in(&p));
        let now = self.now();
        db::channels::expire_invites(&self.env.db, now).await?;
        // An invitation accepted by one who is in now is done, whatever its row
        // says: it holds no place under the cap.
        let open: Vec<_> = db::channels::invites(&self.env.db, "out", &["sent", "accepted"]).await?.into_iter().filter(|i| i.status == "sent" || !is_member(&i.peer)).collect();
        let to_channel = open.iter().filter(|i| i.community_id == community_id).count();
        let to_person = open.iter().filter(|i| i.peer == who.as_hex()).count();
        if open.len() >= MAX_OPEN_INVITES || to_channel >= MAX_INVITES_PER_CHANNEL || to_person >= MAX_INVITES_PER_PERSON {
            return Err(invalid("too many open invitations"));
        }
        let relay = state.relays.first().cloned().unwrap_or_default();
        let invite = Invite {
            invite_id: random_hex(16)?,
            community_id: community_id.to_string(),
            name: state.name.clone(),
            about: state.about.clone(),
            picture: state.picture.clone(),
            relay,
            members: 0,
            levels: 0,
            created_at: now,
            expires_at: now + wire::INVITE_TTL_SECS,
            owner: Some(state.owner.clone()),
        };
        invite.check(now).map_err(invalid)?;
        let row = db::channels::InviteRow {
            invite_id: invite.invite_id.clone(),
            community_id: community_id.to_string(),
            direction: "out".into(),
            peer: who.as_hex().to_string(),
            status: "sent".into(),
            payload_json: serde_json::to_string(&invite)?,
            created_at: now,
            expires_at: invite.expires_at,
            updated_at: now,
            verified: true,
        };
        db::channels::insert_invite(&self.env.db, &row).await?;
        self.send_note(None, community_id, who, wire::T_INVITE, &invite, None).await?;
        self.copy_to_me(community_id, wire::T_INVITE, &invite).await?;
        let mut out = Outcome::default();
        out.event(UI_EVENT_CHANNEL_INVITE, serde_json::json!({ "community_id": community_id, "invite_id": invite.invite_id }));
        Ok((invite.invite_id, out))
    }

    /// My answer to an invitation that came: to the moderator who made it.
    pub async fn answer_invite(&self, invite_id: &str, accept: bool) -> Result<Outcome> {
        let row = db::channels::invite(&self.env.db, invite_id).await?.filter(|i| i.direction == "in").ok_or_else(|| invalid("unknown invitation"))?;
        if row.expires_at <= self.now() {
            return Err(invalid("the invitation expired"));
        }
        let inviter = PubKey::parse(&row.peer).ok_or_else(|| invalid("inviter"))?;
        if accept {
            // The journal fetched for it is taken only under the create of
            // the owner the invitation names (wire §3).
            if let Some(owner) = serde_json::from_str::<Invite>(&row.payload_json).ok().and_then(|i| i.owner) {
                self.expect_owner(&row.community_id, &owner).await?;
            }
        }
        let reply = InviteReply { invite_id: invite_id.to_string(), community_id: row.community_id.clone(), accept };
        self.send_note(None, &row.community_id, &inviter, wire::T_INVITE_REPLY, &reply, None).await?;
        db::channels::set_invite_status(&self.env.db, invite_id, if accept { "accepted" } else { "declined" }).await?;
        let mut out = Outcome { resubscribe: accept, ..Outcome::default() };
        out.event(UI_EVENT_CHANNEL_INVITE, serde_json::json!({ "community_id": row.community_id, "invite_id": invite_id }));
        Ok(out)
    }

    // ─── Getting out ────────────────────────────────────────────────────────

    /// I leave: `channel.leave` to the keeper; the owner does not leave.
    pub async fn leave(&self, keys: &Keys, community_id: &str) -> Result<Outcome> {
        let state = self.state(community_id).await?;
        let me = Self::me(keys)?;
        if state.owner == me {
            return Err(invalid("the owner does not leave"));
        }
        let keeper = state.keeper().ok_or_else(|| invalid("no keeper"))?.inbox().clone();
        self.send_note(Some(&state), community_id, &keeper, wire::T_LEAVE, &Leave { community_id: community_id.to_string() }, None).await?;
        db::channels::set_membership(&self.env.db, community_id, MEMBERSHIP_LEFT).await?;
        db::channels::set_out_reason(&self.env.db, community_id, Some(OUT_LEFT)).await?;
        let mut out = Outcome { resubscribe: true, ..Outcome::default() };
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id }));
        Ok(out)
    }

    /// Forgets a channel on this device. A channel I keep is forgotten
    /// only once disbanded: its keeper would be lost with it.
    pub async fn forget(&self, community_id: &str) -> Result<Outcome> {
        if db::keeper::keeper(&self.env.db, community_id).await?.is_some() {
            let disbanded = self.log(community_id).await?.is_some_and(|l| l.state().disbanded);
            if !disbanded {
                return Err(invalid("this device keeps the channel"));
            }
            db::keeper::delete_keeper(&self.env.db, community_id).await?;
            self.keepers.lock().await.remove(community_id);
            if let Ok(mut s) = self.statuses.lock() {
                s.remove(community_id);
            }
        }
        db::channels::delete(&self.env.db, community_id).await?;
        self.drop_log(community_id).await;
        self.env.fetches.forget_channel(community_id);
        secrets::forget(self.env.secrets.as_ref(), community_id).await?;
        let mut out = Outcome { resubscribe: true, ..Outcome::default() };
        out.event(UI_EVENT_CHANNEL_UPDATED, serde_json::json!({ "community_id": community_id }));
        Ok(out)
    }

    /// A device of mine that holds no bundle of a channel I am in asks the
    /// keeper for one (wire §6.4: no note of my own came).
    pub async fn ensure_keys(&self, keys: &Keys, community_id: &str) -> Result<bool> {
        let state = self.state(community_id).await?;
        let member = self.member(keys);
        if member.has_bundle(community_id, SCOPE_EVERYONE).await? {
            return Ok(false);
        }
        member.ask_rebundle(&state, SCOPE_EVERYONE, 1, self.now()).await?;
        Ok(true)
    }

    /// The channels this device keeps.
    pub async fn kept(&self) -> Result<Vec<String>> {
        db::keeper::kept(&self.env.db).await
    }

    /// The pubkey of the keeper of a channel kept here.
    /// Read without the keeper's lock: the rounds of listening and the
    /// wraps asked it each time, and a turn held it long then.
    pub async fn keeper_pubkey(&self, community_id: &str) -> Result<Option<PubKey>> {
        let Some(k) = self.keeper(community_id).await? else { return Ok(None) };
        let known = self.keeper_pks.lock().ok().and_then(|m| m.get(community_id).and_then(|p| PubKey::parse(p)));
        match known {
            Some(pk) => Ok(Some(pk)),
            None => Ok(Some(k.lock().await.pubkey()?)),
        }
    }

}

/// The ids of gift wraps known to hold nothing of the channels, the
/// oldest forgotten past [`ForeignWraps::MAX`].
#[derive(Default)]
pub(crate) struct ForeignWraps {
    ids: std::collections::HashSet<String>,
    order: std::collections::VecDeque<String>,
}

impl ForeignWraps {
    pub(crate) const MAX: usize = 50_000;

    pub(crate) fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    #[cfg(test)]
    pub(crate) fn ids_len(&self) -> usize {
        self.ids.len()
    }

    /// Puts `id` in; whether it was not there before.
    pub(crate) fn first(&mut self, id: &str) -> bool {
        if self.ids.contains(id) {
            return false;
        }
        self.insert(id);
        true
    }

    pub(crate) fn insert(&mut self, id: &str) {
        if id.is_empty() || !self.ids.insert(id.to_string()) {
            return;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > Self::MAX {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
    }
}

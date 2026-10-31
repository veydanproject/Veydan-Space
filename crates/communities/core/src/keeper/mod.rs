// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper of one channel (internal/communities-spec.md §8,
//! internal/communities-wire.md §5, §7, §9, §10), as it runs inside the
//! app on the device where the channel was made, on any platform, while
//! the app runs: nothing here assumes it is woken in the background.
//!
//! - Keys: the keeper's own key pair (not the owner's) and the owner's
//!   root secret live in the secret store (`secrets`); the master of a
//!   generation is derived from the root whenever the state is put
//!   together and never written to the database.
//! - State: keydist's `TreeKeeper<SdScheme>` per scope (stage 1: one,
//!   `everyone`), saved row by row from its change log (`db::keeper`)
//!   before anything a call returned is sent, put back together by
//!   `replay` at start, and recovered as an emergency generation when the
//!   rows do not add up.
//! - Readers: let in on a checked `channel.join` (a link proof by the
//!   current link epoch, or an admission of a moderator by the journal),
//!   put out on `ban`/`remove` of the journal and on `channel.leave`.
//! - The cycle (`cycle`): an epoch when one is due, all its blobs on the
//!   hosts first, its event after; the secret of the link in the feed.

mod cycle;
mod handover;
mod scope;
mod snapshot;
mod tickets;
mod upload_keys;
mod cloud_keys;
pub use cloud_keys::RETRY_SECS as CLOUD_RETRY_SECS;
pub mod upload;

pub use cycle::{PublishedEpoch, Tick, RETRY_SECS};
pub use upload::{Job, Work, What};
pub(crate) use handover::Recovery;
pub use tickets::ticket_member;
pub use scope::ScopeKeeper;
pub use snapshot::{SnapshotTurn, LEVELS_STAGE1, SETTLE_SECS, SMALL_CHANNEL_READERS};

use crate::blobs::BlobHosts;
use crate::db::{self, keeper as rows, Db};
use crate::log::OpLog;
use crate::op::{JoinMode, Op, OpBody, OpId, MAX_HEADS};
use crate::member::readers::PutOut;
use crate::roles::{permits, Action, Role};
use crate::secrets;
use crate::state::{ChannelState, FLAT_MAX_MEMBERS, SCOPE_EVERYONE};
use crate::wire::{self, Admit, Issue, Join, KeeperMaster, LinkPayload, Missed, Rebundle, Rejected};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use keydist::{
    Change, FlatKeeper, FlatState, GenerationMaster, MemberId, Policy, Revoke, RootSecret, SchemeId, SdKeeper, TreeParams,
};
use messenger_core::{MessengerError, PubKey, RelayUrl, Result, SecretStore, Transport};
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

/// The shape of the trees of stage 1 (keydist phase 1: `sd-v1`, depth
/// 24, shards of 1024 leaves).
pub const TREE: TreeParams = TreeParams { depth: 24, leaves_per_shard: 1024 };
/// How long the keeper takes the link of the previous link epoch after a
/// `rotate_link` whose secret has not come (wire §10).
pub const LINK_GRACE_SECS: i64 = 300;
/// How long blobs of an epoch stay under `from_join` (wire §9: the
/// owner's retention, 30 days by default).
pub const FROM_JOIN_RETENTION_SECS: i64 = 30 * 24 * 3600;
/// How many hosts hold every blob before an epoch event, a snapshot, a
/// file or a cover goes out (wire §9), at most the hosts there are: one.
/// The others get the blobs when they answer again (`catch_up_hosts`).
/// Two held a channel's keys back whenever one host was down (owner,
/// 2026-10-23: eu-1 down → a new channel had no key and no cover).
pub const MIN_HOSTS: usize = 1;
/// How long a keeper may go unheard of (no epoch, no snapshot, no sign of
/// life of wire §18) before a moderator who admits somebody hands a ticket
/// over himself (wire §17).
pub const KEEPER_AWAY_SECS: i64 = 300;
/// Joins a keeper holds for admissions that have not come (wire §7.1).
pub const MAX_HELD_JOINS: i64 = 1000;
/// The most readers a channel holds: what one snapshot names (64 shards
/// of 1024, wire §8); the index blob that would take more is stage 4.
pub const MAX_READERS: usize = crate::snapshot::MAX_SHARDS * crate::snapshot::SHARD_ENTRIES;
/// New readers a keeper lets in a minute: Sybil joins through an open
/// link wait (the joiner sends again), they do not flood the keeper.
pub const JOINS_PER_MINUTE: usize = 120;
/// The first wait before a row of the outbox that did not go out (a note
/// of the keeper, an operation, a post) is sent again; each next one twice
/// as long, up to `OWED_MAX_WAIT_SECS`: well before the reader's own next
/// join (`JOIN_RETRY_SECS`, ten minutes).
pub const OWED_FIRST_WAIT_SECS: i64 = 15;
pub const OWED_MAX_WAIT_SECS: i64 = 600;
/// How often one never let in is told so again (`channel.not_let_in`).
pub const NOT_LET_IN_EVERY: i64 = 600;

/// The bounds of a keeper (fields, so that tests can lower them).
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_readers: usize,
    pub joins_per_minute: usize,
    /// Below this many readers a snapshot follows a change of the readers
    /// within `SETTLE_SECS` (`SMALL_CHANNEL_READERS`).
    pub small_channel: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_readers: MAX_READERS, joins_per_minute: JOINS_PER_MINUTE, small_channel: SMALL_CHANNEL_READERS }
    }
}

/// What a keeper needs of the app: its database, the secret store, the
/// relays and the blob hosts.
#[derive(Clone)]
pub struct Env {
    pub db: Db,
    pub secrets: Arc<dyn SecretStore>,
    pub net: Arc<dyn Transport>,
    pub blobs: Arc<dyn BlobHosts>,
    /// The user at work on this device (`ChannelService::signer`): whose
    /// account the blobs of a keeper kept here are put on the hosts with.
    pub payer: Arc<std::sync::RwLock<Option<Keys>>>,
    /// The lines of the way of a note to the keeper, for the log of the app.
    pub say: Say,
    /// Woken whenever a row is put into the outbox (`db::outbox`): the
    /// worker that sends them (`ChannelService::flush_outbox`) waits on it.
    pub outbox: Arc<tokio::sync::Notify>,
    /// What the member's side of this device wants of the hosts (an epoch
    /// to open, a shard of a snapshot): noted on the way of an event, run
    /// off it (`ChannelService::run_fetch`).
    pub fetches: crate::member::fetch::Fetches,
    /// The keys this device has for the hosts of a server of the kit, and
    /// what each host needs (wire §9.1).
    pub book: Arc<crate::upload_keys::Book>,
    /// The time of the service (an upload key lives so long).
    pub clock: Arc<dyn messenger_core::Clock>,
    /// The owner's Veydan Cloud as the app lends it (wire §9.2): where the
    /// keeper registers the keys of the staff; none, nothing is registered.
    pub cloud: Arc<std::sync::RwLock<Option<Arc<dyn crate::cloud_keys::CloudDelegates>>>>,
}

impl Env {
    /// The Cloud lent to the keepers, if any.
    pub fn cloud(&self) -> Option<Arc<dyn crate::cloud_keys::CloudDelegates>> {
        self.cloud.read().ok().and_then(|c| c.clone())
    }
}

/// One line for whoever reads the log of the app.
pub type SayFn = Arc<dyn Fn(&str) + Send + Sync>;

tokio::task_local! {
    /// The step of the task running now, when it keeps one of its own.
    static STEP: Steps;
}

/// The step a task is at (the module's loop of the events, the keepers'
/// turn), for the watch that says a piece of work that runs too long:
/// `Say::mark` within `run` writes here, not into the one step every
/// other task shares (живой запуск 2026-10-23: «an event of kind 1059
/// holds the loop for 56 s» named no step, as the keepers' turn, the jobs
/// and the fetches had marked and cleared the shared one meanwhile).
#[derive(Clone, Default)]
pub struct Steps(Arc<std::sync::Mutex<Option<String>>>);

impl Steps {
    /// `work`, its marks into this step.
    pub async fn run<F: std::future::Future>(&self, work: F) -> F::Output {
        STEP.scope(self.clone(), work).await
    }

    /// The step marked last.
    pub fn now(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set(&self, what: Option<String>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = what;
    }
}

/// Where the lines of the way of a join go (живой запуск 2026-10-23: the
/// owner reads the terminal of the app, which `tracing` never reaches): the
/// host's own log once it set one (`ChannelService::set_say`), `tracing`
/// before that.
#[derive(Clone, Default)]
pub struct Say {
    to: Arc<std::sync::RwLock<Option<SayFn>>>,
    /// The trouble said last of each thing (`state`): a turn that fails
    /// the same way every minute says it once.
    said: Arc<std::sync::Mutex<BTreeMap<String, String>>>,
    /// The step of work that runs now in a task that keeps no step of its
    /// own (`Steps`: a job of the hosts, a fetch, a command), shared by
    /// all of them (`ChannelService::doing`).
    doing: Arc<std::sync::Mutex<Option<String>>>,
}

impl Say {
    pub fn doing(&self) -> Option<String> {
        self.doing.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The step of work that runs now: into the step of the task it runs
    /// in when that task keeps one of its own (`Steps::run`: the module's
    /// loop, the keepers' turn), into the one every other task shares
    /// otherwise.
    pub fn mark(&self, what: Option<String>) {
        let mut what = Some(what);
        let _ = STEP.try_with(|s| s.set(what.take().unwrap_or_default()));
        if let Some(what) = what {
            *self.doing.lock().unwrap_or_else(|e| e.into_inner()) = what;
        }
    }

    pub fn set(&self, to: Option<SayFn>) {
        *self.to.write().unwrap_or_else(|e| e.into_inner()) = to;
    }

    pub fn line(&self, line: &str) {
        let to = self.to.read().unwrap_or_else(|e| e.into_inner()).clone();
        match to {
            Some(say) => say(line),
            None => tracing::info!("{line}"),
        }
    }

    /// The state of one thing (`what`, its line's start): a trouble said
    /// once until it changes, its end said once (`fine`), nothing while it
    /// stays as it is.
    pub fn state(&self, what: &str, trouble: Option<&str>, fine: &str) {
        self.state_of(what, what, trouble, fine);
    }

    /// `state` of one of several things said under the same start (`key`
    /// tells them apart): the jobs of the hosts of a keeper, a blob of an
    /// epoch and the snapshot, run side by side and fail apart, and one
    /// remembered trouble for both said each of them anew at every turn.
    pub fn state_of(&self, key: &str, what: &str, trouble: Option<&str>, fine: &str) {
        let line = {
            let mut said = self.said.lock().unwrap_or_else(|e| e.into_inner());
            match trouble {
                Some(t) if said.get(key).map(String::as_str) != Some(t) => {
                    said.insert(key.to_string(), t.to_string());
                    Some(format!("{what}: {t}"))
                }
                None if said.remove(key).is_some() => Some(format!("{what}: {fine}")),
                _ => None,
            }
        };
        if let Some(line) = line {
            self.line(&line);
        }
    }
}

impl Env {
    /// The key the blobs of a keeper go up with: the user's, whose account
    /// the hosts of the channel know and charge (the shards and the indexes
    /// of the epochs count in the owner's limit, communities-spec §13; in
    /// stage 1 the keeper is the owner's device), and the keeper's own
    /// only while no user is at work. A host bound to accounts refuses a
    /// key it has no account of (vstore §10.2, `unknown = "refuse"`), and
    /// the keeper's key never has one.
    pub fn uploader(&self, keeper: &Keys) -> Keys {
        self.payer.read().ok().and_then(|p| p.clone()).unwrap_or_else(|| keeper.clone())
    }

    /// Removes a blob put by `uploader`, or by the keeper's own key before
    /// (a host lets only the one who put a blob remove it).
    pub async fn delete_blob(&self, keeper: &Keys, host: &str, sha256: &str) -> Result<()> {
        let by = self.uploader(keeper);
        let key = self.host_key(&by, host, None);
        match self.blobs.delete_keyed(&by, key.as_deref(), host, sha256).await {
            Err(_) if by.public_key() != keeper.public_key() => {
                let key = self.host_key(keeper, host, None);
                self.blobs.delete_keyed(keeper, key.as_deref(), host, sha256).await
            }
            done => done,
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now().secs()
    }

    /// The key a write by `signer` to `host` carries (wire §9.1): the app
    /// key of the card of its server, else an upload key given to the
    /// signer (of `community_id` first); `None`: none here.
    pub fn host_key(&self, signer: &Keys, host: &str, community_id: Option<&str>) -> Option<String> {
        self.book.key_for(host, &signer.public_key().to_hex(), community_id, self.now())
    }

    /// What `host` needs of a writer: what it said lately, else asked now.
    /// A host that does not answer is taken as open for a minute: a write to
    /// it fails as a write to a host that is down.
    pub async fn gate(&self, host: &str) -> crate::upload_keys::Gate {
        use crate::upload_keys::{Gate, GATE_SECS, GATE_UNKNOWN_SECS};
        let now = self.now();
        if let Some(g) = self.book.gate(host, now) {
            return g;
        }
        let (gate, until) = match self.blobs.gate(host).await {
            Some(g) => (g, now + GATE_SECS),
            None => (Gate::Open, now + GATE_UNKNOWN_SECS),
        };
        self.book.set_gate(host, gate, until);
        gate
    }

    /// The hosts of `hosts` a write by `signer` may go to, in their order,
    /// each with the key it carries: a host that needs the key of its server
    /// (wire §9.1) only when this device has it. A reader never has one, so
    /// nothing of his goes to an owner's own server.
    pub async fn writable(&self, signer: &Keys, hosts: &[String], community_id: Option<&str>) -> Vec<(String, Option<String>)> {
        let gates = futures_util::future::join_all(hosts.iter().map(|h| async move {
            match self.host_key(signer, h, community_id) {
                Some(key) => {
                    // The owner's device learns whether the host takes upload
                    // keys, which its keeper then makes for the staff.
                    if self.book.app_key(h).is_some() {
                        self.gate(h).await;
                    }
                    Some(Some(key))
                }
                None if self.gate(h).await == crate::upload_keys::Gate::Open => Some(None),
                None => None,
            }
        }))
        .await;
        let mut out = Vec::new();
        for (h, g) in hosts.iter().zip(gates) {
            match g {
                Some(key) => out.push((h.clone(), key)),
                None => tracing::info!(host = %h, "a host that needs the key of its server, which this device has not: not written to"),
            }
        }
        out
    }
}

/// What became of a `channel.join`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Joined {
    /// In, and `channel.issue` sent.
    Issued,
    /// Too many new readers this minute: nothing sent, the joiner sends
    /// again (`JOIN_RETRY_SECS`).
    Busy,
    /// Waits for the admission it names, which has not come yet (wire
    /// §7.1): nothing is sent; it goes on when the admission comes.
    Held,
    /// Refused, and `channel.rejected` sent (why).
    Refused(&'static str),
}


fn invalid(what: &str) -> MessengerError {
    MessengerError::Invalid(format!("channel keeper: {what}"))
}

fn kd(e: keydist::Error) -> MessengerError {
    MessengerError::Invalid(format!("keydist: {e}"))
}

/// keydist's member id of a person: the 32 bytes of the public key.
pub fn member_id(who: &PubKey) -> Result<MemberId> {
    let raw: [u8; 32] = hex::decode(who.as_hex()).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("public key"))?;
    Ok(MemberId(raw))
}

fn secs(now: i64) -> u64 {
    now.max(0) as u64
}

/// The policy of `everyone` by the journal: open by the link or not, the
/// history of the channel.
pub fn policy(state: &ChannelState) -> Policy {
    let mut p = if state.join == JoinMode::Open { Policy::open() } else { Policy::closed() };
    p.history = state.history.keydist();
    p
}

/// The policy of a scope by the journal: `everyone` as the channel is;
/// a section is closed (only levels let in, and a revoke goes at the ban
/// window), with the history it was made with (wire §14.1).
pub fn policy_for(state: &ChannelState, scope_id: &str) -> Policy {
    if scope_id == SCOPE_EVERYONE {
        return policy(state);
    }
    let mut p = Policy::closed();
    if let Some(def) = state.scopes.get(scope_id) {
        p.history = def.history.keydist();
    }
    p
}

/// The heads of the journal a `channel.issue` names: all of them, or the
/// newest `MAX_HEADS`.
pub fn issue_heads(log: &OpLog) -> Vec<OpId> {
    let mut heads: Vec<(i64, OpId)> = log.heads().into_iter().map(|id| (log.get(&id).map_or(0, |o| o.created_at), id)).collect();
    heads.sort_by(|a, b| b.cmp(a));
    let mut out: Vec<OpId> = heads.into_iter().take(MAX_HEADS).map(|(_, id)| id).collect();
    out.sort();
    out
}

/// The keeper of one channel.
pub struct Keeper {
    env: Env,
    community_id: String,
    keys: Keys,
    root: Option<RootSecret>,
    scopes: BTreeMap<String, ScopeKeeper>,
    /// Scopes an urgent epoch was asked for (`rotate_now`).
    urgent: BTreeSet<String>,
    /// Who belongs to which scope may differ from the journal (the keeper
    /// was loaded, or the levels or the scopes changed): every reader is
    /// put in step on the next turn.
    members_dirty: bool,
    /// When the newest readers were let in: joins past `limits` in a
    /// minute wait for the joiner to send again.
    recent_joins: VecDeque<i64>,
    /// The mark of the readers (`readers_mark`) and the digest of their
    /// entries when they were last read: a turn whose mark is the same
    /// reads no row of a million readers again.
    snapshot_memo: Option<(u64, String)>,
    /// When a turn first saw the readers move from those of `snapshot_memo`
    /// (none: they have not since): a small channel's snapshot is due
    /// `SETTLE_SECS` later.
    readers_moved: Option<i64>,
    pub(crate) limits: Limits,
    /// When this keeper last said it runs (wire §18), in this run of the
    /// app: the first turn after a start says it at once.
    heartbeat_at: Option<i64>,
    /// The snapshot built and sealed, on its way to the hosts and the
    /// relays (`snapshot`).
    snapshot_pending: Option<snapshot::PendingSnapshot>,
    /// The operations of the journal acted on in this run (`on_op`): the
    /// rows (`mark_op`) are asked once for each.
    ops_done: std::collections::HashSet<String>,
    /// When each key the journal names but never let in was last told so
    /// (`channel.not_let_in`, once in `NOT_LET_IN_EVERY`).
    not_let_in_told: std::collections::HashMap<String, i64>,
    /// The issues sent to the user at work on this device (or the owner):
    /// the service hands them to its own member side at once
    /// (`take_issued_to_me`), not by the echo of the relay.
    to_me: Vec<(PubKey, Issue)>,
    /// The signed snapshot published in this turn, for `Tick::snapshot_event`.
    snapshot_out: Option<String>,
    /// The pending epoch of `everyone` built in this run of the app, with
    /// `revoked_mark` at its build: whom it leaves out.
    built_mark: Option<(u32, u64)>,
    /// `revoked_mark` at the build of the newest epoch of `everyone`
    /// published in this run (none: not known here, after a restart).
    published_mark: Option<u64>,
    /// The work of the last job of the keys of the staff in the owner's
    /// Cloud and when it was handed out (wire §9.2): the same work is not
    /// asked again for `cloud_keys::RETRY_SECS`.
    cloud_asked: Option<(i64, String)>,
}

impl Keeper {
    /// Starts keeping a new channel: the keeper's key and the root secret
    /// go to the secret store, the first generation of `everyone` is
    /// derived from the root, and the empty state is written.
    pub async fn create(env: Env, state: &ChannelState, keys: Keys, root: RootSecret, now: i64) -> Result<Self> {
        let cid = state.community_id.clone();
        secrets::put_keeper_keys(env.secrets.as_ref(), &cid, &keys).await?;
        secrets::put_root(env.secrets.as_ref(), &cid, &root).await?;
        let scope = wire::scope_ref(&cid, SCOPE_EVERYONE)?;
        let master = GenerationMaster::derive(&root, &scope, SchemeId::SdV1, 1);
        let sd = SdKeeper::new(scope, policy(state), TREE, master, secs(now)).map_err(kd)?;
        rows::put_keeper(&env.db, &cid, &keys.public_key().to_hex(), now).await?;
        let mut k = Keeper {
            env,
            community_id: cid,
            keys,
            root: Some(root),
            scopes: BTreeMap::from([(SCOPE_EVERYONE.to_string(), ScopeKeeper::Sd(sd))]),
            urgent: BTreeSet::new(),
            members_dirty: false,
            recent_joins: VecDeque::new(),
            snapshot_memo: None,
            readers_moved: None,
            limits: Limits::default(),
            heartbeat_at: None,
            cloud_asked: None,
            snapshot_pending: None,
            ops_done: Default::default(),
            not_let_in_told: Default::default(),
            to_me: Vec::new(),
            snapshot_out: None,
            built_mark: None,
            published_mark: None,
        };
        k.save(SCOPE_EVERYONE).await?;
        Ok(k)
    }

    /// The keeper of a channel kept here, put together from its rows; a
    /// state the rows do not make is recovered as an emergency generation
    /// above everything they named (keydist `recover`): the members keep
    /// their place and get new bundles when they ask.
    pub async fn load(env: Env, state: &ChannelState, now: i64) -> Result<Self> {
        let cid = state.community_id.clone();
        if rows::keeper(&env.db, &cid).await?.is_none() {
            return Err(invalid("this channel is not kept here"));
        }
        let keys = secrets::keeper_keys(env.secrets.as_ref(), &cid).await?.ok_or_else(|| MessengerError::SecretMissing(secrets::keeper_name(&cid)))?;
        let root = secrets::root(env.secrets.as_ref(), &cid).await?;
        let mut k = Keeper {
            env,
            community_id: cid,
            keys,
            root,
            scopes: BTreeMap::new(),
            urgent: BTreeSet::new(),
            members_dirty: true,
            recent_joins: VecDeque::new(),
            snapshot_memo: None,
            readers_moved: None,
            limits: Limits::default(),
            heartbeat_at: None,
            cloud_asked: None,
            snapshot_pending: None,
            ops_done: Default::default(),
            not_let_in_told: Default::default(),
            to_me: Vec::new(),
            snapshot_out: None,
            built_mark: None,
            published_mark: None,
        };
        let sd = k.load_scope(state, SCOPE_EVERYONE, now).await?;
        k.scopes.insert(SCOPE_EVERYONE.to_string(), ScopeKeeper::Sd(sd));
        k.sync_scopes(state, now).await?;
        Ok(k)
    }

    /// The scopes of the journal kept here, each from its rows or new; a
    /// scope the journal deleted goes with its rows (wire §14.2).
    pub(crate) async fn sync_scopes(&mut self, state: &ChannelState, now: i64) -> Result<()> {
        let cid = self.community_id.clone();
        let gone: Vec<String> = self.scopes.keys().filter(|s| !state.scopes.contains_key(*s)).cloned().collect();
        for scope_id in gone {
            self.scopes.remove(&scope_id);
            rows::clear_state(&self.env.db, &cid, &scope_id).await?;
            rows::delete_flat(&self.env.db, &cid, &scope_id).await?;
            tracing::info!(community = %cid, scope = %scope_id, "keeper let a deleted scope go");
        }
        for (scope_id, def) in &state.scopes {
            if self.scopes.contains_key(scope_id) {
                continue;
            }
            let k = match def.scheme {
                SchemeId::FlatV1 => {
                    let k = match rows::flat_state(&self.env.db, &cid, scope_id).await? {
                        Some(json) => FlatKeeper::from_state(serde_json::from_str::<FlatState>(&json)?).map_err(kd)?,
                        None => FlatKeeper::new(wire::scope_ref(&cid, scope_id)?, policy_for(state, scope_id)),
                    };
                    ScopeKeeper::Flat(k)
                }
                _ => {
                    if rows::has_state(&self.env.db, &cid, scope_id).await? {
                        ScopeKeeper::Sd(self.load_scope(state, scope_id, now).await?)
                    } else {
                        let root = self.root.clone().ok_or_else(|| MessengerError::SecretMissing(secrets::root_name(&cid)))?;
                        let scope = wire::scope_ref(&cid, scope_id)?;
                        let master = GenerationMaster::derive(&root, &scope, SchemeId::SdV1, 1);
                        ScopeKeeper::Sd(SdKeeper::new(scope, policy_for(state, scope_id), TREE, master, secs(now)).map_err(kd)?)
                    }
                }
            };
            self.scopes.insert(scope_id.clone(), k);
            self.save(scope_id).await?;
            self.members_dirty = true;
            tracing::info!(community = %cid, scope = %scope_id, "keeper keeps a scope");
        }
        Ok(())
    }

    async fn masters(&mut self, scope_id: &str, upto: u32) -> Result<BTreeMap<u32, GenerationMaster>> {
        let scope = wire::scope_ref(&self.community_id, scope_id)?;
        let mut out = BTreeMap::new();
        // At most two generations live; one more for a margin. A master
        // handed over (`keeper.master`, or with the state of a former
        // keeper, wire §15) first: a new owner's root does not derive the
        // generations of the one before.
        for gen in upto.saturating_sub(2).max(1)..=upto {
            let m = match secrets::master(self.env.secrets.as_ref(), &self.community_id, scope_id, gen).await? {
                Some(m) => Some(m),
                None => self.root.as_ref().map(|root| GenerationMaster::derive(root, &scope, SchemeId::SdV1, gen)),
            };
            if let Some(m) = m {
                out.insert(gen, m);
            }
        }
        Ok(out)
    }

    async fn load_scope(&mut self, state: &ChannelState, scope_id: &str, now: i64) -> Result<SdKeeper> {
        let (gen, _) = rows::high_marks(&self.env.db, &self.community_id, scope_id).await?;
        let masters = self.masters(scope_id, gen.max(1)).await?;
        let read = rows::read_changes(&self.env.db, &self.community_id, scope_id, &masters).await;
        let rebuilt = match read {
            Ok(Some(changes)) => keydist::KeeperState::replay(changes).and_then(SdKeeper::from_state).map_err(kd),
            Ok(None) => Err(invalid("no rows")),
            Err(e) => Err(e),
        };
        match rebuilt {
            Ok(sd) => Ok(sd),
            Err(e) => {
                tracing::error!(community = %self.community_id, scope = scope_id, error = %e, "keeper rows do not make a state: recovering");
                self.recover(state, scope_id, now).await
            }
        }
    }

    /// An emergency generation above the generations and epochs the rows
    /// and the epochs this device took name; every member stays and is
    /// handed a new bundle on `channel.rebundle`.
    async fn recover(&mut self, state: &ChannelState, scope_id: &str, now: i64) -> Result<SdKeeper> {
        let cid = self.community_id.clone();
        let root = self.root.clone().ok_or_else(|| MessengerError::SecretMissing(secrets::root_name(&cid)))?;
        let (gen, mut epoch) = rows::high_marks(&self.env.db, &cid, scope_id).await?;
        for row in db::keys::epochs(&self.env.db, &cid, scope_id).await? {
            epoch = epoch.max(u32::try_from(row.epoch).unwrap_or(0));
        }
        let members = rows::members(&self.env.db, &cid, scope_id).await?;
        let scope = wire::scope_ref(&cid, scope_id)?;
        let master = GenerationMaster::derive(&root, &scope, SchemeId::SdV1, gen.saturating_add(1));
        let (mut sd, _) = SdKeeper::recover(scope, policy_for(state, scope_id), TREE, &members, gen, epoch, master, secs(now)).map_err(kd)?;
        rows::clear_state(&self.env.db, &cid, scope_id).await?;
        let changes = sd.take_changes();
        rows::write_changes(&self.env.db, &cid, scope_id, &changes).await?;
        Ok(sd)
    }

    /// A trouble of the turn (an epoch event no relay took, notes owed that
    /// did not go out), said once a change.
    pub(crate) fn note_turn(&self, what: &str, trouble: Option<String>, fine: &str) {
        self.env.say.state(&self.line_of(what), trouble.as_deref(), fine);
    }

    /// The start of a line of this keeper on the terminal.
    fn line_of(&self, what: &str) -> String {
        format!("keeper {}: {what}", crate::service::short(&self.community_id))
    }

    pub fn community_id(&self) -> &str {
        &self.community_id
    }

    /// The keeper's public key: the one the journal's `keeper` names.
    pub fn pubkey(&self) -> Result<PubKey> {
        PubKey::parse(&self.keys.public_key().to_hex()).ok_or_else(|| invalid("keeper key"))
    }

    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    /// keydist's tree keeper of a scope (none for a flat one).
    pub fn scope(&self, scope_id: &str) -> Option<&SdKeeper> {
        self.scopes.get(scope_id).and_then(ScopeKeeper::sd)
    }

    /// keydist's keeper of a scope, whatever its scheme.
    pub fn scope_keeper(&self, scope_id: &str) -> Option<&ScopeKeeper> {
        self.scopes.get(scope_id)
    }

    /// The scopes kept here.
    pub fn scope_ids(&self) -> Vec<String> {
        self.scopes.keys().cloned().collect()
    }

    fn sk(&mut self, scope_id: &str) -> Result<&mut ScopeKeeper> {
        self.scopes.get_mut(scope_id).ok_or_else(|| invalid("unknown scope"))
    }

    /// The settings the journal holds now (wire §6.6): `join` makes the
    /// scope open or closed, `history` open or from the join. keydist keeps
    /// its policy in its state, so the state is put together again with
    /// the new one and its header written.
    async fn sync_policy(&mut self, state: &ChannelState) -> Result<()> {
        let scopes: Vec<String> = self.scopes.keys().cloned().collect();
        for scope_id in scopes {
            let want = policy_for(state, &scope_id);
            let have = self.sk(&scope_id)?.policy();
            if have.open_scope == want.open_scope && have.history == want.history {
                continue;
            }
            self.save(&scope_id).await?;
            let Some(k) = self.scopes.remove(&scope_id) else { continue };
            let k = k.with_policy(&want).map_err(kd)?;
            let header = k.sd().map(|sd| sd.state().header());
            self.scopes.insert(scope_id.clone(), k);
            match header {
                Some(h) => rows::write_changes(&self.env.db, &self.community_id, &scope_id, &[Change::Header(h)]).await?,
                None => self.save(&scope_id).await?,
            }
            tracing::info!(community = %self.community_id, scope = %scope_id, open = want.open_scope, "keeper took the settings of the journal");
        }
        Ok(())
    }

    /// Writes what changed in a scope; called after every change of the
    /// state and before anything is sent: a tree row by row, a flat scope
    /// whole.
    async fn save(&mut self, scope_id: &str) -> Result<()> {
        let k = self.sk(scope_id)?;
        let changes: Vec<Change> = k.take_changes();
        let flat = k.flat_json().transpose()?;
        if let Some(json) = flat {
            return rows::put_flat(&self.env.db, &self.community_id, scope_id, &json).await;
        }
        rows::write_changes(&self.env.db, &self.community_id, scope_id, &changes).await
    }

    pub fn is_member(&self, who: &PubKey) -> bool {
        self.in_scope(who, SCOPE_EVERYONE)
    }

    /// Whether `who` holds a leaf (a secret) of `scope_id` here.
    pub fn in_scope(&self, who: &PubKey, scope_id: &str) -> bool {
        member_id(who).is_ok_and(|m| self.scopes.get(scope_id).is_some_and(|s| s.kd().is_member(&m)))
    }

    /// The leaf of `who` gone from the tree of `everyone`, its reader row
    /// kept: a tree that lost one (a keeper recovered without them).
    #[cfg(test)]
    pub(crate) async fn drop_leaf(&mut self, who: &PubKey, now: i64) -> Result<()> {
        let m = member_id(who)?;
        self.sk(SCOPE_EVERYONE)?.kd_mut().revoke(&[m], Revoke::Ban, secs(now));
        self.save(SCOPE_EVERYONE).await
    }

    /// Puts `who` out of every scope kept here (a ban, a removal, a leave).
    async fn put_out(&mut self, who: &PubKey, why: Revoke, now: i64) -> Result<()> {
        let m = member_id(who)?;
        for scope_id in self.scope_ids() {
            let k = self.sk(&scope_id)?;
            if k.kd().is_member(&m) {
                k.kd_mut().revoke(&[m], why, secs(now));
                self.save(&scope_id).await?;
            }
        }
        rows::reader_out(&self.env.db, &self.community_id, who.as_hex()).await
    }

    /// A reader of `everyone` in the scopes the journal gives them (wire
    /// §14.1): let into those they lack, each with its own
    /// `channel.issue`, and out of those no level gives any more (an epoch
    /// of that scope alone, at the ban window). A flat scope full
    /// (`FLAT_MAX_MEMBERS`) lets nobody more in.
    pub(crate) async fn sync_member(&mut self, log: &OpLog, who: &PubKey, now: i64) -> Result<()> {
        if !self.is_member(who) {
            return Ok(());
        }
        let want = log.state().scopes_of(who);
        let m = member_id(who)?;
        for scope_id in self.scope_ids().into_iter().filter(|s| s != SCOPE_EVERYONE) {
            let k = self.sk(&scope_id)?;
            let inside = k.kd().is_member(&m);
            if want.contains(&scope_id) && !inside {
                if k.scheme() == SchemeId::FlatV1 && k.members_len() >= FLAT_MAX_MEMBERS {
                    tracing::warn!(community = %self.community_id, scope = %scope_id, "a flat scope is full: nobody more is let in");
                    continue;
                }
                let issued = k.kd_mut().join(m, secs(now)).map_err(kd)?;
                self.save(&scope_id).await?;
                self.send_issue(log, who, &issued, now).await?;
            } else if !want.contains(&scope_id) && inside {
                k.kd_mut().revoke(&[m], Revoke::Ban, secs(now));
                self.save(&scope_id).await?;
            }
        }
        Ok(())
    }

    /// Every reader in step with the journal, when something may have
    /// moved them (`members_dirty`).
    pub(crate) async fn sync_members(&mut self, log: &OpLog, now: i64) -> Result<()> {
        if !self.members_dirty {
            return Ok(());
        }
        let everyone = self.scopes.get(SCOPE_EVERYONE).map(ScopeKeeper::members).unwrap_or_default();
        for m in everyone {
            let Some(who) = PubKey::parse(&hex::encode(m.0)) else { continue };
            self.sync_member(log, &who, now).await?;
        }
        self.members_dirty = false;
        Ok(())
    }

    /// The key of a flat epoch that went out, to each member of its scope
    /// in a `channel.issue` of its own (`Issued` of `flat-v1` with the key
    /// as `current`): a flat scope has no shards, the secret of each member
    /// is all there is (wire §14.2).
    pub(crate) async fn issue_flat(&mut self, log: &OpLog, scope_id: &str, now: i64) -> Result<()> {
        for m in self.sk(scope_id)?.members() {
            let Some(who) = PubKey::parse(&hex::encode(m.0)) else { continue };
            let issued = self.sk(scope_id)?.kd_mut().bundle_for(m, secs(now)).map_err(kd)?;
            self.send_issue(log, &who, &issued, now).await?;
        }
        Ok(())
    }


    // ─── Readers ────────────────────────────────────────────────────────────

    /// Lets `who` in (or hands a member the same leaf again) and sends
    /// `channel.issue`.
    pub async fn admit_reader(&mut self, log: &OpLog, who: &PubKey, now: i64) -> Result<()> {
        let issued = self.sk(SCOPE_EVERYONE)?.kd_mut().join(member_id(who)?, secs(now)).map_err(kd)?;
        self.save(SCOPE_EVERYONE).await?;
        rows::reader_in(&self.env.db, &self.community_id, who.as_hex(), now).await?;
        self.send_issue(log, who, &issued, now).await?;
        // And into the sections their levels give, each with its issue.
        self.sync_member(log, who, now).await
    }

    /// A `channel.join` (wire §7.1), from `from` as the seal says.
    pub async fn on_join(&mut self, log: &OpLog, from: &PubKey, join: &Join, now: i64) -> Result<Joined> {
        let state = log.state();
        if join.community_id != self.community_id {
            return Err(invalid("another channel"));
        }
        if let Err(why) = join.check(now) {
            return self.refuse(from, why, now).await;
        }
        if state.disbanded {
            return self.refuse(from, "disbanded", now).await;
        }
        if state.is_banned(from) {
            return self.refuse(from, "banned", now).await;
        }
        // A join sent again by a member whose issue was lost (wire §7):
        // the same leaf and bundle again, as a rebundle would give; its
        // admission may have been used by the first one.
        // A join written before this member was let in was answered then:
        // a replay (a restart that reads two days of notes again, a relay
        // that hands one twice) sends nothing (the trial of 1.6, problem 7:
        // ~500 bundles, 14 MB, after every restart).
        if self.is_member(from) {
            let since = rows::reader_since(&self.env.db, &self.community_id, from.as_hex()).await?;
            if since.is_some_and(|s| join.created_at <= s) {
                self.env.say.line(&format!("{} from {} for {}: a reader already, the same join again: nothing sent", wire::T_JOIN, from.short(), crate::service::short(&self.community_id)));
                return Ok(Joined::Issued);
            }
            self.admit_reader(log, from, now).await?;
            return Ok(Joined::Issued);
        }
        // A join written before the journal put this person out is one the
        // removal undid: a keeper that reads two days of notes again with no
        // rows of the keeper before it (an emergency on a fresh device) let
        // the one removed back in by it (an e2e of 2026-10-30). Dropped
        // without a word: an old client told `rejected` would stop its feed.
        // The join and the removal are dated by two clocks, and nothing of the
        // journal orders a note: a join dated at or before the removal counts
        // as a replay (a new one comes with a new invitation, minutes later;
        // one refused by a clock far behind is sent again by its app).
        // Operations the journal rejected (a demoted moderator's) put nobody out.
        let out_at = log
            .ordered()
            .filter(|o| log.rejection(&o.id()).is_none())
            .filter(|o| matches!(&o.body, OpBody::Remove { who } | OpBody::Ban { who, .. } if who == from))
            .map(|o| o.created_at)
            .max();
        if out_at.is_some_and(|t| join.created_at <= t) {
            self.env.say.line(&format!("{} from {} for {}: written before the journal put them out, a replay: dropped", wire::T_JOIN, from.short(), crate::service::short(&self.community_id)));
            return Ok(Joined::Refused("put out after this join"));
        }
        // A channel past what a snapshot names would fail every snapshot
        // after it: the newcomer is refused, clearly, at the door.
        let readers = self.scopes.get(SCOPE_EVERYONE).map_or(0, ScopeKeeper::members_len);
        if readers >= self.limits.max_readers {
            return self.refuse(from, "the channel is full", now).await;
        }
        while self.recent_joins.front().is_some_and(|t| *t <= now - 60) {
            self.recent_joins.pop_front();
        }
        if self.recent_joins.len() >= self.limits.joins_per_minute {
            tracing::info!(community = %self.community_id, "keeper is busy with joins: this one waits to be sent again");
            return Ok(Joined::Busy);
        }
        let ok = match (&join.proof, &join.admit_id) {
            (Some(proof), None) => {
                if state.join != JoinMode::Open {
                    return self.refuse(from, "not by link", now).await;
                }
                self.proof_ok(state, from, proof.link_epoch, &proof.mac, now).await?
            }
            (None, Some(admit_id)) => {
                // A keeper that wakes after a day gets the newest notes
                // first: the join before the admission it names. Refused
                // now, the joiner would give up for good.
                if db::channels::admit(&self.env.db, admit_id).await?.is_none() {
                    rows::expire_held_joins(&self.env.db, &self.community_id, now - wire::INVITE_TTL_SECS - wire::SKEW_SECS).await?;
                    let json = serde_json::to_string(join)?;
                    if rows::hold_join(&self.env.db, &self.community_id, from.as_hex(), admit_id, &json, now, MAX_HELD_JOINS).await? {
                        return Ok(Joined::Held);
                    }
                    return self.refuse(from, "too many joins wait", now).await;
                }
                self.admission_ok(log, from, admit_id, now).await?
            }
            _ => false,
        };
        if !ok {
            return self.refuse(from, "not admitted", now).await;
        }
        self.admit_reader(log, from, now).await?;
        self.recent_joins.push_back(now);
        Ok(Joined::Issued)
    }

    async fn refuse(&mut self, to: &PubKey, why: &'static str, now: i64) -> Result<Joined> {
        let body = Rejected { community_id: self.community_id.clone(), reason: None };
        self.send_note(None, to, wire::T_REJECTED, &body, now).await?;
        Ok(Joined::Refused(why))
    }

    /// A link proof: the MAC of the current link epoch; of the previous
    /// one only for `LINK_GRACE_SECS` after a `rotate_link` whose secret
    /// has not come here.
    async fn proof_ok(&mut self, state: &ChannelState, from: &PubKey, link_epoch: u32, mac: &str, now: i64) -> Result<bool> {
        let held = secrets::links(self.env.secrets.as_ref(), &self.community_id).await?;
        let current = state.link_epoch;
        if link_epoch == current {
            return Ok(held.get(&current).is_some_and(|s| s.check_mac(&self.community_id, from, link_epoch, mac)));
        }
        if link_epoch.checked_add(1) != Some(current) || held.contains_key(&current) {
            return Ok(false);
        }
        let row = rows::keeper(&self.env.db, &self.community_id).await?;
        let waiting = row.as_ref().and_then(|r| r.link_wait_epoch.zip(r.link_wait_since)).filter(|(e, _)| *e == i64::from(current));
        match waiting {
            Some((_, since)) if now - since <= LINK_GRACE_SECS => {
                tracing::warn!(community = %self.community_id, link_epoch, "keeper took a link of the previous link epoch: the new secret has not come");
                Ok(held.get(&link_epoch).is_some_and(|s| s.check_mac(&self.community_id, from, link_epoch, mac)))
            }
            _ => {
                tracing::warn!(community = %self.community_id, link_epoch, "keeper refused a link of the previous link epoch: its grace is over and the new secret has not come");
                Ok(false)
            }
        }
    }

    /// An admission for `from` that a moderator made: here, open, not
    /// expired, its author still a moderator and not banned; used once.
    async fn admission_ok(&mut self, log: &OpLog, from: &PubKey, admit_id: &str, now: i64) -> Result<bool> {
        let Some(row) = db::channels::admit(&self.env.db, admit_id).await? else { return Ok(false) };
        let state = log.state();
        let by_ok = PubKey::parse(&row.by).is_some_and(|by| !state.is_banned(&by) && state.role_of(&by).is_some_and(|r| permits(r, Action::Admit)));
        if row.community_id != self.community_id || row.who != from.as_hex() || !by_ok {
            return Ok(false);
        }
        db::channels::use_admit(&self.env.db, admit_id, now).await
    }

    /// A `channel.admit` (wire §7.1): taken from a moderator or above by
    /// the journal, not expired, with an invitation used for no other
    /// admission. `false` when refused.
    pub async fn on_admit(&mut self, log: &OpLog, from: &PubKey, admit: &Admit, now: i64) -> Result<bool> {
        let state = log.state();
        if admit.community_id != self.community_id || admit.check(now).is_err() {
            return Ok(false);
        }
        let moderator = !state.is_banned(from) && state.role_of(from).is_some_and(|r| permits(r, Action::Admit));
        if !moderator {
            tracing::info!(community = %self.community_id, by = %from.short(), "keeper refused an admission of somebody not a moderator");
            return Ok(false);
        }
        if state.is_banned(&admit.who) {
            return Ok(false);
        }
        if let Some(invite_id) = &admit.invite_id {
            if !rows::use_invite(&self.env.db, &self.community_id, invite_id, &admit.admit_id, admit.who.as_hex(), now).await? {
                tracing::info!(community = %self.community_id, "keeper refused an invitation used before");
                return Ok(false);
            }
        }
        let row = db::channels::AdmitRow {
            admit_id: admit.admit_id.clone(),
            community_id: self.community_id.clone(),
            who: admit.who.as_hex().to_string(),
            by: from.as_hex().to_string(),
            levels: i64::try_from(admit.levels).unwrap_or(i64::MAX),
            invite_id: admit.invite_id.clone(),
            expires_at: admit.expires_at,
            status: "open".into(),
            created_at: now,
        };
        if !db::channels::insert_admit(&self.env.db, &row).await? {
            // The same admission again (or the moderator's own copy on
            // this device): taken when it says the same.
            let held = db::channels::admit(&self.env.db, &admit.admit_id).await?;
            return Ok(held.is_some_and(|h| h.who == row.who && h.by == row.by && h.community_id == row.community_id));
        }
        // A ticket the moderator handed the newcomer while I was away (wire §17).
        self.ticket_admitted(admit, from, now).await?;
        // The join that came before this admission goes on now.
        if let Some((who, json)) = rows::take_held_join(&self.env.db, &self.community_id, &admit.admit_id).await? {
            if let (Some(who), Ok(join)) = (PubKey::parse(&who), serde_json::from_str::<Join>(&json)) {
                let joined = self.on_join(log, &who, &join, now).await?;
                tracing::info!(community = %self.community_id, ?joined, "keeper took a join that waited for its admission");
            }
        }
        Ok(true)
    }

    /// `channel.rebundle`: the bundle of the current generation again, to
    /// a current member. One not in the tree of `everyone` is let in again
    /// (`admit_reader`) when it is the owner, or staff by the journal (an
    /// author or above; a reader the journal mutes, trusts or levels is
    /// not) whom this keeper let in before (its rows) or the newest
    /// snapshot names (`known`), and nothing put out since (`PutOut`): a
    /// tree that lost them (a keeper recovered without them) does not keep
    /// them out.
    pub async fn on_rebundle(&mut self, log: &OpLog, from: &PubKey, r: &Rebundle, known: bool, now: i64) -> Result<bool> {
        if r.community_id != self.community_id || r.check().is_err() {
            return Ok(false);
        }
        let state = log.state();
        if !self.is_member(from) && !state.is_banned(from) && !state.disbanded {
            let owner = state.owner == *from;
            let staff = state.role_of(from).is_some_and(|r| r >= Role::Author);
            // A row is no reader once the journal put them out after it: a
            // ban or a removal drops the row (`on_op`); one an older run
            // left is weighed against the time of the removal.
            let known = known
                || (staff && rows::reader_since(&self.env.db, &self.community_id, from.as_hex()).await?.is_some_and(|since| PutOut::of(log, None).keeps(state, from.as_hex(), since)));
            if owner || (staff && known) {
                self.admit_reader(log, from, now).await?;
                self.env.say.line(&format!("{} from {} for {}: not in the tree, {}: let in again", wire::T_REBUNDLE, from.short(), crate::service::short(&self.community_id), if owner { "the owner" } else { "staff known as a reader" }));
                return Ok(true);
            }
        }
        if !self.in_scope(from, &r.scope_id) {
            // Staff by the journal never let in as a reader (made an author
            // before any invitation): told so, once a while, instead of
            // silence, so that it stops asking and is invited. Anyone else
            // is answered nothing; the owner never is told so (let in above).
            let told = self.not_let_in_told.get(from.as_hex()).is_some_and(|t| now - t < NOT_LET_IN_EVERY);
            if !self.is_member(from) && state.owner != *from && state.role_of(from).is_some() && !told {
                self.not_let_in_told.insert(from.as_hex().to_string(), now);
                let body = wire::NotLetIn { community_id: self.community_id.clone(), scope_id: r.scope_id.clone() };
                self.send_note(Some(state), from, wire::T_NOT_LET_IN, &body, now).await?;
                tracing::info!(community = %self.community_id, who = %from.short(), "a bundle asked by one never let in: told to be invited first");
            }
            return Ok(false);
        }
        let issued = self.sk(&r.scope_id)?.kd_mut().bundle_for(member_id(from)?, secs(now)).map_err(kd)?;
        self.save(&r.scope_id).await?;
        self.send_issue(log, from, &issued, now).await?;
        self.resend_upload_keys(from).await?;
        self.resend_cloud_key(from).await?;
        Ok(true)
    }

    /// `channel.missed`: the next page of missed keys, to a member.
    pub async fn on_missed(&mut self, log: &OpLog, from: &PubKey, m: &Missed, now: i64) -> Result<bool> {
        if m.community_id != self.community_id || m.check().is_err() || !self.in_scope(from, &m.scope_id) {
            return Ok(false);
        }
        let page = self.sk(&m.scope_id)?.kd().missed_page(&member_id(from)?, m.after_epoch, keydist::issued::MISSED_PAGE).map_err(kd)?;
        self.send_issue(log, from, &page, now).await?;
        Ok(true)
    }

    /// `channel.leave`: out, by the leave window of the policy.
    pub async fn on_leave(&mut self, from: &PubKey, now: i64) -> Result<bool> {
        if !self.is_member(from) {
            return Ok(false);
        }
        self.put_out(from, Revoke::Leave, now).await?;
        Ok(true)
    }

    /// An operation the journal took (applied, not rejected): a ban or a
    /// removal puts the reader out at the ban window, `rotate_now` asks
    /// for an urgent epoch, `rotate_link` starts the grace of the old
    /// link until its secret comes. Each operation once: those acted on
    /// in this run are known in memory, the rest asked of the rows
    /// (`mark_op`), so that every operation of the journal may be offered
    /// on every event and turn (`ChannelService::sync_ops`) for the cost
    /// of a lookup.
    pub async fn on_op(&mut self, log: &OpLog, op: &Op, now: i64) -> Result<()> {
        let id = op.id();
        if !log.contains(&id) || log.rejection(&id).is_some() || self.ops_done.contains(&id.0) {
            return Ok(());
        }
        let first = rows::mark_op(&self.env.db, &self.community_id, &id.0).await?;
        self.ops_done.insert(id.0.clone());
        if !first {
            return Ok(());
        }
        match &op.body {
            // Out of the trees it is in, and its row dropped whether it is
            // in a tree or not (one the tree lost is not let in again by
            // its row, `on_rebundle`).
            OpBody::Ban { who, .. } | OpBody::Remove { who } => self.put_out(who, Revoke::Ban, now).await?,
            // The levels of one person: their scopes put in step at once,
            // so that a grant issues now (wire §14.4).
            // Marked done already: a failure leaves it to the next turn.
            OpBody::SetLevels { who, .. } => {
                if let Err(e) = self.sync_member(log, who, now).await {
                    self.members_dirty = true;
                    return Err(e);
                }
            }
            // The rules of the channel (how readers get in, the history):
            // the policy follows at once, not at the next turn.
            OpBody::Settings { .. } => self.sync_policy(log.state()).await?,
            // What the scopes are, or what the levels give, or who is staff
            // (staff belong to every scope): everybody on the next turn.
            OpBody::ScopeAdd { .. } | OpBody::ScopeDel { .. } | OpBody::LevelSet { .. } | OpBody::LevelDel { .. } | OpBody::SetRole { .. } => {
                self.sync_scopes(log.state(), now).await?;
                self.members_dirty = true;
                self.sync_members(log, now).await?;
            }
            OpBody::RotateNow { scope_id } => {
                self.urgent.insert(scope_id.clone().unwrap_or_else(|| SCOPE_EVERYONE.to_string()));
            }
            OpBody::RotateLink { link_epoch } => {
                let held = secrets::links(self.env.secrets.as_ref(), &self.community_id).await?;
                if !held.contains_key(link_epoch) {
                    rows::set_link_wait(&self.env.db, &self.community_id, Some((*link_epoch, now))).await?;
                }
                // Its author seals the new secret under the next epoch only
                // (`publish_owed_links`): that epoch is built now, with every
                // revoke pending, so the link waits for nothing else.
                self.urgent.insert(SCOPE_EVERYONE.to_string());
            }
            _ => {}
        }
        Ok(())
    }

    /// The secret of a link epoch, from the feed or made here: kept, and
    /// the grace of the previous link ends.
    pub async fn take_link(&mut self, link: &LinkPayload) -> Result<bool> {
        take_link(&self.env.db, self.env.secrets.as_ref(), &self.community_id, link).await
    }

    /// `keeper.master`: a master handed over by the current owner, kept in
    /// the secret store for a keeper that does not hold the root.
    pub async fn on_master(&mut self, log: &OpLog, from: &PubKey, m: &KeeperMaster) -> Result<bool> {
        if m.community_id != self.community_id || m.check().is_err() || from != &log.state().owner {
            return Ok(false);
        }
        if self.root.is_some() {
            // The masters derive from the root here; one handed over says
            // nothing new.
            return Ok(true);
        }
        let raw: [u8; 32] = B64.decode(&m.master).ok().and_then(|b| b.try_into().ok()).ok_or_else(|| invalid("master"))?;
        secrets::put_master(self.env.secrets.as_ref(), &self.community_id, &m.scope_id, m.gen, &GenerationMaster::from_bytes(raw)).await?;
        Ok(true)
    }

    // ─── Notes ──────────────────────────────────────────────────────────────

    /// A note from the keeper's key to `to`, by NIP-59: a row of the outbox
    /// (`db::outbox`, lane `inbox`), wrapped with the keeper's keys and sent
    /// by the worker off this turn, behind the operations of the channel
    /// queued before it (an issue of a scope after the operation that made
    /// it, wire §14.5), and again after a failure (the trial of 1.6,
    /// problem 9: `timeout`). An issue goes with the channel's relays as the
    /// hint (`state`, stage 1: where members listen); a refusal (`state`
    /// none) to the person's own inbox relays, as a direct message: the
    /// joiner may follow nothing on the channel's.
    async fn send_note<T: serde::Serialize>(&mut self, state: Option<&ChannelState>, to: &PubKey, t: &str, body: &T, now: i64) -> Result<()> {
        let content = wire::note_envelope(t, body)?;
        let hint_relays: Vec<&str> = state.map(|s| s.relays.iter().map(String::as_str).filter(|r| RelayUrl::parse(r).is_some()).collect()).unwrap_or_default();
        let note = db::outbox::NewNote {
            recipient: to.as_hex().to_string(),
            out_id: crate::blobs::sha256_hex(format!("{}:{}", to.as_hex(), content).as_bytes()),
            content,
            hint_json: serde_json::to_string(&hint_relays)?,
            signer: db::outbox::SIGNER_KEEPER.into(),
            after_seq: db::outbox::last_channel_seq(&self.env.db, &self.community_id).await?,
            ref_id: None,
        };
        db::outbox::enqueue_note(&self.env.db, &self.community_id, &note, now).await?;
        self.env.outbox.notify_one();
        self.env.say.line(&format!("{t} to {} for {}: queued", to.short(), crate::service::short(&self.community_id)));
        Ok(())
    }

    /// `channel.issue` with what keydist issued, the heads of the journal
    /// and the secret of the current link.
    async fn send_issue(&mut self, log: &OpLog, to: &PubKey, issued: &keydist::Issued, now: i64) -> Result<()> {
        let state = log.state();
        let held = secrets::links(self.env.secrets.as_ref(), &self.community_id).await?;
        let link = held.get(&state.link_epoch).map(|s| LinkPayload { link_epoch: state.link_epoch, secret: s.encode() });
        let issue = Issue {
            community_id: self.community_id.clone(),
            relays: state.relays.clone(),
            storage: state.storage.clone(),
            heads: issue_heads(log),
            link,
            issued: Issue::encode_issued(issued),
        };
        issue.check().map_err(invalid)?;
        // The device's own user takes it from here at once (the note still
        // goes, for their other devices): by the echo of the relay the owner
        // waited for the key of a channel or a section just made (живой
        // запуск 2026-10-23: «Ждём ключ хранителя»).
        if self.is_mine(state, to) {
            self.to_me.push((to.clone(), issue.clone()));
        }
        self.send_note(Some(state), to, wire::T_ISSUE, &issue, now).await
    }

    /// Whether `who` is the user at work on this device, or the owner by
    /// the journal (stage 1: the keeper runs on the owner's device).
    fn is_mine(&self, state: &ChannelState, who: &PubKey) -> bool {
        let payer = self.env.payer.read().ok().and_then(|p| p.as_ref().map(|k| k.public_key().to_hex()));
        payer.as_deref() == Some(who.as_hex()) || &state.owner == who
    }

    /// The issues sent to this device's own user since the last call
    /// (`to_me`), each with whom it went to, for its member side.
    pub fn take_issued_to_me(&mut self) -> Vec<(PubKey, Issue)> {
        std::mem::take(&mut self.to_me)
    }
}

/// `Keeper::take_link` by the rows and the secret store alone, which is all
/// it touches: a reader of the feed on this device takes a link secret
/// without the keeper's lock, held by a turn that reads content again.
pub(crate) async fn take_link(db: &Db, store: &dyn SecretStore, community_id: &str, link: &LinkPayload) -> Result<bool> {
    let secret = crate::link::LinkSecret::decode(&link.secret)?;
    let taken = secrets::put_link(store, community_id, link.link_epoch, &secret).await?;
    if !taken {
        tracing::warn!(community = %community_id, link_epoch = link.link_epoch, "a second secret for a link epoch: the first one stays");
    }
    if let Some(row) = rows::keeper(db, community_id).await? {
        if row.link_wait_epoch.is_some_and(|e| e <= i64::from(link.link_epoch)) {
            rows::set_link_wait(db, community_id, None).await?;
        }
    }
    Ok(taken)
}

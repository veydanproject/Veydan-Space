// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Where the channels live while the module runs: the service of the core
//! over the database of the identity at work (`<npub>.db`), and the loop
//! that keeps it going (the events of
//! the channels' relays in, the keepers' cycle at the time it asks for, a
//! new session of the messenger followed).
//!
//! The module speaks the messenger's network as the same person (seams of
//! the shell, `net_lend` of the messenger): its requests go through the
//! messenger's pool, which it tells the relays of each channel and from
//! which it takes the events of its own subscriptions (the routes of
//! messenger-transport). Off, nothing of it runs: no routes, no loop, the
//! database closed.

use crate::background::Background;
use crate::wake::{Resolve, Target, WakingNet};
use communities_core::blobs::MediaHosts;
use communities_core::db::{self, Db};
use communities_core::handler::Signal;
use communities_core::keeper::Steps;
use communities_core::service::{ChannelService, Outcome};
use communities_core::transport::{OWN_ROUTE, PART_NOTES, WRAP_MARGIN_SECS};
use messenger_core::outbound::Filter;
use messenger_core::traits::{RelayState, SystemClock, UiEvent};
use messenger_core::{EventSource, MessengerError, Outbound, RawEvent, RelayUrl, Scope, SecretStore, Transport};
use messenger_transport::{route_channels, ChannelRoutes};
use nostr::key::{Keys, SecretKey};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};
use veydan_core::{AppError, CmdResult};
use veydan_lock::Lock;
use veydan_shell::seams::{self, NetworkSession};
use zeroize::Zeroizing;

/// Emitted with a `UiEvent` of the core (`{name, payload}`):
/// `channel.updated`, `channel.message`, `channel.invite`,
/// `channel.request`, `channel.media`, `channel.moderation`,
/// `channel.queue`, `channel.outbox`, and `channel.keeper` and
/// `channel.ready` of this module.
pub const EVENT: &str = "communities://event";

/// The status of a keeper changed here (an epoch went out, the cycle ran):
/// `{community_id}`.
pub const UI_EVENT_CHANNEL_KEEPER: &str = "channel.keeper";

/// The channels of the identity at work are open (the module started, or
/// another identity came): whatever a page shows is read again. `{}`.
pub const UI_EVENT_CHANNEL_READY: &str = "channel.ready";

/// What the commands answer while the module is off or did not start.
pub(crate) const STOPPED: &str = "communities_stopped";

/// What the commands answer while the messenger has no identity at work.
pub(crate) const NO_SESSION: &str = "channel_no_session";

/// The folder of the module in the data directory: its databases, which a
/// backup takes.
pub(crate) const DATA_SUBDIR: &str = "communities";

/// The folder of the files the module fetched (`media/`) and of the copies
/// of those it sends (`outgoing/`): beside its own folder, so that a backup
/// does not carry what the hosts hold anyway, up to a gigabyte a file.
pub(crate) const FILES_SUBDIR: &str = "communities-files";

/// The box of the lock the secrets of the channels live in: keeper keys,
/// root secrets, masters of generations, link secrets.
const SECRET_BOX: &str = "communities";

/// How often the session of the messenger is looked at: a new identity, a
/// lock opened or closed, connections made anew.
const SESSION_CHECK: Duration = Duration::from_secs(5);

/// The cycle of the keepers runs at least this often, whatever it asked.
const TICK_AT_LEAST: Duration = Duration::from_secs(300);

/// The most the journal of a channel opened by its link is waited for: a
/// relay that answered ends the wait sooner (`take_journal`); this is for
/// one that never answers (the pool's patience is 20 s a relay).
pub(crate) const JOURNAL_WAIT: Duration = Duration::from_secs(45);
/// How long the card of a channel shown before joining (its link's
/// preview) is waited for; a relay of the link that does not answer by
/// then is said to be out of reach.
pub(crate) const PREVIEW_WAIT: Duration = Duration::from_secs(10);

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn hold<T>(lock: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Errors of the core in the words of the page: a refusal of the core
/// (`channel: …`) becomes `channel_<code>: <text>`, a stable code the UI
/// translates, as `group_…` is for groups.
pub(crate) fn map_err(e: MessengerError) -> AppError {
    match e {
        MessengerError::SecretsLocked => AppError::VaultLocked,
        // The database closed under a command on its way: the module was
        // switched off (or another identity came), not a fault of the disk.
        MessengerError::Storage(m) if m.contains("closed pool") => AppError::Other(format!("{STOPPED}: {m}")),
        MessengerError::Storage(m) => AppError::Db(m),
        MessengerError::Io(m) => AppError::Io(m),
        MessengerError::NotLoggedIn => AppError::NotFound("messenger identity".into()),
        MessengerError::Invalid(m) => AppError::Other(format!("{}: {m}", code_of(&m))),
        MessengerError::Transport(m) if m.contains("not_delivered") => AppError::Other(format!("channel_not_delivered: {m}")),
        // Every host of the channel needs the key of its own server, which
        // this device has not (communities-wire §9.1).
        MessengerError::Transport(m) if m.contains(communities_core::upload_keys::NEEDS_KEY) => AppError::Other(format!("channel_storage_needs_key: {m}")),
        MessengerError::Transport(m) => AppError::Other(format!("channel_transport: {m}")),
        other => AppError::Other(other.to_string()),
    }
}

/// The code of a refusal of the core by its text.
pub(crate) fn code_of(text: &str) -> &'static str {
    // The media code says `channel media: …` (live 2026-10-23: a cover that is
    // no picture came out as «it did not work», its code never matched).
    let what = text.strip_prefix("channel: ").or_else(|| text.strip_prefix("channel media: ")).or_else(|| text.strip_prefix("channel link: ")).unwrap_or(text);
    if text.starts_with("channel link: ") {
        return "channel_bad_link";
    }
    // A blob host said no: its address and its answer follow, for the page.
    if what.starts_with("upload refused") {
        return "channel_upload_refused";
    }
    // The levels and the sections (wire §14): refused here, or by the
    // journal (`Invalid { what: "…" }` of its rejection).
    for (word, code) in [
        ("level name", "channel_level_name"),
        ("level last", "channel_level_last"),
        ("base level", "channel_level_last"),
        ("level limit", "channel_level_limit"),
        ("level id", "channel_level_limit"),
        ("scope everyone", "channel_scope_everyone"),
        ("scope limit", "channel_scope_limit"),
        ("not member of scope", "channel_not_member_of_scope"),
        ("no post in this scope", "channel_scope_no_post"),
        ("no transfer offer", "channel_no_transfer"),
        ("no tickets in an open channel", "channel_tickets_open"),
        ("no key of the section yet", "channel_no_key"),
    ] {
        if what.contains(word) {
            return code;
        }
    }
    match what {
        "unknown channel" => "channel_unknown",
        "unknown message" => "channel_unknown_message",
        "not permitted" => "channel_not_permitted",
        "not permitted to upload" => "channel_upload_not_permitted",
        "no keeper" => "channel_no_keeper",
        "no key of the channel yet" => "channel_no_key",
        "no key of the channel to hand the new link on" => "channel_no_key",
        "the journal of the channel is not here yet" => "channel_not_here_yet",
        // A take of the keeper before the channel came to its head (wire §15).
        communities_core::handover::EPOCHS_NOT_HERE | communities_core::handover::NO_READERS => "channel_not_here_yet",
        "the link names another owner" => "channel_link_owner",
        "the owner does not leave" => "channel_owner_stays",
        "this device keeps the channel" => "channel_kept_here",
        "too many open invitations" => "channel_too_many_invites",
        communities_core::service::ChannelService::ALREADY_MEMBER => "channel_already_member",
        "this channel takes invitations only" => "channel_invite_only",
        "the invitation expired" => "channel_invite_expired",
        "unknown invitation" => "channel_invite_unknown",
        "only the author edits" => "channel_not_author",
        "banned" => "channel_banned",
        "the upload was removed" => "channel_upload_removed",
        // Its own code already: a send given up while it ran.
        communities_core::media::UPLOAD_CANCELLED => "channel_upload_cancelled",
        "not a file that can be sent" => "channel_file_unsupported",
        "a cover is not a picture" => "channel_cover_unsupported",
        "a cover is too large" => "channel_cover_too_large",
        "the address of the cover" => "channel_cover_address",
        "transfer to oneself" | "removing oneself" => "channel_self",
        "no relay" => "channel_no_relay",
        _ => "channel_invalid",
    }
}

/// `SecretStore` of the channels: the lock's box `communities`, on this
/// device only; a closed lock answers `SecretsLocked`.
struct BoxSecrets {
    app: AppHandle,
}

impl BoxSecrets {
    fn locked(e: veydan_lock::Error) -> MessengerError {
        match e {
            veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => MessengerError::SecretsLocked,
            veydan_lock::Error::Db(m) => MessengerError::Storage(m),
            other => MessengerError::Crypto(other.to_string()),
        }
    }

    fn lock(&self) -> messenger_core::Result<tauri::State<'_, Lock>> {
        self.app.try_state::<Lock>().ok_or(MessengerError::SecretsLocked)
    }
}

#[async_trait::async_trait]
impl SecretStore for BoxSecrets {
    async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
        self.lock()?.secret_box(SECRET_BOX).get(key).await.map_err(Self::locked)
    }

    async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
        self.lock()?.secret_box(SECRET_BOX).put(key, value).await.map_err(Self::locked)
    }

    async fn delete(&self, key: &str) -> messenger_core::Result<()> {
        self.lock()?.secret_box(SECRET_BOX).delete(key).await.map_err(Self::locked)
    }

    async fn is_unlocked(&self) -> bool {
        self.lock().is_ok_and(|lock| lock.secret_box(SECRET_BOX).is_unlocked())
    }
}

/// The owner's Veydan Cloud the module `cloud` lends (the seam
/// `CloudDelegates`), in the shape the keepers ask it
/// (internal/communities-wire.md §9.2).
struct CloudSeam(Arc<dyn veydan_shell::seams::CloudDelegates>);

fn cloud_error(e: veydan_shell::seams::CloudError) -> MessengerError {
    MessengerError::Transport(format!("{}: {}", e.code, e.message))
}

#[async_trait::async_trait]
impl communities_core::cloud_keys::CloudDelegates for CloudSeam {
    async fn account(&self) -> Option<String> {
        self.0.account().await.ok().flatten()
    }

    async fn put(&self, d: &communities_core::cloud_keys::DelegatePut) -> messenger_core::Result<()> {
        let put = veydan_shell::seams::DelegatePut {
            pubkey: d.pubkey.clone(),
            label: d.label.clone(),
            expires_at: u64::try_from(d.expires_at).unwrap_or(0),
            per_day: d.per_day,
            proof: d.proof.clone(),
        };
        self.0.put(&put).await.map_err(cloud_error)
    }

    async fn revoke(&self, pubkey: &str) -> messenger_core::Result<()> {
        self.0.revoke(pubkey).await.map_err(cloud_error)
    }
}

/// The relays of links held, by the channel: the number of each holder and
/// its relay.
type Holds = HashMap<String, Vec<(u64, RelayUrl)>>;

/// The relays of each channel, as the pool asks for them.
#[derive(Clone, Default)]
pub(crate) struct Routes {
    /// Of the channels with a row here, as their journals name them.
    known: Arc<RwLock<HashMap<String, Vec<RelayUrl>>>>,
    /// The relay of the link of a channel being opened, whose row is not
    /// here yet: kept until the row is.
    linked: Arc<RwLock<HashMap<String, Vec<RelayUrl>>>>,
    /// The relays of links being looked at or opened right now, each under
    /// the number of its holder: one that ends takes only its own away.
    held: Arc<RwLock<Holds>>,
    next_hold: Arc<std::sync::atomic::AtomicU64>,
    /// The relays of the channel being made: its id is the core's to
    /// choose, and its journal, card and first epoch go out before its row
    /// is written.
    creating: Arc<RwLock<Vec<RelayUrl>>>,
    /// My own relays, where the notes to me are listened to outside every
    /// channel: the route `OWN_ROUTE`, as the pool took the subscription.
    own: Arc<RwLock<Vec<RelayUrl>>>,
    /// One channel made at a time.
    one: Arc<Mutex<()>>,
}

/// While held, a channel unknown to the routes is the channel being made.
pub(crate) struct Creating {
    creating: Arc<RwLock<Vec<RelayUrl>>>,
    _one: tokio::sync::OwnedMutexGuard<()>,
}

impl Drop for Creating {
    fn drop(&mut self) {
        write(&self.creating).clear();
    }
}

impl Routes {
    fn of(&self, community_id: &str) -> Vec<RelayUrl> {
        if community_id == OWN_ROUTE {
            return read(&self.own).clone();
        }
        let mut relays = read(&self.known).get(community_id).cloned().or_else(|| read(&self.linked).get(community_id).cloned()).unwrap_or_default();
        for (_, relay) in read(&self.held).get(community_id).into_iter().flatten() {
            if !relays.contains(relay) {
                relays.push(relay.clone());
            }
        }
        if relays.is_empty() {
            return read(&self.creating).clone();
        }
        relays
    }

    pub(crate) async fn creating(&self, relays: Vec<RelayUrl>) -> Creating {
        let one = self.one.clone().lock_owned().await;
        *write(&self.creating) = relays;
        Creating { creating: self.creating.clone(), _one: one }
    }

    /// The relays of the channels with a row here, and no others: a
    /// channel forgotten leaves no route behind. A channel opened by its
    /// link keeps the relay of the link until its journal names its own.
    fn set(&self, known: HashMap<String, Vec<RelayUrl>>) {
        write(&self.linked).retain(|id, _| !known.contains_key(id));
        *write(&self.known) = known;
    }

    /// My own relays, as the pool took the notes to me on them.
    fn set_own(&self, relays: Vec<RelayUrl>) {
        *write(&self.own) = relays;
    }

    /// Every relay a subscription of the module goes to: my own and those
    /// of the channels (a link held among them).
    fn listened(&self) -> HashSet<String> {
        let mut out: HashSet<String> = read(&self.own).iter().map(|r| r.as_str().to_string()).collect();
        for map in [&self.known, &self.linked] {
            out.extend(read(map).values().flatten().map(|r| r.as_str().to_string()));
        }
        out.extend(read(&self.held).values().flatten().map(|(_, r)| r.as_str().to_string()));
        out.extend(read(&self.creating).iter().map(|r| r.as_str().to_string()));
        out
    }

    fn add(&self, community_id: &str, relay: RelayUrl) {
        let mut held = write(&self.linked);
        let relays = held.entry(community_id.to_string()).or_default();
        if !relays.contains(&relay) {
            relays.push(relay);
        }
    }

    /// The relay of a link, while the returned hold lives: a preview, or
    /// an opening that keeps it (`RouteHold::keep`) once it succeeded. A
    /// hold that ends leaves the routes of the others as they are.
    pub(crate) fn hold(&self, community_id: &str, relay: RelayUrl) -> RouteHold {
        let id = self.next_hold.fetch_add(1, Ordering::Relaxed);
        write(&self.held).entry(community_id.to_string()).or_default().push((id, relay.clone()));
        RouteHold { routes: self.clone(), community_id: community_id.to_string(), id, relay }
    }
}

/// A relay of a link routed for as long as it lives.
pub(crate) struct RouteHold {
    routes: Routes,
    community_id: String,
    id: u64,
    relay: RelayUrl,
}

impl RouteHold {
    /// The channel was opened: its link's relay stays until its row names
    /// its own.
    pub(crate) fn keep(self) {
        self.routes.add(&self.community_id, self.relay.clone());
    }
}

impl Drop for RouteHold {
    fn drop(&mut self) {
        let mut held = write(&self.routes.held);
        if let Some(list) = held.get_mut(&self.community_id) {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                held.remove(&self.community_id);
            }
        }
    }
}

/// Those waiting for the card of a channel they are not in (the preview of
/// its link), or for its journal (the link opened), by the channel: the
/// core keeps no card of a channel without its journal, and both come as
/// a catch-up does, so the pool's hand-off gives them to the waiting as
/// they come, before the loop's queue (живой запуск 2026-10-23: the
/// journal of a link opened right after a start waited 32 s behind a
/// round's catch-up, «Канал не ответил»).
/// Who waits for the events of one channel: a number each, and where to send.
type Waiting = Vec<(u64, mpsc::UnboundedSender<serde_json::Value>)>;

#[derive(Clone, Default)]
pub(crate) struct Previews {
    waiting: Arc<RwLock<HashMap<String, Waiting>>>,
    next: Arc<std::sync::atomic::AtomicU64>,
}

/// While held, the cards of one channel come to its receiver.
pub(crate) struct Watching {
    previews: Previews,
    community_id: String,
    id: u64,
}

impl Drop for Watching {
    fn drop(&mut self) {
        let mut waiting = write(&self.previews.waiting);
        if let Some(list) = waiting.get_mut(&self.community_id) {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                waiting.remove(&self.community_id);
            }
        }
    }
}

impl Previews {
    pub(crate) fn watch(&self, community_id: &str) -> (Watching, mpsc::UnboundedReceiver<serde_json::Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        write(&self.waiting).entry(community_id.to_string()).or_default().push((id, tx));
        (Watching { previews: self.clone(), community_id: community_id.to_string(), id }, rx)
    }

    /// A card (`d`) or an operation of a journal (`h`) heard from a relay,
    /// to whoever waits for its channel.
    pub(crate) fn offer(&self, signed: &serde_json::Value) {
        let tag = match u16::try_from(signed["kind"].as_u64().unwrap_or(0)).unwrap_or(0) {
            communities_core::wire::KIND_CARD => "d",
            communities_core::wire::KIND_JOURNAL => "h",
            _ => return,
        };
        let d = signed["tags"].as_array().and_then(|tags| tags.iter().find(|t| t[0] == tag).and_then(|t| t[1].as_str()));
        if let Some(list) = d.and_then(|d| read(&self.waiting).get(d).cloned()) {
            for (_, tx) in list {
                let _ = tx.send(signed.clone());
            }
        }
    }
}

/// After the relays of a link answered (EOSE, or the pool's patience),
/// how long what they sent may still be on its way to the one who asked.
const JOURNAL_SETTLE: Duration = Duration::from_secs(2);
/// A link's relay not connected yet (the app just started): asked again
/// this often, until `JOURNAL_WAIT` runs out.
const JOURNAL_RETRY: Duration = Duration::from_secs(1);

/// Ends a task when it goes.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The journal of a channel not here yet, asked of the relays of its link
/// and taken by the one who asked as its operations come: never in the
/// queue of the loop, where a round's catch-up of days of events, or a
/// keeper's turn, held it (живой запуск 2026-10-23: the first «Войти»
/// after a start, the journal 32 s late, «Канал не ответил»). Done when the
/// journal is here; the end is the relays' answer (`ask_journal` returns
/// once each answered), not a fixed wait: a relay that answered without
/// the journal is said at once, one not connected is asked again, and
/// only one that never answers waits out `wait`. What taking it led to.
pub(crate) async fn take_journal(service: &ChannelService, previews: &Previews, community_id: &str, wait: Duration) -> CmdResult<Vec<Outcome>> {
    let keys = service.signer().ok_or_else(|| AppError::Other(format!("{NO_SESSION}: the messenger has no session")))?;
    let (_watching, mut came) = previews.watch(community_id);
    let deadline = tokio::time::Instant::now() + wait;
    let mut outcomes = Vec::new();
    let mut why = String::from("the relay of the link did not answer");
    'ask: loop {
        // A task of its own: a request held mid-way while an operation is
        // taken here could hold the very connection of the database the
        // taking waits for.
        let asked = {
            let (service, cid) = (service.clone(), community_id.to_string());
            tokio::spawn(async move { service.ask_journal(&cid).await })
        };
        let _ask_ends = AbortOnDrop(asked.abort_handle());
        let ask = async move { asked.await.unwrap_or_else(|e| Err(MessengerError::Transport(e.to_string()))) };
        tokio::pin!(ask);
        let mut asking = true;
        let mut settle = deadline;
        loop {
            if service.log(community_id).await.map_err(map_err)?.is_some() {
                return Ok(outcomes);
            }
            tokio::select! {
                biased;
                Some(signed) = came.recv() => {
                    if signed["kind"] != communities_core::wire::KIND_JOURNAL {
                        continue;
                    }
                    let lock = service.lock_of(community_id).await;
                    let _guard = lock.lock().await;
                    match service.on_event(&keys, &signed, false).await {
                        Ok(outcome) => outcomes.push(outcome),
                        Err(e) => crate::log::clog!("an operation of the journal of {} was not taken: {e}", &community_id[..community_id.len().min(8)]),
                    }
                }
                answered = &mut ask, if asking => {
                    asking = false;
                    match answered {
                        Ok(()) => {
                            why = String::from("the relay of the link holds no journal of the channel");
                            settle = (tokio::time::Instant::now() + JOURNAL_SETTLE).min(deadline);
                        }
                        Err(e) => {
                            why = format!("the relay of the link was not reached: {e}");
                            let again = tokio::time::Instant::now() + JOURNAL_RETRY;
                            if again >= deadline {
                                break 'ask;
                            }
                            tokio::time::sleep_until(again).await;
                            continue 'ask;
                        }
                    }
                }
                _ = tokio::time::sleep_until(settle) => break 'ask,
            }
        }
    }
    if service.log(community_id).await.map_err(map_err)?.is_some() {
        return Ok(outcomes);
    }
    Err(AppError::Other(format!("channel_not_here_yet: {why}")))
}

/// One piece of the listening at a time (the live subscriptions, the
/// catch-up: each its own), and whether another was asked for while it
/// ran.
#[derive(Default)]
struct Run {
    asked: AtomicBool,
    running: AtomicBool,
    task: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    /// What the last one that ended did not get out, in one line.
    failed: std::sync::Mutex<Option<String>>,
}

impl Run {
    /// Asks for one and runs `work` until no ask is left, unless one runs
    /// already: that one takes the ask with it when it ends, so asks never
    /// pile up.
    fn ask<F, Fut>(&self, me: &Listener, pick: fn(&Flight) -> &Run, work: F)
    where
        F: Fn(Listener) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<String>> + Send + 'static,
    {
        self.asked.store(true, Ordering::SeqCst);
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = me.clone();
        let task = tokio::spawn(async move {
            loop {
                let run = pick(&me.flight);
                while run.asked.swap(false, Ordering::SeqCst) {
                    let failed = work(me.clone()).await;
                    *hold(&run.failed) = failed;
                    me.settle();
                }
                run.running.store(false, Ordering::SeqCst);
                // An ask that came after the last look found it still
                // running and left it to this one.
                if !run.asked.load(Ordering::SeqCst) || run.running.swap(true, Ordering::SeqCst) {
                    break;
                }
            }
        });
        *hold(&self.task) = Some(task.abort_handle());
    }

    fn abort(&self) {
        if let Some(task) = hold(&self.task).take() {
            task.abort();
        }
    }
}

/// The listening of the channels: the live subscriptions and the catch-up,
/// each one at a time, never one waiting for the other.
#[derive(Default)]
struct Flight {
    /// The live subscriptions, made again: a new channel, a new inbox of
    /// a keeper, connections made anew. Never behind a catch-up (живой
    /// запуск 2026-10-23: a join to the phone's keeper came 92 s late, with
    /// a catch-up: the subscriptions of a round waited until the catch-up
    /// of the round before had paged days of events on a debug build).
    live: Run,
    /// The catch-up of what came while nothing listened: after the live
    /// subscriptions it follows, so nothing falls between.
    catch_up: Run,
    /// The ids subscribed by the last round: what a later round no longer
    /// asks for, and everything at a stop, is closed, or the pool would
    /// keep it (and, without the routes, hand it to the messenger).
    subscribed: std::sync::Mutex<HashSet<String>>,
    /// The module stopped: a command still on its way asks for no round.
    closed: AtomicBool,
    /// Rounds in a row that did not go out whole: how long the next try
    /// waits (`RETRY_WAITS`).
    failed: std::sync::atomic::AtomicUsize,
    /// The try of a round that did not go out whole, waiting.
    retry: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
    /// The relays of the pool connected when the last round started: one
    /// connected since is listened to again (`watch_relays`).
    connected: std::sync::Mutex<HashSet<String>>,
    /// Where the notes to me were last asked for, as said on the terminal.
    notes_on: std::sync::Mutex<String>,
    /// Each subscription as it went out (its filter but `since`, and where
    /// to), and what the pool answered: one the same is not sent again by
    /// a later round. Sent again, it closed and opened at the relay, and
    /// the notes to me on my relays came again for two days back each
    /// round (живой запуск 2026-10-23). Forgotten with the connections.
    sent: std::sync::Mutex<HashMap<String, (String, messenger_core::Ack)>>,
    /// The notes alone, asked for by a wake of the keeper while the live
    /// subscriptions are up (`woken`): never a round of its own.
    notes: Run,
    /// When the relays were last looked at (`watch_relays`), unix seconds:
    /// a look long after the one before is an app that slept.
    looked: std::sync::Mutex<Option<u64>>,
}

/// A look at the relays this long after the one before (`SESSION_CHECK`
/// apart while the app runs) is an app that slept: a phone off the screen
/// whose connections may have dropped and come back unseen in between,
/// and the notes subscriptions, which bring nothing of the past, missed
/// what came meanwhile.
const SLEPT_SECS: u64 = 120;

/// The waits before a round of listening that did not go out whole is
/// tried again, the last one for good: the app listens as it starts, and
/// the messenger's pool may have no relay yet (живой запуск 2026-10-23:
/// `no relay connected` seven times, and the keeper never heard a join).
const RETRY_WAITS: [Duration; 4] = [Duration::from_secs(5), Duration::from_secs(15), Duration::from_secs(60), Duration::from_secs(300)];

/// Makes the subscriptions of the channels again, off the loop: the
/// catch-up waits for the relays, and what they send comes back through
/// the loop, so a loop that waited for it could wait for itself.
#[derive(Clone)]
pub(crate) struct Listener {
    service: ChannelService,
    net: Arc<dyn Transport>,
    routes: Routes,
    flight: Arc<Flight>,
    /// `RETRY_WAITS`; shorter in the tests.
    waits: Arc<[Duration]>,
}

/// The subscription of the notes to me on my own relays: `channel::notes`,
/// the id `sub_id` gives the notes no channel names, so the pool hands
/// them to the module.
fn own_notes_sub() -> messenger_core::SubId {
    communities_core::transport::sub_id("", PART_NOTES)
}

/// A subscription of the notes of a channel (`channel:<id>:notes`, and
/// `…:notes:<key>` of a keeper's own key).
fn is_notes_sub(id: &str) -> bool {
    id.ends_with(&format!(":{PART_NOTES}")) || id.contains(&format!(":{PART_NOTES}:"))
}

impl Listener {
    fn new(service: ChannelService, net: Arc<dyn Transport>, routes: Routes) -> Self {
        Self { service, net, routes, flight: Arc::default(), waits: Arc::new(RETRY_WAITS) }
    }

    /// Asks for the subscriptions anew, and the catch-up after them, and
    /// returns at once. The subscriptions go out at once, whatever a
    /// catch-up asked for before still pages through; asks that come while
    /// either runs make one more of it, not one each.
    pub(crate) fn resubscribe(&self) {
        if self.flight.closed.load(Ordering::SeqCst) {
            return;
        }
        self.flight.live.ask(self, |f| &f.live, |me: Listener| async move {
            let failed = me.subscribe().await;
            // What came before the subscriptions: once they are out.
            if !me.flight.closed.load(Ordering::SeqCst) {
                me.flight.catch_up.ask(&me, |f| &f.catch_up, |me: Listener| async move { me.catch_up().await });
            }
            failed
        });
    }

    /// Everything asked for again from the start: the subscriptions, and
    /// the notes of every key paged again by the catch-up. A relay
    /// connected anew, a new pool: what was sent may be gone, and what came
    /// meanwhile was heard by nobody.
    pub(crate) fn resubscribe_anew(&self) {
        self.forget();
        self.resubscribe();
    }

    /// A wake of the keeper (a push for it, the periodic wake of a phone):
    /// the relays looked at (one connected anew, or an app that slept, has
    /// everything asked for anew), and otherwise nothing of the live
    /// subscriptions is touched while they are up: a round already on its
    /// way takes the wake with it, and one that did not go out whole is
    /// asked for again. Up, only the notes are caught up, by the rule of
    /// the rounds (`catch_up_notes`): a note that came while the phone
    /// slept. Every wake had made a whole round, every channel paged again
    /// (живой запуск 2026-10-23).
    pub(crate) async fn woken(&self) {
        if self.flight.closed.load(Ordering::SeqCst) {
            return;
        }
        self.watch_relays().await;
        let live = &self.flight.live;
        if live.running.load(Ordering::SeqCst) || live.asked.load(Ordering::SeqCst) {
            return;
        }
        if self.flight.failed.load(Ordering::SeqCst) > 0 || hold(&self.flight.subscribed).is_empty() {
            self.resubscribe();
            return;
        }
        self.flight.notes.ask(self, |f| &f.notes, |me: Listener| async move { me.catch_up_notes().await });
    }

    fn forget(&self) {
        hold(&self.flight.sent).clear();
        self.service.forget_caught();
    }

    /// A subscription to the pool, unless the same went out already in
    /// this run (`Flight::sent`); then what the pool answered it.
    async fn send_once(&self, out: Outbound) -> messenger_core::Result<messenger_core::Ack> {
        let Outbound::Subscribe { id, filter, scope } = &out else { return self.net.send(out).await };
        let mut same = filter.0.clone();
        if let Some(f) = same.as_object_mut() {
            f.remove("since");
        }
        let to = match scope {
            Scope::Channel { id } => self.routes.of(id).iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>().join(" "),
            other => format!("{other:?}"),
        };
        let mark = format!("{same} @ {to}");
        if let Some((sent, ack)) = hold(&self.flight.sent).get(&id.0) {
            if *sent == mark {
                return Ok(ack.clone());
            }
        }
        let id = id.0.clone();
        let ack = self.net.send(out).await?;
        // One no relay took is not the same sent: the next round asks again.
        if !ack.accepted_by.is_empty() {
            hold(&self.flight.sent).insert(id, (mark, ack.clone()));
        }
        Ok(ack)
    }

    /// Whether a part of the listening runs or waits to run.
    #[cfg(test)]
    fn busy(&self) -> bool {
        [&self.flight.live, &self.flight.catch_up, &self.flight.notes].iter().any(|r| r.running.load(Ordering::SeqCst) || r.asked.load(Ordering::SeqCst))
    }

    /// Ends what runs and closes every subscription made (the module stops).
    async fn close(&self) {
        self.flight.closed.store(true, Ordering::SeqCst);
        self.flight.live.abort();
        self.flight.catch_up.abort();
        self.flight.notes.abort();
        if let Some(retry) = hold(&self.flight.retry).take() {
            retry.abort();
        }
        hold(&self.flight.sent).clear();
        let ids: Vec<String> = hold(&self.flight.subscribed).drain().collect();
        for id in ids {
            let _ = self.net.send(Outbound::Unsubscribe { id: messenger_core::SubId(id) }).await;
        }
    }

    /// The routes of every channel known here, from their rows.
    pub(crate) async fn refresh_routes(&self) {
        let rows = match db::channels::list(self.service.db()).await {
            Ok(rows) => rows,
            Err(e) => {
                crate::log::clog!("the channels were not read: {e}");
                return;
            }
        };
        let mut known: HashMap<String, Vec<RelayUrl>> = rows
            .into_iter()
            .map(|row| {
                let relays: Vec<String> = serde_json::from_str(&row.relays_json).unwrap_or_default();
                (row.id, relays.iter().filter_map(|r| RelayUrl::parse(r)).collect())
            })
            .collect();
        // A channel my other device is in, whose journal is asked of the
        // relays its note named (wire §6.4): routed there until its row is.
        match self.service.own_waits().await {
            Ok(waits) => {
                for (cid, relays) in waits {
                    known.entry(cid).or_insert_with(|| relays.iter().filter_map(|r| RelayUrl::parse(r)).collect());
                }
            }
            Err(e) => crate::log::clog!("the channels my other device is in were not read: {e}"),
        }
        self.routes.set(known);
    }

    /// One whole round, the subscriptions and then the catch-up, in the
    /// one who asks (the tests). What did not go out, in one line.
    #[cfg(test)]
    async fn listen(&self) -> Option<String> {
        let live = self.subscribe().await;
        let past = self.catch_up().await;
        live.or(past)
    }

    /// The routes, the live subscriptions of every channel and the notes to
    /// me (`ChannelService::subscriptions`, the ids kept), and the close of
    /// those no longer asked for: a channel forgotten or left. Nothing of
    /// the catch-up is read or asked for here: the live ones never wait for
    /// it (живой запуск 2026-10-23). What did not go out, in one line;
    /// `None` when everything did.
    async fn subscribe(&self) -> Option<String> {
        self.refresh_routes().await;
        *hold(&self.flight.connected) = self.connected_now().await;
        // No identity at work: nothing to ask for, nothing lost.
        let keys = self.service.signer()?;
        let mut now: HashSet<String> = HashSet::new();
        let mut notes_on: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let (mut total, mut lost, mut first) = (0usize, 0usize, None::<String>);
        let complete = match self.service.subscriptions(&keys).await {
            Ok(requests) => {
                for out in requests {
                    let mut notes = false;
                    if let Outbound::Subscribe { id, .. } = &out {
                        now.insert(id.0.clone());
                        notes = is_notes_sub(&id.0);
                    }
                    total += 1;
                    match self.send_once(out).await {
                        Ok(ack) if notes => notes_on.extend(ack.accepted_by.iter().map(|r| r.as_str().to_string())),
                        Ok(_) => {}
                        Err(e) => {
                            lost += 1;
                            first.get_or_insert_with(|| e.to_string());
                        }
                    }
                }
                true
            }
            Err(e) => {
                first = Some(format!("the channels were not read: {e}"));
                false
            }
        };
        now.insert(own_notes_sub().0);
        total += 1;
        match self.own_notes(&keys).await {
            Ok(ack) => {
                // The route of my own relays, for their past in the catch-up.
                if !ack.accepted_by.is_empty() {
                    self.routes.set_own(ack.accepted_by.clone());
                }
                notes_on.extend(ack.accepted_by.iter().map(|r| r.as_str().to_string()));
            }
            Err(e) => {
                lost += 1;
                first.get_or_insert_with(|| format!("the notes to me: {e}"));
            }
        }
        self.say_notes_on(&keys, &notes_on);
        // A round that could not read the channels closes nothing.
        let gone: Vec<String> = {
            let mut subscribed = hold(&self.flight.subscribed);
            let gone = if complete { subscribed.difference(&now).cloned().collect() } else { Vec::new() };
            if complete {
                *subscribed = now;
            } else {
                subscribed.extend(now);
            }
            gone
        };
        for id in gone {
            hold(&self.flight.sent).remove(&id);
            let _ = self.net.send(Outbound::Unsubscribe { id: messenger_core::SubId(id) }).await;
        }
        first.map(|e| if lost > 0 { format!("{lost} of {total} subscriptions of the channels did not go out ({e})") } else { e })
    }

    /// The catch-up of every channel (`ChannelService::catch_up_round`):
    /// what came while nothing listened, page after page; the notes to a
    /// key paged already in this run, not again (its live subscription
    /// brought what came since), and the past of my own relays with them
    /// (`OWN_ROUTE`). What did not go out: then the next round pages every
    /// key again.
    async fn catch_up(&self) -> Option<String> {
        self.catch_up_of(false).await
    }

    /// The notes alone of `catch_up` (`ChannelService::catch_up_notes`).
    async fn catch_up_notes(&self) -> Option<String> {
        self.catch_up_of(true).await
    }

    async fn catch_up_of(&self, notes_only: bool) -> Option<String> {
        let keys = self.service.signer()?;
        let own: Vec<String> = self.routes.of(OWN_ROUTE).iter().map(|r| r.as_str().to_string()).collect();
        let asked = if notes_only { self.service.catch_up_notes(&keys, &own).await } else { self.service.catch_up_round(&keys, &own).await };
        let requests = match asked {
            Ok(requests) => requests,
            Err(e) => return Some(format!("the channels were not read: {e}")),
        };
        let (mut total, mut lost, mut first) = (0usize, 0usize, None::<String>);
        for out in requests {
            total += 1;
            if let Err(e) = self.net.send(out).await {
                lost += 1;
                first.get_or_insert_with(|| e.to_string());
            }
        }
        if first.is_some() {
            self.service.forget_caught();
        }
        first.map(|e| format!("{lost} of {total} requests of the catch-up did not go out ({e})"))
    }

    /// After a part of the listening: one that did not go out whole is
    /// tried again after a wait that grows (`RETRY_WAITS`), until both go
    /// out; nothing else would ask for it while the channels and the
    /// connections stay as they are.
    fn settle(&self) {
        let failed = hold(&self.flight.live.failed).clone().or_else(|| hold(&self.flight.catch_up.failed).clone());
        if let Some(retry) = hold(&self.flight.retry).take() {
            retry.abort();
        }
        let Some(why) = failed else {
            if self.flight.failed.swap(0, Ordering::SeqCst) > 0 {
                crate::log::clog!("the channels are listened to again");
            }
            return;
        };
        if self.flight.closed.load(Ordering::SeqCst) {
            return;
        }
        let n = self.flight.failed.fetch_add(1, Ordering::SeqCst);
        let wait = self.waits[n.min(self.waits.len().saturating_sub(1))];
        crate::log::clog!("{why}; tried again in {} s", wait.as_secs());
        let me = self.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            me.resubscribe();
        });
        *hold(&self.flight.retry) = Some(task.abort_handle());
    }

    /// The relays of the pool connected now.
    async fn connected_now(&self) -> HashSet<String> {
        self.net.status().await.relays.into_iter().filter(|r| r.state == RelayState::Connected).map(|r| r.url.as_str().to_string()).collect()
    }

    /// A round when a relay the module listens to connected since the last
    /// one started: the messenger's pool comes up after the module started,
    /// and a relay that came back missed what was sent meanwhile; or when
    /// the app slept (`SLEPT_SECS`). A relay of the pool the module asks
    /// nothing of does not count: the inbox relay of a peer, which the pool
    /// reaches for a moment to send there, came and went with every such
    /// send, and every time everything was asked for anew. Looked at with
    /// the session (`SESSION_CHECK`) and at a wake of the keeper.
    pub(crate) async fn watch_relays(&self) {
        let at = now_secs();
        let slept = hold(&self.flight.looked).replace(at).map(|last| at.saturating_sub(last)).filter(|gap| *gap >= SLEPT_SECS);
        let now = self.connected_now().await;
        // Kept at every look: a relay that dropped and came back between two
        // rounds counts as connected anew.
        let came: Vec<String> = {
            let mut seen = hold(&self.flight.connected);
            let mut came: Vec<String> = now.difference(&seen).cloned().collect();
            came.sort();
            *seen = now;
            came
        };
        // Before the notes to me were taken anywhere, my own relays are not
        // known: every relay counts.
        let came: Vec<String> = if self.routes.of(OWN_ROUTE).is_empty() {
            came
        } else {
            let listened = self.routes.listened();
            came.into_iter().filter(|r| listened.contains(r)).collect()
        };
        if let Some(gap) = slept {
            crate::log::clog!("the app slept {gap} s: the channels are listened to again");
        } else if came.is_empty() {
            return;
        } else {
            crate::log::clog!("connected to {}: the channels are listened to again", came.join(", "));
        }
        // What waited for a relay goes now, not at its next try.
        if let Err(e) = self.service.outbox_relays_came().await {
            crate::log::clog!("the outbox was not woken: {e}");
        }
        self.resubscribe_anew();
    }

    /// The notes to me on my own relays, whatever channels I follow: an
    /// invitation from a channel I know nothing of yet, and the copies of
    /// my other devices, go to my inbox, where no channel's subscription
    /// looks. New ones only (`limit` 0, from two days back: a wrap is dated
    /// back so far), as the notes of a channel: what came before is the
    /// catch-up's (`OWN_ROUTE`). Asked for from where they were heard, the
    /// subscription came back with two days of every wrap to me (the
    /// messenger's direct messages, receipts, presence) each time it went
    /// out (a start, a relay connected anew, the pool's own subscribing
    /// again after a reconnect), handed to the module as live, ahead of a
    /// join that came meanwhile (живой запуск 2026-10-23: a join to the
    /// phone's keeper taken 53–84 s late, old joins «heard already» again
    /// and again).
    async fn own_notes(&self, keys: &Keys) -> messenger_core::Result<messenger_core::Ack> {
        let me = keys.public_key().to_hex();
        let since = i64::try_from(now_secs()).unwrap_or(i64::MAX) - WRAP_MARGIN_SECS;
        let filter = serde_json::json!({ "kinds": [communities_core::notes::KIND_WRAP], "#p": [me], "since": since, "limit": 0 });
        self.send_once(Outbound::Subscribe { id: own_notes_sub(), filter: Filter(filter), scope: Scope::Own }).await
    }

    /// Where the notes to me (and to a keeper kept here) are asked for, said
    /// when it changes: the relays that took the subscriptions. A join that
    /// never comes is first looked for here (живой запуск 2026-10-23).
    fn say_notes_on(&self, keys: &Keys, relays: &std::collections::BTreeSet<String>) {
        let me = keys.public_key().to_hex();
        let line = if relays.is_empty() {
            format!("the notes to {} are asked for on no relay yet", &me[..8])
        } else {
            format!("the notes to {} are asked for on {}", &me[..8], relays.iter().cloned().collect::<Vec<_>>().join(", "))
        };
        let mut said = hold(&self.flight.notes_on);
        if *said != line {
            crate::log::clog!("{line}");
            *said = line;
        }
    }
}

/// The uploads of files running in the background, by their upload id:
/// a stop ends them, and none starts after it.
#[derive(Clone, Default)]
pub(crate) struct Uploads {
    running: Arc<std::sync::Mutex<(bool, HashMap<String, tokio::task::AbortHandle>)>>,
}

impl Uploads {
    /// Runs `work` as the upload `id`, gone from the list when it ends.
    pub(crate) fn spawn(&self, id: &str, work: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut running = hold(&self.running);
        let (list, key) = (self.clone(), id.to_string());
        let task = tokio::spawn(async move {
            work.await;
            hold(&list.running).1.remove(&key);
        });
        if running.0 {
            task.abort();
            return;
        }
        running.1.insert(id.to_string(), task.abort_handle());
    }


    /// Ends them all, and any that would start later.
    fn close(&self) {
        let mut running = hold(&self.running);
        running.0 = true;
        for (_, task) in running.1.drain() {
            task.abort();
        }
    }
}

/// The sender of what the commands and the keepers leave in the outbox
/// (`ChannelService::flush_outbox`, `cm_outbox`): one task beside the
/// loop, woken by every row queued, asleep until the next row is due. What
/// it sends is told to the page (`channel.outbox`), and a journal
/// operation out wakes the keepers' turn, which waited for it. A stop ends
/// it, and none starts after.
#[derive(Clone, Default)]
pub(crate) struct Outbox {
    task: Arc<std::sync::Mutex<(bool, Option<tokio::task::AbortHandle>)>>,
}

/// How long the outbox waits after a pass that failed as a whole (the
/// database, not a relay).
const OUTBOX_RETRY: Duration = Duration::from_secs(15);

impl Outbox {
    pub(crate) fn start(&self, service: ChannelService, signals: mpsc::UnboundedSender<Signal>, emit: impl Fn(&UiEvent) + Send + Sync + 'static) {
        let mut task = hold(&self.task);
        if task.0 {
            return;
        }
        if let Some(old) = task.1.take() {
            old.abort();
        }
        let run = tokio::spawn(async move {
            let notify = service.outbox_notify();
            loop {
                let wait = match service.flush_outbox().await {
                    Ok(f) => {
                        for e in &f.ui {
                            emit(e);
                        }
                        if !f.woke.is_empty() {
                            let _ = signals.send(Signal::Wake(now_secs()));
                        }
                        f.wake.map(|at| Duration::from_secs(at.saturating_sub(now_secs())))
                    }
                    Err(e) => {
                        crate::log::clog!("the outbox was not sent: {e}");
                        Some(OUTBOX_RETRY)
                    }
                };
                // A row queued during the pass is not missed: `notify_one`
                // keeps a permit.
                match wait {
                    Some(d) => tokio::select! {
                        () = notify.notified() => {}
                        () = tokio::time::sleep(d) => {}
                    },
                    None => notify.notified().await,
                }
            }
        });
        task.1 = Some(run.abort_handle());
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        hold(&self.task).1.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Ends it, and none starts later.
    fn close(&self) {
        let mut task = hold(&self.task);
        task.0 = true;
        if let Some(run) = task.1.take() {
            run.abort();
        }
    }
}

/// When the keepers' turn is next asked for (unix seconds), and the
/// notice to the task that runs it (`Keepers`): a command's outcome, an
/// event, the outbox (an operation out), a job of the hosts that ended,
/// each asks with `at`, the earliest time wins. No ask is lost: the time
/// is written before the notice, and the task reads the time before it
/// waits, while `notify_one` keeps a permit for a task not waiting yet.
#[derive(Clone, Default)]
pub(crate) struct Wake {
    at: Arc<std::sync::Mutex<Option<u64>>>,
    notify: Arc<tokio::sync::Notify>,
}

impl Wake {
    /// A turn at `t` at the latest (0: now).
    pub(crate) fn at(&self, t: u64) {
        {
            let mut at = hold(&self.at);
            *at = Some(at.map_or(t, |a| a.min(t)));
        }
        self.notify.notify_one();
    }

    /// The time asked for, left as it is.
    fn asked(&self) -> Option<u64> {
        *hold(&self.at)
    }

    /// The time asked for, taken.
    fn take(&self) -> Option<u64> {
        hold(&self.at).take()
    }

    /// Waits for an ask, at most `for_`.
    async fn wait(&self, for_: Duration) {
        tokio::select! {
            () = self.notify.notified() => {}
            () = tokio::time::sleep(for_) => {}
        }
    }
}

/// The task of the keepers' turn, beside the loop of the events: it waits
/// on the wake (and runs at least every `TICK_AT_LEAST`), runs one turn
/// (`Driver::turn`: the cycle of every keeper kept here, its jobs of the
/// hosts spawned, the page told), and sleeps until the time the turn
/// asked for. Nothing of it holds the loop: an event is heard while a
/// turn runs (живой запуск 2026-10-23: the turn ran in the loop, with its
/// uploads, and «an event of kind 1059 held the loop 83 s»). A stop ends
/// it, and none starts after.
#[derive(Clone, Default)]
pub(crate) struct Keepers {
    task: Arc<std::sync::Mutex<(bool, Option<tokio::task::AbortHandle>)>>,
}

impl Keepers {
    pub(crate) fn start<F, Fut>(&self, wake: Wake, turn: F)
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<u64>> + Send + 'static,
    {
        let mut task = hold(&self.task);
        if task.0 {
            return;
        }
        if let Some(old) = task.1.take() {
            old.abort();
        }
        let run = tokio::spawn(async move {
            let busy: Busy = Arc::default();
            let steps = Steps::default();
            let _watch = Watch::start(busy.clone(), steps.clone(), "runs", "the events are heard meanwhile, the next turn waits");
            let mut last = now_secs();
            loop {
                let due = wake.asked().unwrap_or(u64::MAX).min(last + TICK_AT_LEAST.as_secs());
                let now = now_secs();
                if now >= due {
                    wake.take();
                    last = now;
                    let started = std::time::Instant::now();
                    *hold(&busy) = Some(("the turn of the keepers".into(), started));
                    let next = steps.run(turn()).await;
                    *hold(&busy) = None;
                    steps.set(None);
                    slow(started, "the turn of the keepers", "ran");
                    // What the turn asked for; never at once (a turn that
                    // asks for «now» every time would never stop).
                    if let Some(at) = next {
                        wake.at(at.max(now_secs() + 1));
                    }
                    continue;
                }
                wake.wait(Duration::from_secs(due - now)).await;
            }
        });
        task.1 = Some(run.abort_handle());
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        hold(&self.task).1.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Ends it, and none starts later.
    fn close(&self) {
        let mut task = hold(&self.task);
        task.0 = true;
        if let Some(run) = task.1.take() {
            run.abort();
        }
    }
}

/// A piece of work asked for again and again (the count of the unread on
/// every message of a channel), done once after a short pause for all the
/// asks until it starts.
#[derive(Clone, Default)]
pub(crate) struct Coalesce {
    dirty: Arc<AtomicBool>,
    /// Closed, and the run waiting.
    task: Arc<std::sync::Mutex<(bool, Option<tokio::task::AbortHandle>)>>,
}

impl Coalesce {
    pub(crate) fn ask(&self, after: Duration, work: impl std::future::Future<Output = ()> + Send + 'static) {
        if self.dirty.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut task = hold(&self.task);
        if task.0 {
            return;
        }
        let dirty = self.dirty.clone();
        let run = tokio::spawn(async move {
            tokio::time::sleep(after).await;
            // An ask from now on is after what this run reads: one more.
            dirty.store(false, Ordering::SeqCst);
            work.await;
        });
        task.1 = Some(run.abort_handle());
    }

    /// Nothing runs from now on (the module stops, another identity).
    fn close(&self) {
        let mut task = hold(&self.task);
        task.0 = true;
        if let Some(run) = task.1.take() {
            run.abort();
        }
    }
}

/// Everything the commands and the loop share; cloneable into tasks.
#[derive(Clone)]
pub(crate) struct Driver {
    pub(crate) app: AppHandle,
    pub(crate) service: ChannelService,
    /// The messenger's relays, for what the core does not ask itself: the
    /// journal of a channel opened by its link.
    pub(crate) net: Arc<dyn Transport>,
    pub(crate) routes: Routes,
    listener: Listener,
    pub(crate) uploads: Uploads,
    /// The sender of what the commands and the keepers queued.
    outbox: Outbox,
    /// The task of the keepers' turn, and what asks it for one.
    keepers: Keepers,
    wake: Wake,
    /// The count of the unread asked for again: once for a burst.
    pub(crate) unread: Coalesce,
    pub(crate) previews: Previews,
    /// `FILES_SUBDIR` of the data directory.
    pub(crate) files: PathBuf,
    signals: mpsc::UnboundedSender<Signal>,
    /// When the keepers' cycle runs next, as last asked.
    pub(crate) next_wake: Arc<RwLock<Option<u64>>>,
    /// The keeper while the app is not on the screen of a phone.
    pub(crate) background: Arc<Background>,
}

impl Driver {
    /// Runs the keepers' cycle at once (an epoch asked for now).
    pub(crate) fn wake_now(&self) {
        self.wake.at(0);
    }

    /// The user's key, or `channel_no_session` while the messenger has none.
    pub(crate) fn keys(&self) -> CmdResult<Keys> {
        self.service.signer().ok_or_else(|| AppError::Other(format!("{NO_SESSION}: the messenger has no session")))
    }

    pub(crate) fn emit(&self, event: &UiEvent) {
        let _ = self.app.emit(EVENT, event);
    }

    /// Carry out what an action or an arrival led to: the events to the
    /// page, the subscriptions anew, the next turn of the keepers.
    pub(crate) async fn apply(&self, outcome: Outcome) {
        let message = outcome.events.iter().any(|e| e.name == communities_core::service::UI_EVENT_CHANNEL_MESSAGE);
        for event in &outcome.events {
            self.emit(event);
        }
        for note in &outcome.notes {
            crate::log::clog!("{note}");
        }
        if let Some(at) = outcome.wake {
            self.wake.at(at);
        }
        if outcome.resubscribe {
            self.resubscribe();
        }
        if !outcome.pending.is_empty() {
            self.service.outbox_notify().notify_one();
        }
        // The asks below go where the routes say: a journal asked of the
        // relays of my own note needs them read first.
        if outcome.routes {
            self.refresh_routes().await;
        }
        if message {
            crate::seams::unread_changed(self);
        }
        // What the arrival or the action wanted of the hosts and of the
        // relays, off the loop.
        self.spawn_fetches();
        self.spawn_asks();
    }

    /// Listen to the channels followed here anew, in the background.
    pub(crate) fn resubscribe(&self) {
        self.listener.resubscribe();
    }

    /// A wake of the keeper (`Listener::woken`): the listening as it needs.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) async fn woken(&self) {
        self.listener.woken().await;
    }

    /// The routes of every channel known here read again now: a channel
    /// just made has its row, and what the outbox sends of it goes to its
    /// relays before the listener's next round would read them.
    pub(crate) async fn refresh_routes(&self) {
        self.listener.refresh_routes().await;
    }

    /// One turn of the keepers' cycle (the task `Keepers`); when to come
    /// again. The jobs of the hosts it hands out run as tasks of their
    /// own (`Uploads`), each asking for a turn when it ends. The page
    /// hears of a keeper only when its state changed: a turn that changed
    /// nothing would have every open channel read again.
    async fn turn(&self) -> Option<u64> {
        let status = |cid: &str| self.service.keeper_status(cid).map(|s| communities_core::service::KeeperStatus { at: 0, ..s });
        let before: HashMap<String, _> = self.service.kept().await.unwrap_or_default().into_iter().map(|cid| { let s = status(&cid); (cid, s) }).collect();
        // The app keys of the cards of private servers, as the messenger
        // has them now: a write to an owner's own server carries its key
        // (communities-wire §9.1).
        // The owner's Veydan Cloud, where the keepers register the keys of
        // the staff (communities-wire §9.2): lent while the module `cloud` is here.
        let cloud = seams::seam::<dyn veydan_shell::seams::CloudDelegates, _, _>(&self.app);
        self.service.set_cloud(cloud.map(|c| Arc::new(CloudSeam(c)) as Arc<dyn communities_core::cloud_keys::CloudDelegates>));
        if let Some(src) = seams::seam::<dyn veydan_shell::seams::MediaKeys, _, _>(&self.app) {
            match src.app_keys().await {
                Ok(keys) => self.service.set_app_keys(&keys.into_iter().map(|(url, k)| (url, k.to_string())).collect::<Vec<_>>()),
                Err(e) => crate::log::clog!("the app keys of the private servers were not read: {e}"),
            }
        }
        match self.service.turn().await {
            Ok(turned) => {
                *write(&self.next_wake) = turned.wake;
                for cid in self.service.kept().await.unwrap_or_default() {
                    let now = status(&cid);
                    if now.is_some() && before.get(&cid) == Some(&now) {
                        continue;
                    }
                    self.emit(&UiEvent { name: UI_EVENT_CHANNEL_KEEPER.into(), payload: serde_json::json!({ "community_id": cid }) });
                }
                self.background_soon(false);
                for job in turned.jobs {
                    self.spawn_job(job);
                }
                if let Some(keys) = self.service.signer() {
                    for job in turned.fetches {
                        self.spawn_fetch(&keys, job);
                    }
                }
                self.spawn_asks();
                turned.wake
            }
            Err(e) => {
                crate::log::clog!("the cycle of the keepers failed: {e}");
                None
            }
        }
    }

    /// A job of the hosts (`keeper::upload`), off every lock, under its
    /// key in `Uploads` (a stop ends it); the keeper block of the page and
    /// the keepers' turn hear of its end.
    fn spawn_job(&self, job: communities_core::keeper::Job) {
        let d = self.clone();
        let cid = job.community_id.clone();
        self.uploads.spawn(&job.key(), async move {
            d.service.run_job(job).await;
            d.emit(&UiEvent { name: UI_EVENT_CHANNEL_KEEPER.into(), payload: serde_json::json!({ "community_id": cid }) });
            d.wake.at(now_secs());
        });
    }

    /// The fetches of the member's side due now (an epoch to open, a shard
    /// of a snapshot an event wanted), each a task of its own under its
    /// key in `Uploads` (`core::member::fetch::AT_ONCE` at most): the
    /// loop never waits for a host (живой запуск 2026-10-23: «an event of
    /// kind 1059 holds the loop for 34 s», every minute).
    fn spawn_fetches(&self) {
        let Some(keys) = self.service.signer() else { return };
        for job in self.service.take_fetches() {
            self.spawn_fetch(&keys, job);
        }
    }

    /// What the events wanted of the relays (`ChannelService::take_asks`:
    /// the page before a full page of a catch-up, the heads of the journal
    /// a snapshot names), sent by a task of its own under a key of its own
    /// in `Uploads`: the pool awaits a `Sync` to its end, up to 20 s a
    /// relay, and the loop waited for each (живой запуск 2026-10-23: «an
    /// event of kind 1059 held the loop 56 s» after every channel made).
    pub(crate) fn spawn_asks(&self) {
        let asks = self.service.take_asks();
        if asks.is_empty() {
            return;
        }
        static ASKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let key = format!("asks:{}", ASKS.fetch_add(1, Ordering::Relaxed));
        let d = self.clone();
        self.uploads.spawn(&key, async move {
            d.service.send_asks(asks).await;
        });
    }

    /// One fetch (`ChannelService::run_fetch`): the hosts off every lock,
    /// what they gave opened under the channel's lock; what it read goes
    /// to the page, its next time to the wake, the fetches it let through
    /// start (`apply`).
    fn spawn_fetch(&self, keys: &Keys, job: communities_core::member::fetch::FetchJob) {
        let (d, keys) = (self.clone(), keys.clone());
        self.uploads.spawn(&job.key(), async move {
            let out = d.service.run_fetch(&keys, job).await;
            d.apply(out).await;
        });
    }

    /// An event of a channel's relays: one channel at a time, as the
    /// journal is rebuilt on every operation; a wrap names no channel.
    async fn on_event(&self, raw: RawEvent) {
        // A preview waits for the card, an opening for the journal: given
        // them by the hand-off of the pool already (`Shared::open`).
        // Nothing is read without the identity at work (the lock closed):
        // said once, as a note to the keeper would wait unseen otherwise.
        let Some(keys) = self.service.signer() else {
            self.service.say_state("the events of the channels", Some("not read: the messenger has no identity at work (the lock is closed?)"), "read again");
            return;
        };
        self.service.say_state("the events of the channels", None, "read again");
        let live = !matches!(raw.source, EventSource::Sync { .. });
        let cid = raw.json["tags"].as_array().and_then(|tags| {
            tags.iter().find(|t| t[0] == "h" || t[0] == "d").and_then(|t| t[1].as_str()).map(str::to_string)
        });
        let outcome = match cid {
            Some(cid) => {
                self.service.mark(Some(format!("channel {}: waits for the channel's lock", cid.get(..8).unwrap_or(&cid))));
                let lock = self.service.lock_of(&cid).await;
                let _guard = lock.lock().await;
                self.service.on_event(&keys, &raw.json, live).await
            }
            None => self.service.on_event(&keys, &raw.json, live).await,
        };
        self.service.mark(Some("what the event led to, to the page".into()));
        match outcome {
            Ok(outcome) => self.apply(outcome).await,
            Err(e) => {
                crate::log::clog!("an event was not taken: {e}");
                self.spawn_asks();
            }
        }
        // A note to a keeper kept here: joins streaming in may want the
        // service of a phone.
        if raw.json["kind"] == communities_core::notes::KIND_WRAP && self.background.heard(&raw.json, now_secs()) {
            self.background_soon(false);
        }
    }

    /// The background of a phone read again, off the loop: the plugin's
    /// answers never hold it. `again`: the service asked for once more.
    pub(crate) fn background_soon(&self, again: bool) {
        if !self.background.supported() {
            return;
        }
        let d = self.clone();
        tokio::spawn(async move { d.background_check(again).await });
    }

    /// The keepers kept here, their keys and whether an epoch waits among
    /// them, to the rule of the service (`background`).
    async fn background_check(&self, again: bool) {
        let kept = self.service.kept().await.unwrap_or_default();
        let mut keepers = HashSet::new();
        let mut epoch = false;
        for cid in &kept {
            if let Ok(Some(pk)) = self.service.keeper_pubkey(cid).await {
                keepers.insert(pk.as_hex().to_string());
            }
            epoch |= self.service.keeper_status(cid).is_some_and(|s| s.pending_epoch.is_some() || s.rotate_asked);
        }
        self.background.set_keepers(keepers);
        let before = self.background.running();
        self.background.check(&kept, epoch, now_secs(), again).await;
        // «работает в фоне» on the page of each channel kept here.
        if self.background.running() != before {
            for cid in kept {
                self.emit(&UiEvent { name: UI_EVENT_CHANNEL_KEEPER.into(), payload: serde_json::json!({ "community_id": cid }) });
            }
        }
    }

    /// The loop of the module, the events of the channels' relays and the
    /// signals: until it is aborted by `stop` or by another identity at
    /// work. The keepers' turn is the task `Keepers`, not the loop's: a
    /// signal to wake it is passed on.
    async fn run(self, mut events: mpsc::UnboundedReceiver<RawEvent>, mut signals: mpsc::UnboundedReceiver<Signal>) {
        let busy: Busy = Arc::default();
        // The step of the loop, its own: the keepers' turn, the jobs and
        // the fetches mark theirs beside it.
        let steps = Steps::default();
        let _watch = Watch::start(busy.clone(), steps.clone(), "holds the loop", "nothing is heard meanwhile");
        // What came live goes before what a catch-up brought: a round hands
        // thousands of events of days past, and a join that came live
        // behind them waited minutes in the one queue (живой запуск
        // 2026-10-23: a join published at 18:54:30 taken at 18:58:26).
        let mut queue = Queue::default();
        loop {
            while let Ok(raw) = events.try_recv() {
                queue.push(raw);
            }
            while let Ok(signal) = signals.try_recv() {
                self.on_signal(signal);
            }
            if let Some(raw) = queue.next() {
                let started = std::time::Instant::now();
                let kind = raw.json["kind"].as_u64().unwrap_or(0);
                *hold(&busy) = Some((format!("an event of kind {kind}"), started));
                steps.set(Some(format!("an event of kind {kind}")));
                steps.run(self.on_event(raw)).await;
                *hold(&busy) = None;
                steps.set(None);
                slow(started, &format!("an event of kind {kind}"), "held the loop");
                continue;
            }
            tokio::select! {
                Some(raw) = events.recv() => queue.push(raw),
                Some(signal) = signals.recv() => self.on_signal(signal),
                else => return,
            }
        }
    }

    fn on_signal(&self, signal: Signal) {
        match signal {
            Signal::Wake(at) => self.wake.at(at),
            Signal::Resubscribe => self.resubscribe(),
            Signal::Note(note) => crate::log::clog!("{note}"),
        }
    }
}

/// The events the pool handed the loop: what came live first, then what a
/// catch-up brought of the channels (journals, cards, epochs, posts: what
/// opening a link waits for), then the gift wraps a catch-up brought (days
/// of them), each in the order it came.
#[derive(Default)]
struct Queue {
    live: std::collections::VecDeque<RawEvent>,
    past: std::collections::VecDeque<RawEvent>,
    past_wraps: std::collections::VecDeque<RawEvent>,
}

impl Queue {
    fn push(&mut self, raw: RawEvent) {
        if !matches!(raw.source, EventSource::Sync { .. }) {
            self.live.push_back(raw);
        } else if raw.json["kind"] == communities_core::notes::KIND_WRAP {
            self.past_wraps.push_back(raw);
        } else {
            self.past.push_back(raw);
        }
    }

    fn next(&mut self) -> Option<RawEvent> {
        self.live.pop_front().or_else(|| self.past.pop_front()).or_else(|| self.past_wraps.pop_front())
    }
}

/// What holds the loop now, and since when.
type Busy = Arc<std::sync::Mutex<Option<(String, std::time::Instant)>>>;

/// Says a piece of work of a task (the loop, the keepers' turn) that
/// still runs after [`SLOW`], every [`SLOW`], with the step it is at:
/// `slow` speaks only once it ends, and a cycle that never ends said
/// nothing (живой запуск 2026-10-23: a join queued from a catch-up waited
/// for a turn that never came, and the terminal was silent). `verb`: what
/// the work does to its task («holds the loop», «runs»); `meanwhile`: what
/// that means for the rest. Ends with the task.
struct Watch(tokio::task::AbortHandle);

impl Watch {
    fn start(busy: Busy, steps: Steps, verb: &'static str, meanwhile: &'static str) -> Self {
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(SLOW).await;
                let held = hold(&busy).clone();
                let Some((what, since)) = held else { continue };
                let took = since.elapsed();
                if took < SLOW {
                    continue;
                }
                let at = steps.now().map(|d| format!(", at: {d}")).unwrap_or_default();
                crate::log::clog!("{what} {verb} for {} s{at}: {meanwhile}", took.as_secs());
            }
        });
        Self(task.abort_handle())
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How long one piece of work of the loop may take before it is said: the
/// loop takes one at a time, and nothing is heard while one runs.
const SLOW: Duration = Duration::from_secs(20);

/// Says a piece of work of a task that took longer than [`SLOW`]: a
/// keeper silent for minutes may be one that waits on a host.
fn slow(started: std::time::Instant, what: &str, verb: &str) {
    let took = started.elapsed();
    if took >= SLOW {
        crate::log::clog!("{what} {verb} {} s", took.as_secs());
    }
}

/// The way from a keeper's key to its channel and owner, for the wake of
/// the owner's phone (`wake`): over the channels known here, by their
/// journals; none for my own channel and with no identity at work.
fn resolver(service: ChannelService) -> Resolve {
    Arc::new(move |keeper| {
        let service = service.clone();
        Box::pin(async move {
            let from = service.signer()?;
            let me = from.public_key().to_hex();
            for row in db::channels::list(service.db()).await.ok()? {
                let Ok(Some(log)) = service.log(&row.id).await else { continue };
                let state = log.state();
                if state.keeper().map(|k| &k.pubkey) != Some(&keeper) {
                    continue;
                }
                if state.owner.as_hex() == me {
                    return None;
                }
                return Some(Target { community_id: row.id, owner: state.owner.clone(), from });
            }
            None
        })
    })
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The database of an identity: one file each in the module's folder, so
/// that another identity at work on this device never sees the channels,
/// the roles and the read marks of the one before. A `communities.db` of
/// the builds before is not opened any more and stays as it is.
pub(crate) fn db_path(dir: &Path, keys: &Keys) -> PathBuf {
    use nostr::nips::nip19::ToBech32;
    let name = keys.public_key().to_bech32().unwrap_or_else(|_| keys.public_key().to_hex());
    dir.join(format!("{name}.db"))
}

/// The driver of the identity at work and its loop.
struct Running {
    pubkey: String,
    driver: Driver,
    task: tauri::async_runtime::JoinHandle<()>,
    /// The messenger's network with the keeper's wake (`wake`).
    waking: Arc<WakingNet>,
    /// The rule of a phone's service read again now and then.
    recheck: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl Running {
    /// Ends the loop, the uploads and the subscriptions, and closes the
    /// database; the keepers held in memory go with the service.
    async fn close(self) {
        self.task.abort();
        if let Some(recheck) = &self.recheck {
            recheck.abort();
        }
        self.driver.background.close().await;
        // The service held the network, and the network the service.
        self.waking.set_resolve(None);
        self.driver.keepers.close();
        self.driver.uploads.close();
        self.driver.outbox.close();
        self.driver.unread.close();
        self.driver.listener.close().await;
        self.driver.service.set_signer(None);
        self.driver.service.db().close().await;
    }
}

/// What the module and its watcher of the messenger's session share.
#[derive(Clone)]
struct Shared {
    app: AppHandle,
    dir: PathBuf,
    files: PathBuf,
    running: Arc<RwLock<Option<Running>>>,
    /// One change at a time: a start, a stop, another identity.
    switching: Arc<Mutex<()>>,
    /// The generation of the messenger's connections the identity at work
    /// was last listened to with.
    seen: Arc<std::sync::Mutex<Option<u64>>>,
    /// The keeper on a phone: one for the app, the owner's switches in it.
    background: Arc<Background>,
}

impl Shared {
    async fn follow(&self) {
        let _switching = self.switching.lock().await;
        self.follow_held().await;
    }

    /// Follows the session of the messenger, `switching` held: another
    /// identity closes what ran for the one before and opens its own
    /// database; new connections mean the subscriptions are made again
    /// and the keepers run.
    async fn follow_held(&self) {
        let Some(lend) = seams::seam::<dyn NetworkSession, _, _>(&self.app) else { return };
        let session = match lend.session().await {
            Ok(session) => session,
            Err(e) => {
                crate::log::clog!("the session of the messenger was not read: {e}");
                return;
            }
        };
        let Some(session) = session else {
            // No identity for now (the lock closed): the channels wait
            // without a signer.
            if let Some(running) = read(&self.running).as_ref() {
                running.driver.service.set_signer(None);
            }
            *hold(&self.seen) = None;
            return;
        };
        let Ok(secret) = SecretKey::from_slice(session.secret.as_slice()) else { return };
        let keys = Keys::new(secret);
        let pubkey = keys.public_key().to_hex();
        let opened = read(&self.running).as_ref().is_none_or(|r| r.pubkey != pubkey);
        if opened {
            let before = write(&self.running).take();
            if let Some(before) = before {
                before.close().await;
            }
            *hold(&self.seen) = None;
            match self.open(&keys).await {
                Ok(running) => *write(&self.running) = Some(running),
                Err(e) => {
                    crate::log::clog!("the channels of the identity were not opened: {e}");
                    route_channels(None);
                    return;
                }
            }
        }
        let Some(driver) = read(&self.running).as_ref().map(|r| r.driver.clone()) else { return };
        if hold(&self.seen).replace(session.generation) == Some(session.generation) {
            // The same pool: a relay of it that connected since is listened
            // to again (it had none yet when the app started).
            driver.listener.watch_relays().await;
            return;
        }
        driver.service.set_signer(Some(keys));
        // A new pool: nothing of what was sent to the one before is there.
        driver.listener.resubscribe_anew();
        driver.wake_now();
        crate::seams::unread_changed(&driver);
        if opened {
            driver.emit(&UiEvent { name: UI_EVENT_CHANNEL_READY.into(), payload: serde_json::json!({}) });
        }
    }

    /// The database of `keys`, the messenger's network and the loop.
    async fn open(&self, keys: &Keys) -> CmdResult<Running> {
        let net = seams::seam::<dyn Transport, _, _>(&self.app)
            .ok_or_else(|| AppError::Other(format!("{STOPPED}: the messenger is not in the product")))?;
        let db = Db::open(&db_path(&self.dir, keys)).await.map_err(map_err)?;
        let secrets: Arc<dyn SecretStore> = Arc::new(BoxSecrets { app: self.app.clone() });
        let blobs = Arc::new(MediaHosts::new().map_err(map_err)?);
        // The notes to a keeper go with the wake of its owner's phone.
        let waking = WakingNet::new(net.clone());
        let service = ChannelService::new(db, secrets, waking.clone(), blobs, Arc::new(SystemClock));
        // The way of a join, on the terminal the owner reads.
        service.set_say(Some(Arc::new(|line: &str| crate::log::clog!("{line}"))));
        waking.set_resolve(Some(resolver(service.clone())));
        // Unbounded as the pool's hand-off is (messenger-transport): the
        // pool never waits for the loop.
        let (events_tx, events) = mpsc::unbounded_channel();
        let (signals_tx, signals) = mpsc::unbounded_channel();
        let routes = Routes::default();
        let driver = Driver {
            app: self.app.clone(),
            listener: Listener::new(service.clone(), net.clone(), routes.clone()),
            service,
            net,
            routes,
            uploads: Uploads::default(),
            outbox: Outbox::default(),
            keepers: Keepers::default(),
            wake: Wake::default(),
            unread: Coalesce::default(),
            previews: Previews::default(),
            files: self.files.clone(),
            signals: signals_tx,
            next_wake: Arc::default(),
            background: self.background.clone(),
        };
        driver.listener.refresh_routes().await;
        let routes = driver.routes.clone();
        // A card a preview waits for goes to it as it comes, not when the
        // loop gets to it: behind a round's catch-up the preview of a link
        // ran out of its ten seconds («Не удалось открыть канал по ссылке»,
        // живой запуск 2026-10-23).
        let (pool_tx, mut pool_rx) = mpsc::unbounded_channel::<RawEvent>();
        let previews = driver.previews.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(raw) = pool_rx.recv().await {
                previews.offer(&raw.json);
                if events_tx.send(raw).is_err() {
                    break;
                }
            }
        });
        // A marker of a catch-up read (`ChannelService::settle`) goes down
        // the same stream, behind the pages the pool handed it: the keeper
        // is taken over only once what the relays answered was read.
        let settle_tx = pool_tx.clone();
        let me = communities_core::service::ChannelService::me(keys).map_err(map_err)?;
        driver.service.set_settle_sink(Some(Arc::new(move |json| {
            let (Some(id), Some(url)) = (messenger_core::EventId::parse(json["id"].as_str().unwrap_or_default()), RelayUrl::parse("wss://settle.invalid")) else { return false };
            settle_tx.send(RawEvent { id, kind: 0, pubkey: me.clone(), created_at: messenger_core::Timestamp(0), json, source: EventSource::Sync { url } }).is_ok()
        })));
        route_channels(Some(ChannelRoutes { relays: Arc::new(move |id| routes.of(id)), events: pool_tx }));
        let task = tauri::async_runtime::spawn(driver.clone().run(events, signals));
        // The keepers' turn beside the loop, from the start (what a run
        // before left pending: an epoch, a snapshot, the garbage).
        let turner = driver.clone();
        driver.keepers.start(driver.wake.clone(), move || {
            let d = turner.clone();
            async move { d.turn().await }
        });
        // What a run before left in the outbox goes out as this one starts.
        let emitter = driver.clone();
        driver.outbox.start(driver.service.clone(), driver.signals.clone(), move |e| emitter.emit(e));
        // The sends a run before left `uploading` (the app closed under
        // them) go on from the chunks the hosts hold, as a chat's do.
        match driver.service.interrupted_uploads().await {
            Ok(rows) => {
                for row in rows {
                    crate::commands::spawn_send(&driver, keys.clone(), row.community_id, row.id, driver.files.join(crate::views::OUTGOING_DIR));
                }
            }
            Err(e) => crate::log::clog!("unfinished uploads were not taken up again: {e}"),
        }
        // Copies no row names any more (a pick given up, a post removed).
        let sweeper = driver.clone();
        tauri::async_runtime::spawn(async move { crate::commands::sweep_outgoing(&sweeper).await });
        let recheck = driver.background.supported().then(|| {
            let driver = driver.clone();
            tauri::async_runtime::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(crate::background::RECHECK_SECS));
                loop {
                    every.tick().await;
                    driver.background_check(true).await;
                }
            })
        });
        Ok(Running { pubkey: keys.public_key().to_hex(), driver, task, waking, recheck })
    }
}

/// The state of the module, managed by its setup; `start` and `stop` fill
/// and empty it as the shell switches the module on and off.
pub struct CommunitiesState {
    shared: Shared,
    /// The module is on.
    on: AtomicBool,
    /// The watcher of the messenger's session, while the module is on.
    watcher: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    /// The wake of the push plugin is listened to (once a run of the app).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    woken: AtomicBool,
}

impl CommunitiesState {
    pub(crate) fn new(app: AppHandle, app_data_dir: &Path) -> Self {
        // The push plugin is the messenger's, set up before this module.
        #[cfg(target_os = "android")]
        let phone = crate::background::android::phone(&app);
        #[cfg(not(target_os = "android"))]
        let phone = None;
        let dir = app_data_dir.join(DATA_SUBDIR);
        let shared = Shared {
            app,
            background: Arc::new(Background::new(phone, &dir)),
            dir,
            files: app_data_dir.join(FILES_SUBDIR),
            running: Arc::default(),
            switching: Arc::default(),
            seen: Arc::default(),
        };
        Self { shared, on: AtomicBool::new(false), watcher: std::sync::Mutex::new(None), woken: AtomicBool::new(false) }
    }

    /// The driver of the identity at work; `channel_no_session` while the
    /// module is on and the messenger has none yet.
    pub(crate) fn driver(&self) -> CmdResult<Driver> {
        if let Some(running) = read(&self.shared.running).as_ref() {
            return Ok(running.driver.clone());
        }
        if self.on.load(Ordering::SeqCst) {
            return Err(AppError::Other(format!("{NO_SESSION}: the messenger has no session")));
        }
        Err(AppError::Other(format!("{STOPPED}: the module is off")))
    }

    /// Takes the messenger's network and follows its session: the
    /// database of the identity at work opens and its loop starts at once
    /// when there is one, later when one comes. Nothing happens when the
    /// module is on already.
    pub(crate) async fn start(&self) -> CmdResult<()> {
        let _switching = self.shared.switching.lock().await;
        if self.on.load(Ordering::SeqCst) {
            return Ok(());
        }
        if seams::seam::<dyn Transport, _, _>(&self.shared.app).is_none() {
            return Err(AppError::Other(format!("{STOPPED}: the messenger is not in the product")));
        }
        self.on.store(true, Ordering::SeqCst);
        // A push for the keeper, or its periodic wake (Android).
        #[cfg(target_os = "android")]
        if !self.woken.swap(true, Ordering::SeqCst) {
            crate::background::android::listen(&self.shared.app).await;
        }
        self.shared.follow_held().await;
        let shared = self.shared.clone();
        let watcher = tauri::async_runtime::spawn(async move {
            let mut check = tokio::time::interval(SESSION_CHECK);
            check.tick().await;
            loop {
                check.tick().await;
                shared.follow().await;
            }
        });
        *hold(&self.watcher) = Some(watcher);
        Ok(())
    }

    /// Ends the watcher and the loop, gives the routes back to the
    /// messenger and closes the database. Nothing happens when it is off.
    pub(crate) async fn stop(&self) {
        let _switching = self.shared.switching.lock().await;
        if !self.on.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Some(watcher) = hold(&self.watcher).take() {
            watcher.abort();
        }
        let running = write(&self.shared.running).take();
        if let Some(running) = running {
            running.close().await;
        }
        route_channels(None);
        *hold(&self.shared.seen) = None;
        crate::seams::clear_unread(&self.shared.app);
    }
}

#[cfg(test)]
#[path = "join_path_tests.rs"]
mod join_path_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use communities_core::db::channels::ChannelRow;
    use messenger_core::traits::RelayStatusSnapshot;
    use messenger_core::{Ack, PubKey};
    use std::sync::atomic::AtomicUsize;

    /// Secrets in memory.
    #[derive(Default)]
    pub(super) struct MemSecrets(std::sync::Mutex<HashMap<String, Vec<u8>>>);

    #[async_trait::async_trait]
    impl SecretStore for MemSecrets {
        async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
            Ok(self.0.lock().unwrap().get(key).cloned().map(Zeroizing::new))
        }
        async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
            self.0.lock().unwrap().insert(key.into(), value.to_vec());
            Ok(())
        }
        async fn delete(&self, key: &str) -> messenger_core::Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
        async fn is_unlocked(&self) -> bool {
            true
        }
    }

    /// The messenger's pool as the loop meets it: every request recorded,
    /// and every catch-up held until the gate lets it go, as the pool held
    /// it while its hand-off to the loop was full.
    struct FakeNet {
        gate: tokio::sync::Semaphore,
        syncs: AtomicUsize,
        sent: std::sync::Mutex<Vec<Outbound>>,
    }

    impl FakeNet {
        fn new(open: bool) -> Arc<Self> {
            let permits = if open { tokio::sync::Semaphore::MAX_PERMITS } else { 0 };
            Arc::new(Self { gate: tokio::sync::Semaphore::new(permits), syncs: AtomicUsize::new(0), sent: Default::default() })
        }

        fn syncs(&self) -> usize {
            self.syncs.load(Ordering::SeqCst)
        }

        fn sent(&self) -> Vec<Outbound> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Transport for FakeNet {
        async fn send(&self, out: Outbound) -> messenger_core::Result<Ack> {
            if matches!(out, Outbound::Sync { .. }) {
                self.syncs.fetch_add(1, Ordering::SeqCst);
                let _pass = self.gate.acquire().await;
            }
            self.sent.lock().unwrap().push(out);
            // Taken by the one relay: a subscription no relay took is sent
            // again by the next round (`send_once`).
            Ok(Ack { accepted_by: vec![RelayUrl::parse("wss://r.example").expect("url")], rejected_by: vec![] })
        }

        fn events(&self) -> mpsc::Receiver<RawEvent> {
            mpsc::channel(1).1
        }

        async fn status(&self) -> RelayStatusSnapshot {
            RelayStatusSnapshot::default()
        }
    }

    fn joined(id: &str) -> ChannelRow {
        ChannelRow {
            id: id.into(),
            out_reason: None,
            keeper_last_seen: None,
            name: "c".into(),
            about: String::new(),
            picture: String::new(),
            relays_json: r#"["wss://r.example"]"#.into(),
            storage_json: "[]".into(),
            owner: "0".repeat(64),
            join_mode: "open".into(),
            membership: communities_core::service::MEMBERSHIP_JOINED.into(),
            my_role: None,
            members: 1,
            link_epoch: 0,
            created_at: 1,
            updated_at: 1,
            needs_update: false,
        }
    }

    /// A service on a database in memory, signed in, over `net`.
    async fn service(net: Arc<FakeNet>) -> ChannelService {
        let db = Db::open_in_memory().await.unwrap();
        let blobs = Arc::new(MediaHosts::new().unwrap());
        let service = ChannelService::new(db, Arc::new(MemSecrets::default()), net, blobs, Arc::new(SystemClock));
        service.set_signer(Some(Keys::generate()));
        service
    }

    async fn until(what: impl Fn() -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !what() {
            assert!(tokio::time::Instant::now() < deadline, "waited too long");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A refusal of a blob host is a code of its own, its answer after it;
    /// a picture that is none, or too large, too (живой запуск 2026-10-22).
    #[test]
    fn the_refusals_of_the_cover_have_codes_of_their_own() {
        let refused = format!("{}: https://h1.example: err.auth_failed: no_account", communities_core::media::UPLOAD_REFUSED);
        assert_eq!(code_of(&refused), "channel_upload_refused");
        assert_eq!(code_of("channel: a cover is not a picture"), "channel_cover_unsupported");
        assert_eq!(code_of("channel: a cover is too large"), "channel_cover_too_large");
        // What the media code really says (`channel media: …`).
        assert_eq!(code_of("channel media: a cover is not a picture"), "channel_cover_unsupported");
        assert_eq!(code_of("channel media: a cover is too large"), "channel_cover_too_large");
        assert_eq!(code_of("channel media: the address of the cover"), "channel_cover_address");
        assert_eq!(code_of("channel media: not permitted to upload"), "channel_upload_not_permitted");
        let needs = messenger_core::MessengerError::Transport(format!("{}: no host of the channel takes a file of mine", communities_core::upload_keys::NEEDS_KEY));
        assert!(map_err(needs).to_string().contains("channel_storage_needs_key"), "a code of its own");
    }

    /// A round of listening whose catch-up the pool holds never holds the
    /// one who asked (the loop), and asks that come meanwhile make one
    /// more round, not one each (ревью модуля п. 1).
    #[tokio::test]
    async fn listening_never_holds_the_loop_and_asks_come_together() {
        let net = FakeNet::new(false);
        let service = service(net.clone()).await;
        db::channels::upsert(service.db(), &joined(&"c".repeat(64))).await.unwrap();
        let per_round = service.catch_up(&service.signer().unwrap()).await.unwrap().len();
        assert!(per_round > 0);
        let listener = Listener::new(service, net.clone(), Routes::default());

        listener.resubscribe();
        until(|| net.syncs() == 1).await;
        for _ in 0..5 {
            listener.resubscribe();
        }
        assert_eq!(net.syncs(), 1, "a round at a time");
        assert_eq!(listener.routes.of(&"c".repeat(64)).len(), 1, "the routes come before the catch-up");

        net.gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS);
        until(|| !listener.busy()).await;
        // The one more pages no notes again: the subscription brings them.
        assert_eq!(net.syncs(), 2 * per_round - 1, "the round asked for and one more for the asks meanwhile");
    }

    /// A round after the first of a run pages no wraps again and sends no
    /// subscription the same as one up (живой запуск 2026-10-23: every
    /// round re-paged two days of the wraps to me, twice: the catch-up of
    /// the notes and the notes to me on my relays); a relay connected anew
    /// has both asked for again.
    #[tokio::test]
    async fn a_later_round_pages_no_wraps_again_until_a_relay_comes_anew() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        let cid = "c".repeat(64);
        db::channels::upsert(service.db(), &joined(&cid)).await.unwrap();
        let listener = Listener::new(service, net.clone(), Routes::default());
        let wraps = || net.sent().iter().filter(|o| matches!(o, Outbound::Sync { filter, .. } if filter.0["kinds"] == serde_json::json!([communities_core::notes::KIND_WRAP]))).count();
        let subs = || net.sent().iter().filter(|o| matches!(o, Outbound::Subscribe { .. })).count();
        listener.listen().await;
        let (first_wraps, first_subs) = (wraps(), subs());
        assert_eq!(first_wraps, 1, "the notes to me paged once");
        assert!(first_subs > 1);
        listener.listen().await;
        assert_eq!(wraps(), first_wraps, "not paged again");
        assert_eq!(subs(), first_subs, "nothing the same sent again");
        // A new channel: its own subscriptions, nothing else again.
        db::channels::upsert(service_db(&listener), &joined(&"d".repeat(64))).await.unwrap();
        listener.listen().await;
        assert!(subs() > first_subs);
        assert_eq!(wraps(), first_wraps, "the notes of the same relays are paged already");
        let (wraps_now, subs_now) = (wraps(), subs());
        listener.forget();
        listener.listen().await;
        assert_eq!(wraps(), wraps_now + 1, "a relay connected anew: paged again");
        assert!(subs() >= subs_now + first_subs, "and subscribed again");
    }

    fn service_db(listener: &Listener) -> &Db {
        listener.service.db()
    }

    /// With no channel at all, the notes to me are listened to on my own
    /// relays under an id the pool hands to the module, new ones only, and
    /// what came before is paged on them by the catch-up; an invitation,
    /// and the copy of my other device, that come there are taken (ревью
    /// модуля п. 3).
    #[tokio::test]
    async fn the_notes_to_me_are_heard_on_my_relays_with_no_channel() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        let keys = service.signer().unwrap();
        let me = keys.public_key().to_hex();
        let listener = Listener::new(service.clone(), net.clone(), Routes::default());
        listener.listen().await;
        let own = |sent: Vec<Outbound>| {
            sent.into_iter().find_map(|o| match o {
                Outbound::Subscribe { id, filter, scope } if id == own_notes_sub() => Some((filter.0, scope)),
                _ => None,
            })
        };
        let (filter, scope) = own(net.sent()).expect("the notes to me are asked for");
        assert!(own_notes_sub().0.starts_with(messenger_transport::CHANNEL_SUB_PREFIX), "the pool hands them to the module");
        assert_eq!(scope, Scope::Own);
        assert_eq!(filter["#p"], serde_json::json!([me]));
        assert_eq!(filter["kinds"], serde_json::json!([communities_core::notes::KIND_WRAP]));
        assert_eq!(filter["limit"], 0, "new ones only: the relay hands nothing it holds back on it");
        let now = i64::try_from(now_secs()).unwrap();
        assert!(now - filter["since"].as_i64().unwrap() >= WRAP_MARGIN_SECS - 5, "a wrap dated back two days is new all the same");
        // What came before: paged by the catch-up on my relays (the route
        // of my own relays), the notes to me only.
        let past: Vec<serde_json::Value> = net
            .sent()
            .into_iter()
            .filter_map(|o| match o {
                Outbound::Sync { scope: Scope::Channel { id }, filter, .. } if id == OWN_ROUTE => Some(filter.0),
                _ => None,
            })
            .collect();
        assert_eq!(past.len(), 1, "{past:?}");
        assert_eq!(past[0]["#p"], serde_json::json!([me]));
        assert_eq!(listener.routes.of(OWN_ROUTE), vec![RelayUrl::parse("wss://r.example").unwrap()], "my relays, as the pool took the subscription");

        // An invitation to me and a copy of mine, as they come on my relays.
        let invite = |id: &str| communities_core::wire::Invite {
            invite_id: id.repeat(32),
            owner: None,
            community_id: "c".repeat(64),
            name: "Channel".into(),
            about: String::new(),
            picture: String::new(),
            relay: "wss://r.example".into(),
            members: 1,
            levels: 0,
            created_at: now,
            expires_at: now + 3600,
        };
        let envelope = |id: &str| communities_core::wire::note_envelope(communities_core::wire::T_INVITE, &invite(id)).unwrap();
        let from = Keys::generate();
        let to_me = communities_core::notes::wrap_note(&from, &PubKey::parse(&me).unwrap(), &envelope("a"), now).unwrap();
        let mine = communities_core::notes::wrap_own(&keys, &envelope("b"), now).unwrap();
        for wrap in [to_me, mine] {
            let outcome = service.on_event(&keys, &wrap.json, true).await;
            assert!(outcome.is_ok(), "{outcome:?}");
        }
        let all = ["sent", "accepted", "declined", "done", "expired", "cancelled", "received"];
        let came = db::channels::invites(service.db(), "in", &all).await.unwrap();
        let mine = db::channels::invites(service.db(), "out", &all).await.unwrap();
        assert_eq!((came.len(), mine.len()), (1, 1), "the invitation and the copy of mine are kept");
        // Heard now: the next round of this run neither asks again (its
        // subscription is up) nor pages my relays again.
        let before = net.sent().len();
        listener.listen().await;
        assert!(own(net.sent()[before..].to_vec()).is_none(), "the same subscription is not sent again");
        assert!(!net.sent()[before..].iter().any(|o| matches!(o, Outbound::Sync { scope: Scope::Channel { id }, .. } if id == OWN_ROUTE)), "not paged again");
    }

    fn subscribes(net: &FakeNet) -> usize {
        net.sent().iter().filter(|o| matches!(o, Outbound::Subscribe { .. })).count()
    }

    /// A wake of the keeper (a push, the periodic wake) while the live
    /// subscriptions are up sends no subscription and pages nothing again:
    /// the push of every note to the keeper had made a round, every
    /// channel paged anew (живой запуск 2026-10-23). Not up yet, it makes
    /// the round; once the notes are due again, it pages the notes alone.
    #[tokio::test]
    async fn a_wake_while_the_subscriptions_are_up_asks_for_nothing_again() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        db::channels::upsert(service.db(), &joined(&"c".repeat(64))).await.unwrap();
        let listener = Listener::new(service.clone(), net.clone(), Routes::default());
        listener.woken().await;
        until(|| !listener.busy()).await;
        let (subs, syncs) = (subscribes(&net), net.syncs());
        assert!(subs > 1 && syncs > 1, "not up: the round");
        for _ in 0..3 {
            listener.woken().await;
            until(|| !listener.busy()).await;
        }
        assert_eq!(subscribes(&net), subs, "no subscription again");
        assert_eq!(net.syncs(), syncs, "nothing paged again");

        // The notes due again (as `NOTES_RECATCH_SECS` after): the notes
        // alone, once.
        service.forget_caught();
        let before = net.sent().len();
        listener.woken().await;
        until(|| !listener.busy()).await;
        let asked = net.sent()[before..].to_vec();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert!(matches!(&asked[0], Outbound::Sync { filter, .. } if filter.0["kinds"] == serde_json::json!([communities_core::notes::KIND_WRAP])), "{asked:?}");
    }

    /// A wake while a round is on its way adds nothing to it: the round's
    /// catch-up pages the notes, and the wake asks no second page of them.
    #[tokio::test]
    async fn a_wake_during_a_round_adds_nothing() {
        let net = FakeNet::new(false);
        let service = service(net.clone()).await;
        db::channels::upsert(service.db(), &joined(&"c".repeat(64))).await.unwrap();
        let listener = Listener::new(service, net.clone(), Routes::default());
        listener.resubscribe();
        until(|| net.syncs() == 1).await;
        let subs = subscribes(&net);
        for _ in 0..3 {
            listener.woken().await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!((subscribes(&net), net.syncs()), (subs, 1), "the catch-up held, nothing more asked");
        net.gate.add_permits(tokio::sync::Semaphore::MAX_PERMITS);
        until(|| !listener.busy()).await;
        let wraps = net.sent().iter().filter(|o| matches!(o, Outbound::Sync { filter, .. } if filter.0["kinds"] == serde_json::json!([communities_core::notes::KIND_WRAP]))).count();
        assert_eq!(wraps, 1, "the notes paged once");
    }

    fn closed(net: &FakeNet) -> Vec<String> {
        net.sent()
            .into_iter()
            .filter_map(|o| match o {
                Outbound::Unsubscribe { id } => Some(id.0),
                _ => None,
            })
            .collect()
    }

    /// A channel my other device is in, whose journal is not here yet: routed
    /// to the relays my note of its keys named, its journal asked there by
    /// the round, and no route once its row names its own (ROO-271).
    #[tokio::test]
    async fn the_journal_my_note_waits_for_is_asked_on_the_relays_it_names() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        let cid = "c".repeat(64);
        let row = db::channels::OwnWaitRow {
            community_id: cid.clone(),
            owner: "d".repeat(64),
            relays_json: serde_json::json!(["wss://mine.example"]).to_string(),
            body_json: "{}".into(),
            received_at: now_secs() as i64,
        };
        db::channels::put_own_wait(service.db(), &row).await.unwrap();
        let listener = Listener::new(service.clone(), net.clone(), Routes::default());
        listener.listen().await;
        assert_eq!(listener.routes.of(&cid), vec![RelayUrl::parse("wss://mine.example").unwrap()]);
        let asked = net.sent().into_iter().any(|o| matches!(&o, Outbound::Sync { scope: Scope::Channel { id }, filter, .. } if id == &cid && filter.0["kinds"] == serde_json::json!([communities_core::wire::KIND_JOURNAL])));
        assert!(asked, "the journal asked by the round");
        db::channels::drop_own_wait(service.db(), &cid).await.unwrap();
        listener.listen().await;
        assert!(listener.routes.of(&cid).is_empty());
    }

    /// A channel no longer followed has its subscriptions closed and its
    /// route gone; a stop closes the rest, and a command still on its way
    /// asks for nothing after it (ревью модуля п. 5).
    #[tokio::test]
    async fn what_is_no_longer_followed_is_closed() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        for id in [&a, &b] {
            db::channels::upsert(service.db(), &joined(id)).await.unwrap();
        }
        let listener = Listener::new(service.clone(), net.clone(), Routes::default());
        listener.listen().await;
        assert!(closed(&net).is_empty());
        assert_eq!(listener.routes.of(&a).len(), 1);

        db::channels::delete(service.db(), &a).await.unwrap();
        listener.listen().await;
        let gone = closed(&net);
        assert!(!gone.is_empty() && gone.iter().all(|id| id.starts_with(&communities_core::transport::sub_id(&a, "").0)), "{gone:?}");
        assert!(listener.routes.of(&a).is_empty(), "a forgotten channel leaves no route");
        assert_eq!(listener.routes.of(&b).len(), 1);

        listener.close().await;
        let all = closed(&net);
        assert!(all.iter().any(|id| id.starts_with(&communities_core::transport::sub_id(&b, "").0)), "{all:?}");
        assert!(all.contains(&own_notes_sub().0));
        let before = net.sent().len();
        listener.resubscribe();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(net.sent().len(), before, "nothing after the stop");
    }

    /// A stop ends an upload on its way, and one asked for after it never
    /// starts (ревью модуля п. 5); the keepers' task the same: a turn
    /// runs on a wake, a stop ends the task mid-wait, and none starts
    /// after it.
    #[tokio::test]
    async fn a_stop_ends_the_uploads() {
        let uploads = Uploads::default();
        let (alive, ended) = tokio::sync::oneshot::channel::<()>();
        uploads.spawn("u1", async move {
            let _alive = alive;
            std::future::pending::<()>().await;
        });
        uploads.close();
        let ended = tokio::time::timeout(Duration::from_secs(5), ended).await;
        assert!(matches!(ended, Ok(Err(_))), "the upload was ended");
        let started = Arc::new(AtomicBool::new(false));
        let flag = started.clone();
        uploads.spawn("u2", async move { flag.store(true, Ordering::SeqCst) });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!started.load(Ordering::SeqCst));

        let keepers = Keepers::default();
        let wake = Wake::default();
        let turns = Arc::new(AtomicUsize::new(0));
        let counted = turns.clone();
        keepers.start(wake.clone(), move || {
            let counted = counted.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                None
            }
        });
        assert!(keepers.running());
        wake.at(0);
        until(|| turns.load(Ordering::SeqCst) == 1).await;
        // An ask while nothing waits yet is not lost.
        wake.at(0);
        until(|| turns.load(Ordering::SeqCst) == 2).await;
        keepers.close();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!keepers.running(), "ended");
        wake.at(0);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(turns.load(Ordering::SeqCst), 2, "no turn after the stop");
        keepers.start(wake, || async { None });
        assert!(!keepers.running(), "none after the stop");
    }

    /// A stop ends the outbox task, and one asked for after it never
    /// starts; what was queued stays for the next start.
    #[tokio::test]
    async fn a_stop_ends_the_outbox_task() {
        let net = FakeNet::new(true);
        let service = service(net.clone()).await;
        let (signals, _rx) = mpsc::unbounded_channel();
        let outbox = Outbox::default();
        outbox.start(service.clone(), signals.clone(), |_| {});
        assert!(outbox.running());
        outbox.close();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!outbox.running(), "ended");
        outbox.start(service, signals, |_| {});
        assert!(!outbox.running(), "none after the stop");
    }

    /// The pool answers slowly (a relay that takes seconds to say OK),
    /// while `slow` is set.
    struct SlowNet {
        delay: Duration,
        slow: AtomicBool,
        sent: std::sync::Mutex<Vec<Outbound>>,
    }

    #[async_trait::async_trait]
    impl Transport for SlowNet {
        async fn send(&self, out: Outbound) -> messenger_core::Result<Ack> {
            if self.slow.load(Ordering::SeqCst) && matches!(out, Outbound::PublishScoped { .. } | Outbound::PublishToInbox { .. } | Outbound::PublishOwn { .. }) {
                tokio::time::sleep(self.delay).await;
            }
            self.sent.lock().unwrap().push(out);
            Ok(Ack { accepted_by: vec![RelayUrl::parse("wss://r.example").unwrap()], rejected_by: vec![] })
        }

        fn events(&self) -> mpsc::Receiver<RawEvent> {
            mpsc::channel(1).1
        }

        async fn status(&self) -> RelayStatusSnapshot {
            RelayStatusSnapshot::default()
        }
    }

    /// A command answers before any relay does (живой запуск 2026-10-23:
    /// a new channel «hung» on the relay's answer): the channel is made
    /// in milliseconds over a pool that takes five seconds a publish, and
    /// the outbox task delivers its journal and card later, the journal
    /// first; a post the same, once the keeper's turn gave the owner the
    /// key (no pool was slow for that).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_command_answers_before_the_relay_and_the_outbox_delivers_later() {
        let net = Arc::new(SlowNet { delay: Duration::from_secs(5), slow: AtomicBool::new(true), sent: Default::default() });
        let db = Db::open_in_memory().await.unwrap();
        let blobs = Arc::new(super::join_path_tests::MemHosts::default());
        let service = ChannelService::new(db, Arc::new(MemSecrets::default()), net.clone(), blobs, Arc::new(SystemClock));
        let keys = Keys::generate();
        service.set_signer(Some(keys.clone()));
        let (signals, mut woken) = mpsc::unbounded_channel();
        let outbox = Outbox::default();
        outbox.start(service.clone(), signals, |_| {});
        let new = communities_core::service::NewChannel {
            name: "Fast".into(),
            about: String::new(),
            picture: String::new(),
            join: communities_core::JoinMode::Open,
            history: communities_core::History::Open,
            relays: vec!["wss://r.example".into()],
            storage: vec!["https://h1.example".into()],
        };
        let started = tokio::time::Instant::now();
        let (cid, outcome) = service.create(&keys, new).await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(200), "answered in {:?}", started.elapsed());
        assert_eq!(outcome.pending.len(), 1);
        assert!(!net.sent.lock().unwrap().iter().any(|o| matches!(o, Outbound::PublishScoped { .. } | Outbound::PublishToInbox { .. } | Outbound::PublishOwn { .. })), "nothing is on the relay yet");
        assert_eq!(service.outbox_summary(&cid).await.unwrap().queued, 4, "the journal, the card, the keeper's issue to me and the copy of my keys to my other devices wait");
        let kinds = || -> Vec<u16> {
            net.sent.lock().unwrap().iter().filter_map(|o| match o {
                Outbound::PublishScoped { event, .. } => event.json["kind"].as_u64().and_then(|k| u16::try_from(k).ok()),
                Outbound::PublishToInbox { .. } | Outbound::PublishOwn { .. } => Some(communities_core::notes::KIND_WRAP),
                _ => None,
            }).collect()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while kinds().len() < 4 {
            assert!(tokio::time::Instant::now() < deadline, "the outbox delivered {:?}", kinds());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The journal before the notes that follow it; the card on its own,
        // wherever a pass of the worker found it (queued before the notes,
        // sent in the first pass when that pass read the rows before the
        // notes were queued).
        let sent = kinds();
        assert_eq!(sent[0], communities_core::wire::KIND_JOURNAL, "the journal first: {sent:?}");
        let mut rest = sent[1..].to_vec();
        rest.sort_unstable();
        assert_eq!(rest, vec![communities_core::notes::KIND_WRAP, communities_core::notes::KIND_WRAP, communities_core::wire::KIND_CARD], "the notes behind it, the card on its own: {sent:?}");
        assert_eq!(service.outbox_summary(&cid).await.unwrap().queued, 0);
        assert!(matches!(woken.try_recv(), Ok(Signal::Wake(_))), "the keeper's turn is woken once its operation is out");

        // The first epoch (the pool quick meanwhile: the hosts are what the
        // turn waits for, not the relay), then a post over the slow pool.
        net.slow.store(false, Ordering::SeqCst);
        service.tick().await.unwrap();
        assert!(service.member(&keys).newest_key(&cid, communities_core::state::SCOPE_EVERYONE).await.unwrap().is_some(), "the owner holds the key of the first epoch");
        net.slow.store(true, Ordering::SeqCst);
        let before = kinds().len();
        let started = tokio::time::Instant::now();
        let (post, outcome) = service.send_post(&keys, &cid, "quick", messenger_core::TextFormat::Plain).await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(200), "the post answered in {:?}", started.elapsed());
        assert!(outcome.pending.contains(&post));
        assert_eq!(kinds().len(), before, "not on the relay yet");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while kinds().len() < before + 1 {
            assert!(tokio::time::Instant::now() < deadline, "the outbox delivered {:?}", kinds());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(kinds().last(), Some(&communities_core::wire::KIND_CONTENT));
        outbox.close();
    }

    /// Each identity has a database of its own in the module's folder, and
    /// the channels of one are not those of another; the file of the
    /// builds before is not one of them (ревью модуля п. 6).
    #[tokio::test]
    async fn another_identity_does_not_see_the_channels_of_the_one_before() {
        let dir = std::env::temp_dir().join(format!("communities-ids-{}-{}", std::process::id(), now_secs()));
        let (a, b) = (Keys::generate(), Keys::generate());
        let (path_a, path_b) = (db_path(&dir, &a), db_path(&dir, &b));
        assert_ne!(path_a, path_b);
        for path in [&path_a, &path_b] {
            assert_eq!(path.parent(), Some(dir.as_path()));
            assert_ne!(path.file_name().and_then(|n| n.to_str()), Some(db::FILE_NAME));
        }
        assert!(path_a.to_string_lossy().ends_with(".db") && path_a.to_string_lossy().contains("npub1"), "{path_a:?}");
        let first = Db::open(&path_a).await.unwrap();
        db::channels::upsert(&first, &joined(&"a".repeat(64))).await.unwrap();
        first.close().await;
        let second = Db::open(&path_b).await.unwrap();
        assert!(db::channels::list(&second).await.unwrap().is_empty(), "the second identity starts with nothing");
        second.close().await;
        let again = Db::open(&path_a).await.unwrap();
        assert_eq!(db::channels::list(&again).await.unwrap().len(), 1, "the first finds its own again");
        again.close().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A burst of asks runs the work once, an ask after it starts runs it
    /// again, and nothing runs once closed (ревью модуля п. 11).
    #[tokio::test]
    async fn a_burst_of_asks_runs_once() {
        let coalesce = Coalesce::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let ask = |c: &Coalesce| {
            let runs = runs.clone();
            c.ask(Duration::from_millis(30), async move {
                runs.fetch_add(1, Ordering::SeqCst);
            });
        };
        for _ in 0..50 {
            ask(&coalesce);
        }
        until(|| runs.load(Ordering::SeqCst) == 1).await;
        ask(&coalesce);
        until(|| runs.load(Ordering::SeqCst) == 2).await;
        ask(&coalesce);
        coalesce.close();
        ask(&coalesce);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "nothing after the close");
    }

    /// A preview that ends takes only its own relay away, not the one an
    /// opening of the same link holds; a failed opening leaves nothing; a
    /// kept one stays until the channel's row names its own (ревью модуля
    /// п. 16).
    #[test]
    fn a_hold_on_a_route_takes_only_its_own_away() {
        let routes = Routes::default();
        let cid = "c".repeat(64);
        let (link, own) = (RelayUrl::parse("wss://link.example").unwrap(), RelayUrl::parse("wss://own.example").unwrap());
        let opening = routes.hold(&cid, link.clone());
        let preview = routes.hold(&cid, link.clone());
        assert_eq!(routes.of(&cid), vec![link.clone()]);
        drop(preview);
        assert_eq!(routes.of(&cid), vec![link.clone()], "the opening still holds it");
        drop(opening);
        assert!(routes.of(&cid).is_empty(), "a failed opening leaves no route");

        routes.hold(&cid, link.clone()).keep();
        assert_eq!(routes.of(&cid), vec![link.clone()], "kept after the hold");
        routes.set(HashMap::from([(cid.clone(), vec![own.clone()])]));
        assert_eq!(routes.of(&cid), vec![own], "its row names its own");
        routes.set(HashMap::new());
        assert!(routes.of(&cid).is_empty(), "forgotten");
    }

    fn card(d: &str) -> serde_json::Value {
        serde_json::json!({ "kind": communities_core::wire::KIND_CARD, "tags": [["d", d]], "content": "{}" })
    }

    /// A card goes to whoever waits for its channel, and to nobody once
    /// they stopped waiting; anything else is not offered.
    #[test]
    fn a_card_reaches_those_waiting_for_its_channel() {
        let previews = Previews::default();
        let (watching, mut cards) = previews.watch("c1");
        let (_other, mut others) = previews.watch("c2");
        previews.offer(&card("c1"));
        previews.offer(&serde_json::json!({ "kind": 1, "tags": [["d", "c1"]] }));
        assert_eq!(cards.try_recv().ok(), Some(card("c1")));
        assert!(cards.try_recv().is_err(), "a note is no card");
        assert!(others.try_recv().is_err(), "another channel's");
        drop(watching);
        assert!(!read(&previews.waiting).contains_key("c1"), "gone with the guard");
        previews.offer(&card("c2"));
        assert!(others.try_recv().is_ok());
    }

    /// What came live is taken before what a catch-up brought, however
    /// much of that waits (живой запуск 2026-10-23: a join that came live
    /// behind two days of wraps of a round was taken four minutes late).
    #[test]
    fn a_live_event_goes_before_the_catch_up_that_came_first_and_wraps_go_last() {
        let event = |n: u64, source: EventSource| RawEvent {
            id: messenger_core::EventId::parse(&format!("{n:064x}")).expect("id"),
            kind: 1059,
            pubkey: PubKey::parse(&"1".repeat(64)).expect("pubkey"),
            created_at: messenger_core::Timestamp(0),
            json: serde_json::json!({ "n": n, "kind": if n == 2000 { 1391 } else { 1059 } }),
            source,
        };
        let relay = RelayUrl::parse("wss://r.example").expect("url");
        let mut q = Queue::default();
        for n in 0..1000 {
            q.push(event(n, EventSource::Sync { url: relay.clone() }));
        }
        q.push(event(2000, EventSource::Sync { url: relay.clone() }));
        q.push(event(1000, EventSource::Relay { url: relay.clone() }));
        q.push(event(1001, EventSource::Relay { url: relay.clone() }));
        let order: Vec<u64> = std::iter::from_fn(|| q.next()).map(|e| e.json["n"].as_u64().unwrap_or(0)).collect();
        assert_eq!(&order[..4], &[1000, 1001, 2000, 0], "live, then a journal of a catch-up, then its wraps");
        assert_eq!(order.len(), 1003);
        assert!(order[3..].windows(2).all(|w| w[0] < w[1]), "the catch-up in its order");
    }
}

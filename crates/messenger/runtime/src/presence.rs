// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Presence in motion (internal/messenger-wire.md, "Присутствие"). Every few
//! seconds of a session the driver watches the presence keys of my
//! approved contacts and my own (for my other device's beat), tells each
//! contact my key when it has not been told the current one, and beats
//! while the app is in sight.
//!
//! "In sight" is a lease the page renews (`presence_foreground`): a page
//! frozen by the system without a word stops the beats within
//! `FOREGROUND_LEASE_SECS`. A beat goes straight to the pool, never to the
//! outbox: a beat sent late says I am here when I am not. Nothing says
//! "gone": a beat expires by itself.
//!
//! The key moves to the next epoch when someone who was told it is no
//! longer an approved contact (removed or blocked, by me or by them) and
//! when the switch is turned off or on. My other devices hear of each
//! move (`own.presence`), the switch with it, so presence turned off on
//! one device is off on all of them. With `privacy.presence` off nothing
//! is told, watched, beaten or shown, and whoever was told my key is told
//! it is gone.
//!
//! The look around is decided under one lock (`turn`), which a rotation
//! waits for; what goes to the relays (the subscription, the beat) is
//! sent after the lock is let go, so a slow relay holds up no command.
//!
//! Beside the beat, the focus (messenger-wire.md §8, «Фокус»; ROO-1275):
//! which chat I am reading on this device, said to my other devices so
//! that they show no notification for it. The page names the chat
//! (`focus`) while it is in sight, its window has the focus and the user
//! does something in it; the name holds `FOCUS_IDLE_SECS` past the last
//! word. The focus event goes as a beat does, past the outbox, every half
//! minute and at once when the chat changes, and once with no chat when
//! the page leaves it. My focus key is watched whatever the presence
//! switch says: it tells nobody but my devices anything.

use crate::MessengerRuntime;
use messenger_contacts::ContactService;
use messenger_core::traits::SystemClock;
use messenger_core::{Clock, Envelope, Outbound, PubKey, Result, Scope, SubId, Transport};
use messenger_dm::wrap::{wrap_note, wrap_own};
use messenger_dm::{presence_proof, DmService};
use messenger_ingress::{filters, Outbox};
use messenger_presence::{
    elsewhere, focus, heartbeat, key, sight, view, FocusKeys, FocusSlot, PresenceView, BEAT_JITTER_SECS, BEAT_SECS,
};
use messenger_store::{presence, settings, Store};
use messenger_transport::RelayPool;
use nostr::key::Keys;
use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};

pub use crate::privacy::KEY_PRESENCE;
pub use messenger_dm::{
    KEY_PRESENCE_DEVICES_TOLD, KEY_PRESENCE_EPOCH, KEY_PRESENCE_SINCE, UI_EVENT_PRESENCE_EPOCH_CHANGED, UI_EVENT_PRESENCE_KEYS_CHANGED,
};
pub use messenger_presence::UI_EVENT_PRESENCE_UPDATED;

/// How often the driver looks around.
pub const PRESENCE_TICK_SECS: u64 = 5;
/// How long the page counts as in sight after it last said so. The page
/// says so every 45 s while it is.
pub const FOREGROUND_LEASE_SECS: i64 = 90;
/// How long the page counts as reading its chat after it last said so;
/// it says so when the user does something in it (a pointer, a key). Five
/// minutes without a touch, and my other devices notify again (ROO-1275).
pub const FOCUS_IDLE_SECS: i64 = 300;
/// The epoch of the focus keys, the push handler's as well.
pub use messenger_presence::FOCUS_EPOCH;

/// How far ahead of the clock a focus event may be dated so that each is
/// later than the one before (`PresenceDriver::focus_if_due`): well within
/// what a reader takes (`FOCUS_AHEAD_SECS`).
pub const FOCUS_LEAD_SECS: i64 = 5;
const PRESENCE_TICK: Duration = Duration::from_secs(PRESENCE_TICK_SECS);

/// Whether to beat now: the page is in sight, the app may reach relays,
/// and the last beat is a beat's interval (with its jitter) behind.
pub fn beat_due(now: i64, foreground_until: i64, last_beat: i64, silent: bool, jitter: i64) -> bool {
    !silent && now < foreground_until && now - last_beat >= BEAT_SECS + jitter
}

/// The chat this device is reading at `now`: the one the page named, while
/// its word and the page's lease of being in sight both hold.
pub fn focus_wanted(now: i64, foreground_until: i64, chat: Option<&str>, focus_until: i64) -> Option<&str> {
    chat.filter(|_| now < foreground_until && now < focus_until)
}

/// Whether to say the focus now: what is wanted is not what was said last
/// (a chat opened, changed or left), or a chat is being read and the last
/// word is a beat's interval (with its jitter) behind. Nothing in silent
/// mode; nothing again while no chat is read.
pub fn focus_due(now: i64, wanted: Option<&str>, said: Option<&str>, last: i64, silent: bool, jitter: i64) -> bool {
    !silent && (wanted != said || (wanted.is_some() && now - last >= BEAT_SECS + jitter))
}

/// Whether a note saying `since` may go at `now`. A contact counts a
/// `since` no further than a second past the note, so a key or a
/// withdrawal waits until its `since` is no more than a second ahead:
/// otherwise two moves of one second would reach the contact as equals.
pub fn may_tell(now: i64, since: i64) -> bool {
    since <= now + 1
}

/// The jitter of the `n`-th beat, spread over `-BEAT_JITTER_SECS..=BEAT_JITTER_SECS`
/// so that the beats of many users do not fall on one second. Nothing
/// secret: a fixed walk is enough.
pub fn jitter(n: u64) -> i64 {
    let span = (2 * BEAT_JITTER_SECS + 1) as u64;
    (n.wrapping_mul(7) % span) as i64 - BEAT_JITTER_SECS
}

pub struct PresenceDriver {
    store: Store,
    dm: DmService,
    contacts: ContactService,
    outbox: Outbox,
    /// Until when the page is in sight (unix seconds); 0: it is not.
    foreground_until: AtomicI64,
    /// When this device last beat; 0: beat at the first chance.
    last_beat: AtomicI64,
    /// How many beats this run made: picks the next jitter.
    beats: AtomicU64,
    /// The presence keys the pool watches now, sorted.
    subscribed: Mutex<Vec<String>>,
    /// My presence key: whose, of which epoch, the keys.
    epoch_key: std::sync::Mutex<Option<(String, u32, Keys)>>,
    /// One look around at a time; a rotation waits for it.
    turn: Mutex<()>,
    /// My focus keys of this session, shared with the handler that opens
    /// what my other devices say (`messenger_presence::FocusHandler`).
    focus_keys: FocusSlot,
    /// The chat the page names, until when its word holds, and the number
    /// of the word (`set_focus`).
    focus_lease: std::sync::Mutex<(Option<String>, i64, u64)>,
    /// What this device said last and when (0: say at the first chance).
    focus_said: std::sync::Mutex<(Option<String>, i64)>,
    /// How many focus events this run made: picks the next jitter.
    focus_beats: AtomicU64,
    /// The `created_at` of the last focus event made: the next is later.
    focus_at: AtomicI64,
    /// One focus event at a time: the tick and the page may both find one due.
    focus_turn: Mutex<()>,
    /// The focus key the pool watches now.
    focus_watched: Mutex<Option<String>>,
}

impl PresenceDriver {
    pub fn new(store: Store, dm: DmService, contacts: ContactService, outbox: Outbox, focus_keys: FocusSlot) -> Self {
        Self {
            store,
            dm,
            contacts,
            outbox,
            foreground_until: AtomicI64::new(0),
            last_beat: AtomicI64::new(0),
            beats: AtomicU64::new(0),
            subscribed: Mutex::new(Vec::new()),
            epoch_key: std::sync::Mutex::new(None),
            turn: Mutex::new(()),
            focus_keys,
            focus_lease: std::sync::Mutex::new((None, 0, 0)),
            focus_said: std::sync::Mutex::new((None, 0)),
            focus_beats: AtomicU64::new(0),
            focus_at: AtomicI64::new(0),
            focus_turn: Mutex::new(()),
            focus_watched: Mutex::new(None),
        }
    }

    pub async fn enabled(&self) -> Result<bool> {
        settings::get_bool(&self.store, KEY_PRESENCE, true).await
    }

    /// A new session: a new pool that watches nothing yet, maybe another
    /// user, and a beat at the first chance. My other devices are told
    /// where this one stands at the first look around: one added since the
    /// last `own.presence`, or one that missed it, would beat from a key
    /// no contact watches; the one behind takes it, the one ahead answers.
    pub(crate) async fn session_started(&self) {
        self.subscribed.lock().await.clear();
        *self.epoch_key.lock().unwrap() = None;
        self.last_beat.store(0, Ordering::SeqCst);
        *self.focus_watched.lock().await = None;
        *self.focus_keys.write().unwrap() = None;
        *self.focus_said.lock().unwrap() = (None, 0);
        if let Err(e) = self.dm.presence_devices_unsure().await {
            eprintln!("messenger presence: my devices not told: {e}");
        }
    }

    /// The device woke from a sleep: beat at the next chance.
    pub fn woke(&self) {
        self.last_beat.store(0, Ordering::SeqCst);
        self.focus_said.lock().unwrap().1 = 0;
    }

    /// The page is in sight (renewing the lease) or hidden.
    pub fn set_foreground(&self, visible: bool, now: i64) {
        self.foreground_until.store(if visible { now + FOREGROUND_LEASE_SECS } else { 0 }, Ordering::SeqCst);
    }

    /// The page reads `chat` now (renewing its word) or no chat. `asked`
    /// numbers the words in the order the page said them: one that comes
    /// after a later one (a chat that waited for the lock or the screen
    /// while the window lost the focus) is dropped. `true` when taken.
    pub fn set_focus(&self, chat: Option<String>, now: i64, asked: u64) -> bool {
        let mut lease = self.focus_lease.lock().unwrap();
        if asked < lease.2 {
            return false;
        }
        *lease = match chat {
            Some(chat) => (Some(chat), now + FOCUS_IDLE_SECS, asked),
            None => (None, 0, asked),
        };
        true
    }

    /// My focus keys of this session: made from `me` at the first need.
    pub fn focus_keys(&self, me: &Keys) -> Arc<FocusKeys> {
        if let Some(keys) = self.focus_keys.read().unwrap().clone() {
            return keys;
        }
        let keys = Arc::new(FocusKeys::derive(me, FOCUS_EPOCH));
        *self.focus_keys.write().unwrap() = Some(keys.clone());
        keys
    }

    pub(crate) async fn turn(&self) -> MutexGuard<'_, ()> {
        self.turn.lock().await
    }

    /// My presence key now: of the current epoch, made from `me`.
    pub async fn presence_keys(&self, me: &Keys) -> Result<(u32, Keys)> {
        let epoch = self.dm.presence_epoch().await?;
        let whose = me.public_key().to_hex();
        let mut cached = self.epoch_key.lock().unwrap();
        if let Some((w, e, k)) = cached.as_ref() {
            if *w == whose && *e == epoch {
                return Ok((epoch, k.clone()));
            }
        }
        let k = key::derive(me, epoch);
        *cached = Some((whose, epoch, k.clone()));
        Ok((epoch, k))
    }

    /// The contacts I share presence with (`DmService::presence_allowed`).
    pub async fn approved(&self, me: &str) -> Result<Vec<PubKey>> {
        let mut out = Vec::new();
        for c in self.contacts.list().await? {
            let Some(pk) = PubKey::parse(&c.pubkey) else { continue };
            if pk.as_hex() != me && self.dm.presence_allowed(&pk).await? {
                out.push(pk);
            }
        }
        Ok(out)
    }

    /// One look around (`presence_loop`): decided under the turn, sent to
    /// the relays after it.
    pub(crate) async fn tick(&self, pool: &RelayPool, keys: &Keys, now: i64) -> Result<()> {
        let watched = {
            let _turn = self.turn().await;
            self.look_around(keys, now).await?
        };
        self.watch(pool, watched).await;
        self.beat_if_due(pool, keys, now).await;
        self.watch_focus(pool, keys).await;
        self.focus_if_due(pool, keys, now).await;
        Ok(())
    }

    /// Watch my focus key, whatever the presence switch: what my other
    /// device reads. A pool that refuses is asked again at the next tick.
    async fn watch_focus(&self, pool: &RelayPool, keys: &Keys) {
        let mine = self.focus_keys(keys).public_key().to_hex();
        let mut watched = self.focus_watched.lock().await;
        if watched.as_deref() == Some(mine.as_str()) {
            return;
        }
        let Some(author) = PubKey::parse(&mine) else { return };
        let out = Outbound::Subscribe { id: SubId(filters::SUB_FOCUS.into()), filter: filters::focus_of(&author), scope: Scope::Own };
        match pool.send(out).await {
            Ok(_) => *watched = Some(mine),
            Err(e) => eprintln!("messenger focus: not watched: {e}"),
        }
    }

    /// Say the focus when it is time (`focus_due`), straight to the pool as
    /// a beat. One no relay answered is not said: the next chance tries
    /// again. `true` when it went.
    ///
    /// Each event is dated a second past the one before, ahead of the
    /// clock if need be: a relay keeps one focus event of the key, and of
    /// two of one second the one with the smaller id (NIP-01), so a chat
    /// left in the second it was opened would stay "in X" on the relay and
    /// keep the phone quiet (ROO-1300). Never more than [`FOCUS_LEAD_SECS`]
    /// ahead: past that the word waits for the tick.
    pub(crate) async fn focus_if_due(&self, pool: &RelayPool, keys: &Keys, now: i64) -> bool {
        let _turn = self.focus_turn.lock().await;
        let wanted = {
            let lease = self.focus_lease.lock().unwrap();
            focus_wanted(now, self.foreground_until.load(Ordering::SeqCst), lease.0.as_deref(), lease.1).map(str::to_string)
        };
        let (said, last) = self.focus_said.lock().unwrap().clone();
        let jitter = jitter(self.focus_beats.load(Ordering::SeqCst));
        if !focus_due(now, wanted.as_deref(), said.as_deref(), last, pool.is_silent(), jitter) {
            return false;
        }
        let at = now.max(self.focus_at.load(Ordering::SeqCst) + 1);
        if at > now + FOCUS_LEAD_SECS {
            return false;
        }
        let keys = self.focus_keys(keys);
        // A chat id too long for the seal is said as no chat.
        let event = match focus::build(&keys, wanted.as_deref(), at).or_else(|_| focus::build(&keys, None, at)) {
            Ok(event) => event,
            Err(e) => {
                eprintln!("messenger focus: not said: {e}");
                return false;
            }
        };
        self.focus_at.store(at, Ordering::SeqCst);
        // Written down before it goes: its copy from a relay is this device's.
        if let Err(e) = elsewhere::sending(&self.store, event.id.as_hex()).await {
            eprintln!("messenger focus: not said, not written down: {e}");
            return false;
        }
        match pool.send(Outbound::PublishOwn { event }).await {
            Ok(ack) if !ack.accepted_by.is_empty() || !ack.rejected_by.is_empty() => {
                *self.focus_said.lock().unwrap() = (wanted, now);
                self.focus_beats.fetch_add(1, Ordering::SeqCst);
                true
            }
            _ => false,
        }
    }

    /// What a tick does on this device: tells my other devices an epoch
    /// they are owed, withdraws my key while off, moves it when someone
    /// told it is gone, and tells the approved contacts. Returns the keys
    /// to watch. The caller holds the turn.
    async fn look_around(&self, keys: &Keys, now: i64) -> Result<Vec<String>> {
        self.tell_devices_if_owed(keys, now).await?;
        if !self.enabled().await? {
            self.withdraw_told(keys, now).await?;
            return Ok(Vec::new());
        }
        let me = keys.public_key().to_hex();
        let approved = self.approved(&me).await?;
        if self.told_someone_gone(&approved).await? {
            self.rotate(Some(keys), now, true).await?;
        }
        let allowed: HashSet<&str> = approved.iter().map(|p| p.as_hex()).collect();
        let mut watched: Vec<String> =
            presence::keys(&self.store).await?.into_iter().filter(|(peer, _)| allowed.contains(peer.as_str())).map(|(_, k)| k).collect();
        // My own key in the same subscription: the beat of my other device
        // is written down (`messenger_presence::sight`). Known as mine
        // before the relays bring it.
        let mine = self.presence_keys(keys).await?.1.public_key().to_hex();
        sight::watching_mine(&self.store, &mine).await?;
        watched.push(mine);
        watched.sort();
        watched.dedup();
        self.announce_if_due(keys, &approved, now).await?;
        Ok(watched)
    }

    /// Watch these presence keys and no others; none while presence is
    /// off, whatever a tick decided before it was turned off. A pool that
    /// refuses keeps what it had: the next tick tries again.
    async fn watch(&self, pool: &RelayPool, mut keys: Vec<String>) {
        let mut subscribed = self.subscribed.lock().await;
        if !keys.is_empty() && !self.enabled().await.unwrap_or(false) {
            keys.clear();
        }
        if *subscribed == keys {
            return;
        }
        let out = if keys.is_empty() {
            Outbound::Unsubscribe { id: SubId(filters::SUB_PRESENCE.into()) }
        } else {
            let authors: Vec<PubKey> = keys.iter().filter_map(|k| PubKey::parse(k)).collect();
            Outbound::Subscribe { id: SubId(filters::SUB_PRESENCE.into()), filter: filters::presence_of(&authors), scope: Scope::Own }
        };
        match pool.send(out).await {
            Ok(_) => *subscribed = keys,
            Err(e) => eprintln!("messenger presence: watching not changed: {e}"),
        }
    }

    /// Someone told my current key is no longer an approved contact: on
    /// this device or another, by me or by them.
    async fn told_someone_gone(&self, approved: &[PubKey]) -> Result<bool> {
        let told = presence::told_peers(&self.store).await?;
        Ok(told.iter().any(|peer| !approved.iter().any(|a| a.as_hex() == peer)))
    }

    /// Tell every approved contact that was not told the current key. A
    /// note that cannot be queued is tried at the next tick.
    async fn announce_if_due(&self, keys: &Keys, approved: &[PubKey], now: i64) -> Result<()> {
        let (epoch, mine) = self.presence_keys(keys).await?;
        let since = self.dm.presence_since().await?;
        if !may_tell(now, since) {
            return Ok(());
        }
        let me = PubKey::parse(&keys.public_key().to_hex()).expect("a key's own hex");
        let (pubkey, proof) = (mine.public_key().to_hex(), presence_proof(&mine, &me, since));
        let note = Envelope::presence_key(Some((&pubkey, &proof)), since);
        let mut queued = false;
        for peer in approved {
            if presence::told(&self.store, peer.as_hex()).await? == Some(epoch) {
                continue;
            }
            match self.note_to(keys, peer, &note, now).await {
                Ok(()) => {
                    presence::mark_told(&self.store, peer.as_hex(), epoch).await?;
                    queued = true;
                }
                Err(e) => eprintln!("messenger presence: key to {}: {e}", peer.as_hex()),
            }
        }
        if queued {
            self.outbox.kick();
        }
        Ok(())
    }

    /// A note to `peer`, tried until it leaves: a key told late still holds.
    async fn note_to(&self, keys: &Keys, peer: &PubKey, note: &Envelope, now: i64) -> Result<()> {
        let w = wrap_note(keys, peer, &note.encode(), now, false, None)?;
        let hint_relays = self.dm.hints(peer).await?;
        self.outbox.enqueue(Outbound::PublishToInbox { recipient: peer.clone(), event: w.to_peer, hint_relays }).await?;
        Ok(())
    }

    /// Beat when it is time. A beat no relay took (none connected yet,
    /// after a start or a wake) is not done: the next chance tries again.
    /// One a relay answered is done, taken or not. `true` when it went.
    pub(crate) async fn beat_if_due(&self, pool: &RelayPool, keys: &Keys, now: i64) -> bool {
        let last = self.last_beat.load(Ordering::SeqCst);
        let n = self.beats.load(Ordering::SeqCst);
        let until = self.foreground_until.load(Ordering::SeqCst);
        if !beat_due(now, until, last, pool.is_silent(), jitter(n)) || !self.enabled().await.unwrap_or(false) {
            return false;
        }
        // The tick and the page may both find it due: one beats.
        if self.last_beat.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return false;
        }
        let undo = || {
            let _ = self.last_beat.compare_exchange(now, last, Ordering::SeqCst, Ordering::SeqCst);
        };
        let event = match self.presence_keys(keys).await.and_then(|(_, k)| heartbeat::build(&k, now)) {
            Ok(event) => event,
            Err(e) => {
                eprintln!("messenger presence: no beat: {e}");
                undo();
                return false;
            }
        };
        // Written down before it goes: its copy from a relay, in this
        // process or the next, is never taken for my other device's.
        if let Err(e) = sight::beating(&self.store, now).await {
            eprintln!("messenger presence: no beat, not written down: {e}");
            undo();
            return false;
        }
        match pool.send(Outbound::PublishOwn { event }).await {
            Ok(ack) if !ack.accepted_by.is_empty() || !ack.rejected_by.is_empty() => {
                self.beats.fetch_add(1, Ordering::SeqCst);
                true
            }
            _ => {
                undo();
                false
            }
        }
    }

    /// Whoever was told my key is told it is gone, as of the `since` of
    /// the epoch I stopped sharing in, so a key told later wins over it.
    /// Done when every note is queued; until then the next tick tries again.
    async fn withdraw_told(&self, keys: &Keys, now: i64) -> Result<()> {
        let told = presence::told_peers(&self.store).await?;
        let since = self.dm.presence_since().await?;
        if told.is_empty() || !may_tell(now, since) {
            return Ok(());
        }
        let note = Envelope::presence_key(None, since);
        let (mut queued, mut all) = (false, true);
        for peer in told {
            let Some(pk) = PubKey::parse(&peer) else { continue };
            match self.note_to(keys, &pk, &note, now).await {
                Ok(()) => queued = true,
                Err(e) => {
                    all = false;
                    eprintln!("messenger presence: withdrawal to {peer}: {e}");
                }
            }
        }
        if all {
            presence::forget_told(&self.store).await?;
        }
        if queued {
            self.outbox.kick();
        }
        Ok(())
    }

    /// My other devices are told the epoch this device moved to, with its
    /// `since` and the switch, once: a rotation made without a session is
    /// told at the first look around of the next one.
    async fn tell_devices_if_owed(&self, keys: &Keys, now: i64) -> Result<()> {
        let Some(state) = self.dm.presence_devices_owed().await? else { return Ok(()) };
        let event = wrap_own(keys, &state.note().encode(), now)?;
        self.outbox.enqueue(Outbound::PublishOwn { event }).await?;
        self.dm.presence_devices_told(state.epoch).await?;
        self.outbox.kick();
        Ok(())
    }

    /// Move to the next epoch, sharing or not from now on. Sharing, nobody
    /// is told the new key yet; not sharing, whoever was told the old one
    /// is left for `withdraw_told`. My other devices are told now, or at
    /// the next session. The caller holds the turn.
    async fn rotate(&self, keys: Option<&Keys>, now: i64, sharing: bool) -> Result<u32> {
        let state = self.dm.rotate_presence(sharing).await?;
        if sharing {
            presence::forget_told(&self.store).await?;
        }
        if let Some(keys) = keys {
            self.tell_devices_if_owed(keys, now).await?;
        }
        Ok(state.epoch)
    }
}

/// Runs for the life of a session.
pub(crate) async fn presence_loop(driver: Arc<PresenceDriver>, pool: Arc<RelayPool>, keys: Keys) {
    loop {
        if let Err(e) = driver.tick(&pool, &keys, SystemClock.now().secs()).await {
            eprintln!("messenger presence: {e}");
        }
        tokio::time::sleep(PRESENCE_TICK).await;
    }
}

impl MessengerRuntime {
    pub fn presence(&self) -> &PresenceDriver {
        &self.presence
    }

    /// The page is in sight or hidden. While in sight the page says so
    /// again every 45 s; in sight, a beat goes at once if one is due.
    pub async fn presence_foreground(&self, visible: bool) -> Result<()> {
        let now = SystemClock.now().secs();
        self.presence.set_foreground(visible, now);
        let Ok(keys) = self.session_keys().await else { return Ok(()) };
        let pool = self.relays.pool().await;
        if visible && self.presence.enabled().await? {
            self.presence.beat_if_due(&pool, &keys, now).await;
        }
        // Hidden: my other devices hear at once that no chat is read here.
        self.presence.focus_if_due(&pool, &keys, now).await;
        Ok(())
    }

    /// The page reads `chat` now (said again while the user does something
    /// in it) or no chat: the window lost the focus, the chat closed. A
    /// change is said to my other devices at once. `asked`: the number of
    /// the page's word, a word older than one taken is dropped
    /// (`PresenceDriver::set_focus`).
    pub async fn focus(&self, chat: Option<String>, asked: u64) -> Result<()> {
        let now = SystemClock.now().secs();
        let opened = chat.clone();
        if !self.presence.set_focus(chat, now, asked) {
            return Ok(());
        }
        // A call may follow: its nodes are asked while the chat is read.
        if let Some(chat) = opened {
            self.call_warm(&chat).await;
        }
        let Ok(keys) = self.session_keys().await else { return Ok(()) };
        let pool = self.relays.pool().await;
        self.presence.focus_if_due(&pool, &keys, now).await;
        Ok(())
    }

    /// Whether another device of mine is reading `chat_id` now: a message
    /// of it is then shown here by nothing, not a notification, not a card.
    pub async fn read_elsewhere(&self, chat_id: &str) -> Result<bool> {
        elsewhere::reading(&self.store, chat_id, SystemClock.now().secs()).await
    }

    /// Look around now instead of at the next tick. Nothing without a
    /// session.
    pub async fn presence_tick(&self) -> Result<()> {
        let Ok(keys) = self.session_keys().await else { return Ok(()) };
        let pool = self.relays.pool().await;
        self.presence.tick(&pool, &keys, SystemClock.now().secs()).await
    }

    /// Move my presence key to the next epoch: whoever knew the old one
    /// watches a key that no longer beats. The approved contacts are told
    /// the new key at the next tick. Returns the new epoch.
    pub async fn presence_rotate(&self) -> Result<u32> {
        let keys = self.session_keys().await.ok();
        let _turn = self.presence.turn().await;
        let sharing = self.presence.enabled().await?;
        self.presence.rotate(keys.as_ref(), SystemClock.now().secs(), sharing).await
    }

    /// The switch moved (`privacy_set`): the key moves on, and my other
    /// devices follow the switch. Off, everyone told my key is told it is
    /// gone and nothing is watched any more; on, the next tick tells the
    /// new key, whose `since` is later than the withdrawal. Returns the new
    /// epoch.
    pub(crate) async fn presence_switched(&self, on: bool) -> Result<u32> {
        let keys = self.session_keys().await.ok();
        let now = SystemClock.now().secs();
        let epoch = {
            let _turn = self.presence.turn().await;
            let epoch = self.presence.rotate(keys.as_ref(), now, on).await?;
            if let (false, Some(keys)) = (on, &keys) {
                self.presence.withdraw_told(keys, now).await?;
            }
            epoch
        };
        if !on && keys.is_some() {
            let pool = self.relays.pool().await;
            self.presence.watch(&pool, Vec::new()).await;
        }
        Ok(epoch)
    }

    /// When each approved contact was last seen and until when it is
    /// online. Empty with presence off: I am shown nobody.
    pub async fn presence_list(&self) -> Result<Vec<PresenceView>> {
        if !self.presence.enabled().await? {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for v in view::snapshot(&self.store).await? {
            let Some(pk) = PubKey::parse(&v.peer) else { continue };
            if self.dm.presence_allowed(&pk).await? {
                out.push(v);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_core::envelope::KIND_PEER_NOTE_RUMOR;
    use messenger_core::MessengerConfig;
    use messenger_testkit::MemorySecretStore;
    use nostr::nips::nip59::UnwrappedGift;
    use nostr::prelude::Event;

    #[test]
    fn a_beat_is_due_only_in_sight_online_and_in_time() {
        let now = 10_000;
        let until = now + FOREGROUND_LEASE_SECS;
        assert!(beat_due(now, until, 0, false, 0), "the first beat goes at once");
        assert!(beat_due(now, until, now - BEAT_SECS, false, 0));
        assert!(!beat_due(now, until, now - BEAT_SECS + 1, false, 0), "too soon");
        assert!(!beat_due(now, until, now - BEAT_SECS, false, 3), "too soon with its jitter");
        assert!(beat_due(now, until, now - BEAT_SECS + 3, false, -3), "on time with its jitter");
        assert!(!beat_due(now, 0, 0, false, 0), "hidden");
        assert!(!beat_due(now, now, 0, false, 0), "the lease ran out");
        assert!(!beat_due(now, until, 0, true, 0), "silent mode");
    }

    #[test]
    fn the_focus_is_the_chat_named_while_both_leases_hold() {
        let now = 10_000;
        let (seen, read) = (now + FOREGROUND_LEASE_SECS, now + FOCUS_IDLE_SECS);
        assert_eq!(focus_wanted(now, seen, Some("dm:x"), read), Some("dm:x"));
        assert_eq!(focus_wanted(now, seen, None, read), None, "in the list");
        assert_eq!(focus_wanted(now, 0, Some("dm:x"), read), None, "hidden");
        assert_eq!(focus_wanted(now, seen, Some("dm:x"), now), None, "five minutes without a touch");
    }

    #[test]
    fn the_focus_is_said_at_a_change_and_every_half_minute_while_a_chat_is_read() {
        let now = 10_000;
        assert!(focus_due(now, Some("dm:x"), None, now - 1, false, 0), "a chat opened: at once");
        assert!(focus_due(now, Some("dm:y"), Some("dm:x"), now - 1, false, 0), "another chat: at once");
        assert!(focus_due(now, None, Some("dm:x"), now - 1, false, 0), "left: once, at once");
        assert!(!focus_due(now, None, None, 0, false, 0), "no chat: nothing again");
        assert!(!focus_due(now, Some("dm:x"), Some("dm:x"), now - BEAT_SECS + 1, false, 0), "too soon");
        assert!(focus_due(now, Some("dm:x"), Some("dm:x"), now - BEAT_SECS, false, 0));
        assert!(!focus_due(now, Some("dm:x"), Some("dm:x"), now - BEAT_SECS, false, 3), "too soon with its jitter");
        assert!(!focus_due(now, Some("dm:x"), None, 0, true, 0), "silent mode");
        // Two events late by all their jitter, and a tick late each, fit in
        // what one promises.
        assert!(2 * (BEAT_SECS + BEAT_JITTER_SECS + PRESENCE_TICK_SECS as i64) <= messenger_presence::FOCUS_TTL_SECS);
    }

    #[test]
    fn a_note_waits_until_its_since_is_no_more_than_a_second_ahead() {
        assert!(may_tell(100, 0));
        assert!(may_tell(100, 101), "a rotation tells at once");
        assert!(!may_tell(100, 102), "two moves in one second: the second waits");
        assert!(may_tell(101, 102));
    }

    #[test]
    fn the_jitter_stays_within_its_bounds_and_moves() {
        let all: Vec<i64> = (0..100).map(jitter).collect();
        assert!(all.iter().all(|j| (-BEAT_JITTER_SECS..=BEAT_JITTER_SECS).contains(j)));
        let distinct: HashSet<i64> = all.iter().copied().collect();
        assert_eq!(distinct.len() as i64, 2 * BEAT_JITTER_SECS + 1, "every value comes");
        // Two beats late by all their jitter, and a tick late each, still
        // fit in what a beat promises.
        let worst = BEAT_SECS + BEAT_JITTER_SECS + PRESENCE_TICK_SECS as i64;
        assert!(2 * worst <= messenger_presence::ONLINE_TTL_SECS);
    }

    /// A runtime with a session, offline, and the relationship gate off: a
    /// contact in the book is an approved one. What the session told my
    /// devices at its first look around (`session_tells_my_devices_where_it_stands`)
    /// has left the outbox.
    async fn started() -> (tempfile::TempDir, MessengerRuntime, Keys) {
        let (dir, rt, keys) = started_fresh().await;
        rt.presence_tick().await.unwrap();
        for row in messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap() {
            messenger_store::outbox::mark_published(rt.store(), &row.local_id).await.unwrap();
        }
        (dir, rt, keys)
    }

    /// As `started`, with the outbox as the session left it.
    async fn started_fresh() -> (tempfile::TempDir, MessengerRuntime, Keys) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = MessengerConfig::new(dir.path().join("messenger"));
        let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
        rt.relays().set_silent(true).await.unwrap();
        crate::servers::use_veydan_offline(&rt).await;
        crate::testing::log_in(&rt).await;
        assert!(rt.refresh_signer().await.unwrap());
        rt.dm().set_gate(false);
        let keys = rt.session_keys().await.unwrap();
        (dir, rt, keys)
    }

    /// What the outbox holds: notes to peers as (peer, envelope), notes to
    /// my devices as envelopes.
    async fn queued(rt: &MessengerRuntime, me: &Keys, peers: &[&Keys]) -> (Vec<(String, Envelope)>, Vec<Envelope>) {
        let rows = messenger_store::outbox::due(rt.store(), i64::MAX / 4, 0).await.unwrap();
        let (mut to_peers, mut to_me) = (Vec::new(), Vec::new());
        for r in &rows {
            match r.outbound().unwrap() {
                Outbound::PublishToInbox { recipient, event, .. } => {
                    assert_eq!(r.expires_at, None, "a key is tried until it leaves");
                    let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
                    assert!(ev.tags.iter().any(|t| t.as_slice() == ["silent", "1"]), "a key wakes nobody");
                    assert!(!ev.tags.iter().any(|t| t.kind() == "expiration"), "a key does not expire");
                    let peer = peers.iter().find(|k| k.public_key().to_hex() == recipient.as_hex()).expect("to a peer of the test");
                    let rumor = UnwrappedGift::from_gift_wrap(*peer, &ev).unwrap().rumor;
                    assert_eq!(rumor.kind.as_u16(), KIND_PEER_NOTE_RUMOR);
                    to_peers.push((recipient.as_hex().to_string(), Envelope::parse(&rumor.content).unwrap()));
                }
                Outbound::PublishOwn { event } => {
                    let ev: Event = serde_json::from_value(event.json.clone()).unwrap();
                    let Ok(u) = UnwrappedGift::from_gift_wrap(me, &ev) else { continue };
                    if u.rumor.kind.as_u16() == messenger_core::envelope::KIND_OWN_RUMOR {
                        to_me.push(Envelope::parse(&u.rumor.content).unwrap());
                    }
                }
                _ => {}
            }
        }
        (to_peers, to_me)
    }

    fn key_notes(of: &[(String, Envelope)]) -> Vec<(String, Option<String>)> {
        of.iter()
            .filter(|(_, e)| e.t == messenger_core::envelope::T_PRESENCE_KEY)
            .map(|(p, e)| (p.clone(), e.str_field("pubkey").map(str::to_string)))
            .collect()
    }

    async fn add_contact(rt: &MessengerRuntime, peer: &Keys) {
        let me = rt.session_pubkey().await.unwrap();
        rt.contacts().add(&me, &peer.public_key().to_hex(), None).await.unwrap();
    }


    /// One look around at `now`, as the loop makes it.
    async fn tick_at(rt: &MessengerRuntime, keys: &Keys, now: i64) {
        let pool = rt.relays().pool().await;
        rt.presence().tick(&pool, keys, now).await.unwrap();
    }

    /// `(epoch, sharing)` of each `own.presence` my devices were sent.
    fn own_notes(of: &[Envelope]) -> Vec<(u64, bool)> {
        of.iter()
            .filter(|e| e.t == messenger_core::envelope::T_OWN_PRESENCE)
            .map(|e| (e.fields["epoch"].as_u64().unwrap(), e.fields["sharing"].as_bool().unwrap()))
            .collect()
    }

    #[tokio::test]
    async fn approved_contacts_are_told_my_key_once() {
        let (_dir, rt, me) = started().await;
        let bob = Keys::generate();
        add_contact(&rt, &bob).await;
        rt.presence_tick().await.unwrap();
        rt.presence_tick().await.unwrap();

        let mine = key::derive(&me, 0).public_key().to_hex();
        let (to_peers, to_me) = queued(&rt, &me, &[&bob]).await;
        assert_eq!(key_notes(&to_peers), vec![(bob.public_key().to_hex(), Some(mine.clone()))], "told once");
        let note = &to_peers[0].1;
        assert_eq!(note.fields["since"], 0, "epoch 0 began at 0");
        let owner = PubKey::parse(&me.public_key().to_hex()).unwrap();
        assert!(messenger_dm::presence_proof_ok(&mine, &owner, 0, note.str_field("proof").unwrap()), "the key proves it is mine");
        assert!(to_me.is_empty());
        assert_eq!(presence::told(rt.store(), &bob.public_key().to_hex()).await.unwrap(), Some(0));
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn removing_a_contact_moves_the_key_and_tells_the_rest() {
        let (_dir, rt, me) = started().await;
        let (bob, carol) = (Keys::generate(), Keys::generate());
        add_contact(&rt, &bob).await;
        add_contact(&rt, &carol).await;
        rt.presence_tick().await.unwrap();

        rt.contact_remove(&PubKey::parse(&carol.public_key().to_hex()).unwrap()).await.unwrap();
        let state = rt.dm().presence_state().await.unwrap();
        assert_eq!((state.epoch, state.sharing), (1, true));
        let (_, to_me) = queued(&rt, &me, &[&bob, &carol]).await;
        let to_me: Vec<_> = to_me.into_iter().filter(|e| e.t == messenger_core::envelope::T_OWN_PRESENCE).collect();
        assert_eq!(to_me, vec![state.note()], "my devices learn the epoch and its since");

        tick_at(&rt, &me, state.since).await;
        let (to_peers, _) = queued(&rt, &me, &[&bob, &carol]).await;
        let new = key::derive(&me, 1).public_key().to_hex();
        let to_new: Vec<_> = to_peers.iter().filter(|(_, e)| e.str_field("pubkey") == Some(new.as_str())).collect();
        assert_eq!(to_new.len(), 1);
        assert_eq!(to_new[0].0, bob.public_key().to_hex(), "only who remains is told");
        assert_eq!(to_new[0].1.fields["since"], state.since);
        assert_eq!(presence::told_peers(rt.store()).await.unwrap(), vec![bob.public_key().to_hex()]);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_told_contact_that_is_gone_moves_the_key_by_itself() {
        let (_dir, rt, me) = started().await;
        let bob = Keys::generate();
        add_contact(&rt, &bob).await;
        rt.presence_tick().await.unwrap();
        // Gone from the book some other way (another device, an import).
        rt.contacts().remove(&PubKey::parse(&bob.public_key().to_hex()).unwrap()).await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(rt.dm().presence_epoch().await.unwrap(), 1);
        assert!(presence::told_peers(rt.store()).await.unwrap().is_empty());
        let (_, to_me) = queued(&rt, &me, &[&bob]).await;
        assert_eq!(own_notes(&to_me), vec![(1, true)]);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_contact_approved_on_my_other_device_is_watched_and_shown_here() {
        use crate::tests::{ctx_of, dm_inbound, pk_of};
        let (_dir, rt, me) = started().await;
        rt.dm().set_gate(true);
        let (me_pk, bob) = (pk_of(&me), Keys::generate());
        let bob_pk = pk_of(&bob);
        // Bob asked, my phone accepted: this device only sees the copies.
        let now = SystemClock.now().secs();
        let ctx = ctx_of(&me_pk, now);
        let control = |a: &str| Envelope::control(a).encode();
        rt.dm().apply_inbound(dm_inbound(&bob_pk, &me_pk, Envelope::text("hi").encode(), now - 3), &ctx).await.unwrap();
        rt.dm().apply_inbound(dm_inbound(&bob_pk, &me_pk, control("dm_accept"), now - 2), &ctx).await.unwrap();
        rt.dm().apply_inbound(dm_inbound(&me_pk, &bob_pk, control("dm_accept"), now - 1), &ctx).await.unwrap();
        assert_eq!(rt.dm().relation(&bob_pk).await.unwrap().mode, "full_chat");
        assert!(rt.contacts().is_contact(&bob_pk).await.unwrap(), "in this device's book too");

        // Bob's key (what his `presence.key` note leaves) and a beat of it.
        let bobs = key::derive(&bob, 0).public_key().to_hex();
        presence::put_key(rt.store(), bob_pk.as_hex(), Some(&bobs), 1).await.unwrap();
        presence::seen(rt.store(), &bobs, now, now + 80).await.unwrap();
        assert_eq!(rt.presence().approved(me_pk.as_hex()).await.unwrap(), vec![bob_pk.clone()], "his key is watched");
        assert_eq!(
            rt.presence_list().await.unwrap(),
            vec![PresenceView { peer: bob_pk.as_hex().to_string(), seen_at: now, online_until: now + 80 }]
        );
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_contact_i_removed_stays_unwatched_after_the_book_is_filled() {
        use crate::tests::pk_of;
        use messenger_store::dm_relations::{self, RelationRow};
        let (_dir, rt, me) = started().await;
        let bob = Keys::generate();
        // Removed from Contacts before the matrix; migration 007 still made
        // the relation approved from the chat.
        add_contact(&rt, &bob).await;
        rt.contacts().remove(&pk_of(&bob)).await.unwrap();
        let row = RelationRow {
            peer_pubkey: bob.public_key().to_hex(),
            server_id: String::new(),
            my_contact: "approved".into(),
            blocked: false,
            peer_signal: "approved".into(),
            was_ever_mutual: true,
            last_signal_at: 0,
            last_my_signal_at: 0,
            request_floor: 0,
            decided_at: 0,
            created_at: 0,
            updated_at: 0,
        };
        dm_relations::put(rt.store(), &row).await.unwrap();
        // The first start after the update: the fill has not run yet.
        messenger_store::settings::set_bool(rt.store(), "contacts.filled_from_relations", false).await.unwrap();

        assert!(rt.dm().fill_book_once().await.unwrap().is_empty());
        assert!(!rt.contacts().is_contact(&pk_of(&bob)).await.unwrap());
        assert!(rt.presence().approved(&me.public_key().to_hex()).await.unwrap().is_empty());
        rt.shutdown().await;
    }

    /// My own key is watched with my contacts' and known as mine, so the
    /// beat of my other device reaches a push here; presence off, nothing.
    #[tokio::test]
    async fn my_own_key_is_watched_for_my_other_device() {
        use messenger_core::presence::KEY_PRESENCE_MINE;
        let (_dir, rt, me) = started().await;
        let mine = |epoch| key::derive(&me, epoch).public_key().to_hex();
        let now = SystemClock.now().secs();
        let watched = {
            let _turn = rt.presence().turn().await;
            rt.presence().look_around(&me, now).await.unwrap()
        };
        assert_eq!(watched, vec![mine(0)], "no contact yet: my key alone");
        assert_eq!(settings::get(rt.store(), KEY_PRESENCE_MINE).await.unwrap(), Some(mine(0)));

        let epoch = rt.presence_rotate().await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(settings::get(rt.store(), KEY_PRESENCE_MINE).await.unwrap(), Some(mine(epoch)), "the key moved on");

        rt.privacy_set(crate::privacy::PrivacySettings { presence: false, ..rt.privacy_settings().await.unwrap() }).await.unwrap();
        let watched = {
            let _turn = rt.presence().turn().await;
            rt.presence().look_around(&me, now + 10).await.unwrap()
        };
        assert!(watched.is_empty(), "presence off: nothing watched");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_move_made_without_a_session_is_told_to_my_devices_at_the_next_look() {
        let (_dir, rt, me) = started().await;
        // As `presence_rotate` does with no session: the epoch moves, no note goes.
        let state = rt.dm().rotate_presence(true).await.unwrap();
        assert!(queued(&rt, &me, &[]).await.1.is_empty());
        rt.presence_tick().await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(queued(&rt, &me, &[]).await.1, vec![state.note()], "owed, then told once");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn session_tells_my_devices_where_it_stands() {
        // A device that never heard a rotation (added later, or the note
        // is gone from the relays) beat from a key no contact watched
        // (ROO-621): each session says where it stands, epoch 0 too.
        let (_dir, rt, me) = started_fresh().await;
        rt.presence_tick().await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(own_notes(&queued(&rt, &me, &[]).await.1), vec![(0, true)], "told once a session");

        // The next session tells again.
        let state = rt.dm().rotate_presence(true).await.unwrap();
        rt.presence_tick().await.unwrap();
        rt.presence().session_started().await;
        rt.presence_tick().await.unwrap();
        assert_eq!(own_notes(&queued(&rt, &me, &[]).await.1), vec![(0, true), (1, true), (1, true)]);
        assert_eq!(rt.dm().presence_state().await.unwrap(), state);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_device_behind_me_is_answered_with_my_epoch() {
        use crate::tests::{ctx_of, dm_inbound, pk_of};
        let (_dir, rt, me) = started().await;
        rt.dm().rotate_presence(true).await.unwrap();
        rt.dm().rotate_presence(true).await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(own_notes(&queued(&rt, &me, &[]).await.1), vec![(2, true)]);

        // My phone, at epoch 0 (it missed both), says where it stands.
        let now = SystemClock.now().secs();
        let mut msg = dm_inbound(&pk_of(&me), &pk_of(&me), Envelope::own_presence(0, 0, true).encode(), now);
        msg.rumor_kind = messenger_core::envelope::KIND_OWN_RUMOR;
        rt.dm().apply_inbound(msg, &ctx_of(&pk_of(&me), now)).await.unwrap();
        assert_eq!(rt.dm().presence_epoch().await.unwrap(), 2, "an older epoch changes nothing here");
        rt.presence_tick().await.unwrap();
        rt.presence_tick().await.unwrap();
        assert_eq!(own_notes(&queued(&rt, &me, &[]).await.1), vec![(2, true), (2, true)], "answered once");
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn turned_off_i_withdraw_my_key_show_nobody_and_tell_nothing() {
        let (_dir, rt, me) = started().await;
        let bob = Keys::generate();
        add_contact(&rt, &bob).await;
        rt.presence_tick().await.unwrap();
        // Bob told me his key and beats.
        let bobs = key::derive(&bob, 0).public_key().to_hex();
        presence::put_key(rt.store(), &bob.public_key().to_hex(), Some(&bobs), 1).await.unwrap();
        presence::seen(rt.store(), &bobs, 100, 180).await.unwrap();
        assert_eq!(rt.presence_list().await.unwrap(), vec![PresenceView { peer: bob.public_key().to_hex(), seen_at: 100, online_until: 180 }]);

        let mut s = rt.privacy_settings().await.unwrap();
        s.presence = false;
        rt.privacy_set(s.clone()).await.unwrap();
        let off = rt.dm().presence_state().await.unwrap();
        assert_eq!((off.epoch, off.sharing), (1, false));
        let (to_peers, to_me) = queued(&rt, &me, &[&bob]).await;
        let old = key::derive(&me, 0).public_key().to_hex();
        let mut notes = key_notes(&to_peers);
        notes.sort();
        assert_eq!(notes, vec![(bob.public_key().to_hex(), None), (bob.public_key().to_hex(), Some(old))], "the key, and that it is gone");
        let gone = to_peers.iter().find(|(_, e)| e.fields["pubkey"].is_null()).unwrap();
        assert_eq!(gone.1.fields["since"], off.since, "as of the epoch I stopped in");
        assert_eq!(to_me, vec![off.note()], "my devices turn it off too");
        assert!(presence::told_peers(rt.store()).await.unwrap().is_empty());
        assert!(rt.presence_list().await.unwrap().is_empty(), "shown nobody");

        // Off, nothing more is told, whatever the ticks.
        rt.presence_tick().await.unwrap();
        rt.privacy_set(s.clone()).await.unwrap();
        assert_eq!(queued(&rt, &me, &[&bob]).await.0.len(), 2);
        assert_eq!(rt.dm().presence_epoch().await.unwrap(), 1, "off again is no news");

        // On again: a new key, later than the withdrawal, told when its second comes.
        s.presence = true;
        rt.privacy_set(s).await.unwrap();
        let on = rt.dm().presence_state().await.unwrap();
        assert_eq!((on.epoch, on.sharing), (2, true));
        assert!(on.since > off.since);
        assert_eq!(own_notes(&queued(&rt, &me, &[&bob]).await.1), vec![(1, false), (2, true)]);
        tick_at(&rt, &me, on.since).await;
        let new = key::derive(&me, 2).public_key().to_hex();
        let notes = key_notes(&queued(&rt, &me, &[&bob]).await.0);
        assert_eq!(notes.len(), 3);
        assert!(notes.contains(&(bob.public_key().to_hex(), Some(new))));
        assert_eq!(rt.presence_list().await.unwrap().len(), 1);
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn turned_off_on_another_device_i_withdraw_what_this_one_told() {
        let (_dir, rt, me) = started().await;
        let bob = Keys::generate();
        add_contact(&rt, &bob).await;
        rt.presence_tick().await.unwrap();
        // What `own.presence {epoch 1, sharing false}` from my phone leaves here.
        let since = SystemClock.now().secs() + 1;
        for (k, v) in [(KEY_PRESENCE_EPOCH, "1".to_string()), (KEY_PRESENCE_SINCE, since.to_string()), (KEY_PRESENCE_DEVICES_TOLD, "1".into())] {
            settings::set(rt.store(), k, &v).await.unwrap();
        }
        settings::set_bool(rt.store(), KEY_PRESENCE, false).await.unwrap();
        presence::carry_told(rt.store(), 1).await.unwrap();

        tick_at(&rt, &me, since).await;
        let (to_peers, to_me) = queued(&rt, &me, &[&bob]).await;
        let gone: Vec<_> = to_peers.iter().filter(|(_, e)| e.fields.get("pubkey").is_some_and(|p| p.is_null())).collect();
        assert_eq!(gone.len(), 1, "Bob, told here, is told it is gone");
        assert_eq!(gone[0].1.fields["since"], since);
        assert!(to_me.is_empty(), "the phone told my devices");
        assert!(presence::told_peers(rt.store()).await.unwrap().is_empty());
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn a_beat_counts_only_when_a_relay_took_it() {
        use messenger_core::traits::RelayState;
        use messenger_transport::RelayConfig;

        let (_dir, rt, me) = started().await;
        let now = SystemClock.now().secs();
        // Silent relays: never due, in sight or not.
        rt.presence_foreground(true).await.unwrap();
        let silent = rt.relays().pool().await;
        assert!(!rt.presence().beat_if_due(&silent, &me, now).await);

        // No relay to take it: not done, so the next chance tries again.
        let nowhere = RelayPool::new(None);
        rt.presence().set_foreground(false, now);
        assert!(!rt.presence().beat_if_due(&nowhere, &me, now).await, "hidden");
        rt.presence().set_foreground(true, now);
        assert!(!rt.presence().beat_if_due(&nowhere, &me, now).await, "nobody took it");

        let relay = messenger_testkit::local_relay().await;
        let url = messenger_core::RelayUrl::parse(relay.url().await.as_str_without_trailing_slash()).unwrap();
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url, read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
        for _ in 0..50 {
            if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(rt.presence().beat_if_due(&pool, &me, now + 1).await, "the beat that found nobody goes at once");
        assert!(!rt.presence().beat_if_due(&pool, &me, now + 2).await, "not again so soon");
        let later = now + 1 + BEAT_SECS + BEAT_JITTER_SECS;
        assert!(rt.presence().beat_if_due(&pool, &me, later).await);
        rt.presence().woke();
        assert!(rt.presence().beat_if_due(&pool, &me, later + 1).await, "a wake beats at once");
        assert!(!rt.presence().beat_if_due(&pool, &me, now + FOREGROUND_LEASE_SECS + 1).await, "the lease ran out");

        // Off: no beat, whatever is due.
        rt.presence().set_foreground(true, later + 60);
        rt.presence().woke();
        settings::set_bool(rt.store(), KEY_PRESENCE, false).await.unwrap();
        assert!(!rt.presence().beat_if_due(&pool, &me, later + 60).await, "presence off");
        pool.shutdown().await;
        rt.shutdown().await;
    }

    /// A pool connected to `url`.
    async fn pool_on(url: &messenger_core::RelayUrl) -> RelayPool {
        use messenger_core::traits::RelayState;
        use messenger_transport::RelayConfig;
        let pool = RelayPool::new(None);
        pool.set_relays(vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: None, server_id: None }]).await.unwrap();
        for _ in 0..50 {
            if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        pool
    }

    /// A word of the page that waited (for the lock, for the screen) while
    /// a later one went through is dropped: the later stands (ROO-1292).
    #[tokio::test]
    async fn a_word_of_the_focus_late_behind_a_later_one_is_dropped() {
        let (_dir, rt, _me) = started().await;
        let now = SystemClock.now().secs();
        let p = rt.presence();
        p.set_foreground(true, now);
        let read = || {
            let lease = p.focus_lease.lock().unwrap();
            focus_wanted(now + 1, p.foreground_until.load(Ordering::SeqCst), lease.0.as_deref(), lease.1).map(str::to_string)
        };
        // As `messenger_focus` does: the number is taken when the page
        // asks, the word is applied after the checks.
        let delayed = |chat: Option<&str>, asked: u64, gate: tokio::sync::oneshot::Receiver<()>| {
            let chat = chat.map(str::to_string);
            let rt = &rt;
            async move {
                gate.await.unwrap();
                rt.focus(chat, asked).await.unwrap();
            }
        };

        // X waits for its check; the window loses the focus: none goes first.
        let (open, gate) = tokio::sync::oneshot::channel();
        tokio::join!(delayed(Some("dm:x"), 1, gate), async {
            rt.focus(None, 2).await.unwrap();
            open.send(()).unwrap();
        });
        assert_eq!(read(), None, "the late X does not come back");

        // X waits; Y is opened meanwhile.
        let (open, gate) = tokio::sync::oneshot::channel();
        tokio::join!(delayed(Some("dm:x"), 3, gate), async {
            rt.focus(Some("dm:y".into()), 4).await.unwrap();
            open.send(()).unwrap();
        });
        assert_eq!(read().as_deref(), Some("dm:y"), "the late X does not cover Y");

        // In order, each is taken.
        rt.focus(Some("dm:z".into()), 5).await.unwrap();
        assert_eq!(read().as_deref(), Some("dm:z"));
        rt.focus(None, 6).await.unwrap();
        assert_eq!(read(), None);
    }

    /// The focus goes as a beat, past the outbox: at once when a chat is
    /// opened, changed or left, every half minute while it is read, never
    /// past five minutes without a touch; with presence off too. Each says
    /// its chat, sealed for my devices, and is written down as this
    /// device's before it goes.
    #[tokio::test]
    async fn the_focus_says_the_chat_read_here_and_when_it_is_left() {
        let (_dir, rt, me) = started().await;
        let relay = messenger_testkit::local_relay().await;
        let url = messenger_core::RelayUrl::parse(relay.url().await.as_str_without_trailing_slash()).unwrap();
        let pool = pool_on(&url).await;
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        // My other device, listening as the runtime does.
        let phone = pool_on(&url).await;
        let mut events = messenger_core::Transport::events(&phone);
        let author = PubKey::parse(&keys.public_key().to_hex()).unwrap();
        phone.send(Outbound::Subscribe { id: SubId(filters::SUB_FOCUS.into()), filter: filters::focus_of(&author), scope: Scope::Own }).await.unwrap();
        settings::set_bool(rt.store(), KEY_PRESENCE, false).await.unwrap();
        let p = rt.presence();
        let now = SystemClock.now().secs();
        let said = |e: &messenger_core::outbound::WireEvent| {
            let ev = focus::parse(&e.json).unwrap();
            (focus::open(&keys, &ev).unwrap().chat, ev.until() - ev.created_at)
        };

        // Watched with presence off: the subscription is of my focus key.
        p.watch_focus(&pool, &me).await;
        assert_eq!(p.focus_watched.lock().await.clone(), Some(keys.public_key().to_hex()));

        p.set_foreground(true, now);
        assert!(!p.focus_if_due(&pool, &me, now).await, "in the list: nothing");
        p.set_focus(Some("dm:x".into()), now, 1);
        assert!(p.focus_if_due(&pool, &me, now).await, "a chat opened: at once");
        assert!(!p.focus_if_due(&pool, &me, now + 1).await, "not again so soon");
        p.set_focus(Some("dm:y".into()), now + 2, 2);
        assert!(p.focus_if_due(&pool, &me, now + 2).await, "another chat: at once");
        let later = now + 2 + BEAT_SECS + BEAT_JITTER_SECS;
        p.set_foreground(true, later);
        assert!(p.focus_if_due(&pool, &me, later).await, "half a minute on");
        assert!(p.focus_if_due(&pool, &me, now + 2 + FOCUS_IDLE_SECS).await, "no touch for five minutes: no chat");
        assert!(!p.focus_if_due(&pool, &me, now + 3 + FOCUS_IDLE_SECS).await, "and nothing more");
        p.set_focus(Some("dm:y".into()), now + 400, 3);
        p.set_foreground(true, now + 400);
        assert!(p.focus_if_due(&pool, &me, now + 400).await);
        p.set_foreground(false, now + 401);
        assert!(p.focus_if_due(&pool, &me, now + 401).await, "hidden: no chat, at once");

        // What went, in order, as the subscription of my focus key heard
        // it; each is known as this device's own.
        let sent = settings::get(rt.store(), messenger_core::presence::KEY_FOCUS_SENT).await.unwrap().unwrap();
        let (mut ids, mut got) = (Vec::new(), Vec::new());
        while let Ok(Some(raw)) = tokio::time::timeout(Duration::from_secs(2), events.recv()).await {
            if ids.contains(&raw.id) {
                continue;
            }
            assert!(sent.split(' ').any(|id| id == raw.id.as_hex()), "written down before it went");
            got.push(said(&messenger_core::outbound::WireEvent { id: raw.id.clone(), json: raw.json.clone() }));
            ids.push(raw.id);
        }
        let ttl = messenger_presence::FOCUS_TTL_SECS;
        let chats: Vec<Option<&str>> = vec![Some("dm:x"), Some("dm:y"), Some("dm:y"), None, Some("dm:y"), None];
        assert_eq!(got, chats.iter().map(|c| (c.map(str::to_string), ttl)).collect::<Vec<_>>());
        phone.shutdown().await;
        pool.shutdown().await;
        rt.shutdown().await;
    }

    /// A chat opened and left, or changed, within one second: the relay
    /// keeps the last word, not the one of the smaller id (ROO-1300).
    #[tokio::test]
    async fn the_last_word_of_one_second_is_what_the_relay_keeps() {
        use nostr_sdk::prelude::{Client, Filter, Kind};
        let (_dir, rt, me) = started().await;
        let relay = messenger_testkit::local_relay().await;
        let url = messenger_core::RelayUrl::parse(relay.url().await.as_str_without_trailing_slash()).unwrap();
        let pool = pool_on(&url).await;
        let keys = FocusKeys::derive(&me, FOCUS_EPOCH);
        // What the phone woken by a push gets: the one event of my focus key.
        let phone = Client::default();
        phone.add_relay(url.as_str()).await.unwrap();
        phone.connect().await;
        let kept = || async {
            let filter = Filter::new().kind(Kind::from(focus::KIND_FOCUS)).author(keys.public_key()).identifier(messenger_presence::FOCUS_D);
            let events = phone.fetch_events(filter).timeout(Duration::from_secs(2)).await.unwrap();
            assert_eq!(events.len(), 1, "one focus event of the key");
            let json = serde_json::to_value(events.first().unwrap()).unwrap();
            focus::open(&keys, &focus::parse(&json).unwrap()).unwrap().chat
        };
        let p = rt.presence();
        let now = SystemClock.now().secs();
        p.set_foreground(true, now);

        let mut asked = 0;
        let mut say = |chat: Option<&str>| {
            asked += 1;
            p.set_focus(chat.map(str::to_string), now, asked);
        };
        say(Some("dm:x"));
        assert!(p.focus_if_due(&pool, &me, now).await);
        say(None);
        assert!(p.focus_if_due(&pool, &me, now).await, "left in the same second: said too");
        assert_eq!(kept().await, None, "X then left: no chat");
        say(Some("dm:x"));
        assert!(p.focus_if_due(&pool, &me, now).await);
        say(Some("dm:y"));
        assert!(p.focus_if_due(&pool, &me, now).await);
        assert_eq!(kept().await.as_deref(), Some("dm:y"), "X then Y: Y");
        say(None);
        assert!(p.focus_if_due(&pool, &me, now).await);
        say(Some("dm:x"));
        assert!(p.focus_if_due(&pool, &me, now).await);
        assert_eq!(kept().await.as_deref(), Some("dm:x"), "six words of one second, the last kept");

        // Past FOCUS_LEAD_SECS ahead the word waits for a later second.
        say(None);
        assert!(!p.focus_if_due(&pool, &me, now).await, "too far ahead of the clock");
        assert!(p.focus_if_due(&pool, &me, now + 2).await, "a later tick says it");
        assert_eq!(kept().await, None);
        phone.disconnect().await;
        pool.shutdown().await;
        rt.shutdown().await;
    }

    #[tokio::test]
    async fn what_another_device_reads_is_read_elsewhere() {
        let (_dir, rt, _me) = started().await;
        let now = SystemClock.now().secs();
        assert!(!rt.read_elsewhere("dm:x").await.unwrap());
        elsewhere::heard(rt.store(), "phone", Some("dm:x"), now, now + 80).await.unwrap();
        assert!(rt.read_elsewhere("dm:x").await.unwrap());
        assert!(!rt.read_elsewhere("dm:y").await.unwrap());
        rt.shutdown().await;
    }
}

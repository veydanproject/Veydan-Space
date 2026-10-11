// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Social Recovery, the owner's side (internal/cloud-recovery.md §8.1,
//! §8.3): the trusted people chosen among the approved contacts, the
//! Recovery Key split into their shares (crates/recovery, §8.2) and
//! delivered silently as `recovery.share`, their acks, the ping every 90
//! days and its pong, the revoke when the key is reissued or a person
//! replaced, and `PUT /cloud/v1/recovery/config` with the numbers.
//!
//! The list of the trusted people and the state of their acks and pongs
//! live in the box `cloud` of the lock on this device alone ([`Trustees`]
//! under [`KEY_TRUSTEES`]); the escrow carries the numbers (`k`, `n`) and
//! never a name (§8.3). The notes travel through the seam `PeerNotes`; the
//! module never goes to the relays itself. What comes back (`ack`, `pong`)
//! is heard by [`listen`], which also hands the trustee's notes to
//! `trustee.rs` and the owner's `recovery.cancel` to `cancel.rs`.
//!
//! Step 5 of a reissue (`reissue.rs`) goes through [`Social`] as its
//! [`ShareDelivery`]: the shares of the new generation go out, and the
//! reissue waits for `k` acks for as long as [`Social::ack_patience`]
//! says; with fewer the reissue is rolled back, the new generation revoked
//! and the shares of the generation in force sent once more.

use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, Runtime};
use veydan_recovery::{shamir, RecoveryKey, RecoveryParams};
use veydan_shell::seams::{
    code, ApprovedContact, CloudError, Contacts, LockBox, Note, PeerNotes, SeamFuture, Signer,
};

use crate::api::{now, CloudApi};
use crate::escrow;
use crate::reissue::ShareDelivery;

/// The key of the box `cloud`: the trusted people of this device, JSON of
/// [`Trustees`].
pub const KEY_TRUSTEES: &str = "recovery.trustees";
/// The event to the screens when the trusted people or their state
/// changed, with a [`TrusteesStatus`].
pub const EVENT_TRUSTEES: &str = "cloud://trustees";

/// The bounds of §8.1: 3 to 7 people, `2 <= k <= n`.
pub const MIN_PEOPLE: u8 = 3;
pub const MAX_PEOPLE: u8 = 7;
/// The default of §8.1: 3 of 5.
pub const DEFAULT_K: u8 = 3;
pub const DEFAULT_N: u8 = 5;
/// No ack within a day: «не доставлено» (§8.3).
pub const ACK_WAIT_SECS: u64 = 24 * 3600;
/// A ping to every trusted person every 90 days (§8.3).
pub const PING_EVERY_SECS: u64 = 90 * 86_400;
/// No pong a week after a ping: «доля не подтверждена», the share sent again.
pub const PONG_WAIT_SECS: u64 = 7 * 86_400;
/// How long a reissue waits for the acks of the new shares before it is
/// rolled back; the screen shows the count meanwhile.
pub const DEFAULT_ACK_PATIENCE: Duration = Duration::from_secs(90);
/// The outer `expiration` of an ack, a ping and a pong (messenger-wire.md §3).
pub const WEEK_SECS: u64 = 7 * 86_400;

/// The types of the notes (messenger-wire.md §3).
pub const T_SHARE: &str = "recovery.share";
pub const T_ACK: &str = "recovery.share.ack";
pub const T_PING: &str = "recovery.share.ping";
pub const T_PONG: &str = "recovery.share.pong";
pub const T_REVOKE: &str = "recovery.share.revoke";

/// The words of the refusals of this side.
pub mod words {
    /// Fewer than 3 or more than 7 people, or one twice.
    pub const TRUSTEES_COUNT: &str = "trustees_count";
    /// `k` out of `2 <= k <= n`.
    pub const THRESHOLD: &str = "threshold";
    /// A person chosen is not an approved contact.
    pub const NOT_APPROVED: &str = "not_approved";
    /// No Recovery Key of the cloud is issued on this device yet.
    pub const RK_NOT_ISSUED: &str = "rk_not_issued";
    /// No trusted people are set up on this device.
    pub const NOT_SET_UP: &str = "not_set_up";
    /// Fewer than `k` people confirmed the new shares in time.
    pub const SHARES_NOT_ACKED: &str = "shares_not_acked";
}

/// One trusted person as this device keeps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trustee {
    /// 64 hex.
    pub pubkey: String,
    /// The name as the picker showed it, for this device's screens alone.
    pub name: String,
    /// `x` of the share, 1..=n.
    pub index: u8,
    /// When the share of the current generation went, unix seconds.
    pub sent_at: u64,
    /// The first ack of the current generation, by any device of theirs.
    pub acked_at: Option<u64>,
    /// The last ping sent.
    pub pinged_at: Option<u64>,
    /// The last pong with `have: true`.
    pub ponged_at: Option<u64>,
    /// A pong with `have: false` came after the last share: the share was
    /// sent again at this time.
    pub resent_at: Option<u64>,
}

/// The trusted people of this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trustees {
    pub k: u8,
    pub n: u8,
    /// The generation of the shares out there.
    pub generation: u32,
    pub people: Vec<Trustee>,
    pub updated_at: u64,
}

/// The state of a share at one person, for the screen (§8.3, §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// Sent less than a day ago, no ack yet.
    Waiting,
    /// No ack a day after the share went: «не доставлено».
    NotDelivered,
    /// Acked; no ping asked since.
    Delivered,
    /// The last ping was answered «have».
    Alive,
    /// A week after a ping, no pong: «доля не подтверждена».
    Unconfirmed,
}

/// One trusted person as the screen shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrusteeView {
    pub pubkey: String,
    pub name: String,
    pub index: u8,
    pub health: Health,
    pub acked_at: Option<u64>,
    /// The last pong with `have`, unix seconds: «проверено N дней назад».
    pub checked_at: Option<u64>,
}

/// What «Доверенные люди» shows (§11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrusteesStatus {
    /// Trusted people are set up on this device.
    pub configured: bool,
    pub k: u8,
    pub n: u8,
    pub generation: u32,
    pub people: Vec<TrusteeView>,
    /// How many acked the current generation.
    pub acked: u8,
    /// How many answered the last ping with `have`: «доли живы: 5 из 5».
    pub alive: u8,
    /// The earliest of the last pongs: «проверено N дней назад».
    pub checked_at: Option<u64>,
}

/// The clock of the side, unix seconds; a fake one in the tests.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_now() -> u64 {
    now().unwrap_or(0)
}

/// The one lane of the set in the box: a command and a note that came
/// (an ack while the people are being changed) read and write it in turn,
/// never over each other.
static LANE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// 64 hex, lower case, as 32 bytes.
pub fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The trusted people kept in the box, if any.
pub async fn load(boxed: &dyn LockBox) -> Result<Option<Trustees>, CloudError> {
    let Some(bytes) = boxed.get(KEY_TRUSTEES).await? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| crypto(format!("the trusted people in the box: {e}")))
}

async fn save(boxed: &dyn LockBox, trustees: &Trustees) -> Result<(), CloudError> {
    let json = serde_json::to_vec(trustees).map_err(crypto)?;
    boxed.put(KEY_TRUSTEES, &json).await
}

/// The numbers of the trusted people the escrow carries (§4 `recovery`):
/// `k` and `n` of this device's set, 0 of 0 without one.
pub async fn recovery_params(
    boxed: &dyn LockBox,
    generation: u32,
) -> Result<RecoveryParams, CloudError> {
    let (k, n) = match load(boxed).await? {
        Some(t) => (t.k, t.n),
        None => (0, 0),
    };
    Ok(RecoveryParams { k, n, generation })
}

/// The health of one person at `now` (§8.3).
pub fn health_of(t: &Trustee, now: u64) -> Health {
    match (t.acked_at, t.pinged_at) {
        (None, _) if now.saturating_sub(t.sent_at) >= ACK_WAIT_SECS => Health::NotDelivered,
        (None, _) => Health::Waiting,
        (Some(_), None) => Health::Delivered,
        (Some(_), Some(pinged)) => match t.ponged_at {
            Some(ponged) if ponged >= pinged => Health::Alive,
            _ if now.saturating_sub(pinged) >= PONG_WAIT_SECS => Health::Unconfirmed,
            // Acked, pinged, the week not over: still as delivered.
            _ => Health::Delivered,
        },
    }
}

/// The status of a set at `now`.
pub fn status_of(trustees: Option<&Trustees>, now: u64) -> TrusteesStatus {
    let Some(t) = trustees else {
        return TrusteesStatus {
            configured: false,
            k: DEFAULT_K,
            n: DEFAULT_N,
            generation: 0,
            people: Vec::new(),
            acked: 0,
            alive: 0,
            checked_at: None,
        };
    };
    let people: Vec<TrusteeView> = t
        .people
        .iter()
        .map(|p| TrusteeView {
            pubkey: p.pubkey.clone(),
            name: p.name.clone(),
            index: p.index,
            health: health_of(p, now),
            acked_at: p.acked_at,
            checked_at: p.ponged_at.filter(|at| p.pinged_at.is_some_and(|pinged| *at >= pinged)),
        })
        .collect();
    let acked = people.iter().filter(|p| p.acked_at.is_some()).count() as u8;
    let alive = people.iter().filter(|p| p.health == Health::Alive).count() as u8;
    let checked_at = people.iter().filter_map(|p| p.checked_at).min();
    TrusteesStatus {
        configured: true,
        k: t.k,
        n: t.n,
        generation: t.generation,
        people,
        acked,
        alive,
        checked_at,
    }
}

/// The owner's side over the seams of the messenger.
#[derive(Clone)]
pub struct Social {
    notes: Arc<dyn PeerNotes>,
    boxed: Arc<dyn LockBox>,
    api: Arc<dyn CloudApi>,
    /// My key, 64 hex: the `npub` of every share.
    me: String,
    clock: Clock,
    /// How long a reissue waits for `k` acks.
    pub ack_patience: Duration,
    /// Told when the state changed; the app sends [`EVENT_TRUSTEES`].
    changed: Option<Arc<dyn Fn(TrusteesStatus) + Send + Sync>>,
}

impl Social {
    /// A side with the clock of the system and the patience of production.
    pub fn new(
        notes: Arc<dyn PeerNotes>,
        boxed: Arc<dyn LockBox>,
        api: Arc<dyn CloudApi>,
        me: String,
    ) -> Self {
        Self {
            notes,
            boxed,
            api,
            me,
            clock: Arc::new(system_now),
            ack_patience: DEFAULT_ACK_PATIENCE,
            changed: None,
        }
    }

    /// The same with another clock and patience: the tests.
    pub fn with(mut self, clock: Clock, ack_patience: Duration) -> Self {
        self.clock = clock;
        self.ack_patience = ack_patience;
        self
    }

    /// Told of every change of the state.
    pub fn on_change(mut self, changed: Arc<dyn Fn(TrusteesStatus) + Send + Sync>) -> Self {
        self.changed = Some(changed);
        self
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    async fn tell(&self) {
        if let Some(changed) = &self.changed {
            if let Ok(trustees) = load(self.boxed.as_ref()).await {
                changed(status_of(trustees.as_ref(), self.now()));
            }
        }
    }

    /// What the screen shows now.
    pub async fn status(&self) -> Result<TrusteesStatus, CloudError> {
        Ok(status_of(load(self.boxed.as_ref()).await?.as_ref(), self.now()))
    }

    fn my_npub(&self) -> Result<[u8; 32], CloudError> {
        unhex32(&self.me).ok_or_else(|| crypto("my key is not 64 hex"))
    }

    /// The shares of `rk` under `generation` for `people`, in their order;
    /// each goes to its person as `recovery.share`. A note that does not go
    /// is an error: the set is not changed by half.
    async fn send_shares(
        &self,
        rk: &RecoveryKey,
        generation: u32,
        k: u8,
        people: &[ApprovedContact],
    ) -> Result<(), CloudError> {
        let n = people.len() as u8;
        let shares = shamir::split(rk, &self.my_npub()?, generation, k, n).map_err(crypto)?;
        for (person, share) in people.iter().zip(shares.iter()) {
            let mut fields = Map::new();
            fields.insert(
                "share".into(),
                Value::String(B64.encode(share.to_bytes().as_slice())),
            );
            self.notes.send(Note::new(T_SHARE, fields), &person.pubkey).await?;
        }
        Ok(())
    }

    /// `recovery.share.revoke` of `generation` to `pubkey`, as best it goes.
    async fn send_revoke(&self, pubkey: &str, generation: u32) -> bool {
        let mut fields = Map::new();
        fields.insert("npub".into(), Value::String(self.me.clone()));
        fields.insert("generation".into(), Value::from(generation));
        self.notes.send(Note::new(T_REVOKE, fields), pubkey).await.is_ok()
    }

    /// §8.1: `people` (3 to 7 approved contacts, each once) hold the shares
    /// of the key in force, any `k` of them enough. A set in place is
    /// replaced: everyone chosen gets a fresh share of the same generation,
    /// whoever is no longer chosen a revoke; this is also «Заменить» one
    /// person. Then `PUT /cloud/v1/recovery/config`. The escrow is written
    /// again by the caller (its `recovery` numbers changed).
    pub async fn setup(
        &self,
        k: u8,
        people: Vec<ApprovedContact>,
        approved: &[ApprovedContact],
    ) -> Result<TrusteesStatus, CloudError> {
        let n = u8::try_from(people.len()).unwrap_or(u8::MAX);
        if !(MIN_PEOPLE..=MAX_PEOPLE).contains(&n) {
            return Err(CloudError::new(
                words::TRUSTEES_COUNT,
                "choose from 3 to 7 trusted people",
            ));
        }
        for (i, p) in people.iter().enumerate() {
            if unhex32(&p.pubkey).is_none() {
                return Err(CloudError::new(code::BAD_REQUEST, "a key is 64 hex in lower case"));
            }
            if p.pubkey == self.me || people[..i].iter().any(|q| q.pubkey == p.pubkey) {
                return Err(CloudError::new(
                    words::TRUSTEES_COUNT,
                    "every trusted person is chosen once, and not yourself",
                ));
            }
            if !approved.iter().any(|a| a.pubkey == p.pubkey) {
                return Err(CloudError::new(
                    words::NOT_APPROVED,
                    format!("{} is not an approved contact", p.pubkey),
                ));
            }
        }
        if !(shamir::MIN_THRESHOLD <= k && k <= n) {
            return Err(CloudError::new(words::THRESHOLD, "2 ≤ k ≤ n"));
        }
        let _lane = LANE.lock().await;
        let rk = escrow::load_rk(self.boxed.as_ref())
            .await?
            .ok_or_else(|| CloudError::new(words::RK_NOT_ISSUED, "issue the Recovery Key first"))?;
        let generation = escrow::load_generation(self.boxed.as_ref()).await?;
        let before = load(self.boxed.as_ref()).await?;
        let now = self.now();
        let trustees = Trustees {
            k,
            n,
            generation,
            people: people
                .iter()
                .enumerate()
                .map(|(i, p)| Trustee {
                    pubkey: p.pubkey.clone(),
                    name: p.name.clone(),
                    index: i as u8 + 1,
                    sent_at: now,
                    acked_at: None,
                    pinged_at: None,
                    ponged_at: None,
                    resent_at: None,
                })
                .collect(),
            updated_at: now,
        };
        // The set first, the shares after: an ack that comes at once finds
        // the person it is for.
        save(self.boxed.as_ref(), &trustees).await?;
        if let Err(e) = self.send_shares(&rk, generation, k, &people).await {
            match &before {
                Some(before) => save(self.boxed.as_ref(), before).await?,
                None => self.boxed.delete(KEY_TRUSTEES).await?,
            }
            return Err(e);
        }
        if let Some(before) = before {
            for gone in before
                .people
                .iter()
                .filter(|p| !people.iter().any(|q| q.pubkey == p.pubkey))
            {
                self.send_revoke(&gone.pubkey, before.generation).await;
            }
        }
        self.api.recovery_config_put(k, n, generation).await?;
        self.tell().await;
        Ok(status_of(Some(&trustees), now))
    }

    /// «Выключить восстановление» of the trusted people: a revoke to every
    /// one of them, the config deleted on the server, the set forgotten.
    pub async fn disable(&self) -> Result<u32, CloudError> {
        let _lane = LANE.lock().await;
        let Some(trustees) = load(self.boxed.as_ref()).await? else {
            return Err(CloudError::new(words::NOT_SET_UP, "no trusted people on this device"));
        };
        let revoked = self.revoke_everyone(&trustees).await;
        self.api.recovery_config_delete().await?;
        self.boxed.delete(KEY_TRUSTEES).await?;
        self.tell().await;
        Ok(revoked)
    }

    /// A revoke of the generation in force to every trusted person, best
    /// effort (plan 4.6, the deletion of the account); how many went.
    pub async fn revoke_all(&self) -> Result<u32, CloudError> {
        match load(self.boxed.as_ref()).await? {
            Some(trustees) => Ok(self.revoke_everyone(&trustees).await),
            None => Ok(0),
        }
    }

    async fn revoke_everyone(&self, trustees: &Trustees) -> u32 {
        let mut revoked = 0;
        for p in &trustees.people {
            if self.send_revoke(&p.pubkey, trustees.generation).await {
                revoked += 1;
            }
        }
        revoked
    }

    /// `recovery.share.ack` from `from`: counted by the person, not the
    /// device; `npub` must be mine and `generation` the one out there.
    pub async fn on_ack(&self, from: &str, fields: &Map<String, Value>) -> Result<bool, CloudError> {
        let _lane = LANE.lock().await;
        let Some(mut trustees) = load(self.boxed.as_ref()).await? else {
            return Ok(false);
        };
        if fields.get("npub").and_then(Value::as_str) != Some(self.me.as_str())
            || fields.get("generation").and_then(Value::as_u64) != Some(u64::from(trustees.generation))
        {
            return Ok(false);
        }
        let index = fields.get("index").and_then(Value::as_u64);
        let now = self.now();
        let Some(person) = trustees
            .people
            .iter_mut()
            .find(|p| p.pubkey == from && index.is_none_or(|i| i == u64::from(p.index)))
        else {
            return Ok(false);
        };
        if person.acked_at.is_some() {
            return Ok(false);
        }
        person.acked_at = Some(now);
        trustees.updated_at = now;
        save(self.boxed.as_ref(), &trustees).await?;
        self.tell().await;
        Ok(true)
    }

    /// `recovery.share.pong` from `from`: «have» marks the share alive;
    /// «have: false» after the last share sends it again (idempotent).
    pub async fn on_pong(&self, from: &str, fields: &Map<String, Value>) -> Result<bool, CloudError> {
        let _lane = LANE.lock().await;
        let Some(mut trustees) = load(self.boxed.as_ref()).await? else {
            return Ok(false);
        };
        if fields.get("npub").and_then(Value::as_str) != Some(self.me.as_str())
            || fields.get("generation").and_then(Value::as_u64) != Some(u64::from(trustees.generation))
        {
            return Ok(false);
        }
        let have = fields.get("have").and_then(Value::as_bool).unwrap_or(false);
        let now = self.now();
        let Some(person) = trustees.people.iter_mut().find(|p| p.pubkey == from) else {
            return Ok(false);
        };
        if have {
            person.ponged_at = Some(now);
            if person.acked_at.is_none() {
                person.acked_at = Some(now);
            }
        } else if person.resent_at.is_none_or(|at| at < person.sent_at) {
            // One device of theirs lost the share: it goes again to all.
            let rk = escrow::load_rk(self.boxed.as_ref()).await?;
            if let Some(rk) = rk {
                let people: Vec<ApprovedContact> = trustees
                    .people
                    .iter()
                    .map(|p| ApprovedContact {
                        pubkey: p.pubkey.clone(),
                        name: p.name.clone(),
                    })
                    .collect();
                let (k, generation) = (trustees.k, trustees.generation);
                self.send_shares(&rk, generation, k, &people).await?;
                for p in trustees.people.iter_mut() {
                    p.sent_at = now;
                    p.resent_at = Some(now);
                    p.acked_at = None;
                }
            }
        }
        trustees.updated_at = now;
        save(self.boxed.as_ref(), &trustees).await?;
        self.tell().await;
        Ok(true)
    }

    /// The health round (§8.3): a ping to everyone whose last ping is 90
    /// days old (or who was never pinged, with `force`), the shares sent
    /// again to whoever did not answer a ping within a week. Once a day,
    /// and «Проверить» on the screen with `force`.
    pub async fn check(&self, force: bool) -> Result<TrusteesStatus, CloudError> {
        let _lane = LANE.lock().await;
        let Some(mut trustees) = load(self.boxed.as_ref()).await? else {
            return Ok(status_of(None, self.now()));
        };
        let now = self.now();
        let mut changed = false;
        let mut resend = false;
        for p in trustees.people.iter_mut() {
            let due = match p.pinged_at {
                None => force || now.saturating_sub(p.sent_at) >= PING_EVERY_SECS,
                Some(at) => force || now.saturating_sub(at) >= PING_EVERY_SECS,
            };
            if p.acked_at.is_some() && health_of(p, now) == Health::Unconfirmed {
                resend = true;
            }
            if due && p.acked_at.is_some() {
                let mut fields = Map::new();
                fields.insert("generation".into(), Value::from(trustees.generation));
                let mut note = Note::new(T_PING, fields);
                note.expires_at = Some(now + WEEK_SECS);
                if self.notes.send(note, &p.pubkey).await.is_ok() {
                    p.pinged_at = Some(now);
                    changed = true;
                }
            }
        }
        if resend {
            if let Some(rk) = escrow::load_rk(self.boxed.as_ref()).await? {
                let people: Vec<ApprovedContact> = trustees
                    .people
                    .iter()
                    .map(|p| ApprovedContact {
                        pubkey: p.pubkey.clone(),
                        name: p.name.clone(),
                    })
                    .collect();
                let (k, generation) = (trustees.k, trustees.generation);
                if self.send_shares(&rk, generation, k, &people).await.is_ok() {
                    for p in trustees.people.iter_mut() {
                        if health_of(p, now) == Health::Unconfirmed {
                            p.sent_at = now;
                            p.resent_at = Some(now);
                            p.acked_at = None;
                            p.pinged_at = None;
                        }
                    }
                    changed = true;
                }
            }
        }
        if changed {
            trustees.updated_at = now;
            save(self.boxed.as_ref(), &trustees).await?;
            self.tell().await;
        }
        Ok(status_of(Some(&trustees), now))
    }

    /// How many people acked the generation out there.
    async fn acked(&self) -> Result<(u8, u8), CloudError> {
        Ok(match load(self.boxed.as_ref()).await? {
            Some(t) => (t.people.iter().filter(|p| p.acked_at.is_some()).count() as u8, t.k),
            None => (0, 0),
        })
    }
}

impl ShareDelivery for Social {
    /// Step 5 of a reissue: the shares of `generation` under `rk` to the
    /// trusted people in place, then the wait for `k` acks. Nothing without
    /// trusted people.
    fn deliver<'a>(&'a self, rk: &'a RecoveryKey, generation: u32) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let lane = LANE.lock().await;
            let Some(mut trustees) = load(self.boxed.as_ref()).await? else {
                return Ok(());
            };
            let people: Vec<ApprovedContact> = trustees
                .people
                .iter()
                .map(|p| ApprovedContact {
                    pubkey: p.pubkey.clone(),
                    name: p.name.clone(),
                })
                .collect();
            let before = trustees.clone();
            let now = self.now();
            trustees.generation = generation;
            for p in trustees.people.iter_mut() {
                p.sent_at = now;
                p.acked_at = None;
                p.pinged_at = None;
                p.ponged_at = None;
                p.resent_at = None;
            }
            trustees.updated_at = now;
            save(self.boxed.as_ref(), &trustees).await?;
            if let Err(e) = self.send_shares(rk, generation, trustees.k, &people).await {
                save(self.boxed.as_ref(), &before).await?;
                return Err(e);
            }
            drop(lane);
            self.tell().await;
            let deadline = tokio::time::Instant::now() + self.ack_patience;
            loop {
                let (acked, k) = self.acked().await?;
                if acked >= k {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(CloudError::new(
                        words::SHARES_NOT_ACKED,
                        format!("{acked} of {k} trusted people confirmed the new shares"),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
    }

    /// `recovery.share.revoke` of `generation` to everyone. A generation
    /// newer than the key in force is a reissue rolled back: the shares of
    /// the generation in force go out once more after the revoke, so that
    /// the people who replaced theirs hold a share again.
    fn revoke(&self, generation: u32) -> SeamFuture<'_, ()> {
        Box::pin(async move {
            let _lane = LANE.lock().await;
            let Some(mut trustees) = load(self.boxed.as_ref()).await? else {
                return Ok(());
            };
            for p in &trustees.people {
                self.send_revoke(&p.pubkey, generation).await;
            }
            let in_force = escrow::load_generation(self.boxed.as_ref()).await?;
            if generation > in_force && trustees.generation == generation {
                if let Some(rk) = escrow::load_rk(self.boxed.as_ref()).await? {
                    let people: Vec<ApprovedContact> = trustees
                        .people
                        .iter()
                        .map(|p| ApprovedContact {
                            pubkey: p.pubkey.clone(),
                            name: p.name.clone(),
                        })
                        .collect();
                    let k = trustees.k;
                    self.send_shares(&rk, in_force, k, &people).await?;
                    let now = self.now();
                    trustees.generation = in_force;
                    for p in trustees.people.iter_mut() {
                        p.sent_at = now;
                        p.acked_at = None;
                        p.pinged_at = None;
                        p.ponged_at = None;
                        p.resent_at = Some(now);
                    }
                    trustees.updated_at = now;
                    save(self.boxed.as_ref(), &trustees).await?;
                    self.tell().await;
                }
            }
            Ok(())
        })
    }
}

// ── The app ──────────────────────────────────────────────────────────────

fn stopped(what: &str) -> CloudError {
    CloudError::new(
        code::MESSENGER_STOPPED,
        format!("the messenger gives no {what} in this product"),
    )
}

/// The box `cloud` of the lock, through a seam in a test.
pub(crate) fn box_of<R: Runtime>(app: &AppHandle<R>) -> Arc<dyn LockBox> {
    crate::account::boxed(app)
}

/// The owner's side of the app: the seams of the messenger, the client of
/// the cloud, the box; the screens hear [`EVENT_TRUSTEES`].
pub async fn social_of<R: Runtime>(app: &AppHandle<R>) -> Result<Social, CloudError> {
    let notes = veydan_shell::seam::<dyn PeerNotes, _, _>(app).ok_or_else(|| stopped("notes"))?;
    let signer = veydan_shell::seam::<dyn Signer, _, _>(app).ok_or_else(|| stopped("signer"))?;
    let me = signer.pubkey().await?;
    let api = crate::client(app);
    let emitter = app.clone();
    Ok(Social::new(notes, box_of(app), api, me).on_change(Arc::new(move |status| {
        let _ = emitter.emit(EVENT_TRUSTEES, &status);
    })))
}

/// The approved contacts of the messenger, for the picker.
pub async fn approved_of<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<ApprovedContact>, CloudError> {
    let contacts =
        veydan_shell::seam::<dyn Contacts, _, _>(app).ok_or_else(|| stopped("address book"))?;
    contacts.approved().await
}

/// What «Доверенные люди» shows; an empty, unconfigured status without the
/// messenger's seams or while the lock is closed.
#[tauri::command]
pub async fn cloud_trustees_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<TrusteesStatus, CloudError> {
    social_of(&app).await?.status().await
}

/// The contacts the picker offers (§8.1: `full_chat`).
#[tauri::command]
pub async fn cloud_contacts_approved<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ApprovedContact>, CloudError> {
    approved_of(&app).await
}

/// «Доверенные люди» → the chosen `people` with the threshold `k`: the
/// shares go, the config is written, the escrow carries the numbers.
#[tauri::command]
pub async fn cloud_trustees_setup<R: Runtime>(
    k: u8,
    people: Vec<ApprovedContact>,
    app: AppHandle<R>,
) -> Result<TrusteesStatus, CloudError> {
    let approved = approved_of(&app).await?;
    let status = social_of(&app).await?.setup(k, people, &approved).await?;
    refresh_escrow_later(&app);
    Ok(status)
}

/// «Выключить» the trusted people: revokes, the config deleted, the set
/// forgotten; how many revokes went.
#[tauri::command]
pub async fn cloud_trustees_disable<R: Runtime>(app: AppHandle<R>) -> Result<u32, CloudError> {
    let revoked = social_of(&app).await?.disable().await?;
    refresh_escrow_later(&app);
    Ok(revoked)
}

/// «Проверить»: a ping to everyone now; the status as it stands.
#[tauri::command]
pub async fn cloud_trustees_check<R: Runtime>(
    app: AppHandle<R>,
) -> Result<TrusteesStatus, CloudError> {
    social_of(&app).await?.check(true).await
}

/// The escrow written again in the background: its `recovery` numbers
/// changed (§4).
fn refresh_escrow_later<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::recovery::refresh_escrow(&app).await {
            if !e.is(code::LOCKED) && !e.is(code::NO_HOST) {
                eprintln!("cloud: the escrow was not written after the trusted people changed: {e}");
            }
        }
    });
}

/// Hears the service notes of the module for the whole run of the app:
/// acks and pongs to this side, shares, pings and revokes to the trustee's
/// side, `recovery.cancel` to the cards. Nothing without the seam.
pub fn listen<R: Runtime>(app: AppHandle<R>) {
    let Some(notes) = veydan_shell::seam::<dyn PeerNotes, _, _>(&app) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let Ok(mut rx) = notes.incoming().await else {
            return;
        };
        while let Some(note) = rx.recv().await {
            if let Err(e) = dispatch(&app, &note).await {
                if !e.is(code::LOCKED) {
                    eprintln!("cloud: a note {} was not applied: {e}", note.t);
                }
            }
        }
    });
}

/// One note to the side it belongs to.
pub async fn dispatch<R: Runtime>(app: &AppHandle<R>, note: &Note) -> Result<bool, CloudError> {
    let Some(from) = note.from.as_deref() else {
        return Ok(false);
    };
    match note.t.as_str() {
        T_ACK => social_of(app).await?.on_ack(from, &note.fields).await,
        T_PONG => social_of(app).await?.on_pong(from, &note.fields).await,
        T_SHARE | T_PING | T_REVOKE => crate::trustee::side_of(app).await?.on_note(note).await,
        crate::cancel::T_CANCEL => crate::cancel::on_cancel_note(app, &note.fields).await,
        _ => Ok(false),
    }
}

/// The daily round of the health (§8.3), an hour after the start and then
/// once a day.
pub fn start_daily<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            match social_of(&app).await {
                Ok(social) => {
                    if let Err(e) = social.check(false).await {
                        if !e.is(code::LOCKED) {
                            eprintln!("cloud: the check of the trusted people: {e}");
                        }
                    }
                }
                Err(_) => continue,
            }
            tokio::time::sleep(Duration::from_secs(23 * 3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(acked: Option<u64>, pinged: Option<u64>, ponged: Option<u64>) -> Trustee {
        Trustee {
            pubkey: "ab".repeat(32),
            name: "Ann".into(),
            index: 1,
            sent_at: 1_000,
            acked_at: acked,
            pinged_at: pinged,
            ponged_at: ponged,
            resent_at: None,
        }
    }

    #[test]
    fn the_health_follows_the_acks_and_the_pongs() {
        let soon = 1_000 + 60;
        let later = 1_000 + ACK_WAIT_SECS;
        assert_eq!(health_of(&person(None, None, None), soon), Health::Waiting);
        assert_eq!(health_of(&person(None, None, None), later), Health::NotDelivered);
        assert_eq!(health_of(&person(Some(1_100), None, None), later), Health::Delivered);
        assert_eq!(health_of(&person(Some(1_100), Some(2_000), Some(2_500)), later), Health::Alive);
        assert_eq!(health_of(&person(Some(1_100), Some(2_000), Some(1_900)), 2_100), Health::Delivered);
        assert_eq!(
            health_of(&person(Some(1_100), Some(2_000), None), 2_000 + PONG_WAIT_SECS),
            Health::Unconfirmed
        );
        let status = status_of(None, 5);
        assert!(!status.configured && status.k == 3 && status.n == 5);
        let json = serde_json::to_value(Health::NotDelivered).expect("json");
        assert_eq!(json, "not_delivered");
    }

    #[test]
    fn the_status_counts_by_person() {
        let t = Trustees {
            k: 2,
            n: 3,
            generation: 1,
            people: vec![
                person(Some(1_100), Some(2_000), Some(2_500)),
                person(Some(1_100), Some(2_000), Some(2_600)),
                person(None, None, None),
            ],
            updated_at: 1_000,
        };
        let s = status_of(Some(&t), 3_000);
        assert_eq!((s.acked, s.alive, s.checked_at), (2, 2, Some(2_500)));
        assert_eq!(s.people[2].health, Health::Waiting);
        assert_eq!(unhex32(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(unhex32("AB"), None);
    }

    #[tokio::test]
    async fn without_the_seams_the_commands_say_the_messenger_is_stopped() {
        let app = tauri::test::mock_app();
        let e = cloud_trustees_status(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_contacts_approved(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_trustees_setup(2, Vec::new(), app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_trustees_disable(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let e = cloud_trustees_check(app.handle().clone()).await.err().map(|e| e.code);
        assert_eq!(e.as_deref(), Some(code::MESSENGER_STOPPED));
        let boxed = crate::fake::MemoryBox::default();
        let params = recovery_params(&boxed, 3).await.expect("params");
        assert_eq!((params.k, params.n, params.generation), (0, 0, 3));
    }
}

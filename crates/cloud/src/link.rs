// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Linking a device (internal/cloud-recovery.md §6, internal/vcloud-plan.md
//! 5.3, internal/messenger-wire.md §3): the nsec goes from the old device A
//! to the new device B over the relays of the project alone, in two wraps.
//!
//! - B ([`Linker::start`]): a temporary key T and a code of 8 words; the QR
//!   carries `veydan://link/<T.pub>/<words>`; for a device without a camera
//!   T.pub goes on the relays as kind 30078 under `d = sha256(code)`; B
//!   listens as T for ten minutes.
//! - A ([`Linker::resolve`], [`Linker::approve`]): the QR or the words give
//!   T.pub; both screens show the 4 words of T's fingerprint; after the
//!   secret of the lock the nsec goes inside `XChaCha20-Poly1305(Argon2id(
//!   code, sha256(T.pub)[..16]))` (crates/recovery) and outside in NIP-44
//!   to T (the messenger's `device.link`, expiring in ten minutes); once B
//!   says `device.link.done`, or after ten minutes, the wrap is deleted.
//! - B ([`Linker::receive`]): the inner wrap opens with the code, its npub
//!   is its nsec's and the sealer's; the identity is imported, my devices
//!   hear `device.link.done`, the code is deleted, T is wiped.
//!
//! The module never goes to the relays: the messenger does, through
//! `PeerNotes` and `DeviceLink`. The vault of the account ([`VaultEntry`]):
//! A sends the vault it is bound to along with the nsec — its id, its key
//! and where it lives, with the access to a storage of the user's own —
//! and B enters it in the order of §7 before the identity is imported
//! (storage set up as on A, vault joined, lock opened). A device bound to
//! no vault sends none, and a storage B cannot reach (a folder of another
//! computer, any folder on a phone) is `vault_unreachable`: the person may
//! go on without it, and the vault is joined from the settings of sync.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::mpsc;
use veydan_recovery::devicelink::{self, DeviceLinkError, LinkCode, LinkPayload, LINK_URI_PREFIX};
use veydan_recovery::{kdf, rk, KdfParams, SyncKeys};
use veydan_shell::seams::{code, CloudError, DeviceLink, Note, PeerNotes, SeamFuture, TempKey};
use zeroize::Zeroizing;

use crate::restore::Vault;

/// How long a code, T and the wrap with the nsec live (§6).
pub use veydan_shell::seams::LINK_TTL_SECS;
/// The type of the note with the nsec, to T.
pub const T_DEVICE_LINK: &str = "device.link";
/// The type of the note of my own devices that ends a link.
pub const T_DEVICE_LINK_DONE: &str = "device.link.done";

/// The words of the errors of a link, beside those of the seams.
pub mod link_code {
    /// No link is under way on this device, or not this side of one.
    pub const LINK_STATE: &str = "link_state";
    /// The ten minutes are over: «Код устарел».
    pub const LINK_EXPIRED: &str = "link_expired";
    /// No live code under these words on the relays.
    pub const LINK_NOT_FOUND: &str = "link_not_found";
    /// What was typed or scanned is not 8 words of the list nor a link.
    pub const LINK_CODE: &str = "link_code";
}

/// The vault of sync of a device, for a link: what A sends of it and how
/// B enters it. Offered through the shell by the module's setup.
pub trait VaultEntry: Send + Sync {
    /// A: the vault this device is bound to and where it lives; `None`
    /// without a binding.
    fn keys(&self) -> SeamFuture<'_, Option<SyncKeys>>;
    /// B: the order of cloud-recovery.md §7 before the identity is
    /// imported — the vault `sync` opened with its key where it lives
    /// (requests to Veydan Cloud signed by `nsec`, in memory), its key row
    /// adopted, the settings of sync set as on A, the lock opened with
    /// `password`. `vault_unreachable` when the storage is not within
    /// reach of this device.
    fn enter<'a>(
        &'a self,
        nsec: &'a Zeroizing<[u8; 32]>,
        sync: &'a SyncKeys,
        password: Option<&'a str>,
    ) -> SeamFuture<'a, ()>;
}

/// The clock of a link, unix seconds; a fake one in the tests.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn state_error(message: &str) -> CloudError {
    CloudError::new(link_code::LINK_STATE, message)
}

fn expired_error() -> CloudError {
    CloudError::new(link_code::LINK_EXPIRED, "the code is out of date")
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

fn code_error(e: DeviceLinkError) -> CloudError {
    let message = match e {
        DeviceLinkError::WordCount(n) => format!("a code is 8 words, not {n}"),
        DeviceLinkError::UnknownWord(i) => format!("word {} is not in the list", i + 1),
        other => other.to_string(),
    };
    CloudError::new(link_code::LINK_CODE, message)
}

fn fingerprint_of(t_pub: &[u8; 32]) -> Vec<String> {
    devicelink::fingerprint(t_pub)
        .iter()
        .map(|w| w.to_string())
        .collect()
}

/// What B shows: the QR, the words, the fingerprint, the end of the time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkStart {
    /// T, 64 hex.
    pub t_pub: String,
    /// The 8 words of the code, in order.
    pub words: Vec<String>,
    /// `veydan://link/<T.pub>/<words joined by ->`: the content of the QR.
    pub uri: String,
    /// The QR code of `uri` as SVG.
    pub qr_svg: String,
    /// The 4 words of the fingerprint of T.
    pub fingerprint: Vec<String>,
    /// When the code and T end, unix seconds.
    pub expires_at: u64,
}

/// Where the link of B is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// No link on this device.
    Idle,
    /// T listens; nothing came yet.
    Waiting,
    /// The nsec came and opened: the identity can be taken over.
    Received,
    /// Ten minutes passed with nothing: «Код устарел».
    Expired,
}

/// The answer of `cloud_link_poll`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkPoll {
    /// Where the link is.
    pub state: LinkState,
    /// When it ends, while there is one.
    pub expires_at: Option<u64>,
    /// The key that came, 64 hex, once [`LinkState::Received`].
    pub npub: Option<String>,
}

/// What A found behind the QR or the words.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkTarget {
    /// T, 64 hex.
    pub t_pub: String,
    /// The 4 words of the fingerprint of T, to compare with the new device.
    pub fingerprint: Vec<String>,
    /// Until when the nsec may go, unix seconds.
    pub expires_at: u64,
}

/// A sent the nsec to T.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkSent {
    /// T, 64 hex.
    pub t_pub: String,
    /// When the wrap expires on the relays, unix seconds.
    pub expires_at: u64,
}

/// B took the identity over.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LinkDone {
    /// The key of the identity now on this device, 64 hex.
    pub npub: String,
    /// Whether my other devices were told (`device.link.done`); without it
    /// A deletes its wrap after ten minutes.
    pub done_sent: bool,
    /// Whether B entered the vault A sent along, by the order of §7.
    pub vault_joined: bool,
}

/// The side of B while a link is under way.
struct NewSide {
    /// Which start this is: a listener of an earlier one changes nothing.
    round: u64,
    t: TempKey,
    t_pub: [u8; 32],
    code_event: Option<String>,
    expires_at: u64,
    received: Option<LinkPayload>,
}

/// The side of A once it found T.
struct OldSide {
    code: LinkCode,
    t_pub: [u8; 32],
    expires_at: u64,
    sent: bool,
}

/// The links of this device, kept by the module between the commands.
#[derive(Default)]
pub struct Sessions {
    new_side: Mutex<Option<NewSide>>,
    old_side: Mutex<Option<OldSide>>,
    rounds: Mutex<u64>,
    /// The end of the last link of B that expired with nothing: its T is
    /// wiped already, `poll` says `expired` until a new start. Locked after
    /// `new_side` when both are.
    expired: Mutex<Option<u64>>,
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A link of this device over the messenger's seams.
#[derive(Clone)]
pub struct Linker {
    notes: Arc<dyn PeerNotes>,
    link: Arc<dyn DeviceLink>,
    entry: Option<Arc<dyn VaultEntry>>,
    sessions: Arc<Sessions>,
    params: KdfParams,
    clock: Clock,
}

impl Linker {
    /// A linker with the production parameters of Argon2id and the clock of
    /// the system.
    pub fn new(
        notes: Arc<dyn PeerNotes>,
        link: Arc<dyn DeviceLink>,
        entry: Option<Arc<dyn VaultEntry>>,
        sessions: Arc<Sessions>,
    ) -> Self {
        Self {
            notes,
            link,
            entry,
            sessions,
            params: KdfParams::production(),
            clock: Arc::new(system_now),
        }
    }

    /// The same with other parameters and another clock: the tests.
    pub fn with(mut self, params: KdfParams, clock: Clock) -> Self {
        self.params = params;
        self.clock = clock;
        self
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    // ── B: the new device ───────────────────────────────────────────────

    /// A new T and code (an earlier link of B ends), T listening, the code
    /// on the relays. With no relay nothing is published and nothing kept.
    pub async fn start(&self) -> Result<LinkStart, CloudError> {
        self.cancel_new().await;
        let t = TempKey::generate()?;
        let t_pub =
            rk::npub_of(t.secret_bytes()).ok_or_else(|| crypto("T is not a key of secp256k1"))?;
        let code = LinkCode::generate().map_err(crypto)?;
        let expires_at = self.now() + LINK_TTL_SECS;
        let uri = devicelink::link_uri(&t_pub, &code);
        let qr_svg = self.link.qr_svg(&uri).await?;
        let listener = TempKey::new(Zeroizing::new(*t.secret_bytes()));
        let heard = self.notes.listen_as(listener).await?;
        // Without a camera A finds T by the code: published only once T listens.
        let code_event = self
            .link
            .publish_code(&t, &code.d_tag(), expires_at)
            .await?;
        let round = {
            let mut rounds = guard(&self.sessions.rounds);
            *rounds += 1;
            *rounds
        };
        *guard(&self.sessions.new_side) = Some(NewSide {
            round,
            t,
            t_pub,
            code_event: Some(code_event),
            expires_at,
            received: None,
        });
        tokio::spawn(
            self.clone()
                .hear(round, heard, code.clone(), t_pub, expires_at),
        );
        Ok(LinkStart {
            t_pub: hex(&t_pub),
            words: code.words().iter().map(|w| w.to_string()).collect(),
            uri,
            qr_svg,
            fingerprint: fingerprint_of(&t_pub),
            expires_at,
        })
    }

    /// The notes that come to T until one opens or the time is over; a
    /// blob that does not open with the code, or whose npub is not the
    /// sealer's, is dropped without a trace. When the time is over with
    /// nothing, T is wiped and the code deleted at once.
    async fn hear(
        self,
        round: u64,
        mut heard: mpsc::Receiver<Note>,
        code: LinkCode,
        t_pub: [u8; 32],
        expires_at: u64,
    ) {
        let left = Duration::from_secs(expires_at.saturating_sub(self.now()));
        let deadline = tokio::time::Instant::now() + left;
        loop {
            let note = match tokio::time::timeout_at(deadline, heard.recv()).await {
                Ok(Some(note)) => note,
                Ok(None) => return,
                Err(_) => {
                    drop(heard);
                    self.expire(Some(round)).await;
                    return;
                }
            };
            let Some(payload) = self.open_note(&note, &code, &t_pub).await else {
                continue;
            };
            let mut side = guard(&self.sessions.new_side);
            match side.as_mut() {
                Some(s) if s.round == round && self.now() < s.expires_at => {
                    s.received = Some(payload);
                    return;
                }
                _ => return,
            }
        }
    }

    async fn open_note(
        &self,
        note: &Note,
        code: &LinkCode,
        t_pub: &[u8; 32],
    ) -> Option<LinkPayload> {
        if note.t != T_DEVICE_LINK {
            return None;
        }
        let sealer = note.from.as_deref().and_then(unhex32)?;
        let blob = B64.decode(note.fields.get("blob")?.as_str()?).ok()?;
        let (code, t_pub, params) = (code.clone(), *t_pub, self.params);
        let opened = tokio::task::spawn_blocking(move || {
            devicelink::open_with(&code, &t_pub, &blob, &params)
        })
        .await
        .ok()?
        .ok()?;
        (opened.npub == sealer).then_some(opened)
    }

    /// Where the link of B is; a link whose ten minutes are over with
    /// nothing received ends here (T wiped, the code deleted).
    pub async fn poll(&self) -> LinkPoll {
        self.expire(None).await;
        let side = guard(&self.sessions.new_side);
        match side.as_ref() {
            None => {
                let expired = *guard(&self.sessions.expired);
                LinkPoll {
                    state: if expired.is_some() {
                        LinkState::Expired
                    } else {
                        LinkState::Idle
                    },
                    expires_at: expired,
                    npub: None,
                }
            }
            Some(s) => match &s.received {
                Some(p) => LinkPoll {
                    state: LinkState::Received,
                    expires_at: Some(s.expires_at),
                    npub: Some(hex(&p.npub)),
                },
                None => LinkPoll {
                    state: LinkState::Waiting,
                    expires_at: Some(s.expires_at),
                    npub: None,
                },
            },
        }
    }

    /// Ends the link of B whose time is over with nothing received: T is
    /// wiped at once and the code deleted as best it goes; `poll` says
    /// `expired` until a new start. `round` names the listener whose timer
    /// ran out; without it the clock decides.
    async fn expire(&self, round: Option<u64>) {
        let side = {
            let mut slot = guard(&self.sessions.new_side);
            let due = slot.as_ref().is_some_and(|s| {
                s.received.is_none()
                    && match round {
                        Some(r) => r == s.round,
                        None => self.now() >= s.expires_at,
                    }
            });
            if !due {
                return;
            }
            let side = slot.take();
            *guard(&self.sessions.expired) = side.as_ref().map(|s| s.expires_at);
            side
        };
        if let Some(side) = side {
            self.delete_code(side).await;
        }
    }

    /// The identity that came, taken over: after the vault A sent along is
    /// entered ([`VaultEntry`]), or alone when A sent none or the person
    /// goes on without it ([`Vault::Skip`]); then my devices hear
    /// `device.link.done`, the code is deleted and T wiped. A closed lock
    /// (`locked`), a wrong secret (`wrong_password`), a storage out of
    /// reach (`vault_unreachable`) and a device with a lock or a vault of
    /// its own (`not_clean`) keep the link for another try.
    pub async fn receive(
        &self,
        password: Option<&str>,
        vault: Vault,
    ) -> Result<LinkDone, CloudError> {
        let (payload, t_pub) = {
            let side = guard(&self.sessions.new_side);
            let s = side.as_ref().ok_or_else(|| {
                if guard(&self.sessions.expired).is_some() {
                    expired_error()
                } else {
                    state_error("no link is under way on this device")
                }
            })?;
            let p = s
                .received
                .clone()
                .ok_or_else(|| state_error("nothing came from the other device yet"))?;
            (p, s.t_pub)
        };
        let expected = hex(&payload.npub);
        let nsec = Zeroizing::new(payload.nsec);
        let vault_joined = match (&self.entry, &payload.sync, vault) {
            (Some(entry), Some(sync), Vault::Join) => {
                match entry.enter(&nsec, sync, password).await {
                    Ok(()) => true,
                    // A device that has an identity keeps its vault as it
                    // is; `take_over` says whether the identity is this one.
                    Err(e) if e.is(code::IDENTITY_EXISTS) => false,
                    Err(e) => return Err(e),
                }
            }
            _ => false,
        };
        drop(payload);
        let npub = self.link.take_over(nsec, password).await?;
        if npub != expected {
            return Err(crypto("the identity taken over is not the one that came"));
        }
        let mut fields = Map::new();
        fields.insert("t_pub".into(), Value::String(hex(&t_pub)));
        let done_sent = self
            .notes
            .send_own(Note::new(T_DEVICE_LINK_DONE, fields))
            .await
            .is_ok();
        let side = guard(&self.sessions.new_side).take();
        if let Some(side) = side {
            self.delete_code(side).await;
        }
        Ok(LinkDone {
            npub,
            done_sent,
            vault_joined,
        })
    }

    /// NIP-09 of the code of B, as best it goes; T is wiped with the side.
    async fn delete_code(&self, side: NewSide) {
        if let Some(id) = &side.code_event {
            // The expiration of the event is the cleaning when this fails.
            let _ = self.link.delete_code(&side.t, id).await;
        }
    }

    async fn cancel_new(&self) {
        let side = {
            let mut slot = guard(&self.sessions.new_side);
            *guard(&self.sessions.expired) = None;
            slot.take()
        };
        if let Some(side) = side {
            self.delete_code(side).await;
        }
    }

    // ── A: the old device ───────────────────────────────────────────────

    /// T behind a scanned link or 8 typed words (then the kind 30078 under
    /// their `d`), and the fingerprint to compare. The time to send ends
    /// with the code's own `expiration` on the relays when it is sooner
    /// than ten minutes from now.
    pub async fn resolve(&self, input: &str) -> Result<LinkTarget, CloudError> {
        let input = Zeroizing::new(input.trim().to_string());
        let n = LINK_URI_PREFIX.len();
        let (t_pub, code, code_end) = match (input.get(..n), input.get(n..)) {
            (Some(head), Some(rest)) if head.eq_ignore_ascii_case(LINK_URI_PREFIX) => {
                // The scheme may come in any case; the rest is the parser's.
                let uri = Zeroizing::new(format!("{LINK_URI_PREFIX}{rest}"));
                let (t_pub, code) = devicelink::parse_link_uri(&uri).map_err(code_error)?;
                (t_pub, code, None)
            }
            _ => {
                let code = LinkCode::parse(&input).map_err(code_error)?;
                let found = self.link.find_code(&code.d_tag()).await?;
                let not_found =
                    || CloudError::new(link_code::LINK_NOT_FOUND, "no live code under these words");
                let found = found.ok_or_else(not_found)?;
                let t_pub = unhex32(&found.t_pub).ok_or_else(not_found)?;
                (t_pub, code, found.expires_at)
            }
        };
        let now = self.now();
        let expires_at = code_end.map_or(now + LINK_TTL_SECS, |end| end.min(now + LINK_TTL_SECS));
        if expires_at <= now {
            return Err(expired_error());
        }
        *guard(&self.sessions.old_side) = Some(OldSide {
            code,
            t_pub,
            expires_at,
            sent: false,
        });
        Ok(LinkTarget {
            t_pub: hex(&t_pub),
            fingerprint: fingerprint_of(&t_pub),
            expires_at,
        })
    }

    /// The fingerprints matched («Совпадает»): after the secret of the lock
    /// the nsec goes to T in its two wraps; the wrap is deleted once my new
    /// device says it is in, or after ten minutes.
    pub async fn approve(&self, password: Option<&str>) -> Result<LinkSent, CloudError> {
        let (code, t_pub, expires_at) = {
            let side = guard(&self.sessions.old_side);
            let s = side
                .as_ref()
                .ok_or_else(|| state_error("no other device was found yet"))?;
            if s.sent {
                return Err(state_error("the identity went to this device already"));
            }
            if self.now() >= s.expires_at {
                return Err(expired_error());
            }
            (s.code.clone(), s.t_pub, s.expires_at)
        };
        let nsec = self.link.hand_over(password).await?;
        let npub =
            rk::npub_of(&nsec).ok_or_else(|| crypto("the identity is not a key of secp256k1"))?;
        let sync = match &self.entry {
            Some(entry) => entry.keys().await?,
            None => None,
        };
        let payload = LinkPayload {
            npub,
            nsec: *nsec,
            created_at: self.now(),
            sync,
        };
        drop(nsec);
        let nonce = kdf::random_array().map_err(crypto)?;
        let params = self.params;
        let blob = tokio::task::spawn_blocking(move || {
            devicelink::seal_with(&code, &t_pub, &payload, &params, nonce)
        })
        .await
        .map_err(crypto)?
        .map_err(crypto)?;
        let mut fields = Map::new();
        fields.insert("blob".into(), Value::String(B64.encode(&blob)));
        let mut note = Note::new(T_DEVICE_LINK, fields);
        note.expires_at = Some(expires_at.min(self.now() + LINK_TTL_SECS));
        let to = hex(&t_pub);
        self.notes.send_to_key(note, &to).await?;
        if let Some(s) = guard(&self.sessions.old_side).as_mut() {
            s.sent = true;
        }
        let link = self.link.clone();
        let until = expires_at;
        let t_hex = to.clone();
        tokio::spawn(async move {
            // Done or not, the wrap with the nsec goes when it served.
            let _ = link.await_done(&t_hex, until).await;
            let _ = link.withdraw(&t_hex).await;
        });
        Ok(LinkSent {
            t_pub: to,
            expires_at,
        })
    }

    /// Ends the link of this device on either side: B deletes its code and
    /// wipes T, A forgets T and deletes a wrap it sent.
    pub async fn cancel(&self) -> Result<(), CloudError> {
        self.cancel_new().await;
        let old = guard(&self.sessions.old_side).take();
        if let Some(old) = old {
            if old.sent {
                self.link.withdraw(&hex(&old.t_pub)).await?;
            }
        }
        Ok(())
    }
}

// ── Commands ─────────────────────────────────────────────────────────────

/// The linker of the app: the messenger's seams, the module's sessions.
fn linker<R: Runtime>(app: &AppHandle<R>) -> Result<Linker, CloudError> {
    let stopped = || {
        CloudError::new(
            code::MESSENGER_STOPPED,
            "the messenger does not link devices in this product",
        )
    };
    let notes = veydan_shell::seam::<dyn PeerNotes, _, _>(app).ok_or_else(stopped)?;
    let link = veydan_shell::seam::<dyn DeviceLink, _, _>(app).ok_or_else(stopped)?;
    let entry = veydan_shell::seam::<dyn VaultEntry, _, _>(app);
    let sessions = app
        .try_state::<Arc<Sessions>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| state_error("the module cloud is not set up"))?;
    Ok(Linker::new(notes, link, entry, sessions))
}

/// B: a new T and code, the QR, the words and the fingerprint.
#[tauri::command]
pub async fn cloud_link_start<R: Runtime>(app: AppHandle<R>) -> Result<LinkStart, CloudError> {
    linker(&app)?.start().await
}

/// B: where the link is.
#[tauri::command]
pub async fn cloud_link_poll<R: Runtime>(app: AppHandle<R>) -> Result<LinkPoll, CloudError> {
    Ok(linker(&app)?.poll().await)
}

/// B: the identity that came, taken over, with the vault A sent along;
/// `without_vault` is the person's answer to `vault_unreachable`.
#[tauri::command]
pub async fn cloud_link_receive<R: Runtime>(
    app: AppHandle<R>,
    password: Option<String>,
    without_vault: Option<bool>,
) -> Result<LinkDone, CloudError> {
    let password = password.map(Zeroizing::new);
    linker(&app)?
        .receive(
            password.as_deref().map(String::as_str),
            Vault::of(without_vault),
        )
        .await
}

/// A: T behind a scanned link or 8 typed words.
#[tauri::command]
pub async fn cloud_link_resolve<R: Runtime>(
    app: AppHandle<R>,
    input: String,
) -> Result<LinkTarget, CloudError> {
    let input = Zeroizing::new(input);
    linker(&app)?.resolve(&input).await
}

/// A: the fingerprints matched; the nsec goes after the secret of the lock.
#[tauri::command]
pub async fn cloud_link_approve<R: Runtime>(
    app: AppHandle<R>,
    password: Option<String>,
) -> Result<LinkSent, CloudError> {
    let password = password.map(Zeroizing::new);
    linker(&app)?
        .approve(password.as_deref().map(String::as_str))
        .await
}

/// Either side: the link ends («Не совпадает», «Отмена»).
#[tauri::command]
pub async fn cloud_link_cancel<R: Runtime>(app: AppHandle<R>) -> Result<(), CloudError> {
    linker(&app)?.cancel().await
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

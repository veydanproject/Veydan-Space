// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The seams the messenger offers the module `cloud` through the shell
//! (internal/vcloud-plan.md 2.2, services/cloud/spec/cloud-api.md §12):
//!
//! - [`IdentitySigner`]: NIP-98 and NIP-44 with the key of the identity,
//!   read from the lock for one operation and never handed out;
//! - [`ServiceNotes`]: the service notes of messenger-wire.md §3 (`PeerNotes`,
//!   with `send_to_key` and `listen_as` of a key T); what it checks is
//!   here, the wraps and the outbox are the runtime's;
//! - [`PinnedCards`]: the cards of the vcloud bot, taken only from the
//!   pinned key and given to whoever listens;
//! - [`LinkWire`] (crate::cloud_link): `DeviceLink`, the linking of a
//!   device over the relays of the project (cloud-recovery.md §6);
//! - [`ProjectHttp`]: requests to `/cloud/` and `/vault/` of the hosts of
//!   the project, by the messenger's HTTPS client and its bridges;
//! - [`ManifestVaultHosts`]: `VaultHosts`, the storage hosts of the
//!   manifest in use.
//!
//! The module `cloud` names none of these types: it takes the traits of
//! `veydan_shell::seams` from the state of the app.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use messenger_core::MessengerError;
use nostr::nips::nip44;
use nostr::prelude::{EventBuilder, FinalizeEvent, Keys, Kind, PublicKey, Tag, Timestamp};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use veydan_shell::seams::{
    self, code, BotCards, Card, CloudError, Note, PeerNotes, SeamFuture, Signer, TempKey,
    WireEvent, OWN_TYPES, PEER_TYPES, TO_KEY_TYPES,
};
use veydan_shell::seams::{CloudReach, Http, HttpRequest, HttpResponse};
use veydan_shell::seams::{StorageHost, VaultHosts, DERIVE_LABELS, DERIVE_SALT};
use zeroize::Zeroizing;

use crate::cloud_link::LinkWire;
use crate::commands::MessengerState;
use veydan_shell::seams::DeviceLink;

/// The pinned key of the vcloud bot (messenger-wire.md §3, the cards;
/// cloud-recovery.md §10), built into a release and changed only with one.
/// The key made on the test vcloud on 2026-10-10 (`vcloud bot new`,
/// npub1hkk7d6av96y6wcxyww0c9wfczxeh0w3yv9vjmrwt4ygnd6ge22lsdy7sse); a
/// production vcloud brings its own with a release.
pub const VCLOUD_BOT_PUBKEY: Option<&str> =
    Some("bdade6ebac2e89a760c4739f82b93811b377ba2461592d8dcba91136e91952bf");

/// How many cards wait for a slow listener, and how many wait for the
/// first listener, before the oldest of them are dropped.
const CARDS_QUEUE: usize = 32;

/// How many ids of cards given out, and of cards dismissed, are kept in
/// memory; the oldest are forgotten first.
const CARDS_REMEMBERED: usize = 1024;

/// The longest address a NIP-98 event is signed for.
const MAX_URL: usize = 4096;

/// Offers the six seams to the shell (Signer, PeerNotes, BotCards,
/// DeviceLink, Http, VaultHosts). Called by the setup of the module.
pub(crate) fn offer(app: &AppHandle) {
    let signer: Arc<dyn Signer> = Arc::new(IdentitySigner { app: app.clone() });
    let wire = Arc::new(LinkWire::new(app.clone()));
    let notes: Arc<dyn PeerNotes> = Arc::new(ServiceNotes::in_app(wire.clone(), app.clone()));
    let contacts: Arc<dyn seams::Contacts> = Arc::new(crate::cloud_notes::ApprovedContacts { app: app.clone() });
    seams::provide(app, contacts);
    // Kept as its own state too: the forwarder of cloud_notes feeds it the
    // cards of the bot the runtime hands up as `cloud.card`.
    let pinned = Arc::new(PinnedCards::new(VCLOUD_BOT_PUBKEY));
    app.manage(pinned.clone());
    let cards: Arc<dyn BotCards> = pinned;
    seams::provide(app, signer);
    seams::provide(app, notes);
    seams::provide(app, cards);
    let link: Arc<dyn DeviceLink> = wire;
    seams::provide(app, link);
    let http: Arc<dyn Http> = Arc::new(ProjectHttp::new(app.clone()));
    seams::provide(app, http);
    let vault: Arc<dyn VaultHosts> = Arc::new(ManifestVaultHosts { app: app.clone() });
    seams::provide(app, vault);
}

/// Unix seconds; a clock before 1970 is refused here rather than signed as
/// 0 and turned away by the server as `auth_expired`.
fn now() -> Result<u64, CloudError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| CloudError::new(code::CRYPTO, "the clock of the device is before 1970"))
}

fn bad_request(message: impl Into<String>) -> CloudError {
    CloudError::new(code::BAD_REQUEST, message)
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// What the runtime said, in the words of the seams.
fn from_messenger(e: MessengerError) -> CloudError {
    match e {
        MessengerError::NotLoggedIn => {
            CloudError::new(code::NO_IDENTITY, "no identity on this device")
        }
        MessengerError::SecretsLocked => CloudError::new(code::LOCKED, "the lock is closed"),
        other => CloudError::new(code::CRYPTO, other.to_string()),
    }
}

/// A nostr key as the contract writes it: 64 hex, lower case, x-only.
fn key(hex: &str) -> Result<PublicKey, CloudError> {
    let lower = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !lower {
        return Err(bad_request("a key is 64 hex characters in lower case"));
    }
    PublicKey::from_hex(hex).map_err(|_| bad_request("not a key of secp256k1"))
}

/// The event of NIP-98 for one request (cloud-api.md 2.3): kind 27235 at
/// `created_at`, `u` exactly as given (the server compares it character
/// by character), `method` in capitals, `payload` the sha256 of a body,
/// and a `salt` of 16 random hex characters: the server remembers the ids
/// of the events it took, so two requests of one URL within one second
/// (the page refreshed and read again) must be two events, not a replay
/// of one. The server reads no such tag.
pub fn nip98_event(
    keys: &Keys,
    method: &str,
    url: &str,
    payload_sha256: Option<[u8; 32]>,
    created_at: u64,
) -> Result<WireEvent, CloudError> {
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(bad_request("the method of a request is a word of letters"));
    }
    let scheme = url.starts_with("https://") || url.starts_with("http://");
    if !scheme || url.len() > MAX_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad_request("the address of a request is an http(s) URL"));
    }
    let salt = Keys::generate().secret_key().to_secret_hex()[..16].to_string();
    let mut tags = vec![
        Tag::parse(["u", url]).map_err(crypto)?,
        Tag::parse(["method", &method.to_ascii_uppercase()]).map_err(crypto)?,
        Tag::parse(["salt", salt.as_str()]).map_err(crypto)?,
    ];
    if let Some(hash) = payload_sha256 {
        tags.push(Tag::parse(["payload", &hex(&hash)]).map_err(crypto)?);
    }
    let event = EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .finalize(keys)
        .map_err(crypto)?;
    serde_json::from_value(serde_json::to_value(&event).map_err(crypto)?).map_err(crypto)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// NIP-44 v2 to `to`: the base64 payload as bytes.
pub fn nip44_encrypt(keys: &Keys, to: &str, plaintext: &[u8]) -> Result<Vec<u8>, CloudError> {
    let to = key(to)?;
    nip44::encrypt(keys.secret_key(), &to, plaintext, nip44::Version::V2)
        .map(String::into_bytes)
        .map_err(crypto)
}

/// The inverse of [`nip44_encrypt`]; the plaintext is wiped when dropped.
pub fn nip44_decrypt(
    keys: &Keys,
    from: &str,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CloudError> {
    let from = key(from)?;
    nip44::decrypt_to_bytes(keys.secret_key(), &from, ciphertext)
        .map(Zeroizing::new)
        .map_err(crypto)
}

/// [`Signer`] over the identity of the running messenger.
pub struct IdentitySigner {
    app: AppHandle,
}

impl IdentitySigner {
    fn runtime(&self) -> Result<Arc<messenger_runtime::MessengerRuntime>, CloudError> {
        self.app
            .state::<MessengerState>()
            .runtime()
            .map_err(|e| CloudError::new(code::MESSENGER_STOPPED, e.to_string()))
    }

    /// The keys of the identity for one operation: they leave memory with it.
    async fn keys(&self) -> Result<Keys, CloudError> {
        self.runtime()?
            .identity()
            .load_keys()
            .await
            .map_err(from_messenger)
    }
}

impl Signer for IdentitySigner {
    fn pubkey(&self) -> SeamFuture<'_, String> {
        Box::pin(async move {
            let identity = self
                .runtime()?
                .identity()
                .get()
                .await
                .map_err(from_messenger)?;
            identity
                .map(|id| id.pubkey.as_hex().to_string())
                .ok_or_else(|| from_messenger(MessengerError::NotLoggedIn))
        })
    }

    fn nip98<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(async move {
            let at = now()?;
            nip98_event(&self.keys().await?, method, url, payload_sha256, at)
        })
    }

    fn nip44_encrypt<'a>(&'a self, to: &'a str, plaintext: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
        Box::pin(async move { nip44_encrypt(&self.keys().await?, to, plaintext) })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        from: &'a str,
        ciphertext: &'a [u8],
    ) -> SeamFuture<'a, Zeroizing<Vec<u8>>> {
        Box::pin(async move { nip44_decrypt(&self.keys().await?, from, ciphertext) })
    }

    fn nip98_at<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SeamFuture<'a, WireEvent> {
        Box::pin(async move {
            nip98_event(&self.keys().await?, method, url, payload_sha256, created_at)
        })
    }

    fn derive_secret<'a>(&'a self, label: &'a str) -> SeamFuture<'a, Zeroizing<[u8; 32]>> {
        Box::pin(async move { derive_secret(&self.keys().await?, label) })
    }

    /// The nsec for the escrow (cloud-recovery.md §4): read from the box of
    /// the lock for this one call, `locked` while the lock is closed.
    fn export_for_escrow(&self) -> SeamFuture<'_, Zeroizing<[u8; 32]>> {
        Box::pin(async move {
            let keys = self.keys().await?;
            Ok(Zeroizing::new(keys.secret_key().to_secret_bytes()))
        })
    }

    /// Step 6 of a restore (cloud-recovery.md §7): the nsec goes into the
    /// box of the lock, the messenger starts signing with it.
    fn import_for_restore<'a>(&'a self, nsec: &'a [u8; 32]) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let rt = self.runtime()?;
            let secret = Zeroizing::new(hex(nsec));
            rt.identity()
                .import_nsec(&secret)
                .await
                .map_err(|e| match e {
                    MessengerError::Invalid(m) => bad_request(m),
                    other => from_messenger(other),
                })?;
            rt.refresh_signer().await.map_err(from_messenger)?;
            // An account restored from Veydan Cloud lives on the project's
            // servers: a clean install that has not chosen yet is put there,
            // so the chats and the module work without the second step of
            // the onboarding. The manifest check needs the network; a
            // failure there is not a failure of the restore.
            if rt.relays().servers_mode().await.map_err(from_messenger)?.is_none() {
                if let Err(e) = rt.servers_use_veydan().await {
                    eprintln!("cloud restore: the servers of the project were not applied: {e}");
                }
            }
            Ok(())
        })
    }
}

/// HKDF-SHA256 of the nsec, salt [`DERIVE_SALT`], info `label` of
/// [`DERIVE_LABELS`] (cloud-recovery.md §3); the nsec stays in `keys`.
pub fn derive_secret(keys: &Keys, label: &str) -> Result<Zeroizing<[u8; 32]>, CloudError> {
    if !DERIVE_LABELS.contains(&label) {
        return Err(bad_request(format!("no secret is derived for {label:?}")));
    }
    let hk = hkdf::Hkdf::<sha2::Sha256>::new(
        Some(DERIVE_SALT.as_bytes()),
        keys.secret_key().as_secret_bytes(),
    );
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(label.as_bytes(), out.as_mut_slice())
        .map_err(|_| CloudError::new(code::CRYPTO, "the derivation failed"))?;
    Ok(out)
}

/// [`VaultHosts`] over the manifest in use: `storage[]` for the region of
/// the device, and none while the servers are not the project's (plan 4.5).
pub struct ManifestVaultHosts {
    app: AppHandle,
}

impl VaultHosts for ManifestVaultHosts {
    fn storage_hosts(&self) -> SeamFuture<'_, Vec<StorageHost>> {
        Box::pin(async move {
            let runtime = self
                .app
                .state::<MessengerState>()
                .runtime()
                .map_err(|e| CloudError::new(code::MESSENGER_STOPPED, e.to_string()))?;
            let relays = runtime.relays();
            // The user's own servers: no host of the project. Servers not
            // chosen yet (a clean install restoring by the Recovery Key,
            // cloud-recovery.md §7, plan 4.4): the manifest in use, the
            // embedded one at least.
            let mode = relays.servers_mode().await.map_err(from_messenger)?;
            if mode == Some(messenger_runtime::ServersMode::Own) {
                return Ok(Vec::new());
            }
            let (manifest, _) = relays.current_manifest().await.map_err(from_messenger)?;
            let region = relays.region().await.map_err(from_messenger)?;
            Ok(manifest
                .storage_for_region(&region)
                .into_iter()
                .map(|s| StorageHost {
                    url: s.url.clone(),
                    regions: s.regions.clone(),
                    priority: s.priority,
                })
                .collect())
        })
    }

    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        Box::pin(async move {
            // Asked again for every request: a switch to the user's own
            // servers stops the next one.
            let hosts: Vec<String> = self
                .storage_hosts()
                .await?
                .into_iter()
                .map(|h| h.url)
                .collect();
            let host = vault_target(&request.url, &hosts)?;
            send_limited(&request, &host, VAULT_LIMITS).await
        })
    }
}

/// The requests of `/vault/`: the timeouts of the client of sync
/// (spec/vstore.md §14) and an answer up to the largest object, 64 MiB.
const VAULT_LIMITS: Limits = Limits {
    connect: Duration::from_secs(15),
    total: Duration::from_secs(90),
    max_answer: (64 << 20) + (64 << 10),
};

/// The host of a request to one of the vault `hosts` (`https://…/vault`),
/// for the choice of the way; `no_host` for any other URL.
pub fn vault_target(url: &str, hosts: &[String]) -> Result<String, CloudError> {
    let ours = hosts.iter().find(|h| {
        h.starts_with("https://")
            && url
                .strip_prefix(h.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    });
    let Some(base) = ours else {
        return Err(CloudError::new(
            code::NO_HOST,
            "the request does not go to a host of the vault in use",
        ));
    };
    let authority = base
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or_default();
    let host = authority.split(':').next().unwrap_or(authority);
    Ok(host.to_ascii_lowercase())
}

/// Whether a note may go to `pubkey` past the rule of contacts: only the
/// types of [`TO_KEY_TYPES`], to a well-formed key.
pub fn check_to_key(note: &Note, pubkey: &str) -> Result<(), CloudError> {
    key(pubkey)?;
    if !TO_KEY_TYPES.contains(&note.t.as_str()) {
        return Err(bad_request(format!(
            "a note of type {:?} goes to an approved contact only",
            note.t
        )));
    }
    Ok(())
}

/// Whether a note may go to an approved contact: only the types of
/// [`PEER_TYPES`], to a well-formed key. `recovery.answer` and
/// `device.link` go to a key T alone ([`check_to_key`]), the notes to my
/// own devices by [`check_own`].
pub fn check_to_peer(note: &Note, peer: &str) -> Result<(), CloudError> {
    key(peer)?;
    if TO_KEY_TYPES.contains(&note.t.as_str()) {
        return Err(bad_request(format!(
            "a note of type {:?} goes to a temporary key only",
            note.t
        )));
    }
    if !PEER_TYPES.contains(&note.t.as_str()) {
        return Err(bad_request(format!(
            "{:?} is not a note to a contact of the cloud",
            note.t
        )));
    }
    Ok(())
}

/// Whether a note may go to my own devices: only the types of [`OWN_TYPES`].
pub fn check_own(note: &Note) -> Result<(), CloudError> {
    if !OWN_TYPES.contains(&note.t.as_str()) {
        return Err(bad_request(format!(
            "{:?} is not a note to my own devices",
            note.t
        )));
    }
    Ok(())
}

/// [`PeerNotes`] of the messenger. It checks what the contract asks of a
/// note before it goes; the wrap of a note (`wrap_note` with `Wake::Nobody`
/// and no copy to myself, `wrap_own` for my devices) and its place in the
/// outbox belong to the runtime. The notes to a key T, listening as T and
/// `device.link.done` go through [`LinkWire`] (stage C4); a note to a
/// contact and `recovery.cancel` are refused with `not_supported` until
/// the runtime gives the adapter a way to send them.
pub struct ServiceNotes {
    wire: Arc<LinkWire>,
    /// The app whose runtime sends to a contact and hears from one
    /// (`cloud_notes`); none in a test without an app.
    app: Option<AppHandle>,
}

impl ServiceNotes {
    /// The notes of the messenger whose side of a link is `wire`.
    pub fn new(wire: Arc<LinkWire>) -> Self {
        Self { wire, app: None }
    }

    /// [`ServiceNotes::new`] in the app whose runtime carries the notes to
    /// and from the contacts.
    pub fn in_app(wire: Arc<LinkWire>, app: AppHandle) -> Self {
        Self { wire, app: Some(app) }
    }
}

fn not_yet<'a, T: Send + 'a>(what: &'static str) -> SeamFuture<'a, T> {
    Box::pin(async move {
        Err(CloudError::new(
            code::NOT_SUPPORTED,
            format!("the messenger does not {what} in this version"),
        ))
    })
}

fn refused<'a, T: Send + 'a>(e: CloudError) -> SeamFuture<'a, T> {
    Box::pin(async move { Err(e) })
}

impl PeerNotes for ServiceNotes {
    fn send<'a>(&'a self, note: Note, peer: &'a str) -> SeamFuture<'a, ()> {
        match (check_to_peer(&note, peer), &self.app) {
            (Ok(()), Some(app)) => Box::pin(async move { crate::cloud_notes::send(app, &note, peer).await }),
            (Ok(()), None) => not_yet("send service notes"),
            (Err(e), _) => refused(e),
        }
    }

    fn incoming(&self) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        match &self.app {
            Some(app) => {
                let inbox = crate::cloud_notes::CloudInbox::of(app);
                Box::pin(async move { Ok(inbox.listen()) })
            }
            None => not_yet("hear service notes"),
        }
    }

    fn send_own(&self, note: Note) -> SeamFuture<'_, ()> {
        match check_own(&note) {
            Ok(()) => Box::pin(async move { self.wire.send_own(&note).await }),
            Err(e) => refused(e),
        }
    }

    fn send_to_key<'a>(&'a self, note: Note, pubkey: &'a str) -> SeamFuture<'a, ()> {
        match check_to_key(&note, pubkey) {
            Ok(()) => Box::pin(async move { self.wire.send_to_key(&note, pubkey).await }),
            Err(e) => refused(e),
        }
    }

    fn listen_as(&self, t: TempKey) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        Box::pin(async move { self.wire.listen_as(t).await })
    }
}

/// Ids kept in the order they came, at most [`CARDS_REMEMBERED`].
#[derive(Default)]
struct Remembered {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl Remembered {
    fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    fn insert(&mut self, id: &str) {
        if !self.ids.insert(id.to_string()) {
            return;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > CARDS_REMEMBERED {
            if let Some(oldest) = self.order.pop_front() {
                self.ids.remove(&oldest);
            }
        }
    }
}

#[derive(Default)]
struct CardsState {
    listeners: Vec<mpsc::Sender<Card>>,
    /// Cards no listener took yet: given to the first that comes.
    pending: VecDeque<Card>,
    /// Cards a listener took: not given again.
    given: Remembered,
    /// Cards handled: never given again in this run.
    dismissed: Remembered,
}

/// [`BotCards`]: the cards whose seal the pinned key signed, to every
/// listener, each card once and never one that was dismissed. A card that
/// comes before any listener (the ingress runs before the module `cloud`
/// listens) waits for the first, at most [`CARDS_QUEUE`] of them. What was
/// dismissed is remembered in memory only; keeping it across restarts is
/// the messenger's settings table's, with the cards themselves (stage C5).
pub struct PinnedCards {
    pin: Option<String>,
    state: Mutex<CardsState>,
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PinnedCards {
    pub fn new(pin: Option<&str>) -> Self {
        Self {
            pin: pin.map(str::to_string),
            state: Mutex::new(CardsState::default()),
        }
    }

    /// A card the ingress of the messenger found, sealed by `signer`. Taken
    /// when the signer is the pin and the card is new: it goes to every
    /// listener, and a listener that is gone is forgotten; when no listener
    /// took it, it waits for the next. Whether it was taken.
    pub fn offer(&self, signer: &str, card: Card) -> bool {
        if self.pin.as_deref() != Some(signer) {
            return false;
        }
        let mut state = guard(&self.state);
        let known = state.given.contains(&card.id)
            || state.dismissed.contains(&card.id)
            || state.pending.iter().any(|c| c.id == card.id);
        if known {
            return false;
        }
        let mut taken = false;
        state
            .listeners
            .retain(|listener| match listener.try_send(card.clone()) {
                Ok(()) => {
                    taken = true;
                    true
                }
                Err(mpsc::error::TrySendError::Full(_)) => true,
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            });
        if taken {
            state.given.insert(&card.id);
        } else {
            state.pending.push_back(card);
            if state.pending.len() > CARDS_QUEUE {
                state.pending.pop_front();
            }
        }
        true
    }
}

impl BotCards for PinnedCards {
    fn cards(&self) -> SeamFuture<'_, mpsc::Receiver<Card>> {
        let (tx, rx) = mpsc::channel(CARDS_QUEUE);
        let mut state = guard(&self.state);
        // The queue holds as many as wait, so every waiting card fits.
        while let Some(card) = state.pending.pop_front() {
            let id = card.id.clone();
            if let Err(e) = tx.try_send(card) {
                state.pending.push_front(e.into_inner());
                break;
            }
            state.given.insert(&id);
        }
        state.listeners.push(tx);
        Box::pin(async move { Ok(rx) })
    }

    fn dismiss<'a>(&'a self, id: &'a str) -> SeamFuture<'a, ()> {
        let mut state = guard(&self.state);
        state.dismissed.insert(id);
        state.pending.retain(|c| c.id != id);
        Box::pin(async { Ok(()) })
    }
}

/// The longest answer [`ProjectHttp`] reads: the answers of `/cloud/v1/`
/// are a few hundred bytes (cloud-api.md 1), a page of usage a few KiB.
const MAX_ANSWER: usize = 1 << 20;
const HTTP_CONNECT: Duration = Duration::from_secs(10);
const HTTP_TOTAL: Duration = Duration::from_secs(30);

fn transport(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::TRANSPORT, e.to_string())
}

/// `https://host[:port]` of a URL of a server, without its path; `None`
/// for anything but https with a plain host.
fn https_base(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let plain = !authority.is_empty()
        && authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'));
    plain.then(|| format!("https://{}", authority.to_ascii_lowercase()))
}

/// The bases of `/cloud/` among the media servers of the manifest, given
/// as `(type, url)`: nginx of the project's media hosts serves `location
/// /cloud/` and `/vault/` beside blossom (cloud-api.md 1, `public_host =
/// "eu-1-media…"`). Blossom over https only, in the manifest's order, each
/// once; a section of its own in the manifest replaces this list later.
pub fn cloud_bases<'a>(media: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut bases: Vec<String> = Vec::new();
    for (kind, url) in media {
        if kind != "blossom" {
            continue;
        }
        if let Some(base) = https_base(url) {
            if !bases.contains(&base) {
                bases.push(base);
            }
        }
    }
    bases
}

/// The host of a request when it goes to one of `bases`, for the choice of
/// the way (`messenger_http::following_route`); `no_host` otherwise.
pub fn target_host(url: &str, bases: &[String]) -> Result<String, CloudError> {
    let ours = bases.iter().find(|base| {
        url.strip_prefix(base.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    });
    let Some(base) = ours else {
        return Err(CloudError::new(
            code::NO_HOST,
            "the request does not go to a host of the project in use",
        ));
    };
    let authority = base.trim_start_matches("https://");
    Ok(authority.split(':').next().unwrap_or(authority).to_string())
}

/// One request by the messenger's HTTPS client, made again on the new way
/// when the way to `host` changes under it (bridges turned on or off).
pub async fn send_request(request: &HttpRequest, host: &str) -> Result<HttpResponse, CloudError> {
    let limits = Limits {
        connect: HTTP_CONNECT,
        total: HTTP_TOTAL,
        max_answer: MAX_ANSWER,
    };
    send_limited(request, host, limits).await
}

/// The timeouts and the longest answer of one request.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The connect timeout.
    pub connect: Duration,
    /// The timeout of the whole request.
    pub total: Duration,
    /// The longest body read.
    pub max_answer: usize,
}

/// [`send_request`] under `limits`.
pub async fn send_limited(
    request: &HttpRequest,
    host: &str,
    limits: Limits,
) -> Result<HttpResponse, CloudError> {
    let client = messenger_http::client(limits.connect, limits.total).map_err(transport)?;
    let attempt = || async {
        let builder = match request.method.as_str() {
            "GET" => client.get(&request.url),
            "POST" => client.post(&request.url),
            "PUT" => client.put(&request.url),
            "DELETE" => client.delete(&request.url),
            "HEAD" => client.head(&request.url),
            other => return Err(bad_request(format!("the method {other:?} is not sent"))),
        };
        let mut builder = builder;
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let mut response = builder.send().await.map_err(transport)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                let value = value.to_str().ok()?;
                Some((name.as_str().to_ascii_lowercase(), value.to_string()))
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            if body.len() + chunk.len() > limits.max_answer {
                return Err(transport("the answer of the server is too large"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    };
    messenger_http::following_route(host, attempt)
        .await
        .map_err(transport)?
}

/// [`Http`] over the messenger: the project's hosts come from the manifest
/// in use, and only while the user is on the project's servers; nothing
/// goes to the project when the servers are the user's own (plan 4.5).
pub struct ProjectHttp {
    app: AppHandle,
}

impl ProjectHttp {
    /// The way of the app: what the seam `Http` offers the module cloud,
    /// and what a report of the messenger goes by (commands/report.rs).
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }

    async fn reach_now(&self) -> Result<CloudReach, CloudError> {
        let runtime = self
            .app
            .state::<MessengerState>()
            .runtime()
            .map_err(|e| CloudError::new(code::MESSENGER_STOPPED, e.to_string()))?;
        let relays = runtime.relays();
        match relays.servers_mode().await.map_err(from_messenger)? {
            None => Ok(CloudReach::Unchosen {
                hosts: self.manifest_hosts().await?,
            }),
            Some(messenger_runtime::ServersMode::Own) => Ok(CloudReach::Own),
            Some(messenger_runtime::ServersMode::Veydan) => Ok(CloudReach::Veydan {
                hosts: self.manifest_hosts().await?,
            }),
        }
    }

    /// The bases of `/cloud/` from the manifest in use (the embedded one on
    /// a clean install), for the region of the device.
    async fn manifest_hosts(&self) -> Result<Vec<String>, CloudError> {
        let runtime = self
            .app
            .state::<MessengerState>()
            .runtime()
            .map_err(|e| CloudError::new(code::MESSENGER_STOPPED, e.to_string()))?;
        let relays = runtime.relays();
        let (manifest, _) = relays.current_manifest().await.map_err(from_messenger)?;
        let region = relays.region().await.map_err(from_messenger)?;
        let media = manifest.media_for_region(&region);
        Ok(cloud_bases(media.iter().map(|m| (m.kind.as_str(), m.url.as_str()))))
    }
}

impl Http for ProjectHttp {
    fn reach(&self) -> SeamFuture<'_, CloudReach> {
        Box::pin(self.reach_now())
    }

    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        Box::pin(async move {
            // Asked again for every request: a switch to the user's own
            // servers stops the next one. Servers not chosen yet: a clean
            // install recovering (the rendezvous of the trusted people, the
            // close of a restore) reaches the project by the manifest in use.
            let hosts = match self.reach_now().await? {
                CloudReach::Veydan { hosts } => hosts,
                CloudReach::Unchosen { hosts } => hosts,
                CloudReach::Own => {
                    return Err(CloudError::new(
                        code::NO_HOST,
                        "the servers of the project are not in use",
                    ))
                }
            };
            let host = target_host(&request.url, &hosts)?;
            send_request(&request, &host).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use nostr::prelude::Event;
    use serde_json::{json, Map};
    use sha2::{Digest, Sha256};

    const URL: &str = "https://eu-1-media.example.com/cloud/v1/recovery/request?npub=3f";

    /// The checks of services/push/crates/vpush-server/src/auth.rs, which
    /// vcloud copies (cloud-api.md 2.3): the header as a caller builds it
    /// from the event, read as the server reads it. The signer's key.
    fn server_accepts(
        event: &WireEvent,
        method: &str,
        url: &str,
        body: &[u8],
        now: u64,
    ) -> Result<String, String> {
        let header = format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(event).map_err(|e| e.to_string())?)
        );
        let encoded = header
            .strip_prefix("Nostr ")
            .ok_or("the scheme is not Nostr")?;
        let json = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|_| "not base64")?;
        let event: Event = serde_json::from_slice(&json).map_err(|_| "not an event")?;
        if event.kind != Kind::HttpAuth {
            return Err("the event is not of kind 27235".into());
        }
        event.verify().map_err(|_| "the signature does not match")?;
        if event.created_at.as_secs().abs_diff(now) > 60 {
            return Err("expired".into());
        }
        let tag = |name: &str| -> Option<String> {
            event.tags.iter().find_map(|t| match t.as_slice() {
                [kind, value, ..] if kind == name => Some(value.clone()),
                _ => None,
            })
        };
        if tag("u").as_deref() != Some(url) {
            return Err("url mismatch".into());
        }
        if !tag("method").is_some_and(|m| m.eq_ignore_ascii_case(method)) {
            return Err("signed for another method".into());
        }
        if !body.is_empty() {
            let want = hex(&Sha256::digest(body));
            if !tag("payload").is_some_and(|p| p.eq_ignore_ascii_case(&want)) {
                return Err("signed for another body".into());
            }
        }
        Ok(event.pubkey.to_hex())
    }

    fn digest(body: &[u8]) -> [u8; 32] {
        Sha256::digest(body).into()
    }

    #[test]
    fn a_nip98_event_of_the_signer_passes_the_checks_of_the_server() {
        let keys = Keys::generate();
        let at = 1_791_012_345;
        let body = br#"{"npub":"3f","temp_pub":"ab"}"#;
        let event = nip98_event(&keys, "post", URL, Some(digest(body)), at).expect("event");
        assert_eq!(event.kind, 27235);
        assert_eq!(event.pubkey, keys.public_key().to_hex());
        assert_eq!(
            server_accepts(&event, "POST", URL, body, at + 30),
            Ok(keys.public_key().to_hex())
        );
        // The method went in capitals, the address character by character.
        assert!(event
            .tags
            .contains(&vec!["method".to_string(), "POST".to_string()]));
        assert!(event.tags.contains(&vec!["u".to_string(), URL.to_string()]));

        // What the server refuses.
        assert!(server_accepts(&event, "POST", URL, body, at + 61).is_err());
        assert!(server_accepts(&event, "GET", URL, body, at).is_err());
        assert!(server_accepts(&event, "POST", &URL.replace("eu-1", "eu-2"), body, at).is_err());
        assert!(server_accepts(&event, "POST", URL, b"another body", at).is_err());
        let mut forged = event.clone();
        forged.content = "changed".into();
        assert!(server_accepts(&forged, "POST", URL, body, at).is_err());

        // A request without a body signs no payload, and passes.
        let get = nip98_event(&keys, "GET", URL, None, at).expect("event");
        assert!(get.tags.iter().all(|t| t[0] != "payload"));
        assert!(server_accepts(&get, "GET", URL, b"", at).is_ok());
    }

    #[test]
    fn a_nip98_event_is_not_signed_for_a_strange_request() {
        let keys = Keys::generate();
        for (method, url) in [
            ("", URL),
            ("GE T", URL),
            ("GET", "ftp://x.example.com/"),
            ("GET", "https://a.example.com/ b"),
        ] {
            let e = nip98_event(&keys, method, url, None, 1)
                .err()
                .map(|e| e.code);
            assert_eq!(e.as_deref(), Some(code::BAD_REQUEST), "{method} {url}");
        }
    }

    #[test]
    fn nip44_goes_between_two_keys_and_no_third() {
        let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
        let secret = b"a share of 88 bytes, or a blob";
        let sealed = nip44_encrypt(&a, &b.public_key().to_hex(), secret).expect("encrypt");
        // The payload of NIP-44: base64 text, version 2.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&sealed)
            .expect("base64");
        assert_eq!(decoded[0], 2);
        let opened = nip44_decrypt(&b, &a.public_key().to_hex(), &sealed).expect("decrypt");
        assert_eq!(opened.as_slice(), secret);
        assert!(nip44_decrypt(&c, &a.public_key().to_hex(), &sealed).is_err());
        let upper = b.public_key().to_hex().to_uppercase();
        assert_eq!(
            nip44_encrypt(&a, &upper, secret)
                .err()
                .map(|e| e.code)
                .as_deref(),
            Some(code::BAD_REQUEST)
        );
    }

    fn note(t: &str) -> Note {
        let mut fields = Map::new();
        fields.insert("request_id".into(), json!("9c"));
        Note::new(t, fields)
    }

    #[tokio::test]
    async fn each_note_goes_only_by_the_door_of_its_type() {
        let k = Keys::generate().public_key().to_hex();
        // To a key T: the answer and the link alone.
        assert!(check_to_key(&note("recovery.answer"), &k).is_ok());
        assert!(check_to_key(&note("device.link"), &k).is_ok());
        for t in [
            "recovery.share",
            "recovery.cancel",
            "own.read",
            "receipt.read",
            "",
        ] {
            let e = check_to_key(&note(t), &k).err().map(|e| e.code);
            assert_eq!(e.as_deref(), Some(code::BAD_REQUEST), "{t}");
        }
        assert!(check_to_key(&note("device.link"), "npub1nope").is_err());
        // To a contact: the rows of the table whose recipient is a contact.
        for t in PEER_TYPES {
            assert!(check_to_peer(&note(t), &k).is_ok(), "{t}");
        }
        for t in [
            "recovery.answer",
            "device.link",
            "recovery.cancel",
            "device.link.done",
            "recovery.x",
            "reaction",
        ] {
            let e = check_to_peer(&note(t), &k).err().map(|e| e.code);
            assert_eq!(e.as_deref(), Some(code::BAD_REQUEST), "{t}");
        }
        // To my own devices: the cancel and the end of a link.
        assert!(check_own(&note("recovery.cancel")).is_ok());
        assert!(check_own(&note("device.link.done")).is_ok());
        for t in ["recovery.share", "device.link", "own.read"] {
            assert_eq!(
                check_own(&note(t)).err().map(|e| e.code).as_deref(),
                Some(code::BAD_REQUEST),
                "{t}"
            );
        }
        // What the seam does with each today.
        let notes = ServiceNotes::new(Arc::new(LinkWire::detached()));
        let refused = notes
            .send_to_key(note("recovery.share"), &k)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(refused.as_deref(), Some(code::BAD_REQUEST));
        let refused = notes
            .send(note("device.link"), &k)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(refused.as_deref(), Some(code::BAD_REQUEST));
        let refused = notes
            .send_own(note("recovery.share"))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(refused.as_deref(), Some(code::BAD_REQUEST));
        let later = notes
            .send_to_key(note("recovery.answer"), &k)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(later.as_deref(), Some(code::BAD_REQUEST));
        // A link goes to the runtime, which is not there.
        let later = notes
            .send_to_key(note("device.link"), &k)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(later.as_deref(), Some(code::MESSENGER_STOPPED));
        let later = notes
            .send(note("recovery.share"), &k)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(later.as_deref(), Some(code::NOT_SUPPORTED));
        let later = notes
            .send_own(note("recovery.cancel"))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(later.as_deref(), Some(code::NOT_SUPPORTED));
        let listen = notes
            .listen_as(TempKey::generate().expect("key"))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(listen.as_deref(), Some(code::MESSENGER_STOPPED));
    }

    #[test]
    fn the_time_of_a_signature_is_the_clock_of_the_device() {
        let at = now().expect("a clock after 1970");
        assert!(at > 1_700_000_000);
    }

    /// The vector of crates/recovery (golden/recovery-v1.json,
    /// `sync_vault_passphrase`): the nsec 0x42 × 32 gives this passphrase.
    #[test]
    fn the_passphrase_of_the_vault_is_derived_as_recovery_derives_it() {
        let secret = nostr::prelude::SecretKey::from_slice(&[0x42; 32]).expect("scalar");
        let keys = Keys::new(secret);
        let okm =
            derive_secret(&keys, veydan_shell::seams::SYNC_VAULT_PASSPHRASE).expect("derived");
        let passphrase = base64::engine::general_purpose::STANDARD.encode(okm.as_slice());
        assert_eq!(passphrase, "B2fw67HdFNJT2vAlkJ9Q19jFqc2F2V3iUZF4uCgnEnI=");
        // Another label is refused: the seam derives one thing only.
        let other = derive_secret(&keys, "veydan-recovery/escrow-auth/1")
            .err()
            .map(|e| e.code);
        assert_eq!(other.as_deref(), Some(code::BAD_REQUEST));
        // The time of an event of NIP-98 is the one asked.
        let event = nip98_event(
            &keys,
            "GET",
            "https://media-eu.example.com/vault/v1/usage",
            None,
            1_791_000_000,
        )
        .expect("event");
        assert_eq!(event.created_at, 1_791_000_000);
    }

    fn card(id: &str) -> Card {
        Card {
            id: id.into(),
            t: "recovery.requested".into(),
            fields: Map::new(),
            at: 1,
        }
    }

    #[tokio::test]
    async fn a_card_comes_from_the_pin_once_and_not_after_it_was_dismissed() {
        let pin = Keys::generate().public_key().to_hex();
        let other = Keys::generate().public_key().to_hex();
        let cards = PinnedCards::new(Some(&pin));
        let mut first = cards.cards().await.expect("listen");
        let mut second = cards.cards().await.expect("listen");
        assert!(
            !cards.offer(&other, card("a")),
            "a card from another key is a message, not a card"
        );
        assert!(cards.offer(&pin, card("a")));
        assert!(!cards.offer(&pin, card("a")), "the same card again");
        assert_eq!(first.recv().await.map(|c| c.id).as_deref(), Some("a"));
        assert_eq!(second.recv().await.map(|c| c.id).as_deref(), Some("a"));
        cards.dismiss("b").await.expect("dismiss");
        assert!(!cards.offer(&pin, card("b")), "dismissed before it came");
        drop(second);
        assert!(cards.offer(&pin, card("c")));
        assert_eq!(first.recv().await.map(|c| c.id).as_deref(), Some("c"));
        assert_eq!(
            guard(&cards.state).listeners.len(),
            1,
            "the listener that left is forgotten"
        );
        // Without a pin nothing is a card.
        assert!(!PinnedCards::new(None).offer(&pin, card("d")));
        let pinned = VCLOUD_BOT_PUBKEY.expect("the bot of the test vcloud is pinned");
        assert!(pinned.len() == 64 && pinned.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }

    #[tokio::test]
    async fn a_card_that_came_before_any_listener_waits_for_the_first() {
        let pin = Keys::generate().public_key().to_hex();
        let cards = PinnedCards::new(Some(&pin));
        assert!(cards.offer(&pin, card("early")));
        assert!(
            !cards.offer(&pin, card("early")),
            "waiting is not a second card"
        );
        assert!(cards.offer(&pin, card("handled")));
        cards.dismiss("handled").await.expect("dismiss");
        let mut first = cards.cards().await.expect("listen");
        assert_eq!(first.recv().await.map(|c| c.id).as_deref(), Some("early"));
        assert!(
            first.try_recv().is_err(),
            "a card dismissed while it waited is not given"
        );
        // Given once: a later listener does not get it again.
        let mut later = cards.cards().await.expect("listen");
        assert!(later.try_recv().is_err());
        drop((first, later));

        // At most a queue of cards waits; the oldest are dropped.
        let cards = PinnedCards::new(Some(&pin));
        for i in 0..CARDS_QUEUE + 8 {
            assert!(cards.offer(&pin, card(&format!("c{i}"))));
        }
        let mut rx = cards.cards().await.expect("listen");
        let mut ids = Vec::new();
        while let Ok(c) = rx.try_recv() {
            ids.push(c.id);
        }
        assert_eq!(ids.len(), CARDS_QUEUE);
        assert_eq!(ids.first().map(String::as_str), Some("c8"));
    }

    #[test]
    fn the_ids_remembered_are_bounded() {
        let mut ids = Remembered::default();
        for i in 0..CARDS_REMEMBERED + 10 {
            ids.insert(&format!("id{i}"));
        }
        ids.insert("id20");
        assert_eq!(ids.order.len(), CARDS_REMEMBERED);
        assert_eq!(ids.ids.len(), CARDS_REMEMBERED);
        assert!(
            !ids.contains("id0")
                && ids.contains("id10")
                && ids.contains(&format!("id{}", CARDS_REMEMBERED + 9))
        );
    }

    /// The hosts of `/cloud/` are the project's blossom servers over https,
    /// each once; nothing else of the manifest is one.
    #[test]
    fn the_bases_of_the_cloud_are_the_blossom_hosts_of_the_manifest() {
        let media = [
            ("blossom", "https://eu-1-media.example.com"),
            ("blossom", "https://EU-1-media.example.com/"),
            ("s3", "https://s3.example.com/bucket"),
            ("blossom", "http://plain.example.com"),
            ("blossom", "https://us-1-media.example.com:8443/path"),
            ("blossom", "https://user@evil.example.com"),
        ];
        assert_eq!(
            cloud_bases(media),
            vec![
                "https://eu-1-media.example.com".to_string(),
                "https://us-1-media.example.com:8443".to_string(),
            ]
        );
    }

    #[test]
    fn a_request_goes_only_to_a_host_of_the_project() {
        let bases = vec![
            "https://eu-1-media.example.com".to_string(),
            "https://us-1-media.example.com:8443".to_string(),
        ];
        let host = |url: &str| target_host(url, &bases).map_err(|e| e.code);
        assert_eq!(
            host("https://eu-1-media.example.com/cloud/v1/account"),
            Ok("eu-1-media.example.com".into())
        );
        assert_eq!(
            host("https://us-1-media.example.com:8443/vault/v1/usage"),
            Ok("us-1-media.example.com".into())
        );
        for other in [
            "https://eu-1-media.example.com.evil.example.com/cloud/v1/account",
            "https://203.0.113.7/cloud/v1/account",
            "http://eu-1-media.example.com/cloud/v1/account",
            "https://eu-1-media.example.com",
        ] {
            assert_eq!(host(other), Err(code::NO_HOST.to_string()), "{other}");
        }
    }

    #[test]
    fn a_vault_request_goes_only_to_a_host_of_storage() {
        let hosts = vec![
            "https://media-eu.example.com/vault".to_string(),
            "https://Media-US.example.com:8443/vault".to_string(),
            "http://media-ru.example.com/vault".to_string(),
        ];
        let host = |url: &str| vault_target(url, &hosts).map_err(|e| e.code);
        assert_eq!(
            host("https://media-eu.example.com/vault/v1/o/blobs/aa"),
            Ok("media-eu.example.com".into())
        );
        assert_eq!(
            host("https://Media-US.example.com:8443/vault/v1/o?prefix="),
            Ok("media-us.example.com".into())
        );
        for other in [
            "https://media-eu.example.com/vaultx/v1/o/a",
            "https://media-eu.example.com/cloud/v1/account",
            "https://media-eu.example.com.evil.example.com/vault/v1/o/a",
            "http://media-ru.example.com/vault/v1/o/a",
            "https://media-eu.example.com/vault",
        ] {
            assert_eq!(host(other), Err(code::NO_HOST.to_string()), "{other}");
        }
    }

    /// A one-shot HTTP/1.1 server on loopback: it reads one request whole
    /// and answers `answer`; what it read comes back through the handle.
    async fn one_answer(answer: String) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.expect("read");
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let length = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().to_string())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if seen.len() >= head_end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            socket.write_all(answer.as_bytes()).await.expect("write");
            socket.shutdown().await.ok();
            seen
        });
        (format!("http://{addr}"), task)
    }

    #[tokio::test]
    async fn a_request_is_sent_as_written_and_its_answer_read_whole() {
        let body = r#"{"error":{"code":"pow_insufficient","d":10}}"#;
        let (base, server) = one_answer(format!(
            "HTTP/1.1 428 Precondition Required\r\nContent-Type: application/json\r\nX-Request-Id: 9c\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
        .await;
        let request = HttpRequest {
            method: "POST".into(),
            url: format!("{base}/cloud/v1/recovery/request"),
            headers: vec![
                ("authorization".into(), "Nostr e30=".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: b"{\"npub\":\"3f\"}".to_vec(),
        };
        let answer = send_request(&request, "127.0.0.1").await.expect("answer");
        assert_eq!(answer.status, 428);
        assert_eq!(answer.header("X-Request-Id"), Some("9c"));
        assert!(answer
            .body
            .starts_with(b"{\"error\":{\"code\":\"pow_insufficient\""));
        let seen = String::from_utf8(server.await.expect("server")).expect("utf-8");
        assert!(
            seen.starts_with("POST /cloud/v1/recovery/request HTTP/1.1\r\n"),
            "{seen}"
        );
        assert!(seen.contains("authorization: Nostr e30=\r\n"), "{seen}");
        assert!(seen.ends_with("{\"npub\":\"3f\"}"), "{seen}");

        let refused = HttpRequest {
            method: "PATCH".into(),
            ..request
        };
        assert_eq!(
            send_request(&refused, "127.0.0.1")
                .await
                .map_err(|e| e.code),
            Err(code::BAD_REQUEST.to_string())
        );
        let nobody = HttpRequest {
            method: "GET".into(),
            url: "http://127.0.0.1:9/cloud/v1/account".into(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        assert_eq!(
            send_request(&nobody, "127.0.0.1")
                .await
                .map_err(|e| e.code),
            Err(code::TRANSPORT.to_string())
        );
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The seams between modules: services one module offers another through
//! the shell, so that neither depends on the other's crate nor reads its
//! tables (internal/vcloud-plan.md 2.2, services/cloud/spec/cloud-api.md
//! §12). The module that owns a service puts it in place in its setup with
//! [`provide`]; a module set up after it, or any code that runs later,
//! takes it with [`seam`]. The shell knows the shapes and no module by name.
//!
//! The messenger offers [`Signer`], [`PeerNotes`] and [`BotCards`]; the
//! module `cloud` takes them, and offers its own client of Veydan Cloud
//! (`CloudApi` of `veydan-cloud`) the same way. Every method is async and
//! fails with one error, [`CloudError`].

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{Manager, Runtime};
use tokio::sync::mpsc;
use veydan_core::BoxFuture;
use zeroize::Zeroizing;

/// What a method of a seam returns.
pub type SeamFuture<'a, T> = BoxFuture<'a, Result<T, CloudError>>;

/// The one error of the seams (cloud-api.md §12): `code` is a word of
/// section 9 of the contract, a word of the client (`transport`,
/// `no_host`) or a local refusal of a seam ([`code`]); `message` is for a
/// person and may change, a caller branches on `code` alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudError {
    pub code: String,
    pub message: String,
    /// The `request_id` of the answer of a server, when one answered.
    pub request_id: Option<String>,
}

impl CloudError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            request_id: None,
        }
    }

    /// Whether the error is the one named by `code`.
    pub fn is(&self, code: &str) -> bool {
        self.code == code
    }
}

impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(id) = &self.request_id {
            write!(f, " (request {id})")?;
        }
        Ok(())
    }
}

impl std::error::Error for CloudError {}

/// The words of [`CloudError::code`] that do not come from a server: the
/// first two are the client's, the others local refusals of a seam that no
/// server sends; cloud-api.md §12 lists them all.
pub mod code {
    /// The network failed on the way (cloud-api.md §12).
    pub const TRANSPORT: &str = "transport";
    /// No host of `/cloud/` or `storage[]` answered, or none is known yet
    /// (cloud-api.md §12).
    pub const NO_HOST: &str = "no_host";
    /// An argument out of bounds: a key that is not 64 hex, a note of a type
    /// the method does not send. The word of section 9 for the same case.
    pub const BAD_REQUEST: &str = "bad_request";
    /// The messenger is switched off or its runtime did not start.
    pub const MESSENGER_STOPPED: &str = "messenger_stopped";
    /// The messenger has no identity on this device.
    pub const NO_IDENTITY: &str = "no_identity";
    /// The lock is closed: the key of the identity cannot be read.
    pub const LOCKED: &str = "locked";
    /// The seam exists and the service behind it does not do this yet.
    pub const NOT_SUPPORTED: &str = "not_supported";
    /// Signing, encryption or decryption failed.
    pub const CRYPTO: &str = "crypto";
    /// No relay of the project could be reached (the screens of a device
    /// link say «нет связи с реле», cloud-recovery.md §6).
    pub const NO_RELAY: &str = "no_relay";
    /// The secret given is not the lock's; the counter of the lock counts it.
    pub const WRONG_PASSWORD: &str = "wrong_password";
    /// Another identity is on this device already.
    pub const IDENTITY_EXISTS: &str = "identity_exists";
    /// The data file or a table of the device refused: the lock's `Db`, an
    /// error of sync-host, a part of the app that is not in place.
    pub const STORAGE: &str = "storage";
}

/// A nostr event as it travels (NIP-01): what [`Signer::nip98`] signs.
/// The caller builds `Authorization: Nostr <base64 of its JSON>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

/// A service note (internal/messenger-wire.md §3): rumor kind 30079, or
/// 30078 to my own devices, with the envelope `{"v":1,"t":…,…fields}`. Its
/// fields may carry a share or a sealed nsec: Debug names them only.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// `recovery.share`, `recovery.answer`, `device.link`, …
    pub t: String,
    /// The fields of the envelope besides `v` and `t`.
    pub fields: Map<String, Value>,
    /// The `expiration` of the outer wrap, unix seconds; `None` takes the
    /// one messenger-wire.md §3 gives the type.
    pub expires_at: Option<u64>,
    /// Who sealed a note that came (64 hex); `None` on a note that goes.
    pub from: Option<String>,
}

impl Note {
    /// A note to send.
    pub fn new(t: impl Into<String>, fields: Map<String, Value>) -> Self {
        Self {
            t: t.into(),
            fields,
            expires_at: None,
            from: None,
        }
    }
}

/// The names of the fields, never their values.
fn field_names(fields: &Map<String, Value>) -> Vec<&str> {
    fields.keys().map(String::as_str).collect()
}

impl fmt::Debug for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Note")
            .field("t", &self.t)
            .field("fields", &field_names(&self.fields))
            .field("expires_at", &self.expires_at)
            .field("from", &self.from)
            .finish()
    }
}

/// The types [`PeerNotes::send`] sends to an approved contact: the rows of
/// the table of messenger-wire.md §3 whose recipient is the trusted person
/// or the owner.
pub const PEER_TYPES: [&str; 5] = [
    "recovery.share",
    "recovery.share.ack",
    "recovery.share.ping",
    "recovery.share.pong",
    "recovery.share.revoke",
];

/// The only types [`PeerNotes::send_to_key`] sends and
/// [`PeerNotes::listen_as`] takes: the one door past the rule that notes go
/// to approved contacts only (messenger-wire.md §3).
pub const TO_KEY_TYPES: [&str; 2] = ["recovery.answer", "device.link"];

/// The types [`PeerNotes::send_own`] sends to my own devices, as kind 30078
/// (messenger-wire.md §3, the notes to my own devices).
pub const OWN_TYPES: [&str; 2] = ["recovery.cancel", "device.link.done"];

/// A card of the vcloud bot (messenger-wire.md §3, the table of cards),
/// taken only from the pinned key. Debug names its fields only.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// The id of the rumor: what [`BotCards::dismiss`] names.
    pub id: String,
    /// `recovery.requested`, `account.deleted`, `plan.expiring`, …
    pub t: String,
    pub fields: Map<String, Value>,
    /// When the bot wrote it, unix seconds.
    pub at: u64,
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Card")
            .field("id", &self.id)
            .field("t", &self.t)
            .field("fields", &field_names(&self.fields))
            .field("at", &self.at)
            .finish()
    }
}

/// The temporary secp256k1 key T of one procedure (cloud-recovery.md §8.3):
/// it signs the requests of the procedure and receives the notes sent to
/// it. Wiped when dropped; never cloned, never shown.
pub struct TempKey {
    secret: Zeroizing<[u8; 32]>,
}

impl TempKey {
    /// A new key from the random source of the system, written straight
    /// into memory that is wiped. A value that is not a scalar of secp256k1
    /// (a chance of about 2^-128) is refused where it signs.
    pub fn generate() -> Result<Self, CloudError> {
        let mut secret = Zeroizing::new([0u8; 32]);
        getrandom::fill(secret.as_mut_slice())
            .map_err(|e| CloudError::new(code::CRYPTO, format!("no random source: {e}")))?;
        Ok(Self { secret })
    }

    /// A key the caller holds already, from wiped memory to wiped memory.
    pub fn new(secret: Zeroizing<[u8; 32]>) -> Self {
        Self { secret }
    }

    pub fn secret_bytes(&self) -> &[u8; 32] {
        &self.secret
    }
}

impl fmt::Debug for TempKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TempKey(..)")
    }
}

/// The key of the messenger's identity at work, without giving it out
/// (cloud-api.md §12, 2.3). Keys are 64 hex, lower case.
pub trait Signer: Send + Sync {
    /// The public key of the identity.
    fn pubkey(&self) -> SeamFuture<'_, String>;
    /// An event of kind 27235 with the tags `u` (exactly `url`), `method`
    /// and, when the body is not empty, `payload` (its sha256), signed by
    /// the identity now.
    fn nip98<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
    ) -> SeamFuture<'a, WireEvent>;
    /// NIP-44 v2 to the key `to`: the payload as NIP-44 writes it (base64
    /// text), as bytes.
    fn nip44_encrypt<'a>(&'a self, to: &'a str, plaintext: &'a [u8]) -> SeamFuture<'a, Vec<u8>>;
    /// The inverse of [`Signer::nip44_encrypt`], from the key `from`.
    fn nip44_decrypt<'a>(
        &'a self,
        from: &'a str,
        ciphertext: &'a [u8],
    ) -> SeamFuture<'a, Zeroizing<Vec<u8>>>;
    /// [`Signer::nip98`] made at `created_at` (unix seconds) instead of
    /// now: the clock of the server after an `auth_expired`
    /// (spec/vstore.md §14). A signer that cannot says `not_supported`.
    fn nip98_at<'a>(
        &'a self,
        method: &'a str,
        url: &'a str,
        payload_sha256: Option<[u8; 32]>,
        created_at: u64,
    ) -> SeamFuture<'a, WireEvent> {
        let _ = (method, url, payload_sha256, created_at);
        not_supported("nip98_at")
    }
    /// HKDF-SHA256 of the nsec with the salt [`DERIVE_SALT`] and the info
    /// `label`, one of [`DERIVE_LABELS`] (cloud-recovery.md §3): a secret of
    /// the identity for one purpose, never the nsec. Another label is
    /// `bad_request`; a signer that cannot says `not_supported`.
    fn derive_secret<'a>(&'a self, label: &'a str) -> SeamFuture<'a, Zeroizing<[u8; 32]>> {
        let _ = label;
        not_supported("derive_secret")
    }
    /// The 32 bytes of the nsec, for the escrow of the module `cloud` alone
    /// (cloud-recovery.md §4): the one place the key leaves the lock, in a
    /// wiped buffer, and only while the lock is open (`locked` otherwise).
    /// A signer that cannot says `not_supported`.
    fn export_for_escrow(&self) -> SeamFuture<'_, Zeroizing<[u8; 32]>> {
        not_supported("export_for_escrow")
    }
    /// Step 6 of a restore from the escrow (cloud-recovery.md §7): the nsec
    /// goes into the box of the lock as the identity of this device, and the
    /// messenger starts with it. Refused while an identity exists
    /// (`bad_request`) or the lock is closed (`locked`); a signer that cannot
    /// says `not_supported`.
    fn import_for_restore<'a>(&'a self, nsec: &'a [u8; 32]) -> SeamFuture<'a, ()> {
        let _ = nsec;
        not_supported("import_for_restore")
    }
}

/// The salt of what [`Signer::derive_secret`] gives (cloud-recovery.md §3).
pub const DERIVE_SALT: &str = "veydan-cloud";

/// The info of the passphrase of the sync vault on Veydan Cloud: its base64
/// is the passphrase (internal/vcloud-plan.md 4.8).
pub const SYNC_VAULT_PASSPHRASE: &str = "sync-vault-passphrase/1";

/// The only labels [`Signer::derive_secret`] derives.
pub const DERIVE_LABELS: [&str; 1] = [SYNC_VAULT_PASSPHRASE];

fn not_supported<'a, T: Send + 'a>(what: &'static str) -> SeamFuture<'a, T> {
    Box::pin(async move {
        Err(CloudError::new(
            code::NOT_SUPPORTED,
            format!("the signer does not do {what}"),
        ))
    })
}

/// A host of `/vault/` as `storage[]` of the manifest names it
/// (internal/vcloud-plan.md 3.3, 4.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageHost {
    /// The `public_url` of vstore, `https://media-eu.example.com/vault`.
    pub url: String,
    /// Regions served, `*` for any (internal/vcloud-plan.md 3.3).
    pub regions: Vec<String>,
    /// Lower first.
    pub priority: u32,
}

/// `storage[]` of the manifest in use, from the messenger that keeps it.
pub trait VaultHosts: Send + Sync {
    /// The hosts for the region of the device, the one to try first first;
    /// none while the servers in use are not the project's (plan 4.5).
    fn storage_hosts(&self) -> SeamFuture<'_, Vec<StorageHost>>;
    /// One request to a host of `storage[]` by the messenger's client: by a
    /// bridge wherever the media go by one, made again when that way changes
    /// (spec/vstore.md §14). Any other host is `no_host`, a network failure
    /// `transport`; every answer of a server is `Ok`.
    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        let _ = request;
        not_supported("send")
    }
}

/// Service notes over the messenger's own wire (cloud-api.md §12,
/// messenger-wire.md §3): the module `cloud` never goes to the relays
/// itself.
pub trait PeerNotes: Send + Sync {
    /// A note to an approved contact (`full_chat`), only of [`PEER_TYPES`].
    fn send<'a>(&'a self, note: Note, peer: &'a str) -> SeamFuture<'a, ()>;
    /// A note to my own devices (kind 30078, sealed and wrapped to my key
    /// alone), only of [`OWN_TYPES`].
    fn send_own(&self, note: Note) -> SeamFuture<'_, ()>;
    /// A note to any key, only of [`TO_KEY_TYPES`].
    fn send_to_key<'a>(&'a self, note: Note, pubkey: &'a str) -> SeamFuture<'a, ()>;
    /// The notes of [`TO_KEY_TYPES`] that come to T, before there is an
    /// identity. Dropping the receiver ends the listening.
    fn listen_as(&self, t: TempKey) -> SeamFuture<'_, mpsc::Receiver<Note>>;
    /// The notes that come to the identity and are the module `cloud`'s
    /// (messenger-wire.md §3): those of [`PEER_TYPES`] from an approved
    /// contact, `from` the sealer, and those of [`OWN_TYPES`] from my other
    /// devices, `from` my own key. Dropping the receiver ends the listening;
    /// a messenger that cannot says `not_supported`.
    fn incoming(&self) -> SeamFuture<'_, mpsc::Receiver<Note>> {
        not_supported("incoming")
    }
}

/// An approved contact (`full_chat`) as the picker of the trusted people
/// shows it (cloud-recovery.md §8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedContact {
    /// 64 hex, lower case.
    pub pubkey: String,
    /// The name the contact is known by here; never empty.
    pub name: String,
}

/// The address book of the messenger, as far as the module `cloud` needs
/// it: whom a service note may go to.
pub trait Contacts: Send + Sync {
    /// The contacts we both chose to talk with, by name.
    fn approved(&self) -> SeamFuture<'_, Vec<ApprovedContact>>;
}

/// How long a code of a link, its key T and the wrap with the nsec live
/// (cloud-recovery.md §6): ten minutes.
pub const LINK_TTL_SECS: u64 = 600;

/// A live code of a link found on the relays ([`DeviceLink::find_code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCodeFound {
    /// The key T that signed it, 64 hex.
    pub t_pub: String,
    /// Its NIP-40 `expiration`, unix seconds, when it names one.
    pub expires_at: Option<u64>,
}

/// What the messenger does for the linking of a device besides the notes
/// (cloud-recovery.md §6, messenger-wire.md §3): the typed code on the
/// relays, the nsec handed over after the lock's secret and taken over on the
/// new device, the end of the procedure. Keys are 64 hex, lower case.
pub trait DeviceLink: Send + Sync {
    /// The nsec of the identity for the inner wrap of a link: only after
    /// the lock's secret, counted as an unlock is (`wrong_password`); a
    /// device without a lock needs none.
    fn hand_over<'a>(&'a self, password: Option<&'a str>) -> SeamFuture<'a, Zeroizing<[u8; 32]>>;
    /// The identity of `nsec` imported on this device, the lock opened with
    /// `password` first when it is closed (`locked` without one); its key.
    fn take_over<'a>(
        &'a self,
        nsec: Zeroizing<[u8; 32]>,
        password: Option<&'a str>,
    ) -> SeamFuture<'a, String>;
    /// Publishes the kind 30078 of a typed code: signed by T, `d` = hex
    /// sha256 of the code, forgotten at `expires_at`; the id of the event.
    fn publish_code<'a>(
        &'a self,
        t: &'a TempKey,
        d: &'a str,
        expires_at: u64,
    ) -> SeamFuture<'a, String>;
    /// The key T and the `expiration` of the latest live kind 30078 under
    /// `d`, if any.
    fn find_code<'a>(&'a self, d: &'a str) -> SeamFuture<'a, Option<LinkCodeFound>>;
    /// NIP-09 of an event of T (its code), signed by T.
    fn delete_code<'a>(&'a self, t: &'a TempKey, event_id: &'a str) -> SeamFuture<'a, ()>;
    /// Waits until `until` for `device.link.done` with this `t_pub` from one
    /// of my devices: whether it came.
    fn await_done<'a>(&'a self, t_pub: &'a str, until: u64) -> SeamFuture<'a, bool>;
    /// NIP-09 of every wrap [`PeerNotes::send_to_key`] sent to `t_pub`, by
    /// the outer keys it kept; nothing when it kept none.
    fn withdraw<'a>(&'a self, t_pub: &'a str) -> SeamFuture<'a, ()>;
    /// The picture of `text` as a QR code (SVG), drawn as every code of the
    /// app is.
    fn qr_svg<'a>(&'a self, text: &'a str) -> SeamFuture<'a, String>;
}

/// The cards of the vcloud bot (cloud-api.md §12, messenger-wire.md §3).
pub trait BotCards: Send + Sync {
    /// The cards from the pinned key, already filtered by the signer of
    /// the seal, as they come.
    fn cards(&self) -> SeamFuture<'_, mpsc::Receiver<Card>>;
    /// The card was handled: it is not given again.
    fn dismiss<'a>(&'a self, id: &'a str) -> SeamFuture<'a, ()>;
}

/// One HTTP request to a server of the project, as [`Http::send`] takes it.
/// `url` is sent exactly as written: a NIP-98 tag `u` signed for it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// `GET`, `HEAD`, `POST`, `PUT` or `DELETE`.
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The answer of a server to an [`HttpRequest`]; header names in lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Whose servers the messenger uses, and so whether Veydan Cloud may be
/// asked anything (internal/vcloud-plan.md 4.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "servers", rename_all = "snake_case")]
pub enum CloudReach {
    /// The user has not chosen yet (onboarding): nothing goes out on its
    /// own. The bases of the manifest in use (the embedded one on a clean
    /// install) are here for a recovery the person starts (cloud-recovery.md
    /// §7, §8.5): the escrow and the rendezvous come before the choice.
    Unchosen { hosts: Vec<String> },
    /// The user's own servers: no request goes to a host of the project.
    Own,
    /// The project's servers: the bases (`https://host`) where `/cloud/`
    /// and `/vault/` answer, the preferred first.
    Veydan { hosts: Vec<String> },
}

/// The messenger's way to the servers of the project, with its bridges
/// (cloud-api.md §12: "маршрут через мост как у медиа"): a module that has
/// no HTTP client of its own reaches `/cloud/` through it.
pub trait Http: Send + Sync {
    /// Whose servers are in use, and the hosts of the project if theirs.
    fn reach(&self) -> SeamFuture<'_, CloudReach>;
    /// One request to a host [`Http::reach`] names; any other host, or any
    /// host while the servers are not the project's, is `no_host`. A
    /// network failure is `transport`; every answer of a server is `Ok`.
    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse>;
}

/// A box of the lock (veydan-lock's `SecretBox`) for a module that does not
/// depend on the lock: values kept under the vault key, on this device only.
pub trait LockBox: Send + Sync {
    /// The value under `key`; `locked` while the vault is closed.
    fn get<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Option<Zeroizing<Vec<u8>>>>;
    /// Keeps `value` under `key`; `locked` while the vault is closed.
    fn put<'a>(&'a self, key: &'a str, value: &'a [u8]) -> SeamFuture<'a, ()>;
    /// Removes the value under `key`; works while the vault is closed.
    fn delete<'a>(&'a self, key: &'a str) -> SeamFuture<'a, ()>;
}

/// The box `name` of the lock of the app.
struct AppLockBox<R: Runtime> {
    app: tauri::AppHandle<R>,
    name: &'static str,
}

/// The error of the lock in the words of the seams.
fn lock_error(e: veydan_lock::Error) -> CloudError {
    match e {
        veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
            CloudError::new(code::LOCKED, "the lock is closed")
        }
        other => CloudError::new(code::CRYPTO, other.to_string()),
    }
}

fn no_lock() -> CloudError {
    CloudError::new(code::LOCKED, "the app has no lock in place")
}

impl<R: Runtime> LockBox for AppLockBox<R> {
    fn get<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Option<Zeroizing<Vec<u8>>>> {
        Box::pin(async move {
            let lock = self
                .app
                .try_state::<veydan_lock::Lock>()
                .ok_or_else(no_lock)?;
            let value = lock.secret_box(self.name).get(key).await;
            value.map_err(lock_error)
        })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a [u8]) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let lock = self
                .app
                .try_state::<veydan_lock::Lock>()
                .ok_or_else(no_lock)?;
            let done = lock.secret_box(self.name).put(key, value).await;
            done.map_err(lock_error)
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let lock = self
                .app
                .try_state::<veydan_lock::Lock>()
                .ok_or_else(no_lock)?;
            let done = lock.secret_box(self.name).delete(key).await;
            done.map_err(lock_error)
        })
    }
}

/// The box `name` of the lock the shell runs, for a module of `app`.
pub fn lock_box<R: Runtime>(app: &tauri::AppHandle<R>, name: &'static str) -> Arc<dyn LockBox> {
    Arc::new(AppLockBox {
        app: app.clone(),
        name,
    })
}

/// What the lock said to the secret of a dangerous action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockConfirm {
    /// No lock is set on the app: there is nothing to pass.
    NoLock,
    /// The secret is the lock's.
    Confirmed,
    /// A lock is set and the secret is not its own, or none was given.
    Wrong,
}

/// Checks `secret` against the lock of the app without changing anything
/// (veydan-lock's `Lock::verify`): the confirmation of a dangerous action,
/// such as deleting the account of Veydan Cloud (vcloud-plan.md 4.6).
pub async fn lock_confirm<R: Runtime>(
    app: &tauri::AppHandle<R>,
    secret: Option<&str>,
) -> Result<LockConfirm, CloudError> {
    let lock = app.try_state::<veydan_lock::Lock>().ok_or_else(no_lock)?;
    if !lock.status().await.enabled {
        return Ok(LockConfirm::NoLock);
    }
    let Some(secret) = secret.filter(|s| !s.is_empty()) else {
        return Ok(LockConfirm::Wrong);
    };
    match lock.verify(secret).await {
        Ok(()) => Ok(LockConfirm::Confirmed),
        Err(veydan_lock::Error::VaultLocked) => Ok(LockConfirm::NoLock),
        Err(veydan_lock::Error::Db(m)) => Err(CloudError::new(code::CRYPTO, m)),
        Err(_) => Ok(LockConfirm::Wrong),
    }
}

/// How a seam is kept in the state of the app: one per service type.
pub struct Seam<S: ?Sized>(Arc<S>);

/// Put `service` in place for the other modules. Called in the setup of
/// the module that owns it; false when one was in place already.
pub fn provide<S, R, M>(manager: &M, service: Arc<S>) -> bool
where
    S: ?Sized + Send + Sync + 'static,
    R: Runtime,
    M: Manager<R>,
{
    manager.manage(Seam(service))
}

/// The service of a seam, when the module that owns it is in the product.
pub fn seam<S, R, M>(manager: &M) -> Option<Arc<S>>
where
    S: ?Sized + Send + Sync + 'static,
    R: Runtime,
    M: Manager<R>,
{
    manager.try_state::<Seam<S>>().map(|held| held.0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;

    impl Signer for Fixed {
        fn pubkey(&self) -> SeamFuture<'_, String> {
            Box::pin(async { Ok("ab".repeat(32)) })
        }
        fn nip98<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: Option<[u8; 32]>,
        ) -> SeamFuture<'a, WireEvent> {
            Box::pin(async { Err(CloudError::new(code::NOT_SUPPORTED, "fixed")) })
        }
        fn nip44_encrypt<'a>(&'a self, _: &'a str, p: &'a [u8]) -> SeamFuture<'a, Vec<u8>> {
            Box::pin(async move { Ok(p.to_vec()) })
        }
        fn nip44_decrypt<'a>(
            &'a self,
            _: &'a str,
            c: &'a [u8],
        ) -> SeamFuture<'a, Zeroizing<Vec<u8>>> {
            Box::pin(async move { Ok(Zeroizing::new(c.to_vec())) })
        }
    }

    #[test]
    fn a_seam_is_taken_where_it_was_provided_and_once() {
        let app = tauri::test::mock_app();
        assert!(seam::<dyn Signer, _, _>(&app).is_none());
        assert!(provide::<dyn Signer, _, _>(&app, Arc::new(Fixed)));
        assert!(!provide::<dyn Signer, _, _>(&app, Arc::new(Fixed)));
        let signer = seam::<dyn Signer, _, _>(&app).expect("provided");
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        assert_eq!(
            rt.block_on(signer.pubkey()).expect("pubkey"),
            "ab".repeat(32)
        );
        // Another service type is another seam.
        assert!(seam::<dyn PeerNotes, _, _>(&app).is_none());
    }

    #[test]
    fn a_signer_without_the_clock_or_the_derivation_says_so() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let at =
            rt.block_on(Fixed.nip98_at("GET", "https://media.example.com/vault/v1/usage", None, 1));
        assert_eq!(
            at.err().map(|e| e.code).as_deref(),
            Some(code::NOT_SUPPORTED)
        );
        let derived = rt.block_on(Fixed.derive_secret(SYNC_VAULT_PASSPHRASE));
        assert_eq!(
            derived.err().map(|e| e.code).as_deref(),
            Some(code::NOT_SUPPORTED)
        );
        assert_eq!(DERIVE_LABELS, ["sync-vault-passphrase/1"]);
        assert_eq!(DERIVE_SALT, "veydan-cloud");
        let host: StorageHost = serde_json::from_value(serde_json::json!({
            "url": "https://media-eu.example.com/vault", "regions": ["*"], "priority": 0,
        }))
        .expect("host");
        assert_eq!(host.priority, 0);
    }

    #[test]
    fn the_error_names_its_code_and_the_request() {
        let mut e = CloudError::new(code::NO_HOST, "nothing answered");
        assert!(e.is(code::NO_HOST) && !e.is(code::TRANSPORT));
        assert_eq!(e.to_string(), "no_host: nothing answered");
        e.request_id = Some("9c".into());
        assert_eq!(e.to_string(), "no_host: nothing answered (request 9c)");
        let json = serde_json::to_value(&e).expect("json");
        assert_eq!(json["code"], "no_host");
    }

    #[test]
    fn a_temporary_key_is_not_printed() {
        let t = TempKey::new(Zeroizing::new([7; 32]));
        assert_eq!(format!("{t:?}"), "TempKey(..)");
        assert_eq!(t.secret_bytes(), &[7; 32]);
        let (a, b) = (
            TempKey::generate().expect("key"),
            TempKey::generate().expect("key"),
        );
        assert_ne!(a.secret_bytes(), b.secret_bytes());
        assert_ne!(a.secret_bytes(), &[0; 32]);
    }

    #[test]
    fn a_note_and_a_card_print_the_names_of_their_fields_never_the_values() {
        let mut fields = Map::new();
        fields.insert("share".into(), Value::String("U0VDUkVUU0hBUkU=".into()));
        fields.insert("request_id".into(), Value::String("9c".into()));
        let note = Note::new("recovery.answer", fields.clone());
        let printed = format!("{note:?}");
        assert!(
            printed.contains("recovery.answer") && printed.contains("\"share\""),
            "{printed}"
        );
        assert!(
            !printed.contains("U0VDUkVUU0hBUkU=") && !printed.contains("\"9c\""),
            "{printed}"
        );
        let card = Card {
            id: "e1".into(),
            t: "plan.expiring".into(),
            fields,
            at: 1,
        };
        let printed = format!("{card:?}");
        assert!(
            printed.contains("plan.expiring") && !printed.contains("U0VDUkVUU0hBUkU="),
            "{printed}"
        );
    }

    #[test]
    fn a_wire_event_reads_the_json_of_nip01() {
        let json = serde_json::json!({
            "id": "00".repeat(32), "pubkey": "11".repeat(32), "created_at": 1791012345u64,
            "kind": 27235, "tags": [["u", "https://eu-1-media.example.com/cloud/v1/account"], ["method", "GET"]],
            "content": "", "sig": "22".repeat(64),
        });
        let event: WireEvent = serde_json::from_value(json.clone()).expect("event");
        assert_eq!(event.kind, 27235);
        assert_eq!(serde_json::to_value(&event).expect("json"), json);
    }

    #[test]
    fn a_response_finds_its_header_in_any_case_and_the_reach_names_the_servers() {
        let answer = HttpResponse {
            status: 429,
            headers: vec![("retry-after".into(), "30".into())],
            body: Vec::new(),
        };
        assert_eq!(answer.header("Retry-After"), Some("30"));
        assert_eq!(answer.header("x-request-id"), None);
        let reach = CloudReach::Veydan {
            hosts: vec!["https://eu-1-media.example.com".into()],
        };
        let json = serde_json::to_value(&reach).expect("json");
        assert_eq!(json["servers"], "veydan");
        assert_eq!(json["hosts"][0], "https://eu-1-media.example.com");
        let own = serde_json::to_value(CloudReach::Own).expect("json");
        assert_eq!(own, serde_json::json!({ "servers": "own" }));
    }

    /// A box of the lock through the seam: closed before the lock is in
    /// place, a round trip once it is, and the secret of a dangerous action
    /// checked against the lock without opening or closing anything.
    #[tokio::test]
    async fn a_box_of_the_lock_and_its_confirmation_go_through_the_shell() {
        let dir = tempfile::tempdir().expect("dir");
        let db = veydan_core::db::open(
            &dir.path().join(veydan_core::db::DB_FILE),
            &[veydan_core::SCHEMA, crate::lock::SCHEMA],
        )
        .await
        .expect("db");
        let app = tauri::test::mock_app();
        let boxed = lock_box(app.handle(), "cloud");
        assert_eq!(
            boxed.get("account").await.map_err(|e| e.code),
            Err(code::LOCKED.to_string())
        );
        assert_eq!(
            lock_confirm(app.handle(), Some("x"))
                .await
                .map_err(|e| e.code),
            Err(code::LOCKED.to_string())
        );

        let lock = veydan_lock::Lock::with_key_users(db.clone(), Vec::new());
        lock.open_default().await.expect("open");
        app.manage(lock);
        boxed
            .put("account", b"{\"plan\":\"free\"}")
            .await
            .expect("put");
        let kept = boxed.get("account").await.expect("get").expect("kept");
        assert_eq!(kept.as_slice(), b"{\"plan\":\"free\"}");
        assert_eq!(
            lock_box(app.handle(), "other").get("account").await.ok(),
            Some(None)
        );
        assert_eq!(
            lock_confirm(app.handle(), None).await.ok(),
            Some(LockConfirm::NoLock)
        );

        let lock = app.state::<veydan_lock::Lock>();
        lock.set(Some("4821".into()), None, Some("pin".into()), None)
            .await
            .expect("set");
        assert_eq!(
            lock_confirm(app.handle(), Some("4821")).await.ok(),
            Some(LockConfirm::Confirmed)
        );
        assert_eq!(
            lock_confirm(app.handle(), Some("0000")).await.ok(),
            Some(LockConfirm::Wrong)
        );
        assert_eq!(
            lock_confirm(app.handle(), None).await.ok(),
            Some(LockConfirm::Wrong)
        );
        // Nothing was closed by the checks.
        assert!(!lock.is_locked().await);
        boxed.delete("account").await.expect("delete");
        assert_eq!(boxed.get("account").await.ok(), Some(None));
        db.close().await;
    }
}

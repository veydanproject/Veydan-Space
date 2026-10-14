// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The escrow v1 of this device (internal/cloud-recovery.md §4): the copy
//! of the secrets a restore needs — the nsec, the key of the sync vault
//! and the key of the lock — sealed under the Recovery Key and kept in
//! vstore at `escrow/<sha256(auth_pub)>`, outside `vault/<npub>/`.
//!
//! The plaintext is gathered here from the lock (`veydan-lock`), the
//! binding of sync (`veydan-sync-host`) and the messenger's `Signer`
//! (`export_for_escrow`, the one place the nsec leaves the lock); the
//! vault comes with where it lives — Veydan Cloud, or the user's own S3,
//! WebDAV or folder with its access ([`sync_keys`]) — so that a restore
//! sets up the storage the account had; the sealing is `crates/recovery`'s. The object goes to `PUT /vault/v1/escrow/
//! <id>` signed by the identity by the way of the `/vault/` traffic, the
//! seam `VaultHosts` (`send`: the bridges as the media go), and is written
//! again whenever a field changed ([`refresh`]): the fingerprint of the
//! last write and the sha256 of the object are kept in the box `cloud` of
//! the lock, and once a day the object is compared by `HEAD` ([`check`]).
//! Reading it back (`GET`) is signed by the key `auth` derived from the
//! Recovery Key alone (§7).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use veydan_lock::Lock;
use veydan_recovery::escrow as sealed;
use veydan_recovery::{AuthKey, Escrow, LockKey, RecoveryKey, RecoveryParams, Storage, SyncKeys};
use veydan_shell::seams::{
    code, CloudError, HttpRequest, HttpResponse, LockBox, SeamFuture, Signer, TempKey, VaultHosts,
};
use zeroize::Zeroizing;

use crate::api::{now, refusal_of};
use crate::nip98;

/// The box of the lock the module keeps its state in (platform-spec 8.1).
pub const BOX: &str = "cloud";
/// The key of the box: the 16 bytes of the Recovery Key of this account,
/// so that the escrow can be written again when a field changes (§4).
pub const KEY_RK: &str = "recovery.rk";
/// The key of the box: the generation of the Recovery Key, decimal.
pub const KEY_GENERATION: &str = "recovery.generation";
/// The key of the box: the [`EscrowState`] of the last write, JSON.
pub const KEY_ESCROW_STATE: &str = "escrow.state";

/// The words of a refusal of the recovery of the module that no server
/// sends (cloud-api.md §12 names the shape, section 9 the server's words).
pub mod words {
    /// No escrow at the address of these words (`404` of vstore).
    pub const NOT_FOUND: &str = "not_found";
    /// The device is not bound to a sync vault: the escrow has nothing to carry.
    pub const NO_SYNC_VAULT: &str = "no_sync_vault";
    /// The 12 words are not a Recovery Key (count, list or checksum).
    pub const BAD_WORDS: &str = "bad_words";
    /// A Recovery Key is already issued on this device: reissue it instead.
    pub const ALREADY_SET_UP: &str = "already_set_up";
    /// The escrow did not open with the words, or its nsec is not its npub.
    pub const ESCROW_INVALID: &str = "escrow_invalid";
    /// The box holds no escrow of this device, yet the row of the lock carries
    /// the words of a key (version 2) or the server knows a configured recovery:
    /// the Recovery Key was issued on another device, and issuing here would
    /// replace it unseen. Issued anyway only on the owner's explicit word.
    pub const ISSUED_ELSEWHERE: &str = "issued_elsewhere";
    /// A reissue is under way on this device.
    pub const REISSUE_PENDING: &str = "reissue_pending";
    /// A restore needs a clean install: this device is bound to a vault or
    /// has a lock already (§7).
    pub const NOT_CLEAN: &str = "not_clean";
    /// The storage of the vault the escrow or a link names — the user's
    /// own S3, WebDAV or folder — does not answer, or is not on this device,
    /// or does not hold this vault (§7).
    pub const VAULT_UNREACHABLE: &str = "vault_unreachable";
}

/// The path of the escrow under the `public_url` of a host of `storage[]`.
const ESCROW_PATH: &str = "/v1/escrow/";

/// What the box keeps of the last escrow written by this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowState {
    /// `hex sha256(auth_pub)`: the object in vstore.
    pub address: String,
    /// [`fingerprint`] of what was sealed, hex.
    pub fingerprint: String,
    pub generation: u32,
    /// Unix seconds of the write.
    pub written_at: u64,
    /// `hex sha256` of the sealed object as written or read: what the
    /// `ETag` of a `HEAD` is compared with (§4). None from an older build.
    #[serde(default)]
    pub sealed_sha256: Option<String>,
}

/// The object store of the escrow: `/vault/v1/escrow/<id>` of vstore
/// (spec/vstore.md §8), or a fake of it in a test.
pub trait EscrowStore: Send + Sync {
    /// `PUT`, signed by the identity; a second object of the same npub is
    /// allowed for the time of a reissue.
    fn put<'a>(&'a self, address: &'a str, body: &'a [u8]) -> SeamFuture<'a, ()>;
    /// `GET`, signed by the key `auth` of the Recovery Key, no npub in the
    /// request; [`words::NOT_FOUND`] when nothing is at the address.
    fn get<'a>(&'a self, auth: &'a AuthKey) -> SeamFuture<'a, Vec<u8>>;
    /// `HEAD`, signed by the identity: the `ETag` of an object of this npub.
    fn head<'a>(&'a self, address: &'a str) -> SeamFuture<'a, Option<String>>;
    /// `DELETE`, signed by the identity: whether there was one to delete.
    fn delete<'a>(&'a self, address: &'a str) -> SeamFuture<'a, bool>;
}

/// Who signs a request of the store.
enum Auth<'a> {
    Identity,
    Words(&'a AuthKey),
}

/// [`EscrowStore`] over the seams of the messenger: the hosts of
/// `storage[]` and the way to them (`VaultHosts`, the one made for the
/// `/vault/` traffic) and its key (`Signer`). The first host that answers
/// is the one.
pub struct HttpEscrowStore {
    hosts: Arc<dyn VaultHosts>,
    signer: Arc<dyn Signer>,
}

impl HttpEscrowStore {
    pub fn new(hosts: Arc<dyn VaultHosts>, signer: Arc<dyn Signer>) -> Self {
        Self { hosts, signer }
    }

    /// The `Authorization` of one request.
    async fn authorization(
        &self,
        auth: &Auth<'_>,
        method: &str,
        url: &str,
        body: &[u8],
    ) -> Result<String, CloudError> {
        let event = match auth {
            Auth::Identity => {
                self.signer
                    .nip98(method, url, nip98::payload_hash(body))
                    .await?
            }
            Auth::Words(key) => {
                let t = TempKey::new(Zeroizing::new(*key.secret_bytes()));
                nip98::event_by(&t, method, url, nip98::payload_hash(body), now()?)?
            }
        };
        nip98::authorization(&event)
    }

    /// One request to the first host that answers; a host the network does
    /// not reach gives way to the next.
    async fn send(
        &self,
        method: &str,
        address: &str,
        body: Vec<u8>,
        auth: Auth<'_>,
    ) -> Result<HttpResponse, CloudError> {
        let hosts = self.hosts.storage_hosts().await?;
        if hosts.is_empty() {
            return Err(CloudError::new(
                code::NO_HOST,
                "no host of the vault of Veydan Cloud is known on this device",
            ));
        }
        let mut last = None;
        for host in hosts {
            let url = format!("{}{ESCROW_PATH}{address}", host.url.trim_end_matches('/'));
            let authorization = self.authorization(&auth, method, &url, &body).await?;
            let mut headers = vec![("Authorization".to_string(), authorization)];
            if !body.is_empty() {
                headers.push((
                    "Content-Type".to_string(),
                    "application/octet-stream".to_string(),
                ));
            }
            let request = HttpRequest {
                method: method.to_string(),
                url,
                headers,
                body: body.clone(),
            };
            match self.hosts.send(request).await {
                Ok(answer) => return Ok(answer),
                Err(e) if e.is(code::TRANSPORT) => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| CloudError::new(code::TRANSPORT, "no host answered")))
    }
}

/// A refusal of vstore in the words of the seams; `404` is [`words::NOT_FOUND`].
fn refused(answer: &HttpResponse) -> CloudError {
    if answer.status == 404 {
        return CloudError::new(words::NOT_FOUND, "nothing is at this address");
    }
    refusal_of(answer).error
}

impl EscrowStore for HttpEscrowStore {
    fn put<'a>(&'a self, address: &'a str, body: &'a [u8]) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let answer = self
                .send("PUT", address, body.to_vec(), Auth::Identity)
                .await?;
            match answer.status {
                200 | 201 => Ok(()),
                _ => Err(refused(&answer)),
            }
        })
    }

    fn get<'a>(&'a self, auth: &'a AuthKey) -> SeamFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let address = auth.escrow_address();
            let answer = self
                .send("GET", &address, Vec::new(), Auth::Words(auth))
                .await?;
            match answer.status {
                200 => Ok(answer.body),
                _ => Err(refused(&answer)),
            }
        })
    }

    fn head<'a>(&'a self, address: &'a str) -> SeamFuture<'a, Option<String>> {
        Box::pin(async move {
            let answer = self
                .send("HEAD", address, Vec::new(), Auth::Identity)
                .await?;
            match answer.status {
                200 => Ok(Some(answer.header("etag").unwrap_or_default().to_string())),
                404 => Ok(None),
                _ => Err(refused(&answer)),
            }
        })
    }

    fn delete<'a>(&'a self, address: &'a str) -> SeamFuture<'a, bool> {
        Box::pin(async move {
            let answer = self
                .send("DELETE", address, Vec::new(), Auth::Identity)
                .await?;
            match answer.status {
                200 | 204 => Ok(true),
                404 => Ok(false),
                _ => Err(refused(&answer)),
            }
        })
    }
}

/// Where the fields of the escrow come from on this device.
pub struct Sources<'a> {
    pub lock: &'a Lock,
    pub db: &'a Pool<Sqlite>,
    pub signer: &'a dyn Signer,
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// Where the vault of sync of this device lives, as its settings of sync
/// say, with the access a clean device needs to reach it (§4).
pub async fn storage_of(db: &Pool<Sqlite>) -> Result<Storage, CloudError> {
    let cfg = veydan_sync_host::config::load_config(db).await;
    match cfg.backend.as_str() {
        veydan_sync_host::config::CLOUD_BACKEND => Ok(Storage::Cloud),
        "s3" => Ok(Storage::S3 {
            endpoint: cfg.s3.endpoint,
            region: cfg.s3.region,
            bucket: cfg.s3.bucket,
            prefix: cfg.s3.prefix,
            access_key: cfg.s3.access_key,
            secret_key: cfg.s3.secret_key.unwrap_or_default(),
            path_style: cfg.s3.path_style,
        }),
        "webdav" => Ok(Storage::WebDav {
            url: cfg.webdav.url,
            username: cfg.webdav.username,
            password: cfg.webdav.password.unwrap_or_default(),
        }),
        "folder" => Ok(Storage::Folder {
            path: cfg.folder_path,
        }),
        other => Err(crypto(format!("unknown sync backend {other}"))),
    }
}

/// The settings of sync `cfg` take the storage of a vault: the inverse of
/// [`storage_of`], on the device that restores or is linked.
pub fn apply_storage(cfg: &mut veydan_sync_host::SyncConfig, storage: &Storage) {
    match storage {
        Storage::Cloud => cfg.backend = veydan_sync_host::config::CLOUD_BACKEND.into(),
        Storage::S3 {
            endpoint,
            region,
            bucket,
            prefix,
            access_key,
            secret_key,
            path_style,
        } => {
            cfg.backend = "s3".into();
            cfg.s3.endpoint = endpoint.clone();
            cfg.s3.region = region.clone();
            cfg.s3.bucket = bucket.clone();
            cfg.s3.prefix = prefix.clone();
            cfg.s3.access_key = access_key.clone();
            cfg.s3.secret_key = Some(secret_key.clone());
            cfg.s3.has_secret_key = !secret_key.is_empty();
            cfg.s3.path_style = *path_style;
        }
        Storage::WebDav {
            url,
            username,
            password,
        } => {
            cfg.backend = "webdav".into();
            cfg.webdav.url = url.clone();
            cfg.webdav.username = username.clone();
            cfg.webdav.password = Some(password.clone());
            cfg.webdav.has_password = !password.is_empty();
        }
        Storage::Folder { path } => {
            cfg.backend = "folder".into();
            cfg.folder_path = path.clone();
        }
    }
}

/// The vault of sync this device is bound to — its id, its key and its
/// storage — or `None` without a binding.
pub async fn sync_keys(db: &Pool<Sqlite>) -> Result<Option<SyncKeys>, CloudError> {
    let Some(binding) = veydan_sync_host::config::load_binding(db).await else {
        return Ok(None);
    };
    let vmk = veydan_sync::Vmk::from_base64(&binding.vmk_b64)
        .map_err(|_| crypto("the key of the sync vault kept on this device is not 32 bytes"))?;
    Ok(Some(SyncKeys {
        vault_id: binding.vault_id,
        vmk: vmk.0,
        storage: storage_of(db).await?,
    }))
}

/// The plaintext of the escrow as this device stands now (§4): the nsec
/// through the seam, the vault of sync from the binding and the settings
/// of sync, the key of the lock from the open vault (`None` while no lock
/// was ever set), the numbers of the trusted people as given. Fails
/// `locked` while the lock is closed and [`words::NO_SYNC_VAULT`] without
/// a binding.
pub async fn gather(
    sources: &Sources<'_>,
    generation: u32,
    recovery: RecoveryParams,
) -> Result<Escrow, CloudError> {
    let nsec = sources.signer.export_for_escrow().await?;
    let npub = veydan_recovery::rk::npub_of(&nsec)
        .ok_or_else(|| crypto("the nsec of the identity is not a key"))?;
    let sync = sync_keys(sources.db).await?.ok_or_else(|| {
        CloudError::new(
            words::NO_SYNC_VAULT,
            "this device is not bound to a sync vault",
        )
    })?;
    let lock = if sources.lock.enabled().await {
        let (key, vault_id) = sources.lock.require_open().map_err(|e| match e {
            veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
                CloudError::new(code::LOCKED, "the lock is closed")
            }
            other => crypto(other),
        })?;
        Some(LockKey {
            vault_id,
            vault_key: *key.to_bytes(),
        })
    } else {
        None
    };
    Ok(Escrow {
        npub,
        created_at: now()?,
        generation,
        nsec: *nsec,
        sync,
        lock,
        recovery,
    })
}

/// What tells one escrow from another but the time of its write: the
/// sha256 of the plaintext with `created_at` zeroed, hex. The same
/// fields twice give the same fingerprint, so a write is made only when a
/// field changed (§4).
pub fn fingerprint(escrow: &Escrow) -> String {
    let mut stamped = escrow.clone();
    stamped.created_at = 0;
    nip98::hex(&Sha256::digest(stamped.to_cbor().as_slice()))
}

/// `seal` of crates/recovery in a blocking thread: Argon2id of 64 MiB.
pub async fn seal(rk: &RecoveryKey, escrow: &Escrow) -> Result<Vec<u8>, CloudError> {
    let rk = rk.clone();
    let escrow = escrow.clone();
    tokio::task::spawn_blocking(move || sealed::seal(&rk, &escrow))
        .await
        .map_err(crypto)?
        .map_err(crypto)
}

/// `open` of crates/recovery in a blocking thread; a wrong key, a wrong
/// address and a changed byte all end in [`words::ESCROW_INVALID`].
pub async fn open(rk: &RecoveryKey, bytes: Vec<u8>) -> Result<Escrow, CloudError> {
    let rk = rk.clone();
    tokio::task::spawn_blocking(move || sealed::open(&rk, &bytes))
        .await
        .map_err(crypto)?
        .map_err(|e| CloudError::new(words::ESCROW_INVALID, e.to_string()))
}

/// The state of the last write, from the box.
pub async fn load_state(boxed: &dyn LockBox) -> Result<Option<EscrowState>, CloudError> {
    let Some(bytes) = boxed.get(KEY_ESCROW_STATE).await? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| crypto(format!("the state of the escrow in the box: {e}")))
}

/// Keeps the state of a write in the box.
pub async fn save_state(boxed: &dyn LockBox, state: &EscrowState) -> Result<(), CloudError> {
    let json = serde_json::to_vec(state).map_err(crypto)?;
    boxed.put(KEY_ESCROW_STATE, &json).await
}

/// The Recovery Key of this account, from the box; `None` before the
/// cloud's recovery was set up on this device.
pub async fn load_rk(boxed: &dyn LockBox) -> Result<Option<RecoveryKey>, CloudError> {
    let Some(bytes) = boxed.get(KEY_RK).await? else {
        return Ok(None);
    };
    let mut entropy = Zeroizing::new([0u8; 16]);
    if bytes.len() != entropy.len() {
        return Err(crypto("the recovery key in the box is not 16 bytes"));
    }
    entropy.copy_from_slice(&bytes);
    RecoveryKey::from_bytes(*entropy).map(Some).map_err(crypto)
}

/// Keeps the Recovery Key and its generation in the box.
pub async fn save_rk(boxed: &dyn LockBox, rk: &RecoveryKey, generation: u32) -> Result<(), CloudError> {
    boxed.put(KEY_RK, rk.as_bytes()).await?;
    boxed
        .put(KEY_GENERATION, generation.to_string().as_bytes())
        .await
}

/// The generation of the Recovery Key kept on this device; 0 before any.
pub async fn load_generation(boxed: &dyn LockBox) -> Result<u32, CloudError> {
    let Some(bytes) = boxed.get(KEY_GENERATION).await? else {
        return Ok(0);
    };
    std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| crypto("the generation in the box is not a number"))
}

/// `hex sha256` of a sealed object: the `ETag` vstore gives it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    nip98::hex(&Sha256::digest(bytes))
}

/// The address of the escrow of `rk`.
fn address_of(rk: &RecoveryKey) -> Result<String, CloudError> {
    Ok(rk.escrow_auth().map_err(crypto)?.escrow_address())
}

/// Seals and writes the escrow at the address of `rk`, whatever the box
/// says, and keeps the state of the write there.
pub async fn upload(
    store: &dyn EscrowStore,
    boxed: &dyn LockBox,
    rk: &RecoveryKey,
    escrow: &Escrow,
) -> Result<(), CloudError> {
    let address = address_of(rk)?;
    let body = seal(rk, escrow).await?;
    store.put(&address, &body).await?;
    save_state(
        boxed,
        &EscrowState {
            address,
            fingerprint: fingerprint(escrow),
            generation: escrow.generation,
            written_at: escrow.created_at,
            sealed_sha256: Some(sha256_hex(&body)),
        },
    )
    .await
}

/// Writes the escrow when its fields changed since the last write by this
/// device, or when it was never written: `true` when a `PUT` was made. The
/// state of the write goes to the box.
pub async fn ensure_uploaded(
    store: &dyn EscrowStore,
    boxed: &dyn LockBox,
    rk: &RecoveryKey,
    escrow: &Escrow,
) -> Result<bool, CloudError> {
    let address = address_of(rk)?;
    let print = fingerprint(escrow);
    if let Some(state) = load_state(boxed).await? {
        if state.address == address && state.fingerprint == print {
            return Ok(false);
        }
    }
    upload(store, boxed, rk, escrow).await?;
    Ok(true)
}

/// The plaintext of the escrow as this device stands, under the generation
/// the box knows; the numbers of the trusted people of this device
/// (social.rs), 0 of 0 without any.
pub async fn current(sources: &Sources<'_>, boxed: &dyn LockBox) -> Result<Escrow, CloudError> {
    let generation = load_generation(boxed).await?;
    let recovery = crate::social::recovery_params(boxed, generation).await?;
    gather(sources, generation, recovery).await
}

/// §4, «перезаписывается при любом изменении поля»: writes the escrow again
/// when a field of this device changed since the last write — the lock
/// was set, changed or reset, the binding of sync changed — and, with
/// `force`, when the object in vstore was found to differ. Nothing while
/// the recovery is not set up on this device. `true` when a `PUT` was made.
pub async fn refresh(
    sources: &Sources<'_>,
    store: &dyn EscrowStore,
    boxed: &dyn LockBox,
    force: bool,
) -> Result<bool, CloudError> {
    let Some(rk) = load_rk(boxed).await? else {
        return Ok(false);
    };
    let escrow = current(sources, boxed).await?;
    if force {
        upload(store, boxed, &rk, &escrow).await?;
        return Ok(true);
    }
    ensure_uploaded(store, boxed, &rk, &escrow).await
}

/// Whether the object in vstore is not the one this device wrote last: gone
/// (`HEAD` 404) or with another sha256. Not comparable without the sha256
/// of the last write (a state of an older build): false.
pub async fn behind(store: &dyn EscrowStore, state: &EscrowState) -> Result<bool, CloudError> {
    let Some(known) = &state.sealed_sha256 else {
        return Ok(false);
    };
    Ok(match store.head(&state.address).await? {
        None => true,
        Some(etag) => etag.trim().trim_matches('"') != known,
    })
}

/// The daily check of §4: the object in vstore is compared by `HEAD` with
/// the last write of this device and written again when it fell behind.
/// `true` when a `PUT` was made; nothing without a write to compare with.
pub async fn check(
    sources: &Sources<'_>,
    store: &dyn EscrowStore,
    boxed: &dyn LockBox,
) -> Result<bool, CloudError> {
    let Some(state) = load_state(boxed).await? else {
        return Ok(false);
    };
    if load_rk(boxed).await?.is_none() || !behind(store, &state).await? {
        return Ok(false);
    }
    refresh(sources, store, boxed, true).await
}

#[cfg(test)]
pub(crate) mod testing {
    //! A fake of `/vault/v1/escrow/` behind the `VaultHosts` seam, and a
    //! box in memory: what the store and the flows are tested against
    //! without a network.

    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use veydan_shell::seams::{StorageHost, WireEvent};

    /// vstore's escrow table, in memory: `PUT`/`HEAD`/`DELETE` by the
    /// identity, `GET` by the key whose sha256 is the address (vstore.md §8).
    #[derive(Default)]
    pub struct FakeVstore {
        pub objects: Mutex<HashMap<String, Vec<u8>>>,
        pub requests: Mutex<Vec<(String, String)>>,
        pub identity: Mutex<Option<String>>,
        /// Every `DELETE` answers `502 storage`, as a host whose bucket is away.
        pub fail_delete: Mutex<bool>,
    }

    pub const HOST: &str = "https://media-eu.example.com";

    fn event_of(request: &HttpRequest) -> Option<WireEvent> {
        let auth = request
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))?;
        let encoded = auth.1.strip_prefix("Nostr ")?;
        let json = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()?;
        serde_json::from_slice(&json).ok()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .filter_map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
            .collect()
    }

    impl FakeVstore {
        fn answer(&self, request: &HttpRequest) -> HttpResponse {
            let path = request.url.strip_prefix(HOST).unwrap_or_default();
            let id = path
                .strip_prefix("/vault/v1/escrow/")
                .unwrap_or_default()
                .to_string();
            self.requests
                .lock()
                .expect("requests")
                .push((request.method.clone(), id.clone()));
            let Some(event) = event_of(request) else {
                return response(401, b"{\"error\":{\"code\":\"auth_invalid\"}}".to_vec());
            };
            if event.tags.first().map(|t| t[1].as_str()) != Some(request.url.as_str()) {
                return response(401, b"{\"error\":{\"code\":\"auth_invalid\"}}".to_vec());
            }
            let mut objects = self.objects.lock().expect("objects");
            match request.method.as_str() {
                "GET" => {
                    let of_key = nip98::hex(&Sha256::digest(unhex(&event.pubkey)));
                    match objects.get(&id) {
                        Some(body) if of_key == id => response(200, body.clone()),
                        _ => response(404, b"{\"error\":{\"code\":\"not_found\"}}".to_vec()),
                    }
                }
                method => {
                    let identity = self.identity.lock().expect("identity");
                    if identity.as_deref() != Some(event.pubkey.as_str()) {
                        return response(403, b"{\"error\":{\"code\":\"no_account\"}}".to_vec());
                    }
                    match method {
                        "PUT" => {
                            let created = objects.insert(id, request.body.clone()).is_none();
                            response(if created { 201 } else { 200 }, b"{}".to_vec())
                        }
                        "HEAD" => match objects.get(&id) {
                            Some(body) => {
                                // vstore.md §8: a strong ETag, the sha256 in quotes.
                                let mut answer = response(200, Vec::new());
                                answer
                                    .headers
                                    .push(("etag".into(), format!("\"{}\"", sha256_hex(body))));
                                answer
                            }
                            None => response(404, Vec::new()),
                        },
                        "DELETE" if *self.fail_delete.lock().expect("fail_delete") => {
                            response(502, b"{\"error\":{\"code\":\"storage\"}}".to_vec())
                        }
                        "DELETE" => match objects.remove(&id) {
                            Some(_) => response(204, Vec::new()),
                            None => response(404, b"{\"error\":{\"code\":\"not_found\"}}".to_vec()),
                        },
                        _ => response(405, Vec::new()),
                    }
                }
            }
        }
    }

    fn response(status: u16, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            headers: Vec::new(),
            body,
        }
    }

    impl VaultHosts for FakeVstore {
        fn storage_hosts(&self) -> SeamFuture<'_, Vec<StorageHost>> {
            Box::pin(async {
                Ok(vec![StorageHost {
                    url: format!("{HOST}/vault"),
                    regions: vec!["eu".into()],
                    priority: 1,
                }])
            })
        }
        fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
            Box::pin(async move { Ok(self.answer(&request)) })
        }
    }

    /// A box in memory, open or closed; a `put` of one key can be made to
    /// fail once (`fail_put`), the way a data file does.
    #[derive(Default)]
    pub struct MemoryBox {
        pub values: Mutex<HashMap<String, Vec<u8>>>,
        pub locked: Mutex<bool>,
        pub fail_put: Mutex<Option<String>>,
    }

    impl MemoryBox {
        fn check(&self) -> Result<(), CloudError> {
            if *self.locked.lock().expect("locked") {
                return Err(CloudError::new(code::LOCKED, "the lock is closed"));
            }
            Ok(())
        }
    }

    impl LockBox for MemoryBox {
        fn get<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Option<Zeroizing<Vec<u8>>>> {
            Box::pin(async move {
                self.check()?;
                Ok(self
                    .values
                    .lock()
                    .expect("values")
                    .get(key)
                    .cloned()
                    .map(Zeroizing::new))
            })
        }
        fn put<'a>(&'a self, key: &'a str, value: &'a [u8]) -> SeamFuture<'a, ()> {
            Box::pin(async move {
                self.check()?;
                let mut fail = self.fail_put.lock().expect("fail_put");
                if fail.as_deref() == Some(key) {
                    *fail = None;
                    return Err(CloudError::new(code::STORAGE, "the data file is busy"));
                }
                self.values
                    .lock()
                    .expect("values")
                    .insert(key.to_string(), value.to_vec());
                Ok(())
            })
        }
        fn delete<'a>(&'a self, key: &'a str) -> SeamFuture<'a, ()> {
            Box::pin(async move {
                self.values.lock().expect("values").remove(key);
                Ok(())
            })
        }
    }

    /// A signer over a plain nsec, for the fakes: what `MemorySigner` of
    /// the restore is.
    pub fn signer(nsec: [u8; 32]) -> Arc<dyn Signer> {
        Arc::new(crate::restore::MemorySigner::new(Zeroizing::new(nsec)).expect("key"))
    }

    /// An escrow with fixed fields.
    pub fn escrow(nsec: [u8; 32], generation: u32) -> Escrow {
        Escrow {
            npub: veydan_recovery::rk::npub_of(&nsec).expect("npub"),
            created_at: 1_791_012_345,
            generation,
            nsec,
            sync: SyncKeys::cloud("3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c", [0x77; 32]),
            lock: Some(LockKey {
                vault_id: "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e".into(),
                vault_key: [0x99; 32],
            }),
            recovery: RecoveryParams {
                k: 0,
                n: 0,
                generation,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{escrow, signer, FakeVstore, MemoryBox};
    use super::*;

    const NSEC: [u8; 32] = [0x42; 32];
    const RK: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];

    async fn store() -> (Arc<FakeVstore>, HttpEscrowStore) {
        let vstore = Arc::new(FakeVstore::default());
        let identity = signer(NSEC);
        *vstore.identity.lock().expect("identity") = Some(identity.pubkey().await.expect("pubkey"));
        let store = HttpEscrowStore::new(vstore.clone(), identity);
        (vstore, store)
    }

    /// The daily check of §4: the object as written passes `HEAD`; one
    /// another device rewrote, or one that is gone, is written again from
    /// the state of this device; a state of an older build is not compared.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_daily_check_writes_again_what_fell_behind() {
        let (vstore, store) = store().await;
        let boxed = MemoryBox::default();
        let rk = RecoveryKey::from_bytes(RK).expect("rk");
        let first = escrow(NSEC, 1);
        upload(&store, &boxed, &rk, &first).await.expect("upload");
        let state = load_state(&boxed).await.expect("state").expect("written");
        assert_eq!(state.sealed_sha256.as_deref().map(str::len), Some(64));
        assert!(!behind(&store, &state).await.expect("head"));

        vstore
            .objects
            .lock()
            .expect("objects")
            .insert(state.address.clone(), b"another device wrote".to_vec());
        assert!(behind(&store, &state).await.expect("head"));
        vstore.objects.lock().expect("objects").clear();
        assert!(behind(&store, &state).await.expect("head"));
        let old_build = EscrowState {
            sealed_sha256: None,
            ..state.clone()
        };
        assert!(!behind(&store, &old_build).await.expect("head"));
    }

    #[test]
    fn the_fingerprint_ignores_the_time_of_the_write_and_nothing_else() {
        let a = escrow(NSEC, 3);
        let mut later = a.clone();
        later.created_at += 60;
        assert_eq!(fingerprint(&a), fingerprint(&later));
        let mut other = a.clone();
        other.generation = 4;
        assert_ne!(fingerprint(&a), fingerprint(&other));
        let mut unlocked = a.clone();
        unlocked.lock = None;
        assert_ne!(fingerprint(&a), fingerprint(&unlocked));
        assert_eq!(fingerprint(&a).len(), 64);
    }

    /// The store speaks to vstore as §8 of its contract says: `PUT`, `HEAD`
    /// and `DELETE` by the identity at the address of the key `auth`,
    /// `GET` by `auth` alone, and `404` is `not_found`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_store_writes_by_the_identity_and_reads_by_the_words() {
        let (vstore, store) = store().await;
        let rk = RecoveryKey::from_bytes(RK).expect("rk");
        let auth = rk.escrow_auth().expect("auth");
        let address = auth.escrow_address();

        assert_eq!(
            store.get(&auth).await.err().map(|e| e.code).as_deref(),
            Some(words::NOT_FOUND)
        );
        assert_eq!(store.head(&address).await.expect("head"), None);
        assert!(!store.delete(&address).await.expect("delete"));

        store.put(&address, b"sealed").await.expect("put");
        assert_eq!(store.get(&auth).await.expect("get"), b"sealed");
        assert!(store.head(&address).await.expect("head").is_some());
        // Another key reads nothing: the address is not its.
        let other = RecoveryKey::generate().expect("rk");
        assert_eq!(
            store
                .get(&other.escrow_auth().expect("auth"))
                .await
                .err()
                .map(|e| e.code)
                .as_deref(),
            Some(words::NOT_FOUND)
        );
        assert!(store.delete(&address).await.expect("delete"));
        assert_eq!(
            store.get(&auth).await.err().map(|e| e.code).as_deref(),
            Some(words::NOT_FOUND)
        );
        let requests = vstore.requests.lock().expect("requests");
        assert_eq!(requests[0], ("GET".to_string(), address.clone()));
        assert_eq!(requests[3], ("PUT".to_string(), address.clone()));
        assert!(requests.iter().all(|(_, id)| id.len() == 64));
    }

    /// Sealed, uploaded, remembered: the same fields are not written twice,
    /// a changed field is, and what was written opens with the words.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_escrow_is_written_when_a_field_changed_and_opens_with_the_words() {
        let (vstore, store) = store().await;
        let boxed = MemoryBox::default();
        let rk = RecoveryKey::from_bytes(RK).expect("rk");
        let first = escrow(NSEC, 1);

        assert!(ensure_uploaded(&store, &boxed, &rk, &first).await.expect("upload"));
        let state = load_state(&boxed).await.expect("state").expect("written");
        assert_eq!(state.address, rk.escrow_auth().expect("auth").escrow_address());
        assert_eq!(state.generation, 1);
        let mut same = first.clone();
        same.created_at += 1;
        assert!(!ensure_uploaded(&store, &boxed, &rk, &same).await.expect("upload"));
        let mut changed = first.clone();
        changed.lock = None;
        assert!(ensure_uploaded(&store, &boxed, &rk, &changed).await.expect("upload"));
        let puts = vstore
            .requests
            .lock()
            .expect("requests")
            .iter()
            .filter(|(m, _)| m == "PUT")
            .count();
        assert_eq!(puts, 2);

        let bytes = store.get(&rk.escrow_auth().expect("auth")).await.expect("get");
        let opened = open(&rk, bytes).await.expect("open");
        assert_eq!(opened, changed);
        let wrong = RecoveryKey::generate().expect("rk");
        let bytes = store.get(&rk.escrow_auth().expect("auth")).await.expect("get");
        assert_eq!(
            open(&wrong, bytes).await.err().map(|e| e.code).as_deref(),
            Some(words::ESCROW_INVALID)
        );
    }

    #[tokio::test]
    async fn the_box_keeps_the_key_and_its_generation() {
        let boxed = MemoryBox::default();
        assert!(load_rk(&boxed).await.expect("rk").is_none());
        assert_eq!(load_generation(&boxed).await.expect("generation"), 0);
        let rk = RecoveryKey::from_bytes(RK).expect("rk");
        save_rk(&boxed, &rk, 3).await.expect("save");
        assert_eq!(load_rk(&boxed).await.expect("rk").expect("kept"), rk);
        assert_eq!(load_generation(&boxed).await.expect("generation"), 3);
        *boxed.locked.lock().expect("locked") = true;
        assert_eq!(
            load_rk(&boxed).await.err().map(|e| e.code).as_deref(),
            Some(code::LOCKED)
        );
    }
}

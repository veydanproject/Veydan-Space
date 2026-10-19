// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Scenario 3 of internal/cloud-recovery.md §7 on two data files: the vault
//! of a device A whose lock is behind a password nobody remembers, a fake
//! vstore on loopback (axum) holding the escrow, and a clean device B with
//! the 12 words and an identity of the messenger over the real lock — a
//! fake without relays that keeps its nsec where `HostSecretStore` of
//! messenger-app keeps it (the box `messenger`, key `identity.nsec`) and
//! answers as it answers (`SecretsLocked` while the vault is closed or does
//! not open); the module `cloud` depends on no crate of the messenger, in
//! its tests either (platform-spec 6.1).
//!
//! The six steps in their order leave the messenger with its key; the order
//! the contract forbids (the identity before the key row of the vault)
//! leaves it in `SecretsLocked` — the failure the contract describes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use base64::Engine as _;
use secp256k1::schnorr::Signature;
use secp256k1::{Secp256k1, XOnlyPublicKey};
use serde_json::Value;
use sha2::{Digest, Sha256};
use veydan_cloud::escrow::{self, EscrowStore, HttpEscrowStore};
use veydan_cloud::restore::{
    self, adopt_and_bind, LockOutcome, MemorySigner, NewSecret, Phase, RestoreEnv, Vault,
};
use veydan_core::{db, BoxFuture, Core, Directory, Schema};
use veydan_lock::Lock;
use veydan_recovery::{Escrow, LockKey, RecoveryKey, RecoveryParams, Storage, SyncKeys};
use veydan_shell::seams::{
    code, CloudError, CloudReach, Http, HttpRequest, HttpResponse, LockBox, SeamFuture, Signer,
    StorageHost, VaultHosts,
};
use veydan_sync::{Engine, Hlc, LocalDir, LocalState, Op, Vmk};
use veydan_sync_host::{Host, Plan, Registry, States, Step};
use zeroize::Zeroizing;

const LOCK: Schema = Schema {
    module: "lock",
    steps: veydan_lock::SCHEMA_STEPS,
};
const ROWS: &[&str] = &["password_vault", "setting", "label"];
const PLAN: Plan = Plan {
    collect: &[Step::rows("rows", None, "app", ROWS)],
    apply: &[Step::rows("rows", None, "app", ROWS)],
    late: &[],
};
const NSEC: [u8; 32] = [0x42; 32];
const FORGOTTEN: &str = "the password nobody remembers";
const NEW_PIN: &str = "7777";

// ── The fake vstore ──────────────────────────────────────────────────────────

#[derive(Default)]
struct Objects {
    escrow: Mutex<HashMap<String, Vec<u8>>>,
    /// Requests seen, `(method, id)`.
    requests: Mutex<Vec<(String, String)>>,
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The pubkey of a NIP-98 header whose event verifies (NIP-01 id, Schnorr).
fn signer_of(headers: &HeaderMap, method: &str, url_path: &str) -> Option<String> {
    let header = headers.get("authorization")?.to_str().ok()?;
    let encoded = header.strip_prefix("Nostr ")?;
    let json = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let event: Value = serde_json::from_slice(&json).ok()?;
    let pubkey = event["pubkey"].as_str()?;
    let tags = event["tags"].as_array()?;
    let tag = |name: &str| {
        tags.iter()
            .find(|t| t[0] == name)
            .and_then(|t| t[1].as_str())
    };
    if tag("method")? != method || !tag("u")?.ends_with(url_path) || event["kind"] != 27235 {
        return None;
    }
    let serial = serde_json::json!([0, pubkey, event["created_at"], 27235, tags, ""]);
    let id: [u8; 32] = Sha256::digest(serial.to_string().as_bytes()).into();
    if unhex(event["id"].as_str()?) != id {
        return None;
    }
    let key = XOnlyPublicKey::from_slice(&unhex(pubkey)).ok()?;
    let sig = Signature::from_slice(&unhex(event["sig"].as_str()?)).ok()?;
    Secp256k1::verification_only()
        .verify_schnorr(&sig, &id, &key)
        .ok()?;
    Some(pubkey.to_string())
}

fn error(status: StatusCode, word: &str) -> Response {
    (status, format!("{{\"error\":{{\"code\":\"{word}\"}}}}")).into_response()
}

async fn escrow_get(
    State(objects): State<Arc<Objects>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    objects.requests.lock().unwrap().push(("GET".into(), id.clone()));
    let Some(pubkey) = signer_of(&headers, "GET", &format!("/vault/v1/escrow/{id}")) else {
        return error(StatusCode::UNAUTHORIZED, "auth_invalid");
    };
    // vstore.md §8: the signer is not looked up; sha256(signer) is the id.
    let of_key: String = Sha256::digest(unhex(&pubkey))
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if of_key != id {
        return error(StatusCode::NOT_FOUND, "not_found");
    }
    match objects.escrow.lock().unwrap().get(&id) {
        Some(body) => (StatusCode::OK, body.clone()).into_response(),
        None => error(StatusCode::NOT_FOUND, "not_found"),
    }
}

async fn escrow_put(
    State(objects): State<Arc<Objects>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    objects.requests.lock().unwrap().push(("PUT".into(), id.clone()));
    if signer_of(&headers, "PUT", &format!("/vault/v1/escrow/{id}")).is_none() {
        return error(StatusCode::UNAUTHORIZED, "auth_invalid");
    }
    if body.len() > 4096 {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "too_large");
    }
    let created = objects.escrow.lock().unwrap().insert(id, body.to_vec()).is_none();
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    (status, "{}").into_response()
}

async fn escrow_delete(
    State(objects): State<Arc<Objects>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    objects.requests.lock().unwrap().push(("DELETE".into(), id.clone()));
    if signer_of(&headers, "DELETE", &format!("/vault/v1/escrow/{id}")).is_none() {
        return error(StatusCode::UNAUTHORIZED, "auth_invalid");
    }
    match objects.escrow.lock().unwrap().remove(&id) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => error(StatusCode::NOT_FOUND, "not_found"),
    }
}

/// vstore on loopback: `(base url, its objects)`.
async fn serve_vstore() -> (String, Arc<Objects>) {
    let objects = Arc::new(Objects::default());
    let app = Router::new()
        .route(
            "/vault/v1/escrow/{id}",
            get(escrow_get).put(escrow_put).delete(escrow_delete),
        )
        .with_state(objects.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (base, objects)
}

/// The seams `Http` and `VaultHosts` of a device whose project is the fake.
struct Loopback {
    base: String,
    client: reqwest::Client,
}

impl Http for Loopback {
    fn reach(&self) -> SeamFuture<'_, CloudReach> {
        Box::pin(async move {
            Ok(CloudReach::Veydan {
                hosts: vec![self.base.clone()],
            })
        })
    }

    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        Box::pin(async move {
            let method = reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|e| CloudError::new(code::BAD_REQUEST, e.to_string()))?;
            let mut builder = self.client.request(method, &request.url);
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            let response = builder
                .body(request.body)
                .send()
                .await
                .map_err(|e| CloudError::new(code::TRANSPORT, e.to_string()))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(n, v)| Some((n.as_str().to_string(), v.to_str().ok()?.to_string())))
                .collect();
            let body = response
                .bytes()
                .await
                .map_err(|e| CloudError::new(code::TRANSPORT, e.to_string()))?
                .to_vec();
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

impl VaultHosts for Loopback {
    fn storage_hosts(&self) -> SeamFuture<'_, Vec<StorageHost>> {
        Box::pin(async move {
            Ok(vec![StorageHost {
                url: format!("{}/vault", self.base),
                regions: vec!["eu".into()],
                priority: 1,
            }])
        })
    }
    fn send(&self, request: HttpRequest) -> SeamFuture<'_, HttpResponse> {
        <Self as Http>::send(self, request)
    }
}

// ── The devices ──────────────────────────────────────────────────────────────

/// The lock of a device, where the hooks of sync find it.
struct Locked(Lock);

impl States for Locked {
    fn state(&self, ty: std::any::TypeId) -> Option<&(dyn std::any::Any + Send + Sync)> {
        (ty == std::any::TypeId::of::<Lock>()).then_some(&self.0 as _)
    }
}

/// What `IdentityService` of the messenger answers (`MessengerError`), as
/// far as the restore meets it.
#[derive(Debug, PartialEq, Eq)]
enum IdentityError {
    /// The host is locked: the box of the lock does not open.
    SecretsLocked,
    /// No identity on this device.
    NotLoggedIn,
    /// An identity exists already.
    Exists,
    Storage(String),
    Crypto(String),
}

/// `HostSecretStore::locked` of messenger-app: the words of the lock in the
/// messenger's.
fn locked(e: veydan_lock::Error) -> IdentityError {
    match e {
        veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
            IdentityError::SecretsLocked
        }
        veydan_lock::Error::Db(m) => IdentityError::Storage(m),
        other => IdentityError::Crypto(other.to_string()),
    }
}

/// The identity of the messenger without relays: `IdentityService` over
/// `HostSecretStore` — the nsec in the box `messenger` of the lock under
/// `identity.nsec`, the public half in a table of its own (here, a field).
struct FakeIdentity {
    lock: Arc<Locked>,
    npub: Mutex<Option<String>>,
}

impl FakeIdentity {
    const SECRET_KEY_REF: &'static str = "identity.nsec";

    /// `IdentityService::import_nsec`: refused while an identity exists or
    /// the host is locked; the secret goes into the box, the row after it.
    async fn import_nsec(&self, hex: &str) -> Result<(), IdentityError> {
        if self.npub.lock().unwrap().is_some() {
            return Err(IdentityError::Exists);
        }
        let secrets = self.lock.0.secret_box("messenger");
        if !secrets.is_unlocked() {
            return Err(IdentityError::SecretsLocked);
        }
        let bytes = unhex(hex);
        let nsec: [u8; 32] = bytes
            .try_into()
            .map_err(|_| IdentityError::Crypto("not 32 bytes".into()))?;
        let npub = veydan_recovery::rk::npub_of(&nsec)
            .ok_or_else(|| IdentityError::Crypto("not a key".into()))?;
        secrets
            .put(Self::SECRET_KEY_REF, &nsec)
            .await
            .map_err(locked)?;
        *self.npub.lock().unwrap() = Some(veydan_cloud::nip98::hex(&npub));
        Ok(())
    }

    /// `IdentityService::load_keys`: `NotLoggedIn` without a row,
    /// `SecretsLocked` when the host does not give the secret.
    async fn load_keys(&self) -> Result<[u8; 32], IdentityError> {
        let npub = self
            .npub
            .lock()
            .unwrap()
            .clone()
            .ok_or(IdentityError::NotLoggedIn)?;
        let bytes = self
            .lock
            .0
            .secret_box("messenger")
            .get(Self::SECRET_KEY_REF)
            .await
            .map_err(locked)?
            .ok_or_else(|| IdentityError::Crypto("secret missing".into()))?;
        let nsec: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::Crypto("not 32 bytes".into()))?;
        let of_secret = veydan_recovery::rk::npub_of(&nsec).map(|p| veydan_cloud::nip98::hex(&p));
        if of_secret.as_deref() != Some(npub.as_str()) {
            return Err(IdentityError::Crypto(
                "stored secret does not match identity".into(),
            ));
        }
        Ok(nsec)
    }

    /// `IdentityService::get`: the public half, if an identity was imported.
    fn get(&self) -> Option<String> {
        self.npub.lock().unwrap().clone()
    }
}

/// A box `cloud` in memory for the bookkeeping of the restore.
#[derive(Default)]
struct MemoryBox(Mutex<HashMap<String, Vec<u8>>>);

impl LockBox for MemoryBox {
    fn get<'a>(&'a self, key: &'a str) -> SeamFuture<'a, Option<Zeroizing<Vec<u8>>>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(key).cloned().map(Zeroizing::new)) })
    }
    fn put<'a>(&'a self, key: &'a str, value: &'a [u8]) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            self.0.lock().unwrap().insert(key.to_string(), value.to_vec());
            Ok(())
        })
    }
    fn delete<'a>(&'a self, key: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            self.0.lock().unwrap().remove(key);
            Ok(())
        })
    }
}

/// A device: its data file, lock, registry and messenger identity.
struct Device {
    core: Core,
    locked: Arc<Locked>,
    registry: Registry,
    identity: FakeIdentity,
    vault_dir: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Device {
    async fn new(vault_dir: &std::path::Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::open(
            &dir.path().join(db::DB_FILE),
            &[veydan_core::SCHEMA, veydan_sync_host::SCHEMA, LOCK],
        )
        .await
        .unwrap();
        let lock = Lock::new(pool.clone());
        lock.open_default().await.unwrap();
        let locked = Arc::new(Locked(lock));
        let identity = FakeIdentity {
            lock: locked.clone(),
            npub: Mutex::new(None),
        };
        Self {
            core: Core::new(pool, dir.path().to_owned(), Directory::default()),
            locked,
            registry: Registry::new().finish(&PLAN).unwrap(),
            identity,
            vault_dir: vault_dir.to_owned(),
            _dir: dir,
        }
    }

    fn lock(&self) -> &Lock {
        &self.locked.0
    }

    fn host(&self) -> Host<'_> {
        Host::States(self.locked.as_ref())
    }

    fn engine(&self, vmk: &Vmk, vault_id: &str, device_id: &str) -> Engine {
        Engine::with_key(
            Box::new(LocalDir::new(&self.vault_dir)),
            vmk,
            vault_id,
            device_id,
        )
    }

    async fn key_row(&self) -> Option<Value> {
        let table = self.registry.table_of("password_vault").unwrap();
        veydan_sync_host::rows::read_row(&self.core.db, table, veydan_lock::ROW_ID)
            .await
            .unwrap()
    }
}

impl RestoreEnv for Device {
    fn lock(&self) -> &Lock {
        &self.locked.0
    }
    fn db(&self) -> &sqlx::Pool<sqlx::Sqlite> {
        &self.core.db
    }
    fn has_identity(&self) -> BoxFuture<'_, Result<bool, CloudError>> {
        Box::pin(async move { Ok(self.identity.get().is_some()) })
    }
    fn open_vault<'a>(
        &'a self,
        _signer: Arc<dyn Signer>,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<Engine, CloudError>> {
        Box::pin(async move {
            // Veydan Cloud is the folder of the test; a storage of the
            // user's own is found by what the escrow says of it, as the app
            // does, and must hold this very vault.
            let Storage::Folder { path } = &sync.storage else {
                return Ok(self.engine(&Vmk(sync.vmk), &sync.vault_id, "device-b"));
            };
            let engine = Engine::with_key(
                Box::new(LocalDir::new(path)),
                &Vmk(sync.vmk),
                &sync.vault_id,
                "device-b",
            );
            engine
                .verify_manifest()
                .await
                .map_err(|e| CloudError::new(escrow::words::VAULT_UNREACHABLE, e.to_string()))?;
            Ok(engine)
        })
    }
    fn adopt_vault_key<'a>(
        &'a self,
        engine: &'a Engine,
        sync: &'a SyncKeys,
    ) -> BoxFuture<'a, Result<bool, CloudError>> {
        Box::pin(async move {
            let mut cfg = veydan_sync_host::config::load_config(&self.core.db).await;
            escrow::apply_storage(&mut cfg, &sync.storage);
            cfg.enabled = true;
            veydan_sync_host::config::save_config(&self.core.db, &cfg)
                .await
                .map_err(restore::app_error)?;
            adopt_and_bind(&self.host(), engine, &self.core, &self.registry, &Vmk(sync.vmk))
                .await
                .map_err(restore::app_error)
        })
    }
    fn import_identity<'a>(&'a self, nsec: &'a [u8; 32]) -> BoxFuture<'a, Result<(), CloudError>> {
        Box::pin(async move {
            let hex: String = nsec.iter().map(|b| format!("{b:02x}")).collect();
            self.identity
                .import_nsec(&hex)
                .await
                .map_err(|e| CloudError::new(code::CRYPTO, format!("{e:?}")))
        })
    }
}

/// The account before the loss: device A with a lock behind `FORGOTTEN`
/// and the words of its Recovery Key, its key row in the vault, and the
/// escrow in vstore under `rk` (which is A's key unless `rk` is another).
struct Account {
    vault_dir: tempfile::TempDir,
    vmk: Vmk,
    vault_id: String,
    /// The words of the Recovery Key of A's lock.
    words_of_a: String,
    row_vault_id: String,
    /// A field of Pass encrypted under the key of A's lock, as the vault
    /// carries it: `encrypt_field(key, "row-1", "password", "hunter2")`.
    field_of_a: String,
    objects: Arc<Objects>,
    store: Arc<HttpEscrowStore>,
}

async fn account(with_lock: bool) -> (Account, RecoveryKey, Escrow) {
    let vault_dir = tempfile::tempdir().unwrap();
    let (engine, vmk) = Engine::create(
        Box::new(LocalDir::new(vault_dir.path())),
        "the passphrase of the vault",
        "device-a",
    )
    .await
    .unwrap();
    let vault_id = engine.vault_id().to_string();

    // Device A: its lock, its key row published to the vault.
    let a = Device::new(vault_dir.path()).await;
    let mut words_of_a = String::new();
    let mut lock_key = None;
    let mut row_vault_id = String::new();
    let mut field_of_a = String::new();
    if with_lock {
        words_of_a = a
            .lock()
            .set(Some(FORGOTTEN.into()), None, None, None)
            .await
            .unwrap()
            .unwrap();
        let (key, id) = a.lock().require_open().unwrap();
        lock_key = Some(LockKey {
            vault_id: id.clone(),
            vault_key: *key.to_bytes(),
        });
        field_of_a = veydan_lock::encrypt_field(&key, "row-1", "password", "hunter2").unwrap();
        row_vault_id = id;
        let row = a.key_row().await.unwrap();
        let mut state = LocalState::default();
        engine
            .push(
                &mut state,
                vec![Op {
                    entity_type: "password_vault".into(),
                    entity_id: veydan_lock::ROW_ID.into(),
                    hlc: Hlc {
                        wall_ms: 1_791_012_345_000,
                        counter: 0,
                        device_id: "device-a".into(),
                    },
                    deleted: false,
                    payload: row,
                }],
            )
            .await
            .unwrap();
    }
    a.core.db.close().await;

    // The escrow in vstore.
    let (base, objects) = serve_vstore().await;
    let loopback = Arc::new(Loopback {
        base,
        client: reqwest::Client::new(),
    });
    let identity: Arc<dyn Signer> = Arc::new(MemorySigner::new(Zeroizing::new(NSEC)).unwrap());
    let store = Arc::new(HttpEscrowStore::new(loopback, identity));
    let rk = RecoveryKey::generate().unwrap();
    let escrow = Escrow {
        npub: veydan_recovery::rk::npub_of(&NSEC).unwrap(),
        created_at: 1_791_012_345,
        generation: 1,
        nsec: NSEC,
        sync: SyncKeys::cloud(vault_id.clone(), vmk.0),
        lock: lock_key,
        recovery: RecoveryParams {
            k: 0,
            n: 0,
            generation: 1,
        },
    };
    let sealed = escrow::seal(&rk, &escrow).await.unwrap();
    store
        .put(&rk.escrow_auth().unwrap().escrow_address(), &sealed)
        .await
        .unwrap();
    (
        Account {
            vault_dir,
            vmk,
            vault_id,
            words_of_a,
            row_vault_id,
            field_of_a,
            objects,
            store,
        },
        rk,
        escrow,
    )
}

/// Seals `escrow` under `rk` into vstore, in place of what was there.
async fn reseal(account: &Account, rk: &RecoveryKey, escrow: &Escrow) {
    let sealed = escrow::seal(rk, escrow).await.unwrap();
    account
        .store
        .put(&rk.escrow_auth().unwrap().escrow_address(), &sealed)
        .await
        .unwrap();
}

fn secret() -> NewSecret {
    NewSecret {
        password: NEW_PIN.into(),
        kind: Some("pin".into()),
        hint: None,
        old: false,
    }
}

/// The escrow of the account is sealed under the Recovery Key of A's lock:
/// what `cloud_recovery_key_show_once` leaves behind.
async fn account_under_the_words_of_a() -> (Account, RecoveryKey) {
    let (account, other, escrow) = account(true).await;
    // The escrow under a key nobody holds goes; the one under A's words stays.
    assert!(account
        .store
        .delete(&other.escrow_auth().unwrap().escrow_address())
        .await
        .unwrap());
    let rk = RecoveryKey::from_words(&account.words_of_a).unwrap();
    let sealed = escrow::seal(&rk, &escrow).await.unwrap();
    account
        .store
        .put(&rk.escrow_auth().unwrap().escrow_address(), &sealed)
        .await
        .unwrap();
    (account, rk)
}

// ── The tests ────────────────────────────────────────────────────────────────

/// The six steps in their order: the row of the vault opens with the words
/// and takes the new PIN, the identity goes in last, and the messenger has
/// its key — not `SecretsLocked`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_order_of_the_contract_leaves_the_messenger_with_its_key() {
    let (account, rk) = account_under_the_words_of_a().await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();
    let phases = Mutex::new(Vec::new());
    let progress = |phase: Phase| phases.lock().unwrap().push(phase);

    let restored = restore::restore(
        &b,
        account.store.as_ref(),
        &boxed,
        &rk.words(),
        secret(),
        Vault::Join,
        &progress,
    )
    .await
    .expect("the restore");

    assert_eq!(restored.lock, LockOutcome::Kept);
    assert!(restored.vault_joined);
    assert_eq!(restored.npub, veydan_cloud::nip98::hex(&veydan_recovery::rk::npub_of(&NSEC).unwrap()));
    assert_eq!(
        *phases.lock().unwrap(),
        [
            Phase::Reading,
            Phase::Opening,
            Phase::Vault,
            Phase::KeyRow,
            Phase::Lock,
            Phase::Identity,
            Phase::Done
        ]
    );
    // The messenger: its key opens, the identity is the one of the escrow.
    let keys = b.identity.load_keys().await.expect("not SecretsLocked");
    assert_eq!(keys, NSEC);
    // The lock: the row of the vault, open, behind the new PIN, the words kept.
    let row = b.key_row().await.unwrap();
    assert_eq!(row["vault_id"], account.row_vault_id);
    assert_eq!(row["recovery_version"], 2);
    assert_eq!(b.lock().require_open().unwrap().1, account.row_vault_id);
    b.lock().lock();
    assert!(b.lock().unlock(FORGOTTEN).await.is_err());
    assert!(b.lock().unlock(NEW_PIN).await.unwrap());
    b.lock().recovery_check(&rk.words()).await.unwrap();
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    // The device is bound to the vault of the escrow.
    let binding = veydan_sync_host::config::load_binding(&b.core.db).await.unwrap();
    assert_eq!(binding.vault_id, account.vault_id);
    assert_eq!(binding.vmk_b64, account.vmk.to_base64());
    // The box: the words and the state of the escrow as read.
    assert_eq!(escrow::load_rk(&boxed).await.unwrap().unwrap(), rk);
    let state = escrow::load_state(&boxed).await.unwrap().unwrap();
    assert_eq!(state.generation, 1);
    assert_eq!(account.objects.escrow.lock().unwrap().len(), 1);
    b.core.db.close().await;
}

/// The order the contract forbids: `import_nsec` before `adopt_vault_key`
/// makes a key row of this device under the built-in secret; the row of
/// the vault replaces it, its password is unknown, and the messenger's
/// store answers `SecretsLocked`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_identity_before_the_key_row_ends_in_secrets_locked() {
    let (account, _rk) = account_under_the_words_of_a().await;
    let b = Device::new(account.vault_dir.path()).await;
    let hex: String = NSEC.iter().map(|b| format!("{b:02x}")).collect();

    // Step 6 first: the identity makes the local key row (`ensure_key`).
    b.identity.import_nsec(&hex).await.unwrap();
    assert!(b.identity.load_keys().await.is_ok());
    let own = b.key_row().await.unwrap();
    assert_ne!(own["vault_id"], account.row_vault_id);

    // Then step 4: the row of the vault takes its place.
    let engine = b.engine(&account.vmk, &account.vault_id, "device-b");
    let replaced = adopt_and_bind(&b.host(), &engine, &b.core, &b.registry, &account.vmk)
        .await
        .unwrap();
    assert!(replaced);
    assert_eq!(b.key_row().await.unwrap()["vault_id"], account.row_vault_id);

    let failure = b.identity.load_keys().await.unwrap_err();
    assert_eq!(failure, IdentityError::SecretsLocked, "the contract names SecretsLocked");
    assert!(b.identity.get().is_some(), "the identity row stands, its key does not");
    b.core.db.close().await;
}

/// «Строка не совпала»: the escrow was sealed under another Recovery Key
/// than the one wrapping the row of the vault (a reissue the escrow did
/// not follow): the lock is set up anew under the words — around the key
/// the escrow carries, so a field of Pass encrypted under it on A still
/// reads on B — the identity and the vault are untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_row_the_words_do_not_open_gives_way_to_a_lock_set_up_anew() {
    let (account, rk, _escrow) = account(true).await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .expect("the restore");

    assert_eq!(restored.lock, LockOutcome::Reinstalled);
    assert_eq!(restored.escrow_refreshed, None, "the command fills it");
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    let row = b.key_row().await.unwrap();
    assert_eq!(row["vault_id"], account.row_vault_id, "the key of the escrow, under its id");
    assert_eq!(row["recovery_version"], 2);
    let (key, id) = b.lock().require_open().unwrap();
    assert_eq!(id, account.row_vault_id);
    assert_eq!(
        veydan_lock::decrypt_field(&key, "row-1", "password", &account.field_of_a).unwrap(),
        "hunter2",
        "what A encrypted under the key still reads"
    );
    b.lock().lock();
    assert!(b.lock().unlock(FORGOTTEN).await.is_err());
    assert!(b.lock().unlock(NEW_PIN).await.unwrap());
    b.lock().recovery_check(&rk.words()).await.unwrap();
    assert_eq!(
        b.lock().recovery_check(&account.words_of_a).await.unwrap_err(),
        veydan_lock::Error::RecoveryInvalid
    );
    let binding = veydan_sync_host::config::load_binding(&b.core.db).await.unwrap();
    assert_eq!(binding.vault_id, account.vault_id);
    b.core.db.close().await;
}

/// §7, the named trigger: `vault_id` of the row the vault brought is not
/// `escrow.lock.vault_id` (A reset its lock after the escrow was written,
/// and the escrow did not follow). The row does not open with the words;
/// the lock is set up anew around the key of the escrow, under its id;
/// the identity survives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_row_of_another_vault_id_than_the_escrow_gives_way_to_the_key_of_the_escrow() {
    let (account, rk, escrow) = account(true).await;
    let other_key = veydan_lock::SecretKey::random();
    let other_id = "b1c2d3e4-f5a6-4b7c-8d9e-0f1a2b3c4d5e";
    let field_of_other =
        veydan_lock::encrypt_field(&other_key, "row-2", "password", "s3cret").unwrap();
    let mut stale = escrow.clone();
    stale.lock = Some(LockKey {
        vault_id: other_id.into(),
        vault_key: *other_key.to_bytes(),
    });
    reseal(&account, &rk, &stale).await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .expect("the restore");

    assert_eq!(restored.lock, LockOutcome::Reinstalled);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    let row = b.key_row().await.unwrap();
    assert_eq!(row["vault_id"], other_id);
    assert_ne!(row["vault_id"], account.row_vault_id);
    let (key, _) = b.lock().require_open().unwrap();
    assert_eq!(
        veydan_lock::decrypt_field(&key, "row-2", "password", &field_of_other).unwrap(),
        "s3cret"
    );
    b.lock().lock();
    assert!(b.lock().unlock(NEW_PIN).await.unwrap());
    b.lock().recovery_check(&rk.words()).await.unwrap();
    assert_eq!(
        veydan_sync_host::config::load_binding(&b.core.db).await.unwrap().vault_id,
        account.vault_id
    );
    b.core.db.close().await;
}

/// §7, «чистая установка»: a device that has an identity, a binding or a
/// lock is refused before vstore is asked, and nothing of it changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_is_not_clean_is_refused_before_anything_is_touched() {
    let (account, rk) = account_under_the_words_of_a().await;
    let hex: String = NSEC.iter().map(|b| format!("{b:02x}")).collect();
    let gets = || {
        account
            .objects
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "GET")
            .count()
    };

    // An identity of the messenger.
    let b = Device::new(account.vault_dir.path()).await;
    b.identity.import_nsec(&hex).await.unwrap();
    let own = b.key_row().await.unwrap();
    let boxed = MemoryBox::default();
    let e = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, code::IDENTITY_EXISTS);
    assert_eq!(b.key_row().await.unwrap(), own);
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    assert!(escrow::load_rk(&boxed).await.unwrap().is_none());
    b.core.db.close().await;

    // A binding to a vault.
    let c = Device::new(account.vault_dir.path()).await;
    veydan_sync_host::config::save_binding(
        &c.core.db,
        &veydan_sync_host::config::VaultBinding {
            vault_id: "3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c".into(),
            vmk_b64: account.vmk.to_base64(),
        },
    )
    .await
    .unwrap();
    let e = restore::restore(&c, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::NOT_CLEAN);
    assert_eq!(
        veydan_sync_host::config::load_binding(&c.core.db).await.unwrap().vault_id,
        "3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c"
    );
    c.core.db.close().await;

    // A lock of its own.
    let d = Device::new(account.vault_dir.path()).await;
    d.lock().set(Some("1234".into()), None, Some("pin".into()), None).await.unwrap();
    let own = d.key_row().await.unwrap();
    let e = restore::restore(&d, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::NOT_CLEAN);
    assert_eq!(d.key_row().await.unwrap(), own);
    assert!(d.lock().unlock("1234").await.unwrap());
    assert_eq!(gets(), 0, "vstore was never asked");
    d.core.db.close().await;
}

/// `lock: null` and no key row in the vault: the install goes on without a
/// lock, the nsec under the built-in secret.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_without_a_lock_is_restored_without_one() {
    let (account, rk, escrow) = account(false).await;
    assert!(escrow.lock.is_none());
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .expect("the restore");

    assert_eq!(restored.lock, LockOutcome::NoLock);
    assert!(!b.lock().enabled().await);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    b.core.db.close().await;
}

/// Wrong words, and words of another account: the store says what vstore
/// says, nothing is written on the device.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn words_that_are_not_a_key_or_know_no_escrow_change_nothing() {
    let (account, _rk, _escrow) = account(true).await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let e = restore::restore(&b, account.store.as_ref(), &boxed, "abandon abandon", secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::BAD_WORDS);
    let other = RecoveryKey::generate().unwrap();
    let e = restore::restore(&b, account.store.as_ref(), &boxed, &other.words(), secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::NOT_FOUND);
    assert!(b.key_row().await.is_none());
    assert!(b.identity.get().is_none());
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    let gets = account
        .objects
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, _)| m == "GET")
        .count();
    assert_eq!(gets, 1, "words that are no key ask the server nothing");
    b.core.db.close().await;
}

/// The account as [`account`] makes it, with the escrow under the words of
/// A's lock naming `storage` as where the vault lives.
async fn account_in(storage: impl FnOnce(&Account) -> Storage) -> (Account, RecoveryKey) {
    let (account, other, mut escrow) = account(true).await;
    assert!(account
        .store
        .delete(&other.escrow_auth().unwrap().escrow_address())
        .await
        .unwrap());
    let rk = RecoveryKey::from_words(&account.words_of_a).unwrap();
    escrow.sync.storage = storage(&account);
    reseal(&account, &rk, &escrow).await;
    (account, rk)
}

fn folder_of(account: &Account) -> Storage {
    Storage::Folder {
        path: account.vault_dir.path().to_string_lossy().into_owned(),
    }
}

/// §4: the vault lives in a storage of the user's own. The escrow names it
/// with its access, the restored device finds the vault there — not on
/// Veydan Cloud — and its settings of sync are the ones the account had.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_storage_of_the_user_is_set_up_as_the_escrow_names_it() {
    let (account, rk) = account_in(folder_of).await;
    // Veydan Cloud of this device holds nothing of the account.
    let nothing = tempfile::tempdir().unwrap();
    let b = Device::new(nothing.path()).await;
    let boxed = MemoryBox::default();

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .expect("the restore");

    assert_eq!(restored.lock, LockOutcome::Kept);
    assert!(restored.vault_joined);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    assert_eq!(b.key_row().await.unwrap()["vault_id"], account.row_vault_id);
    let cfg = veydan_sync_host::config::load_config(&b.core.db).await;
    assert!(cfg.enabled);
    assert_eq!(cfg.backend, "folder");
    assert_eq!(std::path::Path::new(&cfg.folder_path), account.vault_dir.path());
    assert_eq!(
        veydan_sync_host::config::load_binding(&b.core.db).await.unwrap().vault_id,
        account.vault_id
    );
    b.core.db.close().await;
}

/// §7: a storage that is not within reach of the device — a folder of
/// another computer — is said before anything is written, and the person
/// may restore without it: the identity comes back, and the lock is set up
/// around the key of the escrow, the one the row in the vault wraps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_storage_out_of_reach_is_said_and_the_restore_goes_on_without_it() {
    let (account, rk) = account_in(|_| Storage::Folder {
        path: "/no/such/folder/of/another/computer".into(),
    })
    .await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let e = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::VAULT_UNREACHABLE);
    assert!(b.key_row().await.is_none());
    assert!(b.identity.get().is_none());
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    assert!(escrow::load_rk(&boxed).await.unwrap().is_none());

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Skip, &|_| {})
        .await
        .expect("the restore without the vault");
    assert_eq!(restored.lock, LockOutcome::Reinstalled);
    assert!(!restored.vault_joined);
    assert_eq!(b.identity.load_keys().await.expect("not SecretsLocked"), NSEC);
    // The lock of the account: its key, behind the new PIN and the words.
    assert_eq!(b.key_row().await.unwrap()["vault_id"], account.row_vault_id);
    let (key, _) = b.lock().require_open().unwrap();
    assert_eq!(
        veydan_lock::decrypt_field(&key, "row-1", "password", &account.field_of_a).unwrap(),
        "hunter2"
    );
    b.lock().lock();
    assert!(b.lock().unlock(NEW_PIN).await.unwrap());
    b.lock().recovery_check(&rk.words()).await.unwrap();
    // No vault was joined, and the settings of sync were not touched.
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    assert!(!veydan_sync_host::config::load_config(&b.core.db).await.enabled);
    assert_eq!(escrow::load_rk(&boxed).await.unwrap().unwrap(), rk);
    b.core.db.close().await;
}

/// An account without a lock restored without its vault stays without one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_without_a_lock_restored_without_its_vault_has_no_lock() {
    let (account, rk, _escrow) = account(false).await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();

    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), secret(), Vault::Skip, &|_| {})
        .await
        .expect("the restore without the vault");
    assert_eq!(restored.lock, LockOutcome::NoLock);
    assert!(!restored.vault_joined);
    assert!(!b.lock().enabled().await);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    b.core.db.close().await;
}

/// §6, step 4: a linked device enters the vault the old one sent along in
/// the order of §7 — storage set up, key row adopted, lock opened with the
/// secret of the old device — and only then takes the identity. A wrong
/// secret leaves the vault joined, and the next try goes to the lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_device_enters_the_vault_with_the_secret_of_the_lock() {
    let (account, _rk, _escrow) = account(true).await;
    let nothing = tempfile::tempdir().unwrap();
    let b = Device::new(nothing.path()).await;
    let nsec = Zeroizing::new(NSEC);
    let sync = SyncKeys {
        vault_id: account.vault_id.clone(),
        vmk: account.vmk.0,
        storage: folder_of(&account),
    };

    for wrong in [Some("0000"), None] {
        let e = restore::enter(&b, &nsec, &sync, wrong).await.err().unwrap();
        assert_eq!(e.code, code::WRONG_PASSWORD, "{wrong:?}");
        assert!(b.identity.get().is_none());
    }
    assert_eq!(
        veydan_sync_host::config::load_binding(&b.core.db).await.unwrap().vault_id,
        account.vault_id
    );

    restore::enter(&b, &nsec, &sync, Some(FORGOTTEN)).await.expect("the entry");
    b.import_identity(&NSEC).await.expect("the identity");
    assert_eq!(b.identity.load_keys().await.expect("not SecretsLocked"), NSEC);
    assert_eq!(b.key_row().await.unwrap()["vault_id"], account.row_vault_id);
    let cfg = veydan_sync_host::config::load_config(&b.core.db).await;
    assert!(cfg.enabled);
    assert_eq!(cfg.backend, "folder");

    // A device bound to another vault is not the place of this account.
    let mut other = sync.clone();
    other.vault_id = "another-vault".into();
    let e = restore::enter(&b, &nsec, &other, Some(FORGOTTEN)).await.err().unwrap();
    assert_eq!(e.code, escrow::words::NOT_CLEAN);
    b.core.db.close().await;
}

/// A storage the linked device cannot reach leaves it as it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_device_that_cannot_reach_the_storage_is_left_clean() {
    let (account, _rk, _escrow) = account(true).await;
    let b = Device::new(account.vault_dir.path()).await;
    let sync = SyncKeys {
        vault_id: account.vault_id.clone(),
        vmk: account.vmk.0,
        storage: Storage::Folder {
            path: "/no/such/folder/of/another/computer".into(),
        },
    };
    let e = restore::enter(&b, &Zeroizing::new(NSEC), &sync, Some(FORGOTTEN))
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::VAULT_UNREACHABLE);
    assert!(b.key_row().await.is_none());
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    b.core.db.close().await;
}

/// No secret, an old one, or a new one: the secret of the lock as the
/// person gives it (§7, 2026-10-19).
fn given(password: &str, old: bool) -> NewSecret {
    NewSecret { password: password.into(), kind: Some("pin".into()), hint: None, old }
}

/// The look before the secret is asked says whether the account has a
/// lock, and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_look_says_whether_the_account_has_a_lock_and_writes_nothing() {
    let (locked, locked_rk) = account_under_the_words_of_a().await;
    let b = Device::new(locked.vault_dir.path()).await;
    let peeked = restore::peek(&b, locked.store.as_ref(), &locked_rk.words()).await.expect("the look");
    assert!(peeked.lock);
    assert!(b.key_row().await.is_none());
    assert!(b.identity.get().is_none());
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    b.core.db.close().await;

    let (plain, plain_rk, _escrow) = account(false).await;
    let c = Device::new(plain.vault_dir.path()).await;
    assert!(!restore::peek(&c, plain.store.as_ref(), &plain_rk.words()).await.expect("the look").lock);
    c.core.db.close().await;
}

/// An account without a lock is restored with no secret at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_without_a_lock_needs_no_secret() {
    let (account, rk, _escrow) = account(false).await;
    let b = Device::new(account.vault_dir.path()).await;
    let restored = restore::restore(&b, account.store.as_ref(), &MemoryBox::default(), &rk.words(), given("", false), Vault::Join, &|_| {})
        .await
        .expect("the restore");
    assert_eq!(restored.lock, LockOutcome::NoLock);
    assert!(!b.lock().enabled().await);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    b.core.db.close().await;
}

/// An account with a lock and no secret given: said before anything is
/// written, the device stays clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_with_a_lock_and_no_secret_is_told_before_anything_is_written() {
    let (account, rk) = account_under_the_words_of_a().await;
    let b = Device::new(account.vault_dir.path()).await;
    let e = restore::restore(&b, account.store.as_ref(), &MemoryBox::default(), &rk.words(), given("", false), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, escrow::words::LOCK_SECRET_NEEDED);
    assert!(b.key_row().await.is_none());
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_none());
    assert!(b.identity.get().is_none());
    b.core.db.close().await;
}

/// The secret the person remembers opens the row of the vault and stays the
/// secret; a wrong one is said, and the next try with the same words carries
/// on from the lock (the device is bound already, not refused as unclean).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_secret_remembered_opens_the_lock_and_a_wrong_one_is_tried_again() {
    let (account, rk) = account_under_the_words_of_a().await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();
    let e = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), given("not it", true), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, code::WRONG_PASSWORD);
    assert!(b.identity.get().is_none(), "nothing of the identity before the lock is open");
    assert!(veydan_sync_host::config::load_binding(&b.core.db).await.is_some());

    // The look of the next try still sees the lock, and the device is not refused.
    assert!(restore::peek(&b, account.store.as_ref(), &rk.words()).await.expect("the look").lock);
    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), given(FORGOTTEN, true), Vault::Join, &|_| {})
        .await
        .expect("the restore");
    assert_eq!(restored.lock, LockOutcome::Opened);
    assert!(restored.vault_joined);
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    b.lock().lock();
    assert!(b.lock().unlock(FORGOTTEN).await.unwrap(), "the old secret stays");
    assert!(veydan_core::settings::get(&b.core.db, restore::RESTORE_PENDING).await.is_none(), "the restore is done");
    // Done: a restore once more is refused, as on any device with an identity.
    let e = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), given(FORGOTTEN, true), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    assert_eq!(e.code, code::IDENTITY_EXISTS);
    b.core.db.close().await;
}

/// After a wrong old secret the person sets a new one instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_wrong_old_secret_a_new_one_is_set() {
    let (account, rk) = account_under_the_words_of_a().await;
    let b = Device::new(account.vault_dir.path()).await;
    let boxed = MemoryBox::default();
    restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), given("not it", true), Vault::Join, &|_| {})
        .await
        .err()
        .unwrap();
    let restored = restore::restore(&b, account.store.as_ref(), &boxed, &rk.words(), given(NEW_PIN, false), Vault::Join, &|_| {})
        .await
        .expect("the restore");
    assert_eq!(restored.lock, LockOutcome::Kept);
    b.lock().lock();
    assert!(b.lock().unlock(NEW_PIN).await.unwrap());
    assert_eq!(b.identity.load_keys().await.unwrap(), NSEC);
    b.core.db.close().await;
}

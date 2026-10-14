// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The Recovery Key of the account on this device: its first issue when
//! the recovery of the cloud is set up, and its reissue — all or nothing,
//! in the order of internal/cloud-recovery.md §3 (platform-spec 8.1):
//!
//! 1. the secret of the lock, the confirmation of the owner;
//! 2. a new key RK′ in memory, `generation′ = generation + 1`;
//! 3. the wrap of version 2 of the key of the lock under RK′, in memory;
//! 4. the new escrow under RK′ at its new address; the old one stays;
//! 5. the shares of `generation′` to the trusted people, their acks;
//! 6. only now the wrap goes into the key row, and the row to sync;
//! 7. the words, once; the old escrow deleted, the old shares revoked.
//!
//! Step 6 is the point of no return. A failure before it undoes what was
//! made (the new escrow, the new shares) and the old key stays in force.
//! After it the new key is in force: whatever fails from there on, the
//! words are handed out, and the rest of step 7 — the key into the box,
//! the old escrow and shares gone — is finished by [`resume`] at the next
//! start. The state of a reissue under way lives in the box `cloud` of the
//! lock ([`Pending`], with the mark of the point of no return), so a start
//! after a crash can tell what to do. The old escrow is written down as
//! [`Retired`] before the state forgets it and stays so until its delete
//! and revoke went ([`retire`], at every start and daily): the old words
//! open nothing once that is done, however the first try ended. Step 5
//! goes through [`ShareDelivery`]: until stage C5 fills it ([`NoShares`]), there are no
//! trusted people, nothing is delivered, and a start with a reissue under
//! way before step 6 always rolls it back — continuing a wait for acks is
//! C5's.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use veydan_recovery::RecoveryKey;
use veydan_shell::seams::{code, CloudError, LockBox, SeamFuture};
use zeroize::Zeroizing;

use crate::api::now;
use crate::escrow::{self, words, EscrowState, EscrowStore, Sources};
use crate::restore::lock_error;

/// The key of the box: a reissue under way, JSON of [`Pending`].
pub const KEY_PENDING: &str = "reissue.pending";
/// The key of the box: the escrows a reissue retired that are not gone
/// yet, JSON of a list of [`Retired`].
pub const KEY_RETIRED: &str = "escrow.retired";

/// An escrow of an older key, retired by a reissue: it is deleted and its
/// shares revoked until both are done ([`retire`]), so that the old words
/// stop opening anything even when the first try at step 7 failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retired {
    pub address: String,
    pub generation: u32,
    /// The shares of `generation` are revoked already: the trusted people
    /// are told once, however many times the delete is tried.
    #[serde(default)]
    pub revoked: bool,
}

fn ser_secret<S: Serializer>(value: &Zeroizing<String>, s: S) -> Result<S::Ok, S::Error> {
    value.as_str().serialize(s)
}

fn de_secret<'de, D: Deserializer<'de>>(d: D) -> Result<Zeroizing<String>, D::Error> {
    String::deserialize(d).map(Zeroizing::new)
}

/// A reissue under way, as the box keeps it: from step 4 on, and with
/// `committed` from step 6 on.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// The 16 bytes of RK′, hex; wiped when dropped.
    #[serde(serialize_with = "ser_secret", deserialize_with = "de_secret")]
    pub rk_hex: Zeroizing<String>,
    pub generation: u32,
    /// The address of the new escrow.
    pub address: String,
    /// [`escrow::fingerprint`] of what was sealed into it.
    pub fingerprint: String,
    /// `hex sha256` of the sealed object.
    pub sealed_sha256: String,
    /// `created_at` of what was sealed, unix seconds.
    pub written_at: u64,
    /// Unix seconds.
    pub started_at: u64,
    /// Step 6 is done: the new key is in force, the rest is finished, never
    /// undone.
    #[serde(default)]
    pub committed: bool,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("generation", &self.generation)
            .field("address", &self.address)
            .field("committed", &self.committed)
            .finish_non_exhaustive()
    }
}

impl Pending {
    /// RK′ itself.
    fn rk(&self) -> Result<RecoveryKey, CloudError> {
        let text = self.rk_hex.as_str();
        let mut bytes = Zeroizing::new([0u8; 16]);
        if text.len() != 32 {
            return Err(crypto("the reissue under way holds no key of 16 bytes"));
        }
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = text
                .get(2 * i..2 * i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| crypto("the reissue under way holds a key that is not hex"))?;
        }
        RecoveryKey::from_bytes(*bytes).map_err(crypto)
    }
}

/// Step 5 and the revoke of step 7: the shares of the trusted people
/// (cloud-recovery.md §8.3). Stage C5 fills it; [`NoShares`] until then.
pub trait ShareDelivery: Send + Sync {
    /// The shares of `generation` under `rk` to every trusted person, and
    /// the wait for k acks; an error undoes the reissue.
    fn deliver<'a>(&'a self, rk: &'a RecoveryKey, generation: u32) -> SeamFuture<'a, ()>;
    /// `recovery.share.revoke` for `generation`.
    fn revoke(&self, generation: u32) -> SeamFuture<'_, ()>;
}

/// No trusted people yet: nothing to deliver, nothing to revoke.
pub struct NoShares;

impl ShareDelivery for NoShares {
    fn deliver<'a>(&'a self, _rk: &'a RecoveryKey, _generation: u32) -> SeamFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn revoke(&self, _generation: u32) -> SeamFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

/// What an issue works with on this device.
pub struct Env<'a> {
    pub sources: Sources<'a>,
    pub store: &'a dyn EscrowStore,
    /// The box `cloud` of the lock.
    pub boxed: &'a dyn LockBox,
    pub shares: &'a dyn ShareDelivery,
}

/// What an issue hands out, once.
pub struct Issued {
    /// The 12 words, one space between; wiped when dropped.
    pub words: Zeroizing<String>,
    pub generation: u32,
}

impl std::fmt::Debug for Issued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Issued")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

fn crypto(e: impl std::fmt::Display) -> CloudError {
    CloudError::new(code::CRYPTO, e.to_string())
}

/// The reissue under way, from the box.
pub async fn load_pending(boxed: &dyn LockBox) -> Result<Option<Pending>, CloudError> {
    let Some(bytes) = boxed.get(KEY_PENDING).await? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| crypto(format!("the reissue under way in the box: {e}")))
}

async fn save_pending(boxed: &dyn LockBox, pending: &Pending) -> Result<(), CloudError> {
    let json = Zeroizing::new(serde_json::to_vec(pending).map_err(crypto)?);
    boxed.put(KEY_PENDING, &json).await
}

/// Undo a reissue that did not reach step 6: the new escrow goes, the
/// new shares are revoked, the box forgets it. Errors of the store are not
/// an error here: an orphan is replaced by the next reissue (§3).
async fn rollback(env: &Env<'_>, pending: &Pending) -> Result<(), CloudError> {
    let _ = env.store.delete(&pending.address).await;
    let _ = env.shares.revoke(pending.generation).await;
    env.boxed.delete(KEY_PENDING).await
}

/// The escrows retired and not gone yet, from the box.
pub async fn load_retired(boxed: &dyn LockBox) -> Result<Vec<Retired>, CloudError> {
    let Some(bytes) = boxed.get(KEY_RETIRED).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| crypto(format!("the retired escrows in the box: {e}")))
}

async fn save_retired(boxed: &dyn LockBox, retired: &[Retired]) -> Result<(), CloudError> {
    if retired.is_empty() {
        return boxed.delete(KEY_RETIRED).await;
    }
    let json = serde_json::to_vec(retired).map_err(crypto)?;
    boxed.put(KEY_RETIRED, &json).await
}

/// The escrows retired by a reissue deleted and their shares revoked: an
/// entry leaves the box only when its `DELETE` was answered — `204`, or
/// `404` when nothing of this npub is at the address (vstore knows the
/// owner on every host, spec/vstore.md §8) — and its revoke went. How
/// many are left for the next try (the daily pass of the app, `recovery`).
pub async fn retire(env: &Env<'_>) -> Result<usize, CloudError> {
    let retired = load_retired(env.boxed).await?;
    if retired.is_empty() {
        return Ok(0);
    }
    // Never the escrow in force, whatever the list says.
    let current = escrow::load_state(env.boxed).await?.map(|s| s.address);
    let mut left = Vec::new();
    for mut old in retired {
        if current.as_deref() == Some(old.address.as_str()) {
            continue;
        }
        if !old.revoked {
            old.revoked = env.shares.revoke(old.generation).await.is_ok();
        }
        let deleted = env.store.delete(&old.address).await.is_ok();
        if !(deleted && old.revoked) {
            left.push(old);
        }
    }
    save_retired(env.boxed, &left).await?;
    Ok(left.len())
}

/// Step 7 after the point of no return: the old escrow written down as
/// retired, the new key and the state of its escrow into the box, the box
/// forgets the reissue, then the retired escrows deleted and their shares
/// revoked ([`retire`]; what fails there stays retired for the next try).
/// Made again from the start by [`resume`] when a step failed: every step
/// stands being made twice.
async fn finish(env: &Env<'_>, pending: &Pending) -> Result<(), CloudError> {
    let rk = pending.rk()?;
    let previous = escrow::load_state(env.boxed).await?;
    // Before the state forgets the old address: a finish made again after
    // a failure finds only the new escrow in the state.
    if let Some(old) = previous.filter(|old| old.address != pending.address) {
        let mut retired = load_retired(env.boxed).await?;
        if !retired.iter().any(|r| r.address == old.address) {
            retired.push(Retired {
                address: old.address,
                generation: old.generation,
                revoked: false,
            });
            save_retired(env.boxed, &retired).await?;
        }
    }
    escrow::save_rk(env.boxed, &rk, pending.generation).await?;
    escrow::save_state(
        env.boxed,
        &EscrowState {
            address: pending.address.clone(),
            fingerprint: pending.fingerprint.clone(),
            generation: pending.generation,
            written_at: pending.written_at,
            sealed_sha256: Some(pending.sealed_sha256.clone()),
        },
    )
    .await?;
    env.boxed.delete(KEY_PENDING).await?;
    if let Err(e) = retire(env).await {
        eprintln!("cloud: the retired escrow is deleted at the next pass: {e}");
    }
    Ok(())
}

/// Whether a reissue found under way passed its point of no return: the
/// box says so, or — when the mark itself was not written — the key row
/// opens with RK′ already.
async fn in_force(env: &Env<'_>, pending: &Pending) -> Result<bool, CloudError> {
    if pending.committed {
        return Ok(true);
    }
    let lock = env.sources.lock;
    if !lock.enabled().await {
        return Ok(false);
    }
    let words = pending.rk()?.words();
    Ok(lock.recovery_check(&words).await.is_ok())
}

/// At the start of the app: a reissue under way is finished when it passed
/// step 6 and rolled back when it did not (see the module). Whether there
/// was one.
pub async fn resume(env: &Env<'_>) -> Result<bool, CloudError> {
    match load_pending(env.boxed).await? {
        Some(pending) => {
            if in_force(env, &pending).await? {
                finish(env, &pending).await?;
            } else {
                rollback(env, &pending).await?;
            }
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Whether the Recovery Key was issued on another device: the box holds no
/// escrow state of this one, yet the server says the recovery is configured
/// (`server_configured`, from `GET /account`). The row of the lock cannot
/// tell: `Lock::set` issues words of version 2 on every device, so a lock
/// of version 2 proves nothing (the marker for devices without a lock is
/// decision D12, after the release). A second key issued by accident would
/// replace the first one unseen (audit KA1-02).
pub async fn issued_elsewhere(env: &Env<'_>, server_configured: bool) -> Result<bool, CloudError> {
    if escrow::load_state(env.boxed).await?.is_some() {
        return Ok(false);
    }
    Ok(server_configured)
}

/// [`issued_elsewhere`] as the refusal [`words::ISSUED_ELSEWHERE`].
pub async fn refuse_elsewhere(env: &Env<'_>, server_configured: bool) -> Result<(), CloudError> {
    if issued_elsewhere(env, server_configured).await? {
        return Err(CloudError::new(
            words::ISSUED_ELSEWHERE,
            "the recovery key was issued on another device: reissue it there, restore with the words, or issue anew on purpose",
        ));
    }
    Ok(())
}

/// The first issue of the Recovery Key of the cloud on this device: the
/// words the lock may have issued before are not known here, so a key of
/// the cloud is always a new one. Refused with [`words::ALREADY_SET_UP`]
/// once the box holds a key: the way on is [`reissue`].
pub async fn first_setup(env: &Env<'_>, current_secret: &str) -> Result<Issued, CloudError> {
    if escrow::load_rk(env.boxed).await?.is_some() {
        return Err(CloudError::new(
            words::ALREADY_SET_UP,
            "the recovery key of the cloud is issued on this device",
        ));
    }
    issue(env, current_secret).await
}

/// A new Recovery Key in place of the one in force, by the seven steps.
pub async fn reissue(env: &Env<'_>, current_secret: &str) -> Result<Issued, CloudError> {
    issue(env, current_secret).await
}

async fn issue(env: &Env<'_>, current_secret: &str) -> Result<Issued, CloudError> {
    if load_pending(env.boxed).await?.is_some() {
        return Err(CloudError::new(
            words::REISSUE_PENDING,
            "a reissue is under way on this device; it is finished or rolled back at the next start",
        ));
    }
    let lock = env.sources.lock;
    let generation = escrow::load_generation(env.boxed).await? + 1;

    // Steps 1 to 3: the owner, RK′, the wrap in memory. A device without a
    // lock has no wrap to make; its escrow says `lock: null`.
    let rk = RecoveryKey::generate().map_err(crypto)?;
    let prepared = if lock.enabled().await {
        Some(
            lock.recovery_prepare_v2(current_secret, rk.as_bytes())
                .await
                .map_err(lock_error)?,
        )
    } else {
        None
    };

    // Step 4: the new escrow at its new address; the old one stays. Sealed
    // before the box knows of the reissue: a failure here leaves nothing.
    let recovery = veydan_recovery::RecoveryParams {
        k: 0,
        n: 0,
        generation,
    };
    let plaintext = escrow::gather(&env.sources, generation, recovery).await?;
    let sealed = escrow::seal(&rk, &plaintext).await?;
    let auth = rk.escrow_auth().map_err(crypto)?;
    let mut pending = Pending {
        rk_hex: Zeroizing::new(crate::nip98::hex(rk.as_bytes())),
        generation,
        address: auth.escrow_address(),
        fingerprint: escrow::fingerprint(&plaintext),
        sealed_sha256: escrow::sha256_hex(&sealed),
        written_at: plaintext.created_at,
        started_at: now()?,
        committed: false,
    };
    save_pending(env.boxed, &pending).await?;
    if let Err(e) = env.store.put(&pending.address, &sealed).await {
        rollback(env, &pending).await?;
        return Err(e);
    }

    // Step 5: the shares, and the acks.
    if let Err(e) = env.shares.deliver(&rk, generation).await {
        rollback(env, &pending).await?;
        return Err(e);
    }

    // Step 6: the row changes — the point of no return.
    if let Some(prepared) = &prepared {
        if let Err(e) = lock.recovery_store_v2(prepared).await {
            rollback(env, &pending).await?;
            return Err(lock_error(e));
        }
    }
    pending.committed = true;
    let words = rk.words();
    // From here nothing returns without the words: what the box refuses now
    // is finished at the next start (`resume`).
    if let Err(e) = save_pending(env.boxed, &pending).await {
        eprintln!("cloud: the mark of the reissue was not written: {e}");
    }

    // Step 7: the key is in force; the old escrow and shares go.
    if let Err(e) = finish(env, &pending).await {
        eprintln!("cloud: the reissue is finished at the next start: {e}");
    }
    Ok(Issued { words, generation })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escrow::testing::{signer, FakeVstore, MemoryBox};
    use std::sync::{Arc, Mutex};
    use veydan_core::{db, Schema};
    use veydan_lock::Lock;
    use veydan_shell::seams::Signer;
    use veydan_sync_host::config::{save_binding, VaultBinding};

    const NSEC: [u8; 32] = [0x42; 32];
    const PIN: &str = "4821";
    const LOCK: Schema = Schema {
        module: "lock",
        steps: veydan_lock::SCHEMA_STEPS,
    };

    /// A device with a data file, a lock and a binding to a vault.
    struct Device {
        db: sqlx::Pool<sqlx::Sqlite>,
        lock: Lock,
        signer: Arc<dyn Signer>,
        vstore: Arc<FakeVstore>,
        store: crate::escrow::HttpEscrowStore,
        boxed: MemoryBox,
        _dir: tempfile::TempDir,
    }

    impl Device {
        async fn new(with_lock: bool) -> Self {
            let dir = tempfile::tempdir().expect("dir");
            let db = db::open(&dir.path().join(db::DB_FILE), &[veydan_core::SCHEMA, LOCK])
                .await
                .expect("db");
            let lock = Lock::new(db.clone());
            lock.open_default().await.expect("open");
            if with_lock {
                lock.set(Some(PIN.into()), None, Some("pin".into()), None)
                    .await
                    .expect("set");
            }
            save_binding(
                &db,
                &VaultBinding {
                    vault_id: "3f2a9c1e-5b7d-4e8a-9c0b-1d2e3f4a5b6c".into(),
                    vmk_b64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        [0x77; 32],
                    ),
                },
            )
            .await
            .expect("binding");
            let signer = signer(NSEC);
            let vstore = Arc::new(FakeVstore::default());
            *vstore.identity.lock().expect("identity") =
                Some(signer.pubkey().await.expect("pubkey"));
            let store = crate::escrow::HttpEscrowStore::new(vstore.clone(), signer.clone());
            Self {
                db,
                lock,
                signer,
                vstore,
                store,
                boxed: MemoryBox::default(),
                _dir: dir,
            }
        }

        fn sources(&self) -> Sources<'_> {
            Sources {
                lock: &self.lock,
                db: &self.db,
                signer: self.signer.as_ref(),
            }
        }

        fn env<'a>(&'a self, shares: &'a dyn ShareDelivery) -> Env<'a> {
            Env {
                sources: self.sources(),
                store: &self.store,
                boxed: &self.boxed,
                shares,
            }
        }

        fn objects(&self) -> Vec<String> {
            let mut keys: Vec<String> = self
                .vstore
                .objects
                .lock()
                .expect("objects")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        }

        /// The escrow at the address of `words`, opened with them.
        async fn escrow_of(&self, words: &str) -> veydan_recovery::Escrow {
            let rk = RecoveryKey::from_words(words).expect("rk");
            let bytes = self
                .store
                .get(&rk.escrow_auth().expect("auth"))
                .await
                .expect("get");
            escrow::open(&rk, bytes).await.expect("open")
        }
    }

    /// Shares that fail when told to, remember the revokes, and may reset
    /// the lock from under the reissue while delivering (another path of
    /// the app between step 5 and step 6).
    #[derive(Default)]
    struct Shares {
        fail: Mutex<bool>,
        reset_lock: Mutex<Option<Lock>>,
        delivered: Mutex<Vec<u32>>,
        revoked: Mutex<Vec<u32>>,
    }

    impl ShareDelivery for Shares {
        fn deliver<'a>(&'a self, _rk: &'a RecoveryKey, generation: u32) -> SeamFuture<'a, ()> {
            Box::pin(async move {
                if *self.fail.lock().expect("fail") {
                    return Err(CloudError::new("transport", "the relay is away"));
                }
                let other = self.reset_lock.lock().expect("reset").take();
                if let Some(other) = other {
                    other.open_default().await.expect("open");
                    other
                        .reset(PIN, |_| Box::pin(async { Ok(()) }))
                        .await
                        .expect("reset");
                }
                self.delivered.lock().expect("delivered").push(generation);
                Ok(())
            })
        }
        fn revoke(&self, generation: u32) -> SeamFuture<'_, ()> {
            self.revoked.lock().expect("revoked").push(generation);
            Box::pin(async { Ok(()) })
        }
    }

    /// §4: the escrow names the storage the vault of this device lives
    /// in, with the access to it, and follows the settings of sync.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_escrow_names_the_storage_of_the_device_and_follows_it() {
        use veydan_recovery::Storage;
        use veydan_sync_host::config::{load_config, save_config};

        let device = Device::new(true).await;
        let mut cfg = load_config(&device.db).await;
        cfg.backend = "s3".into();
        cfg.s3.endpoint = "https://s3.example.com".into();
        cfg.s3.bucket = "notes".into();
        cfg.s3.access_key = "AKIAEXAMPLE".into();
        cfg.s3.secret_key = Some("s3-secret".into());
        cfg.s3.path_style = true;
        save_config(&device.db, &cfg).await.expect("save");
        let shares = Shares::default();
        let env = device.env(&shares);
        let first = first_setup(&env, PIN).await.expect("first");
        let s3 = Storage::S3 {
            endpoint: "https://s3.example.com".into(),
            region: String::new(),
            bucket: "notes".into(),
            prefix: String::new(),
            access_key: "AKIAEXAMPLE".into(),
            secret_key: "s3-secret".into(),
            path_style: true,
        };
        assert_eq!(device.escrow_of(&first.words).await.sync.storage, s3);
        assert!(!escrow::refresh(&env.sources, env.store, env.boxed, false).await.expect("same"));

        // The vault moved: the settings of sync name another storage, and
        // the escrow is written again with it. A secret the form did not
        // send (`None` keeps the stored one) is the stored one.
        let mut moved = load_config(&device.db).await;
        moved.backend = "webdav".into();
        moved.webdav.url = "https://dav.example.com/veydan".into();
        moved.webdav.username = "anna".into();
        moved.webdav.password = Some("dav-secret".into());
        save_config(&device.db, &moved).await.expect("save");
        moved.webdav.password = None;
        save_config(&device.db, &moved).await.expect("save");
        assert!(escrow::refresh(&env.sources, env.store, env.boxed, false).await.expect("moved"));
        let webdav = Storage::WebDav {
            url: "https://dav.example.com/veydan".into(),
            username: "anna".into(),
            password: "dav-secret".into(),
        };
        assert_eq!(device.escrow_of(&first.words).await.sync.storage, webdav);

        // The settings a restored device takes are the ones the escrow carries.
        let mut restored = veydan_sync_host::SyncConfig::default();
        escrow::apply_storage(&mut restored, &s3);
        assert_eq!(restored.backend, "s3");
        assert_eq!(restored.s3.secret_key.as_deref(), Some("s3-secret"));
        assert!(restored.s3.has_secret_key && restored.s3.path_style);
        escrow::apply_storage(&mut restored, &webdav);
        assert_eq!(restored.backend, "webdav");
        assert_eq!(restored.webdav.password.as_deref(), Some("dav-secret"));
        escrow::apply_storage(&mut restored, &Storage::Cloud);
        assert_eq!(restored.backend, veydan_sync_host::config::CLOUD_BACKEND);
        escrow::apply_storage(&mut restored, &Storage::Folder { path: "/sync".into() });
        assert_eq!((restored.backend.as_str(), restored.folder_path.as_str()), ("folder", "/sync"));
    }

    /// A device with an empty box whose server says the recovery is
    /// configured: the key was issued elsewhere, and issuing here is refused
    /// until the owner says so (KA1-02). The lock row cannot tell (`set`
    /// issues words of version 2 everywhere). After the detour the old key
    /// is not revoked: its escrow stays and the old words still open it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_without_the_box_does_not_issue_a_second_key_by_accident() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        assert!(!issued_elsewhere(&env, false).await.expect("fresh"), "the lock row of version 2 proves nothing");
        let first = first_setup(&env, PIN).await.expect("first");
        assert!(!issued_elsewhere(&env, true).await.expect("here"), "the state is in the box");

        // The same lock row on a device with an empty box.
        let empty = MemoryBox::default();
        let other = Env {
            sources: device.sources(),
            store: &device.store,
            boxed: &empty,
            shares: &shares,
        };
        refuse_elsewhere(&other, false).await.expect("a server that knows nothing refuses nothing");
        assert_eq!(
            refuse_elsewhere(&other, true).await.err().map(|e| e.code).as_deref(),
            Some(words::ISSUED_ELSEWHERE)
        );

        // «Выпустить заново»: the owner's word; the old key is not revoked.
        let again = reissue(&other, PIN).await.expect("issued anew");
        assert_eq!(again.generation, 1);
        assert!(device.objects().len() == 2, "the escrow of the first key stays until it is deleted");
        let old = device.escrow_of(&first.words).await;
        assert_eq!(old.nsec, NSEC, "the old words still open the old escrow");
        device.db.close().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_first_issue_and_a_reissue_leave_one_escrow_and_one_key_in_force() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        // The words the lock issued at `set` are unknown to the cloud: a new key.
        let first = first_setup(&env, PIN).await.expect("first");
        assert_eq!(first.generation, 1);
        assert_eq!(first.words.split(' ').count(), 12);
        device.lock.recovery_check(&first.words).await.expect("the new words open the lock");
        assert_eq!(device.objects().len(), 1);
        let state = escrow::load_state(&device.boxed).await.expect("state").expect("written");
        assert_eq!(device.objects(), vec![state.address.clone()]);
        assert_eq!(state.sealed_sha256.as_deref().map(str::len), Some(64));
        assert_eq!(
            first_setup(&env, PIN).await.err().map(|e| e.code).as_deref(),
            Some(words::ALREADY_SET_UP)
        );
        // The escrow carries the key of the lock: it opens with the words.
        let opened = device.escrow_of(&first.words).await;
        let (key, vault_id) = device.lock.require_open().expect("open");
        assert_eq!(opened.lock.as_ref().map(|l| l.vault_id.as_str()), Some(vault_id.as_str()));
        assert_eq!(opened.lock.as_ref().map(|l| l.vault_key), Some(*key.to_bytes()));
        assert_eq!(opened.nsec, NSEC);
        assert_eq!(opened.sync.vmk, [0x77; 32]);
        // Neither the words nor the key in the box show in a debug print.
        let pending = Pending {
            rk_hex: Zeroizing::new("00".repeat(16)),
            generation: 1,
            address: "a".into(),
            fingerprint: "f".into(),
            sealed_sha256: "s".into(),
            written_at: 1,
            started_at: 1,
            committed: false,
        };
        assert!(!format!("{pending:?}").contains("0000"));
        assert!(!format!("{first:?}").contains(first.words.split(' ').next().unwrap_or("?")));

        // A wrong secret of the lock issues nothing.
        let refused = reissue(&env, "0000").await.expect_err("refused");
        assert_eq!(refused.code, code::WRONG_PASSWORD);
        assert_eq!(device.objects(), vec![state.address.clone()]);

        let second = reissue(&env, PIN).await.expect("reissue");
        assert_eq!(second.generation, 2);
        assert_ne!(second.words, first.words);
        device.lock.recovery_check(&second.words).await.expect("the new words");
        assert_eq!(
            device.lock.recovery_check(&first.words).await.unwrap_err(),
            veydan_lock::Error::RecoveryInvalid
        );
        let after = escrow::load_state(&device.boxed).await.expect("state").expect("written");
        assert_ne!(after.address, state.address);
        assert_eq!(device.objects(), vec![after.address.clone()], "the old escrow is gone");
        assert_eq!(*shares.delivered.lock().expect("delivered"), vec![1, 2]);
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![1]);
        assert_eq!(escrow::load_generation(&device.boxed).await.expect("generation"), 2);
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        device.db.close().await;
    }

    /// A failure at step 5 undoes step 4 and leaves the old key in force;
    /// a reissue found under way before step 6 is rolled back at the start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failure_before_step_6_changes_nothing() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        let first = first_setup(&env, PIN).await.expect("first");
        let before = escrow::load_state(&device.boxed).await.expect("state").expect("written");

        *shares.fail.lock().expect("fail") = true;
        let failed = reissue(&env, PIN).await.expect_err("fails");
        assert_eq!(failed.code, "transport");
        device.lock.recovery_check(&first.words).await.expect("the old words still open");
        assert_eq!(device.objects(), vec![before.address.clone()]);
        assert_eq!(escrow::load_generation(&device.boxed).await.expect("generation"), 1);
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![2]);

        // A reissue interrupted between step 4 and 6: rolled back at the start.
        let orphan = "ab".repeat(32);
        device
            .vstore
            .objects
            .lock()
            .expect("objects")
            .insert(orphan.clone(), b"new".to_vec());
        save_pending(
            &device.boxed,
            &Pending {
                rk_hex: Zeroizing::new("00".repeat(16)),
                generation: 2,
                address: orphan,
                fingerprint: "f".into(),
                sealed_sha256: "s".into(),
                written_at: 1,
                started_at: 1,
                committed: false,
            },
        )
        .await
        .expect("pending");
        assert_eq!(
            reissue(&env, PIN).await.err().map(|e| e.code).as_deref(),
            Some(words::REISSUE_PENDING)
        );
        assert!(resume(&env).await.expect("resume"));
        assert!(!resume(&env).await.expect("resume"));
        assert_eq!(device.objects(), vec![before.address]);
        device.lock.recovery_check(&first.words).await.expect("the old words");
        device.db.close().await;
    }

    /// Step 4 refused by vstore (no account of the identity: 403): the
    /// reissue fails, nothing is written, the old key stays in force.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_put_vstore_refuses_undoes_the_reissue() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        let first = first_setup(&env, PIN).await.expect("first");
        let before = escrow::load_state(&device.boxed).await.expect("state").expect("written");

        *device.vstore.identity.lock().expect("identity") = None;
        let failed = reissue(&env, PIN).await.expect_err("refused");
        assert_eq!(failed.code, "no_account");
        assert_eq!(device.objects(), vec![before.address.clone()]);
        device.lock.recovery_check(&first.words).await.expect("the old words");
        assert_eq!(escrow::load_generation(&device.boxed).await.expect("generation"), 1);
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        assert_eq!(*shares.delivered.lock().expect("delivered"), vec![1]);
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![2]);
        device.db.close().await;
    }

    /// Step 6 refused: the key row changed between the wrap in memory and
    /// its write (a reset by another path of the app while the shares were
    /// delivered). The new escrow goes, the generation stays.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_key_row_that_changed_before_step_6_undoes_the_reissue() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        first_setup(&env, PIN).await.expect("first");
        let before = escrow::load_state(&device.boxed).await.expect("state").expect("written");

        *shares.reset_lock.lock().expect("reset") = Some(Lock::new(device.db.clone()));
        let failed = reissue(&env, PIN).await.expect_err("refused");
        assert_eq!(failed.code, code::BAD_REQUEST);
        assert!(failed.message.contains("key row changed"));
        assert_eq!(device.objects(), vec![before.address.clone()]);
        assert_eq!(escrow::load_generation(&device.boxed).await.expect("generation"), 1);
        assert_eq!(
            escrow::load_state(&device.boxed).await.expect("state").expect("kept"),
            before
        );
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        assert_eq!(*shares.delivered.lock().expect("delivered"), vec![1, 2]);
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![2]);
        device.db.close().await;
    }

    /// After step 6 the new key is in force whatever the box does: the words
    /// come back, a start finishes the rest and never deletes the escrow
    /// that matches the wrap in the row.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn after_step_6_the_words_come_back_and_the_start_finishes_the_reissue() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        let first = first_setup(&env, PIN).await.expect("first");
        let before = escrow::load_state(&device.boxed).await.expect("state").expect("written");

        // The box refuses the key after the row took the wrap.
        *device.boxed.fail_put.lock().expect("fail") = Some(escrow::KEY_RK.to_string());
        let second = reissue(&env, PIN).await.expect("the words come back");
        assert_eq!(second.generation, 2);
        device.lock.recovery_check(&second.words).await.expect("the new words open the lock");
        assert_eq!(
            device.lock.recovery_check(&first.words).await.unwrap_err(),
            veydan_lock::Error::RecoveryInvalid
        );
        let pending = load_pending(&device.boxed).await.expect("pending").expect("kept");
        assert!(pending.committed);
        // Not finished yet: the box still holds the old key and both escrows stand.
        assert_eq!(
            escrow::load_rk(&device.boxed).await.expect("rk"),
            Some(RecoveryKey::from_words(&first.words).expect("rk"))
        );
        assert_eq!(device.objects().len(), 2);
        assert_eq!(
            reissue(&env, PIN).await.err().map(|e| e.code).as_deref(),
            Some(words::REISSUE_PENDING)
        );

        // The start finishes forward.
        assert!(resume(&env).await.expect("resume"));
        assert_eq!(
            escrow::load_rk(&device.boxed).await.expect("rk"),
            Some(RecoveryKey::from_words(&second.words).expect("rk"))
        );
        assert_eq!(escrow::load_generation(&device.boxed).await.expect("generation"), 2);
        let after = escrow::load_state(&device.boxed).await.expect("state").expect("written");
        assert_eq!(after.address, pending.address);
        assert_ne!(after.address, before.address);
        assert_eq!(device.objects(), vec![after.address.clone()], "the old escrow is gone");
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![1]);
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        let opened = device.escrow_of(&second.words).await;
        assert_eq!(opened.generation, 2);

        // The mark itself not written: the row tells the start the key is in force.
        let mut unmarked = pending.clone();
        unmarked.committed = false;
        save_pending(&device.boxed, &unmarked).await.expect("pending");
        assert!(resume(&env).await.expect("resume"));
        assert_eq!(device.objects(), vec![after.address]);
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        device.db.close().await;
    }

    /// Step 7's delete refused by vstore: the old escrow stays retired in
    /// the box, the pass of the start and the day deletes it, and the old
    /// words read nothing after; the trusted people are told once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_delete_refused_after_step_6_is_made_again_until_the_old_words_open_nothing() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        let first = first_setup(&env, PIN).await.expect("first");
        let before = escrow::load_state(&device.boxed).await.expect("state").expect("written");

        *device.vstore.fail_delete.lock().expect("fail") = true;
        let second = reissue(&env, PIN).await.expect("the words come back");
        assert!(load_pending(&device.boxed).await.expect("pending").is_none());
        let retired = vec![Retired {
            address: before.address.clone(),
            generation: 1,
            revoked: true,
        }];
        assert_eq!(load_retired(&device.boxed).await.expect("retired"), retired);
        assert_eq!(device.escrow_of(&first.words).await.generation, 1, "the old escrow stands still");

        // The pass of the start: nothing under way, the delete refused again.
        assert!(!resume(&env).await.expect("resume"));
        assert_eq!(retire(&env).await.expect("retire"), 1);
        assert_eq!(load_retired(&device.boxed).await.expect("retired"), retired);

        // The daily pass, the host back: the old escrow goes, the box forgets it.
        *device.vstore.fail_delete.lock().expect("fail") = false;
        assert_eq!(retire(&env).await.expect("retire"), 0);
        assert!(device.boxed.values.lock().expect("values").get(KEY_RETIRED).is_none());
        let after = escrow::load_state(&device.boxed).await.expect("state").expect("written");
        assert_eq!(device.objects(), vec![after.address.clone()]);
        let rk = RecoveryKey::from_words(&first.words).expect("rk");
        let read = device.store.get(&rk.escrow_auth().expect("auth")).await;
        assert_eq!(read.err().map(|e| e.code).as_deref(), Some(words::NOT_FOUND));
        assert_eq!(device.escrow_of(&second.words).await.generation, 2);
        assert_eq!(*shares.revoked.lock().expect("revoked"), vec![1]);

        // A retired address with nothing there (`404`) is gone as well; the
        // escrow in force is never deleted, whatever the list says.
        save_retired(
            &device.boxed,
            &[
                Retired { address: "cd".repeat(32), generation: 1, revoked: true },
                Retired { address: after.address.clone(), generation: 2, revoked: true },
            ],
        )
        .await
        .expect("retired");
        assert_eq!(retire(&env).await.expect("retire"), 0);
        assert_eq!(device.objects(), vec![after.address]);
        assert!(load_retired(&device.boxed).await.expect("retired").is_empty());
        device.db.close().await;
    }

    /// The box erased by the deletion of the account: a new activation of
    /// the same npub issues a key anew from generation 1, not
    /// `already_set_up`, and the reissue after it works as ever.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn after_the_account_is_deleted_a_key_is_issued_anew() {
        let device = Device::new(true).await;
        let shares = Shares::default();
        let env = device.env(&shares);
        first_setup(&env, PIN).await.expect("first");
        *device.vstore.fail_delete.lock().expect("fail") = true;
        reissue(&env, PIN).await.expect("reissue");
        assert_eq!(load_retired(&device.boxed).await.expect("retired").len(), 1);
        *device.vstore.fail_delete.lock().expect("fail") = false;

        crate::delete::erase_box(&device.boxed).await.expect("erased");
        let values = device.boxed.values.lock().expect("values").clone();
        for key in crate::delete::RECOVERY_KEYS {
            assert!(!values.contains_key(key), "{key} is gone");
        }
        let again = first_setup(&env, PIN).await.expect("issued anew");
        assert_eq!(again.generation, 1);
        device.lock.recovery_check(&again.words).await.expect("the words open the lock");
        let next = reissue(&env, PIN).await.expect("reissue");
        assert_eq!(next.generation, 2);
        assert_eq!(device.escrow_of(&next.words).await.generation, 2);
        let replaced = RecoveryKey::from_words(&again.words)
            .expect("rk")
            .escrow_auth()
            .expect("auth")
            .escrow_address();
        assert!(!device.objects().contains(&replaced), "the escrow of the first key is gone");
        assert!(load_retired(&device.boxed).await.expect("retired").is_empty());
        device.db.close().await;
    }

    /// Without a lock the escrow says `lock: null`, and no row is wrapped.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_device_without_a_lock_issues_a_key_for_the_escrow_alone() {
        let device = Device::new(false).await;
        let env = device.env(&NoShares);
        let issued = first_setup(&env, "").await.expect("first");
        let opened = device.escrow_of(&issued.words).await;
        assert_eq!(opened.lock, None);
        assert!(!device.lock.status().await.has_recovery);
        device.db.close().await;
    }

    /// §4: a change of the lock after the issue — a reset, a new key —
    /// writes the escrow again under the same words, with the key in use;
    /// nothing changed, nothing is written; the daily check writes again
    /// what another device overwrote.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_changed_lock_writes_the_escrow_again() {
        let device = Device::new(true).await;
        let env = device.env(&NoShares);
        let issued = first_setup(&env, PIN).await.expect("first");
        let sources = device.sources();
        assert!(!escrow::refresh(&sources, &device.store, &device.boxed, false)
            .await
            .expect("refresh"));

        device
            .lock
            .reset(PIN, |_| Box::pin(async { Ok(()) }))
            .await
            .expect("reset");
        let (key, vault_id) = device.lock.require_open().expect("open");
        assert!(escrow::refresh(&sources, &device.store, &device.boxed, false)
            .await
            .expect("refresh"));
        let opened = device.escrow_of(&issued.words).await;
        assert_eq!(opened.lock.as_ref().map(|l| l.vault_id.as_str()), Some(vault_id.as_str()));
        assert_eq!(opened.lock.as_ref().map(|l| l.vault_key), Some(*key.to_bytes()));
        assert_eq!(device.objects().len(), 1);

        assert!(!escrow::check(&sources, &device.store, &device.boxed)
            .await
            .expect("check"));
        let state = escrow::load_state(&device.boxed).await.expect("state").expect("written");
        device
            .vstore
            .objects
            .lock()
            .expect("objects")
            .insert(state.address.clone(), b"stale".to_vec());
        assert!(escrow::check(&sources, &device.store, &device.boxed)
            .await
            .expect("check"));
        let opened = device.escrow_of(&issued.words).await;
        assert_eq!(opened.lock.as_ref().map(|l| l.vault_key), Some(*key.to_bytes()));

        // Without a key in the box nothing is written.
        device.boxed.values.lock().expect("values").remove(escrow::KEY_RK);
        assert!(!escrow::refresh(&sources, &device.store, &device.boxed, true)
            .await
            .expect("refresh"));
        device.db.close().await;
    }
}

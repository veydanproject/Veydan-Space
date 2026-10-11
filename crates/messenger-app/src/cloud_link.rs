// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The messenger's side of the linking of devices
//! (internal/cloud-recovery.md §6, internal/messenger-wire.md §3): the notes
//! to and from a temporary key T, the typed code on the relays, the nsec
//! handed over after the lock's secret and taken over on the new device,
//! and the deletions at the end. The relays are the runtime's
//! (`messenger_runtime::tempkey`); this file speaks the words of the seams.
//!
//! [`LinkWire`] serves `PeerNotes::send_to_key`, `listen_as` and the note
//! `device.link.done` of `send_own` (through `ServiceNotes`), and is the
//! seam [`DeviceLink`] itself. The outer keys of the wraps it sent are kept
//! in memory until [`DeviceLink::withdraw`]: a restart of the app forgets
//! them, and the `expiration` of the wrap is then the only cleaning.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use messenger_core::envelope::{Envelope, KIND_LINK_CODE, T_DEVICE_LINK, T_DEVICE_LINK_DONE};
use messenger_core::{EventId, MessengerError, PubKey};
use messenger_runtime::tempkey::{self, OWN_SERVERS};
use messenger_runtime::MessengerRuntime;
use nostr::prelude::{Keys, SecretKey};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use veydan_lock::Lock;
use veydan_shell::seams::{
    code, CloudError, DeviceLink, LinkCodeFound, Note, SeamFuture, TempKey, LINK_TTL_SECS,
    TO_KEY_TYPES,
};
use zeroize::Zeroizing;

use crate::commands::MessengerState;

/// The text of `veydan_lock::Error::Other` for a secret that is not the
/// lock's (`ERR_WRONG` of crates/lock); any other `Other` is a failure.
const LOCK_WRONG_PASSWORD: &str = "Wrong password";

/// The kind of a gift wrap, for the deletion of one.
const KIND_GIFT_WRAP: u16 = 1059;

/// How many notes to T wait for a slow reader.
const NOTES_QUEUE: usize = 16;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn bad_request(message: impl Into<String>) -> CloudError {
    CloudError::new(code::BAD_REQUEST, message)
}

/// What the runtime said, in the words of the seams: a relay out of reach
/// is `no_relay`, the user's own servers `no_host`.
pub fn from_runtime(e: MessengerError) -> CloudError {
    match e {
        MessengerError::Transport(m) => CloudError::new(code::NO_RELAY, m),
        MessengerError::Invalid(m) if m == OWN_SERVERS => {
            CloudError::new(code::NO_HOST, "the servers in use are the user's own")
        }
        MessengerError::NotLoggedIn => {
            CloudError::new(code::NO_IDENTITY, "no identity on this device")
        }
        MessengerError::SecretsLocked => CloudError::new(code::LOCKED, "the lock is closed"),
        other => CloudError::new(code::CRYPTO, other.to_string()),
    }
}

/// What the lock said to a secret, in the words of the seams.
fn from_lock(e: veydan_lock::Error) -> CloudError {
    match e {
        veydan_lock::Error::Other(m) if m == LOCK_WRONG_PASSWORD => {
            CloudError::new(code::WRONG_PASSWORD, "the secret is not the lock's")
        }
        veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
            CloudError::new(code::LOCKED, "the lock is closed")
        }
        other => CloudError::new(code::CRYPTO, other.to_string()),
    }
}

/// A key as the contract writes it: 64 hex, lower case.
fn pubkey(hex: &str) -> Result<PubKey, CloudError> {
    let lower = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    PubKey::parse(hex)
        .filter(|_| lower)
        .ok_or_else(|| bad_request("a key is 64 hex characters in lower case"))
}

/// The keys of a secret of 32 bytes; the copy is wiped with the keys.
fn keys_of(secret: &[u8; 32]) -> Result<Keys, CloudError> {
    SecretKey::from_slice(secret)
        .map(Keys::new)
        .map_err(|_| CloudError::new(code::CRYPTO, "not a key of secp256k1"))
}

/// The `expiration` of a note to a key (messenger-wire.md §3): `device.link`
/// at most ten minutes ahead, ten when none is asked; `recovery.answer` the
/// `expires_at` of its request, which the caller gives.
pub fn expiration_of(note: &Note, now: u64) -> Result<u64, CloudError> {
    match (note.t.as_str(), note.expires_at) {
        (T_DEVICE_LINK, None) => Ok(now + LINK_TTL_SECS),
        (T_DEVICE_LINK, Some(at)) if at > now && at <= now + LINK_TTL_SECS => Ok(at),
        (T_DEVICE_LINK, Some(_)) => Err(bad_request("a device link expires within ten minutes")),
        (_, Some(at)) if at > now => Ok(at),
        (_, Some(_)) => Err(bad_request("the note has expired already")),
        (t, None) => Err(bad_request(format!(
            "a note of type {t:?} names its expiration"
        ))),
    }
}

/// The envelope of a note: `{"v":1,"t":…,…fields}`.
pub fn envelope_of(note: &Note) -> Envelope {
    let mut envelope = Envelope::new(&note.t);
    envelope.fields = note.fields.clone();
    envelope.fields.remove("v");
    envelope.fields.remove("t");
    envelope
}

/// A note that came to T, as the seams give it: only of [`TO_KEY_TYPES`].
pub fn note_of(envelope: Envelope, from: &PubKey) -> Option<Note> {
    if !TO_KEY_TYPES.contains(&envelope.t.as_str()) {
        return None;
    }
    let mut note = Note::new(envelope.t, envelope.fields);
    note.from = Some(from.as_hex().to_string());
    Some(note)
}

/// A wrap sent to a key T and the key that signed its outside.
struct SentWrap {
    id: EventId,
    wrapper: Keys,
}

/// Where the runtime and the lock of a link come from.
enum Source {
    /// The app: its running messenger and its lock.
    App(AppHandle),
    /// A runtime and a lock held directly, without an app (the tests).
    #[cfg(test)]
    Parts(Arc<MessengerRuntime>, Arc<Lock>),
    /// Nothing behind it.
    Detached,
}

/// The messenger's side of a link. Without an app (tests) every call that
/// needs the runtime is `messenger_stopped`.
pub struct LinkWire {
    source: Source,
    sent: Mutex<HashMap<String, Vec<SentWrap>>>,
}

fn guard<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn stopped(message: &str) -> CloudError {
    CloudError::new(code::MESSENGER_STOPPED, message)
}

impl LinkWire {
    fn with_source(source: Source) -> Self {
        Self {
            source,
            sent: Mutex::new(HashMap::new()),
        }
    }

    /// The side of a link of the running messenger of `app`.
    pub fn new(app: AppHandle) -> Self {
        Self::with_source(Source::App(app))
    }

    /// A side with no messenger behind it.
    pub fn detached() -> Self {
        Self::with_source(Source::Detached)
    }

    /// A side over a runtime and a lock without an app.
    #[cfg(test)]
    fn with_parts(runtime: Arc<MessengerRuntime>, lock: Arc<Lock>) -> Self {
        Self::with_source(Source::Parts(runtime, lock))
    }

    fn runtime(&self) -> Result<Arc<MessengerRuntime>, CloudError> {
        match &self.source {
            Source::App(app) => {
                let state = app
                    .try_state::<MessengerState>()
                    .ok_or_else(|| stopped("the messenger is not in the product"))?;
                state.runtime().map_err(|e| stopped(&e.to_string()))
            }
            #[cfg(test)]
            Source::Parts(runtime, _) => Ok(runtime.clone()),
            Source::Detached => Err(stopped("the messenger does not run")),
        }
    }

    fn lock(&self) -> Result<&Lock, CloudError> {
        match &self.source {
            Source::App(app) => app
                .try_state::<Lock>()
                .map(|state| state.inner())
                .ok_or_else(|| CloudError::new(code::LOCKED, "the app has no lock in place")),
            #[cfg(test)]
            Source::Parts(_, lock) => Ok(lock),
            Source::Detached => Err(stopped("the messenger does not run")),
        }
    }

    /// A note to the key `to`, checked by `ServiceNotes` already: wrapped by
    /// the runtime, its outer key kept for [`DeviceLink::withdraw`].
    pub async fn send_to_key(&self, note: &Note, to: &str) -> Result<(), CloudError> {
        let to_key = pubkey(to)?;
        let expiration = expiration_of(note, now())?;
        let rt = self.runtime()?;
        let kept = rt
            .send_note_to_key(&to_key, &envelope_of(note), expiration as i64)
            .await
            .map_err(from_runtime)?;
        guard(&self.sent)
            .entry(to.to_string())
            .or_default()
            .push(SentWrap {
                id: kept.wrap.id,
                wrapper: kept.wrapper,
            });
        Ok(())
    }

    /// A note to my own devices: `device.link.done` alone goes this way.
    pub async fn send_own(&self, note: &Note) -> Result<(), CloudError> {
        if note.t != T_DEVICE_LINK_DONE {
            return Err(CloudError::new(
                code::NOT_SUPPORTED,
                format!(
                    "the messenger does not send {:?} to its own devices in this version",
                    note.t
                ),
            ));
        }
        let t_pub = note
            .fields
            .get("t_pub")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        pubkey(t_pub)?;
        let rt = self.runtime()?;
        rt.send_own_note_now(&envelope_of(note))
            .await
            .map_err(from_runtime)?;
        Ok(())
    }

    /// The notes of [`TO_KEY_TYPES`] that come to T, the signer of each
    /// seal in `from`. T goes into the runtime's listener and is wiped
    /// with it when the receiver is dropped.
    pub async fn listen_as(&self, t: TempKey) -> Result<mpsc::Receiver<Note>, CloudError> {
        let rt = self.runtime()?;
        let keys = keys_of(t.secret_bytes())?;
        drop(t);
        let mut heard = rt.listen_as_key(keys).await.map_err(from_runtime)?;
        let (tx, rx) = mpsc::channel(NOTES_QUEUE);
        tauri::async_runtime::spawn(async move {
            loop {
                let next = tokio::select! {
                    _ = tx.closed() => break,
                    next = heard.recv() => next,
                };
                let Some(key_note) = next else { break };
                let Some(note) = note_of(key_note.envelope, &key_note.from) else {
                    continue;
                };
                if tx.send(note).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }
}

impl DeviceLink for LinkWire {
    fn hand_over<'a>(&'a self, password: Option<&'a str>) -> SeamFuture<'a, Zeroizing<[u8; 32]>> {
        Box::pin(async move {
            let lock = self.lock()?;
            if lock.enabled().await {
                let secret = password.filter(|p| !p.is_empty()).ok_or_else(|| {
                    CloudError::new(code::WRONG_PASSWORD, "the secret of the lock is needed")
                })?;
                // An unlock: the counter of failures and its pause apply.
                lock.unlock(secret).await.map_err(from_lock)?;
            }
            let keys = self
                .runtime()?
                .identity()
                .load_keys()
                .await
                .map_err(from_runtime)?;
            Ok(Zeroizing::new(keys.secret_key().to_secret_bytes()))
        })
    }

    fn take_over<'a>(
        &'a self,
        nsec: Zeroizing<[u8; 32]>,
        password: Option<&'a str>,
    ) -> SeamFuture<'a, String> {
        Box::pin(async move {
            let keys = keys_of(&nsec)?;
            drop(nsec);
            let me = keys.public_key().to_hex();
            let rt = self.runtime()?;
            if let Some(identity) = rt.identity().get().await.map_err(from_runtime)? {
                return if identity.pubkey.as_hex() == me {
                    Ok(me)
                } else {
                    Err(CloudError::new(
                        code::IDENTITY_EXISTS,
                        "another identity is on this device",
                    ))
                };
            }
            let lock = self.lock()?;
            if lock.is_locked().await {
                let secret = password
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| CloudError::new(code::LOCKED, "the lock is closed"))?;
                lock.unlock(secret).await.map_err(from_lock)?;
            }
            let secret = Zeroizing::new(keys.secret_key().to_secret_hex());
            rt.identity()
                .import_nsec(&secret)
                .await
                .map_err(from_runtime)?;
            rt.refresh_signer().await.map_err(from_runtime)?;
            // The link came over the relays of the project (the new device
            // listened by the manifest in use), so a device that has not
            // chosen its servers yet is put on the project's: no second step
            // of the onboarding after «У меня уже есть устройство» (E1). The
            // manifest check needs the network; a failure there does not
            // undo the link.
            if rt.relays().servers_mode().await.map_err(from_runtime)?.is_none() {
                if let Err(e) = rt.servers_use_veydan().await {
                    eprintln!("cloud link: the servers of the project were not applied: {e}");
                }
            }
            Ok(me)
        })
    }

    fn publish_code<'a>(
        &'a self,
        t: &'a TempKey,
        d: &'a str,
        expires_at: u64,
    ) -> SeamFuture<'a, String> {
        Box::pin(async move {
            let keys = keys_of(t.secret_bytes())?;
            let rt = self.runtime()?;
            let id = rt
                .publish_link_code(&keys, d, expires_at as i64)
                .await
                .map_err(from_runtime)?;
            Ok(id.as_hex().to_string())
        })
    }

    fn find_code<'a>(&'a self, d: &'a str) -> SeamFuture<'a, Option<LinkCodeFound>> {
        Box::pin(async move {
            let found = self
                .runtime()?
                .find_link_code(d)
                .await
                .map_err(from_runtime)?;
            Ok(found.map(|f| LinkCodeFound {
                t_pub: f.pubkey.as_hex().to_string(),
                expires_at: f.expiration,
            }))
        })
    }

    fn delete_code<'a>(&'a self, t: &'a TempKey, event_id: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let id =
                EventId::parse(event_id).ok_or_else(|| bad_request("an event id is 64 hex"))?;
            let keys = keys_of(t.secret_bytes())?;
            let rt = self.runtime()?;
            rt.delete_events(&keys, &[id], KIND_LINK_CODE)
                .await
                .map_err(from_runtime)
        })
    }

    fn await_done<'a>(&'a self, t_pub: &'a str, until: u64) -> SeamFuture<'a, bool> {
        Box::pin(async move {
            pubkey(t_pub)?;
            let rt = self.runtime()?;
            rt.await_own_note(T_DEVICE_LINK_DONE, "t_pub", t_pub, until as i64)
                .await
                .map_err(from_runtime)
        })
    }

    fn withdraw<'a>(&'a self, t_pub: &'a str) -> SeamFuture<'a, ()> {
        Box::pin(async move {
            let sent = guard(&self.sent).remove(t_pub).unwrap_or_default();
            if sent.is_empty() {
                return Ok(());
            }
            let rt = self.runtime()?;
            let mut failed = None;
            for wrap in sent {
                if let Err(e) = rt
                    .delete_events(&wrap.wrapper, &[wrap.id], KIND_GIFT_WRAP)
                    .await
                {
                    failed = Some(from_runtime(e));
                }
            }
            failed.map_or(Ok(()), Err)
        })
    }

    fn qr_svg<'a>(&'a self, text: &'a str) -> SeamFuture<'a, String> {
        Box::pin(async move { tempkey::link_qr_svg(text).map_err(|e| bad_request(e.to_string())) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};

    fn link_note(expires_at: Option<u64>) -> Note {
        let mut fields = Map::new();
        fields.insert("blob".into(), json!("AQID"));
        let mut note = Note::new(T_DEVICE_LINK, fields);
        note.expires_at = expires_at;
        note
    }

    #[test]
    fn a_link_expires_within_ten_minutes_and_an_answer_when_its_request_does() {
        let now = 1_791_000_000;
        assert_eq!(expiration_of(&link_note(None), now), Ok(now + 600));
        assert_eq!(expiration_of(&link_note(Some(now + 60)), now), Ok(now + 60));
        for late in [Some(now + 601), Some(now), Some(now - 1)] {
            assert_eq!(
                expiration_of(&link_note(late), now).map_err(|e| e.code),
                Err(code::BAD_REQUEST.into())
            );
        }
        let mut answer = Note::new("recovery.answer", Map::new());
        assert!(
            expiration_of(&answer, now).is_err(),
            "an answer names the end of its request"
        );
        answer.expires_at = Some(now + 1800);
        assert_eq!(expiration_of(&answer, now), Ok(now + 1800));
    }

    #[test]
    fn a_note_goes_as_an_envelope_and_comes_back_with_its_sealer() {
        let mut note = link_note(None);
        note.fields.insert("t".into(), json!("forged"));
        let envelope = envelope_of(&note);
        assert_eq!(
            envelope.encode(),
            r#"{"v":1,"t":"device.link","blob":"AQID"}"#
        );
        let from = PubKey::parse(&"ab".repeat(32)).expect("key");
        let back = note_of(Envelope::parse(&envelope.encode()).expect("parse"), &from)
            .expect("a note to T");
        assert_eq!(back.t, T_DEVICE_LINK);
        assert_eq!(back.fields.get("blob"), Some(&json!("AQID")));
        assert_eq!(back.from.as_deref(), Some("ab".repeat(32).as_str()));
        // Only the notes to a key T are given; anything else that came is dropped.
        assert!(note_of(Envelope::new("recovery.share"), &from).is_none());
        assert!(note_of(Envelope::new(T_DEVICE_LINK_DONE), &from).is_none());
    }

    #[test]
    fn the_errors_of_the_runtime_and_the_lock_speak_the_words_of_the_seams() {
        assert_eq!(
            from_runtime(MessengerError::Transport("no relay connected".into())).code,
            code::NO_RELAY
        );
        assert_eq!(
            from_runtime(MessengerError::Invalid(OWN_SERVERS.into())).code,
            code::NO_HOST
        );
        assert_eq!(
            from_runtime(MessengerError::NotLoggedIn).code,
            code::NO_IDENTITY
        );
        assert_eq!(
            from_runtime(MessengerError::SecretsLocked).code,
            code::LOCKED
        );
        assert_eq!(
            from_lock(veydan_lock::Error::Other("Wrong password".into())).code,
            code::WRONG_PASSWORD
        );
        assert_eq!(
            from_lock(veydan_lock::Error::VaultLocked).code,
            code::LOCKED
        );
        // A failure of the storage is not a wrong secret.
        let disk = from_lock(veydan_lock::Error::Other("disk".into()));
        assert_eq!(
            (disk.code.as_str(), disk.message.as_str()),
            (code::CRYPTO, "disk")
        );
        assert_eq!(
            from_lock(veydan_lock::Error::Db("busy".into())).code,
            code::CRYPTO
        );
        assert_eq!(
            pubkey(&"AB".repeat(32)).err().map(|e| e.code).as_deref(),
            Some(code::BAD_REQUEST)
        );
        assert!(keys_of(&[0u8; 32]).is_err(), "zero is not a key");
    }

    #[tokio::test]
    async fn without_a_messenger_nothing_goes_and_nothing_is_kept() {
        let wire = LinkWire::detached();
        let t = "ab".repeat(32);
        let stopped = wire
            .send_to_key(&link_note(None), &t)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(stopped.as_deref(), Some(code::MESSENGER_STOPPED));
        assert!(guard(&wire.sent).is_empty());
        let listen = wire
            .listen_as(TempKey::generate().expect("key"))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(listen.as_deref(), Some(code::MESSENGER_STOPPED));
        assert!(
            wire.withdraw(&t).await.is_ok(),
            "nothing was sent: nothing to delete"
        );
        let cancel = wire
            .send_own(&Note::new("recovery.cancel", Map::new()))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(cancel.as_deref(), Some(code::NOT_SUPPORTED));
        let mut done = Map::new();
        done.insert("t_pub".into(), json!("nope"));
        let bad = wire
            .send_own(&Note::new(T_DEVICE_LINK_DONE, done))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(bad.as_deref(), Some(code::BAD_REQUEST));
        let svg = wire.qr_svg("veydan://link/ab/abandon").await.expect("svg");
        assert!(svg.contains("#ffffff"));
        assert!(wire.hand_over(None).await.is_err());
    }

    /// The box `messenger` of a lock as the store of a runtime, without the app.
    struct LockBox(Arc<Lock>);

    fn boxed(e: veydan_lock::Error) -> MessengerError {
        match e {
            veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => {
                MessengerError::SecretsLocked
            }
            other => MessengerError::Crypto(other.to_string()),
        }
    }

    #[async_trait::async_trait]
    impl messenger_core::SecretStore for LockBox {
        async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
            self.0.secret_box("messenger").get(key).await.map_err(boxed)
        }
        async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
            self.0
                .secret_box("messenger")
                .put(key, value)
                .await
                .map_err(boxed)
        }
        async fn delete(&self, key: &str) -> messenger_core::Result<()> {
            self.0
                .secret_box("messenger")
                .delete(key)
                .await
                .map_err(boxed)
        }
        async fn is_unlocked(&self) -> bool {
            self.0.secret_box("messenger").is_unlocked()
        }
    }

    /// A device as the shell opens it: the data file with the tables of the
    /// core and of the lock, a lock with `secret`, the messenger on its box.
    async fn device(dir: &std::path::Path, secret: &str) -> (Arc<Lock>, Arc<MessengerRuntime>) {
        let lock_schema = veydan_core::Schema {
            module: "lock",
            steps: veydan_lock::SCHEMA_STEPS,
        };
        let db = veydan_core::db::open(
            &dir.join(veydan_core::db::DB_FILE),
            &[veydan_core::SCHEMA, lock_schema],
        )
        .await
        .expect("db");
        let lock = Arc::new(Lock::new(db));
        lock.open_default().await.expect("open");
        lock.set(Some(secret.into()), None, Some("pin".into()), None)
            .await
            .expect("set");
        let config = messenger_core::MessengerConfig::new(dir.join("messenger"));
        let rt = MessengerRuntime::start(config, Arc::new(LockBox(lock.clone())))
            .await
            .expect("start");
        (lock, Arc::new(rt))
    }

    /// The path the user hits on a real lock and a real messenger: A gives
    /// the nsec only for the secret of its lock (counted as an unlock), B
    /// opens its closed lock first, imports the identity, and refuses
    /// another one later.
    #[tokio::test]
    async fn the_lock_guards_the_nsec_on_both_devices() {
        let (dir_a, dir_b) = (
            tempfile::tempdir().expect("dir"),
            tempfile::tempdir().expect("dir"),
        );
        let (lock_a, rt_a) = device(dir_a.path(), "1357").await;
        rt_a.identity()
            .create("backup words")
            .await
            .expect("identity");
        let me = rt_a.identity().load_keys().await.expect("keys");
        lock_a.lock();
        let a = LinkWire::with_parts(rt_a.clone(), lock_a.clone());
        for wrong in [None, Some(""), Some("0000")] {
            let refused = a.hand_over(wrong).await.err().map(|e| e.code);
            assert_eq!(refused.as_deref(), Some(code::WRONG_PASSWORD), "{wrong:?}");
        }
        assert!(lock_a.is_locked().await, "a wrong secret opens nothing");
        let nsec = a.hand_over(Some("1357")).await.expect("hand over");
        assert_eq!(*nsec, me.secret_key().to_secret_bytes());

        let (lock_b, rt_b) = device(dir_b.path(), "2468").await;
        lock_b.lock();
        let b = LinkWire::with_parts(rt_b.clone(), lock_b.clone());
        let closed = b.take_over(nsec.clone(), None).await.err().map(|e| e.code);
        assert_eq!(closed.as_deref(), Some(code::LOCKED));
        let wrong = b
            .take_over(nsec.clone(), Some("0000"))
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(wrong.as_deref(), Some(code::WRONG_PASSWORD));
        assert!(rt_b.identity().get().await.expect("get").is_none());
        let npub = b
            .take_over(nsec.clone(), Some("2468"))
            .await
            .expect("take over");
        assert_eq!(npub, me.public_key().to_hex());
        let held = rt_b.identity().get().await.expect("get").expect("identity");
        assert_eq!(held.pubkey.as_hex(), npub);
        // The same key again changes nothing; another one is refused.
        assert_eq!(b.take_over(nsec, None).await.expect("same"), npub);
        let other = b
            .take_over(Zeroizing::new([0x42; 32]), None)
            .await
            .err()
            .map(|e| e.code);
        assert_eq!(other.as_deref(), Some(code::IDENTITY_EXISTS));
        rt_a.shutdown().await;
        rt_b.shutdown().await;
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Tauri adapter for the messenger module.
//!
//! This is the only place where a product and the messenger crates meet
//! (internal/messenger-spec.md §4.5). It owns three things and nothing else:
//!
//! 1. `MessengerState`: starts and stops `messenger_runtime::MessengerRuntime`
//!    with a data dir under the app's data dir and the host `SecretStore`;
//!    the module's hooks call it when the app starts and exits and when the
//!    user switches the module on and off.
//! 2. `HostSecretStore`: `SecretStore` backed by the lock's secret box
//!    `messenger`: every value is encrypted with the vault key and kept on
//!    this device only; a closed vault yields `SecretsLocked`.
//! 3. `messenger_*` commands that forward to the runtime and translate
//!    `MessengerError` into `AppError`.
//!
//! Keep this file free of messenger logic; if something needs more than a
//! forwarding call, it belongs in a messenger crate.

pub mod push;
// Notifications of a computer: the app shows them itself while it runs.
#[cfg(desktop)]
pub mod desktop_notify;
// The push handler's entry into the messenger: a JNI export, Android only.
#[cfg(target_os = "android")]
pub mod notify_jni;
// Calls: the commands of the page, forwarded to the runtime.
pub mod calls;
// Calls on a phone: the call plugin, driven from Rust. Its decisions that
// need no phone are tested on a computer, where the module is otherwise
// absent.
#[cfg(any(mobile, test))]
pub mod call_android;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use messenger_core::{MessengerConfig, MessengerError, SecretStore};
use messenger_core::PubKey;
use messenger_runtime::{
    ChatView, ContactPatch, ContactView, CreatedIdentity, DmAction, GroupKind, GroupOp, GroupView, Identity, InviteView,
    LinkPreview, LinkView, ManifestCheck, ManifestInfo, MessageView,
    MediaKind, MediaServerInput, MediaServerView, MessengerRuntime, Recording, RelationView, TransferView,
    PresenceView, PrivacySettings, ProfileView, RelayView,
    RuntimeStatus, SharedCounts, SharedSection,
};
use messenger_runtime::net::{NetCheck, NetMode, NetStatus};
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use crate::media_server::MediaServer;
#[cfg(desktop)]
use std::sync::OnceLock;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::{Emitter, Manager};
use veydan_core::{AppError, CmdResult};
use veydan_lock::Lock;
use veydan_shell::Shell;
use zeroize::Zeroizing;

/// Emitted with `Vec<RelayView>` whenever relay state changes.
pub const EVENT_RELAY_STATUS: &str = "messenger://relay-status";
/// Emitted with a runtime `UiEvent` (`{name, payload}`): inbound.dm,
/// inbound.meta, ignored, error, notify.
pub const EVENT_RUNTIME: &str = "messenger://event";

/// The id of the module, as the product lists it: the shell keeps its switch
/// (`modules_enabled`, platform-spec 12).
pub(crate) const MODULE_ID: &str = "messenger";

/// The host setting that showed or hid the module until the shell kept the
/// switches; taken away at setup.
pub(crate) const OLD_ENABLED_KEY: &str = "messenger_enabled";

/// What the commands answer while the messenger is switched off.
const STOPPED: &str = "messenger_stopped";

/// Sub-directory of the app data dir owned entirely by the messenger.
pub(crate) const DATA_SUBDIR: &str = "messenger";

/// A task that serves the runtime; aborted when the messenger stops.
type Task = tauri::async_runtime::JoinHandle<()>;

/// The runtime and the tasks that serve it, while the messenger runs.
struct Live {
    rt: Arc<MessengerRuntime>,
    tasks: Vec<Task>,
}

/// Where the runtime lives. Empty while the messenger is switched off and
/// when its start failed: the commands then answer as they did when the start
/// failed, before the messenger could stop — `AppError::Other` with the
/// reason, `messenger_stopped` when it is off.
#[derive(Default)]
struct Slot {
    live: RwLock<Option<Live>>,
    /// Why the last start failed.
    error: RwLock<Option<String>>,
    /// A start and a stop do not overlap.
    switching: tokio::sync::Mutex<()>,
}

impl Slot {
    /// Start the runtime `run` gives and the tasks `serve` spawns for it,
    /// then catch up with what the lock did while it was off. Nothing
    /// happens when it runs already.
    async fn start<F, S>(&self, run: F, serve: S) -> CmdResult<()>
    where
        F: std::future::Future<Output = messenger_core::Result<MessengerRuntime>>,
        S: FnOnce(&Arc<MessengerRuntime>) -> Vec<Task>,
    {
        let _switching = self.switching.lock().await;
        if self.current().is_some() {
            return Ok(());
        }
        match run.await {
            Ok(rt) => {
                let rt = Arc::new(rt);
                let tasks = serve(&rt);
                *write(&self.live) = Some(Live { rt: rt.clone(), tasks });
                *write(&self.error) = None;
                // The store gives keys from here on: the runtime is in its place.
                caught_up(&rt).await;
                Ok(())
            }
            Err(e) => {
                eprintln!("messenger: start failed: {e}");
                *write(&self.error) = Some(e.to_string());
                Err(AppError::Other(e.to_string()))
            }
        }
    }

    /// Abort the tasks, let `before` do what needs the runtime still running,
    /// and shut the runtime down: its relay connections and its database.
    /// Nothing happens when it does not run.
    async fn stop<B, F>(&self, before: B)
    where
        B: FnOnce(Arc<MessengerRuntime>) -> F,
        F: std::future::Future<Output = ()>,
    {
        let _switching = self.switching.lock().await;
        let Some(live) = write(&self.live).take() else { return };
        for task in &live.tasks {
            task.abort();
        }
        before(live.rt.clone()).await;
        live.rt.shutdown().await;
    }

    fn current(&self) -> Option<Arc<MessengerRuntime>> {
        read(&self.live).as_ref().map(|live| live.rt.clone())
    }

    fn runtime(&self) -> CmdResult<Arc<MessengerRuntime>> {
        self.current()
            .ok_or_else(|| AppError::Other(self.error().unwrap_or_else(|| STOPPED.into())))
    }

    fn error(&self) -> Option<String> {
        read(&self.error).clone()
    }
}

/// A lock a panic poisoned still holds what it held.
fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The state of the messenger module. Managed by the module's setup, empty;
/// `start` and `stop` — the module's hooks, which the shell calls when the
/// app starts and exits and when the user switches the module on and off —
/// fill and empty it. A start that fails leaves it empty: the status command
/// reports the error instead of the app refusing to boot.
pub struct MessengerState {
    app: tauri::AppHandle,
    config: MessengerConfig,
    slot: Slot,
    /// The notifications of a computer: made at the first start, kept for
    /// the process, as the system knows the app by them.
    #[cfg(desktop)]
    desktop: OnceLock<Arc<desktop_notify::DesktopNotify>>,
}

impl MessengerState {
    pub fn new(app: tauri::AppHandle, app_data_dir: &Path) -> Self {
        Self {
            app,
            config: MessengerConfig::new(app_data_dir.join(DATA_SUBDIR)),
            slot: Slot::default(),
            #[cfg(desktop)]
            desktop: OnceLock::new(),
        }
    }

    /// Start the runtime and the tasks that serve it: the watchers of the
    /// relays, the manifest, the network and the lock, the push bridge on
    /// Android, the notifications and the unread count on a computer, the
    /// events for the page — and catch up with a reset of the lock made while
    /// it was off. Nothing happens when it runs already. Runs
    /// inside the async runtime. Never panics: a broken messenger must not
    /// take the host down.
    pub async fn start(&self) -> CmdResult<()> {
        let secrets: Arc<dyn SecretStore> = Arc::new(HostSecretStore { app: self.app.clone() });
        #[cfg(desktop)]
        let desktop = self
            .desktop
            .get_or_init(|| desktop_notify::DesktopNotify::start(self.app.clone()))
            .clone();
        let app = self.app.clone();
        let run = MessengerRuntime::start(self.config.clone(), secrets);
        self.slot
            .start(run, |rt| {
                serve(
                    &app,
                    rt,
                    #[cfg(desktop)]
                    desktop,
                )
            })
            .await
    }

    /// Stop what `start` started and shut the runtime down: no relay
    /// connection, no watcher, no push registration (Android: the push server
    /// forgets the device and the push handler loses its keys), no
    /// notification and no count on a computer. Nothing happens when it does
    /// not run.
    pub async fn stop(&self) {
        let app = self.app.clone();
        self.slot
            .stop(|rt| async move {
                #[cfg(target_os = "android")]
                push::stop_bridge(&app, &rt).await;
                // A call under way ends with the runtime: the shell of the phone (service, audio mode, camera) is let go.
                #[cfg(target_os = "android")]
                call_android::stop_bridge(&app).await;
                #[cfg(not(target_os = "android"))]
                let _ = (app, rt);
            })
            .await;
        #[cfg(desktop)]
        {
            if let Some(desktop) = self.desktop.get().cloned() {
                // Waits for the system for up to a second: off the async threads.
                let _ = tauri::async_runtime::spawn_blocking(move || desktop.withdraw()).await;
            }
            // Leaving, the shell redraws nothing more.
            let shell = self.app.state::<veydan_shell::Shell>();
            if !shell.exiting() {
                shell.set_unread(MODULE_ID, 0);
            }
        }
    }

    /// The runtime, or the error the commands answer while there is none.
    pub(crate) fn runtime(&self) -> CmdResult<Arc<MessengerRuntime>> {
        self.slot.runtime()
    }

    #[cfg(desktop)]
    pub(crate) fn desktop(&self) -> Option<Arc<desktop_notify::DesktopNotify>> {
        self.desktop.get().cloned()
    }

    /// The chat was read, or the page was seen: its notifications go.
    pub(crate) fn notices_seen(&self, key: Option<&str>) {
        unread_changed();
        #[cfg(desktop)]
        if let Some(d) = self.desktop.get() {
            d.clear(key);
        }
        #[cfg(not(desktop))]
        let _ = key;
    }
}

/// The tasks that serve a runtime that just started, in the order the
/// messenger started them before it could stop.
fn serve(
    app: &tauri::AppHandle,
    rt: &Arc<MessengerRuntime>,
    #[cfg(desktop)] desktop: Arc<desktop_notify::DesktopNotify>,
) -> Vec<Task> {
    let mut tasks = vec![
        spawn_relay_status_watcher(app.clone(), rt.clone()),
        spawn_manifest_refresher(rt.clone()),
    ];
    tasks.extend(spawn_net_watcher(rt.clone()));
    tasks.extend(spawn_lock_watcher(app, rt.clone()));
    #[cfg(target_os = "android")]
    tasks.extend(push::spawn_bridge(app.clone(), rt.clone()));
    #[cfg(target_os = "android")]
    tasks.push(call_android::spawn_bridge(app.clone()));
    #[cfg(desktop)]
    tasks.push(desktop_notify::spawn_unread(app.clone(), rt.clone()));
    // An incoming call rings as a toast with its two buttons while the
    // window is not on screen (calls.rs).
    #[cfg(desktop)]
    tasks.push(calls::spawn_desktop_ring(rt.clone(), desktop.clone()));
    tasks.push(spawn_ui_event_forwarder(
        app.clone(),
        rt.clone(),
        #[cfg(desktop)]
        desktop,
    ));
    tasks
}

/// What waits in the chats changed: the count on the icon follows.
fn unread_changed() {
    #[cfg(desktop)]
    desktop_notify::unread_changed();
}

/// Polls relay state and emits `EVENT_RELAY_STATUS` when it changes. The
/// UI relies on this instead of polling commands itself.
fn spawn_relay_status_watcher(app: tauri::AppHandle, rt: Arc<MessengerRuntime>) -> Task {
    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let Ok(list) = rt.relays().list().await else { continue };
            let snapshot = serde_json::to_string(&list).unwrap_or_default();
            if snapshot != last {
                last = snapshot;
                let _ = app.emit(EVENT_RELAY_STATUS, &list);
            }
        }
    })
}

/// Keeps the project's manifest fresh: the runtime asks at most once a day,
/// and only with the Veydan servers chosen; this only wakes it up.
fn spawn_manifest_refresher(rt: Arc<MessengerRuntime>) -> Task {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            if let Err(e) = rt.manifest_refresh(false).await {
                eprintln!("messenger: manifest check: {e}");
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    })
}

/// Keeps an eye on the way to the project's servers: relays that stay down
/// and uploads that fail are signs that the direct way is restricted, and
/// the runtime then finds out whether a bridge would help. What is to be
/// done about it is the runtime's to decide; this only wakes it up. A beat
/// every two seconds lets the runtime notice that the device slept.
fn spawn_net_watcher(rt: Arc<MessengerRuntime>) -> [Task; 3] {
    const BEAT: Duration = Duration::from_secs(2);
    let beating = rt.clone();
    let beat = tauri::async_runtime::spawn(async move {
        loop {
            beating.net_beat(BEAT).await;
            tokio::time::sleep(BEAT).await;
        }
    });
    let watched = rt.clone();
    let watch = tauri::async_runtime::spawn(async move {
        let mut ticks: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if let Err(e) = watched.net_watch().await {
                eprintln!("messenger net: watch: {e}");
            }
            // Once an hour: a list of bridges that grew old, a hold that ran out.
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(360) {
                if let Err(e) = watched.net_tick().await {
                    eprintln!("messenger net: tick: {e}");
                }
            }
        }
    });
    let hear = tauri::async_runtime::spawn(async move {
        let mut rx = rt.ui_events();
        loop {
            match rx.recv().await {
                Ok(event) => rt.net_heard(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    [beat, watch, hear]
}

/// Forwards runtime UI events to the webview as `EVENT_RUNTIME`. On a
/// computer a `notify` goes by the notifications first: when the system
/// shows it, the event says so (`os: true`) and the page shows no card.
fn spawn_ui_event_forwarder(
    app: tauri::AppHandle,
    rt: Arc<MessengerRuntime>,
    #[cfg(desktop)] desktop: Arc<desktop_notify::DesktopNotify>,
) -> Task {
    tauri::async_runtime::spawn(async move {
        let mut rx = rt.ui_events();
        loop {
            match rx.recv().await {
                #[allow(unused_mut)]
                Ok(mut ev) => {
                    #[cfg(desktop)]
                    if ev.name == "notify" && desktop.take(&rt, &ev.payload).await {
                        ev.payload["os"] = serde_json::Value::Bool(true);
                    }
                    // Read on another device of mine: its notifications go here too.
                    if ev.name == messenger_runtime::UI_EVENT_CHAT_READ {
                        unread_changed();
                        #[cfg(desktop)]
                        desktop.clear(ev.payload["chat_id"].as_str());
                    }
                    let _ = app.emit(EVENT_RUNTIME, &ev);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    })
}

/// The box of the lock the messenger keeps its secrets in.
const SECRET_BOX: &str = "messenger";

/// `SecretStore` of the messenger: the lock's box `messenger`. Reaches the
/// lock through the app on each call.
struct HostSecretStore {
    app: tauri::AppHandle,
}

impl HostSecretStore {
    /// Until the runtime is in its place the store answers "locked": early
    /// calls (the signer probe in `MessengerRuntime::start`) get no key, and
    /// `refresh_signer` picks it up on the first status call. A runtime that
    /// is stopping gets none either.
    fn lock(&self) -> messenger_core::Result<tauri::State<'_, Lock>> {
        let running = self
            .app
            .try_state::<MessengerState>()
            .is_some_and(|messenger| messenger.slot.current().is_some());
        if !running {
            return Err(MessengerError::SecretsLocked);
        }
        Ok(self.app.state())
    }

    fn locked(e: veydan_lock::Error) -> MessengerError {
        match e {
            veydan_lock::Error::VaultLocked | veydan_lock::Error::VaultMismatch => MessengerError::SecretsLocked,
            veydan_lock::Error::Db(m) => MessengerError::Storage(m),
            other => MessengerError::Crypto(other.to_string()),
        }
    }
}

#[async_trait]
impl SecretStore for HostSecretStore {
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
        match self.lock() {
            Ok(lock) => lock.secret_box(SECRET_BOX).is_unlocked(),
            Err(_) => false,
        }
    }
}

/// Every event of the lock may change whether the push handler is to hold
/// keys: they follow at once. After a reset the key the messenger kept is
/// gone, and so is the identity: the messenger stands without a key until
/// one is imported. The box may still be closed when the reset is told
/// (demo data wipes the key row and sets a lock after), so each event that
/// may have opened it looks again, and so do events lost on the way.
fn spawn_lock_watcher(app: &tauri::AppHandle, rt: Arc<MessengerRuntime>) -> Option<Task> {
    let lock = app.try_state::<Lock>()?;
    let mut events = lock.subscribe();
    Some(tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    push::lock_changed();
                    if event != veydan_lock::Event::Locked {
                        forget_lost_identity(&rt).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    push::lock_changed();
                    forget_lost_identity(&rt).await;
                }
                Err(_) => break,
            }
        }
    }))
}

/// What the lock did while the messenger was off reached no watcher: a reset
/// then emptied the box all the same. A runtime that has just started looks
/// once, as the watcher looks at each event; a box that is closed now is
/// looked into by the watcher when it opens.
async fn caught_up(rt: &MessengerRuntime) {
    push::lock_changed();
    forget_lost_identity(rt).await;
}

/// The identity goes when its key is no longer in the open box: a reset of
/// the lock emptied it. A closed box or a key that does not decrypt keeps it.
async fn forget_lost_identity(rt: &MessengerRuntime) {
    if !matches!(rt.identity().load_keys().await, Err(MessengerError::SecretMissing(_))) {
        return;
    }
    if let Err(e) = drop_identity(rt).await {
        eprintln!("messenger: the identity whose key is gone stays: {e}");
    }
}

/// Remove the identity: the push registration, the key, the row, the session.
async fn drop_identity(rt: &MessengerRuntime) -> messenger_core::Result<()> {
    // The push server is told to forget the device while there is still a
    // key to sign the request with. It is not a reason to keep the identity:
    // a registration nobody renews runs out by itself.
    if let Err(e) = rt.push_unregister().await {
        eprintln!("messenger push: the registration was not taken back: {e}");
    }
    rt.identity().delete().await?;
    rt.refresh_signer().await?;
    // The push handler's copy of the keys goes with the identity.
    push::lock_changed();
    Ok(())
}

fn map_err(e: MessengerError) -> AppError {
    match e {
        MessengerError::SecretsLocked => AppError::VaultLocked,
        MessengerError::Storage(m) => AppError::Db(m),
        MessengerError::Io(m) => AppError::Io(m),
        MessengerError::NotLoggedIn => AppError::NotFound("messenger identity".into()),
        // Validation messages reach the UI as they are: relationship
        // refusals are stable codes (`dm_waiting_approval`, …) it translates.
        MessengerError::Invalid(m) => AppError::Other(m),
        other => AppError::Other(other.to_string()),
    }
}

// ─── Status / enable ────────────────────────────────────────────────────────

/// What the UI needs to decide whether to show the module.
#[derive(Serialize)]
pub struct MessengerStatus {
    /// Always `true` when this command exists; the UI treats a missing
    /// command as `compiled: false`.
    pub compiled: bool,
    /// The module is switched on: the shell's switch (`modules_enabled`).
    pub enabled: bool,
    pub runtime: Option<RuntimeStatus>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn messenger_status(
    shell: tauri::State<'_, Shell>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessengerStatus> {
    let enabled = shell.module_enabled(MODULE_ID);
    let runtime = match messenger.slot.current() {
        Some(rt) => {
            // The vault may have been unlocked since start-up; pick up the signer.
            if let Err(e) = rt.refresh_signer().await {
                eprintln!("messenger: refresh_signer: {e}");
            }
            Some(rt.status().await.map_err(map_err)?)
        }
        None => None,
    };
    Ok(MessengerStatus { compiled: true, enabled, runtime, error: messenger.slot.error() })
}

/// The module's switch, as the Modules section of the settings turns it: on
/// starts the runtime (and stays off when it does not start), off stops it.
/// Off is refused while the messenger is the only module that is on.
#[tauri::command]
pub async fn messenger_set_enabled(enabled: bool, shell: tauri::State<'_, Shell>) -> CmdResult<()> {
    shell.set_module_enabled(MODULE_ID, enabled).await
}

// ─── Identity ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_identity_get(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Option<Identity>> {
    messenger.runtime()?.identity().get().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_identity_create(
    password: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CreatedIdentity> {
    let rt = messenger.runtime()?;
    let created = rt.identity().create(&password).await.map_err(map_err)?;
    rt.refresh_signer().await.map_err(map_err)?;
    Ok(created)
}

/// `kind` is `nsec` (also accepts hex), `ncryptsec` (needs `password`) or
/// `mnemonic` (`password` is the optional BIP-39 passphrase).
#[tauri::command]
pub async fn messenger_identity_import(
    kind: String,
    secret: String,
    password: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Identity> {
    let rt = messenger.runtime()?;
    let svc = rt.identity();
    let pw = password.unwrap_or_default();
    let res = match kind.as_str() {
        "nsec" => svc.import_nsec(&secret).await,
        "ncryptsec" => svc.import_ncryptsec(&secret, &pw).await,
        "mnemonic" => svc.import_mnemonic(&secret, &pw).await,
        other => Err(MessengerError::Invalid(format!("unknown import kind: {other}"))),
    };
    let identity = res.map_err(map_err)?;
    rt.refresh_signer().await.map_err(map_err)?;
    Ok(identity)
}

#[tauri::command]
pub async fn messenger_identity_export(
    password: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<String> {
    messenger.runtime()?.identity().export_ncryptsec(&password).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_identity_delete(messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    let rt = messenger.runtime()?;
    drop_identity(&rt).await.map_err(map_err)
}

// ─── Relays / manifest ──────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_relays_list(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<RelayView>> {
    messenger.runtime()?.relays().list().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_relays_add(
    url: String,
    api_key: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<RelayView> {
    messenger.runtime()?.relays().add_user(&url, api_key).await.map_err(map_err)
}

// ─── Profiles / contacts ────────────────────────────────────────────────────

fn parse_pubkey(hex: &str) -> CmdResult<PubKey> {
    PubKey::parse(hex).ok_or_else(|| AppError::Other("expected a 64-hex public key".into()))
}

#[derive(Deserialize)]
pub struct ContactPatchInput {
    /// `null` clears; missing leaves untouched.
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub nickname: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub note: Option<Option<String>>,
}

fn deserialize_double_option<'de, D>(d: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(d)?))
}

#[tauri::command]
pub async fn messenger_profile_get(
    pubkey: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<ProfileView>> {
    messenger.runtime()?.profiles().get(&parse_pubkey(&pubkey)?).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_profile_request(pubkey: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.request_profile(&parse_pubkey(&pubkey)?).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_profile_own_get(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Option<ProfileView>> {
    messenger.runtime()?.my_profile().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_profile_own_set(
    input: messenger_contacts_input::ProfileInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<ProfileView> {
    messenger.runtime()?.publish_own_profile(&input).await.map_err(map_err)
}

/// Re-export so the command signature can name the type without the adapter
/// depending on the contacts crate directly.
pub mod messenger_contacts_input {
    pub use messenger_runtime::ProfileInput;
}

/// The links to profiles elsewhere a user can add, in the order of the picker.
#[tauri::command]
pub async fn messenger_social_platforms(
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<messenger_runtime::SocialPlatform>> {
    Ok(messenger.runtime()?.social_platforms())
}

/// A bio as it will show: the live preview of the editor.
#[tauri::command]
pub async fn messenger_bio_parse(
    markup: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<messenger_runtime::Span>> {
    Ok(messenger.runtime()?.bio_parse(&markup))
}

/// A picked picture, decoded and held for the crop. Like a picked file, it
/// is read only when the user picked it: an Android `content://` source,
/// or a path the computer's picker has let in.
#[tauri::command]
pub async fn messenger_avatar_prepare(
    source: String,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<messenger_runtime::AvatarPreview> {
    use tauri_plugin_fs::FsExt;
    let rt = messenger.runtime()?;
    if source.starts_with("content://") {
        let picked = app.state::<veydan_shell::Shell>().open_source(&source)?;
        let bytes = read_picked(picked, messenger_runtime::avatars::MAX_PICK_BYTES).await?;
        return rt.avatar_prepare_bytes(bytes).await.map_err(map_err);
    }
    let path = std::path::PathBuf::from(&source);
    if !app.try_fs_scope().is_some_and(|s| s.is_allowed(&path)) {
        return Err(AppError::Other("the file was not picked".into()));
    }
    rt.avatar_prepare(&path).await.map_err(map_err)
}

/// The bytes of a picked source, at most `max` of them; a larger file is
/// refused as an avatar too large.
async fn read_picked(source: veydan_shell::FileSource, max: usize) -> CmdResult<Vec<u8>> {
    if source.len.is_some_and(|n| n > max as u64) {
        return Err(AppError::Other("avatar_too_large".into()));
    }
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<Vec<u8>> {
        use std::io::Read;
        let file = (source.open)().map_err(AppError::io)?;
        let mut out = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut out).map_err(AppError::io)?;
        if out.len() > max {
            return Err(AppError::Other("avatar_too_large".into()));
        }
        Ok(out)
    })
    .await
    .map_err(AppError::io)?
}

/// Make the part `rect` of the prepared picture my avatar; my profile as
/// it is now.
#[tauri::command]
pub async fn messenger_avatar_set(
    token: String,
    rect: messenger_runtime::CropRect,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<ProfileView> {
    messenger.runtime()?.avatar_set(&token, rect).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_avatar_remove(messenger: tauri::State<'_, MessengerState>) -> CmdResult<ProfileView> {
    messenger.runtime()?.avatar_remove().await.map_err(map_err)
}

/// The avatar at `url` as a `data:` url, or `null` while it is fetched;
/// `avatar.ready {url}` follows as a runtime event.
#[tauri::command]
pub async fn messenger_avatar_cached(url: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<Option<String>> {
    messenger.runtime()?.avatar_cached(&url).await.map_err(map_err)
}

/// My phone, which never goes into kind 0, and whether my card carries it.
#[tauri::command]
pub async fn messenger_own_private_get(
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<messenger_runtime::OwnPrivateView> {
    messenger.runtime()?.own_private_get().await.map_err(map_err)
}

/// `phone` in any way of writing a number; `null` or empty removes it.
/// Error: `phone_invalid`.
#[tauri::command]
pub async fn messenger_own_private_set(
    phone: Option<String>,
    share_phone: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<messenger_runtime::OwnPrivateView> {
    messenger.runtime()?.own_private_set(phone.as_deref(), share_phone).await.map_err(map_err)
}

/// The phone a contact sent me in its own card.
#[tauri::command]
pub async fn messenger_contact_private_get(
    pubkey: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<messenger_runtime::ContactPrivateView> {
    messenger.runtime()?.contact_private_get(&pubkey).await.map_err(map_err)
}

/// Send a contact card to `chat` (a person or `group:<id>`): mine when
/// `pubkey` is `null`, with my phone when `include_phone`; another
/// person's never carries a phone.
#[tauri::command]
pub async fn messenger_card_send(
    chat: String,
    pubkey: Option<String>,
    include_phone: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    messenger.runtime()?.card_send(&chat, pubkey.as_deref(), include_phone).await.map_err(map_err)
}

/// "Add contact" on a received card. Errors: `card_unknown`, `card_is_me`.
#[tauri::command]
pub async fn messenger_card_accept(
    message_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    messenger.runtime()?.card_accept(&message_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_nip05_verify(pubkey: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<bool> {
    messenger.runtime()?.verify_nip05(&parse_pubkey(&pubkey)?).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_contacts_list(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<ContactView>> {
    messenger.runtime()?.contacts().list().await.map_err(map_err)
}

/// `key` is an npub, a hex key or a NIP-05 identifier.
#[tauri::command]
pub async fn messenger_contacts_add(
    key: String,
    nickname: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<ContactView> {
    messenger.runtime()?.contact_add(&key, nickname.as_deref()).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_contacts_update(
    pubkey: String,
    patch: ContactPatchInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<ContactView> {
    let p = ContactPatch { nickname: patch.nickname, note: patch.note };
    messenger.runtime()?.contact_update(&parse_pubkey(&pubkey)?, &p).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_contacts_remove(pubkey: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.contact_remove(&parse_pubkey(&pubkey)?).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_contacts_set_followed(
    pubkey: String,
    followed: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.contact_set_followed(&parse_pubkey(&pubkey)?, followed).await.map_err(map_err)
}

// ─── Chats / DM ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_chats_list(
    include_archived: Option<bool>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<ChatView>> {
    messenger.runtime()?.chats(include_archived.unwrap_or(false)).await.map_err(map_err)
}

/// Open (creating if needed) the chat with `peer` (npub or hex).
#[tauri::command]
pub async fn messenger_chat_open(peer: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<ChatView> {
    messenger.runtime()?.chat_open(&peer).await.map_err(map_err)
}

/// One page of visible messages, oldest first, strictly older than `before`.
#[tauri::command]
pub async fn messenger_chat_messages(
    chat_id: String,
    before: Option<i64>,
    limit: Option<i64>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<MessageView>> {
    messenger.runtime()?.dm().messages(&chat_id, before, limit.unwrap_or(50)).await.map_err(map_err)
}

/// How many messages each section of what a chat has shared holds.
#[tauri::command]
pub async fn messenger_chat_shared_counts(
    chat_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<SharedCounts> {
    messenger.runtime()?.shared_counts(&chat_id).await.map_err(map_err)
}

/// One page of a section of what a chat has shared, newest first, strictly older than `before`.
#[tauri::command]
pub async fn messenger_chat_shared(
    chat_id: String,
    section: SharedSection,
    before: Option<i64>,
    limit: Option<i64>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<MessageView>> {
    messenger.runtime()?.shared(&chat_id, section, before, limit.unwrap_or(60)).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_privacy_get(messenger: tauri::State<'_, MessengerState>) -> CmdResult<PrivacySettings> {
    messenger.runtime()?.privacy_settings().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_privacy_set(
    read_receipts: bool,
    presence: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PrivacySettings> {
    messenger.runtime()?.privacy_set(PrivacySettings { read_receipts, presence }).await.map_err(map_err)
}

/// When each approved contact was last seen; empty with presence off.
#[tauri::command]
pub async fn messenger_presence_list(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<PresenceView>> {
    messenger.runtime()?.presence_list().await.map_err(map_err)
}

/// The page is in sight (said again every 45 s while it is) or hidden.
#[tauri::command]
pub async fn messenger_presence_foreground(visible: bool, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.presence_foreground(visible).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_chat_mark_read(chat_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.chat_mark_read(&chat_id).await.map_err(map_err)?;
    messenger.notices_seen(Some(&chat_id));
    Ok(())
}

#[tauri::command]
pub async fn messenger_chat_set_pinned(
    chat_id: String,
    pinned: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.dm().set_pinned(&chat_id, pinned).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_chat_set_archived(
    chat_id: String,
    archived: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.dm().set_archived(&chat_id, archived).await.map_err(map_err)?;
    unread_changed();
    Ok(())
}

/// Direct chats and groups alike: a muted chat is still counted and shown, only quietly.
#[tauri::command]
pub async fn messenger_chat_set_muted(
    chat_id: String,
    muted: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.dm().set_muted(&chat_id, muted).await.map_err(map_err)?;
    unread_changed();
    Ok(())
}

/// Removes the chat and its messages from this device only.
#[tauri::command]
pub async fn messenger_chat_delete(chat_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.chat_delete(&chat_id).await.map_err(map_err)?;
    unread_changed();
    Ok(())
}

/// Send a text DM to an npub/hex key; returns the stored message.
#[tauri::command]
pub async fn messenger_dm_send_text(
    to: String,
    text: String,
    reply_to: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    messenger.runtime()?.dm_send_text(&to, &text, reply_to.as_deref()).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_dm_edit(
    message_id: String,
    text: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    messenger.runtime()?.dm_edit(&message_id, &text).await.map_err(map_err)
}

/// Put an emoji on a message, or take mine back. Refusals reach the UI as
/// `reaction_limit`, `reaction_invalid` and the codes of the chat.
#[tauri::command]
pub async fn messenger_dm_react(
    message_id: String,
    emoji: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    messenger.runtime()?.dm_react(&message_id, &emoji).await.map_err(map_err)
}

/// An emoji was picked in the composer: it counts among the ones I use most.
#[tauri::command]
pub async fn messenger_emoji_used(emoji: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.emoji_used(&emoji).await.map_err(map_err)
}

/// The emoji I use most, on any of my devices.
#[tauri::command]
pub async fn messenger_emoji_top(n: u32, messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<String>> {
    messenger.runtime()?.emoji_top(n as usize).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_dm_delete(
    message_id: String,
    for_everyone: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.dm_delete(&message_id, for_everyone).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_dm_retry(message_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.dm_retry(&message_id).await.map_err(map_err)
}

// ─── DM relationship ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_dm_relation(
    peer: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<RelationView> {
    messenger.runtime()?.dm_relation(&peer).await.map_err(map_err)
}

/// `action`: request | accept | decline | block | unblock | remove.
#[tauri::command]
pub async fn messenger_dm_action(
    peer: String,
    action: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<RelationView> {
    let action = match action.as_str() {
        "request" => DmAction::Request,
        "accept" => DmAction::Accept,
        "decline" => DmAction::Decline,
        "block" => DmAction::Block,
        "unblock" => DmAction::Unblock,
        "remove" => DmAction::Remove,
        other => return Err(AppError::Other(format!("unknown action: {other}"))),
    };
    messenger.runtime()?.dm_act(&peer, action).await.map_err(map_err)
}

// ─── Groups ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_groups_list(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<GroupView>> {
    messenger.runtime()?.group_list().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_group_get(
    group_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    messenger.runtime()?.group_get(&group_id).await.map_err(map_err)
}

/// `kind`: public | private.
#[tauri::command]
pub async fn messenger_group_create(
    kind: String,
    name: String,
    about: Option<String>,
    history_for_new: Option<bool>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    let kind = match kind.as_str() {
        "public" => GroupKind::Public,
        "private" => GroupKind::Private,
        other => return Err(AppError::Other(format!("unknown group kind: {other}"))),
    };
    messenger
        .runtime()?
        .group_create(kind, &name, about.as_deref().unwrap_or(""), history_for_new.unwrap_or(true))
        .await
        .map_err(map_err)
}

#[tauri::command]
pub async fn messenger_group_invite(
    group_id: String,
    who: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<InviteView> {
    messenger.runtime()?.group_invite(&group_id, &who).await.map_err(map_err)
}

/// `direction`: in (for me) | out (sent by me).
#[tauri::command]
pub async fn messenger_group_invites(
    direction: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<InviteView>> {
    messenger.runtime()?.group_invites(&direction).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_group_answer_invite(
    invite_id: String,
    accept: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.group_answer_invite(&invite_id, accept).await.map_err(map_err)
}

/// Open a `veydan://group/…` link: join a public group, ask a private one.
#[tauri::command]
pub async fn messenger_group_open_link(
    link: String,
    note: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    messenger.runtime()?.group_open_link(&link, note.as_deref().unwrap_or("")).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_group_answer_request(
    group_id: String,
    requester: String,
    approve: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    messenger.runtime()?.group_answer_request(&group_id, &requester, approve).await.map_err(map_err)
}

/// `action` is an operation as the log writes it: `{"op":"remove","who":…}`,
/// `ban`, `unban`, `set_role` (+ `role`), `set_muted` (+ `muted`),
/// `edit_settings`, `transfer_ownership` (+ `to`), `leave`, `disband`.
#[tauri::command]
pub async fn messenger_group_act(
    group_id: String,
    action: serde_json::Value,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    let body: GroupOp = serde_json::from_value(action).map_err(|e| AppError::Other(format!("group action: {e}")))?;
    messenger.runtime()?.group_act(&group_id, body).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_group_rotate_link(
    group_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<GroupView> {
    messenger.runtime()?.group_rotate_link(&group_id).await.map_err(map_err)
}

/// The link of the group as a QR code (SVG).
#[tauri::command]
pub async fn messenger_group_link_qr(
    group_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<String> {
    messenger.runtime()?.group_link_qr(&group_id).await.map_err(map_err)
}

/// Remove from this device a group I am no longer in.
#[tauri::command]
pub async fn messenger_group_forget(group_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.group_forget(&group_id).await.map_err(map_err)
}

/// Hex keys of everyone I block.
#[tauri::command]
pub async fn messenger_dm_blocked(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<String>> {
    messenger.runtime()?.dm_blocked().await.map_err(map_err)
}

// ─── Media ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn messenger_media_servers(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<MediaServerView>> {
    messenger.runtime()?.media_servers().await.map_err(map_err)
}

/// Add or update a blob server. The S3 secret goes to the vault and never
/// comes back to the UI.
#[tauri::command]
pub async fn messenger_media_server_put(
    input: MediaServerInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MediaServerView> {
    messenger.runtime()?.media_server_put(input).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_server_remove(id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.media_server_remove(&id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_server_set_enabled(
    id: String,
    enabled: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.media_server_set_enabled(&id, enabled).await.map_err(map_err)
}

/// Checks credentials, prepares the bucket, writes and reads a probe.
#[tauri::command]
pub async fn messenger_media_server_check(id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.media_server_check(&id).await.map_err(map_err)
}

/// Attach a local file; returns the placeholder message immediately. A
/// photo is made smaller first unless `original` (left out: made smaller).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn messenger_dm_send_file(
    to: String,
    path: String,
    caption: Option<String>,
    // The same for files picked together: they are shown as one album.
    batch: Option<String>,
    original: Option<bool>,
    // A frame the UI took from a video, shown before the file is fetched.
    poster: Option<messenger_runtime::Poster>,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    let rt = messenger.runtime()?;
    let local = import_picked(&app, &path, rt.config().data_dir()).await?;
    rt.dm_send_file_with(&to, &local, caption.as_deref(), batch.as_deref(), original.unwrap_or(false), poster)
        .await
        .map_err(map_err)
}

/// A picked file as the composer holds it until it is sent: a local path
/// and, for a picture, what it looks like. Android's `content://` is copied
/// in first. Only what the user picked is read: that copy, or a path the
/// computer's picker has let in. A file over 1 GiB is refused
/// (`err.file_too_large`), a `content://` one before it is copied and
/// with the name its source gives (`err.file_too_large: IMG_1.mp4`).
#[tauri::command]
pub async fn messenger_media_import(
    path: String,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Picked> {
    use tauri_plugin_fs::FsExt;
    let rt = messenger.runtime()?;
    let copied = path.starts_with("content://");
    let local = import_picked(&app, &path, rt.config().data_dir()).await?;
    if !copied && !app.try_fs_scope().is_some_and(|s| s.is_allowed(&local)) {
        return Err(AppError::Other("the file was not picked".into()));
    }
    let picked = rt.picked(&local).await.map_err(map_err)?;
    // The webview reads a picked video by itself, for a frame of it.
    let url = match picked.kind.as_str() {
        "video" if messenger_runtime::passive(&picked.mime) => app.state::<MediaServer>().url(&local, &picked.mime).await,
        _ => None,
    };
    Ok(Picked { picked, url })
}

/// A picked file as a local path. Desktop pickers give paths. Android gives
/// `content://` URIs that only the system can read: those are copied into
/// the messenger's own folder first (the copy is also what the chat shows
/// for a sent file).
async fn import_picked(app: &tauri::AppHandle, picked: &str, data_dir: &Path) -> CmdResult<std::path::PathBuf> {
    if !picked.starts_with("content://") {
        let _ = (app, data_dir);
        return Ok(std::path::PathBuf::from(picked));
    }
    let source = app.state::<veydan_shell::Shell>().open_source(picked)?;
    copy_picked(source, data_dir, messenger_runtime::media::MAX_SEND_BYTES).await
}

/// A file too large to send, by the name its source gives: a `content://`
/// path says nothing a person reads.
fn too_large(name: &str) -> AppError {
    AppError::Other(format!("err.file_too_large: {name}"))
}

/// The picked file copied into `outgoing/` of the messenger's folder, under
/// the last segment of the name its source gives. One larger than `limit`
/// is refused, with that name: before the copy when its source tells its
/// size, else as soon as the copy grows past it (the copy goes).
async fn copy_picked(source: veydan_shell::FileSource, data_dir: &Path, limit: u64) -> CmdResult<std::path::PathBuf> {
    // The display name comes from another app: keep only its last segment.
    let name: String = source
        .name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = if name.trim().is_empty() || name.starts_with('.') { "file".to_string() } else { name };
    if source.len.is_some_and(|len| len > limit) {
        return Err(too_large(&name));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = data_dir.join("outgoing").join(format!("{stamp:x}"));
    let dest = dir.join(&name);
    let out = dest.clone();
    tokio::task::spawn_blocking(move || -> CmdResult<()> {
        use std::io::Read;
        std::fs::create_dir_all(&dir).map_err(AppError::io)?;
        let input = (source.open)().map_err(AppError::io)?;
        let mut file = std::fs::File::create(&out).map_err(AppError::io)?;
        // One byte past the limit is enough to know.
        let copied = std::io::copy(&mut input.take(limit.saturating_add(1)), &mut file).map_err(AppError::io);
        if !matches!(copied, Ok(n) if n <= limit) {
            drop(file);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(copied.err().unwrap_or_else(|| too_large(&name)));
        }
        Ok(())
    })
    .await
    .map_err(AppError::io)??;
    Ok(dest)
}

/// Path of the attachment once it is on this device; `null` when an
/// automatic download decided not to start.
#[tauri::command]
pub async fn messenger_media_download(
    message_id: String,
    manual: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<String>> {
    let p = messenger.runtime()?.media_download(&message_id, manual).await.map_err(map_err)?;
    Ok(p.map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn messenger_media_transfer(
    message_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<TransferView>> {
    messenger.runtime()?.media_transfer(&message_id).await.map_err(map_err)
}

/// Every transfer that is not over (queued, running, waiting for its next
/// attempt, paused, failed), the newest first: the list of transfers and
/// the chip of a chat start from it, events keep it current.
#[tauri::command]
pub async fn messenger_media_transfers(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<TransferView>> {
    messenger.runtime()?.media_transfers().await.map_err(map_err)
}

/// "Retry all": what failed or what the closing of the app paused goes on,
/// never a pause the user made. How many transfers started again.
#[tauri::command]
pub async fn messenger_media_retry_failed(messenger: tauri::State<'_, MessengerState>) -> CmdResult<u32> {
    messenger.runtime()?.media_retry_failed().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_pause(transfer_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.media_pause(&transfer_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_resume(transfer_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.media_resume(&transfer_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_cancel(transfer_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.media_cancel(&transfer_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_media_save_as(
    message_id: String,
    dest: String,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    let rt = messenger.runtime()?;
    if dest.starts_with("content://") {
        // Android: the destination is a document the system opens for us.
        let src = rt
            .media_local_path(&message_id)
            .await
            .map_err(map_err)?
            .ok_or_else(|| AppError::Other("err.not_downloaded".into()))?;
        return tokio::task::spawn_blocking(move || -> CmdResult<()> {
            use tauri_plugin_fs::{FilePath, FsExt, OpenOptions};
            let url = tauri::Url::parse(&dest).map_err(AppError::other)?;
            let mut opts = OpenOptions::new();
            opts.write(true).truncate(true);
            let mut out = app.fs().open(FilePath::Url(url), opts).map_err(AppError::io)?;
            std::io::copy(&mut std::fs::File::open(&src).map_err(AppError::io)?, &mut out).map_err(AppError::io)?;
            Ok(())
        })
        .await
        .map_err(AppError::io)?;
    }
    rt.media_save_as(&message_id, Path::new(&dest)).await.map_err(map_err)
}

/// Inline preview (`data:` url) for images, audio and video that are on
/// this device and small enough; `null` otherwise.
#[tauri::command]
pub async fn messenger_media_data_url(
    message_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<String>> {
    messenger.runtime()?.media_data_url(&message_id).await.map_err(map_err)
}

/// Local path of the attachment if present (to open or reveal it).
#[tauri::command]
pub async fn messenger_media_local_path(
    message_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<String>> {
    let p = messenger.runtime()?.media_local_path(&message_id).await.map_err(map_err)?;
    Ok(p.map(|p| p.to_string_lossy().into_owned()))
}

/// A picked file as `messenger_media_import` gives it: a video also with
/// the address the webview reads it at (`MediaServer`), for a frame of it.
#[derive(serde::Serialize)]
pub struct Picked {
    #[serde(flatten)]
    picked: messenger_runtime::PickedView,
    url: Option<String>,
}

/// The address the webview reads the attachment of a message at, when it
/// is on this device and of a type a webview shows passively: a video
/// plays and seeks from the file however large it is, a picture shows
/// without a copy in memory. Only that file is given a name
/// (`MediaServer`).
#[tauri::command]
pub async fn messenger_media_url(
    message_id: String,
    server: tauri::State<'_, MediaServer>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<String>> {
    let Some((path, mime)) = messenger.runtime()?.media_playable(&message_id).await.map_err(map_err)? else { return Ok(None) };
    Ok(server.url(&path, &mime).await)
}

/// Open a link from a message in the system browser. http(s) only; the
/// opener never goes through a shell.
#[tauri::command]
pub async fn messenger_open_url(url: String, app: tauri::AppHandle) -> CmdResult<()> {
    let url = url.trim();
    let ok = (url.starts_with("https://") || url.starts_with("http://"))
        && url.len() <= 2048
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !ok {
        return Err(AppError::Other("only http(s) links can be opened".into()));
    }
    open_external(&app, url, false)
}

// ─── Links ──────────────────────────────────────────────────────────────────

/// What each link leads to, in the order asked. Takes `veydan://…`,
/// `npub1…` and `nostr:npub1…`; anything else comes back as `invalid`.
#[tauri::command]
pub async fn messenger_links_inspect(
    links: Vec<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Vec<LinkView>> {
    messenger.runtime()?.links_inspect(&links).await.map_err(map_err)
}

/// The `veydan://contact/…` link of a person, to share.
#[tauri::command]
pub async fn messenger_contact_link(pubkey: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<String> {
    messenger.runtime()?.contact_link(&parse_pubkey(&pubkey)?).await.map_err(map_err)
}

/// Title, description and picture of an https page. Asks the page: call
/// it when the user pressed the button, never when a message is shown.
#[tauri::command]
pub async fn messenger_link_preview(
    url: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<LinkPreview> {
    messenger.runtime()?.link_preview(&url).await.map_err(map_err)
}

/// Open the attachment of a message with the system's default application.
#[tauri::command]
pub async fn messenger_media_open(
    message_id: String,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    let path = messenger
        .runtime()?
        .media_local_path(&message_id)
        .await
        .map_err(map_err)?
        .ok_or_else(|| AppError::Other("err.not_downloaded".into()))?;
    // A received file is untrusted: anything the system would run is only
    // shown in its folder, never launched.
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if RUNNABLE_EXTENSIONS.contains(&ext.as_str()) || ext.is_empty() {
        return reveal(&app, &path);
    }
    open_external(&app, &path.to_string_lossy(), true)
}

/// File types that execute code when opened with the default handler.
const RUNNABLE_EXTENSIONS: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "pif", "cpl", "ps1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta",
    "lnk", "reg", "sh", "bash", "zsh", "fish", "desktop", "appimage", "run", "bin", "jar", "app", "command",
    "dmg", "pkg", "deb", "rpm", "apk", "py", "pl", "rb", "php", "html", "htm", "svg", "url", "scpt",
];

#[cfg(desktop)]
fn reveal(app: &tauri::AppHandle, path: &Path) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(path).map_err(|e| AppError::Other(e.to_string()))
}

#[cfg(not(desktop))]
fn reveal(_app: &tauri::AppHandle, _path: &Path) -> CmdResult<()> {
    Err(AppError::Other("not available on this platform".into()))
}

#[cfg(desktop)]
fn open_external(app: &tauri::AppHandle, target: &str, is_path: bool) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let res = if is_path {
        app.opener().open_path(target, None::<&str>)
    } else {
        app.opener().open_url(target, None::<&str>)
    };
    res.map_err(|e| AppError::Other(e.to_string()))
}

#[cfg(not(desktop))]
fn open_external(_app: &tauri::AppHandle, _target: &str, _is_path: bool) -> CmdResult<()> {
    Err(AppError::Other("not available on this platform".into()))
}

/// What the recorder in the UI produced. Bytes travel as base64: mobile
/// IPC has no raw bodies, and recordings are small.
#[derive(Deserialize)]
pub struct RecordingInput {
    /// `voice` | `circle`
    pub kind: String,
    pub mime: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub waveform: Option<Vec<u8>>,
    pub data_base64: String,
}

/// Send a voice message or a video circle recorded in the app.
#[tauri::command]
pub async fn messenger_dm_send_recording(
    to: String,
    recording: RecordingInput,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<MessageView> {
    let kind = MediaKind::parse(&recording.kind).ok_or_else(|| AppError::Other("unknown recording kind".into()))?;
    let bytes = B64
        .decode(recording.data_base64.as_bytes())
        .map_err(|_| AppError::Other("recording is not base64".into()))?;
    let rec = Recording {
        kind,
        mime: recording.mime,
        duration_ms: recording.duration_ms,
        waveform: recording.waveform,
        bytes,
    };
    messenger.runtime()?.dm_send_recording(&to, rec, None).await.map_err(map_err)
}

/// Let the webview answer microphone and camera requests (WebKitGTK denies
/// them unless the app does; other webviews ask the user themselves).
#[tauri::command]
pub fn messenger_media_grant_access(
    window: tauri::WebviewWindow,
    shell: tauri::State<'_, veydan_shell::Shell>,
) -> CmdResult<()> {
    shell.grant_media_access(&window)
}

#[tauri::command]
pub async fn messenger_relays_remove(url: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.relays().remove_user(&url).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_relays_set_enabled(
    url: String,
    enabled: bool,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.relays().set_enabled(&url, enabled).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_relays_set_silent(enabled: bool, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.relays().set_silent(enabled).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_manifest_info(messenger: tauri::State<'_, MessengerState>) -> CmdResult<ManifestInfo> {
    messenger.runtime()?.relays().manifest_info().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_manifest_set_region(
    region: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    messenger.runtime()?.relays().set_region(&region).await.map_err(map_err)
}

/// Use the project's servers: its signed manifest, or the built-in one.
#[tauri::command]
pub async fn messenger_servers_use_veydan(messenger: tauri::State<'_, MessengerState>) -> CmdResult<ManifestCheck> {
    messenger.runtime()?.servers_use_veydan().await.map_err(map_err)
}

/// Use only the user's own servers; nothing is asked of the project.
#[tauri::command]
pub async fn messenger_servers_use_own(messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.servers_use_own().await.map_err(map_err)
}

/// Ask the project for a newer manifest now. `None` with the own servers.
#[tauri::command]
pub async fn messenger_manifest_refresh(
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<Option<ManifestCheck>> {
    messenger.runtime()?.manifest_refresh(true).await.map_err(map_err)
}

// ─── The way to the servers: directly, or through a bridge ──────────────────

#[tauri::command]
pub async fn messenger_net_status(messenger: tauri::State<'_, MessengerState>) -> CmdResult<NetStatus> {
    messenger.runtime()?.net_status().await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_net_set_mode(
    mode: NetMode,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NetStatus> {
    messenger.runtime()?.net_set_mode(mode).await.map_err(map_err)
}

/// Try the direct way and a bridge now.
#[tauri::command]
pub async fn messenger_net_check(messenger: tauri::State<'_, MessengerState>) -> CmdResult<NetCheck> {
    messenger.runtime()?.net_check().await.map_err(map_err)
}

/// Add a bridge by its link (`veydan://vlink/…`) or its reference.
#[tauri::command]
pub async fn messenger_net_bridge_add(
    bridge: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NetStatus> {
    messenger.runtime()?.net_bridge_add(&bridge).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_net_bridge_remove(
    id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NetStatus> {
    messenger.runtime()?.net_bridge_remove(&id).await.map_err(map_err)
}

/// "Not now" to the offer of a bridge.
#[tauri::command]
pub async fn messenger_net_offer_dismiss(messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.net_offer_dismiss().await.map_err(map_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The box `messenger` of a lock as the messenger's store, without the app.
    struct LockBox(Arc<Lock>);

    #[async_trait]
    impl SecretStore for LockBox {
        async fn get(&self, key: &str) -> messenger_core::Result<Option<Zeroizing<Vec<u8>>>> {
            self.0.secret_box(SECRET_BOX).get(key).await.map_err(HostSecretStore::locked)
        }

        async fn put(&self, key: &str, value: &[u8]) -> messenger_core::Result<()> {
            self.0.secret_box(SECRET_BOX).put(key, value).await.map_err(HostSecretStore::locked)
        }

        async fn delete(&self, key: &str) -> messenger_core::Result<()> {
            self.0.secret_box(SECRET_BOX).delete(key).await.map_err(HostSecretStore::locked)
        }

        async fn is_unlocked(&self) -> bool {
            self.0.secret_box(SECRET_BOX).is_unlocked()
        }
    }

    #[tokio::test]
    async fn a_reset_of_the_lock_leaves_the_messenger_without_a_key_until_one_is_imported() {
        let dir = tempfile::tempdir().unwrap();
        // The data file as the shell opens it: the tables of the core and of the lock.
        let lock_schema = veydan_core::Schema { module: "lock", steps: veydan_lock::SCHEMA_STEPS };
        let db = veydan_core::db::open(&dir.path().join(veydan_core::db::DB_FILE), &[veydan_core::SCHEMA, lock_schema])
            .await
            .unwrap();
        let lock = Arc::new(Lock::new(db.clone()));
        lock.open_default().await.unwrap();
        lock.set(Some("1357".into()), None, Some("pin".into()), None).await.unwrap();
        let config = MessengerConfig::new(dir.path().join(DATA_SUBDIR));
        let rt = MessengerRuntime::start(config, Arc::new(LockBox(lock.clone()))).await.unwrap();
        let created = rt.identity().create("backup words").await.unwrap();

        // A closed lock or a key in its place keeps the identity.
        lock.lock();
        forget_lost_identity(&rt).await;
        assert!(rt.identity().get().await.unwrap().is_some());
        assert!(lock.unlock("1357").await.unwrap());
        forget_lost_identity(&rt).await;
        assert!(rt.identity().get().await.unwrap().is_some());

        lock.reset("1357", |_| Box::pin(async { Ok(()) })).await.unwrap();
        forget_lost_identity(&rt).await;
        assert!(rt.identity().get().await.unwrap().is_none());
        assert!(!rt.status().await.unwrap().identity_present);

        let back = rt.identity().import_ncryptsec(&created.ncryptsec, "backup words").await.unwrap();
        assert_eq!(back.npub, created.identity.npub);
        assert!(rt.identity().load_keys().await.is_ok());

        // Demo data: the wipe leaves no key to look into yet, the lock set
        // after it opens the box without the messenger's key.
        lock.wipe().await.unwrap();
        forget_lost_identity(&rt).await;
        assert!(rt.identity().get().await.unwrap().is_some());
        lock.set(Some("demo".into()), None, Some("password".into()), None).await.unwrap();
        forget_lost_identity(&rt).await;
        assert!(rt.identity().get().await.unwrap().is_none());
        rt.shutdown().await;
        db.close().await;
    }

    /// A reset of the lock while the messenger is switched off reaches no
    /// watcher: the start that follows finds the key gone and drops the
    /// identity, as the watcher would have (platform-spec 12, 19 № 41). A
    /// closed box at the start keeps it.
    #[tokio::test]
    async fn a_reset_of_the_lock_while_the_messenger_is_off_reaches_it_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let lock_schema = veydan_core::Schema { module: "lock", steps: veydan_lock::SCHEMA_STEPS };
        let db = veydan_core::db::open(&dir.path().join(veydan_core::db::DB_FILE), &[veydan_core::SCHEMA, lock_schema])
            .await
            .unwrap();
        let lock = Arc::new(Lock::new(db.clone()));
        lock.open_default().await.unwrap();
        lock.set(Some("1357".into()), None, Some("pin".into()), None).await.unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(LockBox(lock.clone()));
        let config = MessengerConfig::new(dir.path().join(DATA_SUBDIR));
        let slot = Slot::default();
        let start = || slot.start(MessengerRuntime::start(config.clone(), secrets.clone()), |_| Vec::new());

        start().await.unwrap();
        slot.runtime().unwrap().identity().create("backup words").await.unwrap();
        slot.stop(|_| async {}).await;

        // A closed box says nothing of the key.
        lock.lock();
        start().await.unwrap();
        let rt = slot.runtime().unwrap();
        assert!(rt.identity().get().await.unwrap().is_some());
        drop(rt);
        slot.stop(|_| async {}).await;

        assert!(lock.unlock("1357").await.unwrap());
        lock.reset("1357", |_| Box::pin(async { Ok(()) })).await.unwrap();
        start().await.unwrap();
        let rt = slot.runtime().unwrap();
        assert!(rt.identity().get().await.unwrap().is_none());
        assert!(!rt.status().await.unwrap().identity_present);
        drop(rt);
        slot.stop(|_| async {}).await;
        db.close().await;
    }

    /// A relay that takes connections, completes the handshake, reads what
    /// the client sends and answers nothing, and counts the connections that
    /// are open.
    struct Relay {
        url: String,
        events: tokio::sync::mpsc::UnboundedReceiver<bool>,
        open: usize,
    }

    impl Relay {
        async fn start() -> Self {
            use futures_util::StreamExt;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://127.0.0.1:{}", listener.local_addr().unwrap().port());
            let (tx, events) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
                        let _ = tx.send(true);
                        while let Some(Ok(_)) = ws.next().await {}
                        let _ = tx.send(false);
                    });
                }
            });
            Self { url, events, open: 0 }
        }

        /// Follows the connections until `done` holds of the number open, or
        /// `wait` runs out; whether it held.
        async fn until(&mut self, wait: Duration, done: impl Fn(usize) -> bool) -> bool {
            let deadline = tokio::time::Instant::now() + wait;
            while !done(self.open) {
                match tokio::time::timeout_at(deadline, self.events.recv()).await {
                    Ok(Some(opened)) => self.count(opened),
                    _ => return false,
                }
            }
            true
        }

        fn count(&mut self, opened: bool) {
            if opened {
                self.open += 1;
            } else {
                self.open -= 1;
            }
        }

        /// No connection opens for `wait`.
        async fn quiet(&mut self, wait: Duration) -> bool {
            let before = self.open;
            !self.until(wait, |open| open > before).await
        }

        /// The connections once nothing has changed for a second: a client
        /// that rebuilds its pool closes one and opens another.
        async fn settled(&mut self) -> usize {
            while let Ok(Some(opened)) = tokio::time::timeout(Duration::from_secs(1), self.events.recv()).await {
                self.count(opened);
            }
            self.open
        }
    }

    /// Switched off and on in one process (platform-spec 12, 19 № 29): the
    /// stop aborts every task that served the runtime and closes its relay
    /// connections even while a command still holds the runtime, nothing
    /// connects while it is off, the commands answer `messenger_stopped`, and
    /// a start over the same folder finds the identity and connects again.
    /// The relay is a local socket: the test needs no network.
    #[tokio::test]
    async fn the_messenger_stops_and_starts_again_in_one_process() {
        let dir = tempfile::tempdir().unwrap();
        let lock_schema = veydan_core::Schema { module: "lock", steps: veydan_lock::SCHEMA_STEPS };
        let db = veydan_core::db::open(&dir.path().join(veydan_core::db::DB_FILE), &[veydan_core::SCHEMA, lock_schema])
            .await
            .unwrap();
        let lock = Arc::new(Lock::new(db.clone()));
        lock.open_default().await.unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(LockBox(lock.clone()));
        let config = MessengerConfig::new(dir.path().join(DATA_SUBDIR));
        let mut relay = Relay::start().await;
        let wait = Duration::from_secs(20);

        // A task like the watchers: it holds the runtime until it is aborted.
        let serve = |rt: &Arc<MessengerRuntime>| {
            let rt = rt.clone();
            vec![tauri::async_runtime::spawn(async move {
                loop {
                    let _ = rt.relays().list().await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })]
        };
        let slot = Slot::default();
        assert!(matches!(slot.runtime(), Err(AppError::Other(m)) if m == STOPPED));
        slot.start(MessengerRuntime::start(config.clone(), secrets.clone()), serve).await.unwrap();
        let rt = slot.runtime().unwrap();
        rt.relays().add_user(&relay.url, None).await.unwrap();
        rt.servers_use_own().await.unwrap();
        let npub = rt.identity().create("pw").await.unwrap().identity.npub;
        // The session starts as at the app's first status call.
        tokio::time::timeout(wait, rt.refresh_signer()).await.unwrap().unwrap();
        assert!(relay.until(wait, |open| open > 0).await, "the session did not connect");
        assert!(relay.settled().await > 0);
        // A second start while it runs changes nothing.
        slot.start(async { Err(MessengerError::Invalid("not called".into())) }, serve).await.unwrap();
        assert!(Arc::ptr_eq(&rt, &slot.runtime().unwrap()));

        // A command under way holds the runtime through the stop.
        let held = Arc::downgrade(&rt);
        slot.stop(|rt| async move { assert!(rt.status().await.is_ok()) }).await;
        assert!(relay.until(Duration::from_secs(10), |open| open == 0).await, "a relay connection stays open");
        assert!(matches!(slot.runtime(), Err(AppError::Other(m)) if m == STOPPED));
        assert_eq!(slot.error(), None);
        drop(rt);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while held.upgrade().is_some() {
            assert!(tokio::time::Instant::now() < deadline, "a task still holds the runtime");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(relay.quiet(Duration::from_secs(3)).await, "something connected while the messenger was off");
        // A second stop does nothing.
        slot.stop(|_| async { panic!("nothing runs") }).await;

        slot.start(MessengerRuntime::start(config.clone(), secrets.clone()), serve).await.unwrap();
        let rt = slot.runtime().unwrap();
        assert_eq!(rt.identity().get().await.unwrap().map(|i| i.npub), Some(npub));
        tokio::time::timeout(wait, rt.refresh_signer()).await.unwrap().unwrap();
        assert!(relay.until(wait, |open| open > 0).await, "the session did not connect again");
        assert!(relay.settled().await > 0);
        drop(rt);
        slot.stop(|_| async {}).await;
        assert!(relay.until(Duration::from_secs(10), |open| open == 0).await);

        // A start that fails leaves the slot empty with the reason.
        let failed = slot.start(async { Err(MessengerError::Storage("broken".into())) }, serve).await;
        assert!(matches!(failed, Err(AppError::Other(_))));
        let reason = slot.error().unwrap();
        assert!(reason.contains("broken"), "{reason}");
        assert!(matches!(slot.runtime(), Err(AppError::Other(m)) if m == reason));
        db.close().await;
    }

    /// What the shell's `open_source` gives for a `content://` URI: a name
    /// from another app and a way to read the file.
    fn picked(name: &str, file: &Path) -> veydan_shell::FileSource {
        let file = file.to_owned();
        veydan_shell::FileSource {
            name: name.into(),
            len: None,
            open: Arc::new(move || std::fs::File::open(&file)),
        }
    }

    #[tokio::test]
    async fn a_picked_file_is_copied_into_the_folder_of_the_messenger_under_its_last_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.bin");
        std::fs::write(&file, b"the bytes of a picked file").unwrap();
        let data_dir = dir.path().join(DATA_SUBDIR);

        let copied = copy_picked(picked("Download/../photos/IMG_1.jpg", &file), &data_dir, 1 << 20).await.unwrap();
        assert_eq!(copied.file_name().unwrap(), "IMG_1.jpg");
        assert_eq!(copied.parent().unwrap().parent().unwrap(), data_dir.join("outgoing"));
        assert_eq!(std::fs::read(&copied).unwrap(), b"the bytes of a picked file");

        // A name that would hide the file or say nothing becomes `file`.
        for name in [".profile", "  ", "a\\b\\", "dir/"] {
            let copied = copy_picked(picked(name, &file), &data_dir, 1 << 20).await.unwrap();
            assert_eq!(copied.file_name().unwrap(), "file", "{name:?}");
        }
    }

    /// A file larger than may be sent is refused: before the copy when its
    /// source tells its size, else once the copy grows past the limit, and
    /// nothing of it stays.
    #[tokio::test]
    async fn a_picked_file_too_large_to_send_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.bin");
        std::fs::write(&file, [7u8; 100]).unwrap();
        let data_dir = dir.path().join(DATA_SUBDIR);
        // Told by the name its source gives, which a person reads.
        let too_large = |e: AppError| assert_eq!(e.to_string(), "err.file_too_large: big.bin");

        let told = veydan_shell::FileSource { len: Some(100), ..picked("big.bin", &file) };
        too_large(copy_picked(told, &data_dir, 99).await.unwrap_err());
        assert!(!data_dir.exists(), "nothing was copied");
        too_large(copy_picked(picked("big.bin", &file), &data_dir, 99).await.unwrap_err());
        let left = std::fs::read_dir(data_dir.join("outgoing")).unwrap().count();
        assert_eq!(left, 0, "the copy went");

        let copied = copy_picked(picked("fits.bin", &file), &data_dir, 100).await.unwrap();
        assert_eq!(std::fs::read(&copied).unwrap().len(), 100, "exactly the limit goes");
    }
}

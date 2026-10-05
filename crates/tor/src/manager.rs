// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The running tor processes: one instance per set of exit countries, each
//! started when a consumer first asks for it and stopped when nobody has
//! used it for a while.
//!
//! A consumer (a browser profile, an SSH session) calls
//! [`TorManager::acquire`] with its exit countries and gets a [`Lease`]: the
//! SOCKS port of the instance and credentials of its own, so tor keeps its
//! streams on circuits apart from those of other consumers (IsolateSOCKSAuth,
//! on by default). Dropping the lease lets the instance go.
//!
//! Each instance lives in `<data>/tor/instances/<key>/` (torrc, the data
//! directory of tor, the control port file and the cookie) and is run by a
//! task of its own: it starts tor, takes ownership of it over the control
//! port, waits for the bootstrap, starts it again if it dies while it is
//! used, and stops it when it idles out, when the user stops it, or when the
//! app exits. tor also exits by itself when the app is gone: the control
//! connection that took ownership closes, and `__OwningControllerProcess`
//! names the app's pid.

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::control::{Bootstrap, Control};
use crate::settings::{self, TorSettings};
use crate::source::{self, Bundle, INSTALL_DIR_LOCK};
use crate::torrc::{self, PtConfig, TorrcPaths};

/// How many instances run at once at most.
pub const MAX_INSTANCES: usize = 8;
/// How long tor may take to reach 100 % of its bootstrap.
pub const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180);
/// How long tor may take to open its control port.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the bootstrap is asked for while it runs.
const POLL: Duration = Duration::from_millis(500);
/// How long tor may take to exit after SIGNAL SHUTDOWN before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// The lines of the output of tor kept per instance.
pub const LOG_LINES: usize = 500;
/// How often in a row an instance that dies is started again.
const MAX_RESTARTS: u32 = 5;
/// An instance that ran this long has its restarts counted from zero again.
const STABLE: Duration = Duration::from_secs(60);

// ── Exit countries ─────────────────────────────────────────────────────────

/// The exit countries of an instance: lower-case ISO 3166 alpha-2 codes,
/// without repeats, sorted. Empty: any exit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExitSet(Vec<String>);

impl ExitSet {
    pub fn any() -> Self {
        Self(Vec::new())
    }

    /// `"DE, nl"` → `de`, `nl`. Codes are separated by commas, semicolons
    /// or spaces; an empty text is any exit.
    pub fn parse(text: &str) -> Result<Self, TorError> {
        Self::from_codes(
            text.split([',', ';', ' ', '\t'])
                .map(str::trim)
                .filter(|c| !c.is_empty()),
        )
    }

    pub fn from_codes<I, S>(codes: I) -> Result<Self, TorError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Vec::new();
        for code in codes {
            out.push(settings::country(code.as_ref()).map_err(TorError::InvalidExit)?);
        }
        out.sort();
        out.dedup();
        Ok(Self(out))
    }

    pub fn codes(&self) -> &[String] {
        &self.0
    }

    pub fn is_any(&self) -> bool {
        self.0.is_empty()
    }

    /// A name safe as a directory name: `any` or `de-nl`.
    pub fn key(&self) -> String {
        if self.0.is_empty() {
            "any".to_string()
        } else {
            self.0.join("-")
        }
    }
}

impl std::fmt::Display for ExitSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key())
    }
}

// ── Errors ─────────────────────────────────────────────────────────────────

/// Why tor could not be had. Shown as `<code>: <text>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorError {
    /// No bundle on this device (or tor is not available on this platform).
    NotInstalled,
    /// The text of the error, starting with `tor_invalid_country`.
    InvalidExit(String),
    InvalidConsumer,
    TooManyInstances(usize),
    /// tor did not reach 100 % in [`BOOTSTRAP_TIMEOUT`]; where it stood.
    BootstrapTimeout {
        progress: u8,
        summary: String,
        warning: Option<String>,
    },
    /// The settings and the bundle make no torrc; the text starts with its
    /// own code (`tor_no_builtin_bridges`, `tor_bad_transport_path`, …).
    Config(String),
    /// tor could not be started, or exited.
    Failed(String),
    /// The instance was stopped while the caller waited; also every call
    /// once the app exits.
    Stopped,
    UnknownInstance(String),
    /// Consumers hold leases on tor.
    InUse(u32),
}

impl std::fmt::Display for TorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled => write!(f, "tor_not_installed: Tor is not installed"),
            Self::InvalidExit(text) | Self::Config(text) => f.write_str(text),
            Self::InvalidConsumer => write!(
                f,
                "tor_invalid_consumer: a consumer id is 1 to 255 bytes"
            ),
            Self::TooManyInstances(limit) => write!(
                f,
                "tor_too_many_instances: at most {limit} tor instances run at once"
            ),
            Self::BootstrapTimeout {
                progress,
                summary,
                warning,
            } => {
                write!(
                    f,
                    "tor_bootstrap_timeout: tor did not connect in {} s, it stopped at {progress} % ({summary})",
                    BOOTSTRAP_TIMEOUT.as_secs()
                )?;
                if let Some(warning) = warning {
                    write!(f, ": {warning}")?;
                }
                Ok(())
            }
            Self::Failed(text) => write!(f, "tor_failed: {text}"),
            Self::Stopped => write!(f, "tor_stopped: the tor instance was stopped"),
            Self::UnknownInstance(key) => write!(f, "tor_unknown_instance: {key}"),
            Self::InUse(n) => write!(f, "tor_in_use: {n} consumer(s) use tor"),
        }
    }
}

impl std::error::Error for TorError {}

impl From<TorError> for veydan_core::AppError {
    fn from(e: TorError) -> Self {
        Self::Other(e.to_string())
    }
}

// ── The output of tor ──────────────────────────────────────────────────────

/// The last lines of the output of tor, in memory only.
#[derive(Debug)]
pub struct LogBuffer {
    lines: VecDeque<String>,
    cap: usize,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(cap.min(64)),
            cap,
        }
    }

    pub fn push(&mut self, line: String) {
        if self.cap == 0 {
            return;
        }
        if self.lines.len() == self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// The last warning or error tor wrote.
    fn last_problem(&self) -> Option<String> {
        self.lines
            .iter()
            .rev()
            .find(|l| l.contains("[err]") || l.contains("[warn]"))
            .cloned()
    }
}

// ── What the UI sees ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Starting,
    Ready,
    /// Started again: it died while used, or the settings changed.
    Restarting,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceInfo {
    pub key: String,
    pub exit: Vec<String>,
    pub state: InstanceState,
    /// The bootstrap, 0 to 100.
    pub bootstrap: u8,
    /// What tor says it is doing, or its last bootstrap warning.
    pub summary: String,
    pub socks_port: Option<u16>,
    /// Leases held.
    pub consumers: u32,
    /// Started by hand (`tor_start`): runs until `tor_stop`.
    pub kept: bool,
    /// The settings changed while consumers held it: they apply at its next
    /// start.
    pub restart_needed: bool,
    pub error: Option<String>,
}

// ── Leases ─────────────────────────────────────────────────────────────────

/// A consumer's hold on an instance. Its streams go to
/// `127.0.0.1:socks_port` with `username` and `password` as SOCKS5
/// credentials. Dropping the last clone releases it.
#[derive(Clone)]
pub struct Lease {
    pub socks_port: u16,
    /// The consumer id.
    pub username: String,
    /// Random per lease.
    pub password: String,
    key: String,
    _hold: Arc<Hold>,
}

impl Lease {
    /// The key of the instance (`any`, `de-nl`).
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn socks_addr(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.socks_port))
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("key", &self.key)
            .field("socks_port", &self.socks_port)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Counts one lease; its drop tells the task of the instance, which then
/// sees whether it idles.
struct Hold {
    instance: Arc<Instance>,
}

impl Hold {
    /// Called under the lock of the instances.
    fn take(instance: &Arc<Instance>) -> Self {
        instance.leases.fetch_add(1, Ordering::SeqCst);
        Self {
            instance: Arc::clone(instance),
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.instance.leases.fetch_sub(1, Ordering::SeqCst);
        let _ = self.instance.cmd.send(Cmd::Wake);
    }
}

fn random_password() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Isolation needs the credentials to differ, not to be secret.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── Instances ──────────────────────────────────────────────────────────────

enum Cmd {
    /// Leases or settings changed: look at the idle timer again.
    Wake,
    /// Stop tor and start it again with the current settings.
    Restart,
    Stop,
    NewIdentity(oneshot::Sender<Result<(), String>>),
}

#[derive(Debug, Clone)]
struct View {
    state: InstanceState,
    bootstrap: u8,
    summary: String,
    socks_port: Option<u16>,
    restart_needed: bool,
    failure: Option<TorError>,
    /// The task has ended and the instance is out of the list.
    gone: bool,
}

impl View {
    fn starting(state: InstanceState) -> Self {
        Self {
            state,
            bootstrap: 0,
            summary: String::new(),
            socks_port: None,
            restart_needed: false,
            failure: None,
            gone: false,
        }
    }
}

struct Instance {
    key: String,
    exit: ExitSet,
    leases: AtomicU32,
    kept: AtomicBool,
    view: watch::Sender<View>,
    log: Mutex<LogBuffer>,
    /// The torrc tor was last started with.
    torrc: Mutex<String>,
    /// The SOCKS port of the last start: a start after it asks tor for the
    /// same one, so the leases stay right. Forgotten when a start fails.
    socks_port: Mutex<Option<u16>>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl Instance {
    fn state(&self) -> InstanceState {
        self.view.borrow().state
    }

    fn info(&self) -> InstanceInfo {
        let view = self.view.borrow();
        InstanceInfo {
            key: self.key.clone(),
            exit: self.exit.codes().to_vec(),
            state: view.state,
            bootstrap: view.bootstrap,
            summary: view.summary.clone(),
            socks_port: view.socks_port,
            consumers: self.leases.load(Ordering::SeqCst),
            kept: self.kept.load(Ordering::SeqCst),
            restart_needed: view.restart_needed,
            error: view.failure.as_ref().map(ToString::to_string),
        }
    }

    fn log_line(&self, line: String) {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line);
    }
}

type OnChange = Box<dyn Fn(Vec<InstanceInfo>) + Send + Sync>;

struct Inner {
    app_data_dir: PathBuf,
    settings: RwLock<TorSettings>,
    instances: Mutex<HashMap<String, Arc<Instance>>>,
    on_change: OnChange,
    exiting: AtomicBool,
}

/// The tor instances of the app. Cheap to clone; managed by the module.
#[derive(Clone)]
pub struct TorManager {
    inner: Arc<Inner>,
}

impl TorManager {
    /// `on_change` hears the whole list whenever any of it changes.
    pub fn new(
        app_data_dir: PathBuf,
        settings: TorSettings,
        on_change: impl Fn(Vec<InstanceInfo>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                app_data_dir,
                settings: RwLock::new(settings),
                instances: Mutex::new(HashMap::new()),
                on_change: Box::new(on_change),
                exiting: AtomicBool::new(false),
            }),
        }
    }

    /// The SOCKS endpoint of the instance for `exit`, started if it is not
    /// running, once its bootstrap reached 100 %. `consumer` names the
    /// caller (`profile:<id>`, `ssh:<id>`) and becomes the SOCKS username.
    /// Calls for the same set at the same time share one start.
    pub async fn acquire(&self, exit: &ExitSet, consumer: &str) -> Result<Lease, TorError> {
        if consumer.is_empty() || consumer.len() > 255 {
            return Err(TorError::InvalidConsumer);
        }
        let hold = self
            .inner
            .hold(exit, Hand::Lease)
            .await?
            .expect("a lease is always held");
        let instance = Arc::clone(&hold.instance);
        let mut view = instance.view.subscribe();
        let waited = tokio::time::timeout(
            BOOTSTRAP_TIMEOUT + CONTROL_TIMEOUT + Duration::from_secs(30),
            view.wait_for(|v| {
                v.gone
                    || matches!(
                        v.state,
                        InstanceState::Ready | InstanceState::Failed | InstanceState::Stopping
                    )
            }),
        )
        .await;
        let outcome = match waited {
            Err(_) => {
                let v = instance.view.borrow();
                Err(TorError::BootstrapTimeout {
                    progress: v.bootstrap,
                    summary: v.summary.clone(),
                    warning: None,
                })
            }
            Ok(Err(_)) => Err(TorError::Stopped),
            Ok(Ok(v)) => match (v.state, v.socks_port) {
                (InstanceState::Ready, Some(port)) if !v.gone => Ok(port),
                (InstanceState::Failed, _) => Err(v
                    .failure
                    .clone()
                    .unwrap_or_else(|| TorError::Failed("tor did not start".into()))),
                _ => Err(TorError::Stopped),
            },
        };
        let socks_port = outcome?;
        self.inner.changed();
        Ok(Lease {
            socks_port,
            username: consumer.to_string(),
            password: random_password(),
            key: instance.key.clone(),
            _hold: Arc::new(hold),
        })
    }

    /// Starts the instance for `exit` by hand: it runs, used or not, until
    /// [`TorManager::stop`]. Returns at once; the bootstrap shows in the
    /// list.
    pub async fn start(&self, exit: &ExitSet) -> Result<InstanceInfo, TorError> {
        self.inner.hold(exit, Hand::Keep).await?;
        let instance = self.inner.get(&exit.key())?;
        Ok(instance.info())
    }

    /// The instance stops being kept by hand, and stops now if no consumer
    /// holds it; with consumers it keeps running for them and the answer is
    /// [`TorError::InUse`].
    pub fn stop(&self, key: &str) -> Result<(), TorError> {
        let map = self.inner.lock_instances();
        let instance = map
            .get(key)
            .ok_or_else(|| TorError::UnknownInstance(key.to_string()))?;
        instance.kept.store(false, Ordering::SeqCst);
        let leases = instance.leases.load(Ordering::SeqCst);
        if leases > 0 {
            let _ = instance.cmd.send(Cmd::Wake);
            drop(map);
            self.inner.changed();
            return Err(TorError::InUse(leases));
        }
        begin_stop(instance);
        drop(map);
        self.inner.changed();
        Ok(())
    }

    /// New circuits for new streams (SIGNAL NEWNYM).
    pub async fn new_identity(&self, key: &str) -> Result<(), TorError> {
        let instance = self.inner.get(key)?;
        if instance.state() != InstanceState::Ready {
            return Err(TorError::Failed("the instance is not ready".into()));
        }
        let (tx, rx) = oneshot::channel();
        instance
            .cmd
            .send(Cmd::NewIdentity(tx))
            .map_err(|_| TorError::Stopped)?;
        rx.await
            .map_err(|_| TorError::Stopped)?
            .map_err(TorError::Failed)
    }

    /// The last lines tor wrote, oldest first.
    pub fn log(&self, key: &str) -> Result<Vec<String>, TorError> {
        let instance = self.inner.get(key)?;
        let lines = instance
            .log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .lines();
        Ok(lines)
    }

    pub fn instances(&self) -> Vec<InstanceInfo> {
        self.inner.list()
    }

    pub fn settings(&self) -> TorSettings {
        self.inner.settings()
    }

    /// Takes settings that were validated and stored. An instance whose
    /// torrc they change starts again at once when no consumer holds it or
    /// it is not ready yet; a ready instance with consumers is marked
    /// `restart_needed` and takes them at its next start.
    pub async fn apply_settings(&self, settings: TorSettings) {
        *self.inner.settings.write().unwrap_or_else(|e| e.into_inner()) = settings;
        let bundle = source::resolve(&self.inner.app_data_dir);
        let instances: Vec<Arc<Instance>> =
            self.inner.lock_instances().values().cloned().collect();
        for instance in instances {
            let state = instance.state();
            if matches!(state, InstanceState::Stopping) {
                continue;
            }
            let fresh = bundle
                .as_ref()
                .and_then(|b| self.inner.render(&instance, b).ok())
                .map(|(text, _)| text);
            let current = instance
                .torrc
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let changed = fresh.as_deref() != Some(current.as_str());
            let busy = state == InstanceState::Ready
                && instance.leases.load(Ordering::SeqCst) > 0;
            if changed && busy {
                instance.view.send_modify(|v| v.restart_needed = true);
            } else if changed {
                let _ = instance.cmd.send(Cmd::Restart);
            } else {
                instance.view.send_if_modified(|v| {
                    std::mem::replace(&mut v.restart_needed, false)
                });
            }
            let _ = instance.cmd.send(Cmd::Wake);
        }
        self.inner.changed();
        self.start_with_app().await;
    }

    /// The module's start: the instance with no exit countries starts with
    /// the app when the settings say so and tor is installed.
    pub async fn start_with_app(&self) {
        if !self.inner.settings().start_with_app
            || source::resolve(&self.inner.app_data_dir).is_none()
        {
            return;
        }
        if let Err(e) = self.inner.hold(&ExitSet::any(), Hand::Pinned).await {
            eprintln!("tor: cannot start with the app: {e}");
        }
    }

    /// Stops every instance and refuses to start any more: the app exits.
    pub async fn stop_all(&self) {
        self.inner.exiting.store(true, Ordering::SeqCst);
        self.stop_every().await;
    }

    /// Lets go of the files of the bundle before they are removed or
    /// replaced: instances nobody holds are stopped; with consumers it is
    /// [`TorError::InUse`] and nothing is stopped. The caller holds
    /// [`INSTALL_DIR_LOCK`], so nothing starts in between.
    pub async fn release_bundle(&self) -> Result<(), TorError> {
        let leases: u32 = self
            .inner
            .lock_instances()
            .values()
            .filter(|i| i.state() != InstanceState::Stopping)
            .map(|i| i.leases.load(Ordering::SeqCst))
            .sum();
        if leases > 0 {
            return Err(TorError::InUse(leases));
        }
        self.stop_every().await;
        Ok(())
    }

    /// Stops every instance and waits (bounded) until the processes are gone.
    async fn stop_every(&self) {
        let mut waits = Vec::new();
        {
            let map = self.inner.lock_instances();
            for instance in map.values() {
                instance.kept.store(false, Ordering::SeqCst);
                begin_stop(instance);
                waits.push(instance.view.subscribe());
            }
        }
        self.inner.changed();
        let all = async {
            for mut view in waits {
                let _ = view.wait_for(|v| v.gone).await;
            }
        };
        let _ = tokio::time::timeout(STOP_GRACE + Duration::from_secs(5), all).await;
    }

    /// `<data>/tor/instances/<key>/`.
    pub fn instance_dir(&self, key: &str) -> PathBuf {
        self.inner.instance_dir(key)
    }
}

/// Marks the instance as stopping and tells its task. Under the lock of the
/// instances, so that an `acquire` sees it and waits for the end.
fn begin_stop(instance: &Instance) {
    instance.view.send_if_modified(|v| {
        if v.state == InstanceState::Stopping || v.gone {
            return false;
        }
        v.state = InstanceState::Stopping;
        true
    });
    let _ = instance.cmd.send(Cmd::Stop);
}

/// How the caller of [`Inner::hold`] holds the instance.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hand {
    Lease,
    /// By the user, until `tor_stop`.
    Keep,
    /// By `start_with_app`; the setting itself keeps it.
    Pinned,
}

impl Inner {
    fn lock_instances(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Instance>>> {
        self.instances.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> TorSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn get(&self, key: &str) -> Result<Arc<Instance>, TorError> {
        self.lock_instances()
            .get(key)
            .cloned()
            .ok_or_else(|| TorError::UnknownInstance(key.to_string()))
    }

    fn tor_root(&self) -> PathBuf {
        self.app_data_dir.join("tor")
    }

    fn instance_dir(&self, key: &str) -> PathBuf {
        self.tor_root().join("instances").join(key)
    }

    fn list(&self) -> Vec<InstanceInfo> {
        let mut list: Vec<InstanceInfo> = self
            .lock_instances()
            .values()
            .map(|i| i.info())
            .collect();
        list.sort_by(|a, b| (!a.exit.is_empty(), &a.key).cmp(&(!b.exit.is_empty(), &b.key)));
        list
    }

    fn changed(&self) {
        (self.on_change)(self.list());
    }

    /// Whether anything keeps the instance running.
    fn wanted(&self, instance: &Instance) -> bool {
        instance.leases.load(Ordering::SeqCst) > 0
            || instance.kept.load(Ordering::SeqCst)
            || (instance.exit.is_any() && self.settings().start_with_app)
    }

    /// Finds or starts the instance for `exit` and holds it as `hand` says;
    /// a lease comes back as its [`Hold`]. An instance on its way out is
    /// waited for first: two tor processes cannot share a data directory.
    async fn hold(self: &Arc<Self>, exit: &ExitSet, hand: Hand) -> Result<Option<Hold>, TorError> {
        let key = exit.key();
        loop {
            let mut ending = {
                let mut map = self.lock_instances();
                if self.exiting.load(Ordering::SeqCst) {
                    return Err(TorError::Stopped);
                }
                match map.get(&key) {
                    Some(instance) if instance.state() == InstanceState::Stopping => {
                        instance.view.subscribe()
                    }
                    Some(instance) => {
                        // Held under the lock: `retire` decides under it too.
                        let instance = Arc::clone(instance);
                        let hold = self.take(&instance, hand);
                        if instance.state() == InstanceState::Failed {
                            instance
                                .view
                                .send_replace(View::starting(InstanceState::Starting));
                            let _ = instance.cmd.send(Cmd::Restart);
                        }
                        drop(map);
                        self.changed();
                        return Ok(hold);
                    }
                    None => {
                        let running = map
                            .values()
                            .filter(|i| i.state() != InstanceState::Failed)
                            .count();
                        if running >= MAX_INSTANCES {
                            return Err(TorError::TooManyInstances(MAX_INSTANCES));
                        }
                        let (cmd, rx) = mpsc::unbounded_channel();
                        let instance = Arc::new(Instance {
                            key: key.clone(),
                            exit: exit.clone(),
                            leases: AtomicU32::new(0),
                            kept: AtomicBool::new(false),
                            view: watch::Sender::new(View::starting(InstanceState::Starting)),
                            log: Mutex::new(LogBuffer::new(LOG_LINES)),
                            torrc: Mutex::new(String::new()),
                            socks_port: Mutex::new(None),
                            cmd,
                        });
                        let hold = self.take(&instance, hand);
                        map.insert(key.clone(), Arc::clone(&instance));
                        drop(map);
                        let inner = Arc::clone(self);
                        tauri::async_runtime::spawn(run(inner, instance, rx));
                        self.changed();
                        return Ok(hold);
                    }
                }
            };
            let _ = ending.wait_for(|v| v.gone).await;
        }
    }

    fn take(&self, instance: &Arc<Instance>, hand: Hand) -> Option<Hold> {
        match hand {
            Hand::Lease => Some(Hold::take(instance)),
            Hand::Keep => {
                instance.kept.store(true, Ordering::SeqCst);
                let _ = instance.cmd.send(Cmd::Wake);
                None
            }
            Hand::Pinned => {
                let _ = instance.cmd.send(Cmd::Wake);
                None
            }
        }
    }

    /// The paths torrc names for `instance`, tor running in `<data>/tor/`.
    fn paths(&self, instance: &Instance, bundle: &Bundle) -> Result<TorrcPaths, TorError> {
        let cwd = self.tor_root();
        let dir = self.instance_dir(&instance.key);
        let name = |p: PathBuf| torrc::torrc_path(&p, &cwd, cfg!(windows));
        Ok(TorrcPaths {
            data_dir: name(dir.join("data")),
            control_port_file: name(dir.join("control.port")),
            cookie_file: name(dir.join("control.cookie")),
            geoip: name(bundle.geoip.clone()),
            geoip6: name(bundle.geoip6.clone()),
            pt_path: torrc::pt_path(&bundle.pt_dir, &cwd).map_err(TorError::Config)?,
        })
    }

    /// The torrc of `instance` under the current settings, and the fixed
    /// external SOCKS port it opens.
    fn render(&self, instance: &Instance, bundle: &Bundle) -> Result<(String, Option<u16>), TorError> {
        let settings = self.settings();
        let pt = PtConfig::read(&bundle.pt_dir).ok();
        let paths = self.paths(instance, bundle)?;
        let external = settings
            .external_socks_port
            .filter(|_| instance.exit.is_any());
        // The port of the last start, unless it became the external one.
        let port = (*instance.socks_port.lock().unwrap_or_else(|e| e.into_inner()))
            .filter(|p| Some(*p) != external);
        let text = torrc::torrc(
            &settings,
            &instance.exit,
            port,
            &paths,
            std::process::id(),
            pt.as_ref(),
        )
        .map_err(TorError::Config)?;
        Ok((text, external))
    }

    /// The idle instance leaves the list as stopping, if nothing took it in
    /// the meantime.
    fn retire(&self, instance: &Instance) -> bool {
        let _map = self.lock_instances();
        if self.wanted(instance) {
            return false;
        }
        begin_stop(instance);
        true
    }

    /// The task of the instance has ended.
    fn remove(&self, instance: &Arc<Instance>) {
        {
            let mut map = self.lock_instances();
            if map
                .get(&instance.key)
                .is_some_and(|i| Arc::ptr_eq(i, instance))
            {
                map.remove(&instance.key);
            }
            instance.view.send_modify(|v| {
                v.state = InstanceState::Stopping;
                v.gone = true;
                v.socks_port = None;
            });
        }
        self.changed();
    }
}

// ── The task of an instance ────────────────────────────────────────────────

/// What ends a wait of the task.
enum Next {
    Restart,
    Stop,
}

async fn run(inner: Arc<Inner>, instance: Arc<Instance>, mut rx: mpsc::UnboundedReceiver<Cmd>) {
    let mut restarts: u32 = 0;
    let mut state = InstanceState::Starting;
    'life: loop {
        // A stop that came in between is kept.
        let mut stopping = false;
        instance.view.send_if_modified(|v| {
            stopping = v.state == InstanceState::Stopping;
            if !stopping {
                *v = View::starting(state);
            }
            !stopping
        });
        if stopping || inner.exiting.load(Ordering::SeqCst) {
            break;
        }
        inner.changed();

        let mut child: Option<Child> = None;
        let mut control: Option<Control> = None;
        let booted = tokio::select! {
            r = boot(&inner, &instance, &mut child, &mut control) => Ok(r),
            next = interrupted(&mut rx) => Err(next),
        };
        let started_at = Instant::now();
        let port = match booted {
            Err(next) => {
                shutdown(child, control).await;
                match next {
                    Next::Restart if instance.state() != InstanceState::Stopping => {
                        state = InstanceState::Restarting;
                        continue 'life;
                    }
                    _ => break 'life,
                }
            }
            Ok(Err(e)) => {
                shutdown(child, control).await;
                instance.log_line(format!("-- {e}"));
                // The port may be taken by now: the next start lets tor pick.
                *instance.socks_port.lock().unwrap_or_else(|e| e.into_inner()) = None;
                // A start after a crash is tried again while consumers wait.
                if restarts > 0 && restarts < MAX_RESTARTS && inner.wanted(&instance) {
                    match backoff(&inner, &instance, &mut rx, restarts).await {
                        Some(Next::Stop) => break 'life,
                        _ => {
                            restarts += 1;
                            state = InstanceState::Restarting;
                            continue 'life;
                        }
                    }
                }
                match fail(&inner, &instance, &mut rx, e).await {
                    Next::Restart => {
                        restarts = 0;
                        state = InstanceState::Starting;
                        continue 'life;
                    }
                    Next::Stop => break 'life,
                }
            }
            Ok(Ok(port)) => port,
        };
        *instance.socks_port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
        let (Some(mut child), Some(mut control)) = (child, control) else {
            break 'life;
        };
        instance.view.send_modify(|v| {
            v.state = InstanceState::Ready;
            v.bootstrap = 100;
            v.socks_port = Some(port);
            v.failure = None;
        });
        inner.changed();

        // Running: until tor dies, a command, or the idle timer.
        let mut idle_since: Option<Instant> = None;
        let died = loop {
            let deadline = if inner.wanted(&instance) {
                idle_since = None;
                None
            } else {
                let since = *idle_since.get_or_insert_with(Instant::now);
                let minutes = u64::from(inner.settings().idle_minutes);
                Some(since + Duration::from_secs(minutes * 60))
            };
            tokio::select! {
                status = child.wait() => {
                    break match status {
                        Ok(s) => format!("tor exited ({s})"),
                        Err(e) => format!("tor is lost: {e}"),
                    };
                }
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Wake) => {}
                    Some(Cmd::NewIdentity(reply)) => {
                        let _ = reply.send(control.signal("NEWNYM").await);
                    }
                    Some(Cmd::Restart) if instance.state() != InstanceState::Stopping => {
                        shutdown(Some(child), Some(control)).await;
                        state = InstanceState::Restarting;
                        continue 'life;
                    }
                    _ => {
                        shutdown(Some(child), Some(control)).await;
                        break 'life;
                    }
                },
                _ = sleep_until(deadline) => {
                    if inner.retire(&instance) {
                        instance.log_line("-- stopped: nobody used it".into());
                        shutdown(Some(child), Some(control)).await;
                        break 'life;
                    }
                }
            }
        };
        drop(control);
        instance.log_line(format!("-- {died}"));
        if inner.exiting.load(Ordering::SeqCst) || instance.state() == InstanceState::Stopping {
            break 'life;
        }
        // Nobody needs it: it does not come back.
        if inner.retire(&instance) {
            break 'life;
        }
        if started_at.elapsed() > STABLE {
            restarts = 0;
        }
        restarts += 1;
        if restarts > MAX_RESTARTS {
            let e = TorError::Failed(format!("{died}; it exited {MAX_RESTARTS} times in a row"));
            match fail(&inner, &instance, &mut rx, e).await {
                Next::Restart => {
                    restarts = 0;
                    state = InstanceState::Starting;
                    continue 'life;
                }
                Next::Stop => break 'life,
            }
        }
        instance.view.send_modify(|v| {
            v.state = InstanceState::Restarting;
            v.socks_port = None;
            v.summary = died.clone();
        });
        inner.changed();
        if let Some(Next::Stop) = backoff(&inner, &instance, &mut rx, restarts).await {
            break 'life;
        }
        state = InstanceState::Restarting;
    }
    inner.remove(&instance);
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Waits for a command that ends a start: a stop or a restart. A request
/// for a new identity is refused meanwhile.
async fn interrupted(rx: &mut mpsc::UnboundedReceiver<Cmd>) -> Next {
    loop {
        match rx.recv().await {
            Some(Cmd::Wake) => {}
            Some(Cmd::NewIdentity(reply)) => {
                let _ = reply.send(Err("the instance is not ready".into()));
            }
            Some(Cmd::Restart) => return Next::Restart,
            Some(Cmd::Stop) | None => return Next::Stop,
        }
    }
}

/// The pause before start number `attempt` + 1 after a crash: 1, 2, 4, 8,
/// 16 s. `Some` when a command cut it short.
async fn backoff(
    inner: &Inner,
    instance: &Instance,
    rx: &mut mpsc::UnboundedReceiver<Cmd>,
    attempt: u32,
) -> Option<Next> {
    let pause = Duration::from_secs(1 << attempt.saturating_sub(1).min(4));
    tokio::select! {
        _ = tokio::time::sleep(pause) => None,
        next = interrupted(rx) => {
            let stop = matches!(next, Next::Stop)
                || instance.state() == InstanceState::Stopping
                || inner.exiting.load(Ordering::SeqCst);
            Some(if stop { Next::Stop } else { Next::Restart })
        }
    }
}

/// The instance failed: it stays in the list with the error until it is
/// started again, stopped, or idles out.
async fn fail(
    inner: &Inner,
    instance: &Instance,
    rx: &mut mpsc::UnboundedReceiver<Cmd>,
    e: TorError,
) -> Next {
    instance.view.send_modify(|v| {
        v.state = InstanceState::Failed;
        v.socks_port = None;
        v.failure = Some(e);
    });
    inner.changed();
    let mut idle_since: Option<Instant> = None;
    loop {
        let deadline = if inner.wanted(instance) {
            idle_since = None;
            None
        } else {
            let since = *idle_since.get_or_insert_with(Instant::now);
            let minutes = u64::from(inner.settings().idle_minutes);
            Some(since + Duration::from_secs(minutes * 60))
        };
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Wake) => {}
                Some(Cmd::NewIdentity(reply)) => {
                    let _ = reply.send(Err("the instance failed".into()));
                }
                Some(Cmd::Restart) if instance.state() != InstanceState::Stopping => {
                    return Next::Restart
                }
                _ => return Next::Stop,
            },
            _ = sleep_until(deadline) => {
                if inner.retire(instance) {
                    return Next::Stop;
                }
            }
        }
    }
}

/// Stops tor: SIGNAL SHUTDOWN, then a kill after [`STOP_GRACE`]. A tor that
/// has not handed its control port over yet is killed at once.
async fn shutdown(child: Option<Child>, control: Option<Control>) {
    let Some(mut child) = child else { return };
    let graceful = match control {
        Some(mut control) => {
            let sent = tokio::time::timeout(Duration::from_secs(2), control.signal("SHUTDOWN"))
                .await
                .is_ok_and(|r| r.is_ok());
            // Closing the connection that took ownership makes tor exit too.
            drop(control);
            sent
        }
        None => false,
    };
    if graceful
        && tokio::time::timeout(STOP_GRACE, child.wait())
            .await
            .is_ok()
    {
        return;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Creates the directory of an instance (unix: 0700, which tor wants of
/// its data directory) and clears the files a killed tor left.
fn prepare_dir(dir: &Path) -> Result<(), String> {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).map_err(|e| format!("cannot create {}: {e}", data.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [dir, data.as_path()] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("cannot protect {}: {e}", path.display()))?;
        }
    }
    for stale in ["control.port", "control.cookie"] {
        let _ = std::fs::remove_file(dir.join(stale));
    }
    // An empty defaults file, so that no torrc-defaults of a tor installed
    // on the system is read.
    std::fs::write(dir.join("torrc-defaults"), b"")
        .map_err(|e| format!("cannot write torrc-defaults: {e}"))?;
    Ok(())
}

/// Writes torrc; it may hold the password of the upstream proxy, so on unix
/// only the user reads it.
fn write_torrc(path: &Path, text: &str) -> Result<(), String> {
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Reads the lines of one stream of tor into the log of the instance. It
/// reads to the end whatever comes, so tor never blocks on a full pipe.
fn pipe(instance: Arc<Instance>, stream: impl AsyncRead + Unpin + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    instance.log_line(line.trim_end().to_string());
                }
            }
        }
    });
}

fn exited(status: std::process::ExitStatus, instance: &Instance) -> TorError {
    let problem = instance
        .log
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last_problem();
    TorError::Failed(match problem {
        Some(line) => format!("tor exited ({status}): {line}"),
        None => format!("tor exited ({status})"),
    })
}

/// Starts tor and brings it to 100 %; the SOCKS port for consumers. The
/// child and the control connection go into the slots as soon as they
/// exist, so that whoever cancels this future can still stop them.
async fn boot(
    inner: &Inner,
    instance: &Arc<Instance>,
    child_slot: &mut Option<Child>,
    control_slot: &mut Option<Control>,
) -> Result<u16, TorError> {
    let dir = inner.instance_dir(&instance.key);
    let port_file = dir.join("control.port");
    let cookie_file = dir.join("control.cookie");

    // The bundle is not swapped while tor starts from it.
    let external = {
        let _bundle_lock = INSTALL_DIR_LOCK.lock().await;
        let bundle = source::resolve(&inner.app_data_dir).ok_or(TorError::NotInstalled)?;
        prepare_dir(&dir).map_err(TorError::Failed)?;
        let (text, external) = inner.render(instance, &bundle)?;
        write_torrc(&dir.join("torrc"), &text).map_err(TorError::Failed)?;
        *instance.torrc.lock().unwrap_or_else(|e| e.into_inner()) = text;

        let mut command = Command::from(bundle.command());
        command
            .arg("-f")
            .arg(dir.join("torrc"))
            .arg("--defaults-torrc")
            .arg(dir.join("torrc-defaults"))
            .current_dir(inner.tor_root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| TorError::Failed(format!("cannot start {}: {e}", bundle.tor.display())))?;
        instance.log_line(format!(
            "-- started tor (pid {})",
            child.id().map_or_else(|| "?".to_string(), |p| p.to_string())
        ));
        if let Some(out) = child.stdout.take() {
            pipe(Arc::clone(instance), out);
        }
        if let Some(err) = child.stderr.take() {
            pipe(Arc::clone(instance), err);
        }
        *child_slot = Some(child);
        external
    };
    let child = child_slot.as_mut().expect("just started");

    // The control port: tor writes its file once it listens.
    let opened = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(exited(status, instance));
        }
        let ready = tokio::fs::read_to_string(&port_file)
            .await
            .ok()
            .and_then(|t| crate::control::parse_port_file(&t))
            .is_some()
            && cookie_file.is_file();
        if ready {
            break;
        }
        if opened.elapsed() > CONTROL_TIMEOUT {
            return Err(TorError::Failed("tor opened no control port".into()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let control = control_slot.insert(
        Control::connect(&port_file, &cookie_file)
            .await
            .map_err(TorError::Failed)?,
    );
    control.take_ownership().await.map_err(TorError::Failed)?;

    // The bootstrap.
    let started = Instant::now();
    let mut warning: Option<String> = None;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(exited(status, instance));
        }
        let Bootstrap {
            progress,
            summary,
            warning: now_warning,
            ..
        } = control.bootstrap().await.map_err(TorError::Failed)?;
        if now_warning.is_some() {
            warning = now_warning.clone();
        }
        let shown = match &now_warning {
            Some(w) => format!("{summary}: {w}"),
            None => summary.clone(),
        };
        let moved = instance.view.send_if_modified(|v| {
            if v.bootstrap == progress && v.summary == shown {
                return false;
            }
            v.bootstrap = progress;
            v.summary = shown.clone();
            true
        });
        if moved {
            inner.changed();
        }
        if progress >= 100 {
            break;
        }
        if started.elapsed() > BOOTSTRAP_TIMEOUT {
            return Err(TorError::BootstrapTimeout {
                progress,
                summary,
                warning,
            });
        }
        tokio::time::sleep(POLL).await;
    }

    let ports = control.socks_ports().await.map_err(TorError::Failed)?;
    ports
        .into_iter()
        .find(|p| Some(*p) != external)
        .ok_or_else(|| TorError::Failed("tor opened no SOCKS port".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_sets_are_normalized() {
        let set = ExitSet::parse("DE, nl de").unwrap();
        assert_eq!(set.codes(), ["de", "nl"]);
        assert_eq!(set.key(), "de-nl");
        assert_eq!(set.to_string(), "de-nl");
        assert_eq!(ExitSet::parse(" ;de;; ").unwrap().key(), "de");
        let any = ExitSet::parse("").unwrap();
        assert!(any.is_any());
        assert_eq!(any.key(), "any");
        assert_eq!(any, ExitSet::any());
        assert_eq!(
            ExitSet::from_codes(["NL", "de"]).unwrap(),
            ExitSet::parse("de,nl").unwrap()
        );
        for bad in ["deu", "d", "1e", "de,xx1", "дe"] {
            let err = ExitSet::parse(bad).unwrap_err();
            assert!(matches!(err, TorError::InvalidExit(_)), "{bad}");
            assert!(err.to_string().starts_with("tor_invalid_country"), "{err}");
        }
    }

    #[test]
    fn the_log_keeps_the_last_lines() {
        let mut log = LogBuffer::new(3);
        for i in 0..5 {
            log.push(format!("line {i}"));
        }
        assert_eq!(log.lines(), ["line 2", "line 3", "line 4"]);
        log.push("Oct 05 [warn] Could not bind to 127.0.0.1:9150".into());
        log.push("Oct 05 [notice] Closing".into());
        assert_eq!(
            log.last_problem().as_deref(),
            Some("Oct 05 [warn] Could not bind to 127.0.0.1:9150")
        );
        let mut none = LogBuffer::new(0);
        none.push("x".into());
        assert!(none.lines().is_empty());
    }

    #[test]
    fn errors_carry_their_codes() {
        assert_eq!(
            TorError::NotInstalled.to_string(),
            "tor_not_installed: Tor is not installed"
        );
        assert_eq!(
            TorError::BootstrapTimeout {
                progress: 10,
                summary: "Connected to a relay".into(),
                warning: Some("Connection refused".into())
            }
            .to_string(),
            "tor_bootstrap_timeout: tor did not connect in 180 s, it stopped at 10 % (Connected to a relay): Connection refused"
        );
        assert!(TorError::InUse(2).to_string().starts_with("tor_in_use: 2"));
        assert!(TorError::TooManyInstances(8).to_string().starts_with("tor_too_many_instances"));
        let app: veydan_core::AppError = TorError::Stopped.into();
        assert!(app.to_string().starts_with("tor_stopped"));
    }

    fn manager(dir: &Path) -> (TorManager, Arc<Mutex<Vec<Vec<InstanceInfo>>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let manager = TorManager::new(dir.to_path_buf(), TorSettings::defaults(), move |list| {
            sink.lock().unwrap().push(list)
        });
        (manager, events)
    }

    #[tokio::test]
    async fn without_a_bundle_tor_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let (manager, events) = manager(dir.path());
        let err = manager.acquire(&ExitSet::any(), "test").await.unwrap_err();
        assert_eq!(err, TorError::NotInstalled);
        // The failed instance is listed with its error, and held by nobody.
        let list = manager.instances();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, InstanceState::Failed);
        assert_eq!(list[0].consumers, 0);
        assert_eq!(list[0].error.as_deref(), Some("tor_not_installed: Tor is not installed"));
        assert!(!events.lock().unwrap().is_empty());

        assert_eq!(
            manager.acquire(&ExitSet::any(), "").await.unwrap_err(),
            TorError::InvalidConsumer
        );
        assert!(matches!(
            manager.stop("de"),
            Err(TorError::UnknownInstance(_))
        ));
        manager.stop("any").unwrap();
        manager.stop_all().await;
        assert!(manager.instances().is_empty());
        assert_eq!(
            manager.acquire(&ExitSet::any(), "test").await.unwrap_err(),
            TorError::Stopped
        );
    }

    /// A tor that is a script: it never opens a control port. Enough to see
    /// leases counted, released on drop, and the limit of instances.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn leases_are_counted_and_released_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bundle = Bundle::at(&source::bundle_dir(dir.path()));
        std::fs::create_dir_all(bundle.tor.parent().unwrap()).unwrap();
        std::fs::write(&bundle.tor, "#!/bin/sh\necho '[notice] fake tor'\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&bundle.tor, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (manager, _) = manager(dir.path());

        let hold = manager
            .inner
            .hold(&ExitSet::parse("de").unwrap(), Hand::Lease)
            .await
            .unwrap()
            .unwrap();
        let second = manager
            .inner
            .hold(&ExitSet::parse("DE").unwrap(), Hand::Lease)
            .await
            .unwrap()
            .unwrap();
        let list = manager.instances();
        assert_eq!(list.len(), 1, "one instance for one set");
        assert_eq!(list[0].key, "de");
        assert_eq!(list[0].consumers, 2);
        assert_eq!(list[0].state, InstanceState::Starting);
        drop(second);
        assert_eq!(manager.instances()[0].consumers, 1);

        // A lease released by its drop, clones counting once.
        let lease = Lease {
            socks_port: 1,
            username: "a".into(),
            password: random_password(),
            key: "de".into(),
            _hold: Arc::new(hold),
        };
        let shown = format!("{lease:?}");
        assert!(!shown.contains(&lease.password), "{shown}");
        let clone = lease.clone();
        drop(lease);
        assert_eq!(manager.instances()[0].consumers, 1);
        drop(clone);
        assert_eq!(manager.instances()[0].consumers, 0);

        // The limit.
        for code in ["aa", "bb", "cc", "dd", "ee", "ff", "gg"] {
            manager.start(&ExitSet::parse(code).unwrap()).await.unwrap();
        }
        assert_eq!(manager.instances().len(), MAX_INSTANCES);
        assert_eq!(
            manager.start(&ExitSet::parse("hh").unwrap()).await.unwrap_err(),
            TorError::TooManyInstances(MAX_INSTANCES)
        );
        // Kept by hand: listed so; a stop of one nobody holds takes it away.
        let info = manager.instances().into_iter().find(|i| i.key == "aa").unwrap();
        assert!(info.kept);
        manager.stop("aa").unwrap();
        manager.stop_all().await;
        assert!(manager.instances().is_empty());
    }

    /// Commands of other crates await these.
    #[test]
    fn the_futures_are_send() {
        fn send<T: Send>(_: &T) {}
        let manager = TorManager::new(PathBuf::new(), TorSettings::defaults(), |_| {});
        let exit = ExitSet::any();
        send(&manager.acquire(&exit, "x"));
        send(&manager.start(&exit));
        send(&manager.new_identity("any"));
        send(&manager.apply_settings(TorSettings::defaults()));
        send(&manager.stop_all());
        send(&manager.release_bundle());
    }

    #[test]
    fn passwords_differ() {
        let a = random_password();
        assert_eq!(a.len(), 32);
        assert_ne!(a, random_password());
    }

    #[test]
    fn the_json_of_an_instance() {
        let info = InstanceInfo {
            key: "de-nl".into(),
            exit: vec!["de".into(), "nl".into()],
            state: InstanceState::Ready,
            bootstrap: 100,
            summary: "Done".into(),
            socks_port: Some(41234),
            consumers: 2,
            kept: false,
            restart_needed: true,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            serde_json::json!({
                "key": "de-nl", "exit": ["de", "nl"], "state": "ready", "bootstrap": 100,
                "summary": "Done", "socks_port": 41234, "consumers": 2, "kept": false,
                "restart_needed": true, "error": null
            })
        );
    }
}

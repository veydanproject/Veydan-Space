// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Which way a connection goes: directly, or through a bridge.
//!
//! The answer depends on the host alone. Through a bridge go the servers
//! of the project, and only while bridges are in use; everything else (a
//! page being previewed, somebody's own relay, an avatar) goes as it
//! always did. A hub would refuse those anyway.
//!
//! The rule is read at every connection, so it may change while clients
//! built long ago are still in use.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};

use tokio::net::TcpListener;

use crate::client::{Client, Error};
use crate::proto::{BridgeRef, H2Stream, Target};
use crate::socks::{self, Login, Opener};

/// What the messenger wants of the network, as of now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetConfig {
    /// Go through bridges. When false everything is direct, whatever else
    /// is set.
    pub active: bool,
    /// The servers of the project, by name.
    pub hosts: BTreeSet<String>,
    /// The bridges to use, the preferred ones first.
    pub bridges: Vec<BridgeRef>,
}

/// Where a request goes when the door is missing: port 0 takes no connections.
const DEAD_END: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Bridge,
}

#[derive(Default)]
struct State {
    active: bool,
    /// Counts the changes that may send a host another way. Whatever is
    /// under way watches it (see [`Net::changes`]).
    generation: u64,
    hosts: Arc<BTreeSet<String>>,
    bridges: Vec<BridgeRef>,
    /// Built from `bridges`; kept across changes that leave them alone, so
    /// that the connection to a bridge survives a switch of the mode.
    client: Option<Client>,
    /// Where the SOCKS door listens, once it was opened.
    door: Option<SocketAddr>,
    /// The task that serves the door; aborting it closes the listener.
    serving: Option<tokio::task::AbortHandle>,
}

struct Inner {
    state: RwLock<State>,
    /// Told the generation every time it grows.
    changed: tokio::sync::watch::Sender<u64>,
    /// What a caller of the SOCKS door must show. Made for this process:
    /// no other program on the machine can use the door.
    login: Login,
}

#[derive(Clone)]
pub struct Net {
    inner: Arc<Inner>,
}

fn random_hex() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 16];
    // Without randomness the door must not open with a guessable login:
    // the caller of `new` gets a panic, as it would for any broken system.
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("the system gives random bytes");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Default for Net {
    fn default() -> Self {
        Self::new()
    }
}

impl Net {
    /// A rule of its own; everything is direct until it is configured.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                state: RwLock::new(State::default()),
                changed: tokio::sync::watch::Sender::new(0),
                login: Login { user: random_hex(), password: random_hex() },
            }),
        }
    }

    /// The rule of this process. HTTP clients and relay connections are
    /// built in many places; they all ask here.
    pub fn global() -> &'static Net {
        static GLOBAL: OnceLock<Net> = OnceLock::new();
        GLOBAL.get_or_init(Net::new)
    }

    /// Puts `config` in force. The first time bridges are given, the SOCKS
    /// door is opened; it stays open until [`Net::close`] and refuses what
    /// the rule does not send to a bridge.
    ///
    /// The door is opened before the rule may send anything to it: there is
    /// no moment in which a host is to go through a bridge and the way to
    /// the bridge is not there yet. Should the door not open at all, the
    /// rule is put in force all the same and the error returned: what is to
    /// go through a bridge then fails, and nothing goes directly instead.
    pub async fn configure(&self, config: NetConfig) -> std::io::Result<()> {
        let hosts: BTreeSet<String> = config.hosts.iter().map(|h| h.to_ascii_lowercase()).collect();
        let needs_door = !config.bridges.is_empty() && self.inner.state.read().expect("net state").door.is_none();
        let opened = if needs_door {
            match TcpListener::bind(("127.0.0.1", 0)).await.and_then(|l| Ok((l.local_addr()?, l))) {
                Ok(pair) => Some(pair),
                Err(e) => {
                    self.put(config.active, hosts, config.bridges);
                    tracing::error!(error = %e, "the SOCKS door did not open: what goes through a bridge fails");
                    return Err(e);
                }
            }
        } else {
            None
        };
        let serve = {
            let mut state = self.inner.state.write().expect("net state");
            // Two calls at once: the first door stays, the other is dropped.
            let serve = match opened {
                Some((addr, listener)) if state.door.is_none() => {
                    state.door = Some(addr);
                    Some(listener)
                }
                _ => None,
            };
            drop(state);
            self.put(config.active, hosts, config.bridges);
            serve
        };
        if let Some(listener) = serve {
            let opener: Arc<dyn Opener> = Arc::new(self.clone());
            let login = Some(self.inner.login.clone());
            let task = tokio::spawn(async move {
                if let Err(e) = socks::serve(listener, opener, login).await {
                    tracing::error!(error = %e, "the SOCKS door closed");
                }
            });
            self.inner.state.write().expect("net state").serving = Some(task.abort_handle());
        }
        Ok(())
    }

    /// The messenger stopped: the SOCKS door closes and the connection to a
    /// bridge goes. The rule stays as it was, so nothing that is to go
    /// through a bridge goes directly meanwhile: it finds no door and
    /// fails. The next [`Net::configure`] with bridges opens a door again.
    /// Idempotent.
    pub fn close(&self) {
        let serving = {
            let mut state = self.inner.state.write().expect("net state");
            state.door = None;
            // A client of the same bridges, not yet connected: the old one
            // takes its connection with it.
            if state.client.is_some() {
                state.client = Some(Client::new(state.bridges.clone()));
            }
            state.serving.take()
        };
        if let Some(serving) = serving {
            serving.abort();
        }
    }

    /// The rule itself, in one step. A change that may send a host another
    /// way is announced to whatever is under way.
    fn put(&self, active: bool, hosts: BTreeSet<String>, bridges: Vec<BridgeRef>) {
        let mut state = self.inner.state.write().expect("net state");
        let moved = state.active != active || *state.hosts != hosts || state.bridges != bridges;
        state.active = active;
        state.hosts = Arc::new(hosts);
        if state.bridges != bridges {
            state.client = (!bridges.is_empty()).then(|| Client::new(bridges.clone()));
            state.bridges = bridges;
        }
        if moved {
            state.generation += 1;
            self.inner.changed.send_replace(state.generation);
        }
    }

    /// Grows every time a host may have to take another way. Whatever is
    /// under way the old way watches it, and starts again.
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    /// Which way a connection to `host` goes.
    pub fn route(&self, host: &str) -> Route {
        let state = self.inner.state.read().expect("net state");
        if state.active && state.client.is_some() && state.hosts.contains(&host.to_ascii_lowercase()) {
            Route::Bridge
        } else {
            Route::Direct
        }
    }

    pub fn is_active(&self) -> bool {
        let state = self.inner.state.read().expect("net state");
        state.active && state.client.is_some()
    }

    /// The client of the bridges that are known, in use or not: a bridge
    /// can be tried before it is relied on.
    pub fn client(&self) -> Option<Client> {
        self.inner.state.read().expect("net state").client.clone()
    }

    /// The bridge in use, when there is a live connection to one.
    pub async fn current(&self) -> Option<BridgeRef> {
        match self.client() {
            Some(client) => client.current().await,
            None => None,
        }
    }

    /// A stream to `host:port` through a bridge. Refused for a host the
    /// rule sends directly: whoever asks here has asked [`Net::route`] first.
    pub async fn open(&self, host: &str, port: u16) -> Result<H2Stream, Error> {
        let target = Target::new(host, port).map_err(|_| Error::NoBridges)?;
        self.open_target(&target).await
    }

    async fn open_target(&self, target: &Target) -> Result<H2Stream, Error> {
        if self.route(&target.host) != Route::Bridge {
            return Err(Error::Refused(target.clone()));
        }
        let client = self.client().ok_or(Error::NoBridges)?;
        client.open(target).await
    }

    /// For an HTTP client: the proxy to reach `host` through, when it goes
    /// through a bridge. `socks5h`: the name travels to the door, and no
    /// resolver of this machine is asked.
    pub fn socks_url(&self, host: &str) -> Option<String> {
        if self.route(host) != Route::Bridge {
            return None;
        }
        // No door (it did not open): a way that leads nowhere, so that the
        // request fails. `None` here would send it directly.
        let door = self.inner.state.read().expect("net state").door.unwrap_or(DEAD_END);
        let Login { user, password } = &self.inner.login;
        Some(format!("socks5h://{user}:{password}@{door}"))
    }
}

/// The SOCKS door asks the rule, not a bridge: a host that goes directly
/// is refused at the door, whoever asks.
impl Opener for Net {
    fn open<'a>(&'a self, target: &'a Target) -> Pin<Box<dyn Future<Output = Result<H2Stream, Error>> + Send + 'a>> {
        Box::pin(self.open_target(target))
    }
}

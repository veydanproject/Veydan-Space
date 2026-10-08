// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The registry of volunteers' call nodes: the last set of servers.
//!
//! The registry (the same one the bridges report to) gives
//! a signed list of its active nodes, the least loaded first (`GET
//! <registry>/v1/calls`; `messenger_vlink::call_list`). The list is
//! checked as the list of bridges is: the root key built into every
//! client (`trust::ROOT_PUB`) delegated the key that signed it, nothing
//! ran out. It is kept as it came (`call.registry.list`) and checked
//! again every time it is read; a list that stops holding (it ran out,
//! the delegation ran out) is no set.
//!
//! The list is asked for again when it is older than [`REFRESH_SECS`] or
//! ran out — **in the background**: whoever reads the sets gets what is
//! cached now and the registry is asked meanwhile, so a call never waits
//! for the registry. The host asks for it at the start and in its
//! housekeeping too (`refresh`, `ensure_fresh`).
//!
//! The fetch itself (HTTPS with the roots of the web, through a bridge
//! when bridges are on) is the host's: this crate takes no HTTP client
//! (scripts/boundaries.sh). The host gives a [`ListFetch`]; the tests a
//! fake registry.

use crate::servers::NodeRef;
use messenger_core::{MessengerError, Result};
use messenger_store::{settings, Store};
use messenger_vlink::call_list::{self, CallList};
use messenger_vlink::proto::list::SignedList;
use messenger_vlink::trust;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// `GET` of one URL: the body as text when the answer was 200. The host
/// builds it on its HTTP client.
pub type ListFetch = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

/// The last list the registry gave, as it came: signed, checked again
/// every time it is read.
pub const KEY_REGISTRY_LIST: &str = "call.registry.list";
/// When the registry was last asked and answered, unix seconds.
pub const KEY_REGISTRY_CHECKED: &str = "call.registry.checked_at";
/// A list older than this is asked for again (as the list of bridges).
pub const REFRESH_SECS: i64 = 6 * 60 * 60;
/// After a try that failed, the next is this much later: a registry
/// that is down is not hammered by every call.
pub const RETRY_SECS: u64 = 15 * 60;
/// The path of the list under a registry's URL.
pub const PATH_CALLS: &str = "/v1/calls";

/// One node of the registry's list, as the sets see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryNode {
    pub node: NodeRef,
    pub turn_port: u16,
    /// 0: no SFU.
    pub sfu_port: u16,
    pub region: String,
    pub caps: Vec<String>,
    /// `active`, or `degraded` (the registry's last check of it failed):
    /// a degraded node is used only when no active one is listed.
    pub state: String,
    /// How full the registry listed it, percent.
    pub load: u8,
}

impl RegistryNode {
    pub fn is_active(&self) -> bool {
        self.state == call_list::STATE_ACTIVE
    }
}

pub struct Registry {
    store: Store,
    fetch: ListFetch,
    registries: Vec<String>,
    root: String,
    refreshing: AtomicBool,
    last_failed: Mutex<Option<Instant>>,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Registry {
    /// The registries and the root built into every client.
    pub fn new(store: Store, fetch: ListFetch) -> Self {
        Self::with_trust(store, fetch, trust::REGISTRIES.iter().map(|r| r.to_string()).collect(), trust::ROOT_PUB.to_string())
    }

    /// Registries and a root of one's own: the tests.
    pub fn with_trust(store: Store, fetch: ListFetch, registries: Vec<String>, root: String) -> Self {
        Self { store, fetch, registries, root, refreshing: AtomicBool::new(false), last_failed: Mutex::new(None) }
    }

    /// The nodes of the cached list, while its signatures hold and it
    /// has not run out. A list that is due is asked for in the
    /// background; what is cached is returned now.
    pub async fn nodes(self: &Arc<Self>) -> Result<Vec<RegistryNode>> {
        let cached = self.cached().await?;
        if self.is_stale().await? {
            self.refresh_in_background();
        }
        Ok(cached)
    }

    /// The nodes of the cached list, checked again; nothing is asked.
    pub async fn cached(&self) -> Result<Vec<RegistryNode>> {
        let Some(text) = settings::get(&self.store, KEY_REGISTRY_LIST).await? else { return Ok(Vec::new()) };
        Ok(self.read(&text).map(nodes_of).unwrap_or_default())
    }

    fn read(&self, text: &str) -> Result<CallList> {
        let signed: SignedList = serde_json::from_str(text)?;
        call_list::verify(&signed, &self.root, now().max(0) as u64).map_err(|e| MessengerError::Invalid(format!("the list of call nodes: {e}")))
    }

    pub async fn checked_at(&self) -> Result<Option<i64>> {
        Ok(settings::get(&self.store, KEY_REGISTRY_CHECKED).await?.and_then(|s| s.parse().ok()))
    }

    /// Whether the list is due: never asked, asked long ago, or what was
    /// kept no longer holds.
    pub async fn is_stale(&self) -> Result<bool> {
        let Some(at) = self.checked_at().await? else { return Ok(true) };
        if now() - at >= REFRESH_SECS {
            return Ok(true);
        }
        Ok(match settings::get(&self.store, KEY_REGISTRY_LIST).await? {
            Some(text) => self.read(&text).is_err(),
            None => true,
        })
    }

    /// Asks the registries for the list, the first that answers with a
    /// list that holds; keeps it. How many nodes it named.
    pub async fn refresh(&self) -> Result<usize> {
        let mut last = MessengerError::Transport("no registry is known".into());
        for registry in &self.registries {
            let asked = async {
                let text = (self.fetch)(format!("{registry}{PATH_CALLS}")).await?;
                let list = self.read(&text)?;
                Ok::<_, MessengerError>((text, list.nodes.len()))
            };
            match asked.await {
                Ok((text, count)) => {
                    settings::set(&self.store, KEY_REGISTRY_LIST, &text).await?;
                    settings::set(&self.store, KEY_REGISTRY_CHECKED, &now().to_string()).await?;
                    *self.last_failed.lock().unwrap() = None;
                    tracing::info!(count, "call nodes: the registry's list was refreshed");
                    return Ok(count);
                }
                Err(e) => last = e,
            }
        }
        *self.last_failed.lock().unwrap() = Some(Instant::now());
        Err(last)
    }

    /// `refresh` when the list is due, waited for: at the start of the
    /// session and in the housekeeping of the host.
    pub async fn ensure_fresh(&self) -> Result<()> {
        if self.is_stale().await? {
            self.refresh().await?;
        }
        Ok(())
    }

    /// `refresh` on a task of its own, one at a time, and not within
    /// [`RETRY_SECS`] of a try that failed. Nothing without a runtime.
    pub fn refresh_in_background(self: &Arc<Self>) {
        if self.last_failed.lock().unwrap().is_some_and(|t| t.elapsed().as_secs() < RETRY_SECS) {
            return;
        }
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.refreshing.store(false, Ordering::SeqCst);
            return;
        };
        let me = self.clone();
        handle.spawn(async move {
            if let Err(e) = me.refresh().await {
                tracing::debug!(error = %e, "call nodes: the registry's list was not refreshed");
            }
            me.refreshing.store(false, Ordering::SeqCst);
        });
    }

    /// Waits for a refresh under way to end (the tests).
    pub async fn settle(&self) {
        while self.refreshing.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    }
}

/// The nodes of a list, in its order (the least loaded first, the
/// degraded last). A node of another class than a volunteer's is left
/// out: the registry lists volunteers, and a client of today knows no
/// other class there.
fn nodes_of(list: CallList) -> Vec<RegistryNode> {
    let version = list.v;
    list.nodes
        .into_iter()
        .filter(|n| n.class == call_list::CLASS_VOLUNTEER)
        .map(|n| RegistryNode {
            state: n.state().to_string(),
            load: n.load_percent(version),
            node: n.node,
            turn_port: n.turn_port,
            sfu_port: n.sfu_port,
            region: n.region,
            caps: n.caps,
        })
        .collect()
}

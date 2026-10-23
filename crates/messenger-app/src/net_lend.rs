// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The messenger's network lent to another module of the app that speaks
//! it as the same person: the channels of communities
//! (internal/communities-stage1.md §6). Three seams: the relays as
//! `messenger_core::Transport`, the shape both sides know, the identity at
//! work (`NetworkSession`), and what the user chose of the borrower's rows
//! of the list of chats (`ChatPrefs`). The borrower sends through the pool of the
//! runtime that runs now; the events of its own subscriptions reach it by
//! the routes it registers with messenger-transport, never through here.

use crate::MessengerState;
use async_trait::async_trait;
use messenger_core::traits::RelayStatusSnapshot;
use messenger_core::{Ack, MessengerError, Outbound, RawEvent, Transport};
use std::sync::{Arc, Mutex, Weak};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;
use veydan_shell::seams::{self, code, ChatPref, ChatPrefs, CloudError, MediaKeys, NetworkSession, SeamFuture, SessionKey};
use zeroize::Zeroizing;

/// What a request answers while the messenger does not run.
const STOPPED: &str = "messenger_stopped";

pub(crate) fn offer(app: &AppHandle) {
    let lend = Arc::new(NetLend { app: app.clone(), seen: Mutex::new((Weak::<Unreachable>::new(), 0)) });
    let transport: Arc<dyn Transport> = lend.clone();
    seams::provide(app, transport);
    let session: Arc<dyn NetworkSession> = lend.clone();
    seams::provide(app, session);
    let prefs: Arc<dyn ChatPrefs> = lend.clone();
    seams::provide(app, prefs);
    let keys: Arc<dyn MediaKeys> = lend;
    seams::provide(app, keys);
}

/// Nothing: the type of an empty `Weak` before the first pool is seen.
struct Unreachable;

#[async_trait]
impl Transport for Unreachable {
    async fn send(&self, _out: Outbound) -> messenger_core::Result<Ack> {
        Err(MessengerError::Transport(STOPPED.into()))
    }

    fn events(&self) -> mpsc::Receiver<RawEvent> {
        mpsc::channel(1).1
    }

    async fn status(&self) -> RelayStatusSnapshot {
        RelayStatusSnapshot { relays: Vec::new() }
    }
}

struct NetLend {
    app: AppHandle,
    /// The pool seen last and the generation it was given. The weak
    /// reference keeps the place of that pool from being taken by the
    /// next one, so a new pool is never mistaken for the old.
    seen: Mutex<(Weak<dyn Transport>, u64)>,
}

impl NetLend {
    fn runtime(&self) -> Option<Arc<messenger_runtime::MessengerRuntime>> {
        self.app.try_state::<MessengerState>()?.runtime().ok()
    }

    /// The generation of `pool`: the same while the runtime keeps it, the
    /// next one once it made another.
    fn generation(&self, pool: &Arc<dyn Transport>) -> u64 {
        let mut seen = self.seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !Weak::ptr_eq(&seen.0, &Arc::downgrade(pool)) {
            seen.0 = Arc::downgrade(pool);
            seen.1 += 1;
        }
        seen.1
    }
}

#[async_trait]
impl Transport for NetLend {
    async fn send(&self, out: Outbound) -> messenger_core::Result<Ack> {
        let rt = self.runtime().ok_or_else(|| MessengerError::Transport(STOPPED.into()))?;
        let pool = rt.relays().pool().await;
        pool.send(out).await
    }

    /// The borrower's events come by the routes of messenger-transport:
    /// this stream is closed from the start.
    fn events(&self) -> mpsc::Receiver<RawEvent> {
        mpsc::channel(1).1
    }

    async fn status(&self) -> RelayStatusSnapshot {
        match self.runtime() {
            Some(rt) => rt.relays().pool().await.status().await,
            None => RelayStatusSnapshot { relays: Vec::new() },
        }
    }
}

impl NetworkSession for NetLend {
    fn session(&self) -> SeamFuture<'_, Option<SessionKey>> {
        Box::pin(async move {
            let Some(rt) = self.runtime() else { return Ok(None) };
            // No session while the servers are not chosen or the lock is
            // closed: nothing would be heard, and nothing may be sent.
            if rt.session_pubkey().await.is_none() {
                return Ok(None);
            }
            let keys = match rt.identity().load_keys().await {
                Ok(keys) => keys,
                Err(MessengerError::NotLoggedIn | MessengerError::SecretsLocked) => return Ok(None),
                Err(e) => return Err(CloudError::new(code::STORAGE, e.to_string())),
            };
            let mut secret = Zeroizing::new([0u8; 32]);
            secret.copy_from_slice(keys.secret_key().as_secret_bytes());
            let pool: Arc<dyn Transport> = rt.relays().pool().await;
            Ok(Some(SessionKey { secret, generation: self.generation(&pool) }))
        })
    }
}

impl ChatPrefs for NetLend {
    fn prefs<'a>(&'a self, prefix: &'a str) -> SeamFuture<'a, Vec<ChatPref>> {
        Box::pin(async move {
            let Some(rt) = self.runtime() else { return Ok(Vec::new()) };
            let rows = messenger_store::chats::prefs(rt.store()).await.map_err(|e| CloudError::new(code::STORAGE, e.to_string()))?;
            Ok(rows
                .into_iter()
                .filter(|r| r.chat_id.starts_with(prefix))
                .map(|r| ChatPref { pinned: r.pinned, muted: r.muted, archived: r.archived, chat_id: r.chat_id })
                .collect())
        })
    }
}

/// The app keys of the cards of private servers (`access_key` of a blossom
/// media server), for the writes of the channels of communities to them
/// (internal/communities-wire.md §9.1).
impl MediaKeys for NetLend {
    fn app_keys(&self) -> SeamFuture<'_, Vec<(String, Zeroizing<String>)>> {
        Box::pin(async move {
            let Some(rt) = self.runtime() else { return Ok(Vec::new()) };
            let rows = rt.media_servers().await.map_err(|e| CloudError::new(code::STORAGE, e.to_string()))?;
            Ok(rows
                .into_iter()
                .filter(|r| r.kind != "s3")
                .filter_map(|r| r.access_key.filter(|k| !k.trim().is_empty()).map(|k| (r.url, Zeroizing::new(k))))
                .collect())
        })
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whose profiles and inbox relays this device follows (`SUB_PROFILES`,
//! `SUB_DM_RELAYS`), and the loop that keeps that list up to date while a
//! session runs.
//!
//! The runtime subscribes at a start and after what it does itself (a
//! contact added, a chat opened). What comes from the relays changes the
//! list too: a contact my other device approved (`contacts.updated` from
//! the book mirror), the chats history sync brings (`history.synced`), a
//! chat a stranger starts (`dm.message`). Those are gathered until they
//! settle, and one REQ follows, only if the list changed: a history sync
//! of hundreds of events costs one.

use crate::relays::RelayService;
use messenger_contacts::{ContactService, UI_EVENT_CONTACTS_UPDATED};
use messenger_core::traits::UiEvent;
use messenger_core::{Outbound, PubKey, Result, Scope, SubId, Transport};
use messenger_dm::{DmPeer, DmService, UI_EVENT_DM_MESSAGE};
use messenger_ingress::filters;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::{self, error::RecvError};

/// Quiet this long after the last reason, then subscribe.
const SETTLE: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(2) };

/// How a followed peer is kept in `MetaFollow::followed`: its hex for the
/// home set, `<hex>@<server>` for a private server's.
fn followed_on(hex: &str, server: Option<&str>) -> String {
    match server {
        Some(id) => format!("{hex}@{id}"),
        None => hex.to_string(),
    }
}

/// The private servers a set of `followed` asks.
fn servers_of(followed: &BTreeSet<String>) -> BTreeSet<String> {
    followed.iter().filter_map(|k| k.split_once('@').map(|(_, s)| s.to_string())).collect()
}

#[derive(Clone)]
pub struct MetaFollow {
    contacts: ContactService,
    dm: DmService,
    relays: Arc<RelayService>,
    /// The authors of the last `SUB_PROFILES` sent.
    followed: Arc<std::sync::Mutex<BTreeSet<String>>>,
    /// One subscribe at a time, so the last one sent is the newest list.
    turn: Arc<tokio::sync::Mutex<()>>,
    /// How many times the subscriptions were sent (for the tests).
    sent: Arc<AtomicU64>,
}

impl MetaFollow {
    pub(crate) fn new(contacts: ContactService, dm: DmService, relays: Arc<RelayService>) -> Self {
        Self {
            contacts,
            dm,
            relays,
            followed: Arc::default(),
            turn: Arc::default(),
            sent: Arc::default(),
        }
    }

    /// Everyone we care about on the home set: contacts, me, then the peers
    /// of home chats up to a cap. The address book is per device, so a
    /// peer who is a contact only on my other device is still followed
    /// here through the chat. A peer of a private chat only is not: asking
    /// the home set for them would tell it whom I talk to on the server.
    pub(crate) async fn peers(&self, me: &PubKey) -> Result<Vec<PubKey>> {
        let mut peers: Vec<PubKey> = self
            .contacts
            .list()
            .await?
            .into_iter()
            .filter_map(|c| PubKey::parse(&c.pubkey))
            .collect();
        peers.push(me.clone());
        for c in self.dm.list_chats(true).await? {
            if peers.len() >= crate::groups::MAX_PROFILE_AUTHORS {
                break;
            }
            if c.server_id.is_some() {
                continue;
            }
            if let Some(pk) = c.peer_pubkey.as_deref().and_then(PubKey::parse) {
                if !peers.contains(&pk) {
                    peers.push(pk);
                }
            }
        }
        Ok(peers)
    }

    /// The peers of private chats, by their server, up to the cap each:
    /// their profiles and inbox lists are asked of that server alone.
    pub(crate) async fn private_peers(&self) -> Result<BTreeMap<String, Vec<PubKey>>> {
        let mut out: BTreeMap<String, Vec<PubKey>> = BTreeMap::new();
        for c in self.dm.list_chats(true).await? {
            let (Some(server), Some(pk)) = (c.server_id.clone(), c.peer_pubkey.as_deref().and_then(PubKey::parse)) else { continue };
            let peers = out.entry(server).or_default();
            if peers.len() < crate::groups::MAX_PROFILE_AUTHORS && !peers.contains(&pk) {
                peers.push(pk);
            }
        }
        Ok(out)
    }

    /// Profiles and inbox relays of `peers`, and my follow list, on the home
    /// set; the profiles and inbox relays of the peers of private chats on
    /// each of their servers. Unless `force`, nothing is sent when the lists
    /// are the ones sent last. Returns whether they were sent.
    pub(crate) async fn resubscribe(&self, me: &PubKey, force: bool) -> Result<bool> {
        let _turn = self.turn.lock().await;
        let peers = self.peers(me).await?;
        let private = self.private_peers().await?;
        let mut set: BTreeSet<String> = peers.iter().map(|p| p.as_hex().to_string()).collect();
        for (server, peers) in &private {
            set.extend(peers.iter().map(|p| followed_on(p.as_hex(), Some(server))));
        }
        let before = self.followed.lock().unwrap().clone();
        if !force && before == set {
            return Ok(false);
        }
        let pool = self.relays.pool().await;
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_PROFILES.into()),
            filter: filters::profiles(&peers),
            scope: Scope::Own,
        })
        .await?;
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_MY_FOLLOWS.into()),
            filter: filters::my_follows(me),
            scope: Scope::Own,
        })
        .await?;
        // Where contacts and chat peers want their DMs delivered.
        pool.send(Outbound::Subscribe {
            id: SubId(filters::SUB_DM_RELAYS.into()),
            filter: filters::dm_relays(&peers),
            scope: Scope::Own,
        })
        .await?;
        for (server, peers) in &private {
            let scope = crate::scopes::of_server(Some(server));
            // A server whose relay is not connected now gets the REQ when
            // it is (nostr-sdk keeps it); one that fails keeps the others.
            for (base, filter) in [(filters::SUB_PROFILES, filters::profiles(peers)), (filters::SUB_DM_RELAYS, filters::dm_relays(peers))] {
                let id = SubId(crate::scopes::sub_id(base, Some(server)));
                if let Err(e) = pool.send(Outbound::Subscribe { id, filter, scope: scope.clone() }).await {
                    eprintln!("messenger: the profiles of a private server's chats not asked: {e}");
                }
            }
        }
        // A server none of whose chats is left is asked nothing more.
        for gone in servers_of(&before).difference(&private.keys().cloned().collect()) {
            for base in [filters::SUB_PROFILES, filters::SUB_DM_RELAYS] {
                let _ = pool.send(Outbound::Unsubscribe { id: SubId(crate::scopes::sub_id(base, Some(gone))) }).await;
            }
        }
        *self.followed.lock().unwrap() = set;
        self.sent.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }

    fn follows(&self, key: &str) -> bool {
        self.followed.lock().unwrap().contains(key)
    }

    /// Whether `ev` may change whom to follow. `SUB_PROFILES` asks every
    /// author with no `since`, so a new REQ brings the newest profile of
    /// one just added, whatever the cache had.
    fn may_change(&self, ev: &UiEvent) -> bool {
        match ev.name.as_str() {
            crate::session::UI_EVENT_HISTORY_SYNCED => true,
            UI_EVENT_CONTACTS_UPDATED => ev.payload["pubkey"].as_str().is_none_or(|pk| !self.follows(pk)),
            // History is covered by `history.synced`.
            UI_EVENT_DM_MESSAGE if ev.payload["historical"] != true => ev.payload["chat_id"]
                .as_str()
                .and_then(DmPeer::of_chat)
                .is_some_and(|c| !self.follows(&followed_on(c.key.as_hex(), c.server()))),
            _ => false,
        }
    }

    /// While the session runs: a reason, then quiet for `SETTLE`, then one
    /// subscribe if the list changed.
    pub(crate) async fn follow_loop(self, me: PubKey, mut ui: broadcast::Receiver<UiEvent>) {
        loop {
            match ui.recv().await {
                Ok(ev) if !self.may_change(&ev) => continue,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
            let mut deadline = tokio::time::Instant::now() + SETTLE;
            loop {
                match tokio::time::timeout_at(deadline, ui.recv()).await {
                    Err(_) => break,
                    Ok(Ok(ev)) if !self.may_change(&ev) => {}
                    Ok(Ok(_)) | Ok(Err(RecvError::Lagged(_))) => deadline = tokio::time::Instant::now() + SETTLE,
                    Ok(Err(RecvError::Closed)) => return,
                }
            }
            if let Err(e) = self.resubscribe(&me, false).await {
                eprintln!("messenger: profiles not subscribed again: {e}");
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn sent(&self) -> u64 {
        self.sent.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn followed(&self) -> BTreeSet<String> {
        self.followed.lock().unwrap().clone()
    }
}

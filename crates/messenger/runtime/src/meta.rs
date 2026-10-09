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
use messenger_dm::{DmService, UI_EVENT_DM_MESSAGE};
use messenger_ingress::filters;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::{self, error::RecvError};

/// Quiet this long after the last reason, then subscribe.
const SETTLE: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(2) };

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

    /// Everyone we care about: contacts, me, then chat peers up to a cap.
    /// The address book is per device, so a peer who is a contact only on
    /// my other device is still followed here through the chat.
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
            if let Some(pk) = c.peer_pubkey.as_deref().and_then(PubKey::parse) {
                if !peers.contains(&pk) {
                    peers.push(pk);
                }
            }
        }
        Ok(peers)
    }

    /// Profiles and inbox relays of `peers`, and my follow list. Unless
    /// `force`, nothing is sent when the list is the one sent last. Returns
    /// whether it was sent.
    pub(crate) async fn resubscribe(&self, me: &PubKey, force: bool) -> Result<bool> {
        let _turn = self.turn.lock().await;
        let peers = self.peers(me).await?;
        let set: BTreeSet<String> = peers.iter().map(|p| p.as_hex().to_string()).collect();
        if !force && *self.followed.lock().unwrap() == set {
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
        *self.followed.lock().unwrap() = set;
        self.sent.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }

    fn follows(&self, hex: &str) -> bool {
        self.followed.lock().unwrap().contains(hex)
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
                .and_then(|c| c.strip_prefix("dm:"))
                .is_some_and(|pk| !self.follows(pk)),
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

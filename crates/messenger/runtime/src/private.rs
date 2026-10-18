// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A private server put in by hand (internal/messenger-wire.md, «Серверы»,
//! «Частный чат»): its row, its relay and its blob server, each of them the
//! server's alone. What a server card does (`card`, stage Э4) with what it
//! reads, after the user said yes; the testing commands and the tests call
//! it too. Removed the same way,
//! and both reach my other devices in the snapshot of my relations
//! («Серверы на всех устройствах»), where a running session listens to a
//! server that came and stops one that went (`Listening`).

use crate::{scopes, MessengerRuntime, RelayService};
use messenger_core::traits::UiEvent;
use messenger_core::{
    MessengerError, Outbound, PubKey, RelayUrl, Result, SubId, Timestamp, Transport,
};
pub use messenger_dm::snapshot::UI_EVENT_SERVERS_UPDATED;
use messenger_ingress::filters;
use messenger_store::{servers, settings, Store};
use messenger_transport::RelayPool;
use std::collections::HashMap;
use tokio::sync::broadcast;

/// What a private server is made of, as a card names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivateServerInput {
    /// The relay, `wss://…`: the server is named by it (`servers::id_of`).
    pub relay_url: String,
    /// The app key the relay and the blob server want.
    pub key: Option<String>,
    /// The blob server, `https://…`.
    pub media_url: Option<String>,
    /// The call node, `address:port#id`.
    pub call_node: Option<String>,
    pub name: Option<String>,
    /// How its phones are woken: `veydan` (through the push of the Veydan
    /// network) or `off`, the default.
    pub push_mode: Option<String>,
}

impl MessengerRuntime {
    /// Put in (or update) the private server of `input.relay_url`: the
    /// server row, its relay (connected at once), its blob server (written
    /// to for its private chats only, with the app key) — the one function
    /// the UI and a server card call to add a server. A running session
    /// listens to it at once (its inbox, with the history it holds), its
    /// inbox list and my profile are published there, and the snapshot of
    /// my relations is owed, so every device of mine gets it
    /// (internal/messenger-wire.md, «Серверы на всех устройствах»). The
    /// server's id.
    pub async fn server_put_private(&self, input: &PrivateServerInput) -> Result<String> {
        if RelayUrl::parse(input.relay_url.trim()).is_none() {
            return Err(MessengerError::Invalid(
                "relay url must be ws:// or wss:// with a host".into(),
            ));
        }
        // A relay of my home set stays home (as a card and the snapshot
        // leave it): one relay is one server.
        if servers::by_relay_url(self.store(), &input.relay_url)
            .await?
            .is_some_and(|s| !s.is_private())
        {
            return Err(MessengerError::Invalid(format!(
                "{}: {} is a relay of the home set",
                servers::ERR_SERVER_IS_HOME,
                input.relay_url.trim()
            )));
        }
        let id = servers::put_private(
            self.store(),
            &servers::ServerRow {
                id: String::new(),
                kind: servers::KIND_PRIVATE.into(),
                name: input.name.clone(),
                relay_url: input.relay_url.clone(),
                relay_key: input.key.clone(),
                media_url: input.media_url.clone(),
                call_node: input.call_node.clone(),
                push_mode: input.push_mode.clone().unwrap_or_else(|| servers::PUSH_OFF.into()),
                source: Some(servers::SOURCE_HAND.into()),
                added_at: 0,
            },
        )
        .await?;
        self.relays.reload().await?;
        if self.session_keys().await.is_ok() {
            if let Err(e) = self.publish_dm_relays(false).await {
                eprintln!("messenger: the inbox relay list of a private server not published: {e}");
            }
            self.profile_to_private_servers().await;
        }
        self.dm.mark_relations_owed().await?;
        // Its groups are heard at once: a group here whose relay it is
        // catches up its past there (`GroupsDriver::servers_changed`).
        self.group_driver.servers_changed().await;
        self.servers_told();
        Ok(id)
    }

    /// Remove private server `id` here and on every device of mine: the
    /// server, its relay and blob server, its private chats with all their
    /// messages, and its groups, forgotten as a group I left is, with
    /// nothing published (`servers::remove_by_me`); the removal goes in
    /// the snapshot of my relations, newer than when the server was added,
    /// so it takes the server and its groups wherever they are. The
    /// transfers of their chats end first, and a call of one of them (a
    /// group call too, `calls::end_calls_of`); the running session stops
    /// listening to the server (`dm-live@<id>`, `groups@<id>`), and the
    /// keys of its groups leave the secret store. The function the UI
    /// calls to remove a server; `server_unknown` for an id that is no
    /// private server of this device.
    pub async fn server_remove_private(&self, id: &str) -> Result<()> {
        let now = messenger_core::Clock::now(&messenger_core::traits::SystemClock).secs();
        if !self
            .servers_private()
            .await?
            .iter()
            .any(|s| s.id == id)
        {
            return Err(MessengerError::Invalid(servers::ERR_SERVER_UNKNOWN.into()));
        }
        let going = servers::lives_on(self.store(), id).await?;
        for chat in going.chats.iter().cloned().chain(going.groups.iter().map(|g| messenger_store::groups::group_chat_id(g))) {
            self.cancel_transfers_of_chat(&chat).await;
        }
        // And their calls: the end goes to the peer while the server is here.
        crate::calls::end_calls_of(self.calls(), self.group_calls(), &going.chats, &going.groups).await;
        if servers::remove_by_me(self.store(), id, now)
            .await?
            .is_none()
        {
            return Err(MessengerError::Invalid(servers::ERR_SERVER_UNKNOWN.into()));
        }
        self.relays.reload().await?;
        self.dm.mark_relations_owed().await?;
        self.group_driver.servers_changed().await;
        self.servers_told();
        let _ = self.ui.send(UiEvent {
            name: messenger_dm::UI_EVENT_CHATS_UPDATED.into(),
            payload: serde_json::json!({}),
        });
        Ok(())
    }

    /// The page and the running session are told the list of my servers
    /// changed: the session listens to what came and stops what went
    /// (`Listening`).
    fn servers_told(&self) {
        let _ = self.ui.send(UiEvent {
            name: UI_EVENT_SERVERS_UPDATED.into(),
            payload: serde_json::json!({}),
        });
    }

    /// The private servers this device has.
    pub async fn servers_private(&self) -> Result<Vec<servers::ServerRow>> {
        servers::private(self.store()).await
    }

    /// `publish_profile_to_private_servers`, its failure told and not
    /// returned: what was done at home stands.
    pub(crate) async fn profile_to_private_servers(&self) {
        if let Err(e) = self.publish_profile_to_private_servers().await {
            eprintln!("messenger: my profile not published on the private servers: {e}");
        }
    }

    /// My kind 0 on every private server, so the peers of my private chats
    /// there see who I am without asking the home set. Its picture is my
    /// avatar put on that server's blob server (with its app key); a server
    /// without one, or an avatar this device does not hold, gets the kind 0
    /// without a picture: the home address would send its peers' devices
    /// to my home servers. At the start of a session and after my profile
    /// or my avatar changed. A server that fails keeps the others going.
    pub async fn publish_profile_to_private_servers(&self) -> Result<()> {
        let keys = self.session_keys().await?;
        let ids = crate::scopes::private_servers(self.store()).await;
        if ids.is_empty() {
            return Ok(());
        }
        let me = messenger_core::PubKey::parse(&keys.public_key().to_hex()).expect("valid pubkey");
        let Some(profile) = self.profiles().get(&me).await? else {
            return Ok(());
        };
        let picture = profile.picture.clone();
        // Once per kind 0 and server: a start of a session sends nothing
        // the server has.
        let fingerprint = format!(
            "{}|{}|{}",
            me.as_hex(),
            profile.event_created_at,
            picture.as_deref().unwrap_or("")
        );
        let avatar = match picture.as_deref() {
            Some(url) => self.avatars.local_bytes(url).await,
            None => None,
        };
        let mut queued = false;
        for id in ids {
            let sent_key = crate::scopes::sub_id("profile.published", Some(&id));
            if settings::get(self.store(), &sent_key).await?.as_deref()
                == Some(fingerprint.as_str())
            {
                continue;
            }
            let url = match &avatar {
                Some(bytes) => match self
                    .media
                    .upload_public_of(&keys, Some(&id), bytes.clone(), "image/jpeg", 1)
                    .await
                {
                    Ok(blobs) => blobs.into_iter().next().map(|b| b.url),
                    Err(e) => {
                        eprintln!("messenger: my avatar not put on a private server: {e}");
                        None
                    }
                },
                None => None,
            };
            let picture = match url.as_deref() {
                Some(u) => messenger_contacts::Picture::Set(u),
                None => messenger_contacts::Picture::Remove,
            };
            let Some(event) = self.profiles().build_copy(&keys, picture).await? else {
                return Ok(());
            };
            self.outbox
                .enqueue(messenger_core::Outbound::PublishScoped {
                    scope: crate::scopes::of_server(Some(&id)),
                    event,
                })
                .await?;
            settings::set(self.store(), &sent_key, &fingerprint).await?;
            queued = true;
        }
        if queued {
            self.outbox.kick();
        }
        Ok(())
    }
}

/// The private servers a session listens to: the inbox of each under its
/// own subscription (`dm-live@<id>`). A server put in by hand or taken from
/// the snapshot of my relations while the session runs is listened to at
/// once, and one removed is let go, without a restart: the session follows
/// `UI_EVENT_SERVERS_UPDATED` (`crate::relations::open_inbox`).
pub(crate) struct Listening {
    me: PubKey,
    /// Where the live inbox starts, as for the home set.
    since: Timestamp,
    /// The servers listened to, each with when it was added: one removed
    /// and put in again before the session looked is another server
    /// (its history forgotten here, `servers::remove_private`), listened
    /// to anew.
    heard: HashMap<String, i64>,
}

impl Listening {
    pub(crate) fn new(me: PubKey, since: Timestamp) -> Self {
        Self {
            me,
            since,
            heard: HashMap::new(),
        }
    }

    /// The private servers as they stand now, and the pool after them: the
    /// inbox of each new one subscribed, the one of each gone closed. The
    /// servers newly listened to, whose history the caller catches up
    /// (`session::history_catch_up`). A server whose relay the pool does
    /// not hold yet (disabled) is tried again at the next change. The
    /// groups of the servers follow the same event in the group driver
    /// (`GroupsDriver::servers_changed`).
    pub(crate) async fn follow(
        &mut self,
        store: &Store,
        relays: &RelayService,
        pool: &RelayPool,
        ui: &broadcast::Sender<UiEvent>,
    ) -> Vec<String> {
        let now: HashMap<String, i64> = match servers::private(store).await {
            Ok(rows) => rows.into_iter().map(|s| (s.id, s.added_at)).collect(),
            Err(e) => {
                eprintln!("messenger servers: the private servers were not read: {e}");
                return Vec::new();
            }
        };
        let mut came: Vec<String> = now
            .iter()
            .filter(|(id, added)| self.heard.get(*id) != Some(added))
            .map(|(id, _)| id.clone())
            .collect();
        came.sort();
        // Gone, or gone and put in again: its old subscription closed.
        let gone: Vec<String> = self
            .heard
            .iter()
            .filter(|(id, added)| now.get(*id) != Some(added))
            .map(|(id, _)| id.clone())
            .collect();
        if came.is_empty() && gone.is_empty() {
            return Vec::new();
        }
        // The relays of the servers that came were put in the store, by
        // hand or by the snapshot: the pool takes them, and lets the gone
        // ones go.
        if let Err(e) = relays.reload().await {
            eprintln!("messenger servers: the relays were not reloaded: {e}");
        }
        for id in gone {
            let sub = SubId(scopes::sub_id(filters::SUB_DM_LIVE, Some(&id)));
            let _ = pool.send(Outbound::Unsubscribe { id: sub }).await;
            self.heard.remove(&id);
        }
        let mut listened = Vec::new();
        for id in came {
            let out = Outbound::Subscribe {
                id: SubId(scopes::sub_id(filters::SUB_DM_LIVE, Some(&id))),
                filter: filters::dm_inbox(&self.me, self.since),
                scope: scopes::of_server(Some(&id)),
            };
            match pool.send(out).await {
                Ok(_) => {
                    self.heard.insert(id.clone(), now[&id]);
                    listened.push(id);
                }
                Err(e) => {
                    let _ = ui.send(UiEvent {
                        name: "error".into(),
                        payload: serde_json::json!({ "family": "inbox", "error": e.to_string() }),
                    });
                }
            }
        }
        listened
    }
}

pub mod card;

#[cfg(test)]
mod tests;

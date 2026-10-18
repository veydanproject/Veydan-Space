// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! My private servers and the relations of their chats in the snapshot of
//! my relations (internal/messenger-wire.md, «Серверы на всех
//! устройствах»): what one device of mine adds or removes, and whom it
//! blocks or approves in a private chat, reaches the others.
//!
//! - `servers`: every private server of mine (`servers::private`), the
//!   first `SERVERS_MAX` added, its app key with it (the snapshot is NIP-44
//!   to me alone). A device takes a server it does not have, unless it
//!   remembers a removal of it not older than the server's `added_at`; a
//!   server it has stays as it is. Absence is no word: an older build
//!   writes no `servers`, and a server left out removes nothing. The home
//!   set is each device's own choice and never travels.
//! - `servers_removed`: `[id, removed_at]`, the newest `SERVERS_REMOVED_MAX`.
//!   A removal newer than the `added_at` of the server here takes it and
//!   everything of it, its groups too; one of a server this device does
//!   not have is remembered, so an older word of a device that still has
//!   it does not bring it back.
//! - `private_peers`: `[hex, server_id, my_contact, blocked, decided_at]`,
//!   as `peers` (the same letters, the same order, the newest decision
//!   first), of the private chats of the servers in the list. A list of
//!   its own: an older build reads `peers` as the home chats, and would
//!   take a block on a server for a block at home. Paged like `peers`,
//!   after them (`super::paginate`): the header carries what fits its
//!   share, the pages the rest, each under `private_peers` of its own.
//!
//! Every entry is read on its own (`serde_json::Value`): one this build
//! cannot read is skipped, and costs nothing else.

use super::{contact_of, letter, Header};
use crate::peer::DmPeer;
use crate::service::DmService;
use messenger_core::{Effect, PubKey, Result};
use messenger_store::dm_relations::RelationRow;
use messenger_store::servers::{self, ServerRow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// The UI is told the list of my servers changed: one added or removed
/// here, or taken from another device of mine. A signal only.
pub const UI_EVENT_SERVERS_UPDATED: &str = "servers.updated";

/// How many private servers the snapshot carries: far more than one
/// person runs, and few enough (a few hundred bytes each) to leave the
/// header its room.
pub const SERVERS_MAX: usize = 32;
/// How many removals of servers it carries, the newest first.
pub const SERVERS_REMOVED_MAX: usize = 64;
/// How many relations of private chats it carries, on the header and the
/// pages after the home ones; what does not fit goes by the copies of
/// their signals on their server, as before.
pub const PRIVATE_PEERS_MAX: usize = 300;
/// A field of a server longer than this is no address nor key of ours:
/// such a server is neither written nor read.
const FIELD_MAX: usize = 512;

/// A private server as the snapshot carries it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ServerEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub relay_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_node: Option<String>,
    #[serde(default = "push_off")]
    pub push_mode: String,
    /// The push server of the server (internal/messenger-wire.md, «Push
    /// через шлюз»), `https://…`; what it says of its gateway each device
    /// asks itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_url: Option<String>,
    pub added_at: i64,
}

fn push_off() -> String {
    servers::PUSH_OFF.into()
}

impl ServerEntry {
    fn of_row(r: &ServerRow, push_url: Option<String>) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            relay_url: r.relay_url.clone(),
            relay_key: r.relay_key.clone(),
            media_url: r.media_url.clone(),
            call_node: r.call_node.clone(),
            push_mode: r.push_mode.clone(),
            push_url,
            added_at: r.added_at,
        }
    }

    /// Every field within bounds, the id the one of the relay (a row can
    /// never name another server's relay), a mode this build knows.
    fn sound(&self) -> bool {
        let short = |v: &Option<String>| v.as_deref().is_none_or(|s| s.len() <= FIELD_MAX);
        self.relay_url.len() <= FIELD_MAX
            && [
                &self.name,
                &self.relay_key,
                &self.media_url,
                &self.call_node,
                &self.push_url,
            ]
            .into_iter()
            .all(short)
            && self.push_url.as_deref().is_none_or(|u| u.starts_with("https://"))
            && servers::is_id(&self.id)
            && servers::id_of(&self.relay_url) == self.id
            && messenger_core::RelayUrl::parse(&self.relay_url).is_some()
            && (self.push_mode == servers::PUSH_OFF || self.push_mode == servers::PUSH_VEYDAN)
            && self.added_at > 0
    }

    fn row(&self) -> ServerRow {
        ServerRow {
            id: self.id.clone(),
            kind: servers::KIND_PRIVATE.into(),
            name: self.name.clone(),
            relay_url: self.relay_url.clone(),
            relay_key: self.relay_key.clone(),
            media_url: self.media_url.clone(),
            call_node: self.call_node.clone(),
            push_mode: self.push_mode.clone(),
            source: Some(servers::SOURCE_SNAPSHOT.into()),
            added_at: self.added_at,
        }
    }
}

/// `[hex, server_id, my_contact, blocked, decided_at]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PrivateEntry(pub String, pub String, pub String, pub u8, pub i64);

/// What this device writes: its private servers, the first `SERVERS_MAX`
/// added; its removals, the newest `SERVERS_REMOVED_MAX`.
pub(super) async fn entries(
    store: &messenger_store::Store,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let mut list = Vec::new();
    for r in servers::private(store).await? {
        let push_url = servers::push_of(store, &r.id).await?.and_then(|p| p.push_url);
        let e = ServerEntry::of_row(&r, push_url);
        if e.sound() {
            list.push(serde_json::to_value(&e)?);
        }
        if list.len() == SERVERS_MAX {
            break;
        }
    }
    let mut removed = servers::removed(store).await?;
    removed.truncate(SERVERS_REMOVED_MAX);
    Ok((
        list,
        removed
            .into_iter()
            .map(|(id, at)| serde_json::json!([id, at]))
            .collect(),
    ))
}

/// The relations of private chats, in the order of `peers`.
pub(super) fn private_entries(rows: &[RelationRow], me: &str) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = super::ordered(rows, me)
        .into_iter()
        .filter_map(|(r, m)| {
            serde_json::to_value(PrivateEntry(
                r.peer_pubkey.clone(),
                r.server_id.clone(),
                letter(m).into(),
                u8::from(r.blocked),
                r.decided_at,
            ))
            .ok()
        })
        .collect();
    out.truncate(PRIVATE_PEERS_MAX);
    out
}

/// What applying the servers of a header did.
#[derive(Default)]
pub(super) struct ServersApplied {
    /// A server was taken or removed here.
    pub changed: bool,
    /// A chat or a group went with a removed server.
    pub chats_gone: bool,
    /// The header is of a build that writes no servers while this device
    /// has some to tell: the list on the relays is behind it.
    pub owed: bool,
}

impl DmService {
    /// The removals first, then the servers, of the header `h`.
    pub(super) async fn apply_server_entries(&self, h: &Header) -> Result<ServersApplied> {
        let mut out = ServersApplied::default();
        for v in &h.servers_removed {
            let Ok((id, removed_at)) = serde_json::from_value::<(String, i64)>(v.clone()) else {
                continue;
            };
            if !servers::is_id(&id) || removed_at <= 0 {
                continue;
            }
            match servers::get(&self.store, &id).await? {
                // Put in here after that removal: it stays, and this
                // device's list says so.
                Some(row) if row.is_private() && row.added_at > removed_at => continue,
                Some(row) if row.is_private() => {
                    if let Some(removed) = servers::remove_private(&self.store, &id).await? {
                        out.chats_gone |= !removed.is_empty();
                        out.changed = true;
                    }
                }
                // A relay of my home set here: not a private server to take.
                Some(_) => continue,
                None => {}
            }
            servers::remember_removed(&self.store, &id, removed_at).await?;
        }
        match &h.servers {
            Some(list) => {
                for v in list {
                    let Ok(e) = serde_json::from_value::<ServerEntry>(v.clone()) else {
                        continue;
                    };
                    if !e.sound() || servers::get(&self.store, &e.id).await?.is_some() {
                        continue;
                    }
                    if servers::removed_at(&self.store, &e.id).await? >= e.added_at {
                        continue;
                    }
                    let id = servers::put_private(&self.store, &e.row()).await?;
                    if e.push_url.is_some() && e.push_mode == servers::PUSH_VEYDAN {
                        servers::set_push(&self.store, &id, e.push_url.as_deref()).await?;
                    }
                    out.changed = true;
                }
            }
            None => {
                out.owed = !servers::private(&self.store).await?.is_empty()
                    || !servers::removed(&self.store).await?.is_empty();
            }
        }
        Ok(out)
    }

    /// The relations of private chats of a header, each as a copy of my
    /// decision (`apply_decision_copy`), on the servers this device has.
    pub(super) async fn apply_private_entries(
        &self,
        me: &str,
        entries: &[serde_json::Value],
    ) -> Result<(Vec<PubKey>, Vec<Effect>)> {
        let (mut changed, mut effects) = (Vec::new(), Vec::new());
        let mut done: HashSet<(String, String)> = HashSet::new();
        let mut mine: HashSet<String> = HashSet::new();
        for s in servers::private(&self.store).await? {
            mine.insert(s.id);
        }
        for v in entries {
            let Ok(PrivateEntry(hex, server, m, blocked, decided_at)) =
                serde_json::from_value::<PrivateEntry>(v.clone())
            else {
                continue;
            };
            if !mine.contains(&server) || hex == me || !done.insert((hex.clone(), server.clone())) {
                continue;
            }
            let (Some(peer), Some(my_contact)) = (PubKey::parse(&hex), contact_of(&m)) else {
                continue;
            };
            if let Some(fx) = self
                .apply_decision_copy(
                    &DmPeer::on(peer.clone(), Some(&server)),
                    my_contact,
                    blocked != 0,
                    decided_at,
                )
                .await?
            {
                changed.push(peer);
                effects.extend(fx);
            }
        }
        Ok((changed, effects))
    }
}

#[cfg(test)]
mod tests;

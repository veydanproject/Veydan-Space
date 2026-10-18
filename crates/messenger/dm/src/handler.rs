// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Inbound consumers owned by the DM module.

use crate::service::DmService;
use async_trait::async_trait;
use messenger_core::{Context, DmInbound, Effect, Handler, MetaInbound, RelayUrl, Result};
use messenger_store::{dm_routes, servers, Store};

pub use crate::service::{UI_EVENT_CHATS_UPDATED, UI_EVENT_DM_MESSAGE, UI_EVENT_DM_UPDATED};

/// `Inbound::Dm` → chats and messages.
pub struct DmHandler {
    service: DmService,
}

impl DmHandler {
    pub fn new(service: DmService) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Handler<DmInbound> for DmHandler {
    async fn handle(&self, msg: DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        self.service.apply_inbound(msg, ctx).await
    }
}

/// Inbox relay lists (kind 10050) and relay lists (kind 10002) →
/// `msg_dm_routes`. Everything else in the meta family is someone else's.
pub struct DmRoutesHandler {
    store: Store,
}

impl DmRoutesHandler {
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

/// Keep well-formed relay urls only, a handful at most.
fn clean(urls: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls {
        if let Some(p) = RelayUrl::parse(&u) {
            let s = p.as_str().to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
        if out.len() == 5 {
            break;
        }
    }
    out
}

impl DmRoutesHandler {
    /// The private server whose relay a list came by; `None` for the home
    /// set and for a relay of nobody's server. A list a peer keeps on a
    /// private server names that server's relays: it is kept apart, and
    /// home messages are never sent by it. `None` from the relay of a
    /// private server removed on purpose: its list is kept nowhere.
    async fn server_of(&self, via: Option<&RelayUrl>) -> Result<Option<Option<String>>> {
        let Some(url) = via else { return Ok(Some(None)) };
        match servers::private_of_relay(&self.store, url.as_str()).await {
            Ok(server) => Ok(Some(server)),
            Err(e) if servers::is_unknown(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl Handler<MetaInbound> for DmRoutesHandler {
    async fn handle(&self, msg: MetaInbound, _ctx: &Context) -> Result<Vec<Effect>> {
        match msg {
            MetaInbound::DmRelays { author, created_at, relays, via } => {
                let Some(server) = self.server_of(via.as_ref()).await? else { return Ok(vec![]) };
                let relays = clean(relays.into_iter());
                dm_routes::replace(&self.store, author.as_hex(), server.as_deref(), dm_routes::SOURCE_NIP17, created_at.secs(), &relays)
                    .await?;
            }
            MetaInbound::RelayList { author, created_at, relays, via } => {
                let Some(server) = self.server_of(via.as_ref()).await? else { return Ok(vec![]) };
                // Relays the author reads from: no marker, or "read".
                let readable = clean(relays.into_iter().filter(|(_, m)| m.as_deref() != Some("write")).map(|(u, _)| u));
                dm_routes::replace(&self.store, author.as_hex(), server.as_deref(), dm_routes::SOURCE_NIP65, created_at.secs(), &readable)
                    .await?;
            }
            _ => {}
        }
        Ok(vec![])
    }
}

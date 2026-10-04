// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Inbound consumers owned by the DM module.

use crate::service::DmService;
use async_trait::async_trait;
use messenger_core::{Context, DmInbound, Effect, Handler, MetaInbound, RelayUrl, Result};
use messenger_store::{dm_routes, Store};

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

#[async_trait]
impl Handler<MetaInbound> for DmRoutesHandler {
    async fn handle(&self, msg: MetaInbound, _ctx: &Context) -> Result<Vec<Effect>> {
        match msg {
            MetaInbound::DmRelays { author, created_at, relays } => {
                dm_routes::replace(&self.store, author.as_hex(), dm_routes::SOURCE_NIP17, created_at.secs(), &clean(relays.into_iter()))
                    .await?;
            }
            MetaInbound::RelayList { author, created_at, relays } => {
                // Relays the author reads from: no marker, or "read".
                let readable = relays.into_iter().filter(|(_, m)| m.as_deref() != Some("write")).map(|(u, _)| u);
                dm_routes::replace(&self.store, author.as_hex(), dm_routes::SOURCE_NIP65, created_at.secs(), &clean(readable))
                    .await?;
            }
            _ => {}
        }
        Ok(vec![])
    }
}

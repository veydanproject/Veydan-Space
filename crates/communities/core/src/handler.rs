// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from crates/messenger/groups/src/handler.rs at 1f2ef31d, then changed for channels.

//! Inbound consumers owned by the channels module, for the adapter
//! (sub-stage 1.4 wires them into the runtime).
//!
//! Two doors: the raw events of the channels' relays (the journal,
//! epochs, content, cards, and gift wraps to a keeper kept here), and the
//! direct messages the messenger already opened, which carry the notes of
//! channels to me; the second stands in front of the chat handler and
//! lets everything else through.

use crate::notes::Opened;
use crate::service::{ChannelService, Outcome};
use async_trait::async_trait;
use messenger_core::{Context, DmInbound, Effect, EventSource, Handler, RawEvent, Result};
use std::sync::Arc;
use tokio::sync::mpsc;

/// What a handler cannot do itself and asks its host for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The set of channels to listen to changed.
    Resubscribe,
    /// Run the keepers' cycle (`ChannelService::tick`) at this moment.
    Wake(u64),
    Note(String),
}

pub type Signals = mpsc::UnboundedSender<Signal>;

/// Events for the UI become effects; the rest is signalled to the host.
/// What goes out was sent by the service itself (`Transport`).
pub fn effects(outcome: Outcome, signals: &Signals) -> Vec<Effect> {
    let out: Vec<Effect> = outcome.events.into_iter().map(Effect::Emit).collect();
    if outcome.resubscribe {
        let _ = signals.send(Signal::Resubscribe);
    }
    if let Some(at) = outcome.wake {
        let _ = signals.send(Signal::Wake(at));
    }
    for n in outcome.notes {
        let _ = signals.send(Signal::Note(n));
    }
    out
}

/// The community a raw event belongs to, by its `h` (or `d` of a card).
fn community_of(event: &RawEvent) -> Option<String> {
    let tags = event.json["tags"].as_array()?;
    tags.iter().find(|t| t[0] == "h" || t[0] == "d").and_then(|t| t[1].as_str()).map(str::to_string)
}

/// Raw events of the channels → journal, keys, content.
pub struct ChannelHandler {
    service: ChannelService,
    signals: Signals,
}

impl ChannelHandler {
    pub fn new(service: ChannelService, signals: Signals) -> Self {
        Self { service, signals }
    }
}

#[async_trait]
impl Handler<RawEvent> for ChannelHandler {
    async fn handle(&self, msg: RawEvent, _ctx: &Context) -> Result<Vec<Effect>> {
        let Some(keys) = self.service.signer() else { return Ok(vec![]) };
        let live = !matches!(msg.source, EventSource::Sync { .. });
        // One channel at a time: the journal is rebuilt on every
        // operation. A wrap names no channel outside: it takes no lock.
        let outcome = match community_of(&msg) {
            Some(cid) => {
                let lock = self.service.lock_of(&cid).await;
                let _guard = lock.lock().await;
                self.service.on_event(&keys, &msg.json, live).await?
            }
            None => self.service.on_event(&keys, &msg.json, live).await?,
        };
        Ok(effects(outcome, &self.signals))
    }
}

/// `Inbound::Dm` → the notes of channels, or on to the chats.
pub struct ChannelDmHandler {
    service: ChannelService,
    signals: Signals,
    next: Arc<dyn Handler<DmInbound>>,
}

impl ChannelDmHandler {
    pub fn new(service: ChannelService, signals: Signals, next: Arc<dyn Handler<DmInbound>>) -> Self {
        Self { service, signals, next }
    }
}

#[async_trait]
impl Handler<DmInbound> for ChannelDmHandler {
    async fn handle(&self, msg: DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        if let Some(keys) = self.service.signer() {
            if let Some(cid) = crate::inbound::channel_of_note(&msg.content) {
                let lock = self.service.lock_of(&cid).await;
                let _guard = lock.lock().await;
                let opened = Opened { id: msg.rumor_id.as_hex().to_string(), sender: msg.sender.clone(), kind: msg.rumor_kind, content: msg.content.clone(), created_at: msg.created_at.secs() };
                let outcome = self.service.on_note(&keys, &opened, None).await?;
                return Ok(effects(outcome, &self.signals));
            }
        }
        self.next.handle(msg, ctx).await
    }
}

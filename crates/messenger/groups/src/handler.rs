// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Inbound consumers owned by the groups module.
//!
//! Two doors: events of the groups themselves, and the direct messages
//! that carry invitations, requests, welcomes and keys. The second one
//! stands in front of the chat handler and lets ordinary messages through.

use crate::service::{GroupService, Outcome};
use async_trait::async_trait;
use messenger_core::{Context, DmInbound, Effect, GroupInbound, Handler, Result};
use std::sync::Arc;
use tokio::sync::mpsc;

/// What a handler cannot do itself and asks its host for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The set of groups to listen to changed.
    Resubscribe,
    /// The key of this group needs a manager, and I am one.
    Maintain(String),
    Note(String),
}

pub type Signals = mpsc::UnboundedSender<Signal>;

/// Publishing, events and notifications become effects; the rest is
/// signalled to the host.
pub fn effects(outcome: Outcome, signals: &Signals) -> Vec<Effect> {
    let mut out: Vec<Effect> = Vec::new();
    out.extend(outcome.publish.into_iter().map(Effect::Send));
    out.extend(outcome.events.into_iter().map(Effect::Emit));
    out.extend(outcome.notify.into_iter().map(Effect::Notify));
    if outcome.resubscribe {
        let _ = signals.send(Signal::Resubscribe);
    }
    for g in outcome.maintain {
        let _ = signals.send(Signal::Maintain(g));
    }
    for n in outcome.notes {
        let _ = signals.send(Signal::Note(n));
    }
    out
}

/// `Inbound::Group` → log, keys, messages.
pub struct GroupHandler {
    service: GroupService,
    signals: Signals,
}

impl GroupHandler {
    pub fn new(service: GroupService, signals: Signals) -> Self {
        Self { service, signals }
    }
}

#[async_trait]
impl Handler<GroupInbound> for GroupHandler {
    async fn handle(&self, msg: GroupInbound, ctx: &Context) -> Result<Vec<Effect>> {
        let Some(keys) = self.service.signer() else { return Ok(vec![]) };
        // One group at a time: the log is rebuilt on every operation.
        let lock = self.service.lock_of(&msg.group_id).await;
        let _guard = lock.lock().await;
        let outcome = self.service.on_event(&keys, msg, ctx).await?;
        Ok(effects(outcome, &self.signals))
    }
}

/// `Inbound::Dm` → the group protocol, or on to the chats.
pub struct GroupDmHandler {
    service: GroupService,
    signals: Signals,
    next: Arc<dyn Handler<DmInbound>>,
}

impl GroupDmHandler {
    pub fn new(service: GroupService, signals: Signals, next: Arc<dyn Handler<DmInbound>>) -> Self {
        Self { service, signals, next }
    }
}

#[async_trait]
impl Handler<DmInbound> for GroupDmHandler {
    async fn handle(&self, msg: DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        if let Some(keys) = self.service.signer() {
            if let Some(group_id) = crate::inbound::group_of_dm(&msg.content) {
                let lock = self.service.lock_of(&group_id).await;
                let _guard = lock.lock().await;
                if let Some(outcome) = self.service.on_dm(&keys, &msg, ctx).await? {
                    return Ok(effects(outcome, &self.signals));
                }
            }
        }
        self.next.handle(msg, ctx).await
    }
}

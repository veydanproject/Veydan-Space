// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The door of the calls in the chain of DM handlers: `call.*` envelopes,
//! whether a message (the invitation) or a note (the rest), go to the
//! call service and nowhere else; everything else goes on to the next
//! handler. Stands in front of `DmHandler`, as `GroupDmHandler` does:
//!
//! ```text
//! GroupDmHandler → CallDmHandler → DmHandler
//! ```
//!
//! The service answers through its outlet, so this handler returns no
//! effects of its own.

use crate::service::CallService;
use async_trait::async_trait;
use messenger_core::{Context, DmInbound, Effect, Envelope, Handler, Result};
use std::sync::Arc;

pub struct CallDmHandler {
    service: CallService,
    next: Arc<dyn Handler<DmInbound>>,
}

impl CallDmHandler {
    pub fn new(service: CallService, next: Arc<dyn Handler<DmInbound>>) -> Self {
        Self { service, next }
    }
}

#[async_trait]
impl Handler<DmInbound> for CallDmHandler {
    async fn handle(&self, msg: DmInbound, ctx: &Context) -> Result<Vec<Effect>> {
        if let Ok(envelope) = Envelope::parse(&msg.content) {
            if envelope.is_call() {
                self.service.on_dm(&msg, &envelope, ctx).await?;
                return Ok(vec![]);
            }
        }
        self.next.handle(msg, ctx).await
    }
}

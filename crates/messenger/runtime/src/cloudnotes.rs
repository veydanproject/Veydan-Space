// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The service notes of the module `cloud` to a contact
//! (internal/messenger-wire.md §3, "Восстановление доступа"): a rumor of
//! `KIND_PEER_NOTE_RUMOR`, quiet, no copy to myself, to a contact we both
//! chose to talk with. The wrap is the one receipts travel in; the outbox
//! carries it, without a term when the note names no `expiration` (a share
//! and a revoke wait for a device that may lie off for weeks), for as long
//! as the expiration says otherwise.

use messenger_core::envelope::Envelope;
use messenger_core::{MessengerError, Outbound, PubKey, Result};
use messenger_dm::wrap::wrap_note;

use crate::MessengerRuntime;

/// The event the DM service hands such a note over with (`cloud.note`).
pub use messenger_dm::notes::UI_EVENT_CLOUD_NOTE;
/// The event the DM service tells of a card of the pinned bot with
/// (`cloud.card`); the card itself is a message of the bot's chat.
pub use messenger_dm::UI_EVENT_BOT_CARD;

impl MessengerRuntime {
    /// `envelope` to `peer` as a service note, sealed by the identity;
    /// `expiration` is the NIP-40 `expiration` of the outer wrap and the
    /// term of the outbox, unix seconds. Refused when `peer` is not a
    /// contact we both chose to talk with (`Invalid("not_approved")`).
    pub async fn send_peer_note(&self, peer: &PubKey, envelope: &Envelope, expiration: Option<i64>) -> Result<()> {
        let keys = self.session_keys().await?;
        if !self.dm().recovery_notes_allowed(peer).await? {
            return Err(MessengerError::Invalid("not_approved".into()));
        }
        let now = self.clock_now();
        let wrapped = wrap_note(&keys, peer, &envelope.encode(), now, false, expiration)?;
        let out = Outbound::PublishToInbox {
            recipient: peer.clone(),
            event: wrapped.to_peer,
            hint_relays: self.dm().hints(peer).await?,
        };
        match expiration {
            Some(at) if at > now => self.outbox().enqueue_for(out, at - now).await?,
            Some(_) => return Err(MessengerError::Invalid("the note has expired already".into())),
            None => self.outbox().enqueue(out).await?,
        };
        self.outbox().kick();
        Ok(())
    }

    fn clock_now(&self) -> i64 {
        messenger_core::traits::Clock::now(&messenger_core::traits::SystemClock).secs()
    }
}

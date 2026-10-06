// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What I tell the people I talk to about me. Each switch works both ways:
//! off, I tell nothing and am shown nothing of the others. Read receipts
//! are a switch of this device; presence is one for all my devices, since
//! they beat from one key (`crate::presence`).

use crate::MessengerRuntime;
use messenger_core::Result;
use messenger_store::settings;
use serde::{Deserialize, Serialize};

pub use messenger_dm::KEY_READ_RECEIPTS;
/// Whether my contacts see when I am online, and I see them
/// (`crate::presence`). My devices carry it between them (`own.presence`).
pub use messenger_dm::KEY_PRESENCE;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PrivacySettings {
    pub read_receipts: bool,
    pub presence: bool,
}

impl MessengerRuntime {
    pub async fn privacy_settings(&self) -> Result<PrivacySettings> {
        Ok(PrivacySettings {
            read_receipts: settings::get_bool(self.store(), KEY_READ_RECEIPTS, true).await?,
            presence: settings::get_bool(self.store(), KEY_PRESENCE, true).await?,
        })
    }

    /// Presence turned off: whoever knew my presence key is told it is
    /// gone, nothing is watched, the key moves on and my other devices turn
    /// it off too. Turned on: the key moves on, my other devices turn it on,
    /// and the presence loop tells the new key at its next tick.
    pub async fn privacy_set(&self, s: PrivacySettings) -> Result<PrivacySettings> {
        let before = self.privacy_settings().await?;
        settings::set_bool(self.store(), KEY_READ_RECEIPTS, s.read_receipts).await?;
        if before.presence != s.presence {
            // Set first: the beats that come and the tick obey it even if
            // the rest fails.
            settings::set_bool(self.store(), KEY_PRESENCE, s.presence).await?;
            if let Err(e) = self.presence_switched(s.presence).await {
                eprintln!("messenger presence: switch not carried out: {e}");
            }
        }
        self.privacy_settings().await
    }
}

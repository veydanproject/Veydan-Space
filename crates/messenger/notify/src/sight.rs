// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Whether another device of mine is in sight now: what a phone woken by
//! a push reads before it rings for a message.
//!
//! Only the store is read: the runtime of the app, while its process
//! lives, hears the beats of my presence key in the subscription it holds
//! anyway and writes down my other device's (`messenger_presence::sight`).
//! No relay is asked when a message comes, so none learns when one came,
//! and none out of reach holds the message up. A process gone, a beat
//! older than a beat promises or presence switched off mean "nobody is in
//! sight", and the phone rings as before.

use messenger_core::presence::KEY_PRESENCE;
use messenger_store::{settings, Store};

/// Whether another device of mine is in sight at `now` (unix seconds).
/// False on any trouble.
pub async fn in_sight(store: &Store, now: i64) -> bool {
    let read = async {
        if !settings::get_bool(store, KEY_PRESENCE, true).await? {
            return Ok(false);
        }
        messenger_presence::sight::elsewhere(store, now).await
    };
    match read.await {
        Ok(seen) => seen,
        Err(e) => {
            eprintln!("messenger notify: presence not read: {e}");
            false
        }
    }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A relay of a test, listening.

use nostr_sdk::local_relay::{LocalRelay, LocalRelayBuilder};

/// A local relay, already listening.
///
/// `LocalRelay` picks a random port by trying it free, lets it go and binds
/// it later, so another test or job on the same host can take it in between
/// (`AddrInUse` on a loaded CI runner): a relay that failed to bind is
/// replaced by a new one, which picks a new port, five times at the most.
pub async fn local_relay() -> LocalRelay {
    local_relay_with(LocalRelay::builder).await
}

/// The same for a relay with policies: `make` builds its builder anew for
/// every try, as a builder is spent by the relay it makes.
pub async fn local_relay_with(make: impl Fn() -> LocalRelayBuilder) -> LocalRelay {
    let mut last = None;
    for _ in 0..5 {
        let relay = make().build();
        match relay.run().await {
            Ok(()) => return relay,
            Err(e) => last = Some(e),
        }
    }
    panic!("no local relay after five ports: {last:?}");
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Messenger network side.
//!
//! - `manifest`: the signed infrastructure manifest (relays, media servers,
//!   further sources, regions) with anti-rollback and region resolution.
//! - `pool`: `RelayPool`, the `core::Transport` implementation over
//!   `nostr-sdk`'s relay pool.
//!
//! This crate knows nothing about message types or storage. It receives
//! `Outbound`, emits `RawEvent`, and reports relay status.

pub mod manifest;
pub mod pool;
pub mod remote;
pub mod ws;

pub use manifest::{Manifest, ManifestMedia, ManifestPush, ManifestRelay, RelayChanges, EMBEDDED_MANIFEST_JSON};
pub use pool::{RelayConfig, RelayPool};
pub use ws::BridgedTransport;
pub use remote::{HttpManifestFetcher, ManifestFetcher, MANIFEST_URLS, PROJECT_MANIFEST_PUBKEY};

/// Make sure the process has a TLS crypto provider. More than one rustls
/// backend can be linked (a host may bring its own); rustls then refuses
/// to pick one. A host that already installed a provider wins, otherwise
/// `ring` is installed. Safe to call any number of times.
pub fn ensure_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

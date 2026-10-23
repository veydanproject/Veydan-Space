// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The writes of a channel to a server of the Veydan-Server kit with
//! `MEDIA_UPLOAD_KEY=required` (internal/communities-wire.md §9.1), through
//! the very code of the app (`MediaHosts`, the Blossom client of the
//! messenger), against a real nginx with the kit's njs and a real
//! blossom-server. Ignored: it needs that server, named by
//!
//!   VEYDAN_KIT_E2E=http://127.0.0.1:18080   VEYDAN_KIT_E2E_APP_KEY=<the app key>
//!   cargo test -p communities-core --test kit_e2e -- --ignored
//!
//! (the kit's media site rendered by a dry run, served by Debian 13's nginx
//! also on plain http, blossom-server beside it).

use communities_core::blobs::{BlobHosts, Gate, MediaHosts};
use communities_core::upload_keys::{label_of, mint, TTL_SECS};
use messenger_media::{BlobBackend, BlossomBackend};
use nostr::key::Keys;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn blob(what: &str) -> (String, Vec<u8>) {
    let bytes = format!("{what} {}", now()).into_bytes();
    (communities_core::blobs::sha256_hex(&bytes), bytes)
}

#[tokio::test]
#[ignore = "needs a server of the kit: VEYDAN_KIT_E2E, VEYDAN_KIT_E2E_APP_KEY"]
async fn the_owner_and_the_staff_write_to_a_server_of_the_kit_and_a_revoked_key_stops() {
    let (Ok(base), Ok(app)) = (std::env::var("VEYDAN_KIT_E2E"), std::env::var("VEYDAN_KIT_E2E_APP_KEY")) else {
        panic!("VEYDAN_KIT_E2E and VEYDAN_KIT_E2E_APP_KEY");
    };
    let hosts = MediaHosts::new().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hosts.gate(&base).await, Some(Gate::Key { scoped: true }), "the kit says it needs a key and takes upload keys");
    let (owner, author, reader) = (Keys::generate(), Keys::generate(), Keys::generate());
    let exists = |sha: String| {
        let base = base.clone();
        async move { BlossomBackend::new(&base, Keys::generate()).unwrap_or_else(|e| panic!("{e}")).exists(&sha).await.unwrap_or(false) }
    };

    // Without a key nothing goes; with the app key of the card it does.
    let (sha, bytes) = blob("owner");
    assert!(hosts.put_keyed(&owner, None, &base, &sha, &bytes).await.is_err(), "no key: refused");
    hosts.put_keyed(&owner, Some(&app), &base, &sha, &bytes).await.unwrap_or_else(|e| panic!("the app key: {e}"));
    assert!(exists(sha.clone()).await, "blossom holds the owner's blob");

    // An upload key the owner's device made for the author, without asking the server.
    let cid = "ab".repeat(32);
    let label = label_of(&cid);
    let t = now();
    let key = mint(&app, &base, &label, &author.public_key().to_hex(), t, t + TTL_SECS);
    let (sha, bytes) = blob("author");
    hosts.put_keyed(&author, Some(&key), &base, &sha, &bytes).await.unwrap_or_else(|e| panic!("the author with his key: {e}"));
    assert!(exists(sha.clone()).await, "blossom holds the author's blob");
    // The chunks of a file go through the engine's backend of the host: the same key.
    let chunk = hosts.chunk_backend_keyed(&author, Some(&key), &base).unwrap_or_else(|| panic!("a backend"));
    let (sha2, bytes2) = blob("chunk");
    chunk.put(&sha2, bytes2).await.unwrap_or_else(|e| panic!("a chunk with the key: {e:?}"));
    hosts.delete_keyed(&author, Some(&key), &base, &sha2).await.unwrap_or_else(|e| panic!("his own blob removed: {e}"));
    assert!(!exists(sha2).await, "gone");

    // Another person with the author's key: refused (the key is bound to his signature).
    let (sha, bytes) = blob("reader");
    assert!(hosts.put_keyed(&reader, Some(&key), &base, &sha, &bytes).await.is_err(), "a reader with the author's key");
    // A key made with another app key, or for another host name: refused.
    let forged = mint("not-the-app-key", &base, &label, &author.public_key().to_hex(), t, t + TTL_SECS);
    assert!(hosts.put_keyed(&author, Some(&forged), &base, &sha, &bytes).await.is_err(), "a forged key");

    // Revoked by the owner's device with the app key: the key stops at once.
    hosts.revoke(&app, &base, &label, &author.public_key().to_hex()).await.unwrap_or_else(|e| panic!("revoke: {e}"));
    let (sha, bytes) = blob("after revoke");
    assert!(hosts.put_keyed(&author, Some(&key), &base, &sha, &bytes).await.is_err(), "the revoked key opens nothing");
    assert!(hosts.revoke(&key, &base, &label, &author.public_key().to_hex()).await.is_err(), "an upload key revokes nothing");
    // A new key, made after the revocation, works again.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let t2 = now() + 1;
    let key2 = mint(&app, &base, &label, &author.public_key().to_hex(), t2, t2 + TTL_SECS);
    hosts.put_keyed(&author, Some(&key2), &base, &sha, &bytes).await.unwrap_or_else(|e| panic!("a key made after the revocation: {e}"));
}

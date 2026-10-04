// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A relay of the project reached through a bridge: the pool connects,
//! publishes and receives, and every byte of it went through the bridge.
//!
//! This file is a process of its own, so it may set the rule of the
//! process, which is the one the pool follows.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use messenger_core::outbound::{Filter, WireEvent};
use messenger_core::traits::RelayState;
use messenger_core::{EventId, Outbound, RelayUrl, Scope, SubId, Transport};
use messenger_transport::{RelayConfig, RelayPool};
use messenger_vlink::testing::{bridge, Behaviour};
use messenger_vlink::{Net, NetConfig};
use nostr_sdk::local_relay::LocalRelay;
use nostr_sdk::prelude::*;

/// A name that resolves nowhere: the only way to it is the bridge.
const RELAY_HOST: &str = "relay.veydan.test";

async fn connected(pool: &RelayPool) -> bool {
    for _ in 0..100 {
        if pool.status().await.relays.iter().all(|r| r.state == RelayState::Connected) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_of_the_project_is_reached_through_the_bridge() {
    let relay = LocalRelay::builder().build();
    relay.run().await.expect("local relay starts");
    let real = relay.url().await;
    let port = url::Url::parse(real.as_str()).unwrap().port().unwrap();

    // The bridge carries whatever host is named to the relay on this machine.
    let fake = bridge(Behaviour::CarryTo(format!("127.0.0.1:{port}").parse().unwrap())).await;
    Net::global()
        .configure(NetConfig {
            active: true,
            hosts: BTreeSet::from([RELAY_HOST.to_string()]),
            bridges: vec![fake.bridge.clone()],
        })
        .await
        .unwrap();

    // The key rides on the address, as with the project's gated relays.
    let url = RelayUrl::parse(&format!("ws://{RELAY_HOST}:{port}")).unwrap();
    let config = || vec![RelayConfig { url: url.clone(), read: true, write: true, api_key: Some("k".repeat(16)) }];

    let alice = Keys::generate();
    let sender = RelayPool::new(Some(alice.clone()));
    sender.set_relays(config()).await.unwrap();
    let receiver = RelayPool::new(None);
    let mut inbox = receiver.events();
    receiver.set_relays(config()).await.unwrap();
    assert!(connected(&sender).await && connected(&receiver).await, "both pools connect through the bridge");
    assert_eq!(fake.streams.load(Ordering::Relaxed), 2);
    assert!(fake.asked.lock().unwrap().iter().all(|a| a == &format!("{RELAY_HOST}:{port}")));

    let filter = Filter(serde_json::json!({ "kinds": [1], "authors": [alice.public_key().to_hex()] }));
    receiver.send(Outbound::Subscribe { id: SubId("t".into()), filter, scope: Scope::Own }).await.unwrap();

    let note = EventBuilder::new(Kind::from(1u16), "through the bridge").finalize(&alice).unwrap();
    let event = WireEvent { id: EventId::parse(&note.id.to_hex()).unwrap(), json: serde_json::to_value(&note).unwrap() };
    let ack = sender.send(Outbound::PublishOwn { event }).await.unwrap();
    assert!(ack.is_delivered(), "the relay takes the note: {ack:?}");

    let got = tokio::time::timeout(Duration::from_secs(5), inbox.recv()).await.expect("the note arrives").unwrap();
    assert_eq!(got.pubkey.as_hex(), alice.public_key().to_hex());

    sender.shutdown().await;
    receiver.shutdown().await;
}

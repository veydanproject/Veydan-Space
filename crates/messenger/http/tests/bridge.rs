// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A client built here goes through a bridge to a server of the project,
//! and directly to anything else.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use messenger_vlink::testing::{bridge, Behaviour};
use messenger_vlink::{Net, NetConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const OURS: &str = "media.veydan.test";

#[tokio::test(flavor = "multi_thread")]
async fn the_projects_server_is_reached_through_the_bridge_and_others_directly() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/blob")).respond_with(ResponseTemplate::new(200).set_body_string("the blob")).mount(&server).await;
    Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(201)).mount(&server).await;

    // The bridge carries whatever host is named to the server above.
    let fake = bridge(Behaviour::CarryTo(*server.address())).await;
    let net = Net::new();
    let config = |active| NetConfig { active, hosts: BTreeSet::from([OURS.to_string()]), bridges: vec![fake.bridge.clone()] };
    // The client is built before bridges are in use, as the clients of the
    // app are: the rule is read when a request is made.
    let client = messenger_http::builder_on(net.clone(), Duration::from_secs(5), Duration::from_secs(20)).unwrap().build().unwrap();

    // Bridges off: the name of the project's server does not resolve here,
    // and nothing reaches the bridge.
    net.configure(config(false)).await.unwrap();
    assert!(client.get(format!("http://{OURS}/blob")).send().await.is_err());
    assert_eq!(fake.streams.load(Ordering::Relaxed), 0);

    // Bridges on: the same client, the same address.
    net.configure(config(true)).await.unwrap();
    let got = client.get(format!("http://{OURS}/blob")).send().await.unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(got.text().await.unwrap(), "the blob");
    assert_eq!(*fake.asked.lock().unwrap(), vec![format!("{OURS}:80")]);

    // A megabyte up, the way a chunk of a file goes.
    let put = client.put(format!("http://{OURS}/upload")).body(vec![7u8; 1024 * 1024]).send().await.unwrap();
    assert_eq!(put.status(), 201);

    // Somebody else's host: directly, the bridge hears nothing of it.
    let before = fake.asked.lock().unwrap().len();
    let direct = client.get(format!("{}/blob", server.uri())).send().await.unwrap();
    assert_eq!(direct.text().await.unwrap(), "the blob");
    assert_eq!(fake.asked.lock().unwrap().len(), before);

    // Off again: the same client goes directly at once. It keeps no
    // connection that could carry a request the old way.
    net.configure(config(false)).await.unwrap();
    assert!(client.get(format!("http://{OURS}/blob")).send().await.is_err());
    // And on again: through the bridge at once.
    net.configure(config(true)).await.unwrap();
    let again = client.get(format!("http://{OURS}/blob")).send().await.unwrap();
    assert_eq!(again.text().await.unwrap(), "the blob");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_may_not_lead_off_the_bridge() {
    let server = MockServer::start().await;
    // The project's server sends the request to an address the rule sends directly.
    let elsewhere = format!("{}/elsewhere", server.uri());
    Mock::given(method("GET")).and(path("/away")).respond_with(ResponseTemplate::new(302).insert_header("location", elsewhere.as_str())).mount(&server).await;
    Mock::given(method("GET")).and(path("/elsewhere")).respond_with(ResponseTemplate::new(200).set_body_string("off the bridge")).expect(0).mount(&server).await;
    // A redirect within the project's servers is followed.
    let inside = format!("http://{OURS}/blob");
    Mock::given(method("GET")).and(path("/inside")).respond_with(ResponseTemplate::new(302).insert_header("location", inside.as_str())).mount(&server).await;
    Mock::given(method("GET")).and(path("/blob")).respond_with(ResponseTemplate::new(200).set_body_string("the blob")).mount(&server).await;

    let fake = bridge(Behaviour::CarryTo(*server.address())).await;
    let net = Net::new();
    net.configure(NetConfig { active: true, hosts: BTreeSet::from([OURS.to_string()]), bridges: vec![fake.bridge.clone()] }).await.unwrap();
    let client = messenger_http::builder_on(net.clone(), Duration::from_secs(5), Duration::from_secs(20)).unwrap().build().unwrap();

    let refused = client.get(format!("http://{OURS}/away")).send().await;
    assert!(refused.is_err(), "the redirect off the bridge must not be followed");
    let followed = client.get(format!("http://{OURS}/inside")).send().await.unwrap();
    assert_eq!(followed.text().await.unwrap(), "the blob");
    // Dropping the server checks that `/elsewhere` was never asked.
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_that_stalled_directly_is_made_again_through_the_bridge() {
    let server = MockServer::start().await;
    Mock::given(method("PUT")).and(path("/upload")).respond_with(ResponseTemplate::new(201)).mount(&server).await;
    let fake = bridge(Behaviour::CarryTo(*server.address())).await;
    let net = Net::new();
    let config = |active| NetConfig { active, hosts: BTreeSet::from([OURS.to_string()]), bridges: vec![fake.bridge.clone()] };
    net.configure(config(false)).await.unwrap();
    let client = messenger_http::builder_on(net.clone(), Duration::from_secs(5), Duration::from_secs(20)).unwrap().build().unwrap();

    // The direct way takes the bytes and never answers, as a throttled way
    // does. Through the bridge the request is answered.
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let request = {
        let (net, client, attempts) = (net.clone(), client.clone(), attempts.clone());
        tokio::spawn(async move {
            messenger_http::following_route_on(&net, OURS, || {
                attempts.fetch_add(1, Ordering::Relaxed);
                let direct = net.route(OURS) == messenger_vlink::Route::Direct;
                let client = client.clone();
                async move {
                    if direct {
                        std::future::pending::<()>().await;
                    }
                    client.put(format!("http://{OURS}/upload")).body(vec![1u8; 256 * 1024]).send().await.map(|r| r.status().as_u16())
                }
            })
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!request.is_finished(), "stalled on the direct way");

    // Bridges on: the stalled request is given up and made again at once.
    net.configure(config(true)).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), request).await.expect("not left waiting").unwrap();
    assert_eq!(status.expect("the way settled").expect("answered"), 201);
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    assert_eq!(fake.streams.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_that_leaves_the_host_alone_interrupts_nothing() {
    let net = Net::new();
    let fake = bridge(Behaviour::Answer(http_status_ok())).await;
    let config = |hosts: &[&str]| NetConfig {
        active: true,
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        bridges: vec![fake.bridge.clone()],
    };
    net.configure(config(&[OURS])).await.unwrap();
    let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (n, a) = (net.clone(), attempts.clone());
    let request = tokio::spawn(async move {
        messenger_http::following_route_on(&n, OURS, || {
            a.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(400))
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Another host joins the list: this one still goes through the bridge.
    net.configure(config(&[OURS, "push.veydan.test"])).await.unwrap();
    assert_eq!(request.await.unwrap(), Ok(()));
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
}

fn http_status_ok() -> http::StatusCode {
    http::StatusCode::OK
}

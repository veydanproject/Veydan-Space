// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A private server told through the gateway, against a gateway (the home
//! push server) and a private push server played by mocks.

use super::*;
use crate::private::PrivateServerInput;
use crate::push::PushChannel;
use base64::Engine;
use messenger_core::MessengerConfig;
use messenger_notify::sealed::Unsealed;
use messenger_testkit::MemorySecretStore;
use nostr::nips::nip44;
use serde_json::{json, Value};
use std::sync::Arc;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A relay nothing listens on, of the home set.
const RELAY: &str = "ws://127.0.0.1:9";
/// The relay of the private server; nothing listens there either.
const PRIVATE_RELAY: &str = "ws://127.0.0.1:11";
const T1: &str = "7a4e2c1f9b8d06e35a1c4f2e8b7d9a0c3e5f1b2d4a6c8e0f1a3b5c7d9e1f2a4b";
const T2: &str = "c03b9e51aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Setup {
    rt: MessengerRuntime,
    config: MessengerConfig,
    /// The home push server, which is the gateway.
    gateway: MockServer,
    /// The push server of the private server.
    private: MockServer,
    /// The key the private push server seals with.
    seal: Keys,
    server_id: String,
    _dir: tempfile::TempDir,
}

fn channel() -> PushChannel {
    PushChannel {
        provider: "fcm".into(),
        token: "token-of-the-phone".into(),
        app_id: "net.veydan.mobile".into(),
        app_version: Some("4.1.0".into()),
    }
}

fn answer(relay: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "device_id": "x", "expires_at": 1_800_000_000u64,
        "relays": [{ "url": relay, "status": "pending" }]
    }))
}

fn issued(ticket: &str, days: u64) -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({ "ticket": ticket, "expires_at": now() + days * 86_400 }))
}

/// The gateway: devices and tickets.
async fn gateway_accepts(gateway: &MockServer) {
    Mock::given(method("PUT")).and(path_regex("^/v1/devices/[0-9a-f]{32}$")).respond_with(answer(RELAY)).mount(gateway).await;
    Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(204)).mount(gateway).await;
}

/// The private push server, naming `gateway_url` as its gateway.
async fn private_accepts(private: &MockServer, gateway_url: &str, seal: &Keys) {
    Mock::given(method("GET"))
        .and(path("/v1/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "0.5.0",
            "apps": [{ "id": "net.veydan.mobile", "providers": ["gateway"] }],
            "relays": { "policy": "allow_list", "allowed": [PRIVATE_RELAY] },
            "limits": { "devices_per_pubkey": 10 },
            "gateway": { "url": gateway_url, "sender": seal.public_key().to_hex() }
        })))
        .mount(private)
        .await;
    Mock::given(method("DELETE")).respond_with(ResponseTemplate::new(204)).mount(private).await;
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let config = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(config.clone(), Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    crate::servers::use_veydan_offline(&rt).await;
    for relay in rt.relays().list().await.unwrap() {
        rt.relays().set_enabled(&relay.url, false).await.unwrap();
    }
    rt.relays().add_user(RELAY, None).await.unwrap();
    rt.relays().set_silent(false).await.unwrap();
    rt.identity().create("pw").await.unwrap();
    rt.refresh_signer().await.unwrap();

    let (gateway, private) = (MockServer::start().await, MockServer::start().await);
    let seal = Keys::generate();
    let server_id = rt
        .server_put_private(&PrivateServerInput {
            relay_url: PRIVATE_RELAY.into(),
            key: Some("app-key".into()),
            media_url: None,
            call_node: None,
            name: Some("S1".into()),
            push_mode: None,
        })
        .await
        .unwrap();
    Setup { rt, config, gateway, private, seal, server_id, _dir: dir }
}

/// Pushes on at the gateway, the push of the private server on.
async fn turned_on(s: &Setup) -> crate::push::PushStatus {
    s.rt.server_set_push(&s.server_id, Some(s.private.uri())).await.unwrap();
    s.rt.push_set_channel(Some(channel())).await.unwrap();
    s.rt.push_set_server(Some(s.gateway.uri())).await.unwrap();
    s.rt.push_set_enabled(true).await.unwrap()
}

async fn asked(server: &MockServer, verb: &str, path_start: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path().starts_with(path_start))
        .collect()
}

fn body(r: &Request) -> Value {
    serde_json::from_slice(&r.body).unwrap()
}

/// What the private push server would hand the gateway: the data map,
/// sealed to the push key, base64url of the payload's bytes.
fn sealed(seal: &Keys, push_key: &str, data: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let to = nostr::key::PublicKey::from_hex(push_key).unwrap();
    let text = nip44::encrypt(seal.secret_key(), &to, serde_json::to_string(data).unwrap(), nip44::Version::V2).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(text).unwrap();
    [
        ("v", "2".to_string()),
        ("type", "sealed".to_string()),
        ("blob", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)),
        ("sender", seal.public_key().to_hex()),
        ("trace", "b21c07d4".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

#[tokio::test]
async fn a_private_server_is_told_through_the_gateway_and_its_push_opens() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).and(path_regex("^/v1/devices/[0-9a-f]{32}$")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;

    // A group that lives on the private server.
    let group = s.rt.group_create(messenger_groups::GroupKind::Private, "Team", "", true, Some(&s.server_id)).await.unwrap();

    let status = turned_on(&s).await;
    assert_eq!(status.state, "registered", "{status:?}");
    assert_eq!(status.targets.len(), 1, "{status:?}");
    let target = &status.targets[0];
    assert_eq!((target.state.as_str(), target.error.as_deref()), ("registered", None), "{target:?}");
    assert_eq!(target.server, s.private.uri());
    assert_eq!(target.gateway.as_deref(), Some(s.gateway.uri().as_str()));
    assert!(target.ticket_expires_at.is_some());
    assert_eq!(target.relays[0].url, PRIVATE_RELAY);

    // The gateway: the device with its token, and the ticket asked for it,
    // signed. Nothing of the private server there.
    let home = asked(&s.gateway, "PUT", "/v1/devices/").await;
    assert_eq!(home.len(), 1);
    let home_body = body(&home[0]);
    assert_eq!(home_body["channel"], json!({ "provider": "fcm", "token": "token-of-the-phone" }));
    assert_eq!(home_body["relays"], json!([{ "url": RELAY, "dm": true, "groups": true }]));
    assert_eq!(home_body["groups"], json!([]), "a group of a private server is not the home server's");
    let home_device = home[0].url.path().trim_start_matches("/v1/devices/").to_string();
    let tickets = asked(&s.gateway, "POST", "/v1/tickets").await;
    assert_eq!(tickets.len(), 1);
    assert_eq!(body(&tickets[0]), json!({ "device_id": home_device }));
    assert!(tickets[0].headers.get("authorization").is_some());

    // The private push server: the ticket and the push key, its relay
    // alone, its group; another device id than at home.
    let puts = asked(&s.private, "PUT", "/v1/devices/").await;
    assert_eq!(puts.len(), 1);
    let put = body(&puts[0]);
    let keys = s.rt.session_keys().await.unwrap();
    let push_key = s.rt.push_key_public(&keys.public_key().to_hex()).await.unwrap().unwrap();
    assert_ne!(push_key, keys.public_key().to_hex(), "the push key is not the identity's");
    assert_eq!(put["channel"], json!({ "provider": "gateway", "ticket": T1, "pubkey": push_key }));
    assert_eq!(put["relays"], json!([{ "url": PRIVATE_RELAY, "dm": true, "groups": true }]));
    let group_keys = s.rt.groups().push_keys(&group.id).await.unwrap();
    assert_eq!(put["groups"], json!([{ "id": group.id, "keys": group_keys }]));
    assert_ne!(puts[0].url.path(), home[0].url.path());
    let text = String::from_utf8_lossy(&puts[0].body).to_string();
    assert!(!text.contains("token-of-the-phone"), "the token never reaches a private server");
    assert!(!text.contains("app-key"), "nor the key of its relay");

    // Nothing changed, nothing said.
    for _ in 0..3 {
        s.rt.push_reconcile(false).await.unwrap();
    }
    assert_eq!(asked(&s.private, "PUT", "/v1/devices/").await.len(), 1);
    assert_eq!(asked(&s.private, "GET", "/v1/info").await.len(), 1);
    assert_eq!(asked(&s.gateway, "POST", "/v1/tickets").await.len(), 1);

    // A push of that server opens with the handler's bundle.
    let bundle = s.rt.notify_bundle().await.unwrap().unwrap();
    assert!(bundle.push_key.is_some());
    let inner: BTreeMap<String, String> = [
        ("v", "2"),
        ("type", "group"),
        ("group_id", group.id.as_str()),
        ("event_id", &"44".repeat(32)),
        ("relay", PRIVATE_RELAY),
        ("trace", "9583cb58"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let push = sealed(&s.seal, &push_key, &inner);
    let opened = messenger_notify::unseal_as(&s.config, Some(&bundle), &push).await.unwrap();
    assert_eq!(opened, Unsealed::Open { data: inner.clone() });
    // A key that is not the seal of a server of mine opens nothing.
    let stranger = sealed(&Keys::generate(), &push_key, &inner);
    assert!(matches!(messenger_notify::unseal_as(&s.config, Some(&bundle), &stranger).await.unwrap(), Unsealed::Dropped { .. }));
    // The question of the registration shows nothing.
    let check: BTreeMap<String, String> =
        [("v", "2"), ("type", "test"), ("check", "1"), ("trace", "x")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let check = sealed(&s.seal, &push_key, &check);
    assert_eq!(messenger_notify::unseal_as(&s.config, Some(&bundle), &check).await.unwrap(), Unsealed::Check);
    // A handler without keys says so.
    assert_eq!(messenger_notify::unseal_as(&s.config, None, &push).await.unwrap(), Unsealed::NoKeys);

    // The fingerprint of the bundle names the push key: the host gives the
    // handler the bundle anew.
    assert!(s.rt.notify_fingerprint().await.unwrap().unwrap().ends_with(&push_key));
    s.rt.shutdown().await;
}

#[tokio::test]
async fn a_ticket_the_gateway_does_not_know_is_asked_anew_once() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).up_to_n_times(1).mount(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T2, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": { "code": "token_invalid", "message": "the gateway does not know the ticket" },
            "request_id": "5f3a9c1e"
        })))
        .up_to_n_times(1)
        .mount(&s.private)
        .await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;

    let status = turned_on(&s).await;
    assert_eq!(status.targets[0].state, "registered", "{status:?}");
    let puts = asked(&s.private, "PUT", "/v1/devices/").await;
    assert_eq!(puts.len(), 2);
    assert_eq!(body(&puts[0])["channel"]["ticket"], T1);
    assert_eq!(body(&puts[1])["channel"]["ticket"], T2);
    // The first ticket is taken back.
    let deletes = asked(&s.gateway, "DELETE", "/v1/tickets/").await;
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].url.path(), format!("/v1/tickets/{T1}"));
    s.rt.shutdown().await;
}

#[tokio::test]
async fn a_ticket_refused_twice_is_a_failure_and_not_tried_at_once_again() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": { "code": "token_invalid", "message": "no" }, "request_id": "r"
        })))
        .mount(&s.private)
        .await;
    let status = turned_on(&s).await;
    let target = &status.targets[0];
    assert_eq!(target.state, "failed");
    assert!(target.error.as_deref().unwrap().starts_with("push_refused_token_invalid"), "{target:?}");
    assert_eq!(asked(&s.private, "PUT", "/v1/devices/").await.len(), 2, "once more, and no more");
    s.rt.push_reconcile(false).await.unwrap();
    assert_eq!(asked(&s.private, "PUT", "/v1/devices/").await.len(), 2, "not sooner than in a minute");
    s.rt.shutdown().await;
}

#[tokio::test]
async fn a_ticket_near_its_end_is_renewed_and_the_old_one_taken_back() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 3)).up_to_n_times(1).mount(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T2, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;

    turned_on(&s).await;
    assert!(asked(&s.gateway, "DELETE", "/v1/tickets/").await.is_empty());
    // Three days left: fewer than seven, so the next round renews it.
    let status = s.rt.push_reconcile(false).await.unwrap();
    assert_eq!(status.targets[0].state, "registered");
    let puts = asked(&s.private, "PUT", "/v1/devices/").await;
    assert_eq!(puts.len(), 2);
    assert_eq!(body(&puts[1])["channel"]["ticket"], T2);
    assert_eq!(puts[0].url.path(), puts[1].url.path(), "the same device there");
    let deletes = asked(&s.gateway, "DELETE", "/v1/tickets/").await;
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].url.path(), format!("/v1/tickets/{T1}"));
    // Thirty days left: nothing more.
    s.rt.push_reconcile(false).await.unwrap();
    assert_eq!(asked(&s.private, "PUT", "/v1/devices/").await.len(), 2);
    s.rt.shutdown().await;
}

#[tokio::test]
async fn a_gateway_that_is_no_push_server_of_the_network_gets_nothing() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, "https://elsewhere.example.org", &s.seal).await;

    let status = turned_on(&s).await;
    let target = &status.targets[0];
    assert_eq!(target.state, "failed");
    assert!(target.error.as_deref().unwrap().contains("push_gateway_unknown"), "{target:?}");
    assert!(asked(&s.gateway, "POST", "/v1/tickets").await.is_empty(), "no ticket for a host the server chose");
    assert!(asked(&s.private, "PUT", "/v1/devices/").await.is_empty());
    s.rt.shutdown().await;
}

#[tokio::test]
async fn a_server_whose_push_goes_off_forgets_the_device_and_its_ticket() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    turned_on(&s).await;
    let put_path = asked(&s.private, "PUT", "/v1/devices/").await[0].url.path().to_string();
    assert_eq!(servers::push_senders(s.rt.store()).await.unwrap(), vec![s.seal.public_key().to_hex()]);

    s.rt.server_set_push(&s.server_id, None).await.unwrap();
    let status = s.rt.push_reconcile(false).await.unwrap();
    assert!(status.targets.is_empty());
    let deletes = asked(&s.private, "DELETE", "/v1/devices/").await;
    assert_eq!(deletes.len(), 1);
    assert_eq!(deletes[0].url.path(), put_path);
    assert_eq!(asked(&s.gateway, "DELETE", "/v1/tickets/").await[0].url.path(), format!("/v1/tickets/{T1}"));
    assert!(servers::push_senders(s.rt.store()).await.unwrap().is_empty(), "its seal is no longer taken");
    // The home registration stays.
    assert!(asked(&s.gateway, "DELETE", "/v1/devices/").await.is_empty());
    s.rt.shutdown().await;
}

#[tokio::test]
async fn pushes_off_take_back_the_private_servers_too() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    turned_on(&s).await;

    let status = s.rt.push_set_enabled(false).await.unwrap();
    assert!(status.targets.is_empty());
    assert_eq!(asked(&s.private, "DELETE", "/v1/devices/").await.len(), 1);
    assert_eq!(asked(&s.gateway, "DELETE", "/v1/tickets/").await.len(), 1);
    assert_eq!(asked(&s.gateway, "DELETE", "/v1/devices/").await.len(), 1);
    assert!(settings::get(s.rt.store(), KEY_PRIVATE).await.unwrap().is_none());
    s.rt.shutdown().await;
}

#[tokio::test]
async fn the_tickets_are_listed_with_their_server_and_one_is_taken_back() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    Mock::given(method("GET"))
        .and(path("/v1/tickets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "tickets": [
            { "ticket": "7a4e2c1f…", "device_id": "d", "issued_at": 1, "expires_at": 2, "last_wake_at": 3 },
            { "ticket": "c03b9e51…", "device_id": "d", "issued_at": 1, "expires_at": 2, "last_wake_at": null }
        ]})))
        .mount(&s.gateway)
        .await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    turned_on(&s).await;

    let list = s.rt.push_tickets_list().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].server_id.as_deref(), Some(s.server_id.as_str()));
    assert_eq!(list[0].last_wake_at, Some(3));
    assert_eq!(list[1].server_id, None, "a ticket of no server of this device");

    assert!(s.rt.push_ticket_revoke("7a4e").await.is_err(), "eight hex at least");
    assert_eq!(s.rt.push_ticket_revoke("7a4e2c1f…").await.unwrap(), 1);
    assert_eq!(asked(&s.gateway, "DELETE", "/v1/tickets/").await[0].url.path(), "/v1/tickets/7a4e2c1f");
    // The server that held it is told anew, with a new ticket.
    s.rt.push_reconcile(false).await.unwrap();
    assert_eq!(asked(&s.gateway, "POST", "/v1/tickets").await.len(), 2);
    assert_eq!(asked(&s.private, "PUT", "/v1/devices/").await.len(), 2);
    s.rt.shutdown().await;
}

#[tokio::test]
async fn the_push_server_of_a_server_is_an_address_and_of_a_server_of_mine() {
    let s = setup().await;
    assert!(s.rt.server_set_push(&s.server_id, Some("ftp://x".into())).await.is_err());
    let e = s.rt.server_set_push(&"0".repeat(16), Some("https://push.example.org".into())).await;
    assert!(matches!(e, Err(MessengerError::Invalid(ref c)) if c == "server_unknown"), "{e:?}");
    s.rt.server_set_push(&s.server_id, Some("https://push.example.org/".into())).await.unwrap();
    let push = servers::push_of(s.rt.store(), &s.server_id).await.unwrap().unwrap();
    assert_eq!((push.push_mode.as_str(), push.push_url.as_deref()), ("veydan", Some("https://push.example.org")));
    s.rt.shutdown().await;
}

/// A gateway keeps eight live tickets of a device: seven servers wake it
/// through one gateway, the eighth ticket is the room of a renewal, and a
/// server beyond is told nothing, so no ticket of another is pushed out.
#[tokio::test]
async fn no_more_servers_wake_through_one_gateway_than_it_gives_tickets_for() {
    let s = setup().await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    for n in 0..SERVERS_PER_GATEWAY {
        let id = s
            .rt
            .server_put_private(&PrivateServerInput {
                relay_url: format!("ws://127.0.0.1:{}", 20 + n),
                key: None,
                media_url: None,
                call_node: None,
                name: None,
                push_mode: None,
            })
            .await
            .unwrap();
        s.rt.server_set_push(&id, Some(s.private.uri())).await.unwrap();
    }
    let status = turned_on(&s).await;
    assert_eq!(status.targets.len(), SERVERS_PER_GATEWAY + 1);
    let registered = status.targets.iter().filter(|t| t.state == "registered").count();
    assert_eq!(registered, SERVERS_PER_GATEWAY, "{:?}", status.targets);
    let over: Vec<_> = status.targets.iter().filter(|t| t.state == "failed").collect();
    assert_eq!(over.len(), 1);
    assert!(over[0].error.as_deref().unwrap().starts_with("push_ticket_limit"), "{over:?}");
    assert_eq!(asked(&s.gateway, "POST", "/v1/tickets").await.len(), SERVERS_PER_GATEWAY);
    s.rt.shutdown().await;
}

/// The push key is made for a private server just before it is told of
/// it, and that server asks with a sealed push right after: the host hears
/// at once that the handler's bundle changed, not at its next look.
#[tokio::test]
async fn a_push_key_made_is_said_at_once() {
    let s = setup().await;
    let mut events = s.rt.ui_events();
    let pubkey = s.rt.session_keys().await.unwrap().public_key().to_hex();
    let said = |events: &mut tokio::sync::broadcast::Receiver<messenger_core::traits::UiEvent>| {
        let mut n = 0;
        while let Ok(ev) = events.try_recv() {
            n += usize::from(ev.name == UI_EVENT_PUSH_KEY);
        }
        n
    };
    assert!(s.rt.push_key(&pubkey, false).await.unwrap().is_none());
    assert_eq!(said(&mut events), 0, "no key asked to be made, nothing said");
    s.rt.push_key(&pubkey, true).await.unwrap().unwrap();
    assert_eq!(said(&mut events), 1);
    s.rt.push_key(&pubkey, true).await.unwrap().unwrap();
    assert_eq!(said(&mut events), 0, "the key there is no news");

    // On the way of a registration: said before the private server is told.
    let s = setup().await;
    let mut events = s.rt.ui_events();
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    turned_on(&s).await;
    assert_eq!(said(&mut events), 1);
    s.rt.shutdown().await;
}

/// A server gone while its gateway is out of reach: the ticket it held
/// cannot be taken back then, and it is not forgotten — every round asks
/// again, after the server is gone and whatever the rounds tell, until
/// the gateway takes it back.
#[tokio::test]
async fn a_ticket_not_taken_back_is_asked_again_after_the_server_is_gone() {
    let s = setup().await;
    // The gateway fails the first take-back.
    Mock::given(method("DELETE"))
        .and(path(format!("/v1/tickets/{T1}")))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": { "code": "unavailable", "message": "later" }, "request_id": "r"
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s.gateway)
        .await;
    gateway_accepts(&s.gateway).await;
    Mock::given(method("POST")).and(path("/v1/tickets")).respond_with(issued(T1, 30)).mount(&s.gateway).await;
    private_accepts(&s.private, &s.gateway.uri(), &s.seal).await;
    Mock::given(method("PUT")).respond_with(answer(PRIVATE_RELAY)).mount(&s.private).await;
    turned_on(&s).await;

    s.rt.server_remove_private(&s.server_id).await.unwrap();
    let status = s.rt.push_reconcile(false).await.unwrap();
    assert!(status.targets.is_empty(), "the server is gone");
    let ticket_deletes = || async { asked(&s.gateway, "DELETE", "/v1/tickets/").await.len() };
    assert_eq!(ticket_deletes().await, 1, "asked once, and refused");
    let kept = settings::get(s.rt.store(), KEY_TAKE_BACK).await.unwrap().expect("the ticket is kept to take back");
    assert!(kept.contains(T1), "{kept}");

    // Not sooner than a minute, unless forced; then asked again, and done.
    s.rt.push_reconcile(false).await.unwrap();
    assert_eq!(ticket_deletes().await, 1);
    s.rt.push_reconcile(true).await.unwrap();
    assert_eq!(ticket_deletes().await, 2);
    let deletes = asked(&s.gateway, "DELETE", "/v1/tickets/").await;
    assert_eq!(deletes[1].url.path(), format!("/v1/tickets/{T1}"));
    assert!(settings::get(s.rt.store(), KEY_TAKE_BACK).await.unwrap().is_none(), "taken back, nothing kept");
    s.rt.push_reconcile(true).await.unwrap();
    assert_eq!(ticket_deletes().await, 2, "nothing more to ask");
    s.rt.shutdown().await;
}

/// Kept tickets: one the gateway has no more, or past its end, is not
/// asked again; the list keeps the longest-lived and no more than its cap.
#[test]
fn the_tickets_to_take_back_are_trimmed() {
    let held = |n: u64, expires_at: u64| Held { gateway: "https://g".into(), pubkey: "p".into(), ticket: format!("{n:064x}"), expires_at };
    let mut back = TakeBack { tickets: (0..(TAKE_BACK_MAX as u64 + 5)).map(|n| held(n, 1_000 + n)).collect(), last_try: 0 };
    back.tickets.push(held(999, 10));
    back.trim(500);
    assert_eq!(back.tickets.len(), TAKE_BACK_MAX);
    assert!(back.tickets.iter().all(|h| h.expires_at > 1_004), "the shortest-lived go first");
    back.trim(2_000);
    assert!(back.tickets.is_empty(), "dead at the gateway anyway");
    assert!(!worth_asking_again(&PushError::Refused { status: 404, code: "not_found".into(), message: String::new(), request_id: String::new() }));
    assert!(worth_asking_again(&PushError::Unreachable("timeout".into())));
    assert!(worth_asking_again(&PushError::Refused { status: 503, code: "unavailable".into(), message: String::new(), request_id: String::new() }));
}

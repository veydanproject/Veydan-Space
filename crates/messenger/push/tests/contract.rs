// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The client against the server's own examples, and against a server
//! played by a mock.
//!
//! `tests/golden` is a copy of `services/push/golden`.
//! `scripts/vpush-contract.sh` fails when the two differ.

use messenger_push::client::PushError;
use messenger_push::{
    Channel, DeviceAnswer, DevicePut, ErrorBody, GroupWatch, Info, Prefs, RelayStatus, RelayWatch, TicketAsk,
    TicketIssued, TicketList, TicketsInfo, VpushClient, Wake,
};
use nostr::key::Keys;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn golden(name: &str) -> String {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn registration() -> DevicePut {
    DevicePut {
        app_id: "net.veydan.mobile".into(),
        channel: Channel::Fcm { token: "fcm-token-of-the-device".into() },
        app_version: Some("4.0.1-alpha.14".into()),
        prefs: Prefs::default(),
        author_key: Some("5f1d0c3b8a7e4f2a9b6c1d0e3f4a5b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6".into()),
        relays: vec![
            RelayWatch { url: "wss://node-1.veydan.net".into(), dm: true, groups: true },
            RelayWatch { url: "wss://relay.example.org".into(), dm: true, groups: false },
        ],
        groups: vec![
            GroupWatch {
                id: "11".repeat(32),
                keys: vec!["c960ddf616adb9a57ac7d0eb04da5e25cf3d139c95d5828addb8f1f0ed50da28".into()],
            },
            GroupWatch { id: "22".repeat(32), keys: vec!["aa".repeat(32), "bb".repeat(32)] },
        ],
    }
}

#[test]
fn what_the_client_sends_is_the_servers_example() {
    let sent = serde_json::to_value(registration()).unwrap();
    let example: Value = serde_json::from_str(&golden("device_put.json")).unwrap();
    assert_eq!(sent, example);
}

#[test]
fn the_servers_answer_is_read() {
    let answer: DeviceAnswer = serde_json::from_str(&golden("device_answer.json")).unwrap();
    assert_eq!(answer.device_id, "phone-0001");
    assert_eq!(answer.relays[0].status, RelayStatus::Pending);
    assert!(answer.relays[0].status.watched());
    assert_eq!(answer.relays[1].status, RelayStatus::NotAllowed);
    assert!(!answer.relays[1].status.watched());
}

#[test]
fn a_word_of_a_newer_server_does_not_break_the_client() {
    let answer: DeviceAnswer = serde_json::from_str(
        r#"{"device_id":"d","expires_at":1,"new_field":true,
            "relays":[{"url":"wss://r","status":"quarantined","why":"x"}]}"#,
    )
    .unwrap();
    assert_eq!(answer.relays[0].status, RelayStatus::Unknown);
    assert!(!answer.relays[0].status.watched());
}

async fn server() -> (MockServer, VpushClient) {
    let server = MockServer::start().await;
    // The mock listens on 127.0.0.1, which the client accepts over http.
    let client = VpushClient::new(&server.uri()).unwrap();
    (server, client)
}

fn refusal(status: u16, code: &str, more: Value) -> ResponseTemplate {
    let mut error = json!({ "code": code, "message": "why" });
    if let Some(extra) = more.as_object() {
        for (k, v) in extra {
            error[k] = v.clone();
        }
    }
    ResponseTemplate::new(status)
        .insert_header("x-request-id", "5f3a9c1e")
        .set_body_json(json!({ "error": error, "request_id": "5f3a9c1e" }))
}

fn signed_at(request: &Request) -> u64 {
    use base64::Engine;
    let header = request.headers.get("authorization").unwrap().to_str().unwrap();
    let json = base64::engine::general_purpose::STANDARD
        .decode(header.strip_prefix("Nostr ").unwrap())
        .unwrap();
    serde_json::from_slice::<Value>(&json).unwrap()["created_at"].as_u64().unwrap()
}

#[tokio::test]
async fn a_device_is_registered() {
    let (server, client) = server().await;
    Mock::given(method("PUT"))
        .and(path("/v1/devices/abcdef0123456789"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(golden("device_answer.json"), "application/json"),
        )
        .mount(&server)
        .await;

    let keys = Keys::generate();
    let answer = client.put_device(&keys, "abcdef0123456789", &registration()).await.unwrap();
    assert_eq!(answer.relays.len(), 2);

    let request = &server.received_requests().await.unwrap()[0];
    let sent: Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(sent["app_id"], "net.veydan.mobile");

    // Signed for the address as it was asked, and for this very body.
    use base64::Engine;
    use sha2::Digest;
    let header = request.headers.get("authorization").unwrap().to_str().unwrap();
    let event: Value = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(header.strip_prefix("Nostr ").unwrap())
            .unwrap(),
    )
    .unwrap();
    let tags = event["tags"].as_array().unwrap();
    let tag = |name: &str| {
        tags.iter()
            .find(|t| t[0] == name)
            .map(|t| t[1].as_str().unwrap().to_string())
    };
    assert_eq!(tag("u").unwrap(), format!("{}/v1/devices/abcdef0123456789", server.uri()));
    assert_eq!(tag("method").unwrap(), "PUT");
    assert_eq!(tag("payload").unwrap(), hex::encode(sha2::Sha256::digest(&request.body)));
    assert_eq!(event["pubkey"], keys.public_key().to_hex());
}

#[tokio::test]
async fn a_refusal_carries_the_servers_reason_and_the_request_id() {
    let (server, client) = server().await;
    Mock::given(method("PUT"))
        .respond_with(refusal(422, "unknown_app", json!({})))
        .mount(&server)
        .await;

    let e = client
        .put_device(&Keys::generate(), "abcdef0123456789", &registration())
        .await
        .unwrap_err();
    assert_eq!(
        e,
        PushError::Refused {
            status: 422,
            code: "unknown_app".into(),
            message: "why".into(),
            request_id: "5f3a9c1e".into(),
        }
    );
    assert!(e.to_string().starts_with("push_refused_unknown_app"), "{e}");
}

#[tokio::test]
async fn a_wrong_clock_is_corrected_once_by_the_servers() {
    let (server, client) = server().await;
    let server_time = 1_600_000_000u64;
    Mock::given(method("DELETE"))
        .respond_with(refusal(401, "auth_expired", json!({ "server_time": server_time })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    client.delete_device(&Keys::generate(), "abcdef0123456789").await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_ne!(signed_at(&requests[0]), server_time);
    assert_eq!(signed_at(&requests[1]), server_time);
}

#[tokio::test]
async fn a_clock_the_server_keeps_refusing_is_not_tried_forever() {
    let (server, client) = server().await;
    Mock::given(method("DELETE"))
        .respond_with(refusal(401, "auth_expired", json!({ "server_time": 1 })))
        .mount(&server)
        .await;

    let e = client.delete_device(&Keys::generate(), "abcdef0123456789").await.unwrap_err();
    assert!(matches!(e, PushError::Refused { ref code, .. } if code == "auth_expired"), "{e:?}");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn what_is_not_a_push_server_is_told_apart() {
    let (server, client) = server().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404).set_body_string("<html>nginx</html>"))
        .mount(&server)
        .await;
    assert!(matches!(client.info().await, Err(PushError::Unreadable(_))));

    let nowhere = VpushClient::new("http://127.0.0.1:1").unwrap();
    assert!(matches!(nowhere.info().await, Err(PushError::Unreachable(_))));
}

#[tokio::test]
async fn info_is_asked_without_a_signature() {
    let (server, client) = server().await;
    Mock::given(method("GET"))
        .and(path("/v1/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "0.0.3",
            "apps": [{ "id": "net.veydan.mobile", "providers": ["fcm"] }],
            "relays": { "policy": "allow_list", "allowed": ["wss://node-1.veydan.net"] },
            "limits": { "devices_per_pubkey": 10 }
        })))
        .mount(&server)
        .await;

    let info: Info = client.info().await.unwrap();
    assert!(info.serves("net.veydan.mobile", "fcm"));
    assert!(!info.serves("net.veydan.mobile", "apns"));
    assert!(!info.serves("com.other", "fcm"));
    let request = &server.received_requests().await.unwrap()[0];
    assert!(request.headers.get("authorization").is_none());
}

#[tokio::test]
async fn a_test_push_is_asked_for() {
    let (server, client) = server().await;
    Mock::given(method("POST"))
        .and(path("/v1/devices/abcdef0123456789/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "outcome": "delivered", "trace": "9583cb58"
        })))
        .mount(&server)
        .await;
    let answer = client.test(&Keys::generate(), "abcdef0123456789").await.unwrap();
    assert_eq!((answer.outcome.as_str(), answer.trace.as_str()), ("delivered", "9583cb58"));
}

// ─── The gateway (spec/protocol.md, "The gateway") ──────────────────────────

#[test]
fn a_registration_through_the_gateway_is_the_servers_example() {
    let sent = DevicePut {
        app_id: "net.veydan.mobile".into(),
        channel: Channel::Gateway {
            ticket: "7a4e2c1f9b8d06e35a1c4f2e8b7d9a0c3e5f1b2d4a6c8e0f1a3b5c7d9e1f2a4b".into(),
            pubkey: "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa".into(),
        },
        app_version: Some("4.1.0".into()),
        prefs: Prefs::default(),
        author_key: None,
        relays: vec![RelayWatch { url: "wss://relay.my-server.example".into(), dm: true, groups: true }],
        groups: vec![],
    };
    let example: Value = serde_json::from_str(&golden("device_put_gateway.json")).unwrap();
    assert_eq!(serde_json::to_value(&sent).unwrap(), example);
    let back: DevicePut = serde_json::from_value(example).unwrap();
    assert_eq!(back.channel.provider(), "gateway");
    assert_eq!(back.channel.token(), "7a4e2c1f9b8d06e35a1c4f2e8b7d9a0c3e5f1b2d4a6c8e0f1a3b5c7d9e1f2a4b");
}

#[test]
fn a_private_push_server_names_its_gateway_and_its_seal() {
    let info: Info = serde_json::from_str(&golden("info_private.json")).unwrap();
    assert!(info.serves("net.veydan.mobile", "gateway"));
    assert!(!info.serves("net.veydan.mobile", "fcm"));
    let gateway = info.gateway.unwrap();
    assert_eq!(gateway.url, "https://eu-1-vpush.veydan.net");
    assert_eq!(gateway.sender, "466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27");
    assert!(info.tickets.is_none());
}

#[test]
fn a_gateway_says_it_issues_tickets() {
    let info: Info = serde_json::from_str(&golden("info_gateway.json")).unwrap();
    assert!(info.serves("net.veydan.mobile", "fcm"));
    assert_eq!(info.tickets, Some(TicketsInfo { days: 30, per_device: 8 }));
    assert!(info.gateway.is_none());
}

#[test]
fn the_bodies_of_the_tickets_are_the_servers_examples() {
    let ask = serde_json::to_value(TicketAsk { device_id: "phone-0001".into() }).unwrap();
    assert_eq!(ask, serde_json::from_str::<Value>(&golden("ticket_ask.json")).unwrap());

    let issued: TicketIssued = serde_json::from_str(&golden("ticket_issued.json")).unwrap();
    assert_eq!(issued.ticket.len(), 64);
    assert_eq!(issued.expires_at, 1792592000);

    let list: TicketList = serde_json::from_str(&golden("tickets.json")).unwrap();
    assert_eq!(list.tickets.len(), 2);
    assert_eq!(list.tickets[0].prefix(), "7a4e2c1f");
    assert_eq!(list.tickets[0].last_wake_at, Some(1790003600));
    assert_eq!(list.tickets[1].last_wake_at, None);
    assert_eq!(list.tickets[1].device_id, "phone-0001");

    let wake: Wake = serde_json::from_str(&golden("wake.json")).unwrap();
    let sealed: Value = serde_json::from_str(&golden("push_sealed.json")).unwrap();
    assert_eq!(wake.blob, sealed["blob"].as_str().unwrap());
    assert_eq!(wake.sender, sealed["sender"].as_str().unwrap());
    assert_eq!(sealed["type"], "sealed");
}

#[test]
fn a_ticket_unknown_is_read_as_a_refusal() {
    let body: ErrorBody = serde_json::from_str(&golden("error_ticket_unknown.json")).unwrap();
    assert_eq!(body.error.code, "ticket_unknown");
    assert_eq!(body.request_id, "5f3a9c1e");
}

#[tokio::test]
async fn tickets_are_asked_listed_and_taken_back_signed() {
    let (server, client) = server().await;
    Mock::given(method("POST"))
        .and(path("/v1/tickets"))
        .respond_with(ResponseTemplate::new(201).set_body_raw(golden("ticket_issued.json"), "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/tickets"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(golden("tickets.json"), "application/json"))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/tickets/7a4e2c1f"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let keys = Keys::generate();
    let issued = client.ticket_issue(&keys, "phone-0001").await.unwrap();
    assert_eq!(issued.expires_at, 1792592000);
    assert_eq!(client.tickets(&keys).await.unwrap().tickets.len(), 2);
    client.ticket_delete(&keys, "7a4e2c1f").await.unwrap();
    assert!(client.ticket_delete(&keys, "7a4e/../x").await.is_err(), "nothing but hex goes in the path");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    for r in &requests {
        assert!(r.headers.get("authorization").is_some(), "{} {} unsigned", r.method, r.url);
    }
    let asked: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(asked, serde_json::from_str::<Value>(&golden("ticket_ask.json")).unwrap());
}

#[tokio::test]
async fn a_gateway_that_does_not_know_the_device_says_so() {
    let (server, client) = server().await;
    Mock::given(method("POST"))
        .respond_with(refusal(409, "no_device", json!({})))
        .mount(&server)
        .await;
    let e = client.ticket_issue(&Keys::generate(), "phone-0001").await.unwrap_err();
    assert_eq!(e.code(), Some("no_device"));
}

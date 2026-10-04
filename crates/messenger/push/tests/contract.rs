// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The client against the server's own examples, and against a server
//! played by a mock.
//!
//! `tests/golden` is a copy of `services/push/golden`.
//! `scripts/vpush-contract.sh` fails when the two differ.

use messenger_push::client::PushError;
use messenger_push::{
    Channel, DeviceAnswer, DevicePut, GroupWatch, Info, Prefs, RelayStatus, RelayWatch, VpushClient,
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

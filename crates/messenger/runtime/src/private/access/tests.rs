// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The access API of a server, played by a mock on the port of the
//! server's relay (`ws://127.0.0.1:<port>` → `http://127.0.0.1:<port>`).

use super::*;
use crate::private::PrivateServerInput;
use base64::Engine;
use messenger_core::MessengerConfig;
use messenger_links::{Access, ServerLink};
use messenger_testkit::MemorySecretStore;
use nostr::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const INVITE: &str = "vi1.TeAm-1nv1te_0123";

struct Setup {
    rt: MessengerRuntime,
    /// The access API, on the port of the server's relay.
    api: MockServer,
    relay: String,
    _dir: tempfile::TempDir,
}

async fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let config = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(config, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().set_silent(true).await.unwrap();
    crate::servers::use_veydan_offline(&rt).await;
    rt.identity().create("pw").await.unwrap();
    rt.refresh_signer().await.unwrap();
    let api = MockServer::start().await;
    let relay = format!("ws://127.0.0.1:{}", api.address().port());
    Setup { rt, api, relay, _dir: dir }
}

async fn put(s: &Setup, access: Option<&str>) -> String {
    s.rt.server_put_private(&PrivateServerInput {
        relay_url: s.relay.clone(),
        name: Some("Team".into()),
        access: access.map(String::from),
        ..Default::default()
    })
    .await
    .unwrap()
}

fn event_of(r: &Request) -> Event {
    let header = r.headers.get("authorization").expect("NIP-98").to_str().unwrap();
    let json = base64::engine::general_purpose::STANDARD.decode(header.strip_prefix("Nostr ").unwrap()).unwrap();
    serde_json::from_slice(&json).unwrap()
}

fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| match t.as_slice() {
        [kind, value, ..] if kind == name => Some(value.as_str()),
        _ => None,
    })
}

async fn joins(api: &MockServer) -> Vec<Request> {
    api.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == JOIN_PATH).collect()
}

#[test]
fn the_api_is_under_the_host_of_the_relay() {
    for (relay, base) in [
        ("wss://relay.s1.example", "https://relay.s1.example"),
        ("wss://Relay.S1.example:7443/", "https://relay.s1.example:7443"),
        ("wss://relay.s1.example/path?key=app-key", "https://relay.s1.example"),
        ("ws://127.0.0.1:7777", "http://127.0.0.1:7777"),
        ("ws://localhost:7777", "http://localhost:7777"),
        ("ws://[::1]:7777", "http://[::1]:7777"),
        ("wss://127.0.0.1:7777", "https://127.0.0.1:7777"),
    ] {
        assert_eq!(access_base(relay).unwrap(), base, "{relay}");
    }
    for bad in ["ws://relay.s1.example", "https://relay.s1.example", "wss://u:p@relay.s1.example", "relay", "ws://10.0.0.1"] {
        assert!(access_base(bad).is_err(), "{bad}");
    }
}

/// The mode a server says is believed for ten minutes and kept as the
/// server's here; a kit without the API is an open server.
#[tokio::test]
async fn the_mode_of_a_server_is_asked_and_kept() {
    let s = setup().await;
    let id = put(&s, None).await;
    Mock::given(method("GET"))
        .and(path(INFO_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "v": 1, "mode": "team", "invite": true, "join": JOIN_PATH })))
        .expect(1)
        .mount(&s.api)
        .await;
    let mut ui = s.rt.ui_events();
    let info = s.rt.server_access_info(&id).await.unwrap();
    assert_eq!(info, AccessInfo { mode: "team".into(), invite: true });
    assert_eq!(s.rt.server_access_info(&id).await.unwrap(), info, "from the cache");
    let view = s.rt.servers_private_view().await.unwrap();
    assert_eq!(view[0].access, crate::links::ServerAccess::Team, "the server's word is kept");
    let told = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if ui.recv().await.unwrap().name == crate::private::UI_EVENT_SERVERS_UPDATED {
                return;
            }
        }
    })
    .await;
    assert!(told.is_ok(), "the UI is told the mode changed");
    assert!(matches!(s.rt.server_access_info("0123456789abcdef").await, Err(MessengerError::Invalid(w)) if w == servers::ERR_SERVER_UNKNOWN));

    // A kit of before the modes.
    let old = setup().await;
    let id = put(&old, Some("team")).await;
    let info = old.rt.server_access_info(&id).await.unwrap();
    assert_eq!(info, AccessInfo { mode: "open".into(), invite: false });

    // A kit of 0.1.8 or before: its nginx answers 401 from `location /`.
    let gated = setup().await;
    let id = put(&gated, Some("team")).await;
    Mock::given(method("GET"))
        .and(path(INFO_PATH))
        .respond_with(ResponseTemplate::new(401))
        .mount(&gated.api)
        .await;
    let info = gated.rt.server_access_info(&id).await.unwrap();
    assert_eq!(info, AccessInfo { mode: "open".into(), invite: false }, "401 is an open server, not an unreachable one");
}

/// A join is a NIP-98 POST of the invite under my key; it lets the
/// server's refusal go and tells the UI.
#[tokio::test]
async fn a_join_sends_the_invite_under_my_key_and_forgets_the_refusal() {
    let s = setup().await;
    let id = put(&s, Some("team")).await;
    servers::set_denied(s.rt.store(), &id, "restricted: not a member of this server").await.unwrap();
    assert!(s.rt.servers_private_view().await.unwrap()[0].denied.is_some());
    Mock::given(method("POST"))
        .and(path(JOIN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "member": true })))
        .expect(1)
        .mount(&s.api)
        .await;
    let mut ui = s.rt.ui_events();
    s.rt.server_join(&id, &format!(" {INVITE} ")).await.unwrap();

    let sent = joins(&s.api).await;
    let body = &sent[0].body;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(body).unwrap(), json!({ "invite": INVITE }));
    let event = event_of(&sent[0]);
    event.verify().unwrap();
    let me = s.rt.session_keys().await.unwrap().public_key();
    assert_eq!(event.pubkey, me);
    assert_eq!(event.kind.as_u16(), 27235);
    assert_eq!(tag(&event, "u"), Some(format!("{}{JOIN_PATH}", s.api.uri()).as_str()), "the whole address");
    assert_eq!(tag(&event, "method"), Some("POST"));
    assert_eq!(tag(&event, "payload"), Some(hex::encode(Sha256::digest(body)).as_str()));

    let view = s.rt.servers_private_view().await.unwrap();
    assert_eq!(view[0].denied, None, "«Нет доступа» goes");
    let joined = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let ev = ui.recv().await.unwrap();
            if ev.name == crate::private::UI_EVENT_SERVERS_UPDATED && ev.payload["joined"] == id.as_str() {
                return;
            }
        }
    })
    .await;
    assert!(joined.is_ok(), "the session hears that the server let me in");
}

/// What a server answers a join with, as the UI reads it; a code that is
/// no invite never leaves the device.
#[tokio::test]
async fn the_answers_of_a_join_are_words_of_the_ui() {
    for (status, body, word) in [
        (401, json!({ "error": "invite_invalid" }), Some(ERR_INVITE_INVALID)),
        (401, json!({ "error": "invite_used" }), Some(ERR_INVITE_USED)),
        (401, json!({ "error": "invite_expired" }), Some(ERR_INVITE_EXPIRED)),
        (401, json!({ "error": "auth_invalid" }), Some(ERR_JOIN_FAILED)),
        (429, json!({}), Some(ERR_JOIN_FAILED)),
        (500, json!({}), Some(ERR_JOIN_FAILED)),
        (200, json!({ "member": false }), Some(ERR_JOIN_FAILED)),
        (403, json!({ "error": "mode_open" }), None),
    ] {
        let s = setup().await;
        let id = put(&s, Some("team")).await;
        servers::set_denied(s.rt.store(), &id, "restricted: x").await.unwrap();
        Mock::given(method("POST"))
            .and(path(JOIN_PATH))
            .respond_with(ResponseTemplate::new(status).set_body_json(body.clone()))
            .mount(&s.api)
            .await;
        let got = s.rt.server_join(&id, INVITE).await;
        match word {
            Some(word) => {
                let Err(MessengerError::Invalid(said)) = &got else { panic!("{status} {body}: {got:?}") };
                assert!(said == word || said.starts_with(&format!("{word}: ")), "{status} {body}: {said}");
                assert!(!said.contains(INVITE), "the invite is never in an error: {said}");
                assert!(servers::denied(s.rt.store(), &id).await.unwrap().is_some(), "still refused");
            }
            None => {
                got.unwrap();
                let row = servers::get(s.rt.store(), &id).await.unwrap().unwrap();
                assert_eq!(row.access, servers::ACCESS_OPEN, "an open server needs no invite");
                assert_eq!(servers::denied(s.rt.store(), &id).await.unwrap(), None);
            }
        }
    }
    let s = setup().await;
    let id = put(&s, Some("team")).await;
    for bad in ["", "abc", "vi1.", "vi1.a b", &format!("vi1.{}", "a".repeat(200))] {
        assert!(matches!(s.rt.server_join(&id, bad).await, Err(MessengerError::Invalid(w)) if w == ERR_INVITE_INVALID), "{bad}");
    }
    assert!(joins(&s.api).await.is_empty(), "nothing was sent");
    // Nobody listens there: the server is unreachable.
    let gone = s.rt.server_put_private(&PrivateServerInput { relay_url: "ws://127.0.0.1:9".into(), ..Default::default() }).await.unwrap();
    let Err(MessengerError::Invalid(said)) = s.rt.server_join(&gone, INVITE).await else { panic!() };
    assert!(said.starts_with(ERR_JOIN_FAILED), "{said}");
}

/// A card of a team server with an invite: the server is added with its
/// mode and joined; a failed join leaves it added, and is the answer.
/// The invite is not kept with the server.
#[tokio::test]
async fn a_card_with_an_invite_adds_the_server_and_joins_it() {
    let s = setup().await;
    let card = ServerLink {
        name: "Team".into(),
        access: Access::Team,
        invite: Some(INVITE.into()),
        ..ServerLink::new(messenger_core::RelayUrl::parse(&s.relay).unwrap())
    };
    let preview = s.rt.server_preview(&card.encode()).await.unwrap();
    assert_eq!((preview.access, preview.invite.as_deref()), (crate::links::ServerAccess::Team, Some(INVITE)));
    assert!(preview.link.contains("inv="), "the dialog hands the invite back to accept");

    Mock::given(method("POST"))
        .and(path(JOIN_PATH))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "invite_used" })))
        .up_to_n_times(1)
        .mount(&s.api)
        .await;
    let got = s.rt.server_accept(&preview.link).await;
    assert!(matches!(&got, Err(MessengerError::Invalid(w)) if w == ERR_INVITE_USED), "{got:?}");
    let view = s.rt.servers_private_view().await.unwrap();
    assert_eq!((view.len(), view[0].id.as_str(), view[0].access), (1, preview.id.as_str(), crate::links::ServerAccess::Team));
    // Its card, as this device writes it, has the mode and no invite.
    let mine = s.rt.server_card(&preview.id).await.unwrap();
    assert_eq!((mine.access, mine.invite), (Access::Team, None));

    Mock::given(method("POST"))
        .and(path(JOIN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "member": true })))
        .mount(&s.api)
        .await;
    assert_eq!(s.rt.server_accept(&preview.link).await.unwrap(), preview.id, "accepted again, joined");
    assert_eq!(joins(&s.api).await.len(), 2);
    // An invite on the card of an open server is no word.
    let open = setup().await;
    let card = ServerLink { invite: Some(INVITE.into()), ..ServerLink::new(messenger_core::RelayUrl::parse(&open.relay).unwrap()) };
    open.rt.server_accept(&card.encode()).await.unwrap();
    assert!(joins(&open.api).await.is_empty());
}

#[tokio::test]
async fn what_i_am_on_a_server_is_asked_under_my_key() {
    let s = setup().await;
    let id = put(&s, Some("team")).await;
    Mock::given(method("GET"))
        .and(path(ME_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "member": true, "admin": false, "mode": "team" })))
        .mount(&s.api)
        .await;
    let me = s.rt.server_access_me(&id).await.unwrap();
    assert_eq!(me, AccessMe { member: true, admin: false, mode: "team".into() });
    let asked = s.api.received_requests().await.unwrap();
    let event = event_of(&asked[0]);
    event.verify().unwrap();
    assert_eq!((tag(&event, "method"), tag(&event, "payload")), (Some("GET"), None));
    assert_eq!(tag(&event, "u"), Some(format!("{}{ME_PATH}", s.api.uri()).as_str()));
}

/// A relay of a team server played by a local relay: it refuses my
/// requests and events while I am no member, and counts the requests.
#[derive(Debug)]
struct Team {
    member: Arc<std::sync::atomic::AtomicBool>,
    asked: Arc<std::sync::atomic::AtomicUsize>,
}

impl nostr_sdk::local_relay::QueryPolicy for Team {
    fn admit_query<'a>(
        &'a self,
        _: &'a mut Filter,
        _: &'a std::net::SocketAddr,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = nostr_sdk::local_relay::QueryPolicyResult> + Send + 'a>> {
        use nostr_sdk::local_relay::QueryPolicyResult;
        use std::sync::atomic::Ordering;
        self.asked.fetch_add(1, Ordering::SeqCst);
        let member = self.member.load(Ordering::SeqCst);
        Box::pin(async move {
            if member {
                QueryPolicyResult::Accept
            } else {
                QueryPolicyResult::reject(
                    nostr::message::MachineReadablePrefix::Restricted,
                    "not a member of this server; ask its admin for an invite",
                )
            }
        })
    }
}

async fn denied_when(rt: &MessengerRuntime, want: bool) -> Option<String> {
    let mut denied = None;
    for _ in 0..100 {
        denied = rt.servers_private_view().await.unwrap()[0].denied.clone();
        if denied.is_some() == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    denied
}

/// The session on a team server I am no member of: its relay refuses the
/// inbox, and the settings say «Нет доступа» with the relay's words. Let
/// in, the session asks the relay again (`nostr-sdk` dropped the refused
/// subscription), and the refusal stays gone.
#[tokio::test]
async fn a_refusal_of_the_relay_is_no_access_until_the_server_lets_me_in() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let home = nostr_sdk::local_relay::LocalRelay::builder().build();
    home.run().await.unwrap();
    let member = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(AtomicUsize::new(0));
    let team = nostr_sdk::local_relay::LocalRelay::builder()
        .query_policy(Team { member: member.clone(), asked: asked.clone() })
        .build();
    team.run().await.unwrap();
    let home_url = home.url().await.as_str_without_trailing_slash().to_string();
    let team_url = team.url().await.as_str_without_trailing_slash().to_string();

    let dir = tempfile::tempdir().unwrap();
    let cfg = MessengerConfig::new(dir.path().join("messenger"));
    let rt = MessengerRuntime::start(cfg, Arc::new(MemorySecretStore::unlocked())).await.unwrap();
    rt.relays().add_user(&home_url, None).await.unwrap();
    rt.servers_use_own().await.unwrap();
    let id = rt
        .server_put_private(&PrivateServerInput { relay_url: team_url, access: Some("team".into()), ..Default::default() })
        .await
        .unwrap();
    rt.identity().create("pw").await.unwrap();
    assert!(rt.refresh_signer().await.unwrap());

    let denied = denied_when(&rt, true).await;
    assert_eq!(denied.as_deref(), Some("restricted: not a member of this server; ask its admin for an invite"));
    assert_eq!(rt.servers_private_view().await.unwrap()[0].access, crate::links::ServerAccess::Team);

    // The admin let me in (what a join does once the server said yes).
    member.store(true, Ordering::SeqCst);
    let before = asked.load(Ordering::SeqCst);
    servers::clear_denied(rt.store(), &id).await.unwrap();
    rt.server_let_in(&id).await;
    let mut again = false;
    for _ in 0..100 {
        if asked.load(Ordering::SeqCst) > before {
            again = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(again, "the inbox of the server is asked again");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(denied_when(&rt, false).await, None, "no refusal since");
    rt.shutdown().await;
}

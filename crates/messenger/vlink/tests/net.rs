// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The rule and the door, against a bridge on this machine.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use http::StatusCode;
use messenger_vlink::{trust, BridgeRef, Error, Net, NetConfig, Route};
use messenger_vlink::testing::{
    self as support, bridge, bridge_with, echo_server, id_of, there_and_back, Behaviour, CERT, KEY, OTHER_CERT, OTHER_KEY,
    RENEWED_CERT, RENEWED_KEY,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const OURS: &str = "media.veydan.test";
const THEIRS: &str = "example.org";

fn config(active: bool, bridges: Vec<BridgeRef>) -> NetConfig {
    NetConfig { active, hosts: BTreeSet::from([OURS.to_string()]), bridges }
}

/// A bridge that carries to an echo, and a rule that uses it.
async fn net_with_bridge() -> (Net, support::FakeBridge) {
    let fake = bridge(Behaviour::CarryTo(echo_server().await)).await;
    let net = Net::new();
    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    (net, fake)
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_projects_servers_go_through_a_bridge() {
    let (net, _fake) = net_with_bridge().await;
    assert_eq!(net.route(OURS), Route::Bridge);
    assert_eq!(net.route("MEDIA.Veydan.Test"), Route::Bridge);
    assert_eq!(net.route(THEIRS), Route::Direct);
    assert!(net.socks_url(OURS).is_some());
    assert!(net.socks_url(THEIRS).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_bridged_until_asked_and_without_bridges() {
    let net = Net::new();
    // Never configured.
    assert_eq!(net.route(OURS), Route::Direct);
    assert!(net.socks_url(OURS).is_none());
    assert!(!net.is_active());

    // Bridges known, not in use: they can be tried, nothing is sent to them.
    let fake = bridge(Behaviour::CarryTo(echo_server().await)).await;
    net.configure(config(false, vec![fake.bridge.clone()])).await.unwrap();
    assert_eq!(net.route(OURS), Route::Direct);
    assert!(net.socks_url(OURS).is_none());
    assert!(net.client().is_some());

    // In use, with no bridge to use.
    net.configure(config(true, vec![])).await.unwrap();
    assert_eq!(net.route(OURS), Route::Direct);
    assert!(!net.is_active());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_goes_through_the_bridge_and_names_its_host() {
    let (net, fake) = net_with_bridge().await;
    let mut stream = net.open(OURS, 443).await.unwrap();
    there_and_back(&mut stream, "through the bridge").await;
    assert_eq!(*fake.asked.lock().unwrap(), vec![format!("{OURS}:443")]);
    assert_eq!(net.current().await, Some(fake.bridge.clone()));

    // What the rule sends directly is not carried, even when asked.
    assert!(matches!(net.open(THEIRS, 443).await, Err(Error::Refused(_))));
    assert_eq!(fake.streams.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rule_changes_while_everything_runs() {
    let (net, fake) = net_with_bridge().await;
    let door = net.socks_url(OURS).unwrap();

    net.configure(config(false, vec![fake.bridge.clone()])).await.unwrap();
    assert_eq!(net.route(OURS), Route::Direct);
    assert!(net.socks_url(OURS).is_none());
    assert!(matches!(net.open(OURS, 443).await, Err(Error::Refused(_))));

    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    // The same door as before: clients that were told of it keep working.
    assert_eq!(net.socks_url(OURS).unwrap(), door);
    there_and_back(&mut net.open(OURS, 443).await.unwrap(), "on again").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_bridge_is_passed_over() {
    let fake = bridge(Behaviour::CarryTo(echo_server().await)).await;
    let dead = BridgeRef { addr: "127.0.0.1:1".parse().unwrap(), id: fake.bridge.id, sni: None };
    let net = Net::new();
    net.configure(config(true, vec![dead, fake.bridge.clone()])).await.unwrap();
    there_and_back(&mut net.open(OURS, 443).await.unwrap(), "the second one").await;
    assert_eq!(net.current().await, Some(fake.bridge));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_address_that_shows_a_certificate_over_another_key_is_no_bridge() {
    // What is at the address has a key of its own: the id of the bridge
    // the user was given is of another key, and the chain shown, however
    // alike, is signed by the wrong one.
    let fake = bridge_with(Behaviour::CarryTo(echo_server().await), OTHER_CERT, OTHER_KEY).await;
    assert_eq!(fake.bridge.id, id_of(OTHER_CERT));
    let wrong = BridgeRef { addr: fake.bridge.addr, id: id_of(CERT), sni: None };
    let net = Net::new();
    net.configure(config(true, vec![wrong])).await.unwrap();
    assert!(matches!(net.open(OURS, 443).await, Err(Error::Unreachable(_))));
    assert_eq!(fake.streams.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tls_certificate_the_bridges_key_did_not_sign_is_no_bridge() {
    // An impostor at the address with the bridge's public chain (anybody
    // who called the bridge has it) but not its key: their own TLS
    // certificate, the bridge's key certificate put behind it. The id of
    // that certificate is the bridge's; the signature is not.
    let cert_of = |chain: &str, n: usize| chain.split_inclusive("-----END CERTIFICATE-----\n").nth(n).unwrap().to_string();
    let forged = cert_of(OTHER_CERT, 0) + &cert_of(CERT, 1);
    let fake = bridge_with(Behaviour::CarryTo(echo_server().await), &forged, OTHER_KEY).await;
    assert_eq!(fake.bridge.id, id_of(CERT));
    let net = Net::new();
    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    assert!(matches!(net.open(OURS, 443).await, Err(Error::Unreachable(_))));
    assert_eq!(fake.streams.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_with_a_renewed_certificate_is_the_bridge_the_user_knows() {
    // The bridge made itself a new TLS key and a new certificate over it,
    // signed by the key it has had all along: the reference the user
    // holds, with the id of that key, still reaches it. The client here is
    // the one the app is built on, for every system alike.
    assert_eq!(id_of(RENEWED_CERT), id_of(CERT));
    assert_ne!(RENEWED_CERT, CERT);
    assert_ne!(RENEWED_KEY, KEY);
    let fake = bridge_with(Behaviour::CarryTo(echo_server().await), RENEWED_CERT, RENEWED_KEY).await;
    let known = BridgeRef { addr: fake.bridge.addr, id: id_of(CERT), sni: None };
    let net = Net::new();
    net.configure(config(true, vec![known.clone()])).await.unwrap();
    let mut stream = net.open(OURS, 443).await.unwrap();
    there_and_back(&mut stream, "the same bridge, a new certificate").await;
    assert_eq!(net.current().await, Some(known));
    assert_eq!(fake.streams.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_bridge_answers_is_told_apart() {
    for (status, expect_refused) in [(StatusCode::FORBIDDEN, true), (StatusCode::SERVICE_UNAVAILABLE, false)] {
        let fake = bridge(Behaviour::Answer(status)).await;
        let net = Net::new();
        net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
        match net.open(OURS, 443).await {
            // The hub does not carry this host: another bridge would say the same.
            Err(Error::Refused(_)) => assert!(expect_refused),
            // The bridge has no hub: it is of no use, and there is no other.
            Err(Error::Unreachable(why)) => {
                assert!(!expect_refused);
                assert!(why.contains("no hub"), "{why}");
            }
            other => panic!("{status}: {:?}", other.err()),
        }
    }
}

// --- the SOCKS door ---------------------------------------------------------

/// `socks5h://user:password@127.0.0.1:port` taken apart.
fn door_of(url: &str) -> (String, String, String) {
    let rest = url.strip_prefix("socks5h://").unwrap();
    let (login, addr) = rest.split_once('@').unwrap();
    let (user, password) = login.split_once(':').unwrap();
    (user.to_string(), password.to_string(), addr.to_string())
}

/// Speaks SOCKS5 to the door and asks for `host:443`. Returns the reply
/// code of the request, or the point at which the door said no.
async fn knock(addr: &str, login: Option<(&str, &str)>, host: &str) -> Result<TcpStream, String> {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let method = if login.is_some() { 2u8 } else { 0u8 };
    s.write_all(&[5, 1, method]).await.unwrap();
    let mut answer = [0u8; 2];
    s.read_exact(&mut answer).await.unwrap();
    if answer[1] == 0xff {
        return Err("no method".into());
    }
    if let Some((user, password)) = login {
        let mut hello = vec![1, user.len() as u8];
        hello.extend_from_slice(user.as_bytes());
        hello.push(password.len() as u8);
        hello.extend_from_slice(password.as_bytes());
        s.write_all(&hello).await.unwrap();
        s.read_exact(&mut answer).await.unwrap();
        if answer[1] != 0 {
            return Err("wrong login".into());
        }
    }
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&443u16.to_be_bytes());
    s.write_all(&request).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    if reply[1] != 0 {
        return Err(format!("reply {}", reply[1]));
    }
    Ok(s)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_door_serves_this_process_and_the_projects_servers() {
    let (net, fake) = net_with_bridge().await;
    let (user, password, addr) = door_of(&net.socks_url(OURS).unwrap());
    assert!(addr.starts_with("127.0.0.1:"), "{addr}");

    // Whoever does not know the login is not served.
    assert_eq!(knock(&addr, None, OURS).await.unwrap_err(), "no method");
    assert_eq!(knock(&addr, Some((&user, "guess")), OURS).await.unwrap_err(), "wrong login");

    // With the login: a server of the project.
    let mut stream = knock(&addr, Some((&user, &password)), OURS).await.unwrap();
    there_and_back(&mut stream, "through the door").await;
    assert_eq!(*fake.asked.lock().unwrap(), vec![format!("{OURS}:443")]);

    // With the login: somebody else's host. "Not allowed by the rules."
    assert_eq!(knock(&addr, Some((&user, &password)), THEIRS).await.unwrap_err(), "reply 2");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_door_closes_with_the_messenger_and_opens_with_it_again() {
    let (net, fake) = net_with_bridge().await;
    let (user, password, addr) = door_of(&net.socks_url(OURS).unwrap());
    there_and_back(&mut net.open(OURS, 443).await.unwrap(), "before").await;
    assert_eq!(net.current().await, Some(fake.bridge.clone()));

    net.close();
    net.close();
    // The listener goes with the task that served it.
    let mut refused = false;
    for _ in 0..50 {
        if TcpStream::connect(&addr).await.is_err() {
            refused = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(refused, "the door at {addr} still takes connections");
    // No bridge is held, and what the rule bridges does not go directly:
    // its way leads nowhere until the door opens again.
    assert_eq!(net.current().await, None);
    assert_eq!(net.route(OURS), Route::Bridge);
    assert!(net.socks_url(OURS).unwrap().ends_with("@127.0.0.1:0"));

    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    let (_, _, again) = door_of(&net.socks_url(OURS).unwrap());
    let mut stream = knock(&again, Some((&user, &password)), OURS).await.unwrap();
    there_and_back(&mut stream, "after").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_rules_have_two_logins() {
    let (a, _fa) = net_with_bridge().await;
    let (b, _fb) = net_with_bridge().await;
    let (user_a, password_a, _) = door_of(&a.socks_url(OURS).unwrap());
    let (user_b, password_b, addr_b) = door_of(&b.socks_url(OURS).unwrap());
    assert_ne!((&user_a, &password_a), (&user_b, &password_b));
    assert_eq!(knock(&addr_b, Some((&user_a, &password_a)), OURS).await.unwrap_err(), "wrong login");
}

// --- what is built in -------------------------------------------------------

#[test]
fn what_is_built_in_is_well_formed() {
    assert_eq!(trust::ROOT_PUB.len(), 64);
    assert!(trust::ROOT_PUB.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(!trust::REGISTRIES.is_empty());
    assert!(trust::REGISTRIES.iter().all(|r| r.starts_with("https://") && !r.ends_with('/')));
    assert!(!trust::SEEDS.is_empty());
    for seed in trust::SEEDS {
        seed.parse::<BridgeRef>().unwrap_or_else(|e| panic!("{seed}: {e}"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn whatever_is_under_way_hears_of_a_change_of_the_way() {
    let fake = bridge(Behaviour::CarryTo(echo_server().await)).await;
    let net = Net::new();
    let mut changes = net.changes();
    let before = *changes.borrow_and_update();

    // Bridges given and in use: a change.
    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    assert!(changes.has_changed().unwrap());
    let on = *changes.borrow_and_update();
    assert!(on > before);

    // The same again: no change, nothing under way is disturbed.
    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    assert!(!changes.has_changed().unwrap());

    // Off: a change.
    net.configure(config(false, vec![fake.bridge.clone()])).await.unwrap();
    assert!(changes.has_changed().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridged_host_never_falls_back_to_the_direct_way() {
    // The door is there before the rule may send anything to it: right
    // after the first configure, a host of the project has a way, and the
    // way is the door.
    let fake = bridge(Behaviour::CarryTo(echo_server().await)).await;
    let net = Net::new();
    net.configure(config(true, vec![fake.bridge.clone()])).await.unwrap();
    let url = net.socks_url(OURS).expect("a host of the project always has a proxy while bridged");
    assert!(url.starts_with("socks5h://") && url.contains("@127.0.0.1:"), "{url}");
}

//! The Linux backend against a notification server of our own, on a
//! private `dbus-daemon`: nothing here depends on the desktop it runs on.
#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use desktop_notify::{Action, AppInfo, Handlers, Kind, Notifier, Toast};
use tokio::sync::mpsc;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

struct Bus(Child);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn private_bus() -> (Bus, String) {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("dbus-daemon is needed for this test");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    (Bus(child), line.trim().to_string())
}

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    replaces: u32,
    summary: String,
    body: String,
    actions: Vec<String>,
    silent: bool,
    entry: String,
    category: String,
    sound: String,
    urgency: Option<u8>,
    timeout: i32,
}

#[derive(Default)]
struct State {
    next: u32,
    seen: Vec<Seen>,
    closed: Vec<u32>,
}

struct Server(Arc<Mutex<State>>);

fn text(hints: &HashMap<String, OwnedValue>, name: &str) -> String {
    hints.get(name).and_then(|v| <&str>::try_from(v).ok().map(str::to_string)).unwrap_or_default()
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app_name: &str,
        replaces_id: u32,
        _app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let mut s = self.0.lock().unwrap();
        let silent = hints.get("suppress-sound").map(|v| bool::try_from(v).unwrap_or(false)).unwrap_or(false);
        s.seen.push(Seen {
            replaces: replaces_id,
            summary: summary.into(),
            body: body.into(),
            actions,
            silent,
            entry: text(&hints, "desktop-entry"),
            category: text(&hints, "category"),
            sound: text(&hints, "sound-name"),
            urgency: hints.get("urgency").and_then(|v| u8::try_from(v).ok()),
            timeout: expire_timeout,
        });
        if replaces_id != 0 {
            return replaces_id;
        }
        s.next += 1;
        s.next
    }

    fn close_notification(&self, id: u32) {
        self.0.lock().unwrap().closed.push(id);
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into(), "body-markup".into()]
    }

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

/// How long a test waits for what it awaits; past it the test fails.
const WAIT: Duration = Duration::from_secs(30);

/// Waits until `done` holds: the bus and the notifier run on their own
/// tasks, and a busy machine may take its time; a deadline ends the wait
/// with a failure.
async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !done() {
        if tokio::time::Instant::now() >= deadline {
            panic!("{what}: not within {WAIT:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn app() -> AppInfo {
    AppInfo { id: "t".into(), name: "Veydan Space".into(), desktop_entry: "veydanspace".into(), icon: "veydanspace".into(), icon_file: None }
}

/// A notification server on a private bus, and what it was asked.
async fn server() -> (Bus, String, zbus::Connection, Arc<Mutex<State>>) {
    let (bus, address) = private_bus();
    let state = Arc::new(Mutex::new(State::default()));
    let conn = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.Notifications")
        .unwrap()
        .serve_at("/org/freedesktop/Notifications", Server(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    (bus, address, conn, state)
}

#[tokio::test(flavor = "multi_thread")]
async fn one_per_key_replaced_cleared_clicked() {
    let (_bus, address, server, state) = server().await;
    let (click_tx, mut click_rx) = mpsc::unbounded_channel();
    let n = Notifier::start_on_bus(app(), Handlers::clicks(Arc::new(move |k| { let _ = click_tx.send(k); })), address);
    until("the server is seen", || n.available()).await;

    let toast = |key: &str, body: &str, silent: bool| Toast {
        key: key.into(),
        title: "Alice".into(),
        body: body.into(),
        silent,
        ..Toast::default()
    };
    n.show(toast("dm:a", "a <b>", false));
    n.show(toast("dm:a", "a <b>\nsecond", true));
    n.show(toast("group:g", "hi", false));
    until("three notifications", || state.lock().unwrap().seen.len() >= 3).await;
    {
        let s = state.lock().unwrap();
        assert_eq!(s.seen.len(), 3);
        assert_eq!(s.seen[0].replaces, 0);
        assert_eq!(s.seen[0].body, "a &lt;b&gt;", "markup is escaped");
        assert_eq!(s.seen[0].actions, vec!["default".to_string(), String::new()]);
        assert_eq!(s.seen[0].entry, "veydanspace");
        assert_eq!((s.seen[0].category.as_str(), s.seen[0].sound.as_str()), ("im.received", "message-new-instant"));
        assert_eq!((s.seen[0].urgency, s.seen[0].timeout), (None, -1));
        assert!(!s.seen[0].silent);
        assert_eq!(s.seen[1].replaces, 1, "the same key replaces");
        assert!(s.seen[1].silent);
        assert_eq!(s.seen[2].replaces, 0, "another key is another notification");
    }

    n.clear("group:g");
    until("a close", || !state.lock().unwrap().closed.is_empty()).await;
    assert_eq!(state.lock().unwrap().closed, vec![2]);

    // A click on the chat's notification hands its key back.
    let emitter = SignalEmitter::new(&server, "/org/freedesktop/Notifications").unwrap();
    Server::action_invoked(&emitter, 1, "default").await.unwrap();
    let key = tokio::time::timeout(WAIT, click_rx.recv()).await.unwrap();
    assert_eq!(key.as_deref(), Some("dm:a"));

    // After the click the key starts a new notification.
    n.show(toast("dm:a", "again", false));
    until("the fourth notification", || state.lock().unwrap().seen.len() >= 4).await;
    assert_eq!(state.lock().unwrap().seen.len(), 4);
    assert_eq!(state.lock().unwrap().seen.last().unwrap().replaces, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_rings_until_a_button_is_pressed() {
    let (_bus, address, server, state) = server().await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (clicks, actions) = (tx.clone(), tx);
    let handlers = Handlers {
        on_click: Arc::new(move |k| { let _ = clicks.send(format!("click {k}")); }),
        on_action: Arc::new(move |k, id| { let _ = actions.send(format!("{id} {k}")); }),
    };
    let n = Notifier::start_on_bus(app(), handlers, address);
    until("the server is seen", || n.available()).await;

    let call = |key: &str| Toast {
        key: key.into(),
        title: "Alice".into(),
        body: "Incoming call".into(),
        kind: Kind::Call,
        actions: vec![Action::new("answer", "Answer"), Action::new("decline", "Decline")],
        ..Toast::default()
    };
    n.show(call("call:c1"));
    n.show(call("call:c2"));
    until("two calls shown", || state.lock().unwrap().seen.len() >= 2).await;
    {
        let s = state.lock().unwrap();
        let seen = &s.seen[0];
        assert_eq!(seen.actions, ["default", "", "answer", "Answer", "decline", "Decline"]);
        assert_eq!(seen.category, "call.incoming");
        assert_eq!(seen.sound, "phone-incoming-call");
        assert_eq!(seen.urgency, Some(2), "critical");
        assert_eq!(seen.timeout, 0, "it stays");
    }

    // Answer on the first: its key and the button come back, the
    // notification is closed; Decline on the second likewise.
    let emitter = SignalEmitter::new(&server, "/org/freedesktop/Notifications").unwrap();
    Server::action_invoked(&emitter, 1, "answer").await.unwrap();
    Server::action_invoked(&emitter, 2, "decline").await.unwrap();
    let mut got = Vec::new();
    for _ in 0..2 {
        got.push(tokio::time::timeout(WAIT, rx.recv()).await.unwrap().unwrap());
    }
    assert_eq!(got, ["answer call:c1", "decline call:c2"]);
    until("both closed", || state.lock().unwrap().closed.len() >= 2).await;
    assert_eq!(state.lock().unwrap().closed, vec![1, 2]);

    // A press on what is gone goes nowhere. The notifier takes the presses
    // in the order the server sent them, so a press on a call still shown,
    // sent right after, is the next thing it hands on: the press before it
    // was taken and led nowhere.
    n.show(call("call:c3"));
    until("the third call shown", || state.lock().unwrap().seen.len() >= 3).await;
    Server::action_invoked(&emitter, 1, "answer").await.unwrap();
    Server::action_invoked(&emitter, 3, "answer").await.unwrap();
    let next = tokio::time::timeout(WAIT, rx.recv()).await.unwrap().unwrap();
    assert_eq!(next, "answer call:c3", "the press on what is gone was handed on");
}

/// Waits until the bus has carried `count` asks for the capabilities of a
/// notification server, as a monitor of the bus sees them.
async fn asks_seen(monitor: &mut zbus::MessageStream, seen: &mut usize, count: usize) {
    use futures_util::StreamExt;
    while *seen < count {
        let message = tokio::time::timeout(WAIT, monitor.next())
            .await
            .unwrap_or_else(|_| panic!("ask {} not seen within {WAIT:?}", *seen + 1))
            .expect("the monitor stays")
            .expect("a message");
        let header = message.header();
        if header.message_type() == zbus::message::Type::MethodCall
            && header.member().is_some_and(|m| m.as_str() == "GetCapabilities")
        {
            *seen += 1;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn no_server_is_not_available() {
    let (_bus, address) = private_bus();
    // A monitor of the bus, there before the notifier starts, sees each of
    // its asks for the server.
    let monitor = zbus::connection::Builder::address(address.as_str()).unwrap().build().await.unwrap();
    zbus::fdo::MonitoringProxy::new(&monitor).await.unwrap().become_monitor(&[], 0).await.unwrap();
    let mut monitor = zbus::MessageStream::from(&monitor);
    let mut seen = 0;

    let app = AppInfo { id: "x".into(), name: "x".into(), desktop_entry: "x".into(), icon: "x".into(), icon_file: None };
    let n = Notifier::start_on_bus(app, Handlers::clicks(Arc::new(|_| {})), address);
    asks_seen(&mut monitor, &mut seen, 1).await;
    // With no server a show asks again, and the notifier asks only after it
    // took the answer to the ask before: once the second ask is on the bus,
    // the first was refused and told.
    n.show(Toast { key: "k".into(), title: "t".into(), body: "b".into(), ..Toast::default() });
    asks_seen(&mut monitor, &mut seen, 2).await;
    assert!(!n.available());
}

/// logind's session as a lock screen leaves it.
struct LoginSession(Arc<Mutex<bool>>);

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl LoginSession {
    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        *self.0.lock().unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_lock_of_the_session_is_asked_of_logind_each_time() {
    let (_bus, address) = private_bus();
    // No logind on the bus: not locked.
    assert_eq!(desktop_notify::session_locked_on_bus(&address).await, None, "no logind on the bus: absent, not silent");
    let locked = Arc::new(Mutex::new(false));
    let _logind = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1/session/auto", LoginSession(locked.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    assert_eq!(desktop_notify::session_locked_on_bus(&address).await, Some(false));
    *locked.lock().unwrap() = true;
    assert_eq!(desktop_notify::session_locked_on_bus(&address).await, Some(true), "a lock comes through at once, nothing is cached");
    *locked.lock().unwrap() = false;
    assert_eq!(desktop_notify::session_locked_on_bus(&address).await, Some(false));
}

/// A logind that fails to answer the property.
struct BrokenSession;

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl BrokenSession {
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::fdo::Result<bool> {
        Err(zbus::fdo::Error::Failed("busy".into()))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_logind_that_fails_counts_the_session_as_locked() {
    let (_bus, address) = private_bus();
    let _logind = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at("/org/freedesktop/login1/session/auto", BrokenSession)
        .unwrap()
        .build()
        .await
        .unwrap();
    assert_eq!(desktop_notify::session_locked_on_bus(&address).await, Some(true), "an error of the moment shows no text");
}

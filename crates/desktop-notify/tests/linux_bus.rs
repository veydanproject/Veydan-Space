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

async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
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
    settle().await;
    assert!(n.available());

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
    settle().await;
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
    settle().await;
    assert_eq!(state.lock().unwrap().closed, vec![2]);

    // A click on the chat's notification hands its key back.
    let emitter = SignalEmitter::new(&server, "/org/freedesktop/Notifications").unwrap();
    Server::action_invoked(&emitter, 1, "default").await.unwrap();
    let key = tokio::time::timeout(Duration::from_secs(2), click_rx.recv()).await.unwrap();
    assert_eq!(key.as_deref(), Some("dm:a"));

    // After the click the key starts a new notification.
    n.show(toast("dm:a", "again", false));
    settle().await;
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
    settle().await;

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
    settle().await;
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
        got.push(tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap());
    }
    assert_eq!(got, ["answer call:c1", "decline call:c2"]);
    settle().await;
    assert_eq!(state.lock().unwrap().closed, vec![1, 2]);

    // A press on what is gone goes nowhere.
    Server::action_invoked(&emitter, 1, "answer").await.unwrap();
    settle().await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_server_is_not_available() {
    let (_bus, address) = private_bus();
    let app = AppInfo { id: "x".into(), name: "x".into(), desktop_entry: "x".into(), icon: "x".into(), icon_file: None };
    let n = Notifier::start_on_bus(app, Handlers::clicks(Arc::new(|_| {})), address);
    settle().await;
    assert!(!n.available());
    n.show(Toast { key: "k".into(), title: "t".into(), body: "b".into(), ..Toast::default() });
    settle().await;
}

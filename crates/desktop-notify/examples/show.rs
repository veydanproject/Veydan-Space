//! Shows, replaces and clears a notification; prints the key of a click.
//!
//! `cargo run --example show` and click the notification within 20 s.
//! `cargo run --example show -- call`: a ringing call with two buttons;
//! prints the press, or takes the call away after 30 s.

use std::sync::Arc;
use std::time::Duration;

use desktop_notify::{Action, AppInfo, Handlers, Kind, Notifier, Toast};

#[tokio::main]
async fn main() {
    let app = AppInfo {
        id: "net.veydan.space.example".into(),
        name: "Veydan Space".into(),
        desktop_entry: "veydanspace".into(),
        icon: "veydanspace".into(),
        icon_file: None,
    };
    let handlers = Handlers {
        on_click: Arc::new(|key| println!("clicked: {key}")),
        on_action: Arc::new(|key, id| println!("pressed: {id} on {key}")),
    };
    let n = Notifier::start(app, handlers);
    tokio::time::sleep(Duration::from_millis(300)).await;
    println!("available: {}", n.available());

    if std::env::args().nth(1).as_deref() == Some("call") {
        n.show(Toast {
            key: "call:example".into(),
            title: "Alice".into(),
            body: "Incoming call".into(),
            kind: Kind::Call,
            actions: vec![Action::new("answer", "Answer"), Action::new("decline", "Decline")],
            ..Toast::default()
        });
        tokio::time::sleep(Duration::from_secs(30)).await;
        n.clear("call:example");
        tokio::time::sleep(Duration::from_millis(300)).await;
        return;
    }

    let mut toast = Toast {
        key: "dm:alice".into(),
        title: "Alice".into(),
        body: "first <line> & more".into(),
        ..Toast::default()
    };
    n.show(toast.clone());
    tokio::time::sleep(Duration::from_secs(2)).await;
    toast.body = "first <line> & more\nsecond line".into();
    toast.silent = true;
    n.show(toast);
    n.show(Toast {
        key: "group:x".into(),
        title: "Group X".into(),
        body: "Bob: hi".into(),
        ..Toast::default()
    });
    tokio::time::sleep(Duration::from_secs(4)).await;
    n.clear("group:x");
    println!("group cleared; waiting for a click on Alice");
    tokio::time::sleep(Duration::from_secs(20)).await;
    n.clear_all();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

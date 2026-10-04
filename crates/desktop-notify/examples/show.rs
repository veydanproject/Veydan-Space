//! Shows, replaces and clears a notification; prints the key of a click.
//!
//! `cargo run --example show` and click the notification within 20 s.

use std::sync::Arc;
use std::time::Duration;

use desktop_notify::{AppInfo, Notifier, Toast};

#[tokio::main]
async fn main() {
    let app = AppInfo {
        id: "net.veydan.space.example".into(),
        name: "Veydan Space".into(),
        desktop_entry: "veydanspace".into(),
        icon: "veydanspace".into(),
        icon_file: None,
    };
    let n = Notifier::start(app, Arc::new(|key| println!("clicked: {key}")));
    tokio::time::sleep(Duration::from_millis(300)).await;
    println!("available: {}", n.available());

    let mut toast = Toast {
        key: "dm:alice".into(),
        title: "Alice".into(),
        body: "first <line> & more".into(),
        image: None,
        silent: false,
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
        image: None,
        silent: false,
    });
    tokio::time::sleep(Duration::from_secs(4)).await;
    n.clear("group:x");
    println!("group cleared; waiting for a click on Alice");
    tokio::time::sleep(Duration::from_secs(20)).await;
    n.clear_all();
    tokio::time::sleep(Duration::from_millis(300)).await;
}

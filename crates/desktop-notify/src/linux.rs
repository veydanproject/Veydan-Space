//! `org.freedesktop.Notifications` over the session bus.

use std::collections::HashMap;

use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use zbus::zvariant::Value;

use crate::{escape, AppInfo, Command, OnClick, Toast};

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_blocking = false
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// What the server can do, asked once it answers.
#[derive(Default)]
struct Caps {
    actions: bool,
    markup: bool,
}

/// Which notification on the screen belongs to which key.
#[derive(Default)]
struct Shown {
    by_key: HashMap<String, u32>,
}

impl Shown {
    fn key_of(&self, id: u32) -> Option<&String> {
        self.by_key.iter().find(|(_, v)| **v == id).map(|(k, _)| k)
    }

    fn forget_id(&mut self, id: u32) {
        self.by_key.retain(|_, v| *v != id);
    }
}

pub(crate) async fn run(
    app: AppInfo,
    mut rx: mpsc::UnboundedReceiver<Command>,
    up: watch::Sender<bool>,
    on_click: OnClick,
    bus: Option<String>,
) {
    let proxy = match connect(bus).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("desktop-notify: no session bus: {e}");
            crate::drain(rx).await;
            return;
        }
    };
    let (Ok(mut clicks), Ok(mut closed)) =
        (proxy.receive_action_invoked().await, proxy.receive_notification_closed().await)
    else {
        eprintln!("desktop-notify: cannot listen to the notification server");
        crate::drain(rx).await;
        return;
    };

    let mut caps: Option<Caps> = None;
    let mut shown = Shown::default();
    ask(&proxy, &mut caps, &up).await;

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                // A daemon started on demand may appear later: ask again.
                if caps.is_none() {
                    ask(&proxy, &mut caps, &up).await;
                }
                let Some(c) = caps.as_ref() else { continue };
                match cmd {
                    Command::Show(t) => show(&proxy, &app, c, &mut shown, t).await,
                    Command::Clear(key) => {
                        if let Some(id) = shown.by_key.remove(&key) {
                            let _ = proxy.close_notification(id).await;
                        }
                    }
                    Command::ClearAll => {
                        for (_, id) in shown.by_key.drain() {
                            let _ = proxy.close_notification(id).await;
                        }
                    }
                    Command::Shutdown(done) => {
                        for (_, id) in shown.by_key.drain() {
                            let _ = proxy.close_notification(id).await;
                        }
                        let _ = done.send(());
                    }
                }
            }
            Some(sig) = clicks.next() => {
                let Ok(args) = sig.args() else { continue };
                if args.action_key != "default" { continue }
                if let Some(key) = shown.key_of(args.id).cloned() {
                    shown.forget_id(args.id);
                    let _ = proxy.close_notification(args.id).await;
                    on_click(key);
                }
            }
            Some(sig) = closed.next() => {
                if let Ok(args) = sig.args() {
                    shown.forget_id(args.id);
                }
            }
        }
    }
}

async fn connect(bus: Option<String>) -> zbus::Result<NotificationsProxy<'static>> {
    let conn = match bus {
        Some(address) => zbus::connection::Builder::address(address.as_str())?.build().await?,
        None => zbus::Connection::session().await?,
    };
    NotificationsProxy::new(&conn).await
}

async fn ask(proxy: &NotificationsProxy<'_>, caps: &mut Option<Caps>, up: &watch::Sender<bool>) {
    match proxy.get_capabilities().await {
        Ok(list) => {
            *caps = Some(Caps {
                actions: list.iter().any(|c| c == "actions"),
                markup: list.iter().any(|c| c == "body-markup"),
            });
            let _ = up.send(true);
        }
        Err(e) => {
            eprintln!("desktop-notify: no notification server: {e}");
            let _ = up.send(false);
        }
    }
}

async fn show(proxy: &NotificationsProxy<'_>, app: &AppInfo, caps: &Caps, shown: &mut Shown, t: Toast) {
    let replaces = shown.by_key.get(&t.key).copied().unwrap_or(0);
    let body = if caps.markup { escape(&t.body) } else { t.body.clone() };
    let actions: &[&str] = if caps.actions { &["default", ""] } else { &[] };
    let image = t.image.as_ref().map(|p| format!("file://{}", p.display()));

    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(app.desktop_entry.as_str()));
    hints.insert("category", Value::from("im.received"));
    if let Some(img) = image.as_deref() {
        hints.insert("image-path", Value::from(img));
    }
    if t.silent {
        hints.insert("suppress-sound", Value::from(true));
    } else {
        hints.insert("sound-name", Value::from("message-new-instant"));
    }

    match proxy
        .notify(&app.name, replaces, &app.icon, &t.title, &body, actions, hints, -1)
        .await
    {
        Ok(id) => {
            shown.forget_id(id);
            shown.by_key.insert(t.key, id);
        }
        Err(e) => eprintln!("desktop-notify: notify failed: {e}"),
    }
}

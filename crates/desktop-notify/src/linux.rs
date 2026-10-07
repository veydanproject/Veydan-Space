//! `org.freedesktop.Notifications` over the session bus.

use std::collections::HashMap;

use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use zbus::zvariant::Value;

use crate::{escape, AppInfo, Command, Handlers, Kind, Press, Toast};

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
    handlers: Handlers,
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
            // A click (`default`) or a button: the notification has done
            // its work either way, and a call's would otherwise stay.
            Some(sig) = clicks.next() => {
                let Ok(args) = sig.args() else { continue };
                if let Some(key) = shown.key_of(args.id).cloned() {
                    shown.forget_id(args.id);
                    let _ = proxy.close_notification(args.id).await;
                    Press::of(key, &args.action_key).deliver(&handlers);
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

/// The urgency of a call (the spec's `critical`): the server keeps it on
/// the screen and lets it through "do not disturb" where it can.
const URGENCY_CRITICAL: u8 = 2;

/// What goes into one `Notify` beside the names.
struct Request {
    body: String,
    /// Pairs of an id and its label; `default` is a click on the
    /// notification, with no button of its own.
    actions: Vec<String>,
    hints: HashMap<&'static str, Value<'static>>,
    /// Milliseconds; -1 the server decides, 0 never.
    timeout: i32,
}

fn request(app: &AppInfo, caps: &Caps, t: &Toast) -> Request {
    let body = if caps.markup { escape(&t.body) } else { t.body.clone() };
    let mut actions = Vec::new();
    if caps.actions {
        actions.extend(["default".to_string(), String::new()]);
        for a in &t.actions {
            actions.extend([a.id.clone(), a.label.clone()]);
        }
    }

    let mut hints: HashMap<&'static str, Value<'static>> = HashMap::new();
    hints.insert("desktop-entry", Value::from(app.desktop_entry.clone()));
    if let Some(img) = &t.image {
        hints.insert("image-path", Value::from(format!("file://{}", img.display())));
    }
    let (category, sound) = match t.kind {
        Kind::Message => ("im.received", "message-new-instant"),
        Kind::Call => {
            hints.insert("urgency", Value::from(URGENCY_CRITICAL));
            ("call.incoming", "phone-incoming-call")
        }
    };
    hints.insert("category", Value::from(category));
    if t.silent {
        hints.insert("suppress-sound", Value::from(true));
    } else {
        hints.insert("sound-name", Value::from(sound));
    }
    let timeout = if t.kind == Kind::Call { 0 } else { -1 };
    Request { body, actions, hints, timeout }
}

async fn show(proxy: &NotificationsProxy<'_>, app: &AppInfo, caps: &Caps, shown: &mut Shown, t: Toast) {
    let replaces = shown.by_key.get(&t.key).copied().unwrap_or(0);
    let r = request(app, caps, &t);
    let actions: Vec<&str> = r.actions.iter().map(String::as_str).collect();
    match proxy
        .notify(&app.name, replaces, &app.icon, &t.title, &r.body, &actions, r.hints, r.timeout)
        .await
    {
        Ok(id) => {
            shown.forget_id(id);
            shown.by_key.insert(t.key, id);
        }
        Err(e) => eprintln!("desktop-notify: notify failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Action;

    fn app() -> AppInfo {
        AppInfo { id: "t".into(), name: "Veydan Chat".into(), desktop_entry: "veydanchat".into(), icon: "i".into(), icon_file: None }
    }

    fn str_of<'a>(r: &'a Request, hint: &str) -> Option<&'a str> {
        r.hints.get(hint).and_then(|v| <&str>::try_from(v).ok())
    }

    fn call() -> Toast {
        Toast {
            key: "call:c1".into(),
            title: "Alice".into(),
            body: "Incoming call".into(),
            image: Some("/tmp/a.png".into()),
            kind: Kind::Call,
            actions: vec![Action::new("answer", "Answer"), Action::new("decline", "Decline")],
            ..Toast::default()
        }
    }

    #[test]
    fn a_call_is_critical_rings_and_stays() {
        let caps = Caps { actions: true, markup: true };
        let r = request(&app(), &caps, &call());
        assert_eq!(r.actions, ["default", "", "answer", "Answer", "decline", "Decline"]);
        assert_eq!(r.timeout, 0, "no timeout");
        assert_eq!(r.hints.get("urgency").and_then(|v| u8::try_from(v).ok()), Some(2));
        assert_eq!(str_of(&r, "category"), Some("call.incoming"));
        assert_eq!(str_of(&r, "sound-name"), Some("phone-incoming-call"));
        assert_eq!(str_of(&r, "image-path"), Some("file:///tmp/a.png"));
        assert_eq!(str_of(&r, "desktop-entry"), Some("veydanchat"));

        let quiet = request(&app(), &caps, &Toast { silent: true, ..call() });
        assert!(!quiet.hints.contains_key("sound-name"));
        assert_eq!(quiet.hints.get("suppress-sound").and_then(|v| bool::try_from(v).ok()), Some(true));
    }

    #[test]
    fn a_message_is_as_it_was() {
        let caps = Caps { actions: true, markup: true };
        let t = Toast { key: "dm:a".into(), title: "A".into(), body: "<b>".into(), ..Toast::default() };
        let r = request(&app(), &caps, &t);
        assert_eq!(r.body, "&lt;b&gt;");
        assert_eq!(r.actions, ["default", ""]);
        assert_eq!(r.timeout, -1);
        assert!(!r.hints.contains_key("urgency"));
        assert_eq!(str_of(&r, "category"), Some("im.received"));
        assert_eq!(str_of(&r, "sound-name"), Some("message-new-instant"));
    }

    #[test]
    fn a_server_without_buttons_gets_none() {
        let r = request(&app(), &Caps { actions: false, markup: false }, &call());
        assert!(r.actions.is_empty());
        assert_eq!(r.body, "Incoming call");
    }
}

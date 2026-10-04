//! WinRT toasts.
//!
//! A toast belongs to an AppUserModelID. The installers put one on the
//! Start menu shortcut; a build without one (dev, portable, a second
//! profile with its own identifier) registers it for the current user
//! under `HKCU\Software\Classes\AppUserModelId`, which is all Windows needs
//! to show and to click toasts of a running app.
//!
//! WinRT is called from a thread of its own that joined the multithreaded
//! apartment; a click comes on a thread of the system's pool.

use tokio::sync::{mpsc, watch};
use windows::core::{HSTRING, IInspectable};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

use crate::toast_xml::toast_xml;
use crate::{short_tag, AppInfo, Command, OnClick, Toast};

/// All toasts of the app share one group; the tag tells the chats apart.
const GROUP: &str = "messages";

pub(crate) fn start(app: AppInfo, rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, on_click: OnClick) {
    let spawned = std::thread::Builder::new()
        .name("desktop-notify".into())
        .spawn(move || run(app, rx, up, on_click));
    if let Err(e) = spawned {
        eprintln!("desktop-notify: no thread: {e}");
    }
}

fn run(app: AppInfo, mut rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, on_click: OnClick) {
    // SAFETY: once per thread, before any WinRT call on it.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    if let Err(e) = register(&app) {
        eprintln!("desktop-notify: the app id was not registered: {e}");
    }
    let aumid = HSTRING::from(app.id.as_str());
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&aumid) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("desktop-notify: no toast notifier: {e}");
            let _ = up.send(false);
            while let Some(cmd) = rx.blocking_recv() {
                if let Command::Shutdown(done) = cmd {
                    let _ = done.send(());
                }
            }
            return;
        }
    };
    let _ = up.send(true);

    while let Some(cmd) = rx.blocking_recv() {
        let done = match cmd {
            Command::Show(t) => show(&notifier, &t, &on_click),
            Command::Clear(key) => clear(&aumid, &key),
            Command::ClearAll => clear_all(&aumid),
            Command::Shutdown(done) => {
                let r = clear_all(&aumid);
                let _ = done.send(());
                r
            }
        };
        if let Err(e) = done {
            eprintln!("desktop-notify: {e}");
        }
    }
}

/// The name and icon Windows shows for toasts of this app id; and the id
/// of this process, so the taskbar and the toasts agree.
fn register(app: &AppInfo) -> windows::core::Result<()> {
    let key = windows_registry::CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{}", app.id))?;
    key.set_string("DisplayName", &app.name)?;
    if let Some(icon) = &app.icon_file {
        key.set_string("IconUri", icon.to_string_lossy().as_ref())?;
    }
    // SAFETY: a plain call with a valid string; before any window is shown.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(app.id.as_str())) }
}

fn show(notifier: &ToastNotifier, t: &Toast, on_click: &OnClick) -> windows::core::Result<()> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(toast_xml(t)))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    toast.SetTag(&HSTRING::from(short_tag(&t.key)))?;
    toast.SetGroup(&HSTRING::from(GROUP))?;
    let key = t.key.clone();
    let on_click = on_click.clone();
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
        on_click(key.clone());
        Ok(())
    }))?;
    notifier.Show(&toast)
}

fn clear(aumid: &HSTRING, key: &str) -> windows::core::Result<()> {
    ToastNotificationManager::History()?.RemoveGroupedTagWithId(
        &HSTRING::from(short_tag(key)),
        &HSTRING::from(GROUP),
        aumid,
    )
}

fn clear_all(aumid: &HSTRING) -> windows::core::Result<()> {
    ToastNotificationManager::History()?.ClearWithId(aumid)
}


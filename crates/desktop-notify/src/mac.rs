//! `UNUserNotificationCenter`.
//!
//! It works for an app bundle only: a binary run by itself (`tauri dev`)
//! has no bundle identifier, and the center would throw. Then nothing is
//! shown and the notifier says it is not available.
//!
//! The permission is asked for with the first notification, not at start:
//! a user who never gets a message is never asked. A notification's
//! identifier is its key, so a chat that writes again replaces its own;
//! the key is also the thread, so the system groups a chat's messages.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_foundation::{NSArray, NSBundle, NSError, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAttachment,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tokio::sync::{mpsc, watch};

use crate::{AppInfo, Command, OnClick, Toast};

struct Ivars {
    on_click: OnClick,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "VeydanDesktopNotifyDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// A click: the notification's identifier is the key.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let key = response.notification().request().identifier().to_string();
            (self.ivars().on_click)(key);
            completion.call(());
        }

        /// The app is active (another of its windows has the focus) and
        /// the caller still chose the system: show it as if it were not.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }
    }
);

impl Delegate {
    fn new(on_click: OnClick) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { on_click });
        // SAFETY: NSObject's plain init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn start(app: AppInfo, rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, on_click: OnClick) {
    let spawned = std::thread::Builder::new()
        .name("desktop-notify".into())
        .spawn(move || run(app, rx, up, on_click));
    if let Err(e) = spawned {
        eprintln!("desktop-notify: no thread: {e}");
    }
}

fn run(_app: AppInfo, mut rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, on_click: OnClick) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("desktop-notify: not an app bundle, notifications are off");
        drain_blocking(rx);
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    // The center keeps its delegate weakly: this one lives as long as the thread.
    let delegate = Delegate::new(on_click);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let _ = up.send(true);

    let mut asked = false;
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            Command::Show(t) => {
                if !asked {
                    asked = true;
                    ask_permission(&center, up.clone());
                }
                show(&center, &t);
            }
            Command::Clear(key) => {
                center.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(&[NSString::from_str(&key)]));
            }
            Command::ClearAll => center.removeAllDeliveredNotifications(),
            Command::Shutdown(done) => {
                center.removeAllDeliveredNotifications();
                let _ = done.send(());
            }
        }
    }
    drop(delegate);
}

fn drain_blocking(mut rx: mpsc::UnboundedReceiver<Command>) {
    while let Some(cmd) = rx.blocking_recv() {
        if let Command::Shutdown(done) = cmd {
            let _ = done.send(());
        }
    }
}

/// Asks once; a refusal makes the notifier unavailable, so the app keeps
/// the card in the window and the settings can say why.
fn ask_permission(center: &UNUserNotificationCenter, up: watch::Sender<bool>) {
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
    let handler = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        if !granted.as_bool() {
            eprintln!("desktop-notify: notifications are not allowed in the system settings");
            let _ = up.send(false);
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(options, &handler);
}

fn show(center: &UNUserNotificationCenter, t: &Toast) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&t.title));
    content.setBody(&NSString::from_str(&t.body));
    content.setThreadIdentifier(&NSString::from_str(&t.key));
    if !t.silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    if let Some(attachment) = t.image.as_deref().and_then(attachment) {
        content.setAttachments(&NSArray::from_retained_slice(&[attachment]));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&t.key), &content, None);
    let handler = RcBlock::new(|error: *mut NSError| {
        // SAFETY: the center passes a valid error or null.
        if let Some(e) = unsafe { error.as_ref() } {
            eprintln!("desktop-notify: not shown: {}", e.localizedDescription());
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&handler));
}

/// The system moves an attached file into its own store and tells its
/// type by the extension: a copy with the right one is attached.
fn attachment(image: &Path) -> Option<Retained<UNNotificationAttachment>> {
    let copy = copy_with_extension(image)?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&copy.to_string_lossy()));
    // SAFETY: no options dictionary.
    unsafe { UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(&NSString::from_str("face"), &url, None) }.ok()
}

fn copy_with_extension(image: &Path) -> Option<PathBuf> {
    static N: AtomicU64 = AtomicU64::new(0);
    let bytes = std::fs::read(image).ok()?;
    let ext = crate::image_extension(&bytes)?;
    let path = std::env::temp_dir().join(format!(
        "veydan-notify-{}-{}.{ext}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

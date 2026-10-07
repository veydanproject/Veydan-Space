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
//!
//! Buttons belong to a category the app registers with the center; a
//! notification names its category. A category is made for each set of
//! buttons (their ids and labels) the first time one is shown, and all of
//! them are registered again together, as the center takes the whole set.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass};
use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString, NSURL};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationAttachment, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use tokio::sync::{mpsc, watch};

use crate::{short_tag, Action, AppInfo, Command, Handlers, Press, Toast};

struct Ivars {
    handlers: Handlers,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "VeydanDesktopNotifyDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// A click or a button: the notification's identifier is the key,
        /// the action's identifier the button's id.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let key = response.notification().request().identifier();
            let action = response.actionIdentifier();
            // SAFETY: constant strings of the framework.
            let (default, dismiss) = unsafe { (UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier) };
            if *action != *dismiss {
                let id = if *action == *default { String::new() } else { action.to_string() };
                if !id.is_empty() {
                    // A button leaves the notification in the list: it is done.
                    center.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_retained_slice(std::slice::from_ref(&key)));
                }
                Press::of(key.to_string(), &id).deliver(&self.ivars().handlers);
            }
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
    fn new(handlers: Handlers) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { handlers });
        // SAFETY: NSObject's plain init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) fn start(app: AppInfo, rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, handlers: Handlers) {
    let spawned = std::thread::Builder::new()
        .name("desktop-notify".into())
        .spawn(move || run(app, rx, up, handlers));
    if let Err(e) = spawned {
        eprintln!("desktop-notify: no thread: {e}");
    }
}

fn run(_app: AppInfo, mut rx: mpsc::UnboundedReceiver<Command>, up: watch::Sender<bool>, handlers: Handlers) {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("desktop-notify: not an app bundle, notifications are off");
        drain_blocking(rx);
        return;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    // The center keeps its delegate weakly: this one lives as long as the thread.
    let delegate = Delegate::new(handlers);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let _ = up.send(true);

    let mut asked = false;
    let mut categories = Categories::default();
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            Command::Show(t) => {
                if !asked {
                    asked = true;
                    ask_permission(&center, up.clone());
                }
                let category = categories.of(&center, &t.actions);
                show(&center, &t, category.as_deref());
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

/// The categories registered so far: their identifiers and their buttons.
#[derive(Default)]
struct Categories {
    known: Vec<(String, Retained<UNNotificationCategory>)>,
}

impl Categories {
    /// The category of these buttons, registered when it is new; None for
    /// a notification without buttons.
    fn of(&mut self, center: &UNUserNotificationCenter, actions: &[Action]) -> Option<String> {
        if actions.is_empty() {
            return None;
        }
        let id = category_id(actions);
        if self.known.iter().any(|(k, _)| *k == id) {
            return Some(id);
        }
        let buttons: Vec<Retained<UNNotificationAction>> = actions
            .iter()
            .map(|a| {
                UNNotificationAction::actionWithIdentifier_title_options(
                    &NSString::from_str(&a.id),
                    &NSString::from_str(&a.label),
                    UNNotificationActionOptions::empty(),
                )
            })
            .collect();
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(&id),
            &NSArray::from_retained_slice(&buttons),
            &NSArray::from_retained_slice(&[]),
            UNNotificationCategoryOptions::empty(),
        );
        self.known.push((id.clone(), category));
        let all: Vec<Retained<UNNotificationCategory>> = self.known.iter().map(|(_, c)| c.clone()).collect();
        center.setNotificationCategories(&NSSet::from_retained_slice(&all));
        Some(id)
    }
}

/// The same buttons with the same words are one category.
fn category_id(actions: &[Action]) -> String {
    let joined: String = actions.iter().map(|a| format!("{}\u{1f}{}\u{1e}", a.id, a.label)).collect();
    format!("actions-{}", short_tag(&joined))
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

/// A call sounds as any notification does (the default sound): the ring
/// itself is the app's.
fn show(center: &UNUserNotificationCenter, t: &Toast, category: Option<&str>) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&t.title));
    content.setBody(&NSString::from_str(&t.body));
    content.setThreadIdentifier(&NSString::from_str(&t.key));
    if let Some(category) = category {
        content.setCategoryIdentifier(&NSString::from_str(category));
    }
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

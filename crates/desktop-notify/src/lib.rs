//! Notifications of the desktop systems.
//!
//! One notification per `key` (a chat): showing a key again replaces what
//! is on the screen, clearing a key takes it away, a click hands the key
//! back. Knows nothing about the messenger: the caller words the text.
//!
//! - Linux: `org.freedesktop.Notifications` over D-Bus.
//! - Windows: WinRT toasts under the app's AppUserModelID, registered for
//!   the current user so that a build without an installer shows them too.
//! - macOS: `UNUserNotificationCenter`, from an app bundle only.
//!
//! Where the system cannot show anything (no notification daemon, no app
//! bundle on macOS), the notifier says so through [`Notifier::available`]
//! and the calls do nothing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;
#[cfg(any(windows, test))]
mod toast_xml;

/// Who is notifying.
#[derive(Clone, Debug)]
pub struct AppInfo {
    /// The app's identifier (Windows: the AppUserModelID).
    pub id: String,
    /// The name the system shows as the source.
    pub name: String,
    /// The desktop entry without `.desktop` (Linux: icon, grouping).
    pub desktop_entry: String,
    /// An icon name or an absolute path (Linux).
    pub icon: String,
    /// The app's icon as a file (Windows: shown as the source).
    pub icon_file: Option<PathBuf>,
}

/// One notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// What it is about; the same key replaces, a click returns it.
    pub key: String,
    pub title: String,
    /// Plain text, lines split by `\n`. Markup is escaped here.
    pub body: String,
    /// A picture of who wrote (a local file).
    pub image: Option<PathBuf>,
    /// No sound.
    pub silent: bool,
}

pub type OnClick = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) enum Command {
    Show(Toast),
    Clear(String),
    ClearAll,
    /// Everything taken away, then the answer: the app is about to quit.
    Shutdown(oneshot::Sender<()>),
}

/// Handle to the notifications; cheap to clone, calls never block.
#[derive(Clone)]
pub struct Notifier {
    tx: mpsc::UnboundedSender<Command>,
    available: watch::Receiver<bool>,
}

impl Notifier {
    /// Starts the notifier on the current tokio runtime.
    pub fn start(app: AppInfo, on_click: OnClick) -> Self {
        Self::spawn(app, on_click, None)
    }

    /// Linux, tests: the notification server on a bus of its own.
    #[doc(hidden)]
    #[cfg(target_os = "linux")]
    pub fn start_on_bus(app: AppInfo, on_click: OnClick, address: String) -> Self {
        Self::spawn(app, on_click, Some(address))
    }

    fn spawn(app: AppInfo, on_click: OnClick, bus: Option<String>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (up, available) = watch::channel(false);
        #[cfg(target_os = "linux")]
        tokio::spawn(linux::run(app, rx, up, on_click, bus));
        #[cfg(target_os = "macos")]
        {
            let _ = bus;
            mac::start(app, rx, up, on_click);
        }
        #[cfg(windows)]
        {
            let _ = bus;
            win::start(app, rx, up, on_click);
        }
        #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
        {
            let _ = (app, on_click, up, bus);
            tokio::spawn(drain(rx));
        }
        Self { tx, available }
    }

    /// Whether the system shows notifications at all.
    pub fn available(&self) -> bool {
        *self.available.borrow()
    }

    pub fn show(&self, toast: Toast) {
        let _ = self.tx.send(Command::Show(toast));
    }

    pub fn clear(&self, key: &str) {
        let _ = self.tx.send(Command::Clear(key.to_string()));
    }

    pub fn clear_all(&self) {
        let _ = self.tx.send(Command::ClearAll);
    }

    /// Takes every notification away and waits for it (up to a second): a
    /// notification left behind by a closed app leads nowhere when clicked.
    /// Blocks; call it on the way out, outside the async runtime.
    pub fn shutdown(&self) {
        let (done, wait) = oneshot::channel();
        if self.tx.send(Command::Shutdown(done)).is_err() {
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut wait = wait;
        while std::time::Instant::now() < deadline {
            match wait.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(10)),
                _ => return,
            }
        }
    }
}

/// Nothing can be shown: commands are taken and dropped, a shutdown answered.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
pub(crate) async fn drain(mut rx: mpsc::UnboundedReceiver<Command>) {
    while let Some(cmd) = rx.recv().await {
        if let Command::Shutdown(done) = cmd {
            let _ = done.send(());
        }
    }
}

/// `&`, `<`, `>` as entities: some servers read the body as markup.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// A short, stable name for a key where the system limits its length
/// (Windows: a toast's tag, at most 64 characters). FNV-1a, 64 bits.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn short_tag(key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The extension of a picture by its first bytes, for systems that tell
/// a file's type by its name (macOS). None: not a picture they show.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_told_by_their_first_bytes() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_extension(b"<svg"), None);
    }

    #[test]
    fn escapes_markup() {
        assert_eq!(escape("a<b>&c"), "a&lt;b&gt;&amp;c");
        assert_eq!(escape("привет"), "привет");
    }

    #[test]
    fn short_tags_are_short_and_stable() {
        let chat = format!("dm:{}", "ab".repeat(32));
        let tag = short_tag(&chat);
        assert_eq!(tag.len(), 16);
        assert_eq!(tag, short_tag(&chat));
        assert_ne!(tag, short_tag("dm:other"));
    }
}

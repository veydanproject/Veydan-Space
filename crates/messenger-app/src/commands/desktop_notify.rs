// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Notifications of a computer. What `Notifier.kt` is to the phone.
//!
//! No push is involved: the app runs, the session takes a message in and
//! raises a `notify` UiEvent (own, old, deleted messages and muted chats
//! are already left out). Here that becomes one of two things:
//!
//! - the window is on the screen and in focus: the card in the app says it
//!   (the event goes on to the page as it is);
//! - otherwise: a notification of the system, and the event is marked
//!   `os: true`, so the page shows no card when the window comes back.
//!
//! One notification per chat. A chat that writes again replaces it with the
//! last lines and a "+N"; reading the chat takes it away; a click brings
//! the window up and opens the chat.
//!
//! A call that rings while the window is away rings in a notification too
//! (`call.incoming`), with the caller's name and face as a message would
//! have them and the buttons Answer and Decline; it goes when the call is
//! answered or over (`call.state`, `call.ended`), here or on another device.
//! The ring of the app itself is the page's.
//!
//! The notifier lives as long as the process: a messenger that is switched
//! off has no runtime to raise a `notify`, and its notifications are taken
//! away when it stops.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use desktop_notify::{Action, AppInfo, Handlers, Kind, Notifier, Toast};
use messenger_notify::{Body, ChatKind, DesktopSettings, LinkKind, Outcome};
use messenger_runtime::{CallMedia, CallPhase, CallView, MessengerRuntime, UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_STATE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};

use super::MessengerState;
use veydan_core::{AppError, CmdResult};
use veydan_shell::{Product, Shell};

/// Emitted with `{chat}` when a notification is clicked.
pub const EVENT_NOTICE_TAP: &str = "messenger://notice-tap";

/// Lines of a chat's notification; older ones become "+N".
const MAX_LINES: usize = 5;
/// The key of notices that name no chat.
const NO_CHAT: &str = "dm";

const AVATAR_TIMEOUT: Duration = Duration::from_millis(2500);
const AVATAR_MAX_BYTES: usize = 2 * 1024 * 1024;

/// The words of the user's language. The page sends them (`i18n.ts` holds
/// every string of the app); these are the words until it does.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Words {
    /// The name of the product (`Product.name`), the title of a notification
    /// that has none. What the page sends here is not taken.
    pub app: String,
    pub new_message: String,
    /// `{n}` is the number.
    pub new_messages: String,
    /// `{n}` is the number.
    pub more: String,
    pub request: String,
    pub group_invite: String,
    pub group_request: String,
    pub group_welcome: String,
    pub photo: String,
    pub video: String,
    pub voice: String,
    pub circle: String,
    pub audio: String,
    pub file: String,
    /// Pictures and videos sent together; `{n}` is how many.
    pub album: String,
    /// Files sent together; `{n}` is how many.
    pub files: String,
    /// `{name}` is the group's.
    pub link_group: String,
    pub link_group_nameless: String,
    /// `{name}` is the person's.
    pub link_contact: String,
    pub link_contact_nameless: String,
    /// The body of a ringing call's notification.
    pub call_audio: String,
    pub call_video: String,
    /// Its buttons.
    pub call_answer: String,
    pub call_decline: String,
}

impl Default for Words {
    fn default() -> Self {
        Self {
            app: String::new(),
            new_message: "New message".into(),
            new_messages: "{n} new messages".into(),
            more: "+{n} more".into(),
            request: "Message request".into(),
            group_invite: "Invites you to a group".into(),
            group_request: "Asks to join the group".into(),
            group_welcome: "You are in the group".into(),
            photo: "Photo".into(),
            video: "Video".into(),
            voice: "Voice message".into(),
            circle: "Video message".into(),
            audio: "Audio".into(),
            file: "File".into(),
            album: "Album: {n}".into(),
            files: "Files: {n}".into(),
            link_group: "Group “{name}”".into(),
            link_group_nameless: "Link to a group".into(),
            link_contact: "Contact: {name}".into(),
            link_contact_nameless: "Contact".into(),
            call_audio: "Incoming call".into(),
            call_video: "Incoming video call".into(),
            call_answer: "Answer".into(),
            call_decline: "Decline".into(),
        }
    }
}

impl Words {
    /// The English words, with the app named as the product is.
    pub fn named(app: &str) -> Self {
        Self { app: app.into(), ..Self::default() }
    }

    /// The words the page sent, the product still named as it is.
    fn with_page(self, page: Words) -> Self {
        Self { app: self.app, ..page }
    }
}

/// `12400` → `0:12`; an hour and more as `1:02:03`.
fn duration(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Files are told by their name; a picture, a video, a recording by what it is.
fn named(kind: &str) -> bool {
    kind == "file" || kind == "audio"
}

/// One message in the user's words: the same line the card in the app
/// makes (`push/wording.ts`) and the phone (`Notifier.kt`).
fn body_line(body: &Body, words: &Words) -> String {
    match body {
        Body::Text { text } => text.clone(),
        Body::Link { link: LinkKind::Group, title } if title.is_empty() => format!("🔗 {}", words.link_group_nameless),
        Body::Link { link: LinkKind::Group, title } => format!("🔗 {}", words.link_group.replace("{name}", title)),
        Body::Link { link: LinkKind::Contact, title } if title.is_empty() => format!("👤 {}", words.link_contact_nameless),
        Body::Link { link: LinkKind::Contact, title } => format!("👤 {}", words.link_contact.replace("{name}", title)),
        Body::Link { link: LinkKind::Web, title } => format!("🔗 {title}"),
        Body::Media { kind, name, caption, duration_ms, .. } => {
            let (emoji, word) = match kind.as_str() {
                "image" => ("📷", &words.photo),
                "video" => ("🎬", &words.video),
                "voice" => ("🎤", &words.voice),
                "circle" => ("⭕", &words.circle),
                "audio" => ("🎵", &words.audio),
                _ => ("📎", &words.file),
            };
            if named(kind) {
                let name = if name.is_empty() { word } else { name };
                match caption {
                    Some(caption) => format!("{emoji} {name} · {caption}"),
                    None => format!("{emoji} {name}"),
                }
            } else if let Some(caption) = caption {
                format!("{emoji} {caption}")
            } else if let Some(ms) = duration_ms {
                format!("{emoji} {word} ({})", duration(*ms))
            } else {
                format!("{emoji} {word}")
            }
        }
        Body::Invite { group_name } => format!("{}: {group_name}", words.group_invite),
        Body::JoinRequest { group_name } => format!("{}: {group_name}", words.group_request),
        Body::Welcome { group_name } => format!("{}: {group_name}", words.group_welcome),
    }
}

/// Files sent together, as far as they came: an album, or a few documents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Album {
    batch: String,
    files: bool,
    n: u32,
    caption: Option<String>,
}

impl Album {
    /// The album a message belongs to, if it was sent with others.
    fn of(body: &Body) -> Option<Self> {
        match body {
            Body::Media { kind, caption, batch: Some(batch), .. } if kind != "voice" && kind != "circle" => {
                Some(Self { batch: batch.clone(), files: named(kind), n: 1, caption: caption.clone() })
            }
            _ => None,
        }
    }

    fn line(&self, words: &Words) -> String {
        let head = if self.files {
            format!("📎 {}", words.files.replace("{n}", &self.n.to_string()))
        } else {
            format!("🖼 {}", words.album.replace("{n}", &self.n.to_string()))
        };
        match &self.caption {
            Some(caption) => format!("{head} · {caption}"),
            None => head,
        }
    }
}

/// One line of a chat's notification: who wrote, where it matters, and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// "Anna: " in a group, "Message request · " from a stranger.
    prefix: String,
    text: String,
    album: Option<Album>,
}

impl Line {
    fn shown(&self) -> String {
        format!("{}{}", self.prefix, self.text)
    }
}

/// What the window looks like to the user right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowSeen {
    pub focused: bool,
    pub visible: bool,
    pub minimized: bool,
}

impl WindowSeen {
    fn of(app: &tauri::AppHandle) -> Self {
        let Some(w) = app.state::<Shell>().main_window() else { return Self::default() };
        Self {
            focused: w.is_focused().unwrap_or(false),
            visible: w.is_visible().unwrap_or(false),
            minimized: w.is_minimized().unwrap_or(false),
        }
    }

    fn on_screen(self) -> bool {
        self.focused && self.visible && !self.minimized
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The card in the app.
    Card,
    /// A notification of the system.
    System,
}

/// The one decision: who tells the user. When the system cannot show
/// anything, or the user turned it off, the card is left to do it.
pub fn route(window: WindowSeen, enabled: bool, available: bool) -> Route {
    if window.on_screen() || !enabled || !available {
        Route::Card
    } else {
        Route::System
    }
}

/// What one chat's notification says so far.
#[derive(Default)]
struct Stack {
    lines: VecDeque<Line>,
    count: u32,
}

impl Stack {
    /// One more of an album that is coming joins its line, which then says
    /// how many there are, not which.
    fn push(&mut self, line: Option<Line>, words: &Words) {
        if let Some(line) = &line {
            if let (Some(next), Some(last)) = (&line.album, self.lines.back_mut()) {
                if let Some(album) = last.album.as_mut().filter(|a| a.batch == next.batch) {
                    album.n += next.n;
                    album.files &= next.files;
                    if album.caption.is_none() {
                        album.caption.clone_from(&next.caption);
                    }
                    last.text = album.line(words);
                    return;
                }
            }
        }
        self.count += 1;
        if let Some(line) = line {
            self.lines.push_back(line);
            while self.lines.len() > MAX_LINES {
                self.lines.pop_front();
            }
        }
    }

    /// The last lines, and "+N" for the messages they leave out.
    fn body(&self, words: &Words) -> String {
        if self.lines.is_empty() {
            return if self.count > 1 {
                words.new_messages.replace("{n}", &self.count.to_string())
            } else {
                words.new_message.clone()
            };
        }
        let mut out: Vec<String> = self.lines.iter().map(Line::shown).collect();
        let left_out = self.count.saturating_sub(self.lines.len() as u32);
        if left_out > 0 {
            out.push(words.more.replace("{n}", &left_out.to_string()));
        }
        out.join("\n")
    }
}

/// A notice as a notification: its key, title, the line it adds (None: it
/// only counts), and the sender's picture.
#[derive(Debug, PartialEq, Eq)]
pub struct Worded {
    pub key: String,
    pub title: String,
    pub line: Option<Line>,
    pub picture: Option<String>,
}

pub fn word(outcome: Outcome, words: &Words) -> Option<Worded> {
    match outcome {
        Outcome::Show(n) => {
            let prefix = match n.kind {
                ChatKind::Group => format!("{}: ", n.sender),
                ChatKind::Request => format!("{} · ", words.request),
                ChatKind::Dm => String::new(),
            };
            let (text, album) = match &n.body {
                None => (words.new_message.clone(), None),
                Some(body) => (body_line(body, words), Album::of(body)),
            };
            Some(Worded {
                key: n.chat.unwrap_or_else(|| NO_CHAT.into()),
                title: n.title,
                line: Some(Line { prefix, text, album }),
                picture: n.picture,
            })
        }
        Outcome::Plain(p) => Some(Worded {
            key: p.chat.unwrap_or_else(|| NO_CHAT.into()),
            title: p.title.unwrap_or_else(|| words.app.clone()),
            line: None,
            picture: None,
        }),
        Outcome::Quiet { .. } => None,
        // An invitation by push is the phone's way to a ringing call; a
        // computer hears of calls from its runtime (`call.incoming`), and
        // of their end too.
        Outcome::Call(_) | Outcome::CallEnd(_) => None,
    }
}

// ─── A ringing call ─────────────────────────────────────────────────────────

/// The ids of the buttons of a ringing call.
pub const ACTION_ANSWER: &str = "answer";
pub const ACTION_DECLINE: &str = "decline";

/// The key of a call's notification: `call:<call_id>`.
const CALL_KEY: &str = "call:";

/// A ringing call's notification goes after this at the latest, should
/// the end of the call never be heard (the runtime gives up at 45 s).
const RING_LIMIT: Duration = Duration::from_secs(90);

fn call_key(call_id: &str) -> String {
    format!("{CALL_KEY}{call_id}")
}

/// What of the `call` of a `call.*` event a notification needs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Ringing {
    pub call_id: String,
    /// The caller, hex.
    pub peer: String,
    pub chat_id: String,
    pub media: CallMedia,
    phase: CallPhase,
}

/// What a `call.*` event means for the notifications.
#[derive(Debug, PartialEq, Eq)]
pub enum CallNotice {
    /// A call rings here.
    Ring(Ringing),
    /// Its ringing is over: answered, declined, ended, taken elsewhere.
    Gone(String),
    Nothing,
}

pub fn call_notice(name: &str, payload: &serde_json::Value) -> CallNotice {
    let Some(call) = payload.get("call").and_then(|c| Ringing::deserialize(c).ok()) else {
        return CallNotice::Nothing;
    };
    match name {
        UI_EVENT_CALL_INCOMING if call.phase == CallPhase::Incoming => CallNotice::Ring(call),
        UI_EVENT_CALL_STATE if call.phase != CallPhase::Incoming => CallNotice::Gone(call.call_id),
        UI_EVENT_CALL_ENDED => CallNotice::Gone(call.call_id),
        _ => CallNotice::Nothing,
    }
}

/// Who calls, as a message of theirs would name them: a PIN on the app or
/// the setting "no content" leaves only the app's name, and no face.
fn caller(outcome: Outcome, words: &Words) -> (String, Option<String>) {
    match outcome {
        Outcome::Show(n) if !n.title.is_empty() => (n.title, n.picture),
        _ => (words.app.clone(), None),
    }
}

fn call_toast(call: &Ringing, title: String, image: Option<PathBuf>, silent: bool, words: &Words) -> Toast {
    let body = match call.media {
        CallMedia::Audio => &words.call_audio,
        CallMedia::Video => &words.call_video,
    };
    Toast {
        key: call_key(&call.call_id),
        title,
        body: body.clone(),
        image,
        silent,
        kind: Kind::Call,
        actions: vec![
            Action::new(ACTION_ANSWER, words.call_answer.clone()),
            Action::new(ACTION_DECLINE, words.call_decline.clone()),
        ],
    }
}

/// The call of `call_state` as a notification needs it, when it rings here.
fn ringing_now(call: CallView) -> Option<Ringing> {
    (call.phase == CallPhase::Incoming).then_some(Ringing {
        call_id: call.call_id,
        peer: call.peer,
        chat_id: call.chat_id,
        media: call.media,
        phase: call.phase,
    })
}

/// What a look at the call ringing now changes after missed events.
#[derive(Debug, PartialEq, Eq)]
struct Recheck {
    /// Notifications of calls that no longer ring.
    gone: Vec<String>,
    /// A call that rings and has no notification yet: its `call.incoming`
    /// was among the missed events.
    ring: Option<Ringing>,
}

/// `now` is the call ringing now (`None`: nothing rings). An error means the
/// runtime could not tell: a call that may still ring keeps its notification.
fn recheck<E>(ringing: &HashSet<String>, now: Result<Option<Ringing>, E>) -> Recheck {
    let Ok(now) = now else {
        return Recheck { gone: Vec::new(), ring: None };
    };
    let id = now.as_ref().map(|c| &c.call_id);
    let mut gone: Vec<String> = ringing.iter().filter(|r| Some(*r) != id).cloned().collect();
    gone.sort();
    let ring = now.filter(|c| !ringing.contains(&c.call_id));
    Recheck { gone, ring }
}

/// A button pressed on a call's notification.
#[derive(Debug, PartialEq, Eq)]
pub enum CallPress {
    Answer(String),
    Decline(String),
}

pub fn call_press(key: &str, action: &str) -> Option<CallPress> {
    let call_id = key.strip_prefix(CALL_KEY).filter(|id| !id.is_empty())?.to_string();
    match action {
        ACTION_ANSWER => Some(CallPress::Answer(call_id)),
        ACTION_DECLINE => Some(CallPress::Decline(call_id)),
        _ => None,
    }
}

pub struct DesktopNotify {
    app: tauri::AppHandle,
    notifier: Notifier,
    words: RwLock<Words>,
    stacks: Mutex<HashMap<String, Stack>>,
    avatars: Option<PathBuf>,
    /// The calls whose notification is up or being made. What is shown and
    /// what is taken away goes under this lock, so a call that ended while
    /// its face was fetched is not shown after.
    ringing: Mutex<HashSet<String>>,
}

impl DesktopNotify {
    /// Runs inside the tokio runtime of Tauri: the notifier spawns its task there.
    pub fn start(app: tauri::AppHandle) -> Arc<Self> {
        // A --workdir profile keeps its cache inside its own directory.
        let cache = match veydan_shell::workdir::current() {
            Some(workdir) => Some(workdir.cache_dir()),
            None => app.path().app_cache_dir().ok(),
        };
        let product = *app.state::<Shell>().product();
        let icon = match (cache.as_deref(), app.default_window_icon()) {
            (Some(cache), Some(icon)) => icon_file(cache, icon),
            _ => None,
        };
        let info = app_info(app.config().identifier.clone(), app.package_info().name.clone(), &product, icon);
        let (tapped, pressed_on) = (app.clone(), app.clone());
        let handlers = Handlers {
            on_click: Arc::new(move |key: String| clicked(&tapped, key)),
            on_action: Arc::new(move |key: String, action: String| pressed(&pressed_on, &key, &action)),
        };
        let notifier = Notifier::start(info, handlers);
        let avatars = cache.map(|d| d.join("notify-avatars"));
        Arc::new(Self {
            app,
            notifier,
            words: RwLock::new(Words::named(product.name)),
            stacks: Mutex::new(HashMap::new()),
            avatars,
            ringing: Mutex::new(HashSet::new()),
        })
    }

    pub fn available(&self) -> bool {
        self.notifier.available()
    }

    /// The messenger stops — it was switched off, or the app quits: its
    /// notifications go with it (a click on one would lead nowhere). Blocks
    /// for up to a second.
    pub fn withdraw(&self) {
        self.stacks.lock().unwrap().clear();
        self.ringing.lock().unwrap().clear();
        self.notifier.shutdown();
    }

    pub fn set_words(&self, words: Words) {
        let mut held = self.words.write().unwrap();
        *held = std::mem::take(&mut *held).with_page(words);
    }

    /// Takes a `notify` payload. True when the system shows it: the card
    /// is then not to.
    pub async fn take(self: &Arc<Self>, rt: &Arc<MessengerRuntime>, payload: &serde_json::Value) -> bool {
        let Ok(notice) = serde_json::from_value::<messenger_core::Notice>(payload.clone()) else {
            return false;
        };
        let enabled = rt.desktop_notify_settings().await.map(|s| s.enabled).unwrap_or(true);
        if route(WindowSeen::of(&self.app), enabled, self.available()) == Route::Card {
            return false;
        }
        let (me, rt) = (self.clone(), rt.clone());
        tauri::async_runtime::spawn(async move { me.show(&rt, notice).await });
        true
    }

    async fn show(&self, rt: &MessengerRuntime, notice: messenger_core::Notice) {
        let outcome = match rt.live_notice(&notice, self.locked().await).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("messenger notify: {e}");
                return;
            }
        };
        let words = self.words.read().unwrap().clone();
        let Some(w) = word(outcome, &words) else { return };
        let sound = rt.desktop_notify_settings().await.map(|s| s.sound).unwrap_or(true);
        let image = match &w.picture {
            Some(url) => self.avatar(url).await,
            None => None,
        };
        let body = {
            let mut stacks = self.stacks.lock().unwrap();
            let stack = stacks.entry(w.key.clone()).or_default();
            stack.push(w.line, &words);
            stack.body(&words)
        };
        self.notifier.show(Toast { key: w.key, title: w.title, body, image, silent: !sound, ..Toast::default() });
    }

    /// A notification to see that notifications work, from the settings.
    pub async fn show_test(&self, rt: &MessengerRuntime, title: String, body: String) {
        let sound = rt.desktop_notify_settings().await.map(|s| s.sound).unwrap_or(true);
        self.notifier.show(Toast { key: "test".into(), title, body, silent: !sound, ..Toast::default() });
    }

    /// Takes a `call.*` event: a call that rings while the window is away
    /// gets a notification; one that stopped ringing loses it.
    pub async fn take_call(self: &Arc<Self>, rt: &Arc<MessengerRuntime>, name: &str, payload: &serde_json::Value) {
        match call_notice(name, payload) {
            CallNotice::Ring(call) => self.start_ring(rt, call).await,
            CallNotice::Gone(call_id) => self.call_gone(&call_id),
            CallNotice::Nothing => {}
        }
    }

    /// A call rings here: its notification, unless the window is seen or
    /// the notifications are off.
    async fn start_ring(self: &Arc<Self>, rt: &Arc<MessengerRuntime>, call: Ringing) {
        let settings = rt.desktop_notify_settings().await.unwrap_or_default();
        if route(WindowSeen::of(&self.app), settings.enabled, self.available()) == Route::Card {
            return;
        }
        self.ringing.lock().unwrap().insert(call.call_id.clone());
        let (me, rt) = (self.clone(), rt.clone());
        tauri::async_runtime::spawn(async move { me.ring(&rt, call, !settings.sound).await });
    }

    async fn ring(self: Arc<Self>, rt: &MessengerRuntime, call: Ringing, silent: bool) {
        let words = self.words.read().unwrap().clone();
        let notice = messenger_core::Notice {
            title: String::new(),
            body: None,
            chat_id: Some(call.chat_id.clone()),
            sender: Some(call.peer.clone()),
            request: false,
        };
        let (title, picture) = match rt.live_notice(&notice, self.locked().await).await {
            Ok(outcome) => caller(outcome, &words),
            Err(e) => {
                eprintln!("messenger notify: call: {e}");
                (words.app.clone(), None)
            }
        };
        let image = match &picture {
            Some(url) => self.avatar(url).await,
            None => None,
        };
        let toast = call_toast(&call, title, image, silent, &words);
        {
            let ringing = self.ringing.lock().unwrap();
            if !ringing.contains(&call.call_id) {
                return;
            }
            self.notifier.show(toast);
        }
        // The end of a call that is never heard must not leave it ringing.
        tokio::time::sleep(RING_LIMIT).await;
        self.call_gone(&call.call_id);
    }

    /// The call stopped ringing: its notification goes.
    fn call_gone(&self, call_id: &str) {
        let mut ringing = self.ringing.lock().unwrap();
        if ringing.remove(call_id) {
            self.notifier.clear(&call_key(call_id));
        }
    }

    /// Events were missed: whatever is not the call ringing now goes, and
    /// a call that rings now gets the notification its lost `call.incoming`
    /// would have given. When the runtime cannot tell, all stays as it is.
    pub async fn recheck_calls(self: &Arc<Self>, rt: &Arc<MessengerRuntime>) {
        let now = rt.call_state().await.map(|state| state.call.and_then(ringing_now));
        let change = recheck(&self.ringing.lock().unwrap(), now);
        for call_id in change.gone {
            self.call_gone(&call_id);
        }
        if let Some(call) = change.ring {
            self.start_ring(rt, call).await;
        }
    }

    async fn locked(&self) -> bool {
        match self.app.try_state::<veydan_lock::Lock>() {
            Some(lock) => lock.enabled().await,
            None => true,
        }
    }

    /// Takes the chat's notification away (it was read), or all of them.
    pub fn clear(&self, key: Option<&str>) {
        let mut stacks = self.stacks.lock().unwrap();
        match key {
            Some(key) => {
                stacks.remove(key);
                self.notifier.clear(key);
                // What came about a person before a chat existed.
                if key.starts_with("dm:") && stacks.remove(NO_CHAT).is_some() {
                    self.notifier.clear(NO_CHAT);
                }
            }
            None => {
                stacks.clear();
                self.notifier.clear_all();
            }
        }
    }

    /// The sender's picture as a file the system can read. Nothing when it
    /// cannot be had quickly: the notification goes without it.
    async fn avatar(&self, url: &str) -> Option<PathBuf> {
        let dir = self.avatars.as_ref()?;
        let name: String = Sha256::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
        let client = reqwest::Client::builder().timeout(AVATAR_TIMEOUT).build().ok()?;
        let resp = client.get(url).send().await.ok()?.error_for_status().ok()?;
        if resp.content_length().is_some_and(|l| l as usize > AVATAR_MAX_BYTES) {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        if bytes.len() > AVATAR_MAX_BYTES {
            return None;
        }
        tokio::fs::create_dir_all(dir).await.ok()?;
        tokio::fs::write(&path, &bytes).await.ok()?;
        Some(path)
    }
}

// ─── Waiting messages on the icon and in the tray ───────────────────────────

static UNREAD_KICK: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Something changed what waits (a chat read, muted, archived): count again now.
pub fn unread_changed() {
    UNREAD_KICK.notify_one();
}

/// Counts the messages waiting in chats that are not muted, after the
/// runtime's events (a burst counts once) and when kicked, and shows the
/// number on the app's icon and in the tray. A messenger that is switched
/// off has no runtime and no count: its stop takes the number away.
pub fn spawn_unread(app: tauri::AppHandle, rt: Arc<MessengerRuntime>) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        let mut rx = rt.ui_events();
        let mut shown: Option<i64> = None;
        loop {
            let n = rt.dm().total_unread().await.unwrap_or(0);
            if shown != Some(n) {
                shown = Some(n);
                show_unread(&app, n);
            }
            tokio::select! {
                ev = rx.recv() => {
                    if let Err(tokio::sync::broadcast::error::RecvError::Closed) = ev { break }
                }
                _ = UNREAD_KICK.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            while !matches!(rx.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty | tokio::sync::broadcast::error::TryRecvError::Closed)) {}
        }
    })
}

/// The count goes to the shell: the tray and the icon of the app show what
/// waits in all modules together.
fn show_unread(app: &tauri::AppHandle, n: i64) {
    app.state::<Shell>().set_unread(super::MODULE_ID, n.max(0) as usize);
}

/// Who the notifications are from, as the systems tell apps apart: the
/// identifier and the name from the product's config, its desktop entry and
/// icon name from its description, its window icon as a file.
fn app_info(id: String, name: String, product: &Product, icon_file: Option<PathBuf>) -> AppInfo {
    AppInfo {
        id,
        name,
        desktop_entry: product.desktop_entry.into(),
        icon: product.icon.into(),
        icon_file,
    }
}

/// The window icon of the product as a file, for systems that take the
/// source's icon from one (Windows). Written into the cache when it is not
/// there yet or holds another icon.
fn icon_file(cache: &Path, icon: &tauri::image::Image<'_>) -> Option<PathBuf> {
    let png = png_of(icon)?;
    let path = cache.join("notify-icon.png");
    if std::fs::read(&path).ok().as_deref() != Some(png.as_slice()) {
        std::fs::create_dir_all(cache).ok()?;
        std::fs::write(&path, &png).ok()?;
    }
    Some(path)
}

/// An icon, which Tauri holds as RGBA pixels, as a PNG.
fn png_of(icon: &tauri::image::Image<'_>) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, icon.width(), icon.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(icon.rgba()).ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

/// A click: the window comes up and the page opens the chat.
fn clicked(app: &tauri::AppHandle, key: String) {
    app.state::<Shell>().show_main_window();
    let chat = (key.starts_with("dm:") || key.starts_with("group:")).then_some(key.clone());
    if let Some(d) = app.try_state::<MessengerState>().and_then(|s| s.desktop()) {
        d.stacks.lock().unwrap().remove(&key);
    }
    let _ = app.emit(EVENT_NOTICE_TAP, serde_json::json!({ "chat": chat }));
}

/// A button on a call's notification, which is gone by then: Answer takes
/// the call and brings the window up, where the call goes on; Decline
/// refuses it. What follows comes to the page as the call's events.
fn pressed(app: &tauri::AppHandle, key: &str, action: &str) {
    let Some(press) = call_press(key, action) else { return };
    let state = app.try_state::<MessengerState>();
    if let Some(d) = state.as_ref().and_then(|s| s.desktop()) {
        if let Some(call_id) = key.strip_prefix(CALL_KEY) {
            d.ringing.lock().unwrap().remove(call_id);
        }
    }
    let Some(rt) = state.and_then(|s| s.runtime().ok()) else { return };
    if matches!(press, CallPress::Answer(_)) {
        app.state::<Shell>().show_main_window();
    }
    tauri::async_runtime::spawn(async move {
        let done = match &press {
            CallPress::Answer(call_id) => rt.call_accept(call_id).await.map(drop),
            CallPress::Decline(call_id) => rt.call_decline(call_id).await,
        };
        if let Err(e) = done {
            eprintln!("messenger notify: {press:?}: {e}");
        }
    });
}

// ─── Commands ───────────────────────────────────────────────────────────────

/// The settings of this computer's notifications, and what stands in their way.
#[derive(Debug, Clone, Serialize)]
pub struct DesktopNotifyView {
    pub enabled: bool,
    pub sound: bool,
    /// The system can show notifications (a notification server, an app bundle).
    pub available: bool,
    /// Closing the window keeps the app running; otherwise nothing comes after it.
    pub close_to_tray: bool,
}

fn desktop(messenger: &MessengerState) -> CmdResult<Arc<DesktopNotify>> {
    messenger.desktop().ok_or_else(|| AppError::Other("notify_unavailable".into()))
}

#[tauri::command]
pub async fn messenger_desktop_notify_get(
    messenger: tauri::State<'_, MessengerState>,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<DesktopNotifyView> {
    let d = desktop(&messenger)?;
    let s = messenger.runtime()?.desktop_notify_settings().await.map_err(super::map_err)?;
    Ok(DesktopNotifyView {
        enabled: s.enabled,
        sound: s.sound,
        available: d.available(),
        close_to_tray: shell.tray_settings().close_to_tray,
    })
}

#[tauri::command]
pub async fn messenger_desktop_notify_set(
    enabled: bool,
    sound: bool,
    messenger: tauri::State<'_, MessengerState>,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<DesktopNotifyView> {
    let d = desktop(&messenger)?;
    let rt = messenger.runtime()?;
    rt.desktop_notify_set_settings(DesktopSettings { enabled, sound }).await.map_err(super::map_err)?;
    if !enabled {
        d.clear(None);
    }
    messenger_desktop_notify_get(messenger, shell).await
}

#[tauri::command]
pub async fn messenger_desktop_notify_test(
    title: String,
    body: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<()> {
    let rt = messenger.runtime()?;
    desktop(&messenger)?.show_test(&rt, title, body).await;
    Ok(())
}

/// Closing the window keeps the app running, in the tray: the one thing a
/// computer needs for notifications to come after the window is closed.
#[tauri::command]
pub async fn messenger_desktop_notify_keep_running(
    messenger: tauri::State<'_, MessengerState>,
    shell: tauri::State<'_, Shell>,
) -> CmdResult<DesktopNotifyView> {
    let settings = veydan_shell::TraySettings {
        close_to_tray: true,
        ..shell.tray_settings()
    };
    shell.set_tray_settings(settings).await?;
    messenger_desktop_notify_get(messenger, shell).await
}

/// The page's words for notifications, in the user's language.
#[tauri::command]
pub fn messenger_notice_words(words: Words, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    desktop(&messenger)?.set_words(words);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_notify::{Notice, Plain, Reason};

    const ON_SCREEN: WindowSeen = WindowSeen { focused: true, visible: true, minimized: false };

    #[test]
    fn the_card_while_the_window_is_seen_the_system_otherwise() {
        assert_eq!(route(ON_SCREEN, true, true), Route::Card);
        for w in [
            WindowSeen { focused: false, ..ON_SCREEN },
            WindowSeen { visible: false, ..ON_SCREEN },
            WindowSeen { minimized: true, ..ON_SCREEN },
        ] {
            assert_eq!(route(w, true, true), Route::System, "{w:?}");
            assert_eq!(route(w, false, true), Route::Card, "turned off");
            assert_eq!(route(w, true, false), Route::Card, "nothing to show it with");
        }
    }

    fn words() -> Words {
        Words::named("Veydan Space")
    }

    /// A line as `word` makes it, of a chat without names in it.
    fn plain_line(text: &str) -> Option<Line> {
        Some(Line { prefix: String::new(), text: text.into(), album: None })
    }

    fn shown_line(w: Worded) -> Option<String> {
        w.line.map(|l| l.shown())
    }

    #[test]
    fn a_stack_keeps_the_last_lines_and_counts_the_rest() {
        let mut s = Stack::default();
        s.push(plain_line("one"), &words());
        assert_eq!(s.body(&words()), "one");
        for i in 2..=7 {
            s.push(plain_line(&format!("line {i}")), &words());
        }
        assert_eq!(s.body(&words()), "line 3\nline 4\nline 5\nline 6\nline 7\n+2 more");
    }

    #[test]
    fn a_plain_stack_only_counts() {
        let mut s = Stack::default();
        s.push(None, &words());
        assert_eq!(s.body(&words()), "New message");
        s.push(None, &words());
        assert_eq!(s.body(&words()), "2 new messages");
    }

    fn notice(kind: ChatKind, body: Option<Body>) -> Outcome {
        Outcome::Show(Notice {
            kind,
            chat: Some("group:g".into()),
            title: "Team".into(),
            sender: "Alice".into(),
            sender_key: String::new(),
            picture: None,
            body,
            muted: false,
            hide_on_lockscreen: false,
            count: 1,
        })
    }

    fn media(kind: &str, name: &str, caption: Option<&str>, duration_ms: Option<u64>, batch: Option<&str>) -> Option<Body> {
        Some(Body::Media {
            kind: kind.into(),
            name: name.into(),
            caption: caption.map(String::from),
            duration_ms,
            batch: batch.map(String::from),
        })
    }

    #[test]
    fn lines_say_who_wrote_where_it_matters() {
        let text = Some(Body::Text { text: "hi".into() });
        let w = word(notice(ChatKind::Group, text.clone()), &words()).unwrap();
        assert_eq!((w.key.as_str(), w.title.as_str()), ("group:g", "Team"));
        assert_eq!(shown_line(w).as_deref(), Some("Alice: hi"));
        assert_eq!(shown_line(word(notice(ChatKind::Dm, text.clone()), &words()).unwrap()).as_deref(), Some("hi"));
        assert_eq!(
            shown_line(word(notice(ChatKind::Request, text), &words()).unwrap()).as_deref(),
            Some("Message request · hi")
        );
        assert_eq!(shown_line(word(notice(ChatKind::Dm, None), &words()).unwrap()).as_deref(), Some("New message"));
        assert_eq!(
            shown_line(word(notice(ChatKind::Dm, Some(Body::Invite { group_name: "X".into() })), &words()).unwrap()).as_deref(),
            Some("Invites you to a group: X")
        );
    }

    #[test]
    fn a_file_is_told_by_what_it_is() {
        let line = |body: Option<Body>| shown_line(word(notice(ChatKind::Dm, body), &words()).unwrap()).unwrap();
        assert_eq!(line(media("image", "IMG_1.jpg", None, None, None)), "📷 Photo");
        assert_eq!(line(media("image", "IMG_1.jpg", Some("the sea"), None, None)), "📷 the sea");
        assert_eq!(line(media("video", "v.mp4", None, Some(42_000), None)), "🎬 Video (0:42)");
        assert_eq!(line(media("voice", "voice.weba", None, Some(12_400), None)), "🎤 Voice message (0:12)");
        assert_eq!(line(media("circle", "c.webm", None, Some(3_723_000), None)), "⭕ Video message (1:02:03)");
        assert_eq!(line(media("file", "report.pdf", None, None, None)), "📎 report.pdf");
        assert_eq!(line(media("file", "report.pdf", Some("by Friday"), None, None)), "📎 report.pdf · by Friday");
        assert_eq!(line(media("audio", "", None, None, None)), "🎵 Audio");

        let link = |link: LinkKind, title: &str| line(Some(Body::Link { link, title: title.into() }));
        assert_eq!(link(LinkKind::Web, "example.org/a"), "🔗 example.org/a");
        assert_eq!(link(LinkKind::Group, "Club"), "🔗 Group “Club”");
        assert_eq!(link(LinkKind::Group, ""), "🔗 Link to a group");
        assert_eq!(link(LinkKind::Contact, "Anna"), "👤 Contact: Anna");
    }

    #[test]
    fn pictures_sent_together_are_one_line_that_counts_them() {
        let w = words();
        let mut s = Stack::default();
        let push = |s: &mut Stack, body: Option<Body>| s.push(word(notice(ChatKind::Group, body), &w).unwrap().line, &w);
        push(&mut s, Some(Body::Text { text: "look".into() }));
        push(&mut s, media("image", "1.jpg", None, None, Some("b1")));
        assert_eq!(s.body(&w), "Alice: look\nAlice: 📷 Photo", "one picture is a picture");
        push(&mut s, media("video", "2.mp4", Some("our trip"), None, Some("b1")));
        push(&mut s, media("image", "3.jpg", None, None, Some("b1")));
        assert_eq!(s.body(&w), "Alice: look\nAlice: 🖼 Album: 3 · our trip");
        // Another album is another line; documents are counted as files.
        push(&mut s, media("file", "a.pdf", None, None, Some("b2")));
        push(&mut s, media("file", "b.pdf", None, None, Some("b2")));
        assert_eq!(s.body(&w), "Alice: look\nAlice: 🖼 Album: 3 · our trip\nAlice: 📎 Files: 2");
        assert_eq!(s.count, 3, "an album counts as one message of the chat");
    }

    #[test]
    fn plain_and_quiet() {
        let p = Outcome::Plain(Plain { kind: ChatKind::Dm, chat: None, title: None, muted: false, count: 1 });
        let w = word(p, &words()).unwrap();
        assert_eq!((w.key.as_str(), w.title.as_str(), w.line), (NO_CHAT, "Veydan Space", None));
        assert_eq!(word(Outcome::Quiet { reason: Reason::Own }, &words()), None);
    }

    /// A product that is not Space: what the systems tell its notifications
    /// apart by comes from its description and its config, none from Space.
    const CHAT: Product = Product {
        id: "chat",
        name: "Veydan Chat",
        desktop_entry: "veydanchat",
        icon: "veydanchat-icon",
        sync: &veydan_shell::Plan { collect: &[], apply: &[], late: &[] },
    };

    #[test]
    fn the_notifications_are_from_the_product_they_run_in() {
        let info = app_info("net.veydan.chat".into(), "Veydan Chat".into(), &CHAT, None);
        assert_eq!(info.id, "net.veydan.chat");
        assert_eq!(info.name, "Veydan Chat");
        assert_eq!(info.desktop_entry, "veydanchat");
        assert_eq!(info.icon, "veydanchat-icon");
        let plain = Outcome::Plain(Plain { kind: ChatKind::Dm, chat: None, title: None, muted: false, count: 1 });
        assert_eq!(word(plain, &Words::named(CHAT.name)).unwrap().title, "Veydan Chat");
        assert_eq!(Words::default().app, "", "no product is named by default");
    }

    /// The page of Space still sends `app: 'Veydan Space'`; the product names
    /// its notifications itself.
    #[test]
    fn the_page_gives_the_words_and_the_product_its_name() {
        let page = Words { app: "Veydan Space".into(), new_message: "Новое сообщение".into(), ..Words::default() };
        let words = Words::named(CHAT.name).with_page(page);
        assert_eq!(words.app, "Veydan Chat");
        assert_eq!(words.new_message, "Новое сообщение");
    }

    fn call_payload(phase: &str, media: &str) -> serde_json::Value {
        // As `messenger-calls` writes it, with fields a notification ignores.
        serde_json::json!({ "call": {
            "call_id": "c1", "peer": "ab".repeat(32), "chat_id": format!("dm:{}", "ab".repeat(32)),
            "direction": "in", "media": media, "phase": phase, "muted": false,
            "started_at": 1_700_000_000, "nodes": [], "via": "relay",
        }})
    }

    #[test]
    fn a_call_rings_until_it_is_answered_or_over() {
        let ring = call_notice(UI_EVENT_CALL_INCOMING, &call_payload("incoming", "video"));
        let CallNotice::Ring(call) = ring else { panic!("{ring:?}") };
        assert_eq!((call.call_id.as_str(), call.media), ("c1", CallMedia::Video));
        assert!(call.chat_id.starts_with("dm:"));

        for phase in ["connecting", "active", "ended"] {
            assert_eq!(call_notice(UI_EVENT_CALL_STATE, &call_payload(phase, "audio")), CallNotice::Gone("c1".into()), "{phase}");
        }
        // Still ringing (a mute, a new offer) is nothing new.
        assert_eq!(call_notice(UI_EVENT_CALL_STATE, &call_payload("incoming", "audio")), CallNotice::Nothing);
        let ended = serde_json::json!({ "call": call_payload("ended", "audio")["call"], "outcome": "answered_elsewhere", "duration_secs": null });
        assert_eq!(call_notice(UI_EVENT_CALL_ENDED, &ended), CallNotice::Gone("c1".into()));
        // My own call rings nowhere here; other events and bad payloads are nothing.
        assert_eq!(call_notice(UI_EVENT_CALL_INCOMING, &call_payload("outgoing", "audio")), CallNotice::Nothing);
        assert_eq!(call_notice("call.level", &serde_json::json!({ "call_id": "c1", "level": 0.5 })), CallNotice::Nothing);
        assert_eq!(call_notice(UI_EVENT_CALL_ENDED, &serde_json::json!({})), CallNotice::Nothing);
    }

    #[test]
    fn a_ringing_call_is_named_as_a_message_would_be_and_has_two_buttons() {
        let w = Words { call_answer: "Ответить".into(), ..words() };
        let CallNotice::Ring(call) = call_notice(UI_EVENT_CALL_INCOMING, &call_payload("incoming", "audio")) else { panic!() };

        let shown = Outcome::Show(Notice {
            kind: ChatKind::Dm,
            chat: Some(call.chat_id.clone()),
            title: "Alice".into(),
            sender: "Alice".into(),
            sender_key: call.peer.clone(),
            picture: Some("https://x/a.png".into()),
            body: None,
            muted: false,
            hide_on_lockscreen: false,
            count: 1,
        });
        assert_eq!(caller(shown, &w), ("Alice".to_string(), Some("https://x/a.png".to_string())));
        // A PIN on the app: only the app's name, and no face.
        let plain = Outcome::Plain(Plain { kind: ChatKind::Dm, chat: Some(call.chat_id.clone()), title: None, muted: false, count: 1 });
        assert_eq!(caller(plain, &w), ("Veydan Space".to_string(), None));

        let t = call_toast(&call, "Alice".into(), Some("/c/a".into()), false, &w);
        assert_eq!((t.key.as_str(), t.title.as_str(), t.body.as_str()), ("call:c1", "Alice", "Incoming call"));
        assert_eq!(t.kind, Kind::Call);
        assert_eq!(t.actions, [Action::new("answer", "Ответить"), Action::new("decline", "Decline")]);
        let video = Ringing { media: CallMedia::Video, ..call };
        assert_eq!(call_toast(&video, "A".into(), None, true, &w).body, "Incoming video call");
    }

    #[test]
    fn missed_events_are_made_up_for_by_the_call_ringing_now() {
        let view = |phase: &str| serde_json::from_value::<CallView>(call_payload(phase, "audio")["call"].clone()).unwrap();
        let now = ringing_now(view("incoming")).expect("it rings");
        assert_eq!((now.call_id.as_str(), now.media), ("c1", CallMedia::Audio));
        assert_eq!(ringing_now(view("active")), None);
        assert_eq!(ringing_now(view("outgoing")), None);

        let none = HashSet::new();
        let c1: HashSet<String> = ["c1".to_string()].into();
        let both: HashSet<String> = ["c0".to_string(), "c1".to_string()].into();
        // Its `call.incoming` was lost: it rings now.
        assert_eq!(recheck::<()>(&none, Ok(Some(now.clone()))), Recheck { gone: vec![], ring: Some(now.clone()) });
        // Already shown: not again; one that stopped ringing goes.
        assert_eq!(recheck::<()>(&c1, Ok(Some(now.clone()))), Recheck { gone: vec![], ring: None });
        assert_eq!(recheck::<()>(&both, Ok(Some(now.clone()))), Recheck { gone: vec!["c0".into()], ring: None });
        // Nothing rings: all go.
        assert_eq!(recheck::<()>(&both, Ok(None)), Recheck { gone: vec!["c0".into(), "c1".into()], ring: None });
        // The runtime could not tell: what rings keeps ringing.
        assert_eq!(recheck(&c1, Err(())), Recheck { gone: vec![], ring: None });
    }

    #[test]
    fn the_buttons_answer_and_decline_that_call() {
        assert_eq!(call_press("call:c1", ACTION_ANSWER), Some(CallPress::Answer("c1".into())));
        assert_eq!(call_press("call:c1", ACTION_DECLINE), Some(CallPress::Decline("c1".into())));
        assert_eq!(call_press("call:c1", "other"), None);
        assert_eq!(call_press("dm:ab", ACTION_ANSWER), None, "not a call");
        assert_eq!(call_press("call:", ACTION_ANSWER), None);
    }

    #[test]
    fn the_window_icon_of_the_product_is_written_as_a_png() {
        let cache = tempfile::tempdir().unwrap();
        let rgba: Vec<u8> = (0..2 * 3 * 4).map(|i| i as u8 * 9).collect();
        let icon = tauri::image::Image::new_owned(rgba.clone(), 2, 3);
        let path = icon_file(cache.path(), &icon).unwrap();
        assert_eq!(path, cache.path().join("notify-icon.png"));

        let mut reader = png::Decoder::new(std::io::Cursor::new(std::fs::read(&path).unwrap()))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((frame.width, frame.height, frame.color_type), (2, 3, png::ColorType::Rgba));
        assert_eq!(&pixels[..frame.buffer_size()], rgba.as_slice());

        // Another icon in its place replaces it.
        let other = tauri::image::Image::new_owned(vec![255; 4], 1, 1);
        assert_eq!(icon_file(cache.path(), &other), Some(path.clone()));
        assert_eq!(std::fs::read(&path).unwrap(), png_of(&other).unwrap());
    }
}

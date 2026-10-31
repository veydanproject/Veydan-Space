// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Links the system hands to the app: a `veydan://…` clicked in a browser,
//! a mail or another app (internal/messenger-spec.md, «Ссылка из системы»).
//!
//! A product takes them when its Tauri config names a scheme of its own
//! (`plugins.deep-link`, `desktop.schemes`): Space and Chat do, Notes and
//! Pass do not; with both installed the system asks which one opens it. A computer starts the app with the link as an argument, and a
//! second start hands its arguments to the first (`single_instance`). A
//! phone hands it to an activity of its own (`LinkActivity` of the shell's
//! Android library), which keeps it in memory, tells the running app
//! (`Java_net_veydan_shell_IncomingLinks_arrived`) and brings it forward;
//! it is taken from there at the start, when told and each time the app
//! comes back.
//!
//! No plugin stands between: `tauri-plugin-deep-link` sends every link it
//! gets to the page as an event (`deep-link://new-url`), the key of a group
//! with it, before the app is unlocked and before anything here looks at it.
//!
//! The shell keeps the last link in memory only — never in a log, never on
//! disk: a link of a group or a channel carries its key. It brings the
//! window forward and tells the page with `EVENT_LINK`, without the link;
//! the module that reads links takes it (`take`) when it can show it, after
//! the app is unlocked. Nothing here opens anything.

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// Emitted with nothing when a link came: the page asks for it.
pub const EVENT_LINK: &str = "links://incoming";

/// More than any link the app reads (2048 bytes, `MAX_LINK_LEN` of the
/// messenger's links): a longer one is cut here and refused there as too
/// long. What the system hands over is never held whole.
const MAX_HELD: usize = 4096;

/// The prefixes this product takes, `veydan://`; none when it takes no link.
#[derive(Default)]
struct Schemes(Vec<String>);

/// The link that waits to be shown: the last one, one at most.
#[derive(Default)]
struct Inbox(Mutex<Option<String>>);

/// The schemes of the product: `plugins.deep-link.desktop` of its config,
/// one (`{ "schemes": [...] }`) or a list of them.
pub(crate) fn schemes(config: &tauri::Config) -> Vec<String> {
    let Some(desktop) = config.plugins.0.get("deep-link").and_then(|c| c.get("desktop")) else {
        return Vec::new();
    };
    let entries = match desktop {
        serde_json::Value::Array(list) => list.iter().collect(),
        one => vec![one],
    };
    entries
        .into_iter()
        .filter_map(|e| e.get("schemes")?.as_array())
        .flatten()
        .filter_map(|s| s.as_str())
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase()))
        .map(|s| format!("{s}://"))
        .collect()
}

/// `text` when it is a link of one of `schemes`, cut to `MAX_HELD` bytes.
fn accept(schemes: &[String], text: &str) -> Option<String> {
    if !schemes.iter().any(|s| text.len() > s.len() && text.starts_with(s.as_str())) {
        return None;
    }
    let mut end = text.len().min(MAX_HELD);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_string())
}

/// The last link among the arguments of a start, the program's name aside.
#[cfg(any(desktop, test))]
fn of_args(schemes: &[String], args: &[String]) -> Option<String> {
    args.iter().skip(1).rev().find_map(|a| accept(schemes, a))
}

/// Puts the state in place and takes the link the app was started with;
/// called once the plugins are set up. A product without a scheme gets an
/// inbox that stays empty.
pub(crate) fn setup(app: &AppHandle, schemes: Vec<String>) {
    let taking = !schemes.is_empty();
    app.manage(Schemes(schemes));
    app.manage(Inbox::default());
    if !taking {
        return;
    }
    #[cfg(target_os = "android")]
    let _ = RUNNING.set(app.clone());
    #[cfg(desktop)]
    {
        let args: Vec<String> = std::env::args().collect();
        received_args(app, &args);
        // An AppImage is installed by nobody: it tells the desktop itself
        // that it opens the scheme. A package does that by its desktop entry;
        // an independent profile (`--workdir`) does not take it over.
        #[cfg(target_os = "linux")]
        if let Some(appimage) = app.env().appimage {
            if crate::workdir::current().is_none() {
                let schemes = app.state::<Schemes>().0.clone();
                let name = app.config().product_name.clone().unwrap_or_default();
                // Commands are run: not on the start's way.
                std::thread::spawn(move || {
                    if let Err(e) = register(&appimage, &name, &schemes) {
                        eprintln!("links: the scheme handler is not registered: {e}");
                    }
                });
            }
        }
    }
}

/// The arguments of a start of the app (on a computer): the first start's,
/// or a second start's handed over by `single_instance`.
#[cfg(desktop)]
pub(crate) fn received_args(app: &AppHandle, args: &[String]) {
    let Some(schemes) = app.try_state::<Schemes>() else {
        return;
    };
    if let Some(link) = of_args(&schemes.0, args) {
        hold(app, link);
    }
}

/// The app came back to the front (`WindowEvent::Resumed`): a link clicked
/// outside may wait with `LinkActivity`. The page is told, without the
/// link; `take` gets it from there. Nothing is asked of Java here: this runs
/// on the thread the activity's calls run on.
#[cfg(target_os = "android")]
pub(crate) fn resumed(app: &AppHandle) {
    if app.try_state::<Schemes>().is_some_and(|s| !s.0.is_empty()) {
        let _ = app.emit(EVENT_LINK, ());
    }
}

/// The app that takes links, for `LinkActivity` to tell: set once it can.
#[cfg(target_os = "android")]
static RUNNING: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// `IncomingLinks.arrived()` of the shell's Android library: `LinkActivity`
/// put a link aside while the app runs. The page is told, without the link.
/// `Resumed` alone is not enough: the activity is paused and resumed within
/// milliseconds and the event loop may see only the pause.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_net_veydan_shell_IncomingLinks_arrived<'l>(_env: jni::JNIEnv<'l>, _class: jni::objects::JClass<'l>) {
    if let Some(app) = RUNNING.get() {
        resumed(app);
    }
}

/// The link `LinkActivity` keeps, if one waits, moved to the inbox. Anything
/// not of the product's schemes is dropped.
#[cfg(target_os = "android")]
fn from_activity(app: &AppHandle) {
    let (Some(schemes), Some(inbox)) = (app.try_state::<Schemes>(), app.try_state::<Inbox>()) else {
        return;
    };
    if schemes.0.is_empty() {
        return;
    }
    if let Some(link) = waiting_android().and_then(|text| accept(&schemes.0, &text)) {
        *inbox.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    }
}

/// `IncomingLinks.take()` of the shell's Android library, through the class
/// loader of the activity: the class is the app's, not the system's.
#[cfg(target_os = "android")]
fn waiting_android() -> Option<String> {
    use jni::objects::{JClass, JString, JValue};
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::wry::prelude::dispatch(move |env, activity, _webview| {
        let mut ask = || -> jni::errors::Result<Option<String>> {
            let loader = env
                .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
                .l()?;
            let name = env.new_string("net.veydan.shell.IncomingLinks")?;
            let class = env
                .call_method(
                    &loader,
                    "loadClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let link = env
                .call_static_method(JClass::from(class), "take", "()Ljava/lang/String;", &[])?
                .l()?;
            if link.is_null() {
                return Ok(None);
            }
            let link = JString::from(link);
            let text: String = env.get_string(&link)?.into();
            Ok(Some(text))
        };
        let answer = ask().ok().flatten();
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_clear();
        }
        let _ = tx.send(answer);
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).ok().flatten()
}

/// The desktop entry that opens the schemes with the AppImage at `exec`:
/// `<name>-handler.desktop` among the user's applications, made the default
/// of each scheme. Nothing is written for a path the entry cannot quote.
#[cfg(target_os = "linux")]
fn register(exec: &std::ffi::OsStr, name: &str, schemes: &[String]) -> Result<(), String> {
    let exec = exec.to_str().ok_or("the path of the AppImage is not UTF-8")?;
    if exec.chars().any(|c| matches!(c, '"' | '`' | '$' | '\\' | '\n' | '\r')) {
        return Err("the path of the AppImage cannot be quoted".into());
    }
    let file = std::path::Path::new(exec)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| format!("{f}-handler.desktop"))
        .ok_or("the AppImage has no file name")?;
    let mimes: Vec<String> = schemes
        .iter()
        .map(|s| format!("x-scheme-handler/{}", s.trim_end_matches("://")))
        .collect();
    let entry = desktop_entry(name, exec, &mimes);
    let home = std::env::var_os("HOME").ok_or("no HOME")?;
    let dir = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(&home).join(".local/share"))
        .join("applications");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&file);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
        std::fs::write(&path, entry).map_err(|e| e.to_string())?;
    }
    let run = |program: &str, args: &[&str]| {
        std::process::Command::new(program)
            .args(args)
            .status()
            .map_err(|e| format!("{program}: {e}"))
    };
    run("update-desktop-database", &[&dir.to_string_lossy()])?;
    for mime in &mimes {
        run("xdg-mime", &["default", &file, mime])?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry(name: &str, exec: &str, mimes: &[String]) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let mimes: String = mimes.iter().map(|m| format!("{m};")).collect();
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{exec}\" %u\nTerminal=false\nMimeType={mimes}\nNoDisplay=true\n"
    )
}

#[cfg(desktop)]
fn hold(app: &AppHandle, link: String) {
    let Some(inbox) = app.try_state::<Inbox>() else {
        return;
    };
    *inbox.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    #[cfg(desktop)]
    crate::tray::show_main_window(app);
    let _ = app.emit(EVENT_LINK, ());
}

/// The link that waits, taken: the next call gets nothing until another
/// comes. Asked by the module that shows links, when it can show one; not
/// on the thread of the activity (a phone's link is asked of Java).
pub fn take(app: &AppHandle) -> Option<String> {
    #[cfg(target_os = "android")]
    from_activity(app);
    let inbox = app.try_state::<Inbox>()?;
    let link = inbox.0.lock().unwrap_or_else(|e| e.into_inner()).take();
    link
}

#[cfg(test)]
mod tests {
    use super::*;

    fn veydan() -> Vec<String> {
        vec!["veydan://".to_string()]
    }

    fn config(plugins: serde_json::Value) -> tauri::Config {
        serde_json::from_value(serde_json::json!({ "identifier": "net.veydan.probe", "plugins": plugins })).unwrap()
    }

    #[test]
    fn the_schemes_are_read_from_the_config_of_the_product() {
        let one = config(serde_json::json!({ "deep-link": { "desktop": { "schemes": ["veydan"] } } }));
        assert_eq!(schemes(&one), veydan());
        let list = config(serde_json::json!({ "deep-link": { "desktop": [{ "schemes": ["veydan"] }, { "schemes": ["other"] }] } }));
        assert_eq!(schemes(&list), vec!["veydan://".to_string(), "other://".to_string()]);
        // A product without the plugin's section takes no link.
        assert!(schemes(&config(serde_json::json!({}))).is_empty());
        // A scheme is a word of small latin letters: nothing else becomes a prefix.
        let odd = config(serde_json::json!({ "deep-link": { "desktop": { "schemes": ["", "Bad", "ht:tp"] } } }));
        assert!(schemes(&odd).is_empty());
    }

    #[test]
    fn only_a_link_of_the_schemes_is_taken() {
        let s = veydan();
        assert_eq!(accept(&s, "veydan://contact/npub1x").as_deref(), Some("veydan://contact/npub1x"));
        assert_eq!(accept(&s, "https://veydan.net/"), None);
        assert_eq!(accept(&s, "veydan://"), None);
        assert_eq!(accept(&s, "--workdir"), None);
        assert_eq!(accept(&[], "veydan://contact/npub1x"), None);
    }

    #[test]
    fn a_long_link_is_cut_on_a_character() {
        let s = veydan();
        let long = format!("veydan://group/a?n={}", "я".repeat(MAX_HELD));
        let held = accept(&s, &long).unwrap();
        assert!(held.len() <= MAX_HELD && held.len() > MAX_HELD - 2);
        assert!(long.starts_with(&held));
    }

    #[test]
    fn the_entry_of_an_appimage_opens_the_schemes_with_it() {
        let entry = desktop_entry("Veydan\nSpace", "/home/u/Apps/Veydan Space.AppImage", &["x-scheme-handler/veydan".into()]);
        assert!(entry.contains("\nName=VeydanSpace\n"));
        assert!(entry.contains("\nExec=\"/home/u/Apps/Veydan Space.AppImage\" %u\n"));
        assert!(entry.contains("\nMimeType=x-scheme-handler/veydan;\n"));
    }

    #[test]
    fn the_last_link_among_the_arguments_is_taken() {
        let s = veydan();
        let args = |v: &[&str]| v.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(of_args(&s, &args(&["veydanspace"])), None);
        // The program's own name is never a link, whatever it is called.
        assert_eq!(of_args(&s, &args(&["veydan://contact/a"])), None);
        assert_eq!(of_args(&s, &args(&["veydanspace", "veydan://contact/a"])).as_deref(), Some("veydan://contact/a"));
        assert_eq!(
            of_args(&s, &args(&["veydanspace", "--workdir", "/tmp/p", "veydan://contact/a", "veydan://group/b"])).as_deref(),
            Some("veydan://group/b")
        );
    }
}

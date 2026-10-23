// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The keeper while the app is not on the screen of a phone
//! (internal/communities-stage1.md §4, the owner's decisions of
//! 2026-10-22): a foreground service while joins stream in or an epoch
//! waits, held on by the owner's switch of a channel, and a periodic wake
//! as the net under the push (`wake`). The phone's side is the push
//! plugin's (`KeeperService`, `KeeperJob`); a desktop has no phone, and
//! there the loop runs as long as the app.
//!
//! The rule of the service is a pure function, [`next`]: on at
//! [`BUSY_NOTES`] notes to a keeper within [`NOTES_WINDOW`], at an epoch
//! waiting, or by the switch; off once nothing came and nothing waits
//! for [`IDLE_OFF`]. No wake lock beyond the service itself.

use async_trait::async_trait;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Notes to a keeper within [`NOTES_WINDOW`] that start the service.
pub(crate) const BUSY_NOTES: usize = 5;
/// The window the notes are counted in, seconds.
pub(crate) const NOTES_WINDOW: u64 = 60;
/// How long the queue stays empty before the service stops, seconds.
pub(crate) const IDLE_OFF: u64 = 120;
/// How often the rule is read again while the module runs on a phone.
pub(crate) const RECHECK_SECS: u64 = 30;
/// The file of the owner's switches, in the module's folder.
const KEEP_ON_FILE: &str = "keeper-background.json";

/// What the service is and why.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fg {
    /// The service is wanted.
    pub(crate) on: bool,
    /// Since when nothing came and nothing waits, while it is on.
    pub(crate) idle_since: Option<u64>,
}

/// The work of the keepers kept here, as the rule reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Load {
    /// A channel is kept on this device.
    pub(crate) kept: bool,
    /// Notes to a keeper within the window.
    pub(crate) notes: usize,
    /// An epoch built and waiting for its hosts, or asked for now.
    pub(crate) epoch: bool,
    /// The owner keeps the keeper on for a channel kept here.
    pub(crate) keep_on: bool,
}

/// The service after `load` at `now`: on at once when busy or held on,
/// off only after [`IDLE_OFF`] seconds of nothing (a single note in
/// between puts the count back), at once when nothing is kept any more.
pub(crate) fn next(fg: Fg, load: Load, now: u64) -> Fg {
    if !load.kept {
        return Fg::default();
    }
    if load.keep_on || load.epoch || load.notes >= BUSY_NOTES {
        return Fg { on: true, idle_since: None };
    }
    if !fg.on {
        return fg;
    }
    if load.notes > 0 {
        return Fg { on: true, idle_since: None };
    }
    let since = fg.idle_since.unwrap_or(now);
    if now.saturating_sub(since) >= IDLE_OFF {
        Fg::default()
    } else {
        Fg { on: true, idle_since: Some(since) }
    }
}

/// The phone's side, a plugin's commands (`tauri-plugin-veydan-push` on
/// Android); a fake in the tests.
#[async_trait]
pub(crate) trait Phone: Send + Sync {
    /// Starts or stops the foreground service; whether it runs after.
    async fn foreground(&self, on: bool) -> bool;
    /// Registers or cancels the periodic wake (every 15 minutes).
    async fn periodic(&self, on: bool);
}

#[derive(Default)]
struct Inner {
    fg: Fg,
    /// The service runs, as the phone last said.
    running: bool,
    /// When notes to a keeper came, unix seconds, within the window.
    notes: VecDeque<u64>,
    /// The keepers' keys of the channels kept here (hex).
    keepers: HashSet<String>,
    /// The periodic wake as last asked of the phone.
    periodic: Option<bool>,
}

/// The state of the background on a phone; nothing on a desktop.
pub(crate) struct Background {
    phone: Option<Arc<dyn Phone>>,
    inner: Mutex<Inner>,
    /// The channels whose keeper the owner keeps on, kept in `file`.
    keep_on: Mutex<HashSet<String>>,
    file: PathBuf,
    /// One change of the phone at a time: the answers come in order.
    calls: tokio::sync::Mutex<()>,
}

fn hold<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Background {
    /// `dir`: the module's folder, where the switches are kept.
    pub(crate) fn new(phone: Option<Arc<dyn Phone>>, dir: &Path) -> Self {
        let file = dir.join(KEEP_ON_FILE);
        let keep_on = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok()).unwrap_or_default();
        Self { phone, inner: Mutex::default(), keep_on: Mutex::new(keep_on.into_iter().collect()), file, calls: Default::default() }
    }

    /// The phone runs the keeper in the background: the switch is shown.
    pub(crate) fn supported(&self) -> bool {
        self.phone.is_some()
    }

    /// The service runs now.
    pub(crate) fn running(&self) -> bool {
        hold(&self.inner).running
    }

    pub(crate) fn keep_on(&self, community_id: &str) -> bool {
        hold(&self.keep_on).contains(community_id)
    }

    /// The owner's switch of a channel, kept across starts.
    pub(crate) fn set_keep_on(&self, community_id: &str, on: bool) -> std::io::Result<()> {
        let list: Vec<String> = {
            let mut set = hold(&self.keep_on);
            if on {
                set.insert(community_id.to_string());
            } else {
                set.remove(community_id);
            }
            let mut list: Vec<String> = set.iter().cloned().collect();
            list.sort();
            list
        };
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, serde_json::to_vec(&list).map_err(std::io::Error::other)?)
    }

    /// The keys of the keepers kept here, for telling the notes to them.
    pub(crate) fn set_keepers(&self, keepers: HashSet<String>) {
        hold(&self.inner).keepers = keepers;
    }

    /// A wrap came: counted when it is addressed (`p`) to a keeper kept
    /// here. Whether it was.
    pub(crate) fn heard(&self, wrap: &serde_json::Value, now: u64) -> bool {
        let mut inner = hold(&self.inner);
        if inner.keepers.is_empty() {
            return false;
        }
        let to_keeper = wrap["tags"]
            .as_array()
            .is_some_and(|tags| tags.iter().any(|t| t[0] == "p" && t[1].as_str().is_some_and(|p| inner.keepers.contains(p))));
        if to_keeper {
            inner.notes.push_back(now);
        }
        to_keeper
    }

    /// The rule read again with the channels `kept` here and whether an
    /// epoch waits among them; the phone told what changed. `again`: the
    /// service is asked for once more though nothing changed (the system
    /// may have ended it: the time limit of a data service).
    pub(crate) async fn check(&self, kept: &[String], epoch: bool, now: u64, again: bool) {
        let Some(phone) = self.phone.clone() else { return };
        let _calls = self.calls.lock().await;
        let keep_on = {
            let set = hold(&self.keep_on);
            kept.iter().any(|c| set.contains(c))
        };
        let (fg, running, periodic) = {
            let mut inner = hold(&self.inner);
            while inner.notes.front().is_some_and(|t| *t + NOTES_WINDOW <= now) {
                inner.notes.pop_front();
            }
            let load = Load { kept: !kept.is_empty(), notes: inner.notes.len(), epoch, keep_on };
            inner.fg = next(inner.fg, load, now);
            (inner.fg, inner.running, inner.periodic)
        };
        let want_periodic = !kept.is_empty();
        if periodic != Some(want_periodic) {
            phone.periodic(want_periodic).await;
            hold(&self.inner).periodic = Some(want_periodic);
        }
        if fg.on != running || (fg.on && again) {
            let now_running = phone.foreground(fg.on).await;
            hold(&self.inner).running = now_running;
        }
    }

    /// The module stops: the service ends and the periodic wake goes.
    pub(crate) async fn close(&self) {
        let Some(phone) = self.phone.clone() else { return };
        let _calls = self.calls.lock().await;
        phone.foreground(false).await;
        phone.periodic(false).await;
        *hold(&self.inner) = Inner { periodic: Some(false), ..Inner::default() };
    }
}

/// The push plugin on Android: its keeper's commands, and its wake told
/// to the loop.
#[cfg(target_os = "android")]
pub(crate) mod android {
    use super::Phone;
    use async_trait::async_trait;
    use tauri::{AppHandle, Manager};
    use tauri_plugin_veydan_push::{Event, VeydanPush};

    pub(crate) struct PushPlugin(pub(crate) AppHandle);

    #[async_trait]
    impl Phone for PushPlugin {
        async fn foreground(&self, on: bool) -> bool {
            let Some(push) = self.0.try_state::<VeydanPush<tauri::Wry>>() else { return false };
            match push.keeper_foreground(on).await {
                Ok(running) => running,
                Err(e) => {
                    crate::log::clog!("the keeper's service was not {}: {e}", if on { "started" } else { "stopped" });
                    false
                }
            }
        }

        async fn periodic(&self, on: bool) {
            let Some(push) = self.0.try_state::<VeydanPush<tauri::Wry>>() else { return };
            if let Err(e) = push.keeper_periodic(on).await {
                crate::log::clog!("the keeper's periodic wake was not {}: {e}", if on { "set" } else { "cancelled" });
            }
        }
    }

    /// The phone, when the product has the push plugin.
    pub(crate) fn phone(app: &AppHandle) -> Option<std::sync::Arc<dyn Phone>> {
        app.try_state::<VeydanPush<tauri::Wry>>()?;
        Some(std::sync::Arc::new(PushPlugin(app.clone())))
    }

    /// A push for the keeper (`channel`), or the periodic wake: one turn of
    /// the keepers, and of the listening only what the subscriptions need
    /// (`Listener::woken`).
    pub(crate) async fn listen(app: &AppHandle) {
        let Some(push) = app.try_state::<VeydanPush<tauri::Wry>>() else { return };
        let handle = app.clone();
        let heard = push
            .listen(Event::KeeperWake, move |why| {
                // The plugin calls on a thread of Java, outside the runtime:
                // the listener's round spawned from there aborted the app at
                // every periodic wake (живой запуск 2026-10-23, «there is no
                // reactor running» in sendChannelData), and the keeper on the
                // phone never ran with the app off the screen.
                let why = why["why"].as_str().unwrap_or("?").to_string();
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let driver = handle.state::<crate::CommunitiesState>().driver();
                    if let Ok(driver) = driver {
                        crate::log::clog!("the keeper woken ({why})");
                        driver.wake_now();
                        // Not a round of listening: one is made only when
                        // the subscriptions are not up (`Listener::woken`).
                        driver.woken().await;
                    }
                });
            })
            .await;
        if let Err(e) = heard {
            crate::log::clog!("the keeper's wake is not heard: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: Load = Load { kept: true, notes: BUSY_NOTES, epoch: false, keep_on: false };
    const QUIET: Load = Load { kept: true, notes: 0, epoch: false, keep_on: false };

    /// The rule: busy at five notes or an epoch waiting, held on by the
    /// switch; four notes start nothing; once on, it stays through a few
    /// notes and goes two minutes after the last thing; nothing kept, off.
    #[test]
    fn the_service_starts_when_busy_and_stops_after_two_quiet_minutes() {
        let off = Fg::default();
        assert!(!next(off, Load { notes: BUSY_NOTES - 1, ..QUIET }, 0).on, "four joins start nothing");
        assert!(next(off, BUSY, 0).on);
        assert!(next(off, Load { epoch: true, ..QUIET }, 0).on, "an epoch waiting");
        assert!(next(off, Load { keep_on: true, ..QUIET }, 0).on, "the owner's switch");
        assert_eq!(next(off, QUIET, 0), off);

        let on = next(off, BUSY, 1000);
        // A few notes keep it on and put the quiet back.
        let after = next(on, QUIET, 1010);
        assert_eq!(after, Fg { on: true, idle_since: Some(1010) });
        let again = next(after, Load { notes: 1, ..QUIET }, 1100);
        assert_eq!(again, Fg { on: true, idle_since: None });
        let quiet = next(again, QUIET, 1110);
        assert!(next(quiet, QUIET, 1110 + IDLE_OFF - 1).on, "not before two minutes");
        assert_eq!(next(quiet, QUIET, 1110 + IDLE_OFF), off, "two quiet minutes");
        // Held on: never off while the switch is.
        let held = Load { keep_on: true, ..QUIET };
        assert!(next(next(off, held, 0), held, 10_000).on);
        // Nothing kept here any more: off at once, whatever it was.
        assert_eq!(next(on, Load { kept: false, keep_on: true, ..BUSY }, 1001), off);
    }

    /// The phone, as the plugin would be told.
    #[derive(Default)]
    struct FakePhone {
        calls: Mutex<Vec<String>>,
        refuse: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl Phone for FakePhone {
        async fn foreground(&self, on: bool) -> bool {
            hold(&self.calls).push(format!("foreground {on}"));
            on && !self.refuse.load(std::sync::atomic::Ordering::SeqCst)
        }
        async fn periodic(&self, on: bool) {
            hold(&self.calls).push(format!("periodic {on}"));
        }
    }

    fn calls(p: &FakePhone) -> Vec<String> {
        std::mem::take(&mut *hold(&p.calls))
    }

    fn wrap_to(p: &str) -> serde_json::Value {
        serde_json::json!({ "kind": 1059, "tags": [["p", p]] })
    }

    /// The periodic wake is set while a channel is kept here and cancelled
    /// at a stop; the service starts at the fifth note to a keeper (a note
    /// to anybody else counts for nothing), ends after the quiet, is asked
    /// again when the system refused it, and ends at a stop.
    #[tokio::test]
    async fn the_phone_is_told_of_the_periodic_wake_and_of_the_service() {
        let dir = std::env::temp_dir().join(format!("communities-bg-{}", std::process::id()));
        let phone = Arc::new(FakePhone::default());
        let bg = Background::new(Some(phone.clone()), &dir);
        let kept = vec!["c1".to_string()];
        let keeper = "aa".repeat(32);
        bg.set_keepers([keeper.clone()].into_iter().collect());

        bg.check(&[], false, 0, false).await;
        assert_eq!(calls(&phone), ["periodic false"], "nothing kept: no periodic wake");
        bg.check(&kept, false, 0, false).await;
        assert_eq!(calls(&phone), ["periodic true"]);
        bg.check(&kept, false, 1, false).await;
        assert!(calls(&phone).is_empty(), "asked once");

        assert!(!bg.heard(&wrap_to(&"bb".repeat(32)), 10));
        for i in 0..BUSY_NOTES as u64 - 1 {
            assert!(bg.heard(&wrap_to(&keeper), 10 + i));
            bg.check(&kept, false, 10 + i, false).await;
        }
        assert!(calls(&phone).is_empty(), "four notes start nothing");
        bg.heard(&wrap_to(&keeper), 20);
        bg.check(&kept, false, 20, false).await;
        assert_eq!(calls(&phone), ["foreground true"]);
        assert!(bg.running());

        // The notes leave the window, the quiet runs out.
        bg.check(&kept, false, 20 + NOTES_WINDOW, false).await;
        bg.check(&kept, false, 20 + NOTES_WINDOW + IDLE_OFF, false).await;
        assert_eq!(calls(&phone), ["foreground false"]);
        assert!(!bg.running());

        // An epoch waits, and the system refuses the service from the
        // background: it is asked again at the next look, not before.
        phone.refuse.store(true, std::sync::atomic::Ordering::SeqCst);
        bg.check(&kept, true, 1000, false).await;
        assert_eq!(calls(&phone), ["foreground true"]);
        phone.refuse.store(false, std::sync::atomic::Ordering::SeqCst);
        bg.check(&kept, true, 1030, true).await;
        assert_eq!(calls(&phone), ["foreground true"]);
        assert!(bg.running());

        bg.close().await;
        assert_eq!(calls(&phone), ["foreground false", "periodic false"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The owner's switch is kept per channel across starts, and holds the
    /// service on only for a channel kept here.
    #[tokio::test]
    async fn the_switch_is_kept_per_channel() {
        let dir = std::env::temp_dir().join(format!("communities-bg-switch-{}", std::process::id()));
        let phone = Arc::new(FakePhone::default());
        let bg = Background::new(Some(phone.clone()), &dir);
        bg.set_keep_on("c1", true).unwrap();
        bg.set_keep_on("c2", true).unwrap();
        bg.set_keep_on("c2", false).unwrap();
        let again = Background::new(None, &dir);
        assert!(again.keep_on("c1") && !again.keep_on("c2"));
        assert!(!again.supported() && bg.supported());

        bg.check(&["c2".to_string()], false, 0, false).await;
        assert_eq!(calls(&phone), ["periodic true"], "the switch of another channel holds nothing");
        bg.check(&["c1".to_string()], false, 1, false).await;
        assert_eq!(calls(&phone), ["foreground true"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A desktop has no phone: nothing is asked, nothing runs.
    #[tokio::test]
    async fn a_desktop_asks_nothing() {
        let bg = Background::new(None, &std::env::temp_dir().join("communities-bg-none"));
        bg.check(&["c1".to_string()], true, 0, true).await;
        bg.close().await;
        assert!(!bg.running() && !bg.supported());
    }
}

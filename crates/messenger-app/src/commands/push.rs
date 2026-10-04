// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Push notifications: what stands between the messenger and the phone.
//!
//! Two sides meet here. The phone's side (the push plugin) knows the
//! permission and the phone's address at the push service. The messenger's
//! side (the runtime) knows what to tell the push server and when. This
//! file passes the one to the other and keeps no logic of its own.
//!
//! The web page never talks to the push plugin. It calls the commands here,
//! and they call the plugin from Rust. A capability naming the plugin would
//! make every Android build without the messenger fail, the release one
//! included; this way the capabilities stay as they are.
//!
//! On a desktop the commands exist and say that pushes are not supported.

use super::{map_err, MessengerState};
use messenger_runtime::push::PushStatus;
use serde::Serialize;
use veydan_core::{AppError, CmdResult};
use veydan_lock::Lock;

#[cfg(target_os = "android")]
use messenger_runtime::push::PushChannel;
#[cfg(target_os = "android")]
use tauri_plugin_veydan_push::{Event, Permission, VeydanPush};

/// Emitted with nothing when a notification was tapped while the app was
/// running (Android). The page answers by asking `messenger_push_take_tap`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const EVENT_TAP: &str = "messenger://push-tap";

/// What the phone says about pushes.
#[derive(Debug, Clone, Serialize)]
pub struct PushDevice {
    /// False on a desktop and in a build without the push bridge.
    pub supported: bool,
    /// False when this phone cannot receive pushes; `reason` says why.
    pub available: bool,
    /// `no_firebase_config`, `no_play_services`, `token_failed`.
    pub reason: Option<String>,
    pub detail: Option<String>,
    /// `granted`, `denied`, `prompt`, `prompt-with-rationale`.
    pub permission: Option<String>,
}

impl PushDevice {
    #[cfg(not(target_os = "android"))]
    fn unsupported() -> Self {
        Self {
            supported: false,
            available: false,
            reason: None,
            detail: None,
            permission: None,
        }
    }
}

/// Everything the settings screen shows: the phone's side and the server's.
#[derive(Debug, Clone, Serialize)]
pub struct PushView {
    pub device: PushDevice,
    pub status: PushStatus,
}

/// A notification the user tapped.
#[derive(Debug, Clone, Serialize)]
pub struct PushTap {
    #[serde(rename = "type")]
    pub kind: String,
    /// `group:<id>` when the push was about a group; a direct message names
    /// no chat, because the server does not know who wrote it.
    pub chat: Option<String>,
}

#[cfg(target_os = "android")]
fn bridge(app: &tauri::AppHandle) -> CmdResult<tauri::State<'_, VeydanPush<tauri::Wry>>> {
    use tauri::Manager;
    app.try_state::<VeydanPush<tauri::Wry>>()
        .ok_or_else(|| AppError::Other("push bridge is not loaded".into()))
}

#[cfg(target_os = "android")]
fn failed(e: tauri_plugin_veydan_push::Error) -> AppError {
    AppError::Other(e.to_string())
}

#[cfg(target_os = "android")]
fn permission_name(p: Permission) -> Option<String> {
    serde_json::to_value(p).ok()?.as_str().map(str::to_string)
}

#[cfg(target_os = "android")]
async fn device(app: &tauri::AppHandle) -> CmdResult<PushDevice> {
    let s = bridge(app)?.state().await.map_err(failed)?;
    Ok(PushDevice {
        supported: true,
        available: s.available,
        reason: s.reason,
        detail: None,
        permission: permission_name(s.permission),
    })
}

#[cfg(not(target_os = "android"))]
async fn device(_app: &tauri::AppHandle) -> CmdResult<PushDevice> {
    Ok(PushDevice::unsupported())
}

/// The plugin keeps a listener for as long as the app runs: they are
/// registered at the first start of the messenger in the process, and a
/// later start finds them in place.
#[cfg(target_os = "android")]
static LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Starts what runs for as long as the runtime does; the tasks are aborted
/// when the messenger stops.
///
/// - The mute of the last stop is lifted.
/// - Taps are passed to the page.
/// - A new address at the push service is passed to the runtime that runs.
/// - The plugin is told whether the messenger is receiving messages by
///   itself, in which case a push about a message is not shown while the
///   app is on the screen.
/// - The push handler is given the keys while the settings want it to.
/// - The runtime's own loop, which keeps the push server told.
#[cfg(target_os = "android")]
pub fn spawn_bridge(
    app: tauri::AppHandle,
    rt: std::sync::Arc<messenger_runtime::MessengerRuntime>,
) -> Vec<tauri::async_runtime::JoinHandle<()>> {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use tauri::Manager;

    let push_loop = tauri::async_runtime::spawn(rt.clone().push_loop());

    let bridge = tauri::async_runtime::spawn(async move {
        // The plugin is set up before the app's own setup ends, but this task
        // may start sooner than the window exists.
        let push = loop {
            if let Some(push) = app.try_state::<VeydanPush<tauri::Wry>>() {
                break push;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        // Muted by the last stop, maybe in an earlier process.
        if let Err(e) = push.set_muted(false).await {
            eprintln!("messenger push: the push handler stays muted: {e}");
        }
        if !LISTENING.swap(true, Ordering::SeqCst) {
            listen(&app, &push).await;
        } else {
            // A start after a stop: a token that changed meanwhile reached
            // no runtime. Asked only of a user who agreed to pushes.
            renew_token(&app, &push, &rt).await;
        }

        let context = async {
            let mut told: Option<bool> = None;
            loop {
                let live = match rt.status().await {
                    Ok(s) => s.session_active && !s.silent_mode && s.relays_connected > 0,
                    Err(_) => false,
                };
                if told != Some(live) && push.set_context(live).await.is_ok() {
                    told = Some(live);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        };
        tokio::join!(keep_handler_keys(app.clone(), rt.clone()), context);
    });
    vec![push_loop, bridge]
}

/// Taps go to the page; a new token goes to the runtime that runs. With the
/// messenger off there is none: the next start asks for the token.
#[cfg(target_os = "android")]
async fn listen(app: &tauri::AppHandle, push: &VeydanPush<tauri::Wry>) {
    use tauri::{Emitter, Manager};

    let page = app.clone();
    if let Err(e) = push
        .listen(Event::Tap, move |_| {
            let _ = page.emit(EVENT_TAP, ());
        })
        .await
    {
        eprintln!("messenger push: cannot listen for taps: {e}");
    }

    let tokens = app.clone();
    if let Err(e) = push
        .listen(Event::Token, move |payload| {
            let Some(token) = payload.get("token").and_then(|t| t.as_str()) else {
                return;
            };
            let Some(runtime) = tokens
                .try_state::<MessengerState>()
                .and_then(|messenger| messenger.runtime().ok())
            else {
                return;
            };
            let channel = channel(&tokens, token.to_string(), None);
            tauri::async_runtime::spawn(async move {
                // Kept whether pushes are on or not; told to the server
                // only when they are.
                if let Err(e) = runtime.push_set_channel(Some(channel)).await {
                    eprintln!("messenger push: the new token was not kept: {e}");
                    return;
                }
                if let Err(e) = runtime.push_reconcile(false).await {
                    eprintln!("messenger push: {e}");
                }
            });
        })
        .await
    {
        eprintln!("messenger push: cannot listen for tokens: {e}");
    }
}

/// The phone's address at the push service, as the runtime keeps it.
#[cfg(target_os = "android")]
fn channel(app: &tauri::AppHandle, token: String, app_id: Option<String>) -> PushChannel {
    PushChannel {
        provider: "fcm".into(),
        token,
        app_id: app_id.unwrap_or_else(|| app.config().identifier.clone()),
        app_version: Some(app.package_info().version.to_string()),
    }
}

/// Asks the push service for the token again and gives it to the runtime,
/// when the user agreed to pushes. The runtime's loop tells the server.
#[cfg(target_os = "android")]
async fn renew_token(
    app: &tauri::AppHandle,
    push: &VeydanPush<tauri::Wry>,
    rt: &messenger_runtime::MessengerRuntime,
) {
    if !rt.push_status().await.is_ok_and(|status| status.enabled) {
        return;
    }
    let answer = match push.token().await {
        Ok(answer) => answer,
        Err(e) => {
            eprintln!("messenger push: no token: {e}");
            return;
        }
    };
    let Some(token) = answer.token.filter(|_| answer.available) else {
        return;
    };
    if let Err(e) = rt.push_set_channel(Some(channel(app, token, answer.app_id))).await {
        eprintln!("messenger push: the token was not kept: {e}");
    }
}

/// The messenger stops: the push handler is muted first — it shows nothing,
/// whatever the push server still sends, and the mute outlives the process —
/// then the push server is told to forget the device while the session can
/// still sign the request, the push handler gives its keys back, and the
/// plugin no longer counts on the app receiving by itself. A server that is
/// not reached keeps pushing until the registration runs out, and the mute
/// holds them; the next start lifts it and registers again. The address at
/// the push service and the user's choices stay.
#[cfg(target_os = "android")]
pub async fn stop_bridge(app: &tauri::AppHandle, rt: &messenger_runtime::MessengerRuntime) {
    use tauri::Manager;
    let push = app.try_state::<VeydanPush<tauri::Wry>>();
    if let Some(push) = &push {
        if let Err(e) = push.set_muted(true).await {
            eprintln!("messenger push: the push handler was not muted: {e}");
        }
    }
    if let Err(e) = rt.push_unregister().await {
        eprintln!("messenger push: the registration was not taken back: {e}");
    }
    let Some(push) = push else { return };
    if let Err(e) = push.clear_keys().await {
        eprintln!("messenger notify: the handler's keys were not taken back: {e}");
    }
    let _ = push.set_context(false).await;
}

/// Asks nothing of the push service or the push server.
#[tauri::command]
pub async fn messenger_push_status(
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PushView> {
    let rt = messenger.runtime()?;
    Ok(PushView { device: device(&app).await?, status: rt.push_status().await.map_err(map_err)? })
}

/// Turning pushes on is the user's agreement: the permission is asked for,
/// then the phone's address at the push service, and then the push server
/// is told. Any of the three may fail; what failed is in the answer, and
/// pushes stay off.
///
/// Turning them off takes the registration back and gives the address up.
#[tauri::command]
pub async fn messenger_push_set_enabled(
    enabled: bool,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PushView> {
    let rt = messenger.runtime()?;
    rt.push_mark_offered().await.map_err(map_err)?;

    #[cfg(target_os = "android")]
    {
        let push = bridge(&app)?;
        if !enabled {
            let status = rt.push_set_enabled(false).await.map_err(map_err)?;
            rt.push_set_channel(None).await.map_err(map_err)?;
            if let Err(e) = push.delete_token().await {
                eprintln!("messenger push: the token was not given up: {e}");
            }
            return Ok(PushView { device: device(&app).await?, status });
        }

        let mut device = device(&app).await?;
        let stays_off = |device: PushDevice, rt: std::sync::Arc<messenger_runtime::MessengerRuntime>| async move {
            Ok(PushView { device, status: rt.push_status().await.map_err(map_err)? })
        };
        if !device.available {
            return stays_off(device, rt.clone()).await;
        }
        let permission = push.request_permission().await.map_err(failed)?;
        device.permission = permission_name(permission);
        if permission != Permission::Granted {
            return stays_off(device, rt.clone()).await;
        }
        let answer = push.token().await.map_err(failed)?;
        let Some(token) = answer.token.filter(|_| answer.available) else {
            device.available = false;
            device.reason = answer.reason.or(Some("token_failed".into()));
            device.detail = answer.detail;
            return stays_off(device, rt.clone()).await;
        };
        rt.push_set_channel(Some(channel(&app, token, answer.app_id)))
            .await
            .map_err(map_err)?;
        let status = rt.push_set_enabled(true).await.map_err(map_err)?;
        Ok(PushView { device, status })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = enabled;
        Ok(PushView {
            device: device(&app).await?,
            status: rt.push_status().await.map_err(map_err)?,
        })
    }
}

/// The user was asked about pushes and said "not now".
#[tauri::command]
pub async fn messenger_push_mark_offered(messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.push_mark_offered().await.map_err(map_err)
}

/// `None` goes back to the server of the manifest.
#[tauri::command]
pub async fn messenger_push_set_server(
    url: Option<String>,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PushView> {
    let rt = messenger.runtime()?;
    let status = rt.push_set_server(url).await.map_err(map_err)?;
    Ok(PushView { device: device(&app).await?, status })
}

#[tauri::command]
pub async fn messenger_push_set_prefs(
    dm: bool,
    groups: bool,
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PushView> {
    let rt = messenger.runtime()?;
    let status = rt.push_set_prefs(dm, groups).await.map_err(map_err)?;
    Ok(PushView { device: device(&app).await?, status })
}

/// Tells the push server again, now.
#[tauri::command]
pub async fn messenger_push_refresh(
    app: tauri::AppHandle,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<PushView> {
    let rt = messenger.runtime()?;
    let status = rt.push_reconcile(true).await.map_err(map_err)?;
    Ok(PushView { device: device(&app).await?, status })
}

/// What the push service answered to a test push: `delivered`, `dead_token`,
/// `rejected`, `retry`; and the id of the push in the server's log.
#[derive(Debug, Clone, Serialize)]
pub struct PushTest {
    pub outcome: String,
    pub trace: String,
}

#[tauri::command]
pub async fn messenger_push_test(messenger: tauri::State<'_, MessengerState>) -> CmdResult<PushTest> {
    let answer = messenger.runtime()?.push_test().await.map_err(map_err)?;
    Ok(PushTest { outcome: answer.outcome, trace: answer.trace })
}

/// The tapped notification, once. Asked at start, to learn about the tap
/// that opened the app, and after every `EVENT_TAP`.
#[tauri::command]
pub async fn messenger_push_take_tap(app: tauri::AppHandle) -> CmdResult<Option<PushTap>> {
    #[cfg(target_os = "android")]
    {
        let tap = bridge(&app)?.take_tap().await.map_err(failed)?;
        Ok(tap.map(|t| PushTap { kind: t.kind, chat: t.chat }))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(None)
    }
}

/// Removes shown notifications: of one chat (`dm`, `group:<id>`), or all
/// that are about messages.
#[tauri::command]
pub async fn messenger_push_clear(key: Option<String>, app: tauri::AppHandle) -> CmdResult<()> {
    #[cfg(target_os = "android")]
    {
        bridge(&app)?.cancel(key.as_deref()).await.map_err(failed)
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri::Manager;
        if let Some(messenger) = app.try_state::<MessengerState>() {
            messenger.notices_seen(key.as_deref());
        }
        Ok(())
    }
}

// Used on Android only; named here so that a desktop build does not warn.
#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn _unused(_: AppError) {}

// ─── The keys and the settings of the push handler ──────────────────────────

/// What a notification may say. Kept by the messenger; the push handler
/// reads it from there.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct NotifySettings {
    /// `sender_text` | `sender` | `none`
    pub content: String,
    pub lockscreen_hidden: bool,
    /// A PIN or password guards the app: the handler gets no keys, and a
    /// notification says only that something came, whatever `content` says.
    pub locked: bool,
}

#[cfg(target_os = "android")]
static KEYS_KICK: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// The lock, the settings of the notifications or the identity changed:
/// the handler's keys follow at once.
pub fn lock_changed() {
    #[cfg(target_os = "android")]
    KEYS_KICK.notify_one();
}

/// Gives the push handler the keys while the settings want it to have them
/// and no lock guards the app; takes them away otherwise. Runs for as long
/// as the runtime does, looking again every 20 s and whenever kicked.
#[cfg(target_os = "android")]
async fn keep_handler_keys(app: tauri::AppHandle, rt: std::sync::Arc<messenger_runtime::MessengerRuntime>) {
    use std::time::Duration;
    use tauri::Manager;

    let Some(push) = app.try_state::<VeydanPush<tauri::Wry>>() else { return };

    // What the handler has now: a fingerprint of the bundle, or nothing.
    // Unknown at start, so the first round always speaks.
    let mut given: Option<Option<String>> = None;
    loop {
        let wanted = match app.try_state::<Lock>() {
            Some(lock) => match rt.notify_wants_keys().await {
                Ok(w) => w && !lock.enabled().await,
                Err(e) => {
                    eprintln!("messenger notify: {e}");
                    false
                }
            },
            None => false,
        };
        let fingerprint = if wanted { rt.notify_fingerprint().await.unwrap_or(None) } else { None };
        if given.as_ref() != Some(&fingerprint) {
            let done = match &fingerprint {
                Some(_) => match rt.notify_bundle().await {
                    Ok(Some(bundle)) => push.store_keys(&bundle.to_json()).await.map_err(|e| e.to_string()),
                    Ok(None) => push.clear_keys().await.map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                },
                None => push.clear_keys().await.map_err(|e| e.to_string()),
            };
            match done {
                Ok(()) => given = Some(fingerprint),
                Err(e) => eprintln!("messenger notify: the handler's keys were not updated: {e}"),
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(20)) => {}
            _ = KEYS_KICK.notified() => {}
        }
    }
}

#[tauri::command]
pub async fn messenger_notify_get(
    lock: tauri::State<'_, Lock>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NotifySettings> {
    let rt = messenger.runtime()?;
    let s = rt.notify_settings().await.map_err(map_err)?;
    Ok(NotifySettings {
        content: s.content.as_str().into(),
        lockscreen_hidden: s.lockscreen_hidden,
        locked: lock.enabled().await,
    })
}

#[tauri::command]
pub async fn messenger_notify_set(
    content: String,
    lockscreen_hidden: bool,
    lock: tauri::State<'_, Lock>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<NotifySettings> {
    use messenger_notify::{Content, Settings};
    let rt = messenger.runtime()?;
    let content = Content::parse(&content).ok_or_else(|| AppError::Other("notify_content".into()))?;
    rt.notify_set_settings(Settings { content, lockscreen_hidden }).await.map_err(map_err)?;
    lock_changed();
    messenger_notify_get(lock, messenger).await
}

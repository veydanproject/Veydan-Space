// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Bridge to the phone's push service (FCM on Android, APNs on iOS).
//!
//! The phone's side owns everything that must work while the app is not
//! running. On Android (Kotlin, `android/`) it receives pushes and shows
//! notifications. On iOS (Swift, `ios/`) the system shows an alert push by
//! itself, in the words of the app's string `push.new` until the app has a
//! Notification Service Extension to write them; the Swift side gets the
//! token, keeps the tap and wakes the keeper for a background push. This
//! side lets the app ask for the token and the permission, tell whether the
//! user is looking at live data, and learn which notification was tapped;
//! both sides take the same commands with the same answers.
//!
//! The plugin knows nothing about the messenger; it carries strings.

#![cfg(any(target_os = "android", target_os = "ios"))]

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "net.veydan.push";

// The Swift side's entry (`@_cdecl("init_plugin_veydan_push")` in
// ios/Sources/VeydanPush/VeydanPushPlugin.swift), which makes the plugin.
#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_veydan_push);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("push bridge: {0}")]
    Invoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Answer to a request for the push token.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAnswer {
    /// False when the phone cannot receive pushes; `reason` says why.
    pub available: bool,
    /// `no_firebase_config`, `no_play_services`, `token_failed`; iOS says
    /// only `token_failed` (APNs did not answer, or refused: `detail`).
    pub reason: Option<String>,
    pub detail: Option<String>,
    /// FCM's token; on iOS the device token of APNs as hex.
    pub token: Option<String>,
    /// The Android package or the iOS bundle id, which is the app id the
    /// push server knows.
    pub app_id: Option<String>,
}

/// What is known about pushes on this phone before anything is asked of
/// the push service.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub available: bool,
    /// `no_firebase_config`, `no_play_services`; never on iOS, where APNs
    /// is part of the system.
    pub reason: Option<String>,
    pub permission: Permission,
    pub app_id: String,
    /// What keeps messages from ringing: `off` (notifications off for the
    /// app), `dm` or `groups` (that channel below high); none when they ring.
    /// iOS has no channels: `off` when notifications show nowhere, `dm`
    /// when they come without a sound or quietly (provisional).
    #[serde(default)]
    pub quiet: Option<String>,
}

/// May the app show notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Granted,
    Denied,
    /// Not asked yet.
    Prompt,
    PromptWithRationale,
}

#[derive(Debug, Deserialize)]
struct PermissionAnswer {
    state: Permission,
}

/// A notification the user tapped.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tap {
    /// `dm`, `group`, `test`, `broadcast`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The chat to open, when the push named one: `group:<id>`.
    pub chat: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TapAnswer {
    tap: Option<Tap>,
}

#[derive(Debug, Deserialize)]
struct RunningAnswer {
    running: bool,
}

/// What the phone's side reports without being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The push service gave the app a new token: `{ "token": … }`. On iOS
    /// also the token of a registration renewed at the start, when it
    /// changed; it waits for the first listener.
    Token,
    /// A notification was tapped while the app was running.
    Tap,
    /// The keeper of a channel is to take a turn: a push marked `channel`
    /// came, or the periodic wake ran (`Keeper.kt`): `{ "why": "push" | "periodic" }`.
    /// On iOS only a background push of a note to the keeper, while the
    /// process lives: no periodic wake yet.
    KeeperWake,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Tap => "tap",
            Self::KeeperWake => "keeper_wake",
        }
    }
}

/// Access to the push bridge.
pub struct VeydanPush<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> VeydanPush<R> {
    async fn call<T: DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T> {
        Ok(self.0.run_mobile_plugin_async(command, payload).await?)
    }

    /// Can this phone receive pushes, and may the app show them. Asks
    /// nothing of the push service.
    pub async fn state(&self) -> Result<State> {
        self.call("getState", ()).await
    }

    /// Asks the push service for the token. This is the first contact with
    /// the service, so it is made only after the user agreed to pushes.
    pub async fn token(&self) -> Result<TokenAnswer> {
        self.call("getToken", ()).await
    }

    /// Gives the token up: the push service stops delivering to this app.
    pub async fn delete_token(&self) -> Result<()> {
        self.call::<serde_json::Value>("deleteToken", ()).await.map(|_| ())
    }

    pub async fn permission(&self) -> Result<Permission> {
        Ok(self.call::<PermissionAnswer>("permissionState", ()).await?.state)
    }

    /// Opens the system settings of what [`State::quiet`] names: the quiet
    /// channel, or the app's notifications.
    pub async fn open_settings(&self) -> Result<()> {
        self.call::<serde_json::Value>("openSettings", ()).await.map(|_| ())
    }

    /// Shows the system question when it may still be shown.
    pub async fn request_permission(&self) -> Result<Permission> {
        Ok(self.call::<PermissionAnswer>("requestPermission", ()).await?.state)
    }

    /// `live`: the app receives messages by itself right now. While it is on
    /// the screen and live, pushes about messages are not shown.
    pub async fn set_context(&self, live: bool) -> Result<()> {
        self.call::<serde_json::Value>("setContext", serde_json::json!({ "live": live }))
            .await
            .map(|_| ())
    }

    /// `muted`: the app wants no push shown, whatever the push server still
    /// sends — the part of the app the pushes are for is switched off. Kept
    /// on the phone, also for the process a push starts.
    pub async fn set_muted(&self, muted: bool) -> Result<()> {
        self.call::<serde_json::Value>("setMuted", serde_json::json!({ "muted": muted }))
            .await
            .map(|_| ())
    }

    /// The tapped notification, once. Also the way to learn about a tap
    /// that started the app, which no event could report.
    pub async fn take_tap(&self) -> Result<Option<Tap>> {
        Ok(self.call::<TapAnswer>("takeTap", ()).await?.tap)
    }

    /// Removes shown notifications: of one chat (`dm`, `group:<id>`) or all.
    pub async fn cancel(&self, key: Option<&str>) -> Result<()> {
        self.call::<serde_json::Value>("cancel", serde_json::json!({ "key": key }))
            .await
            .map(|_| ())
    }

    /// Hands the keys the push handler opens messages with to the phone's
    /// key store, sealed, for the process a push starts. Base64 of the
    /// bundle; what is in it is the app's business.
    pub async fn store_keys(&self, bundle: &[u8]) -> Result<()> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bundle);
        self.call::<serde_json::Value>("storeKeys", serde_json::json!({ "bundle": b64 }))
            .await
            .map(|_| ())
    }

    /// Takes the keys away from the push handler.
    pub async fn clear_keys(&self) -> Result<()> {
        self.call::<serde_json::Value>("clearKeys", ()).await.map(|_| ())
    }

    /// Starts (`on`) or stops the foreground service of the keeper of the
    /// channels, with its notification «Хранитель канала работает».
    /// Answers whether it runs: the system refuses a start from the
    /// background outside a push and a few other cases. iOS has no such
    /// service: never running there.
    pub async fn keeper_foreground(&self, on: bool) -> Result<bool> {
        Ok(self.call::<RunningAnswer>("keeperForeground", serde_json::json!({ "on": on })).await?.running)
    }

    /// Registers (`on`) or cancels the keeper's periodic wake: every 15
    /// minutes, the least the system allows, as [`Event::KeeperWake`].
    /// Nothing on iOS yet (a background task of the system, later).
    pub async fn keeper_periodic(&self, on: bool) -> Result<()> {
        self.call::<serde_json::Value>("keeperPeriodic", serde_json::json!({ "on": on }))
            .await
            .map(|_| ())
    }

    /// Calls `handler` on every `event`, for as long as the app runs.
    pub async fn listen<F>(&self, event: Event, handler: F) -> Result<()>
    where
        F: Fn(serde_json::Value) + Send + Sync + 'static,
    {
        let channel: Channel<serde_json::Value> = Channel::new(move |body| {
            if let InvokeResponseBody::Json(json) = body {
                if let Ok(value) = serde_json::from_str(&json) {
                    handler(value);
                }
            }
            Ok(())
        });
        self.call::<serde_json::Value>(
            "registerListener",
            serde_json::json!({ "event": event.name(), "handler": channel }),
        )
        .await
        .map(|_| ())
    }
}

/// `app.veydan_push()`.
pub trait VeydanPushExt<R: Runtime> {
    fn veydan_push(&self) -> &VeydanPush<R>;
}

impl<R: Runtime, T: Manager<R>> VeydanPushExt<R> for T {
    fn veydan_push(&self) -> &VeydanPush<R> {
        self.state::<VeydanPush<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("veydan-push")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "VeydanPushPlugin")?;
            #[cfg(target_os = "ios")]
            let handle = api.register_ios_plugin(init_plugin_veydan_push)?;
            app.manage(VeydanPush(handle));
            Ok(())
        })
        .build()
}

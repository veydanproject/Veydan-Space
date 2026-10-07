// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The phone's side of a call (Android).
//!
//! The Kotlin side owns what the system must see for a call to behave as
//! one: the ringing notification with its full-screen screen, the foreground
//! service that keeps the process alive while a call goes on, the ringtone
//! and the vibration, the route of the sound and the proximity sensor. This
//! side lets the app's Rust code drive it and learn what the user pressed.
//!
//! No media flows here: the call's engine lives in the messenger's crates
//! and only asks the phone for the shell around it. One thing of the engine
//! is this plugin's to do: its Android init needs the JVM and the context
//! on a thread that came from Java, before the first engine of the process
//! (`messenger_rtc::android`), and the Kotlin side has such a thread at its
//! load. It calls [`Java_net_veydan_call_Engine_init`] there.
//!
//! A button pressed while nobody listens (the process was started by the
//! press itself) is kept by the Kotlin side until `take_actions` asks for it.
//!
//! The plugin knows nothing about the messenger; it carries strings.

#![cfg(target_os = "android")]

use jni::objects::{JClass, JObject};
use jni::sys::jboolean;
use jni::JNIEnv;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

const PLUGIN_IDENTIFIER: &str = "net.veydan.call";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("call bridge: {0}")]
    Invoke(#[from] tauri::plugin::mobile::PluginInvokeError),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A call that rings on this phone.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    /// The app's id of the call; every action names it.
    pub call_id: String,
    /// Who calls, as the app shows them.
    pub name: String,
    /// A picture of the caller as a file on this phone. The Kotlin side
    /// fetches nothing: the app hands what it already has.
    pub avatar: Option<String>,
    pub video: bool,
    /// Who calls stays off the lock screen: the messenger's "hide on lock
    /// screen", or a PIN on the app. The lock screen then shows "Incoming
    /// call" with the buttons, and the ringing screen over it shows neither
    /// the name nor the picture until the phone is unlocked.
    pub hide_on_lockscreen: bool,
}

/// A call that goes on.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ongoing {
    pub call_id: String,
    /// Who the call is with, as the app shows them.
    pub name: String,
    /// The camera is held too.
    pub video: bool,
    /// As in [`Incoming`]: the lock screen shows "Call in progress" alone.
    pub hide_on_lockscreen: bool,
}

/// How the phone could show a ringing call.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    /// The foreground service runs: the process stays while it rings. False
    /// when the system refused to start it from the background; the
    /// notification is shown all the same.
    pub foreground: bool,
    /// The system lets the app take the screen with the ringing call
    /// (`USE_FULL_SCREEN_INTENT`, which the user may take away on Android 14+).
    /// Without it a locked phone only shows the notification.
    pub full_screen: bool,
    /// The user allows notifications at all.
    pub notifications: bool,
    /// The phone rings. It rings only while the user has something to press:
    /// the call notification, or the ringing screen opened at once while the
    /// app is in front. False when neither could be shown (notifications or
    /// the channel of calls turned off, the app in the background): the
    /// phone stays silent, and the app shows the call its own way.
    pub ringing: bool,
    /// The app was what the user looked at when the call rang: the
    /// notification pops up over it, and the app may show its own screen.
    #[serde(default)]
    pub in_front: bool,
}

/// Where the sound of a call goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioRoute {
    /// The small speaker at the ear.
    Earpiece,
    /// The loudspeaker.
    Speaker,
    /// A Bluetooth headset (classic or LE).
    Bluetooth,
    /// A headset on a cable or USB.
    Wired,
}

/// The routes the phone has right now, and the one in use.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Routes {
    /// None while no call holds the sound.
    pub current: Option<AudioRoute>,
    pub available: Vec<AudioRoute>,
}

/// What the user pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Answer,
    Decline,
    Hangup,
}

/// A press on the ringing or the ongoing call.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallAction {
    pub call_id: String,
    pub action: Action,
}

#[derive(Debug, Deserialize)]
struct ActionsAnswer {
    actions: Vec<CallAction>,
}

/// The permissions of a call, as [`VeydanCall::request_permission`] names
/// them.
pub const PERMISSION_CAMERA: &str = "camera";
pub const PERMISSION_MICROPHONE: &str = "microphone";

#[derive(Debug, Deserialize)]
struct PermissionAnswer {
    granted: bool,
}

/// What the Kotlin side reports without being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A button was pressed: [`CallAction`].
    CallAction,
    /// The route of the sound, or the routes there are, changed: [`Routes`].
    AudioRouteChanged,
}

impl Event {
    fn name(self) -> &'static str {
        match self {
            Self::CallAction => "call_action",
            Self::AudioRouteChanged => "audio_route_changed",
        }
    }
}

/// Access to the call bridge.
pub struct VeydanCall<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> VeydanCall<R> {
    async fn call<T: DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T> {
        Ok(self.0.run_mobile_plugin_async(command, payload).await?)
    }

    async fn run(&self, command: &str, payload: impl Serialize) -> Result<()> {
        self.call::<serde_json::Value>(command, payload).await.map(|_| ())
    }

    /// Rings: the call notification with Answer and Decline, the screen over
    /// the lock screen, the ringtone and the vibration as the phone's ringer
    /// mode allows. A second call replaces the first.
    pub async fn show_incoming(&self, call: &Incoming) -> Result<Shown> {
        self.call("showIncoming", call).await
    }

    /// Stops ringing: the call was answered elsewhere, cancelled or expired.
    /// `call_id`: only that call; None: whatever rings.
    pub async fn dismiss_incoming(&self, call_id: Option<&str>) -> Result<()> {
        self.run("dismissIncoming", serde_json::json!({ "callId": call_id })).await
    }

    /// The call goes on: the notification with Hang up, the service that
    /// keeps the process and the microphone (and the camera for `video`),
    /// the sound in the mode of a conversation.
    pub async fn start_ongoing(&self, call: &Ongoing) -> Result<()> {
        self.run("startOngoing", call).await
    }

    /// Ends everything: notification, service, ringing, the sound's mode and
    /// route, the wake locks.
    pub async fn stop(&self) -> Result<()> {
        self.run("stop", ()).await
    }

    /// Sends the sound to `route`. Answers what the phone has after it.
    pub async fn set_audio_route(&self, route: AudioRoute) -> Result<Routes> {
        self.call("setAudioRoute", serde_json::json!({ "route": route })).await
    }

    pub async fn list_audio_routes(&self) -> Result<Routes> {
        self.call("listAudioRoutes", ()).await
    }

    /// Keeps the processor awake and the screen on for the call (a video
    /// call is watched). The proximity sensor is the plugin's own business:
    /// it darkens the screen at the ear while the sound goes to the earpiece.
    pub async fn keep_awake(&self, on: bool) -> Result<()> {
        self.run("keepAwake", serde_json::json!({ "on": on })).await
    }
    /// Opens the camera: its frames go to the sink of [`camera`] from a
    /// moment later. `facing` `front` or `back`; the size is what is
    /// asked of the camera, which gives the nearest it has. Fails when
    /// the camera is not allowed (the app asks for the permission).
    pub async fn start_camera(&self, facing: &str, width: u32, height: u32) -> Result<()> {
        self.run("startCamera", serde_json::json!({ "facing": facing, "width": width, "height": height })).await
    }

    pub async fn stop_camera(&self) -> Result<()> {
        self.run("stopCamera", ()).await
    }

    /// The other camera, front or back.
    pub async fn switch_camera(&self) -> Result<()> {
        self.run("switchCamera", ()).await
    }

    /// What the camera did so far: `{ running, facing, frames, dropped,
    /// width, height, rotation, packUs }` from the Kotlin side.
    pub async fn camera_stats(&self) -> Result<serde_json::Value> {
        self.call("cameraStats", ()).await
    }

    /// Asks the phone for `permission` ([`PERMISSION_CAMERA`] or
    /// [`PERMISSION_MICROPHONE`]): the system's question to the user when
    /// it was never answered, nothing when it was. Answers whether the
    /// permission is granted after that. The camera does not open, and
    /// the service does not hold the microphone, without it.
    pub async fn request_permission(&self, permission: &str) -> Result<bool> {
        Ok(self
            .call::<PermissionAnswer>("requestPermission", serde_json::json!({ "permission": permission }))
            .await?
            .granted)
    }

    /// The presses nobody heard yet, oldest first, once. The way to learn of
    /// a press that started the app, which no event could report.
    pub async fn take_actions(&self) -> Result<Vec<CallAction>> {
        Ok(self.call::<ActionsAnswer>("takeActions", ()).await?.actions)
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
        self.run(
            "registerListener",
            serde_json::json!({ "event": event.name(), "handler": channel }),
        )
        .await
    }

    /// [`Event::CallAction`], read.
    pub async fn on_action<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(CallAction) + Send + Sync + 'static,
    {
        self.listen(Event::CallAction, move |value| {
            if let Ok(action) = serde_json::from_value(value) {
                handler(action);
            }
        })
        .await
    }

    /// [`Event::AudioRouteChanged`], read.
    pub async fn on_audio_route<F>(&self, handler: F) -> Result<()>
    where
        F: Fn(Routes) + Send + Sync + 'static,
    {
        self.listen(Event::AudioRouteChanged, move |value| {
            if let Ok(routes) = serde_json::from_value(value) {
                handler(routes);
            }
        })
        .await
    }
}

/// `app.veydan_call()`.
pub trait VeydanCallExt<R: Runtime> {
    fn veydan_call(&self) -> &VeydanCall<R>;
}

impl<R: Runtime, T: Manager<R>> VeydanCallExt<R> for T {
    fn veydan_call(&self) -> &VeydanCall<R> {
        self.state::<VeydanCall<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("veydan-call")
        .setup(|app, api| {
            let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "VeydanCallPlugin")?;
            app.manage(VeydanCall(handle));
            Ok(())
        })
        .build()
}

/// The frames of the phone's camera, on their way to the engine.
///
/// The Kotlin side (`Camera.kt`) packs every image into NV21 and calls
/// [`Java_net_veydan_call_Camera_push`] on the camera's thread; the frame
/// goes to the sink the app set ([`camera::set_sink`]), which hands it to
/// the engine of calls as a pushed frame (the core's `PushedFrame`: the
/// same fields). Without a sink the frames are counted and dropped, which
/// is how the capture alone is measured on a phone ([`camera::stats`]).
pub mod camera {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    /// One frame as the camera gave it: NV21 (Y, then V and U interleaved),
    /// tightly packed, `rotation` the degrees clockwise it is to be turned
    /// to stand upright, `timestamp_us` of the camera's monotonic clock.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Frame {
        pub width: u32,
        pub height: u32,
        pub rotation: u16,
        pub timestamp_us: i64,
        pub data: Vec<u8>,
    }

    impl Frame {
        /// How many bytes NV21 of this size holds.
        pub fn len_for(width: u32, height: u32) -> usize {
            let (cw, ch) = (width.div_ceil(2) as usize, height.div_ceil(2) as usize);
            width as usize * height as usize + 2 * cw * ch
        }

        /// The frame as the engine of calls takes it: NV21 as it is, the
        /// same size, rotation and time; the engine turns and converts it.
        pub fn into_pushed(self) -> messenger_rtc::PushedFrame {
            messenger_rtc::PushedFrame {
                format: messenger_rtc::PixelFormat::Nv21,
                width: self.width,
                height: self.height,
                rotation: self.rotation,
                timestamp_us: self.timestamp_us,
                data: self.data,
            }
        }
    }

    pub type Sink = Box<dyn Fn(Frame) + Send + Sync>;

    static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    static TAKEN: AtomicU64 = AtomicU64::new(0);
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    /// Microseconds the last frame spent between the JNI entry and the
    /// sink's return: the copy out of the JVM and whatever the sink does.
    static LAST_US: AtomicU64 = AtomicU64::new(0);

    fn slot() -> &'static Mutex<Option<Sink>> {
        SINK.get_or_init(|| Mutex::new(None))
    }

    /// Where the frames go from now on; `None` drops them.
    pub fn set_sink(sink: Option<Sink>) {
        *slot().lock().unwrap_or_else(|e| e.into_inner()) = sink;
    }

    /// Frames taken by a sink, frames dropped for want of one, and what
    /// the last one cost on the camera's thread (microseconds).
    pub fn stats() -> (u64, u64, u64) {
        (TAKEN.load(Ordering::Relaxed), DROPPED.load(Ordering::Relaxed), LAST_US.load(Ordering::Relaxed))
    }

    /// A frame from the JNI entry. `true` when a sink took it.
    pub(crate) fn push(frame: Frame, since: std::time::Instant) -> bool {
        let guard = slot().lock().unwrap_or_else(|e| e.into_inner());
        let taken = match guard.as_ref() {
            Some(sink) => {
                sink(frame);
                TAKEN.fetch_add(1, Ordering::Relaxed);
                true
            }
            None => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
                false
            }
        };
        LAST_US.store(since.elapsed().as_micros() as u64, Ordering::Relaxed);
        taken
    }
}

/// `net.veydan.call.Camera.push(nv21, width, height, rotation,
/// timestampUs)`: one frame of the camera, on the camera's thread. The
/// bytes are copied out of the JVM once, into the frame the sink gets. A
/// frame whose size does not match its bytes is dropped.
#[no_mangle]
pub extern "system" fn Java_net_veydan_call_Camera_push<'l>(
    env: JNIEnv<'l>,
    _class: JClass<'l>,
    nv21: jni::objects::JByteArray<'l>,
    width: jni::sys::jint,
    height: jni::sys::jint,
    rotation: jni::sys::jint,
    timestamp_us: jni::sys::jlong,
) -> jboolean {
    let since = std::time::Instant::now();
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else { return 0 };
    let rotation = match rotation.rem_euclid(360) {
        r @ (0 | 90 | 180 | 270) => r as u16,
        _ => return 0,
    };
    let len = camera::Frame::len_for(width, height);
    let Ok(have) = env.get_array_length(&nv21) else { return 0 };
    if width == 0 || height == 0 || (have as usize) < len {
        return 0;
    }
    let mut data = vec![0u8; len];
    // The JVM's bytes are signed; the same bits.
    let view = unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr() as *mut i8, len) };
    if env.get_byte_array_region(&nv21, 0, view).is_err() {
        return 0;
    }
    u8::from(camera::push(camera::Frame { width, height, rotation, timestamp_us, data }, since))
}

/// `net.veydan.call.Engine.init(context)`: hands the JVM and the
/// application context to the media engine (`messenger_rtc::android`).
/// Called by the Kotlin side on the main thread when the plugin loads,
/// which is before any call and on a thread that came from Java, as the
/// engine demands. `true` once the engine may be made; `false` when the
/// init was refused (the engine then refuses every call and says why in
/// the log, instead of aborting the process inside libwebrtc).
///
/// A product without the engine has no such symbol: the Kotlin side takes
/// the `UnsatisfiedLinkError` for "no engine".
#[no_mangle]
pub extern "system" fn Java_net_veydan_call_Engine_init<'l>(env: JNIEnv<'l>, _class: JClass<'l>, context: JObject<'l>) -> jboolean {
    let Ok(vm) = env.get_java_vm() else { return 0 };
    u8::from(messenger_rtc::android::init_android(&vm, &context))
}

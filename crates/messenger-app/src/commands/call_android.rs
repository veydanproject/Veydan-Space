// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Calls on a phone: what stands between the messenger and the call plugin.
//!
//! The call itself is the runtime's (`messenger_runtime::calls`); the
//! phone's shell around it — the ringing notification and its screen over
//! the lock screen, the foreground service that keeps the process and the
//! microphone, the sound in the mode of a conversation and its route — is
//! the plugin's (`tauri-plugin-veydan-call`). The bridge here reads the
//! runtime's events and drives the plugin, and answers the plugin's presses
//! with the runtime's commands:
//!
//! ```text
//! call.incoming ──▶ show_incoming       Answer  ──▶ call_accept
//! call.state    ──▶ start_ongoing       Decline ──▶ call_decline
//!   (outgoing, connecting, active)      Hang up ──▶ call_end
//! call.ended    ──▶ stop
//! ```
//!
//! A press can come before the runtime knows the call: Answer on a call a
//! push rang while the app was dead starts the app, whose runtime then
//! catches up with the relays and hears the invitation a moment later. Such
//! a press is kept as the *expected* answer for a short while
//! ([`EXPECTED_FOR`]), and the call is taken the moment it rings in.
//!
//! The room of a group call takes the same shell, without a ringing: a
//! call announced in a group is a banner in its chat, not a call that
//! rings here (messenger-wire §10). The room I start or join goes on from
//! its first word (`group_call.state`: the service, the sound's mode, the
//! processor awake), and ends when I leave it or it goes:
//!
//! ```text
//! group_call.state (starting … reconnecting) ──▶ start_ongoing   Hang up ──▶ group_call_leave
//! group_call.state (left), group_call.ended  ──▶ stop
//! ```
//!
//! A call of two that rings in while I am in a room is not rung here
//! (`IncomingStep::InRoom`): the core leaves it ringing for my other
//! devices, and the phone's shell is the room's until I am out of it. Out
//! of the room while that call still rings in the core (an invitation
//! lives 45 s), the phone rings it then (`rings_after_room`): the user
//! may have left the room for it.
//!
//! The camera of a room is the phone's as in a call of two: it follows
//! `video_local` and `camera` of the room's word, and its frames go into
//! the room through the runtime (`group_call_push_video_frame`), never
//! through the page; the other camera comes by
//! `messenger_group_call_switch_camera`, whose answer the bridge follows.
//!
//! A call announced in a group while the app is away is a quiet
//! notification (`group_call.started` ──▶ `show_group_call`): the group's
//! name over "A call is on", no sound, no buttons; a tap opens the group's
//! chat, whose banner has Join. Once per call, not for a call I started or
//! am in; gone when the call ends or I join it. The bridge keeps the
//! calls it showed for the life of the process (`NOTICED`); the plugin
//! keeps them, and the calls that went on here, across processes
//! (`GroupCallMemory` of the Kotlin side), so that a process started
//! anew shows no notice of a call I left before it died, and takes down
//! the notice a dead process left.
//!
//! As with pushes, the web page never talks to the plugin: the route of the
//! sound is a command here (`messenger_call_audio_route`), and a change of
//! it is an event (`call.audio_route`). The developer's command rings this
//! phone with a call from nobody, so that the shell can be checked without
//! a peer.

use serde::Deserialize;
use veydan_core::{AppError, CmdResult};

#[cfg(target_os = "android")]
use super::{MessengerState, EVENT_RUNTIME};
use messenger_notify::Content;
#[cfg(target_os = "android")]
use messenger_runtime::calls::VideoInput;
use messenger_runtime::{CallMedia, CallPhase};
#[cfg(target_os = "android")]
use messenger_runtime::{CallView, GroupCallAnnounced, GroupCallView, MessengerRuntime};
use messenger_runtime::GroupCallPhase;
#[cfg(target_os = "android")]
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "android")]
use tauri_plugin_veydan_call::{Action, AudioRoute, CallAction, GroupCallNotice, Incoming, NetworkChange, Ongoing, Routes, VeydanCall};

/// The event of the developer's command and of what the plugin reports.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const EVENT_DEBUG: &str = "call.debug";
/// The event of a change of the route of the sound: the payload is
/// `{ current, available }` as the plugin reports it.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const EVENT_AUDIO_ROUTE: &str = "call.audio_route";
/// The event of a camera that would not open for my video: the payload is
/// `{ callId, error, denied }`, `denied` when the phone's permission for
/// the camera is what is missing. My video is off in the core by then
/// (the peer is told so); the page may ask for the permission and turn
/// it on again.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const EVENT_CAMERA_FAILED: &str = "call.camera_failed";

/// How long an Answer pressed before the runtime knew the call waits for
/// the call to ring in: the time of a cold start and a catch-up with the
/// relays, within the life of an invitation (45 s from its making).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const EXPECTED_FOR: Duration = Duration::from_secs(20);

/// What the debug command is asked to do.
#[cfg_attr(not(mobile), allow(dead_code))]
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugRing {
    /// `ring` (the default): a call rings; `ongoing`: the call goes on at
    /// once; `dismiss`: the ringing stops; `routes`: the routes of the sound;
    /// `route`: the sound goes to `route`; `awake`: the processor and the
    /// screen are kept awake (`on`) or let go; `stop`: everything ends.
    pub op: Option<String>,
    /// Who calls; "Veydan test" when not given.
    pub name: Option<String>,
    pub video: Option<bool>,
    /// `earpiece`, `speaker`, `bluetooth`, `wired`.
    pub route: Option<String>,
    pub on: Option<bool>,
    /// Rings after this many seconds (up to a minute), so that the phone can
    /// be locked or the app put away first. The command answers at once.
    pub delay_secs: Option<u64>,
    /// Keeps who calls off the lock screen, as the messenger's privacy
    /// settings will ask; shown when not given.
    pub hide_on_lockscreen: Option<bool>,
}

/// What the route command is asked to do.
#[cfg_attr(not(mobile), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRouteInput {
    /// `list`: the routes there are and the one in use; `set`: the sound
    /// goes to `route`, and the routes after that are answered.
    pub op: String,
    /// `earpiece`, `speaker`, `bluetooth`, `wired`.
    pub route: Option<String>,
}

/// The call the debug command rang, for the answer to it.
#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
struct Rung {
    call_id: String,
    name: String,
    video: bool,
    hidden: bool,
}

#[cfg(target_os = "android")]
impl Rung {
    fn ongoing(&self) -> Ongoing {
        Ongoing {
            call_id: self.call_id.clone(),
            name: self.name.clone(),
            video: self.video,
            hide_on_lockscreen: self.hidden,
        }
    }
}

#[cfg(target_os = "android")]
static RUNG: std::sync::Mutex<Option<Rung>> = std::sync::Mutex::new(None);

/// The plugin keeps a listener for as long as the app runs.
#[cfg(target_os = "android")]
static LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The call the plugin shows as going on, and whether with the camera, so
/// that the service is started once for each.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static ONGOING: std::sync::Mutex<Option<(String, bool)>> = std::sync::Mutex::new(None);

/// An Answer pressed on a call the runtime did not know yet, and when.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static EXPECTED: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);

/// What a `group_call.state` does to the phone's shell: the room is going
/// on (the shell started once per room and per `video`, the wording of
/// the notification), or over for me.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomStep {
    /// The room goes on: the shell for it, unless it is shown already.
    Ongoing,
    /// I am out of the room: the shell of the room, if it is shown, stops.
    Over,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn room_step(phase: GroupCallPhase) -> RoomStep {
    match phase {
        GroupCallPhase::Starting | GroupCallPhase::Joining | GroupCallPhase::InRoom | GroupCallPhase::Reconnecting => RoomStep::Ongoing,
        GroupCallPhase::Left => RoomStep::Over,
    }
}

/// The calls announced in groups whose quiet notification was shown, by
/// call id, the newest last: one notification per call, however many
/// times the group announces it (every change of who is in). Forgotten
/// when the call ends; a few at a time, the oldest first, so that the
/// calls of a long life never pile up. For the life of the process: the
/// plugin remembers across processes (see the module's doc), and answers
/// `show_group_call` with `false` for a call it knows.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static NOTICED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// How many announced calls [`NOTICED`] remembers.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const NOTICED_MAX: usize = 16;

/// Whether a `group_call.started` is news for the phone's shade: a call
/// I am not in and did not start (`me`: my key, hex, as `started_by`
/// has it), not shown yet. A call I started or joined is on my screen;
/// one shown already stays as it is while the group tells of every seat
/// taken.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn group_call_is_news(joined: bool, started_by: &str, me: &str, shown: bool) -> bool {
    !joined && !shown && started_by != me
}

/// Remembers `call_id` as shown (see [`NOTICED`]).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn noticed(call_id: &str) {
    let mut list = lock(&NOTICED);
    if list.iter().any(|c| c == call_id) {
        return;
    }
    list.push(call_id.to_string());
    if list.len() > NOTICED_MAX {
        let extra = list.len() - NOTICED_MAX;
        list.drain(..extra);
    }
}

/// Whether `call_id` was shown (see [`NOTICED`]).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn was_noticed(call_id: &str) -> bool {
    lock(&NOTICED).iter().any(|c| c == call_id)
}

/// The call `call_id` ended: not shown any more, and forgotten.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn forget_noticed(call_id: &str) -> bool {
    let mut list = lock(&NOTICED);
    let before = list.len();
    list.retain(|c| c != call_id);
    list.len() != before
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What the end of a call does to the phone's shell, given the call the
/// plugin shows as going on. The core ends calls that never took the
/// phone: a second invitation during a call is answered Busy, an
/// invitation older than its life caught up from the relays is Missed,
/// each with the same `call.ended`. The shell of the call under way, or
/// the ringing of the call that rings, is not theirs to take down (`stop`
/// on the Kotlin side has no id: it ends the sound's mode, the service,
/// the notification and the wake lock of whatever goes on), so only the
/// call shown as going on stops everything; any other end is a dismissal
/// by id, which the plugin answers only for the call that rings.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellEnd {
    /// The call that goes on ended: everything of the phone ends.
    Stop,
    /// Another call, or none, goes on: a ringing of the ended one, if it
    /// is the one that rings, is dismissed; nothing else changes.
    DismissOnly,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn shell_end(ongoing: Option<&str>, ended: &str) -> ShellEnd {
    match ongoing {
        Some(id) if id == ended => ShellEnd::Stop,
        _ => ShellEnd::DismissOnly,
    }
}

/// What a `call.incoming` of the runtime does on the phone, given an
/// answer expected for the call, the phase the runtime has the call in
/// right now (`None`: no such call any more) and whether I am in the room
/// of a group call. The event may be read after the call moved on: the
/// bridge subscribes before it takes the presses made before it listened,
/// and an Answer taken then accepts a call the runtime already had; its
/// buffered `call.incoming` must not ring the answered call again.
///
/// A call of two that rings in during a room is the core's to leave
/// ringing: it answers Busy to a second call of two alone, and refuses
/// my answer while the room is on (`call_accept`: one call at a time,
/// and the ringing one keeps ringing for my other devices). The phone
/// must not ring for it either: the plugin knows one call at a time, and
/// a ringing over the room would end — by a press or by its limit — in
/// the plugin's `stop`, which takes the room's service, notification,
/// sound mode and wake lock down with it while the core keeps me in the
/// room, with nothing to bring them back.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IncomingStep {
    /// Answered on the notification before the runtime heard it: taken.
    Accept,
    /// Rings on the phone.
    Ring,
    /// Not ringing any more: answered, declined or over already.
    Skip,
    /// A room goes on here: not rung, and the plugin remembers the call
    /// as over (a late push for it rings nothing); it rings on for my
    /// other devices and ends as missed here.
    InRoom,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn incoming_step(expected: bool, phase: Option<CallPhase>, in_room: bool) -> IncomingStep {
    match (expected, phase, in_room) {
        (_, Some(phase), _) if phase != CallPhase::Incoming => IncomingStep::Skip,
        (_, None, _) => IncomingStep::Skip,
        (_, _, true) => IncomingStep::InRoom,
        (true, _, false) => IncomingStep::Accept,
        (false, _, false) => IncomingStep::Ring,
    }
}

/// Whether the phone rings, once a room is over for me, the call of two
/// that rang in during the room and was left to my other devices
/// (`IncomingStep::InRoom`): yes while the core still has it ringing
/// (`phase`, `None` for no such call) and no room has taken its place
/// (`in_room`: the core's word after the stop, not the shell's). The core
/// takes `call_accept` again the moment the room is gone, and the user may
/// have left the room for this very call; without a ringing it would go
/// missed in silence (the push for it was dismissed as over with the
/// room, and rings nothing).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn rings_after_room(in_room: bool, phase: Option<CallPhase>) -> bool {
    !in_room && phase == Some(CallPhase::Incoming)
}

/// Who the call notification names, as a message of theirs would: the
/// setting "no content" and a PIN on the app leave no name (the plugin
/// shows the app's), as the notifications of a computer do
/// (`desktop_notify::caller`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn shown_name(name: String, content: Content, locked: bool) -> String {
    if content == Content::None || locked {
        String::new()
    } else {
        name
    }
}

/// Whether the camera failed for want of the phone's permission, from
/// the plugin's word for it (`Camera.start`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn camera_denied(error: &str) -> bool {
    error.contains("not allowed")
}

/// The call shown as going on, and whether its service holds the camera.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type Shell = (String, bool);
/// The call whose camera is open, and which camera (`front` or `back`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
type OpenCamera = (String, String);

/// Everything the bridge remembers of the phone, taken: the call shown as
/// going on and the camera open (answered), the answer expected (dropped).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn forget() -> (Option<Shell>, Option<OpenCamera>) {
    *lock(&EXPECTED) = None;
    (lock(&ONGOING).take(), lock(&CAMERA).take())
}

#[cfg(target_os = "android")]
fn bridge(app: &tauri::AppHandle) -> CmdResult<tauri::State<'_, VeydanCall<tauri::Wry>>> {
    use tauri::Manager;
    app.try_state::<VeydanCall<tauri::Wry>>()
        .ok_or_else(|| AppError::Other("call bridge is not loaded".into()))
}

/// The runtime, while the messenger runs.
#[cfg(target_os = "android")]
fn runtime(app: &tauri::AppHandle) -> Option<Arc<MessengerRuntime>> {
    use tauri::Manager;
    app.try_state::<MessengerState>().and_then(|s| s.runtime().ok())
}

#[cfg(target_os = "android")]
fn failed(e: tauri_plugin_veydan_call::Error) -> AppError {
    AppError::Other(e.to_string())
}

/// Tells the page and the log.
#[cfg(target_os = "android")]
fn report(app: &tauri::AppHandle, name: &str, payload: serde_json::Value) {
    use tauri::Emitter;
    eprintln!("messenger call: {name} {payload}");
    let _ = app.emit(EVENT_RUNTIME, serde_json::json!({ "name": name, "payload": payload }));
}

/// The phone's permissions a call needs, asked before the call starts or
/// is taken: the microphone, and the camera too for a video call. The
/// system asks the user when it never did (nothing when it did); a no is
/// told to the page (`error`, scope `calls`) and is an error here, so that
/// no call goes without a microphone: the engine records silence without
/// the permission, and the service may not hold the microphone. A
/// computer has no such question.
#[cfg(target_os = "android")]
pub async fn permissions_for_call(app: &tauri::AppHandle, media: CallMedia) -> CmdResult<()> {
    use tauri_plugin_veydan_call::{PERMISSION_CAMERA, PERMISSION_MICROPHONE};
    let call = bridge(app)?;
    let wanted: &[&str] = match media {
        CallMedia::Audio => &[PERMISSION_MICROPHONE],
        CallMedia::Video => &[PERMISSION_MICROPHONE, PERMISSION_CAMERA],
    };
    for permission in wanted {
        if !call.request_permission(permission).await.map_err(failed)? {
            let error = permission_refused(permission);
            report(app, "error", serde_json::json!({ "scope": "calls", "error": error }));
            return Err(AppError::Other(error));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
pub async fn permissions_for_call(_app: &tauri::AppHandle, _media: CallMedia) -> CmdResult<()> {
    Ok(())
}

/// What the page shows for a permission the user refused (the words of
/// the camera's are the ones `camera_denied` knows).
fn permission_refused(permission: &str) -> String {
    match permission {
        "camera" => "the camera is not allowed".to_string(),
        _ => "the microphone is not allowed".to_string(),
    }
}

/// The expected answer, if it is for `call_id` and not too old; taken.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn take_expected(call_id: &str) -> bool {
    let mut slot = lock(&EXPECTED);
    match slot.as_ref() {
        Some((id, at)) if id == call_id && at.elapsed() <= EXPECTED_FOR => {
            *slot = None;
            true
        }
        _ => false,
    }
}

/// A press: told, and given to the runtime. Decline and Hang up the plugin
/// has ended on the phone by itself; the runtime tells the peer.
#[cfg(target_os = "android")]
async fn pressed(app: &tauri::AppHandle, press: CallAction) {
    report(app, EVENT_DEBUG, serde_json::json!({ "action": press.action, "callId": press.call_id }));
    // The developer's call from nobody: answered as the engine would.
    let rung = lock(&RUNG).clone().filter(|r| r.call_id == press.call_id);
    if let Some(rung) = rung {
        let Ok(call) = bridge(app) else { return };
        match press.action {
            Action::Answer => {
                if let Err(e) = call.start_ongoing(&rung.ongoing()).await {
                    report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string() }));
                }
            }
            Action::Decline | Action::Hangup => *lock(&RUNG) = None,
        }
        return;
    }
    let rt = runtime(app);
    let current = match &rt {
        Some(rt) => rt.call_state().await.ok().and_then(|s| s.call).filter(|c| c.call_id == press.call_id),
        None => None,
    };
    // Hang up on the notification of a room: out of it (the room goes on
    // for the others, or ends with the last one out).
    if let (Action::Hangup, Some(rt), None) = (press.action, &rt, &current) {
        if rt.group_call_state(None).await.call.is_some_and(|c| c.call_id == press.call_id) {
            if let Err(e) = rt.group_call_leave().await {
                report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": press.call_id }));
            }
            return;
        }
    }
    let outcome = match (press.action, rt, current) {
        (Action::Answer, Some(rt), Some(call)) if call.phase == CallPhase::Incoming => {
            // The microphone (and the camera) first; a no was told to
            // the page, and the call keeps ringing for another press.
            if permissions_for_call(app, call.media).await.is_err() {
                return;
            }
            rt.call_accept(&press.call_id).await.map(|_| ())
        }
        // Not ringing here yet: the app was started by this very press, or
        // the invitation is still on its way from the relays.
        (Action::Answer, _, _) => {
            *lock(&EXPECTED) = Some((press.call_id.clone(), Instant::now()));
            eprintln!("messenger call: answer on {} expected within {} s", press.call_id, EXPECTED_FOR.as_secs());
            Ok(())
        }
        (Action::Decline, Some(rt), Some(_)) => rt.call_decline(&press.call_id).await,
        (Action::Hangup, Some(rt), Some(_)) => rt.call_end(&press.call_id).await,
        // A call the runtime does not have: nothing to end; the plugin
        // cleared the phone already. A press on a call nobody knows is
        // also how the expected answer is withdrawn.
        (Action::Decline | Action::Hangup, _, _) => {
            *lock(&EXPECTED) = None;
            Ok(())
        }
    };
    if let Err(e) = outcome {
        report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": press.call_id }));
    }
}

/// How long the restart of a call waits for a relay to be connected again
/// after the connections were made anew: `nostr-sdk` tries a dropped relay
/// again after 3 s, and the handshake on the new network follows. Past
/// this, the restart goes without: its signal waits in the outbox.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const RELAYS_BACK_WITHIN: Duration = Duration::from_secs(10);
/// How long the drop of the connections is given to show in the state of
/// the relays (the stream of a closed socket ends at once, and `nostr-sdk`
/// marks the relay within milliseconds). A state that stays `Connected`
/// this long had nothing to close.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const RELAYS_DROP_SEEN_WITHIN: Duration = Duration::from_secs(1);
/// How often the state of the relays is looked at meanwhile.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const RELAYS_BACK_EVERY: Duration = Duration::from_millis(100);

/// What the wait for the relays came to.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelaysBack {
    /// A relay dropped and is connected again: the sockets are live.
    Reconnected,
    /// No relay dropped within [`RELAYS_DROP_SEEN_WITHIN`]: there was nothing
    /// to make anew, and the relays stand as they are.
    Unchanged,
    /// No relay came back within [`RELAYS_BACK_WITHIN`].
    TimedOut,
}

/// Waits, after the relay connections were closed, until one is connected
/// again. `connected` says whether any relay is connected now. Right after
/// the close the relays still read `Connected` for a moment (the close is
/// seen by `nostr-sdk` when the stream ends), so a relay counts as back
/// only after it was seen down: a signal queued on the stale reading would
/// hit a dead socket and wait out the outbox's backoff.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
async fn relays_back<F, Fut>(mut connected: F, within: Duration, drop_seen_within: Duration, every: Duration) -> RelaysBack
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = Instant::now();
    let mut seen_down = false;
    loop {
        let up = connected().await;
        let since = start.elapsed();
        if up && seen_down {
            return RelaysBack::Reconnected;
        }
        if !up {
            seen_down = true;
        } else if since >= drop_seen_within {
            return RelaysBack::Unchanged;
        }
        if since >= within {
            return RelaysBack::TimedOut;
        }
        tokio::time::sleep(every).await;
    }
}

/// The phone's network changed (`how`: `back` after none, `other` for
/// another one): the relay connections are made anew, the restart of the
/// call waits for the first of them to be back, and only then the core is
/// told.
///
/// The order matters. A closed socket fails a send at once, and the
/// outbox then holds the signal back for its first backoff (5 s): the
/// signals of a restart queued before the relays are back would leave
/// later than the relays took to come back. Queued once a relay is
/// connected again, they leave with the outbox's kick, at once. Without a
/// call, or with the relays silenced, the core is told at once: it does
/// nothing without a call, and nothing leaves in silence.
#[cfg(target_os = "android")]
async fn network_changed(app: &tauri::AppHandle, how: &str) {
    use messenger_core::traits::RelayState;
    let Some(rt) = runtime(app) else { return };
    let pool = rt.relays().pool().await;
    pool.reopen();
    // A room counts too: its signals (the words of identity, a key for a
    // new epoch) go through the relays the same way, so the wait for a
    // relay is the same. The core restarts the way of a call of two
    // alone (`call_network_changed` is the DM service's); the room has
    // no restart yet, and its way to the node either recovers by the
    // engine's own gathering or fails, which ends the room for me.
    let in_call = rt.call_state().await.ok().and_then(|s| s.call).is_some() || rt.group_call_state(None).await.call.is_some();
    if !in_call || pool.is_silent() {
        eprintln!("messenger call: the network changed ({how}); the relay connections are made anew");
        rt.call_network_changed().await;
        return;
    }
    let started = Instant::now();
    let status = {
        let pool = pool.clone();
        move || {
            let pool = pool.clone();
            async move { messenger_core::Transport::status(&*pool).await.relays.iter().any(|r| r.state == RelayState::Connected) }
        }
    };
    let back = relays_back(status, RELAYS_BACK_WITHIN, RELAYS_DROP_SEEN_WITHIN, RELAYS_BACK_EVERY).await;
    eprintln!(
        "messenger call: the network changed ({how}); the relay connections made anew: {back:?} after {} ms; the call restarts",
        started.elapsed().as_millis()
    );
    rt.call_network_changed().await;
}

/// Registers the listeners once in the process and takes the presses made
/// before: the press that started the app came before any listener.
#[cfg(target_os = "android")]
async fn listen(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    let Ok(call) = bridge(app) else { return };
    if LISTENING.swap(true, Ordering::SeqCst) {
        return;
    }
    let presses = app.clone();
    if let Err(e) = call
        .on_action(move |press| {
            let app = presses.clone();
            tauri::async_runtime::spawn(async move { pressed(&app, press).await });
        })
        .await
    {
        eprintln!("messenger call: cannot listen for presses: {e}");
    }
    let routes = app.clone();
    if let Err(e) = call
        .on_audio_route(move |r: Routes| {
            report(&routes, EVENT_AUDIO_ROUTE, serde_json::to_value(r).unwrap_or_default());
        })
        .await
    {
        eprintln!("messenger call: cannot listen for the routes: {e}");
    }
    // The phone's word that its network changed: the relay connections,
    // which sat on the old network and may look open for a long while,
    // are made anew, and the call's way is restarted as soon as one of
    // them is back (the engine would notice the loss only after seconds
    // of silence, and a signal queued before the relays are back waits
    // out the outbox's backoff).
    let network = app.clone();
    if let Err(e) = call
        .on_network_changed(move |change: NetworkChange| {
            let app = network.clone();
            tauri::async_runtime::spawn(async move { network_changed(&app, &change.how).await });
        })
        .await
    {
        eprintln!("messenger call: cannot listen for the network: {e}");
    }
    match call.take_actions().await {
        Ok(waiting) => {
            for press in waiting {
                pressed(app, press).await;
            }
        }
        Err(e) => eprintln!("messenger call: the presses before the start are lost: {e}"),
    }
}

/// Whether the app has a PIN (`veydan_lock`): the notifications then
/// name nobody, as on a computer. Without the lock's state, locked.
#[cfg(target_os = "android")]
async fn locked(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    match app.try_state::<veydan_lock::Lock>() {
        Some(lock) => lock.enabled().await,
        None => true,
    }
}

/// Who the call is with, as the chat shows them and as the notifications'
/// settings allow (see [`shown_name`]), and whether who calls stays off
/// the lock screen. Settings that cannot be read hide everything: a
/// notification says less rather than more when in doubt.
#[cfg(target_os = "android")]
async fn face_of(app: &tauri::AppHandle, rt: &MessengerRuntime, call: &CallView) -> (String, bool) {
    let name = match rt.dm().chat(&call.chat_id).await {
        Ok(Some(chat)) => chat.title,
        _ => match messenger_core::PubKey::parse(&call.peer) {
            Some(pk) => rt.contacts().face_of(&pk).await.map(|(n, _)| n).unwrap_or_default(),
            None => String::new(),
        },
    };
    named(app, rt, name).await
}

/// The group of a room, as its chat shows it, under the same settings.
#[cfg(target_os = "android")]
async fn face_of_room(app: &tauri::AppHandle, rt: &MessengerRuntime, group_id: &str, chat_id: &str) -> (String, bool) {
    let name = match rt.group_get(group_id).await {
        Ok(group) if !group.name.is_empty() => group.name,
        _ => match rt.dm().chat(chat_id).await {
            Ok(Some(chat)) => chat.title,
            _ => String::new(),
        },
    };
    named(app, rt, name).await
}

/// `name` as the notifications' settings allow it (see [`shown_name`]),
/// and whether it stays off the lock screen.
#[cfg(target_os = "android")]
async fn named(app: &tauri::AppHandle, rt: &MessengerRuntime, name: String) -> (String, bool) {
    let (content, hidden) = match messenger_notify::Settings::load(rt.store()).await {
        Ok(s) => (s.content, s.lockscreen_hidden),
        Err(e) => {
            eprintln!("messenger call: the settings of notifications cannot be read, nobody is named: {e}");
            (Content::None, true)
        }
    };
    (shown_name(name, content, locked(app).await), hidden)
}

/// One event of the runtime, to the plugin.
#[cfg(target_os = "android")]
async fn on_event(app: &tauri::AppHandle, rt: &Arc<MessengerRuntime>, name: &str, payload: &serde_json::Value) {
    use messenger_runtime::{UI_EVENT_CALL_ENDED, UI_EVENT_CALL_INCOMING, UI_EVENT_CALL_STATE};
    if name.starts_with("group_call.") {
        return on_room_event(app, rt, name, payload).await;
    }
    let Ok(call) = bridge(app) else { return };
    let Ok(view) = serde_json::from_value::<CallView>(payload["call"].clone()) else { return };
    match name {
        n if n == UI_EVENT_CALL_INCOMING => {
            // The runtime's word now, not the event's: the event may be
            // read after the call was taken (see `incoming_step`). A
            // state that cannot be read is taken as the event says.
            let phase = match rt.call_state().await {
                Ok(state) => state.call.filter(|c| c.call_id == view.call_id).map(|c| c.phase),
                Err(_) => Some(view.phase),
            };
            let in_room = rt.group_call_state(None).await.call.map(|c| c.call_id);
            match incoming_step(take_expected(&view.call_id), phase, in_room.is_some()) {
                IncomingStep::InRoom => {
                    // The room keeps the phone (see `IncomingStep`); the
                    // plugin is told the call is over for the phone, as
                    // for a call the core answered Busy, so that a push
                    // for it rings nothing when the room ends first.
                    eprintln!(
                        "messenger call: {} rang in while the room {} goes on; not rung here",
                        view.call_id,
                        in_room.unwrap_or_default()
                    );
                    if let Err(e) = call.dismiss_incoming(Some(&view.call_id)).await {
                        report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id }));
                    }
                    return;
                }
                IncomingStep::Accept => {
                    // Answered on the notification before the runtime heard
                    // the invitation: taken now, and the phone is not rung.
                    eprintln!("messenger call: {} rang in and is taken as answered", view.call_id);
                    if permissions_for_call(app, view.media).await.is_err() {
                        return;
                    }
                    if let Err(e) = rt.call_accept(&view.call_id).await {
                        report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id }));
                    }
                    return;
                }
                IncomingStep::Skip => {
                    eprintln!("messenger call: {} rang in and is {phase:?} already; not rung", view.call_id);
                    return;
                }
                IncomingStep::Ring => {}
            }
            ring(app, rt, &view).await;
        }
        n if n == UI_EVENT_CALL_STATE => {
            if !matches!(view.phase, CallPhase::Outgoing | CallPhase::Connecting | CallPhase::Active | CallPhase::Reconnecting) {
                return;
            }
            // The service holds the camera only when told so at its start
            // (its type): a camera turned on inside a voice call starts it
            // again, with the camera. The ringing service, if any, is
            // replaced; nothing else of the phone changes on a repeat.
            // Whether my video is on, and through which camera, is the
            // core's word in the view: `front` or `back` on a phone, and
            // the front one until a choice is made.
            let camera = view.video_local && !view.video_screen;
            let facing = view.camera.clone().unwrap_or_else(|| CAMERA_FRONT.to_string());
            let video = view.media == CallMedia::Video || camera;
            let started = lock(&ONGOING).clone();
            if started.as_ref() != Some(&(view.call_id.clone(), video)) {
                *lock(&ONGOING) = Some((view.call_id.clone(), video));
                let (name, hidden) = face_of(app, rt, &view).await;
                let ongoing = Ongoing { call_id: view.call_id.clone(), name, video, hide_on_lockscreen: hidden };
                // The service with the microphone, the sound in the mode of a
                // conversation; the processor awake for the call, whatever the
                // screen does (the plugin lets it go at the stop).
                if let Err(e) = call.start_ongoing(&ongoing).await {
                    report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id }));
                }
                let _ = call.keep_awake(true).await;
            }
            sync_camera(app, rt, &view.call_id, camera.then_some(facing.as_str()), Feed::Dm).await;
        }
        n if n == UI_EVENT_CALL_ENDED => {
            let end = {
                let mut ongoing = lock(&ONGOING);
                let end = shell_end(ongoing.as_ref().map(|(id, _)| id.as_str()), &view.call_id);
                if end == ShellEnd::Stop {
                    *ongoing = None;
                }
                end
            };
            close_camera(app, &view.call_id).await;
            let outcome = match end {
                ShellEnd::Stop => call.stop().await,
                // Not the call under way: one that rang here (stopped by
                // its id), or one the core ended on its own (Busy while
                // another goes on, Missed when caught up late), which the
                // phone never showed and the plugin leaves alone.
                ShellEnd::DismissOnly => call.dismiss_incoming(Some(&view.call_id)).await,
            };
            if let Err(e) = outcome {
                report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id }));
            }
        }
        _ => {}
    }
}

/// The phone rings `view`, whether the app is in front or away: the
/// ringtone is the plugin's on a phone (the page of the call sounds
/// nothing for a call that comes in), and so is the notification. With
/// the app in front the plugin opens no screen of its own: the page of
/// the messenger shows the call. A call that rings already goes on
/// ringing as it is.
#[cfg(target_os = "android")]
async fn ring(app: &tauri::AppHandle, rt: &Arc<MessengerRuntime>, view: &CallView) {
    let Ok(call) = bridge(app) else { return };
    let (name, hidden) = face_of(app, rt, view).await;
    let incoming = Incoming {
        call_id: view.call_id.clone(),
        name,
        avatar: None,
        video: view.media == CallMedia::Video,
        hide_on_lockscreen: hidden,
    };
    match call.show_incoming(&incoming).await {
        Ok(shown) => report(app, EVENT_DEBUG, serde_json::json!({ "shown": shown, "callId": view.call_id })),
        Err(e) => report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id })),
    }
}

/// One `group_call.*` event of the runtime, to the plugin: the room I am
/// in takes the shell of a call that goes on (the service, the sound's
/// mode, the processor awake, the camera while my video is on), and
/// gives it back when I am out of it or it went. A call announced in a
/// group (`group_call.started`) that is not mine is a quiet notification
/// while the app is away (the plugin shows nothing over the app: the
/// chat's banner says it there), once per call.
#[cfg(target_os = "android")]
async fn on_room_event(app: &tauri::AppHandle, rt: &Arc<MessengerRuntime>, name: &str, payload: &serde_json::Value) {
    use messenger_runtime::{UI_EVENT_GROUP_CALL_ENDED, UI_EVENT_GROUP_CALL_STARTED, UI_EVENT_GROUP_CALL_STATE};
    let Ok(call) = bridge(app) else { return };
    match name {
        n if n == UI_EVENT_GROUP_CALL_STATE => {
            let Ok(view) = serde_json::from_value::<GroupCallView>(payload["call"].clone()) else { return };
            if room_step(view.phase) == RoomStep::Over {
                return stop_room(app, rt, &view.call_id).await;
            }
            // The service holds the camera only when told so at its start
            // (its type), as for a call of two: a camera turned on inside
            // a voice room starts it again, with the camera. The camera
            // follows the room's word: on through `front` or `back`
            // (`messenger_group_call_switch_camera` changes it), off.
            let camera = view.video_local;
            let facing = view.camera.clone().unwrap_or_else(|| CAMERA_FRONT.to_string());
            let video = view.media == CallMedia::Video || camera;
            let started = lock(&ONGOING).clone();
            if started.as_ref() != Some(&(view.call_id.clone(), video)) {
                *lock(&ONGOING) = Some((view.call_id.clone(), video));
                let (name, hidden) = face_of_room(app, rt, &view.group_id, &view.chat_id).await;
                let ongoing = Ongoing { call_id: view.call_id.clone(), name, video, hide_on_lockscreen: hidden };
                if let Err(e) = call.start_ongoing(&ongoing).await {
                    report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": view.call_id }));
                }
                let _ = call.keep_awake(true).await;
                // I am in: the quiet notification of this call, if any,
                // goes, and the call is no news afterwards either (the
                // group announces it again at every seat taken, and after
                // I leave it while it goes on).
                if was_noticed(&view.call_id) {
                    let _ = call.dismiss_group_call(&view.call_id).await;
                }
                noticed(&view.call_id);
            }
            sync_camera(app, rt, &view.call_id, camera.then_some(facing.as_str()), Feed::Room).await;
        }
        n if n == UI_EVENT_GROUP_CALL_STARTED => {
            let Ok(announced) = serde_json::from_value::<GroupCallAnnounced>(payload["call"].clone()) else { return };
            let me = rt.session_pubkey().await.map(|k| k.to_string()).unwrap_or_default();
            if !group_call_is_news(announced.joined, &announced.started_by, &me, was_noticed(&announced.call_id)) {
                return;
            }
            let (name, hidden) = face_of_room(app, rt, &announced.group_id, &announced.chat_id).await;
            let notice = GroupCallNotice {
                call_id: announced.call_id.clone(),
                chat_id: announced.chat_id.clone(),
                name,
                video: announced.media == CallMedia::Video,
                hide_on_lockscreen: hidden,
            };
            match call.show_group_call(&notice).await {
                // Shown once; not shown (the app in front, notifications
                // off): the next word of the call, with the app away, may.
                Ok(true) => noticed(&announced.call_id),
                Ok(false) => {}
                Err(e) => report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": announced.call_id })),
            }
        }
        n if n == UI_EVENT_GROUP_CALL_ENDED => {
            // The room went for everybody while I was in it (the core
            // says `left` too; the second stop finds nothing), or a call
            // announced in a group is over: its notification goes. Asked
            // of the plugin whether this process showed one or not: the
            // notice may be a dead process's, caught up with now.
            let Ok(ended) = serde_json::from_value::<GroupCallAnnounced>(payload["call"].clone()) else { return };
            forget_noticed(&ended.call_id);
            if let Err(e) = call.dismiss_group_call(&ended.call_id).await {
                report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": ended.call_id }));
            }
            stop_room(app, rt, &ended.call_id).await;
        }
        _ => {}
    }
}

/// The room `call_id` is over for me: its camera closes and its shell
/// stops, when it is the one shown as going on; the shell of anything
/// else is left alone. Then the call of two that rang in during the
/// room, if it still rings in the core, rings on the phone at last (see
/// [`rings_after_room`]).
#[cfg(target_os = "android")]
async fn stop_room(app: &tauri::AppHandle, rt: &Arc<MessengerRuntime>, call_id: &str) {
    let end = {
        let mut ongoing = lock(&ONGOING);
        let end = shell_end(ongoing.as_ref().map(|(id, _)| id.as_str()), call_id);
        if end == ShellEnd::Stop {
            *ongoing = None;
        }
        end
    };
    close_camera(app, call_id).await;
    if end == ShellEnd::Stop {
        let Ok(call) = bridge(app) else { return };
        if let Err(e) = call.stop().await {
            report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": call_id }));
        }
    }
    let Ok(state) = rt.call_state().await else { return };
    let Some(view) = state.call else { return };
    let in_room = rt.group_call_state(None).await.call.is_some();
    if rings_after_room(in_room, Some(view.phase)) {
        eprintln!("messenger call: {} rang in during the room {call_id}; the room is over, it rings here now", view.call_id);
        ring(app, rt, &view).await;
    }
}

/// What the camera is asked for: the size the plan measures the phone at.
#[cfg(target_os = "android")]
const CAMERA_WIDTH: u32 = 640;
#[cfg(target_os = "android")]
const CAMERA_HEIGHT: u32 = 360;
/// The camera of a phone the core names when none was chosen yet.
#[cfg(target_os = "android")]
const CAMERA_FRONT: &str = "front";

/// Frames the camera's thread may leave for the task that hands them to
/// the engine before it drops new ones: the engine is asked for a frame
/// at a time, and a page that stalls must not pile the camera up.
#[cfg(target_os = "android")]
const CAMERA_QUEUE: usize = 2;

/// Where the frames of the camera go: into the call of two
/// (`call_push_video_frame`) or into the room of a group call
/// (`group_call_push_video_frame`). The two are the runtime's separate
/// services, one of them with a session at a time; the bridge opens the
/// camera for the one whose word asked for it.
#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Feed {
    Dm,
    Room,
}

/// The call whose camera is open, and which camera (`front` or `back`):
/// the camera follows `video_local` and `camera` of the call's state (the
/// core turns it on for a video call at the answer and at the start, the
/// user turns it on and off and flips it), and is opened once.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static CAMERA: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

/// The camera as the state of the call wants it: open and facing
/// `wanted` while my video is on, closed (`None`) otherwise. The frames
/// go through the plugin's sink to the engine of the runtime, as pushed
/// frames (NV21, turned by the engine).
#[cfg(target_os = "android")]
async fn sync_camera(app: &tauri::AppHandle, rt: &Arc<MessengerRuntime>, call_id: &str, wanted: Option<&str>, feed: Feed) {
    let Ok(call) = bridge(app) else { return };
    let open = lock(&CAMERA).clone();
    let open_for = open.as_ref().filter(|(id, _)| id == call_id).map(|(_, facing)| facing.as_str());
    let Some(facing) = wanted else {
        if open_for.is_some() {
            close_camera(app, call_id).await;
        }
        return;
    };
    match open_for {
        Some(f) if f == facing => return,
        Some(_) => {
            // The other camera, with the frames still flowing the same way.
            *lock(&CAMERA) = Some((call_id.to_string(), facing.to_string()));
            if let Err(e) = call.switch_camera().await {
                report(app, EVENT_DEBUG, serde_json::json!({ "error": e.to_string(), "callId": call_id }));
            }
            return;
        }
        None => {}
    }
    *lock(&CAMERA) = Some((call_id.to_string(), facing.to_string()));
    let (tx, rx) = tokio::sync::mpsc::channel::<tauri_plugin_veydan_call::camera::Frame>(CAMERA_QUEUE);
    tauri_plugin_veydan_call::camera::set_sink(Some(Box::new(move |frame| {
        // A full queue drops the frame: the newest come after it.
        let _ = tx.try_send(frame);
    })));
    spawn_camera_feed(rt.clone(), rx, call_id.to_string(), feed);
    // The phone's permission first: the system asks the user when it was
    // never asked, and the camera opens on a yes. An answer that does
    // not come (no window to ask from) is a no.
    let opened = match call.request_permission(tauri_plugin_veydan_call::PERMISSION_CAMERA).await {
        Ok(true) => call.start_camera(facing, CAMERA_WIDTH, CAMERA_HEIGHT).await.map_err(|e| e.to_string()),
        Ok(false) => Err("the camera is not allowed".to_string()),
        Err(e) => Err(e.to_string()),
    };
    match opened {
        Ok(()) => spawn_camera_log(app.clone(), call_id.to_string()),
        Err(error) => {
            close_camera(app, call_id).await;
            // The core believes my video is on and told the peer so: off
            // again, so that nobody waits for a picture that never comes,
            // and the state no longer asks for the camera at every event.
            let off = match feed {
                Feed::Dm => rt.call_set_video(VideoInput::Off).await.map(|_| ()),
                Feed::Room => rt.group_call_set_video(VideoInput::Off).await.map(|_| ()),
            };
            if let Err(e) = off {
                eprintln!("messenger call: my video could not be turned off after the camera failed: {e}");
            }
            report(
                app,
                EVENT_CAMERA_FAILED,
                serde_json::json!({ "callId": call_id, "error": error, "denied": camera_denied(&error) }),
            );
        }
    }
}

/// The camera of `call_id` closed, if it is open; its frames go nowhere.
#[cfg(target_os = "android")]
async fn close_camera(app: &tauri::AppHandle, call_id: &str) {
    {
        let mut open = lock(&CAMERA);
        if open.as_ref().map(|(id, _)| id.as_str()) != Some(call_id) {
            return;
        }
        *open = None;
    }
    tauri_plugin_veydan_call::camera::set_sink(None);
    if let Ok(call) = bridge(app) {
        let _ = call.stop_camera().await;
    }
}

/// Hands the frames of the camera to the engine, one at a time, until the
/// sink is dropped (the camera closed). A frame the engine refuses (no
/// session yet, the call ending) is dropped; the first refusal and every
/// hundredth after it go to the log.
#[cfg(target_os = "android")]
fn spawn_camera_feed(rt: Arc<MessengerRuntime>, mut rx: tokio::sync::mpsc::Receiver<tauri_plugin_veydan_call::camera::Frame>, call_id: String, feed: Feed) {
    tauri::async_runtime::spawn(async move {
        let mut refused = 0u64;
        while let Some(frame) = rx.recv().await {
            let pushed = match feed {
                Feed::Dm => rt.calls().push_video_frame(frame.into_pushed()).await,
                Feed::Room => rt.group_call_push_video_frame(frame.into_pushed()).await,
            };
            if let Err(e) = pushed {
                if refused % 100 == 0 {
                    eprintln!("messenger call: a frame of the camera was refused ({refused} before): {e}");
                }
                refused += 1;
            }
        }
        eprintln!("messenger call: the camera of {call_id} stopped feeding the engine ({refused} frames refused)");
    });
}

/// Writes what the camera does every few seconds while it is open for
/// `call_id`: the frames the Kotlin side packed and the Rust side took or
/// dropped, and what the last one cost. For the log of a phone, nothing
/// else.
#[cfg(target_os = "android")]
fn spawn_camera_log(app: tauri::AppHandle, call_id: String) {
    tauri::async_runtime::spawn(async move {
        let mut last = (0u64, 0u64);
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if lock(&CAMERA).as_ref().map(|(id, _)| id.as_str()) != Some(call_id.as_str()) {
                break;
            }
            let Ok(call) = bridge(&app) else { break };
            let kotlin = call.camera_stats().await.unwrap_or_default();
            let (taken, dropped, last_us) = tauri_plugin_veydan_call::camera::stats();
            let per_second = (taken + dropped).saturating_sub(last.0 + last.1) as f64 / 5.0;
            last = (taken, dropped);
            eprintln!("messenger call: camera {kotlin} rust taken {taken} dropped {dropped} last {last_us} us, {per_second:.1} frames/s");
        }
    });
}

/// Listens from the start of the messenger, for a press that started the
/// app, and carries the runtime's events to the plugin for as long as the
/// runtime runs. Waits for the plugin and for the runtime, which take their
/// places after the tasks are spawned.
#[cfg(target_os = "android")]
pub fn spawn_bridge(app: tauri::AppHandle) -> tauri::async_runtime::JoinHandle<()> {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        let rt = loop {
            if app.try_state::<VeydanCall<tauri::Wry>>().is_some() {
                if let Some(rt) = runtime(&app) {
                    break rt;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        let mut events = rt.ui_events();
        listen(&app).await;
        loop {
            match events.recv().await {
                Ok(ev) if ev.name.starts_with("call.") || ev.name.starts_with("group_call.") => {
                    on_event(&app, &rt, &ev.name, &ev.payload).await
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    })
}

/// The messenger stops (the module switched off, the app leaving) with a
/// call on the phone: the bridge's task is aborted before any end could
/// reach it, and the core ends no call on the phone's behalf. The phone
/// is given back what the bridge held — the camera, the shell of the call
/// (its service, notification, sound mode, wake lock) or a ringing — and
/// the bridge forgets them, so that the next start begins clean. Called
/// from `MessengerState::stop`, while the plugin is still there.
#[cfg(target_os = "android")]
pub async fn stop_bridge(app: &tauri::AppHandle) {
    let (ongoing, camera) = forget();
    tauri_plugin_veydan_call::camera::set_sink(None);
    let Ok(call) = bridge(app) else { return };
    if let Some((id, _)) = &camera {
        if let Err(e) = call.stop_camera().await {
            eprintln!("messenger call: the camera of {id} was not closed at the stop: {e}");
        }
    }
    if let Err(e) = call.stop().await {
        eprintln!("messenger call: the phone was not cleared at the stop: {e}");
    }
    if let Some((id, _)) = ongoing {
        eprintln!("messenger call: {id} was going on at the stop; the phone is cleared");
    }
}

#[cfg(target_os = "android")]
fn route_of(name: Option<&str>) -> CmdResult<AudioRoute> {
    serde_json::from_value(serde_json::json!(name.unwrap_or_default()))
        .map_err(|_| AppError::Other(format!("not a route: {}", name.unwrap_or_default())))
}

/// Where the sound of the call goes: the routes there are, or the sound
/// sent to one of them (see [`AudioRouteInput`]). Answers
/// `{ current, available }`; an error while no call holds the sound.
#[cfg_attr(not(mobile), allow(dead_code))]
#[tauri::command]
pub async fn messenger_call_audio_route(app: tauri::AppHandle, input: AudioRouteInput) -> CmdResult<serde_json::Value> {
    #[cfg(target_os = "android")]
    {
        let call = bridge(&app)?;
        let routes = match input.op.as_str() {
            "list" => call.list_audio_routes().await.map_err(failed)?,
            "set" => call.set_audio_route(route_of(input.route.as_deref())?).await.map_err(failed)?,
            other => return Err(AppError::Other(format!("not an op: {other}"))),
        };
        Ok(serde_json::to_value(routes).unwrap_or_default())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, input);
        Err(AppError::Other("the route of the sound is the phone's".into()))
    }
}

/// What the camera command is asked to do.
#[cfg_attr(not(mobile), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInput {
    /// `switch`: the other camera, front or back, while my video is on;
    /// `stats`: what the camera did so far.
    pub op: String,
}

/// The camera of the phone, which the plugin holds (the engine lists no
/// cameras here): the other one on `switch` (the core's choice, `front`
/// or `back`, which the bridge follows; the same as
/// `messenger_call_switch_camera`), or what it did so far on `stats`
/// (`{ running, facing, frames, dropped, width, height, rotation, packUs,
/// taken, refused, lastUs }`: the Kotlin side's count, and the frames the
/// Rust side handed to the engine or dropped for want of a call, with the
/// cost of the last one on the camera's thread).
#[cfg_attr(not(mobile), allow(dead_code))]
#[tauri::command]
pub async fn messenger_call_camera(app: tauri::AppHandle, input: CameraInput) -> CmdResult<serde_json::Value> {
    #[cfg(target_os = "android")]
    {
        let call = bridge(&app)?;
        match input.op.as_str() {
            "switch" => {
                if lock(&CAMERA).is_none() {
                    return Err(AppError::Other("my video is off".into()));
                }
                let rt = runtime(&app).ok_or_else(|| AppError::Other("the messenger is not running".into()))?;
                let view = rt.call_switch_camera(None).await.map_err(super::map_err)?;
                Ok(serde_json::json!({ "camera": view.camera }))
            }
            "stats" => {
                let mut stats = call.camera_stats().await.map_err(failed)?;
                let (taken, dropped, last_us) = tauri_plugin_veydan_call::camera::stats();
                if let Some(map) = stats.as_object_mut() {
                    map.insert("taken".into(), taken.into());
                    map.insert("refused".into(), dropped.into());
                    map.insert("lastUs".into(), last_us.into());
                }
                Ok(stats)
            }
            other => Err(AppError::Other(format!("not an op: {other}"))),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, input);
        Err(AppError::Other("the camera is the phone's".into()))
    }
}

/// For the developer: rings this phone with a call from nobody, or moves
/// that call along (see [`DebugRing`]). Answers what the plugin said.
#[cfg_attr(not(mobile), allow(dead_code))]
#[tauri::command]
pub async fn messenger_call_debug_ring(
    app: tauri::AppHandle,
    input: Option<DebugRing>,
) -> CmdResult<serde_json::Value> {
    let input = input.unwrap_or_default();
    #[cfg(target_os = "android")]
    {
        listen(&app).await;
        let call = bridge(&app)?;
        let op = input.op.as_deref().unwrap_or("ring");
        let video = input.video.unwrap_or(false);
        let rung = || lock(&RUNG).clone().ok_or_else(|| AppError::Other("no call rang".into()));
        match op {
            "ring" | "ongoing" => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or_default();
                let new = Rung {
                    call_id: format!("debug-{now}"),
                    name: input.name.clone().unwrap_or_else(|| "Veydan test".into()),
                    video,
                    hidden: input.hide_on_lockscreen.unwrap_or(false),
                };
                *lock(&RUNG) = Some(new.clone());
                if op == "ongoing" {
                    call.start_ongoing(&new.ongoing()).await.map_err(failed)?;
                    return Ok(serde_json::json!({ "callId": new.call_id }));
                }
                let incoming = Incoming {
                    call_id: new.call_id.clone(),
                    name: new.name,
                    avatar: None,
                    video: new.video,
                    hide_on_lockscreen: new.hidden,
                };
                let delay = input.delay_secs.unwrap_or(0).min(60);
                if delay > 0 {
                    let later = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        let Ok(call) = bridge(&later) else { return };
                        match call.show_incoming(&incoming).await {
                            Ok(shown) => report(&later, EVENT_DEBUG, serde_json::json!({ "shown": shown, "callId": incoming.call_id })),
                            Err(e) => report(&later, EVENT_DEBUG, serde_json::json!({ "error": e.to_string() })),
                        }
                    });
                    return Ok(serde_json::json!({ "callId": new.call_id, "inSecs": delay }));
                }
                let shown = call.show_incoming(&incoming).await.map_err(failed)?;
                report(&app, EVENT_DEBUG, serde_json::json!({ "shown": shown, "callId": incoming.call_id }));
                Ok(serde_json::json!({ "callId": incoming.call_id, "shown": shown }))
            }
            "dismiss" => {
                let r = rung()?;
                call.dismiss_incoming(Some(&r.call_id)).await.map_err(failed)?;
                Ok(serde_json::json!({ "callId": r.call_id }))
            }
            "routes" => Ok(serde_json::json!(call.list_audio_routes().await.map_err(failed)?)),
            "route" => {
                let route = route_of(input.route.as_deref())?;
                Ok(serde_json::json!(call.set_audio_route(route).await.map_err(failed)?))
            }
            "awake" => {
                let on = input.on.unwrap_or(true);
                call.keep_awake(on).await.map_err(failed)?;
                Ok(serde_json::json!({ "awake": on }))
            }
            "stop" => {
                *lock(&RUNG) = None;
                call.stop().await.map_err(failed)?;
                Ok(serde_json::json!({}))
            }
            other => Err(AppError::Other(format!("not an op: {other}"))),
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, input);
        Err(AppError::Other("calls on this phone are not supported".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On a call with Y, X calls: the core answers Busy and ends X with the
    /// same `call.ended`. The phone keeps Y's shell; only a ringing of X
    /// is dismissed. Y's own end clears the phone. While Y rings (nothing
    /// going on yet), a stale invitation X caught up from the relays ends
    /// as Missed: Y keeps ringing, since the dismissal names X; Y's own
    /// end is a dismissal too, which the plugin answers for the call that
    /// rings.
    #[test]
    fn the_end_of_a_busy_rejected_call_keeps_the_call_under_way() {
        assert_eq!(shell_end(Some("y"), "x"), ShellEnd::DismissOnly);
        assert_eq!(shell_end(Some("y"), "y"), ShellEnd::Stop);
        assert_eq!(shell_end(None, "x"), ShellEnd::DismissOnly);
        assert_eq!(shell_end(None, "y"), ShellEnd::DismissOnly);
    }

    /// Answer pressed on a push-rung call before the bridge listened, with
    /// the invitation already in: `pressed` accepts it directly, and the
    /// `call.incoming` buffered before that must not ring the answered
    /// call again. The runtime's phase decides; an answer expected is
    /// taken only while the call still rings in the runtime.
    #[test]
    fn a_buffered_incoming_of_a_call_taken_already_does_not_ring() {
        assert_eq!(incoming_step(false, Some(CallPhase::Incoming), false), IncomingStep::Ring);
        assert_eq!(incoming_step(true, Some(CallPhase::Incoming), false), IncomingStep::Accept);
        assert_eq!(incoming_step(false, Some(CallPhase::Connecting), false), IncomingStep::Skip);
        assert_eq!(incoming_step(false, Some(CallPhase::Active), false), IncomingStep::Skip);
        assert_eq!(incoming_step(true, Some(CallPhase::Reconnecting), false), IncomingStep::Skip);
        assert_eq!(incoming_step(true, Some(CallPhase::Ended), false), IncomingStep::Skip);
        assert_eq!(incoming_step(false, None, false), IncomingStep::Skip, "over, or another call is current");
        assert_eq!(incoming_step(true, None, false), IncomingStep::Skip);
    }

    /// In the room of a Trio, a contact calls me 1:1: the core lets it
    /// ring (its Busy is for a second call of two), but the phone must
    /// not — the ringing over the room ended in the plugin's `stop`,
    /// which took the room's service, notification and sound mode down
    /// for good. Not rung, whatever was pressed before (the answer would
    /// be refused by the core anyway); a call that moved on is skipped
    /// as before, room or not.
    #[test]
    fn a_call_ringing_in_during_a_room_does_not_ring_the_phone() {
        assert_eq!(incoming_step(false, Some(CallPhase::Incoming), true), IncomingStep::InRoom);
        assert_eq!(incoming_step(true, Some(CallPhase::Incoming), true), IncomingStep::InRoom);
        assert_eq!(incoming_step(false, Some(CallPhase::Ended), true), IncomingStep::Skip);
        assert_eq!(incoming_step(false, None, true), IncomingStep::Skip);
    }

    /// In the room of a Trio, the screen dark, Alice calls 1:1: not rung
    /// (above). I leave the room, by Hang up on its notification or on
    /// its page, while her call still rings in the core: the core takes
    /// my answer now, and the phone must ring at last — it once stayed
    /// silent, the push for the call dismissed as over with the room,
    /// and the call went missed. Not for a call that moved on or is
    /// gone, nor while another room took the shell at once.
    #[test]
    fn the_call_left_ringing_during_a_room_rings_once_the_room_is_over() {
        assert!(rings_after_room(false, Some(CallPhase::Incoming)));
        assert!(!rings_after_room(true, Some(CallPhase::Incoming)));
        assert!(!rings_after_room(false, Some(CallPhase::Active)));
        assert!(!rings_after_room(false, Some(CallPhase::Ended)));
        assert!(!rings_after_room(false, None));
    }

    /// "No content" in the notifications and a PIN on the app name nobody
    /// in the call notification, as on a computer; otherwise the name is
    /// shown, also with "hide on lock screen" (the plugin keeps it private
    /// there itself).
    #[test]
    fn no_content_and_the_app_lock_name_nobody() {
        assert_eq!(shown_name("Alice".into(), Content::SenderText, false), "Alice");
        assert_eq!(shown_name("Alice".into(), Content::Sender, false), "Alice");
        assert_eq!(shown_name("Alice".into(), Content::None, false), "");
        assert_eq!(shown_name("Alice".into(), Content::SenderText, true), "");
    }

    /// The camera refused for want of the permission is told apart from a
    /// camera that would not open, so that the page knows what to ask.
    #[test]
    fn a_camera_not_allowed_is_a_denied_permission() {
        assert!(camera_denied("call bridge: the camera is not allowed"));
        assert!(!camera_denied("call bridge: no camera"));
        // A refusal before the call, in the same words.
        assert!(camera_denied(&permission_refused("camera")));
        assert_eq!(permission_refused("microphone"), "the microphone is not allowed");
    }

    /// The messenger stops on a call: what the bridge held of the phone is
    /// answered once, to be given back, and forgotten, so that a late
    /// event or the next start finds nothing.
    #[test]
    fn the_stop_of_the_messenger_takes_what_the_bridge_held() {
        *lock(&ONGOING) = Some(("y".into(), true));
        *lock(&CAMERA) = Some(("y".into(), "front".into()));
        *lock(&EXPECTED) = Some(("x".into(), Instant::now()));
        let (ongoing, camera) = forget();
        assert_eq!(ongoing, Some(("y".to_string(), true)));
        assert_eq!(camera, Some(("y".to_string(), "front".to_string())));
        assert!(lock(&ONGOING).is_none());
        assert!(lock(&CAMERA).is_none());
        assert!(!take_expected("x"), "the answer expected before the stop is dropped");
        assert_eq!(forget(), (None, None));
    }

    /// The room of a group call holds the phone's shell from its first
    /// word (the room being made, my offer with the node) to my leaving;
    /// a word of a room I am out of stops it. The shell of a room ends
    /// only for the room shown as going on (`shell_end`), as for a call
    /// of two.
    #[test]
    fn a_room_holds_the_shell_until_i_am_out() {
        for phase in [GroupCallPhase::Starting, GroupCallPhase::Joining, GroupCallPhase::InRoom, GroupCallPhase::Reconnecting] {
            assert_eq!(room_step(phase), RoomStep::Ongoing, "{phase:?}");
        }
        assert_eq!(room_step(GroupCallPhase::Left), RoomStep::Over);
    }

    /// Alice starts a call in the Trio while Bob's phone is in his pocket:
    /// the quiet notification, once — the group announces the call again
    /// at every seat taken. Not for a call Bob started (his screen has
    /// it), nor for one he is in; a call he joined is noted as shown, so
    /// that his leaving it (announced again) is no news either. The call
    /// over, it is forgotten, so that
    /// a call started anew under the same id (never, but cheap) is news
    /// again; and the list of shown calls stays short.
    #[test]
    fn a_call_on_in_a_group_is_news_once_unless_it_is_mine() {
        const ALICE: &str = "a1";
        const BOB: &str = "b2";
        lock(&NOTICED).clear();
        assert!(group_call_is_news(false, ALICE, BOB, was_noticed("c1")));
        noticed("c1");
        assert!(!group_call_is_news(false, ALICE, BOB, was_noticed("c1")), "the next word of the same call: shown already");
        assert!(!group_call_is_news(false, BOB, BOB, false), "my own call");
        assert!(!group_call_is_news(true, ALICE, BOB, false), "a call I am in");
        assert!(forget_noticed("c1"));
        assert!(!forget_noticed("c1"), "forgotten once");
        assert!(group_call_is_news(false, ALICE, BOB, was_noticed("c1")));
        for n in 0..NOTICED_MAX + 3 {
            noticed(&format!("call-{n}"));
        }
        assert_eq!(lock(&NOTICED).len(), NOTICED_MAX);
        assert!(!was_noticed("call-0") && !was_noticed("call-2"), "the oldest forgotten first");
        assert!(was_noticed("call-3") && was_noticed(&format!("call-{}", NOTICED_MAX + 2)));
        lock(&NOTICED).clear();
    }

    /// The readings of the relays after `reopen`, one per look, and how
    /// the wait came out: the restart of a call is asked only once a relay
    /// is back, never on the stale `Connected` of a socket just closed.
    async fn relays_back_after(readings: &'static [bool]) -> (RelaysBack, usize) {
        let looks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = looks.clone();
        let connected = move || {
            let i = counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let up = readings[i.min(readings.len() - 1)];
            async move { up }
        };
        let every = Duration::from_millis(2);
        let back = relays_back(connected, every * 40, every * 10, every).await;
        (back, looks.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// A socket just closed still reads `Connected` for a moment, then the
    /// relay drops and comes back after its retry: the wait ends at that
    /// moment, well within its limit. Relays that read down from the first
    /// look (the drop was seen before the look) end it the same way.
    #[tokio::test]
    async fn the_restart_waits_for_a_relay_seen_down_and_back() {
        let (back, looks) = relays_back_after(&[true, true, false, false, false, false, true]).await;
        assert_eq!((back, looks), (RelaysBack::Reconnected, 7), "the stale reading is not a relay back");
        let (back, looks) = relays_back_after(&[false, false, true]).await;
        assert_eq!((back, looks), (RelaysBack::Reconnected, 3));
    }

    /// Relays that never drop had no socket to close (none was open):
    /// after a short while the restart goes on their reading as it is.
    /// Relays that never come back are given the whole time, no more.
    #[tokio::test]
    async fn the_restart_goes_on_without_a_drop_or_after_the_time() {
        let started = Instant::now();
        let (back, looks) = relays_back_after(&[true]).await;
        assert_eq!(back, RelaysBack::Unchanged);
        // A look every 2 ms until 20 ms: 11 looks at most, fewer when a
        // sleep overshoots; never the whole wait of 80 ms (41 looks).
        assert!((2..=12).contains(&looks), "about the drop's time, not the whole wait: {looks}");
        assert!(started.elapsed() >= Duration::from_millis(20) && started.elapsed() < Duration::from_millis(80));
        let started = Instant::now();
        let (back, looks) = relays_back_after(&[false]).await;
        assert_eq!(back, RelaysBack::TimedOut);
        assert!((12..=41).contains(&looks), "the whole wait: {looks}");
        assert!(started.elapsed() >= Duration::from_millis(80));
    }
}

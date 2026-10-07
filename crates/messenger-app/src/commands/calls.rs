// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Calls: the commands of the page, forwarded to the runtime
//! (`messenger_runtime::calls`). What a call does comes back as runtime
//! events on `messenger://event`: `call.incoming`, `call.state`,
//! `call.ended`, `call.stats`, `call.level`, with the types of
//! `ui/src/lib/messenger/generated/calls.ts`.
//!
//! Video: the frames of a call go to the page as raw bytes on a
//! `tauri::ipc::Channel` (`messenger_call_video_subscribe`), one message
//! a frame, I420 behind a small header ([`HEADER_BYTES`]; the layout is
//! in internal/messenger-wire.md §10, "Видео"). I420 rather than RGBA:
//! a third of the bytes through the IPC, and the page turns it into RGB
//! in a shader for less than the conversion would cost here
//! (`messenger-rtc/src/video.rs`).
//!
//! One frame is on its way to the page at a time: the page acknowledges
//! each by its `seq` (`messenger_call_video_ack`), and the next goes
//! only then; meanwhile only the newest frame that came is kept
//! ([`InFlight`]). The channel of Tauri does not wait for the page: a
//! body of a kilobyte or more is kept in a queue of the process until
//! the page fetches it, so without the acknowledgement a page that
//! draws slower than the frames come, or one paused in the background
//! while the call goes on, would pile them up at 10–50 MB a second
//! until the process is killed. There is no rate limit of its own: the
//! source already gives 30 a second at most, and the page takes what it
//! can draw.
//!
//! Nothing of a call is decided here; the phone's shell around one (the
//! ringing notification, the route of the sound, the camera) is
//! `call_android.rs`, a computer's ringing notification `desktop_notify.rs`.

use super::{map_err, MessengerState};
use messenger_runtime::calls::{CameraInfo, ScreenInfo, VideoFrame, VideoInput, VideoQuality, VideoTrack};
use messenger_runtime::{CallMedia, CallNodeInput, CallState, CallView, MessengerRuntime, RelayPolicy};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::ipc::{Channel, Response};
use tokio::sync::watch;
use veydan_core::CmdResult;

/// The header before the planes of a frame on the channel, little-endian:
/// `width: u32`, `height: u32`, `rotation: u32` (0, 90, 180, 270 degrees
/// clockwise to stand upright), `seq: u32` (counts the frames sent on
/// this subscription), `timestamp_us: i64` (the engine's clock, 0 when
/// unknown). Then the I420 planes packed tight: Y of width×height, U
/// and V of ⌈width/2⌉×⌈height/2⌉ each. A header with `width` and
/// `height` 0 and no planes is the last message: the subscription ended.
pub const HEADER_BYTES: usize = 24;

/// How long a subscription made before the call's media exists waits
/// between looks (a call that rings, or is being set up).
const MEDIA_POLL: Duration = Duration::from_millis(200);

/// One subscription of the page: its task, and where its acknowledgements
/// go (the `seq` of the last frame the page took).
struct Subscription {
    task: tauri::async_runtime::JoinHandle<()>,
    acked: watch::Sender<Option<u32>>,
}

/// The subscriptions of the page, by the id it was given.
fn subscriptions() -> &'static Mutex<HashMap<u64, Subscription>> {
    static SUBS: OnceLock<Mutex<HashMap<u64, Subscription>>> = OnceLock::new();
    SUBS.get_or_init(Default::default)
}

/// What is between the frames that come and the page: one frame on its
/// way at a time (the module's note), the newest of the rest waiting.
struct InFlight {
    /// The `seq` of the frame the page has not acknowledged yet.
    sent: Option<u32>,
    /// The frame to send next: the newest that came while one was away.
    pending: Option<Arc<VideoFrame>>,
    next_seq: u32,
}

impl InFlight {
    fn new() -> Self {
        Self { sent: None, pending: None, next_seq: 0 }
    }

    /// A frame came: it is the one to send next, whatever waited before.
    fn came(&mut self, frame: Arc<VideoFrame>) {
        self.pending = Some(frame);
    }

    /// The page took the frame `seq` (or a later one): the way is free.
    fn acked(&mut self, seq: u32) {
        if self.sent.is_some_and(|sent| seq >= sent) {
            self.sent = None;
        }
    }

    /// The frame to send now with its `seq`, when the way is free and
    /// one waits; it is on its way from here on.
    fn outgoing(&mut self) -> Option<(u32, Arc<VideoFrame>)> {
        if self.sent.is_some() {
            return None;
        }
        let frame = self.pending.take()?;
        let seq = self.next_seq;
        self.next_seq = seq.wrapping_add(1);
        self.sent = Some(seq);
        Some((seq, frame))
    }

    /// The `seq` the end marker carries: the next one.
    fn seq(&self) -> u32 {
        self.next_seq
    }
}

/// A frame as the channel carries it: the header, then the planes.
fn pack(frame: &VideoFrame, seq: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES + frame.data.len());
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.extend_from_slice(&(frame.rotation as u32).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&frame.timestamp_us.to_le_bytes());
    out.extend_from_slice(&frame.data);
    out
}

/// The last message of a subscription.
fn end_marker(seq: u32) -> Vec<u8> {
    pack(&VideoFrame { width: 0, height: 0, rotation: 0, timestamp_us: 0, data: vec![] }, seq)
}

/// Carries the frames of `track` to `channel` until the call is over, the
/// page is gone (a send fails) or the subscription is taken back, one
/// frame on its way at a time ([`InFlight`]; `acked` says when the page
/// took it). Before the call's media is there it waits; without a call
/// it ends at once.
async fn pump_frames(rt: Arc<MessengerRuntime>, track: VideoTrack, channel: Channel<Response>, mut acked: watch::Receiver<Option<u32>>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut flight = InFlight::new();
    'calls: loop {
        let Some(mut frames) = rt.call_video_frames(track).await else {
            if rt.calls().current().await.is_none() {
                break;
            }
            tokio::time::sleep(MEDIA_POLL).await;
            continue;
        };
        loop {
            if let Some((seq, frame)) = flight.outgoing() {
                if channel.send(Response::new(pack(&frame, seq))).is_err() {
                    return;
                }
            }
            tokio::select! {
                came = frames.recv() => match came {
                    Ok(frame) => flight.came(frame),
                    // Behind: the newest frame is the one to draw.
                    Err(RecvError::Lagged(_)) => {}
                    // The session is gone: with the call, or for another one.
                    Err(RecvError::Closed) => continue 'calls,
                },
                changed = acked.changed() => {
                    // The sender goes with the subscription: taken back.
                    if changed.is_err() {
                        return;
                    }
                    if let Some(seq) = *acked.borrow_and_update() {
                        flight.acked(seq);
                    }
                }
            }
        }
    }
    let _ = channel.send(Response::new(end_marker(flight.seq())));
}

/// A computer: the `call.*` events go to the notifications as well, where
/// a call that rings while the window is away rings too. Missed events
/// (the channel lagged behind `call.level`) are made up for by asking the
/// runtime which call rings now.
#[cfg(desktop)]
pub(crate) fn spawn_desktop_ring(
    rt: std::sync::Arc<messenger_runtime::MessengerRuntime>,
    desktop: std::sync::Arc<super::desktop_notify::DesktopNotify>,
) -> tauri::async_runtime::JoinHandle<()> {
    use tokio::sync::broadcast::error::RecvError;
    tauri::async_runtime::spawn(async move {
        let mut rx = rt.ui_events();
        loop {
            match rx.recv().await {
                Ok(ev) if ev.name.starts_with("call.") => desktop.take_call(&rt, &ev.name, &ev.payload).await,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => desktop.recheck_calls(&rt).await,
                Err(RecvError::Closed) => break,
            }
        }
    })
}

/// Call `peer` (hex or npub) with `media` (`audio` | `video`).
#[tauri::command]
pub async fn messenger_call_start(
    app: tauri::AppHandle,
    peer: String,
    media: CallMedia,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallView> {
    permissions_for_call(&app, media).await?;
    messenger.runtime()?.call_start(&peer, media).await.map_err(map_err)
}

/// A phone asks for the microphone (and the camera of a video call)
/// before a call starts or is taken, and refuses the call on a no
/// (`call_android::permissions_for_call`); a computer asks nothing.
async fn permissions_for_call(app: &tauri::AppHandle, media: CallMedia) -> CmdResult<()> {
    #[cfg(mobile)]
    {
        super::call_android::permissions_for_call(app, media).await
    }
    #[cfg(not(mobile))]
    {
        let _ = (app, media);
        Ok(())
    }
}

/// Take the ringing call.
#[tauri::command]
pub async fn messenger_call_accept(
    app: tauri::AppHandle,
    call_id: String,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallView> {
    let rt = messenger.runtime()?;
    // A phone asks for the microphone (and the camera of a video call)
    // first; a call the runtime does not have fails on its own below.
    let media = rt
        .call_state()
        .await
        .map_err(map_err)?
        .call
        .filter(|c| c.call_id == call_id)
        .map(|c| c.media)
        .unwrap_or(CallMedia::Audio);
    permissions_for_call(&app, media).await?;
    rt.call_accept(&call_id).await.map_err(map_err)
}

/// Refuse the ringing call; my other devices stop ringing too.
#[tauri::command]
pub async fn messenger_call_decline(call_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.call_decline(&call_id).await.map_err(map_err)
}

/// Hang up, or give up calling.
#[tauri::command]
pub async fn messenger_call_end(call_id: String, messenger: tauri::State<'_, MessengerState>) -> CmdResult<()> {
    messenger.runtime()?.call_end(&call_id).await.map_err(map_err)
}

#[tauri::command]
pub async fn messenger_call_mute(muted: bool, messenger: tauri::State<'_, MessengerState>) -> CmdResult<CallView> {
    messenger.runtime()?.call_set_mute(muted).await.map_err(map_err)
}

/// My video in the call under way: `{kind: "camera", id?}`, `{kind:
/// "screen", id?}` or `{kind: "off"}`. The peer is told; nothing is
/// renegotiated. A camera that will not open is an error, and the call
/// goes on without the video.
#[tauri::command]
pub async fn messenger_call_set_video(input: VideoInput, messenger: tauri::State<'_, MessengerState>) -> CmdResult<CallView> {
    messenger.runtime()?.call_set_video(input).await.map_err(map_err)
}

/// The next camera of the list (or the one `camera` names): switched at
/// once when my camera is on, kept for when it goes on otherwise.
#[tauri::command]
pub async fn messenger_call_switch_camera(
    camera: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallView> {
    messenger.runtime()?.call_switch_camera(camera).await.map_err(map_err)
}

/// The cameras this machine has, the default first; empty on a phone
/// (its cameras are `front` and `back`) and without one.
#[tauri::command]
pub async fn messenger_call_list_cameras(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<CameraInfo>> {
    Ok(messenger.runtime()?.call_cameras().await)
}

/// The screens and windows that can be shared.
#[cfg(desktop)]
#[tauri::command]
pub async fn messenger_call_list_screens(messenger: tauri::State<'_, MessengerState>) -> CmdResult<Vec<ScreenInfo>> {
    Ok(messenger.runtime()?.call_screens().await)
}

/// My screen (or the window `screen` names) instead of my camera, in the
/// call under way; `messenger_call_set_video` with `off` or a camera
/// ends it.
#[cfg(desktop)]
#[tauri::command]
pub async fn messenger_call_share_screen(
    screen: Option<String>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallView> {
    messenger.runtime()?.call_set_video(VideoInput::Screen { id: screen }).await.map_err(map_err)
}

/// `360p` | `720p`, for the next time my video goes on.
#[tauri::command]
pub async fn messenger_call_set_video_quality(
    quality: VideoQuality,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallState> {
    messenger.runtime()?.call_set_video_quality(quality).await.map_err(map_err)
}

/// The frames of `track` (`local` | `remote`) of the call under way, on
/// `channel` as raw bytes ([`HEADER_BYTES`]), until the call ends (the
/// last message is an empty header) or `messenger_call_video_unsubscribe`
/// takes the id back. The page acknowledges every frame it took with
/// `messenger_call_video_ack` and gets the next one after that, not
/// before. Made before the call's media is there (ringing, connecting)
/// it waits for it; made without a call it ends at once.
#[tauri::command]
pub async fn messenger_call_video_subscribe(
    track: VideoTrack,
    channel: Channel<Response>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<u64> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let rt = messenger.runtime()?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (acked, acked_rx) = watch::channel(None);
    let task = tauri::async_runtime::spawn(async move {
        pump_frames(rt, track, channel, acked_rx).await;
        subscriptions().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    });
    subscriptions().lock().unwrap_or_else(|e| e.into_inner()).insert(id, Subscription { task, acked });
    Ok(id)
}

/// The page took (drew, or let go of) the frame `seq` of the subscription
/// `id`: the next frame may come. Without it no further frame is sent.
/// Nothing for an id that is over already.
#[tauri::command]
pub async fn messenger_call_video_ack(id: u64, seq: u32) -> CmdResult<()> {
    if let Some(sub) = subscriptions().lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        let _ = sub.acked.send(Some(seq));
    }
    Ok(())
}

/// No more frames on the subscription `id`; nothing for an id that is
/// over already.
#[tauri::command]
pub async fn messenger_call_video_unsubscribe(id: u64) -> CmdResult<()> {
    if let Some(sub) = subscriptions().lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        sub.task.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_packed_behind_its_header() {
        let frame = VideoFrame { width: 4, height: 2, rotation: 90, timestamp_us: 1_000_001, data: vec![7; 12] };
        let packed = pack(&frame, 5);
        assert_eq!(packed.len(), HEADER_BYTES + 12);
        assert_eq!(u32::from_le_bytes(packed[0..4].try_into().unwrap()), 4);
        assert_eq!(u32::from_le_bytes(packed[4..8].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(packed[8..12].try_into().unwrap()), 90);
        assert_eq!(u32::from_le_bytes(packed[12..16].try_into().unwrap()), 5);
        assert_eq!(i64::from_le_bytes(packed[16..24].try_into().unwrap()), 1_000_001);
        assert!(packed[HEADER_BYTES..].iter().all(|&b| b == 7));
        let end = end_marker(6);
        assert_eq!(end.len(), HEADER_BYTES);
        assert_eq!(&end[0..8], &[0; 8]);
        assert_eq!(u32::from_le_bytes(end[12..16].try_into().unwrap()), 6);
    }

    /// One frame on its way at a time: nothing more goes until the page
    /// acknowledged it, frames that come meanwhile replace each other and
    /// only the newest goes after the acknowledgement; an old
    /// acknowledgement frees nothing, a later one does.
    #[test]
    fn one_frame_is_on_its_way_at_a_time_and_the_newest_waits() {
        let frame = |n: i64| Arc::new(VideoFrame { width: 2, height: 2, rotation: 0, timestamp_us: n, data: vec![0; 6] });
        let mut f = InFlight::new();
        assert!(f.outgoing().is_none(), "nothing came");
        f.came(frame(1));
        let (seq, sent) = f.outgoing().expect("the first frame goes at once");
        assert_eq!((seq, sent.timestamp_us), (0, 1));
        f.came(frame(2));
        f.came(frame(3));
        assert!(f.outgoing().is_none(), "one is on its way: the rest wait");
        f.acked(0);
        let (seq, sent) = f.outgoing().expect("free again");
        assert_eq!((seq, sent.timestamp_us), (1, 3), "the newest of what came, not a backlog");
        assert!(f.outgoing().is_none());
        f.came(frame(4));
        f.acked(0);
        assert!(f.outgoing().is_none(), "an old acknowledgement frees nothing");
        f.acked(2);
        assert_eq!(f.outgoing().map(|(s, fr)| (s, fr.timestamp_us)), Some((2, 4)), "a later one does");
        assert_eq!(f.seq(), 3, "the end marker carries the next seq");
    }

    /// An acknowledgement of a frame never sent (the page acks `5` while
    /// `3` is on its way) counts as later: the page is past it either
    /// way, and a subscription must not get stuck on a miscount.
    #[test]
    fn a_later_acknowledgement_frees_the_way() {
        let frame = Arc::new(VideoFrame { width: 2, height: 2, rotation: 0, timestamp_us: 0, data: vec![0; 6] });
        let mut f = InFlight::new();
        for _ in 0..4 {
            f.came(frame.clone());
            let (seq, _) = f.outgoing().unwrap();
            f.acked(seq);
        }
        f.came(frame.clone());
        assert_eq!(f.outgoing().map(|(s, _)| s), Some(4));
        f.acked(9);
        f.came(frame);
        assert_eq!(f.outgoing().map(|(s, _)| s), Some(5));
    }
}

/// `auto` | `relay_only`, for the next call.
#[tauri::command]
pub async fn messenger_call_set_policy(
    policy: RelayPolicy,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallState> {
    messenger.runtime()?.call_set_policy(policy).await.map_err(map_err)
}

/// My own call nodes, replacing the list (the developer setting `call.nodes`).
#[tauri::command]
pub async fn messenger_call_set_nodes(
    nodes: Vec<CallNodeInput>,
    messenger: tauri::State<'_, MessengerState>,
) -> CmdResult<CallState> {
    messenger.runtime()?.call_set_nodes(nodes).await.map_err(map_err)
}

/// The call under way, the policy and the nodes.
#[tauri::command]
pub async fn messenger_call_get_state(messenger: tauri::State<'_, MessengerState>) -> CmdResult<CallState> {
    messenger.runtime()?.call_state().await.map_err(map_err)
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The screen of a computer, shared: libwebrtc's own desktop capturer
//! (`webrtc::DesktopCapturer`, through the `desktop_capturer` module of
//! the libwebrtc crate, 0.3.51) read on a thread of its own into a
//! [`VideoSource`] made for a screencast.
//!
//! The capturer gives frames of 4 bytes a pixel (B, G, R, A in memory,
//! libyuv's "ARGB") at the screen's own size, which are turned into
//! I420, scaled down to fit the size the source was made for (the
//! profile: 640×360 or 1280×720) and pushed. The encoder would scale a
//! bigger frame down by itself, but the frame also fans out to the
//! small picture of oneself and through the IPC to the page, where a
//! 4K screen would be 12 MB a frame. A screen changes little, so the
//! rate asked is modest ([`SCREEN_FPS`]) and the encoder, told it is a
//! screencast, keeps the text sharp.
//!
//! Platforms: X11 and PipeWire on Linux (PipeWire under Wayland needs
//! the portal's dialog and the `glib-main-loop` feature of the crate,
//! which is not on: a Wayland session shares nothing yet), DXGI and GDI
//! on Windows, ScreenCaptureKit on macOS (which asks the user once).
//! A phone shares no screen here.
//!
//! On Linux the capturer of libwebrtc talks to the desktop portal over
//! GIO (PipeWire itself it opens at run time), and `webrtc-sys` links
//! none of GLib on purpose (it takes the headers alone): the three
//! libraries are named here, so that a binary with the screen in it
//! links. Every machine that runs the app has them (WebKitGTK stands
//! on them), and so does the build (the headers of the bridge come from
//! the same packages).
//!
//! A source that goes away (the shared window closed: the capturer says
//! so for good) or fails again and again ends the thread, which tells
//! `on_lost` so that the call learns its video is gone. The drop waits
//! a bounded time for the thread, as for a camera (`camera.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use libwebrtc::desktop_capturer::{CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions};
use messenger_calls::engine::ScreenInfo;

use crate::camera::{stop_worker, OnLost};
use crate::video::{argb_to_i420, VideoSource};
use crate::{Error, Result};

#[cfg(target_os = "linux")]
#[link(name = "gio-2.0")]
#[link(name = "gobject-2.0")]
#[link(name = "glib-2.0")]
extern "C" {}

/// Frames a second asked of the screen.
pub const SCREEN_FPS: u32 = 15;

/// Failures of the capturer in a row after which the sharing gives up
/// (about two seconds at [`SCREEN_FPS`]).
const MAX_FAILURES: u32 = 30;

/// The prefix of a screen's id and of a window's: the two are listed by
/// two capturers.
const SCREEN: &str = "screen:";
const WINDOW: &str = "window:";

/// A screen or window being read into a source. Dropping it stops the
/// sharing.
pub struct ScreenCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// What the capturer's callback tells the loop: frames that failed in a
/// row, and whether the source is gone for good.
#[derive(Default)]
struct Health {
    failures: u32,
    gone: bool,
}

impl Health {
    /// `Some(why)` once the sharing should stop.
    fn note(&mut self, result: std::result::Result<(), CaptureError>) -> Option<String> {
        match result {
            Ok(()) => {
                self.failures = 0;
                None
            }
            Err(CaptureError::Permanent) => {
                self.gone = true;
                Some("the screen or window is no longer there".into())
            }
            Err(CaptureError::Temporary) => {
                self.failures += 1;
                (self.failures >= MAX_FAILURES).then(|| format!("{MAX_FAILURES} frames failed in a row"))
            }
        }
    }
}

impl ScreenCapture {
    /// The source `id` names (`ScreenInfo::id`, from [`list`]), or the
    /// first screen, read at [`SCREEN_FPS`] into `source` (scaled to fit
    /// the size it was made for) until the capture is dropped; `on_lost`
    /// is told if the sharing ends by itself before that. Fails where
    /// there is no display to read, or no such source.
    pub fn start(id: Option<&str>, source: VideoSource, on_lost: OnLost) -> Result<ScreenCapture> {
        let (window, wanted) = match id {
            Some(id) if id.starts_with(WINDOW) => (true, id[WINDOW.len()..].parse::<u64>().ok()),
            Some(id) if id.starts_with(SCREEN) => (false, id[SCREEN.len()..].parse::<u64>().ok()),
            Some(id) => return Err(Error::State(format!("not a screen or a window: {id}"))),
            None => (false, None),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let stop_in = stop.clone();
        let thread = std::thread::Builder::new()
            .name("veydan-screen".into())
            .spawn(move || {
                let kind = if window { DesktopCaptureSourceType::Window } else { DesktopCaptureSourceType::Screen };
                let mut options = DesktopCapturerOptions::new(kind);
                options.set_include_cursor(true);
                let Some(mut capturer) = DesktopCapturer::new(options) else {
                    let _ = ready_tx.send(Err("the screen cannot be captured here (no display)".into()));
                    return;
                };
                let sources = capturer.get_source_list();
                let picked = match wanted {
                    Some(id) => sources.into_iter().find(|s| s.id() == id),
                    None => sources.into_iter().next(),
                };
                let Some(picked) = picked else {
                    let _ = ready_tx.send(Err(match wanted {
                        Some(id) => format!("no such {} {id}", if window { "window" } else { "screen" }),
                        None => "no screen to share".into(),
                    }));
                    return;
                };
                tracing::info!(id = picked.id(), title = %picked.title(), window, "screen: sharing");
                let pushed = source.clone();
                let max = source.resolution();
                let health = Arc::new(std::sync::Mutex::new(Health::default()));
                let lost: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
                let (health_in, lost_in) = (health.clone(), lost.clone());
                capturer.start_capture(Some(picked), move |result| {
                    let verdict = match result {
                        Ok(frame) => {
                            let (w, h) = (frame.width().max(0) as u32, frame.height().max(0) as u32);
                            if w > 0 && h > 0 {
                                let converted = argb_to_i420(frame.data(), frame.stride(), w, h, max);
                                pushed.push(Arc::new(converted));
                            }
                            Ok(())
                        }
                        Err(e) => {
                            tracing::debug!(?e, "screen: a frame failed");
                            Err(e)
                        }
                    };
                    if let Some(why) = health_in.lock().unwrap_or_else(|e| e.into_inner()).note(verdict) {
                        lost_in.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(why);
                    }
                });
                let _ = ready_tx.send(Ok(()));
                let period = Duration::from_micros(1_000_000 / SCREEN_FPS as u64);
                let mut next = Instant::now();
                let why = loop {
                    if stop_in.load(Ordering::SeqCst) {
                        break None;
                    }
                    capturer.capture_frame();
                    if let Some(why) = lost.lock().unwrap_or_else(|e| e.into_inner()).take() {
                        tracing::warn!(%why, "screen: the sharing stops");
                        break Some(why);
                    }
                    next += period;
                    if let Some(wait) = next.checked_duration_since(Instant::now()) {
                        std::thread::sleep(wait);
                    } else {
                        next = Instant::now();
                    }
                };
                drop(capturer);
                if let Some(why) = why {
                    if !stop_in.load(Ordering::SeqCst) {
                        on_lost(why);
                    }
                }
            })
            .expect("a thread for the screen");
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(ScreenCapture { stop, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(Error::State(e))
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::State("the screen thread ended before it started".into()))
            }
        }
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        stop_worker("screen", &self.stop, self.thread.take());
    }
}

/// The screens and the windows that can be shared; empty where there is
/// no display.
pub fn list() -> Vec<ScreenInfo> {
    let mut out = Vec::new();
    for (kind, window, prefix) in [(DesktopCaptureSourceType::Screen, false, SCREEN), (DesktopCaptureSourceType::Window, true, WINDOW)] {
        let Some(capturer) = DesktopCapturer::new(DesktopCapturerOptions::new(kind)) else { continue };
        for s in capturer.get_source_list() {
            let title = s.title();
            out.push(ScreenInfo {
                id: format!("{prefix}{}", s.id()),
                title: if title.is_empty() { format!("{} {}", if window { "Window" } else { "Screen" }, s.id()) } else { title },
                window,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame that failed for good ends the sharing at once; passing
    /// failures end it only when there are too many in a row, and a good
    /// frame in between starts the count again.
    #[test]
    fn the_sharing_gives_up_on_a_source_that_is_gone_or_keeps_failing() {
        let mut h = Health::default();
        assert_eq!(h.note(Ok(())), None);
        assert!(h.note(Err(CaptureError::Permanent)).is_some());
        assert!(h.gone);

        let mut h = Health::default();
        for _ in 0..MAX_FAILURES - 1 {
            assert_eq!(h.note(Err(CaptureError::Temporary)), None);
        }
        assert_eq!(h.note(Ok(())), None, "a good frame starts the count again");
        for _ in 0..MAX_FAILURES - 1 {
            assert_eq!(h.note(Err(CaptureError::Temporary)), None);
        }
        assert!(h.note(Err(CaptureError::Temporary)).is_some());
        assert!(!h.gone);
    }
}

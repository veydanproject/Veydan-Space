// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The camera of a computer: listed, opened, read on a thread of its own
//! into a [`VideoSource`].
//!
//! Three platforms, two crates:
//!
//! - Linux: V4L2 through `rscam`, pure Rust over `ioctl`. The bindings
//!   of nokhwa for Linux need bindgen and a libclang when they build,
//!   which this repository's build has only inside the Android NDK, so
//!   they are not taken. The name of a camera comes from sysfs.
//! - Windows (Media Foundation) and macOS (AVFoundation): `nokhwa`
//!   without its decoders. Neither is built on this machine; the code
//!   follows nokhwa's documented API and waits for a run on the platform.
//! - Android: the plugin holds the camera and pushes its frames
//!   ([`crate::Session::push_video_frame`]); nothing here.
//!
//! Frames are taken uncompressed, YUYV or NV12, and converted
//! (`video.rs`). A camera that offers MJPEG alone is refused for now:
//! a JPEG decoder is a later addition (see the report of stage 6).
//! The size asked for is matched by the nearest the camera has.
//!
//! The thread must stop when told, whatever the device does. A device
//! that streams but delivers nothing (a virtual camera with no producer
//! behind it, a USB camera that went to sleep) would keep a blocking
//! read waiting forever, and whoever drops the capture with it: the
//! core does so under its state lock, so every command of the call
//! would hang. On Linux the device is therefore read with a timeout
//! (`poll` on its descriptor, made non-blocking); on every platform
//! the drop waits a bounded time for the thread and leaves it to end
//! by itself after that ([`stop_worker`]).
//!
//! A camera that fails for good in the middle (unplugged: the reads
//! answer with errors from then on) ends its thread, and says so to
//! whoever asked for it (`on_lost`), so that the call learns the video
//! is gone instead of believing it on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use messenger_calls::engine::{CameraInfo, VideoSettings};

use crate::video::VideoSource;
use crate::{Error, Result};

/// Failures of a camera in a row after which its thread gives up.
const MAX_FAILURES: u32 = 20;

/// How long a drop waits for the thread of a camera or a screen to end
/// before leaving it to end by itself.
pub(crate) const STOP_WAIT: Duration = Duration::from_millis(500);

/// Told once when a capture ended on its own (not when it was stopped):
/// why, in a word for the log and the screen.
pub type OnLost = Box<dyn FnOnce(String) + Send>;

/// A camera being read into a source. Dropping it stops the reading.
pub struct CameraCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    info: CameraInfo,
}

impl CameraCapture {
    /// The camera `id` names (`CameraInfo::id`, from [`list`]), or the
    /// first one, read at about `settings` into `source` until the
    /// capture is dropped; `on_lost` is told if the reading ends by
    /// itself before that. Opens the device here: a moment, run off the
    /// async threads.
    pub fn start(id: Option<&str>, settings: &VideoSettings, source: VideoSource, on_lost: OnLost) -> Result<CameraCapture> {
        let cameras = list();
        let info = match id {
            Some(id) => cameras.into_iter().find(|c| c.id == id).ok_or_else(|| Error::State(format!("no camera {id}")))?,
            None => cameras.into_iter().next().ok_or_else(|| Error::State("no camera on this machine".into()))?,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let thread = platform::spawn(&info, settings, source, stop.clone(), on_lost)?;
        Ok(CameraCapture { stop, thread: Some(thread), info })
    }

    pub fn info(&self) -> &CameraInfo {
        &self.info
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        stop_worker("camera", &self.stop, self.thread.take());
    }
}

/// Tells the thread of a capture to stop and waits [`STOP_WAIT`] for it.
/// One that is still inside the device after that (a blocking read the
/// platform gives no timeout for) is left to end by itself: it checks
/// `stop` as soon as the device lets it go, and a thread left behind
/// costs less than a call that cannot be hung up.
pub(crate) fn stop_worker(what: &str, stop: &AtomicBool, thread: Option<JoinHandle<()>>) {
    stop.store(true, Ordering::SeqCst);
    let Some(thread) = thread else { return };
    let deadline = Instant::now() + STOP_WAIT;
    while !thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    if thread.is_finished() {
        let _ = thread.join();
    } else {
        tracing::warn!("{what}: its thread is stuck in the device after {STOP_WAIT:?}; left to end by itself");
    }
}

/// The cameras of this machine, the default first; empty on a phone.
pub fn list() -> Vec<CameraInfo> {
    platform::list()
}

/// The loop that reads frames: `next` gives a frame, `Ok(None)` when
/// none came in time (the device is polled with a timeout, so that
/// `stop` is seen), or an error. `Ok(())` when stopped; `Err(why)` when
/// the device failed [`MAX_FAILURES`] times in a row and the reading
/// gave up.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn run<F>(stop: &AtomicBool, source: &VideoSource, mut next: F) -> std::result::Result<(), String>
where
    F: FnMut() -> std::result::Result<Option<messenger_calls::engine::PushedFrame>, String>,
{
    let mut failures = 0u32;
    while !stop.load(Ordering::SeqCst) {
        match next() {
            Ok(Some(frame)) => {
                failures = 0;
                if let Err(e) = source.push_raw(frame) {
                    tracing::debug!(error = %e, "camera: a frame was not taken");
                }
            }
            Ok(None) => {}
            Err(e) => {
                failures += 1;
                if failures >= MAX_FAILURES {
                    tracing::warn!(error = %e, "camera: {MAX_FAILURES} failures in a row, the capture stops");
                    return Err(e);
                }
                tracing::debug!(error = %e, "camera: a frame failed");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Ok(())
}

/// The end of a capture thread: the reading gave up on its own (not
/// stopped) is told to `on_lost`.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn report(stop: &AtomicBool, outcome: std::result::Result<(), String>, on_lost: OnLost) {
    if let Err(why) = outcome {
        if !stop.load(Ordering::SeqCst) {
            on_lost(why);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use messenger_calls::engine::{PixelFormat, PushedFrame};
    use std::os::unix::io::RawFd;

    /// The uncompressed layouts taken, in the order preferred.
    const FORMATS: [(&[u8; 4], PixelFormat); 2] = [(b"YUYV", PixelFormat::Yuyv), (b"NV12", PixelFormat::Nv12)];

    /// How long one wait for a frame lasts before `stop` is looked at.
    const POLL: Duration = Duration::from_millis(100);

    pub(super) fn list() -> Vec<CameraInfo> {
        let Ok(dir) = std::fs::read_dir("/dev") else { return vec![] };
        let mut paths: Vec<String> = dir
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("video") && n[5..].chars().all(|c| c.is_ascii_digit()))
            .map(|n| format!("/dev/{n}"))
            .collect();
        paths.sort_by_key(|p| p[10..].parse::<u32>().unwrap_or(u32::MAX));
        paths
            .into_iter()
            .filter_map(|path| {
                // A node without a capture format is a metadata node of
                // the same camera, or an output: not a camera.
                let camera = rscam::new(&path).ok()?;
                let captures = camera.formats().any(|f| f.map(|f| FORMATS.iter().any(|(code, _)| **code == f.format)).unwrap_or(false));
                captures.then(|| CameraInfo { name: name_of(&path), id: path })
            })
            .collect()
    }

    /// The card's name from sysfs; the path when sysfs says nothing.
    fn name_of(path: &str) -> String {
        let node = path.trim_start_matches("/dev/");
        std::fs::read_to_string(format!("/sys/class/video4linux/{node}/name"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string())
    }

    /// The descriptor rscam opened for `path`, made non-blocking. rscam
    /// keeps it to itself and reads with a bare `VIDIOC_DQBUF`, which
    /// waits forever on a device that delivers nothing; the descriptor is
    /// found through `/proc/self/fd`, where the open device shows by its
    /// path. Taken only when it is the one description of that path in
    /// the process (a thread left behind by [`stop_worker`] may still
    /// hold the same device): `None` otherwise, and the reads block as
    /// before.
    fn nonblocking_fd(path: &str) -> Option<RawFd> {
        let dir = std::fs::read_dir("/proc/self/fd").ok()?;
        let mut found: Vec<RawFd> = dir
            .flatten()
            .filter(|e| std::fs::read_link(e.path()).is_ok_and(|target| target.to_str() == Some(path)))
            .filter_map(|e| e.file_name().to_str()?.parse().ok())
            .collect();
        if found.len() != 1 {
            return None;
        }
        let fd = found.pop()?;
        // SAFETY: fcntl on a descriptor this process holds open; the flag
        // changes how its reads wait and nothing else.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return None;
        }
        Some(fd)
    }

    /// Waits [`POLL`] for a frame to be ready on `fd`: `Ok(true)` when
    /// one is, `Ok(false)` when the time passed, an error when the
    /// device reports one.
    fn frame_ready(fd: RawFd) -> std::result::Result<bool, String> {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd, as declared, for the time given.
        let n = unsafe { libc::poll(&mut pfd, 1, POLL.as_millis() as i32) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            return if e.kind() == std::io::ErrorKind::Interrupted { Ok(false) } else { Err(e.to_string()) };
        }
        if n == 0 {
            return Ok(false);
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(format!("the device reports an error (poll 0x{:x})", pfd.revents));
        }
        Ok(true)
    }

    pub(super) fn spawn(info: &CameraInfo, settings: &VideoSettings, source: VideoSource, stop: Arc<AtomicBool>, on_lost: OnLost) -> Result<JoinHandle<()>> {
        let mut camera = rscam::new(&info.id).map_err(|e| Error::State(format!("{}: {e}", info.id)))?;
        let formats: Vec<[u8; 4]> = camera.formats().flatten().map(|f| f.format).collect();
        let (code, format) = FORMATS
            .iter()
            .find(|(code, _)| formats.contains(code))
            .map(|(code, f)| (**code, *f))
            .ok_or_else(|| Error::State(format!("{}: no uncompressed format (YUYV or NV12); it offers {}", info.id, describe(&formats))))?;
        let resolution = pick_resolution(camera.resolutions(&code).map_err(|e| Error::State(e.to_string()))?, (settings.width, settings.height));
        let interval = pick_interval(camera.intervals(&code, resolution).map_err(|e| Error::State(e.to_string()))?, settings.fps);
        let config = rscam::Config { interval, resolution, format: &code, ..Default::default() };
        camera.start(&config).map_err(|e| Error::State(format!("{}: {e}", info.id)))?;
        let fd = nonblocking_fd(&info.id);
        if fd.is_none() {
            tracing::warn!(camera = %info.id, "camera: its descriptor was not found, the reads block");
        }
        tracing::info!(camera = %info.id, ?resolution, ?interval, format = %String::from_utf8_lossy(&code), "camera: started");
        Ok(std::thread::Builder::new()
            .name("veydan-camera".into())
            .spawn(move || {
                let outcome = run(&stop, &source, || {
                    if let Some(fd) = fd {
                        if !frame_ready(fd)? {
                            return Ok(None);
                        }
                    }
                    let frame = match camera.capture() {
                        Ok(frame) => frame,
                        // Polled ready and gone again (another reader of
                        // the device): no frame this time, no failure.
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                        Err(e) => return Err(e.to_string()),
                    };
                    Ok(Some(PushedFrame {
                        format,
                        width: frame.resolution.0,
                        height: frame.resolution.1,
                        rotation: 0,
                        // The engine stamps the frame on arrival: a
                        // camera's own clock is not the engine's.
                        timestamp_us: 0,
                        data: frame.to_vec(),
                    }))
                });
                let _ = camera.stop();
                report(&stop, outcome, on_lost);
            })
            .expect("a thread for the camera"))
    }

    fn describe(formats: &[[u8; 4]]) -> String {
        formats.iter().map(|f| String::from_utf8_lossy(f).into_owned()).collect::<Vec<_>>().join(", ")
    }

    /// The size nearest by area to the one wanted.
    pub(super) fn pick_resolution(info: rscam::ResolutionInfo, want: (u32, u32)) -> (u32, u32) {
        let area = |(w, h): (u32, u32)| w as i64 * h as i64;
        let target = area(want);
        match info {
            rscam::ResolutionInfo::Discretes(list) => {
                list.into_iter().min_by_key(|r| (area(*r) - target).abs()).unwrap_or(want)
            }
            rscam::ResolutionInfo::Stepwise { min, max, step } => {
                // On the grid of `st` from `lo`; a step of zero is no grid.
                let clamp = |v: u32, lo: u32, hi: u32, st: u32| {
                    let v = v.clamp(lo, hi);
                    (v - lo).checked_div(st).map_or(v, |steps| lo + steps * st)
                };
                (clamp(want.0, min.0, max.0, step.0), clamp(want.1, min.1, max.1, step.1))
            }
        }
    }

    /// The frame interval nearest to `1/fps`.
    pub(super) fn pick_interval(info: rscam::IntervalInfo, fps: u32) -> (u32, u32) {
        let target = 1.0 / fps.max(1) as f64;
        let secs = |(n, d): (u32, u32)| if d == 0 { f64::MAX } else { n as f64 / d as f64 };
        match info {
            rscam::IntervalInfo::Discretes(list) => list
                .into_iter()
                .min_by(|a, b| (secs(*a) - target).abs().partial_cmp(&(secs(*b) - target).abs()).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((1, fps.max(1))),
            rscam::IntervalInfo::Stepwise { min, max, .. } => {
                // Intervals, so the shortest is the fastest.
                let want = (1, fps.max(1));
                if secs(want) < secs(min) {
                    min
                } else if secs(want) > secs(max) {
                    max
                } else {
                    want
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_nearest_size_and_rate_are_picked() {
            let sizes = rscam::ResolutionInfo::Discretes(vec![(160, 120), (640, 480), (1280, 720), (1920, 1080)]);
            assert_eq!(pick_resolution(sizes, (640, 360)), (640, 480));
            let sizes = rscam::ResolutionInfo::Discretes(vec![(640, 480), (1280, 720)]);
            assert_eq!(pick_resolution(sizes, (1280, 720)), (1280, 720));
            let step = rscam::ResolutionInfo::Stepwise { min: (32, 32), max: (1920, 1080), step: (16, 16) };
            assert_eq!(pick_resolution(step, (640, 360)), (640, 352));
            let rates = rscam::IntervalInfo::Discretes(vec![(1, 5), (1, 15), (1, 30), (1, 60)]);
            assert_eq!(pick_interval(rates, 30), (1, 30));
            let rates = rscam::IntervalInfo::Discretes(vec![(1, 10), (1, 25)]);
            assert_eq!(pick_interval(rates, 30), (1, 25));
            let step = rscam::IntervalInfo::Stepwise { min: (1, 60), max: (1, 5), step: (1, 1) };
            assert_eq!(pick_interval(step, 30), (1, 30));
            assert_eq!(pick_interval(rscam::IntervalInfo::Stepwise { min: (1, 15), max: (1, 5), step: (1, 1) }, 30), (1, 15));
        }

        /// On this machine (a container, WSL) there may be no camera at
        /// all: listing says so and panics on nothing; with one, its node
        /// has a name and a path.
        #[test]
        fn listing_the_cameras_does_not_fail_without_one() {
            let cameras = list();
            eprintln!("cameras: {cameras:?}");
            for c in &cameras {
                assert!(c.id.starts_with("/dev/video"));
                assert!(!c.name.is_empty());
            }
        }

        /// The descriptor of an open file is found by its path and made
        /// non-blocking; a path opened twice is left alone (which one it
        /// is cannot be told), and so is one not open at all. A file of
        /// the test stands in for the device: the kernel's table knows no
        /// difference.
        #[test]
        fn the_descriptor_of_the_device_is_found_once_and_made_nonblocking() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("video9");
            std::fs::write(&path, b"").unwrap();
            // As the kernel names it: the link in /proc shows the real path.
            let path = std::fs::canonicalize(&path).unwrap().to_str().unwrap().to_string();
            assert_eq!(nonblocking_fd(&path), None, "not open");
            let one = std::fs::File::open(&path).unwrap();
            let fd = nonblocking_fd(&path).expect("the one descriptor of the path");
            use std::os::unix::io::AsRawFd;
            assert_eq!(fd, one.as_raw_fd());
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            assert_ne!(flags & libc::O_NONBLOCK, 0, "non-blocking now");
            // A regular file is always ready to read.
            assert_eq!(frame_ready(fd), Ok(true));
            let _two = std::fs::File::open(&path).unwrap();
            assert_eq!(nonblocking_fd(&path), None, "open twice: which one is the thread's cannot be told");
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform {
    use super::*;
    use messenger_calls::engine::{PixelFormat, PushedFrame};
    use nokhwa::pixel_format::YuyvFormat;
    use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
    use nokhwa::Camera;

    /// The uncompressed layouts taken.
    const FORMATS: [FrameFormat; 2] = [FrameFormat::YUYV, FrameFormat::NV12];

    pub(super) fn list() -> Vec<CameraInfo> {
        let Some(backend) = nokhwa::native_api_backend() else { return vec![] };
        nokhwa::query(backend)
            .unwrap_or_default()
            .into_iter()
            .map(|c| CameraInfo { id: c.index().to_string(), name: c.human_name() })
            .collect()
    }

    fn index_of(id: &str) -> CameraIndex {
        match id.parse::<u32>() {
            Ok(n) => CameraIndex::Index(n),
            Err(_) => CameraIndex::String(id.to_string()),
        }
    }

    pub(super) fn spawn(info: &CameraInfo, settings: &VideoSettings, source: VideoSource, stop: Arc<AtomicBool>, on_lost: OnLost) -> Result<JoinHandle<()>> {
        let index = index_of(&info.id);
        let wanted = CameraFormat::new(Resolution::new(settings.width, settings.height), FrameFormat::YUYV, settings.fps);
        let requested = RequestedFormat::with_formats(RequestedFormatType::Closest(wanted), &FORMATS);
        // The camera is opened on its own thread: the platform's objects
        // are not always sendable, and the opening may take a moment.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let id = info.id.clone();
        let thread = std::thread::Builder::new()
            .name("veydan-camera".into())
            .spawn(move || {
                let mut camera = match Camera::new(index, requested).and_then(|mut c| c.open_stream().map(|()| c)) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("{id}: {e}")));
                        return;
                    }
                };
                let format = camera.frame_format();
                let pixel = match format {
                    FrameFormat::YUYV => PixelFormat::Yuyv,
                    FrameFormat::NV12 => PixelFormat::Nv12,
                    other => {
                        let _ = ready_tx.send(Err(format!("{id}: the camera gives {other:?}, not YUYV or NV12")));
                        return;
                    }
                };
                let resolution = camera.resolution();
                tracing::info!(camera = %id, ?resolution, ?format, "camera: started");
                let _ = ready_tx.send(Ok(()));
                // nokhwa's read blocks until a frame: a device that gives
                // none holds the thread, and the drop leaves it behind
                // after STOP_WAIT (stop_worker).
                let outcome = run(&stop, &source, || {
                    let data = camera.frame_raw().map_err(|e| e.to_string())?.into_owned();
                    Ok(Some(PushedFrame { format: pixel, width: resolution.width(), height: resolution.height(), rotation: 0, timestamp_us: 0, data }))
                });
                let _ = camera.stop_stream();
                report(&stop, outcome, on_lost);
            })
            .expect("a thread for the camera");
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(thread),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(Error::State(e))
            }
            Err(_) => {
                let _ = thread.join();
                Err(Error::State(format!("{}: the camera thread ended before it started", info.id)))
            }
        }
    }

    #[allow(dead_code)]
    const _: () = {
        // YuyvFormat is named so that the decoder set of nokhwa stays in
        // view should a decoding path be added; the raw frames are taken.
        let _ = std::mem::size_of::<YuyvFormat>();
    };
}

#[cfg(target_os = "android")]
mod platform {
    use super::*;

    pub(super) fn list() -> Vec<CameraInfo> {
        vec![]
    }

    pub(super) fn spawn(info: &CameraInfo, _: &VideoSettings, _: VideoSource, _: Arc<AtomicBool>, _: OnLost) -> Result<JoinHandle<()>> {
        Err(Error::State(format!("{}: on a phone the plugin holds the camera and pushes its frames", info.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loop stops when told, between frames and while waiting for
    /// one; it gives up after `MAX_FAILURES` errors in a row and says so,
    /// but not when it was stopped meanwhile.
    #[test]
    fn the_reading_stops_when_told_and_reports_when_it_gives_up() {
        let (fan_out, _keep) = tokio::sync::broadcast::channel(2);
        let source = VideoSource::new(4, 2, false, fan_out);
        let stop = AtomicBool::new(false);
        let mut waits = 0;
        let outcome = run(&stop, &source, || {
            waits += 1;
            if waits == 3 {
                stop.store(true, Ordering::SeqCst);
            }
            Ok(None)
        });
        assert_eq!(outcome, Ok(()));
        assert_eq!(waits, 3, "stop is seen after every wait");

        let stop = AtomicBool::new(false);
        let outcome = run(&stop, &source, || Err("unplugged".to_string()));
        assert_eq!(outcome, Err("unplugged".to_string()));
        let told = std::sync::Arc::new(std::sync::Mutex::new(None));
        let t = told.clone();
        report(&stop, outcome, Box::new(move |why| *t.lock().unwrap() = Some(why)));
        assert_eq!(told.lock().unwrap().as_deref(), Some("unplugged"));

        // Stopped while it failed: no report, the stop was wanted.
        stop.store(true, Ordering::SeqCst);
        let t = told.clone();
        *told.lock().unwrap() = None;
        report(&stop, Err("gone".into()), Box::new(move |why| *t.lock().unwrap() = Some(why)));
        assert_eq!(*told.lock().unwrap(), None);
    }

    /// A thread that ends when told is joined; one that never does is
    /// left behind after `STOP_WAIT`, and the drop returns.
    #[test]
    fn stopping_waits_a_bounded_time_for_the_thread() {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let quick = std::thread::spawn(move || while !s.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        });
        let t = Instant::now();
        stop_worker("test", &stop, Some(quick));
        assert!(t.elapsed() < STOP_WAIT, "joined as soon as it ended");

        let stop = Arc::new(AtomicBool::new(false));
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let stuck = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let t = Instant::now();
        stop_worker("test", &stop, Some(stuck));
        let waited = t.elapsed();
        assert!(waited >= STOP_WAIT && waited < STOP_WAIT * 4, "left behind after {waited:?}");
        assert!(stop.load(Ordering::SeqCst));
        let _ = release_tx.send(());
    }
}

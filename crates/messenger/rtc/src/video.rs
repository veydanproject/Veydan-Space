// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Video: frames into the engine and out of it, and the shapes they come
//! in.
//!
//! One frame is the core's [`VideoFrame`]: I420 with its three planes
//! packed tight, and a rotation. A [`VideoSource`] takes frames from
//! whatever captures (the camera thread of `camera.rs`, the screen of
//! `screen.rs`, the plugin of a phone through [`VideoSource::push_raw`],
//! a test pattern) and hands them to the encoder; it also fans them out
//! to whoever watches this side's own picture. A [`VideoOutput`] hands
//! out what the far end sent, decoded.
//!
//! Why I420 and not RGBA on the way to the page: a 640×360 frame is
//! 346 KB as I420 and 922 KB as RGBA, 30 of them a second; the shader of
//! the page turns YUV into RGB in 0.2–0.5 ms a frame, the conversion here
//! would cost about the same (libyuv, 26 µs on x86_64) and then three
//! times the bytes through the IPC, which is the narrow place on a phone
//! (tmp/calls-spike/REPORT.md, section 6; measured again by the test
//! `i420_is_cheaper_than_rgba_on_the_way_to_the_page`).
//!
//! Conversions from what a camera gives (YUYV of a USB camera, NV12 and
//! NV21 of a phone) go through libyuv where it has the routine and
//! through a plain loop where the Rust wrapper lacks it (YUYV).

use std::sync::Arc;

use futures_util::StreamExt;
use libwebrtc::native::yuv_helper;
use libwebrtc::video_frame::{I420Buffer, VideoBuffer, VideoFrame as LkFrame, VideoRotation};
use libwebrtc::video_source::native::NativeVideoSource;
use libwebrtc::video_source::VideoResolution;
use libwebrtc::video_stream::native::NativeVideoStream;
use tokio::sync::broadcast;

pub use messenger_calls::engine::{PixelFormat, PushedFrame, VideoFrame};

use crate::{Error, Result};

/// Frames kept for a late reader of the far end's video.
pub(crate) const OUTPUT_QUEUE_FRAMES: usize = 2;
/// Frames a subscriber of either video may fall behind before it skips
/// to the newest: a page that stalls gets the present, not a backlog.
pub(crate) const FAN_OUT_FRAMES: usize = 2;

/// Where the frames this side sends come from. Fed by whatever captures;
/// cheap to clone (a handle on one source). Frames pushed into it reach
/// the encoder and everybody who subscribed to this side's own picture.
#[derive(Clone)]
pub struct VideoSource {
    pub(crate) inner: NativeVideoSource,
    width: u32,
    height: u32,
    screencast: bool,
    fan_out: broadcast::Sender<Arc<VideoFrame>>,
}

impl VideoSource {
    /// A source of frames of about `width`×`height`; the engine scales
    /// what it sends to the bandwidth it has. `screencast` tells the
    /// encoder to keep text sharp rather than motion smooth. The frames
    /// go out on `fan_out` as well as to the encoder.
    ///
    /// `NativeVideoSource::new` of the crate pushes black frames at 10 a
    /// second until the first real one, to keep an encoder going; a
    /// session's source sits idle until the camera goes on, and those
    /// black frames would reach the far end as its first "video" (and
    /// be told as a size). So a camera source takes the constructor
    /// without that keepalive (`new_encoded`: the name is the crate's,
    /// the source takes raw frames all the same); a screen source keeps
    /// it for the screencast hint, which only that constructor sets,
    /// and the hint matters more than the few black frames before its
    /// first capture, which the thread pushes at once.
    pub(crate) fn new(width: u32, height: u32, screencast: bool, fan_out: broadcast::Sender<Arc<VideoFrame>>) -> VideoSource {
        let resolution = VideoResolution { width, height };
        let inner = if screencast { NativeVideoSource::new(resolution, true) } else { NativeVideoSource::new_encoded(resolution) };
        VideoSource { inner, width, height, screencast, fan_out }
    }

    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn is_screencast(&self) -> bool {
        self.screencast
    }

    /// One frame to the encoder and to the watchers of this side's own
    /// picture. `false` when the frame is malformed or the engine dropped
    /// it: adapting its rate, or with no encoder running yet (before the
    /// connection is made, every frame is dropped; the watchers of this
    /// side's own picture get it all the same).
    pub fn push(&self, frame: Arc<VideoFrame>) -> bool {
        if !frame.is_well_formed() {
            return false;
        }
        let (y, u, v) = frame.planes().expect("well formed");
        let mut buffer = I420Buffer::new(frame.width, frame.height);
        let (sy, su, sv) = buffer.strides();
        let (dy, du, dv) = buffer.data_mut();
        let (cw, ch) = VideoFrame::chroma_size(frame.width, frame.height);
        copy_plane(dy, sy as usize, y, frame.width as usize, frame.height as usize);
        copy_plane(du, su as usize, u, cw as usize, ch as usize);
        copy_plane(dv, sv as usize, v, cw as usize, ch as usize);
        let f = LkFrame { rotation: rotation_of(frame.rotation), timestamp_us: frame.timestamp_us, frame_metadata: None, buffer };
        let taken = self.inner.capture_frame(&f);
        // Nobody watching is fine.
        let _ = self.fan_out.send(frame);
        taken
    }

    /// A frame as the platform captured it, converted and pushed: an
    /// error for a malformed one, `Ok(false)` for one the engine dropped
    /// (see [`VideoSource::push`]), which is no fault of the pusher.
    pub fn push_raw(&self, frame: PushedFrame) -> Result<bool> {
        let frame = to_i420(frame)?;
        Ok(self.push(Arc::new(frame)))
    }

    /// This side's own frames as they are pushed. A reader that falls
    /// behind skips to the newest.
    pub fn frames(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        self.fan_out.subscribe()
    }
}

/// Rows of `width` bytes from the tight `src` into `dst` with `dst_stride`.
fn copy_plane(dst: &mut [u8], dst_stride: usize, src: &[u8], width: usize, rows: usize) {
    for row in 0..rows {
        let d = row * dst_stride;
        let s = row * width;
        dst[d..d + width].copy_from_slice(&src[s..s + width]);
    }
}

fn rotation_of(degrees: u16) -> VideoRotation {
    match degrees {
        90 => VideoRotation::VideoRotation90,
        180 => VideoRotation::VideoRotation180,
        270 => VideoRotation::VideoRotation270,
        _ => VideoRotation::VideoRotation0,
    }
}

fn degrees_of(rotation: VideoRotation) -> u16 {
    match rotation {
        VideoRotation::VideoRotation90 => 90,
        VideoRotation::VideoRotation180 => 180,
        VideoRotation::VideoRotation270 => 270,
        VideoRotation::VideoRotation0 => 0,
    }
}

/// A frame of any layout the platform gives, as I420. Refused when the
/// data is shorter than the size says, or the rotation is not one of
/// the four.
pub fn to_i420(frame: PushedFrame) -> Result<VideoFrame> {
    let PushedFrame { format, width, height, rotation, timestamp_us, data } = frame;
    if width == 0 || height == 0 || !matches!(rotation, 0 | 90 | 180 | 270) {
        return Err(Error::State(format!("a frame of {width}×{height} turned by {rotation}°")));
    }
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = VideoFrame::chroma_size(width, height);
    let (cw, ch) = (cw as usize, ch as usize);
    let need = match format {
        PixelFormat::I420 => VideoFrame::len_for(width, height),
        PixelFormat::Nv12 | PixelFormat::Nv21 => w * h + 2 * cw * ch,
        PixelFormat::Yuyv => w * h * 2,
    };
    if data.len() < need {
        return Err(Error::State(format!("a {format:?} frame of {width}×{height} needs {need} bytes, has {}", data.len())));
    }
    let data = match format {
        PixelFormat::I420 => {
            let mut data = data;
            data.truncate(need);
            data
        }
        PixelFormat::Nv12 | PixelFormat::Nv21 => {
            let mut out = vec![0u8; VideoFrame::len_for(width, height)];
            let (y, uv) = data.split_at(w * h);
            let (dy, rest) = out.split_at_mut(w * h);
            let (du, dv) = rest.split_at_mut(cw * ch);
            // libyuv's NV12 routine writes the first byte of each pair to
            // U and the second to V; NV21 has them the other way round, so
            // the destinations are swapped.
            let (first, second) = if format == PixelFormat::Nv12 { (du, dv) } else { (dv, du) };
            yuv_helper::nv12_to_i420(y, width, uv, cw as u32 * 2, dy, width, first, cw as u32, second, cw as u32, width as i32, height as i32);
            out
        }
        PixelFormat::Yuyv => yuyv_to_i420(&data, w, h),
    };
    Ok(VideoFrame { width, height, rotation, timestamp_us, data })
}

/// Y0 U Y1 V, two bytes a pixel, into tight I420 planes: the chroma of
/// two rows is averaged into one, as a camera's own converter would.
fn yuyv_to_i420(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (cw, ch) = VideoFrame::chroma_size(w as u32, h as u32);
    let (cw, ch) = (cw as usize, ch as usize);
    let mut out = vec![0u8; w * h + 2 * cw * ch];
    let (y_out, rest) = out.split_at_mut(w * h);
    let (u_out, v_out) = rest.split_at_mut(cw * ch);
    for row in 0..h {
        let line = &src[row * w * 2..(row + 1) * w * 2];
        let y_row = &mut y_out[row * w..(row + 1) * w];
        for (x, px) in line.as_chunks::<2>().0.iter().enumerate() {
            y_row[x] = px[0];
        }
        let even = row % 2 == 0;
        let last = row + 1 == h;
        for (cx, quad) in line.chunks(4).enumerate() {
            let (u, v) = (quad[1] as u16, quad.get(3).copied().unwrap_or(quad[1]) as u16);
            let at = (row / 2) * cw + cx;
            if even {
                // The first row of the pair: kept whole when it is the
                // last row of the picture, halved otherwise.
                if last {
                    u_out[at] = u as u8;
                    v_out[at] = v as u8;
                } else {
                    u_out[at] = (u / 2) as u8;
                    v_out[at] = (v / 2) as u8;
                }
            } else {
                u_out[at] = (u_out[at] as u16 + u.div_ceil(2)) as u8;
                v_out[at] = (v_out[at] as u16 + v.div_ceil(2)) as u8;
            }
        }
    }
    out
}

/// A frame of 4 bytes a pixel in libyuv's ARGB order (B, G, R, A in
/// memory: what the desktop capturer gives) into I420, scaled down to
/// fit within `max` (width, height) when it is bigger, its shape kept.
/// A screen is read at its own size, and a 4K one is 12 MB a frame as
/// I420: more than the encoder would ever send and far too much for
/// the small picture of oneself on the way to the page.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub(crate) fn argb_to_i420(src: &[u8], stride: u32, width: u32, height: u32, max: (u32, u32)) -> VideoFrame {
    let mut buffer = I420Buffer::new(width, height);
    let (sy, su, sv) = buffer.strides();
    let (dy, du, dv) = buffer.data_mut();
    yuv_helper::argb_to_i420(src, stride, dy, sy, du, su, dv, sv, width as i32, height as i32);
    let (w, h) = fit_within((width, height), max);
    if (w, h) == (width, height) {
        tight(&buffer, 0, 0)
    } else {
        // libyuv's I420Scale with its box filter, as the encoder scales.
        tight(&buffer.scale(w as i32, h as i32), 0, 0)
    }
}

/// `size` scaled down to fit within `max`, its shape kept, both sides
/// even (the chroma planes are half of each); unchanged when it fits.
pub(crate) fn fit_within(size: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (w, h) = size;
    let (mw, mh) = max;
    if w == 0 || h == 0 || mw == 0 || mh == 0 || (w <= mw && h <= mh) {
        return size;
    }
    // The tighter of the two ratios, in integers.
    let (sw, sh) = if (w as u64) * (mh as u64) > (h as u64) * (mw as u64) {
        (mw, ((h as u64 * mw as u64) / w as u64) as u32)
    } else {
        (((w as u64 * mh as u64) / h as u64) as u32, mh)
    };
    (sw.max(2) & !1, sh.max(2) & !1)
}

/// A moving picture for the tests and the command line: a gradient that
/// slides with `seq`, and a bright square that travels across the
/// middle, so that a frozen or a skipped frame would show.
pub fn test_pattern(width: u32, height: u32, seq: u32) -> VideoFrame {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = VideoFrame::chroma_size(width, height);
    let (cw, ch) = (cw as usize, ch as usize);
    let mut data = Vec::with_capacity(VideoFrame::len_for(width, height));
    let shift = (seq as usize * 4) % w.max(1);
    let (sq_w, sq_h) = (w / 8, h / 4);
    let sq_x = (seq as usize * 6) % (w - sq_w).max(1);
    let (sq_top, sq_bottom) = (h / 2 - sq_h / 2, h / 2 + sq_h / 2);
    for y in 0..h {
        for x in 0..w {
            let in_square = x >= sq_x && x < sq_x + sq_w && y >= sq_top && y < sq_bottom;
            data.push(if in_square { 235 } else { 16 + (((x + shift) % w) * 200 / w) as u8 });
        }
    }
    for y in 0..ch {
        for _ in 0..cw {
            data.push(96 + (y * 64 / ch.max(1)) as u8);
        }
    }
    for _ in 0..ch {
        for x in 0..cw {
            data.push(96 + (((x + shift / 2) % cw.max(1)) * 64 / cw.max(1)) as u8);
        }
    }
    VideoFrame { width, height, rotation: 0, timestamp_us: 0, data }
}

/// Whether `frame` shows the bright square of [`test_pattern`]: the
/// brightest run of the middle row stands well above the gradient, as it
/// still does in a decoded copy of the pattern.
pub fn has_test_square(frame: &VideoFrame) -> bool {
    let Some((y, _, _)) = frame.planes() else { return false };
    let w = frame.width as usize;
    let row = (frame.height as usize) / 2;
    let line = &y[row * w..(row + 1) * w];
    let brightest = line.iter().copied().max().unwrap_or(0);
    let mean = line.iter().map(|&b| b as u32).sum::<u32>() / w.max(1) as u32;
    brightest >= 200 && brightest as u32 > mean + 60
}

/// Where the far end's frames come out. Available once its video track
/// arrived ([`crate::SessionEvent::RemoteVideo`]); a session pumps it
/// into the subscribers of the far end's video by itself. A reader that
/// is late gets the latest frames: only a couple are kept.
pub struct VideoOutput {
    stream: NativeVideoStream,
}

impl VideoOutput {
    pub(crate) fn new(stream: NativeVideoStream) -> Self {
        Self { stream }
    }

    /// The next decoded frame, packed tight, or `None` once the track is
    /// gone.
    pub async fn next(&mut self) -> Option<VideoFrame> {
        let frame = self.stream.next().await?;
        let rotation = degrees_of(frame.rotation);
        Some(match frame.buffer.as_i420() {
            Some(i420) => tight(i420, rotation, frame.timestamp_us),
            // A decoder that hands out another layout: every buffer of
            // libwebrtc converts.
            None => tight(&frame.buffer.to_i420(), rotation, frame.timestamp_us),
        })
    }
}

/// An I420 buffer of the engine, rows packed one after another.
fn tight(i420: &I420Buffer, rotation: u16, timestamp_us: i64) -> VideoFrame {
    let (width, height) = (i420.width(), i420.height());
    let (sy, su, sv) = i420.strides();
    let (y, u, v) = i420.data();
    let (cw, ch) = VideoFrame::chroma_size(width, height);
    let mut data = Vec::with_capacity(VideoFrame::len_for(width, height));
    for row in 0..height as usize {
        let s = row * sy as usize;
        data.extend_from_slice(&y[s..s + width as usize]);
    }
    for (plane, stride) in [(u, su), (v, sv)] {
        for row in 0..ch as usize {
            let s = row * stride as usize;
            data.extend_from_slice(&plane[s..s + cw as usize]);
        }
    }
    VideoFrame { width, height, rotation, timestamp_us, data }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn nv12_and_nv21_differ_only_in_the_order_of_the_chroma() {
        // 4×2: Y plane of 8, then 2×1 pairs of chroma.
        let y: Vec<u8> = (10..18).collect();
        let nv12 = PushedFrame { format: PixelFormat::Nv12, width: 4, height: 2, rotation: 90, timestamp_us: 7, data: [y.clone(), vec![100, 200, 110, 210]].concat() };
        let nv21 = PushedFrame { format: PixelFormat::Nv21, data: [y.clone(), vec![200, 100, 210, 110]].concat(), ..nv12.clone() };
        let a = to_i420(nv12).unwrap();
        let b = to_i420(nv21).unwrap();
        assert_eq!(a, b);
        assert_eq!((a.width, a.height, a.rotation, a.timestamp_us), (4, 2, 90, 7));
        let (py, pu, pv) = a.planes().unwrap();
        assert_eq!(py, &y[..]);
        assert_eq!(pu, &[100, 110]);
        assert_eq!(pv, &[200, 210]);
    }

    #[test]
    fn yuyv_averages_the_chroma_of_two_rows() {
        // 2×2 pixels: rows [Y0 U Y1 V].
        let data = vec![10, 100, 11, 200, 12, 110, 13, 210];
        let f = to_i420(PushedFrame { format: PixelFormat::Yuyv, width: 2, height: 2, rotation: 0, timestamp_us: 0, data }).unwrap();
        let (y, u, v) = f.planes().unwrap();
        assert_eq!(y, &[10, 11, 12, 13]);
        assert_eq!(u, &[105]);
        assert_eq!(v, &[205]);
        // An odd height keeps the last row's chroma whole.
        let data = vec![1, 50, 2, 60];
        let f = to_i420(PushedFrame { format: PixelFormat::Yuyv, width: 2, height: 1, rotation: 0, timestamp_us: 0, data }).unwrap();
        let (_, u, v) = f.planes().unwrap();
        assert_eq!((u, v), (&[50][..], &[60][..]));
    }

    #[test]
    fn a_frame_that_is_too_short_or_oddly_turned_is_refused() {
        let short = PushedFrame { format: PixelFormat::I420, width: 4, height: 4, rotation: 0, timestamp_us: 0, data: vec![0; 10] };
        assert!(to_i420(short).is_err());
        let turned = PushedFrame { format: PixelFormat::I420, width: 2, height: 2, rotation: 45, timestamp_us: 0, data: vec![0; 6] };
        assert!(to_i420(turned).is_err());
        let longer = PushedFrame { format: PixelFormat::I420, width: 2, height: 2, rotation: 180, timestamp_us: 0, data: vec![7; 9] };
        assert_eq!(to_i420(longer).unwrap().data.len(), 6, "extra bytes are cut");
    }

    #[test]
    fn the_test_pattern_has_its_square_and_moves() {
        let a = test_pattern(640, 360, 0);
        let b = test_pattern(640, 360, 10);
        assert!(a.is_well_formed() && b.is_well_formed());
        assert_ne!(a.data, b.data);
        assert!(has_test_square(&a) && has_test_square(&b));
        assert!(!has_test_square(&VideoFrame::black(640, 360)));
        assert!(test_pattern(33, 17, 3).is_well_formed(), "odd sizes");
    }

    #[test]
    fn argb_becomes_i420_of_the_same_size() {
        // 2×2 of white and black, 4 bytes a pixel, with a padded stride.
        let mut src = vec![0u8; 2 * 12];
        src[0..4].copy_from_slice(&[255, 255, 255, 255]);
        src[12..16].copy_from_slice(&[255, 255, 255, 255]);
        let f = argb_to_i420(&src, 12, 2, 2, (1280, 720));
        let (y, u, v) = f.planes().unwrap();
        assert!(y[0] > 200 && y[1] < 30 && y[2] > 200 && y[3] < 30, "{y:?}");
        assert_eq!((u.len(), v.len()), (1, 1));
    }

    /// A screen bigger than the profile is scaled down to fit it, its
    /// shape kept; one that fits is left as it is.
    #[test]
    fn a_big_screen_is_scaled_to_fit_the_profile() {
        assert_eq!(fit_within((3840, 2160), (1280, 720)), (1280, 720));
        assert_eq!(fit_within((2560, 1440), (640, 360)), (640, 360));
        assert_eq!(fit_within((1920, 1200), (1280, 720)), (1152, 720), "16:10 fits by its height");
        assert_eq!(fit_within((1080, 1920), (1280, 720)), (404, 720), "a tall screen fits by its height");
        assert_eq!(fit_within((1280, 720), (1280, 720)), (1280, 720));
        assert_eq!(fit_within((800, 600), (1280, 720)), (800, 600), "smaller stays");
        assert_eq!(fit_within((3000, 7), (1280, 720)), (1280, 2), "never below two");
        // A white 1920×1080 "screen" comes out white at 1280×720.
        let src = vec![255u8; 1920 * 1080 * 4];
        let f = argb_to_i420(&src, 1920 * 4, 1920, 1080, (1280, 720));
        assert_eq!((f.width, f.height), (1280, 720));
        assert!(f.is_well_formed());
        let (y, _, _) = f.planes().unwrap();
        assert!(y.iter().all(|&b| b > 200), "white throughout");
    }

    /// The figures behind the choice of I420 on the way to the page:
    /// what the IPC carries and what the conversion costs.
    #[test]
    fn i420_is_cheaper_than_rgba_on_the_way_to_the_page() {
        let frame = test_pattern(640, 360, 3);
        let (y, u, v) = frame.planes().unwrap();
        let (cw, _) = VideoFrame::chroma_size(640, 360);
        let mut rgba = vec![0u8; 640 * 360 * 4];
        let runs = 20;
        let t = Instant::now();
        for _ in 0..runs {
            yuv_helper::i420_to_abgr(y, 640, u, cw, v, cw, &mut rgba, 640 * 4, 640, 360);
        }
        let convert_us = t.elapsed().as_micros() as f64 / runs as f64;
        let t = Instant::now();
        for _ in 0..runs {
            let mut packed = Vec::with_capacity(20 + frame.data.len());
            packed.extend_from_slice(&[0u8; 20]);
            packed.extend_from_slice(&frame.data);
            std::hint::black_box(packed);
        }
        let pack_us = t.elapsed().as_micros() as f64 / runs as f64;
        eprintln!(
            "640x360: I420 {} B packed in {pack_us:.0} us; RGBA {} B after {convert_us:.0} us of conversion ({:.2}x the bytes)",
            frame.data.len(),
            rgba.len(),
            rgba.len() as f64 / frame.data.len() as f64
        );
        assert!(rgba.len() > 2 * frame.data.len());
    }
}

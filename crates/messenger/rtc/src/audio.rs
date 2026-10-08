// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Audio on the pushed path: frames in, frames out, and the processing
//! between them.
//!
//! Frames pushed through a `NativeAudioSource` go straight to the sinks of
//! the track, past the `AudioProcessing` of the factory, which sees the
//! capture of the platform device only (tmp/calls-spike/REPORT.md,
//! section 3). So a session on the pushed path runs an
//! `AudioProcessingModule` of its own: every frame the far end sent is
//! handed to it as the render stream as it is pulled ([`AudioOutput`]),
//! every frame pushed is its capture stream ([`AudioInput`]). The
//! canceller then removes from the capture what the output played. The
//! delay between the two is the caller's to tell ([`AudioInput::set_echo_delay_ms`]):
//! how long the frames took from being pulled to being heard by whatever
//! captured them.

use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::native::NativeAudioSource;
use libwebrtc::audio_stream::native::NativeAudioStream;
use libwebrtc::native::apm::AudioProcessingModule;
use tokio::sync::watch;

use crate::{Error, Result};

/// The one rate of the pushed path, Hz.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in one frame of the pushed path: 10 ms of mono at
/// [`SAMPLE_RATE`].
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE / 100) as usize;

/// What this crate's own processing does to the frames of the pushed path.
/// The default is the echo canceller with its high-pass filter: what a
/// call needs, measured by the spike (49 dB of echo removed). Everything
/// off means no module at all, and the frames go as pushed (a test that
/// measures a tone wants that).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioProcessing {
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub gain_control: bool,
    pub high_pass: bool,
}

impl Default for AudioProcessing {
    fn default() -> Self {
        Self { echo_cancellation: true, noise_suppression: false, gain_control: false, high_pass: true }
    }
}

impl AudioProcessing {
    /// No processing at all.
    pub const NONE: AudioProcessing =
        AudioProcessing { echo_cancellation: false, noise_suppression: false, gain_control: false, high_pass: false };

    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
}

/// Several frames of the same length summed into one, for a reader of
/// the pushed path that takes the outputs of a room (one per remote
/// track, [`crate::Session::take_audio_output_of`]) and wants to hear
/// them all, as the device path does by itself. Clipped, not scaled: a
/// voice stays as loud as it came. Frames of another length than the
/// first are skipped.
pub fn mix(frames: &[&[i16]]) -> Vec<i16> {
    let Some(first) = frames.first() else { return Vec::new() };
    let mut out: Vec<i32> = first.iter().map(|&s| i32::from(s)).collect();
    for frame in &frames[1..] {
        if frame.len() == out.len() {
            for (o, &s) in out.iter_mut().zip(frame.iter()) {
                *o += i32::from(s);
            }
        }
    }
    out.into_iter().map(|s| s.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16).collect()
}

/// The processing module shared by the input and the output of one
/// session.
pub(crate) struct Apm(Mutex<AudioProcessingModule>);

impl Apm {
    pub(crate) fn new(p: AudioProcessing) -> Option<Arc<Apm>> {
        if p.is_none() {
            return None;
        }
        let apm = AudioProcessingModule::new(p.echo_cancellation, p.gain_control, p.high_pass, p.noise_suppression);
        Some(Arc::new(Apm(Mutex::new(apm))))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AudioProcessingModule> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A playout or recording device of the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub index: u16,
    pub name: String,
    pub guid: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioDevices {
    pub playout: Vec<AudioDevice>,
    pub recording: Vec<AudioDevice>,
}

/// Where the caller pushes the frames the far end should hear
/// ([`crate::AudioMode::Pushed`]). Taken once from a session
/// ([`crate::Session::take_audio_input`]); `Clone` for a task that pumps.
#[derive(Clone)]
pub struct AudioInput {
    source: NativeAudioSource,
    apm: Option<Arc<Apm>>,
}

impl AudioInput {
    pub(crate) fn new(source: NativeAudioSource, apm: Option<Arc<Apm>>) -> Self {
        Self { source, apm }
    }

    /// One frame of exactly [`FRAME_SAMPLES`] samples, 48 kHz mono. Push
    /// one every 10 ms: the engine takes them as they come and plays no
    /// catch-up. The frame is processed in place (the echo canceller
    /// changes it) and then sent.
    pub async fn push(&self, frame: &mut [i16]) -> Result<()> {
        if frame.len() != FRAME_SAMPLES {
            return Err(Error::Audio(format!("a frame has {} samples, not {FRAME_SAMPLES}", frame.len())));
        }
        if let Some(apm) = &self.apm {
            apm.lock().process_stream(frame, SAMPLE_RATE as i32, 1)?;
        }
        let f = AudioFrame {
            data: (&*frame).into(),
            sample_rate: SAMPLE_RATE,
            num_channels: 1,
            samples_per_channel: FRAME_SAMPLES as u32,
        };
        self.source.capture_frame(&f).await?;
        Ok(())
    }

    /// How long a frame takes from [`AudioOutput::next`] to the capture
    /// that hears it again, for the echo canceller. Nothing to set when
    /// the output is never heard by the input (a file, a test).
    pub fn set_echo_delay_ms(&self, delay_ms: i32) -> Result<()> {
        if let Some(apm) = &self.apm {
            apm.lock().set_stream_delay_ms(delay_ms)?;
        }
        Ok(())
    }
}

/// Where the frames the far end sent come out ([`crate::AudioMode::Pushed`]).
/// Available once the far end's track arrived ([`crate::SessionEvent::RemoteAudio`]),
/// taken once ([`crate::Session::take_audio_output`]). The engine decodes
/// a frame every 10 ms whether or not anybody pulls; what is not pulled
/// within half a second is dropped, oldest first, so a slow reader hears
/// the present, not the past.
///
/// It ends ([`AudioOutput::next`] gives `None`) when its track is gone:
/// the m-line of a room's track closed
/// ([`crate::SessionEvent::RemoteTrackGone`]) or the session closed.
/// libwebrtc's own stream never ends by itself (its queue closes only
/// when the stream is dropped, and the reader holds it), so the session
/// keeps the other end of `ended` and pulls it.
pub struct AudioOutput {
    stream: NativeAudioStream,
    apm: Option<Arc<Apm>>,
    /// `true`, or the sender gone, once the track is.
    ended: watch::Receiver<bool>,
}

/// Frames kept for a reader that is late: half a second.
pub(crate) const OUTPUT_QUEUE_FRAMES: usize = 50;

/// The session's end of an output's `ended`.
pub(crate) type AudioEnd = watch::Sender<bool>;

impl AudioOutput {
    pub(crate) fn new(stream: NativeAudioStream, apm: Option<Arc<Apm>>) -> (AudioEnd, Self) {
        let (end, ended) = watch::channel(false);
        (end, Self { stream, apm, ended })
    }

    /// The next frame, [`FRAME_SAMPLES`] samples of 48 kHz mono, or `None`
    /// once the track is gone (and `None` from then on). The frame is
    /// what the far end sent, and it is the render reference of the echo
    /// canceller from here on.
    pub async fn next(&mut self) -> Option<Vec<i16>> {
        let frame = tokio::select! {
            biased;
            // Told to end, or the session let go of its end: the stream
            // is closed here (the reader holds it, nobody else can), so
            // that every later call finds its queue closed.
            _ = self.ended.changed() => {
                self.stream.close();
                return None;
            }
            frame = self.stream.next() => frame?,
        };
        let mut data: Vec<i16> = if frame.num_channels == 1 {
            frame.data.into_owned()
        } else {
            // The sink is asked for mono; should it ever give more, the
            // left channel is the frame.
            frame.data.chunks(frame.num_channels as usize).map(|c| c[0]).collect()
        };
        if let Some(apm) = &self.apm {
            if data.len() == FRAME_SAMPLES {
                if let Err(e) = apm.lock().process_reverse_stream(&mut data, SAMPLE_RATE as i32, 1) {
                    tracing::debug!(error = %e, "apm: reverse stream");
                }
            }
        }
        Some(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_summed_and_clipped() {
        assert_eq!(mix(&[&[1, 2, 3], &[10, 20, 30]]), vec![11, 22, 33]);
        assert_eq!(mix(&[&[i16::MAX, i16::MIN], &[10, -10]]), vec![i16::MAX, i16::MIN], "clipped");
        assert_eq!(mix(&[&[1, 2], &[7]]), vec![1, 2], "another length is skipped");
        assert!(mix(&[]).is_empty());
    }
}

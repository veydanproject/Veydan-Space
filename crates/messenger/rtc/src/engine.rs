// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The engine: one factory of libwebrtc and the way audio enters and
//! leaves it.

use std::sync::Arc;

use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;

use crate::audio::{AudioDevice, AudioDevices, AudioProcessing};
use crate::session::{Session, SessionConfig};
use crate::{Error, Result};

/// How audio enters and leaves the engine. Decided once, for every session
/// of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMode {
    /// The platform's audio device: the microphone through the engine's
    /// own echo canceller, the far end through the speaker.
    Device,
    /// The caller pushes and pulls 10 ms frames ([`crate::AudioInput`],
    /// [`crate::AudioOutput`]), with the processing named here run by this
    /// crate on them.
    Pushed(AudioProcessing),
}

/// The factory of libwebrtc. Make one per process and keep it: a session
/// borrows nothing from it, but the platform audio device lives as long as
/// the engine does.
pub struct Engine {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) factory: PeerConnectionFactory,
    pub(crate) mode: AudioMode,
}

impl Engine {
    /// The engine in `mode`. [`AudioMode::Device`] takes the platform's
    /// audio device module and fails where there is none to take.
    ///
    /// On Android it needs [`crate::android::init_android`] to have gone
    /// through first, in *every* mode: the factory's constructor reaches
    /// Java for the codec factories, and without the init that aborts the
    /// process instead of failing; so the engine is refused with an error
    /// before the factory is made. The init runs on the thread that came
    /// from Java (the plugin's entry), this may run anywhere.
    pub fn new(mode: AudioMode) -> Result<Engine> {
        platform_ready()?;
        let factory = PeerConnectionFactory::default();
        match mode {
            AudioMode::Device => {
                if !factory.acquire_platform_adm() {
                    return Err(Error::Engine("the platform audio device module could not be started".into()));
                }
                // The far end through the speaker; without this the engine
                // decodes into a synthetic device that nobody hears.
                factory.set_adm_playout_enabled(true);
                factory.set_adm_recording_enabled(true);
            }
            AudioMode::Pushed(_) => {
                // Nothing to start: the factory runs its synthetic device,
                // frames come and go through the session's tracks.
                factory.set_adm_playout_enabled(false);
                factory.set_adm_recording_enabled(false);
            }
        }
        Ok(Engine { inner: Arc::new(Inner { factory, mode }) })
    }

    pub fn mode(&self) -> AudioMode {
        self.inner.mode
    }

    /// A session: one PeerConnection with its audio track. Call it inside
    /// a tokio runtime; what the session reports arrives on its events.
    pub fn session(&self, config: SessionConfig) -> Result<Session> {
        Session::new(self.inner.clone(), config)
    }

    /// The audio devices of the platform ([`AudioMode::Device`] only;
    /// empty lists otherwise).
    pub fn audio_devices(&self) -> AudioDevices {
        let f = &self.inner.factory;
        if self.inner.mode != AudioMode::Device {
            return AudioDevices::default();
        }
        let list = |count: i16, name: &dyn Fn(u16) -> String, guid: &dyn Fn(u16) -> String| {
            (0..count.max(0) as u16).map(|i| AudioDevice { index: i, name: name(i), guid: guid(i) }).collect()
        };
        AudioDevices {
            playout: list(f.playout_devices(), &|i| f.playout_device_name(i), &|i| f.playout_device_guid(i)),
            recording: list(f.recording_devices(), &|i| f.recording_device_name(i), &|i| f.recording_device_guid(i)),
        }
    }

    /// The speaker, by its index in [`Engine::audio_devices`].
    pub fn set_playout_device(&self, index: u16) -> bool {
        self.inner.factory.set_playout_device(index)
    }

    /// The microphone, by its index in [`Engine::audio_devices`].
    pub fn set_recording_device(&self, index: u16) -> bool {
        self.inner.factory.set_recording_device(index)
    }

    /// Whether this engine opens the camera itself (a computer with its
    /// devices) or takes pushed frames (the pushed path of the CLI and
    /// the tests; a phone, whose plugin holds the camera).
    pub fn captures_video(&self) -> bool {
        self.inner.mode == AudioMode::Device && !cfg!(target_os = "android")
    }

    /// The cameras of this machine, the default first; empty on a phone
    /// and on the pushed path, where frames are pushed instead.
    pub fn cameras(&self) -> Vec<messenger_calls::engine::CameraInfo> {
        if self.captures_video() {
            crate::camera::list()
        } else {
            vec![]
        }
    }

    /// The screens and windows that can be shared (a computer; empty
    /// where there is no display).
    pub fn screens(&self) -> Vec<messenger_calls::engine::ScreenInfo> {
        #[cfg(not(target_os = "android"))]
        {
            crate::screen::list()
        }
        #[cfg(target_os = "android")]
        {
            vec![]
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if self.mode == AudioMode::Device {
            self.factory.release_platform_adm();
        }
    }
}

/// Whether the platform is ready for the factory of libwebrtc: on Android
/// only once [`crate::android::init_android`] succeeded (the module says
/// why and on which thread); everywhere else nothing is needed.
fn platform_ready() -> Result<()> {
    #[cfg(target_os = "android")]
    if !crate::android::ready() {
        return Err(Error::Engine(
            "Android: init_android must succeed before the engine is made, in every mode, \
             on the thread that entered Rust from Java"
                .into(),
        ));
    }
    Ok(())
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").field("mode", &self.inner.mode).finish()
    }
}

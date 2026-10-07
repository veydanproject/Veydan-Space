// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Android: the JVM and the application context, before the engine.
//!
//! On Android libwebrtc reaches Java over JNI: the audio device module
//! (`org.webrtc.audio.*` in `libwebrtc.jar`, which the plugin packages),
//! the hardware codec factories, and the class loader it resolves all of
//! them through. [`init_android`] hands it the JavaVM and an application
//! context. Two rules, and a breach of either aborts the process inside
//! libwebrtc (`RTC_CHECK` on a JNI lookup that failed) rather than coming
//! back as an error:
//!
//! 1. **Before the engine, in every mode.** [`crate::Engine::new`] makes
//!    the factory of PeerConnections, and the factory's constructor looks
//!    up `livekit/org/webrtc/DefaultVideoEncoderFactory` and the decoder's
//!    over JNI whether a session will ever carry video or not, and whether
//!    the audio is the device's ([`crate::AudioMode::Device`]) or pushed.
//!    Without the init there is no class loader to look them up through.
//!    So on Android [`crate::Engine::new`] refuses to run before
//!    [`init_android`] succeeded ([`ready`]), instead of aborting.
//! 2. **On a thread that came from Java.** The init resolves
//!    `livekit/org/webrtc/WebRtcClassLoader` and `.../ContextUtils` with
//!    the raw `FindClass`, which takes the class loader of the Java frame
//!    under the native one. A thread attached from native code (a tokio
//!    worker, where the first call would make the engine) has no Java
//!    frame: `FindClass` goes to the system class loader, which knows no
//!    class of the app. The init belongs in the plugin's entry from Java
//!    (`crates/tauri-plugin-veydan-call`, its setup on the main thread),
//!    not in the task that makes the first call.
//!
//! A wrong thread is caught here before libwebrtc sees it: the lookup is
//! tried through the `jni` crate first, which reports a failure instead of
//! aborting, and [`init_android`] answers `false`.

use std::sync::atomic::{AtomicBool, Ordering};

use jni::objects::JObject;
use jni::JavaVM;

/// Set once [`init_android`] went through; [`crate::Engine::new`] reads it.
static READY: AtomicBool = AtomicBool::new(false);

/// The first class libwebrtc's init resolves with the raw `FindClass`: when
/// this thread cannot, libwebrtc would abort on it.
const CLASS_LOADER: &str = "livekit/org/webrtc/WebRtcClassLoader";

/// Hands the JVM and the application context to libwebrtc. Idempotent:
/// `true` at once after a first success. `false` when this thread did not
/// come from Java (see the module), or the context could not be taken;
/// nothing then reached libwebrtc, and the engine stays refused.
///
/// Call it on the thread that entered Rust from Java, before the first
/// [`crate::Engine::new`] of the process, in every [`crate::AudioMode`].
pub fn init_android(vm: &JavaVM, context: &JObject) -> bool {
    if READY.load(Ordering::SeqCst) {
        return true;
    }
    if !finds_app_classes(vm) {
        tracing::error!(
            "init_android: this thread cannot resolve {CLASS_LOADER}; it did not come from Java \
             (call it from the plugin's entry on the main thread, not from a tokio task)"
        );
        return false;
    }
    let ok = libwebrtc::android::initialize_android_context(vm, context);
    if ok {
        READY.store(true, Ordering::SeqCst);
    } else {
        tracing::error!("init_android: libwebrtc could not take the application context");
    }
    ok
}

/// The same from the raw pointers `ndk-context` gives a library
/// (`android_context().vm()` and `.context()`). The same thread and order
/// rules as [`init_android`].
///
/// # Safety
/// `vm` is a live JavaVM of this process and `context` a global reference
/// to an `android.content.Context` that outlives this call.
pub unsafe fn init_android_raw(vm: *mut jni::sys::JavaVM, context: jni::sys::jobject) -> bool {
    let vm = match JavaVM::from_raw(vm) {
        Ok(vm) => vm,
        Err(_) => return false,
    };
    let context = JObject::from_raw(context);
    init_android(&vm, &context)
}

/// Whether [`init_android`] succeeded in this process: what
/// [`crate::Engine::new`] needs on Android.
pub fn ready() -> bool {
    READY.load(Ordering::SeqCst)
}

/// Can this thread resolve the app's classes the way libwebrtc will, with
/// `FindClass`? A thread attached from native code cannot (the system
/// class loader), and one not attached at all never came from Java. The
/// `jni` crate reports the failure, where libwebrtc would abort on it.
fn finds_app_classes(vm: &JavaVM) -> bool {
    let Ok(mut env) = vm.get_env() else { return false };
    match env.find_class(CLASS_LOADER) {
        Ok(_) => true,
        Err(_) => {
            // The ClassNotFoundException stays pending until cleared, and a
            // pending exception breaks the next JNI call of this thread.
            let _ = env.exception_clear();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    //! On the phone only (`cargo test --target aarch64-linux-android` under
    //! a host that runs the tests there); the crate's tests on a computer
    //! never compile this module.
    use super::*;

    /// Before the init the engine is refused in every mode, instead of
    /// aborting in the factory's constructor.
    #[test]
    fn the_engine_is_refused_before_the_init() {
        assert!(!ready());
        let err = crate::Engine::new(crate::AudioMode::Pushed(crate::AudioProcessing::NONE)).err().expect("refused");
        assert!(err.to_string().contains("init_android"), "{err}");
        let err = crate::Engine::new(crate::AudioMode::Device).err().expect("refused");
        assert!(err.to_string().contains("init_android"), "{err}");
    }
}

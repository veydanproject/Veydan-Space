fn main() {
    // The Java side of the engine of calls, on Android (the product's build
    // script alone can ask the linker for it).
    veydan_build_cfg::android_webrtc_jni_exports();
    tauri_build::build()
}

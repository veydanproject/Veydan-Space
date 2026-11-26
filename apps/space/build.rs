fn main() {
    // The Java side of the engine of calls, on Android (the product's build
    // script alone can ask the linker for it).
    veydan_build_cfg::android_webrtc_jni_exports();
    // Its Objective-C categories, on macOS (ROO-869: without -ObjC
    // the process dies before `main`).
    veydan_build_cfg::darwin_webrtc_objc();
    // The binary's own gtk_clipboard_request_targets, in its dynamic symbol
    // table, so WebKitGTK's call binds to it (src/linux_clipboard.rs).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,--export-dynamic-symbol=gtk_clipboard_request_targets"
        );
    }
    tauri_build::build()
}

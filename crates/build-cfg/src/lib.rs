// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a library crate needs from a build script and `tauri-build` gives
//! only to the application: the `desktop` / `mobile` cfg names Tauri uses
//! ([`platform_cfg`]) and, on Windows, the application manifest of its test
//! executables ([`windows_test_manifest`]).
//!
//! `tauri-build` (the application crate) and `tauri-plugin` (a plugin crate)
//! print these lines from their own build scripts, and a `cargo:rustc-cfg`
//! line applies only to the crate whose build script printed it. A plain
//! library therefore sees `cfg(desktop)` as false on every platform: the code
//! compiles and silently loses its desktop branches. A library that uses
//! either name has this crate as a build-dependency and a `build.rs` of one
//! line:
//!
//! ```ignore
//! fn main() { veydan_build_cfg::platform_cfg(); }
//! ```

/// The cfg name of the platform a target OS belongs to; the rule of `tauri-build`.
fn platform(target_os: &str) -> &'static str {
    match target_os {
        "android" | "ios" => "mobile",
        _ => "desktop",
    }
}

/// Declares both names to rustc and sets the one of the target: cargo gives a
/// build script the OS it compiles for in `CARGO_CFG_TARGET_OS`, which follows
/// `--target`, not the host.
pub fn platform_cfg() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")
        .expect("CARGO_CFG_TARGET_OS is set by cargo for build scripts");
    println!("cargo:rustc-check-cfg=cfg(desktop)");
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    println!("cargo:rustc-cfg={}", platform(&target_os));
    // The answer depends on the target alone, never on the sources.
    println!("cargo:rerun-if-changed=build.rs");
}

/// The manifest `tauri-build` gives the application (its
/// `windows-app-manifest.xml`): version 6 of the Common Controls. Without it
/// Windows binds version 5 of `comctl32.dll`, which lacks functions the
/// windows and dialogs of Tauri import (`TaskDialogIndirect` among them), and
/// the loader refuses the executable before `main`.
const WINDOWS_TEST_MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#;

/// Whether the linker of the target takes `/MANIFEST:EMBED`: `link.exe` and
/// `lld-link` do, the GNU linker does not.
fn takes_manifest(target_os: &str, target_env: &str) -> bool {
    target_os == "windows" && target_env == "msvc"
}

/// Gives the test executables of a library that links Tauri the application
/// manifest a Tauri application has.
///
/// The application gets its manifest from `tauri-build`, as a resource of its
/// binary. The executable `cargo test` builds from a library has none, and on
/// Windows it does not start: exit code 0xc0000139,
/// STATUS_ENTRYPOINT_NOT_FOUND (see [`WINDOWS_TEST_MANIFEST`]). The build
/// script of the `tauri` crate does the same for its own tests.
///
/// A `cargo:rustc-link-arg` line reaches what the package of the build script
/// links itself (its unit and integration tests, its examples) and never the
/// crates that depend on it: the binary of a product keeps the one manifest of
/// `tauri-build`. A product crate (`apps/*`) must not call this: its binary
/// would get a second manifest, a duplicate resource the linker refuses.
///
/// Every library crate that depends on `tauri` calls it from its `build.rs`
/// (scripts/boundaries.sh checks that):
///
/// ```ignore
/// fn main() { veydan_build_cfg::windows_test_manifest(); }
/// ```
pub fn windows_test_manifest() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")
        .expect("CARGO_CFG_TARGET_OS is set by cargo for build scripts");
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // The answer depends on the target alone, never on the sources.
    println!("cargo:rerun-if-changed=build.rs");
    if !takes_manifest(&target_os, &target_env) {
        return;
    }
    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo for build scripts");
    let manifest = std::path::Path::new(&out_dir).join("windows-test-manifest.xml");
    std::fs::write(&manifest, WINDOWS_TEST_MANIFEST)
        .unwrap_or_else(|e| panic!("cannot write {}: {e}", manifest.display()));
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}

/// The tag of the prebuilt libwebrtc the engine of calls is built with
/// (`webrtc_sys_build::WEBRTC_TAG`; scripts/webrtc-toolchain.sh pins it).
const WEBRTC_TAG: &str = "webrtc-89d790b";

/// The JNI functions of libwebrtc's Java side, as the archive names them.
const WEBRTC_JNI_PREFIX: &str = "Java_livekit_org_webrtc_";

/// The folder of the archive for a target of Android, under the tag:
/// `android-<arch>-release`, with the arch spelt as libwebrtc spells it.
fn webrtc_android_dir(target_arch: &str) -> Option<String> {
    let arch = match target_arch {
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86_64" => "x64",
        "x86" => "x86",
        _ => return None,
    };
    Some(format!("android-{arch}-release"))
}

/// The symbols to keep and export, out of what `llvm-nm` lists: the defined
/// text symbols with the JNI prefix, each once.
fn webrtc_jni_symbols(nm: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in nm.lines() {
        // `<address> T <name>`; an undefined symbol has no address and `U`.
        let mut parts = line.split_whitespace();
        let (Some(_addr), Some(kind), Some(name)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if kind == "T" && name.starts_with(WEBRTC_JNI_PREFIX) && !out.iter().any(|s| s == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Keeps and exports the JNI functions of libwebrtc from a product's library
/// on Android.
///
/// The Java side of libwebrtc (`livekit.org.webrtc.*` of libwebrtc.jar, which
/// tauri-plugin-veydan-call packages) reaches its native side by name:
/// `Java_livekit_org_webrtc_*` of libwebrtc.a, which the engine of calls
/// links. Rust refers to none of them, so the linker leaves them out, and a
/// cdylib exports only what rustc puts in its version script. Without them
/// the first call of the Java side (the factory of encoders, the audio
/// device) throws `UnsatisfiedLinkError` and libwebrtc aborts the process.
/// Two link arguments mend it: `--undefined=` for each symbol, so the
/// archive's objects are linked, and a version script that makes them
/// global. `webrtc-sys` prints them from its own build script, which reaches
/// nothing (see [`windows_test_manifest`]): the product's build script must
/// say them, and a product with the messenger calls this from its `build.rs`.
///
/// Nothing is said off Android, and nothing when no archive is pinned for
/// the target (`LK_CUSTOM_WEBRTC` names it, or `VEYDAN_WEBRTC_DIR` the
/// folder of all of them, as scripts/android/android-env.sh exports): the
/// product is then built without the engine and the symbols would be
/// undefined. The symbols are read with the `llvm-nm` of the NDK
/// (`ANDROID_NDK_HOME` or `NDK_HOME`).
pub fn android_webrtc_jni_exports() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("CARGO_CFG_TARGET_OS is set by cargo for build scripts");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VEYDAN_WEBRTC_DIR");
    println!("cargo:rerun-if-env-changed=LK_CUSTOM_WEBRTC");
    if target_os != "android" {
        return;
    }
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let archive = match std::env::var_os("LK_CUSTOM_WEBRTC") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => {
            let (Some(root), Some(dir)) = (std::env::var_os("VEYDAN_WEBRTC_DIR"), webrtc_android_dir(&target_arch)) else {
                println!("cargo:warning=no libwebrtc for {target_arch}: the Java side of the engine of calls is not exported");
                return;
            };
            std::path::PathBuf::from(root).join(WEBRTC_TAG).join(dir)
        }
    }
    .join("lib")
    .join("libwebrtc.a");
    if !archive.is_file() {
        println!("cargo:warning={} is missing: the Java side of the engine of calls is not exported", archive.display());
        return;
    }
    let ndk = std::env::var_os("ANDROID_NDK_HOME")
        .or_else(|| std::env::var_os("NDK_HOME"))
        .expect("ANDROID_NDK_HOME (or NDK_HOME) names the NDK the engine of calls was built with");
    let host = match std::env::consts::OS {
        "macos" => "darwin-x86_64",
        "windows" => "windows-x86_64",
        _ => "linux-x86_64",
    };
    let nm = std::path::PathBuf::from(ndk).join("toolchains/llvm/prebuilt").join(host).join("bin/llvm-nm");
    let listed = std::process::Command::new(&nm)
        .args(["--defined-only", "--extern-only"])
        .arg(&archive)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", nm.display()));
    let symbols = webrtc_jni_symbols(&String::from_utf8_lossy(&listed.stdout));
    assert!(!symbols.is_empty(), "{} has no {WEBRTC_JNI_PREFIX}* symbols", archive.display());
    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is set by cargo for build scripts");
    let script = std::path::Path::new(&out_dir).join("webrtc-jni.map");
    let body = format!("WEBRTC_JNI {{\n  global:\n    {};\n}};\n", symbols.join(";\n    "));
    std::fs::write(&script, body).unwrap_or_else(|e| panic!("cannot write {}: {e}", script.display()));
    for symbol in &symbols {
        println!("cargo:rustc-link-arg=-Wl,--undefined={symbol}");
    }
    println!("cargo:rustc-link-arg=-Wl,--version-script={}", script.display());
}

#[cfg(test)]
mod tests {
    use super::{platform, takes_manifest, webrtc_android_dir, webrtc_jni_symbols, WINDOWS_TEST_MANIFEST};

    #[test]
    fn the_jni_symbols_are_the_defined_ones_with_the_prefix_once() {
        let nm = "0000000000000000 T Java_livekit_org_webrtc_PeerConnection_nativeClose\n\
                  0000000000000010 T Java_livekit_org_webrtc_PeerConnection_nativeClose\n\
                  0000000000000020 T Java_livekit_J_N_MMv8RAm7\n\
                  0000000000000030 t Java_livekit_org_webrtc_Hidden_native\n\
                  0000000000000040 T JNI_OnLoad\n\
                                   U Java_livekit_org_webrtc_Missing_native\n\
                  0000000000000050 T Java_livekit_org_webrtc_AudioTrack_nativeSetVolume\n";
        assert_eq!(
            webrtc_jni_symbols(nm),
            ["Java_livekit_org_webrtc_PeerConnection_nativeClose", "Java_livekit_org_webrtc_AudioTrack_nativeSetVolume"]
        );
    }

    #[test]
    fn the_archive_folder_follows_the_arch_of_the_target() {
        assert_eq!(webrtc_android_dir("aarch64").as_deref(), Some("android-arm64-release"));
        assert_eq!(webrtc_android_dir("x86_64").as_deref(), Some("android-x64-release"));
        assert_eq!(webrtc_android_dir("riscv64"), None);
    }

    #[test]
    fn phones_are_mobile_and_everything_else_is_desktop() {
        for os in ["android", "ios"] {
            assert_eq!(platform(os), "mobile");
        }
        for os in ["linux", "windows", "macos", "freebsd"] {
            assert_eq!(platform(os), "desktop");
        }
    }

    #[test]
    fn only_the_msvc_linker_is_given_a_manifest() {
        assert!(takes_manifest("windows", "msvc"));
        for (os, env) in [("windows", "gnu"), ("linux", "gnu"), ("macos", ""), ("android", "")] {
            assert!(!takes_manifest(os, env), "{os} {env}");
        }
    }

    #[test]
    fn the_manifest_asks_for_version_6_of_the_common_controls() {
        assert!(WINDOWS_TEST_MANIFEST.contains(r#"name="Microsoft.Windows.Common-Controls""#));
        assert!(WINDOWS_TEST_MANIFEST.contains(r#"version="6.0.0.0""#));
    }
}

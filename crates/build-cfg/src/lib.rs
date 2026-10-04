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

#[cfg(test)]
mod tests {
    use super::{platform, takes_manifest, WINDOWS_TEST_MANIFEST};

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

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The build script of the vendored webrtc-sys fails closed: without
// VEYDAN_WEBRTC_DIR (or LK_CUSTOM_WEBRTC) it refuses to build rather than
// let webrtc-sys-build download libwebrtc with no check of its hash
// (vendor/webrtc-sys/PATCH.md, patch 2). The script is run as cargo runs
// it, from the binary cargo compiled for this very build: a refusal costs
// nothing, the script stops at its first lines.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// The compiled build script of webrtc-sys next to this test's binary
/// (`target/<profile>/build/webrtc-sys-<hash>/build-script-build`); the
/// newest when several builds left one.
fn build_script() -> PathBuf {
    let exe = std::env::current_exe().expect("the test's binary");
    let profile = exe.parent().and_then(Path::parent).expect("target/<profile>/deps/<test>");
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(profile.join("build")).expect("target/<profile>/build") {
        let entry = entry.expect("entry");
        if !entry.file_name().to_string_lossy().starts_with("webrtc-sys-") {
            continue;
        }
        let bin = entry.path().join("build-script-build");
        let Ok(modified) = std::fs::metadata(&bin).and_then(|m| m.modified()) else { continue };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, bin));
        }
    }
    newest.expect("the build script of webrtc-sys, compiled with this crate").1
}

/// Runs the build script for the host as cargo would, with the two
/// variables of the archive unset and `extra` on top: whether it went on,
/// and what it said.
fn run(extra: &[(&str, &str)]) -> (bool, String) {
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/webrtc-sys");
    let out = Command::new(build_script())
        .current_dir(&vendor)
        .env_remove("VEYDAN_WEBRTC_DIR")
        .env_remove("LK_CUSTOM_WEBRTC")
        .env_remove("DOCS_RS")
        .env("CARGO_CFG_TARGET_OS", "linux")
        .env("CARGO_CFG_TARGET_ARCH", "x86_64")
        .env("TARGET", "x86_64-unknown-linux-gnu")
        .envs(extra.iter().copied())
        .output()
        .expect("run the build script");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn without_an_archive_named_the_build_is_refused_not_downloaded() {
    let (ok, said) = run(&[]);
    assert!(!ok, "the build script went on without an archive:\n{said}");
    assert!(said.contains("neither VEYDAN_WEBRTC_DIR nor LK_CUSTOM_WEBRTC"), "{said}");
    assert!(said.contains("scripts/webrtc-toolchain.sh"), "{said}");
}

#[test]
fn a_folder_without_the_archive_of_this_target_is_refused() {
    let empty = tempfile::tempdir().expect("tempdir");
    let (ok, said) = run(&[("VEYDAN_WEBRTC_DIR", empty.path().to_str().expect("utf-8"))]);
    assert!(!ok, "the build script went on with an empty folder:\n{said}");
    assert!(said.contains("libwebrtc for this target is not in"), "{said}");
    assert!(said.contains("linux-x64-release"), "the folder of the host's target is named:\n{said}");
}

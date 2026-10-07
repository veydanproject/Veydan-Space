# webrtc-sys 0.3.48, vendored with four patches

This folder is the crate `webrtc-sys` 0.3.48 as published on crates.io
(livekit/rust-sdks, Apache-2.0: `LICENSE`, `NOTICE.md`), taken by
`[patch.crates-io]` in `crates/messenger/Cargo.toml` and in the root
`Cargo.toml`. `Cargo.toml.orig` and `.cargo_vcs_info.json` of the package are
left out; everything else is the published copy, except the hunks below.
The companion crates `libwebrtc` 0.3.51 and `webrtc-sys-build` 0.3.19 come
from the registry unchanged. The stage-0 spike (tmp/calls-spike/REPORT.md,
section 2) found what the patches fix.

## Why a copy and not a fork or an upstream change

The changes are small, and two of them are about this repository's
build environment, not about the engine. A copy with a diff is read in one
place; moving the crate to a new version is `cp` of the new package and
applying the diff again.

## 1. `build.rs`: the order of include paths (Linux)

The crate `pkg-config` sets `PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1`, so the `.pc`
files of glib in data/dev-prefix (prefix=/usr) yield `-I/usr/include`. A
`-I` is searched before the `-isystem` of the hermetic libc++ of libwebrtc,
so `<math.h>` comes from glibc with its `isfinite` macros and libc++ does not
compile. The patch skips `/usr/include`, which the compiler searches by
itself. Good for upstream as it is.

## 2. `build.rs`: the toolchain of the monorepo

- `VEYDAN_WEBRTC_DIR` (exported by `scripts/webrtc-toolchain.sh`, sourced
  from `build-env.sh` and `android-env.sh`) names the folder with one
  unpacked archive per target, `<tag>/<os>-<arch>-<profile>/`; the one of the
  target being built is chosen in `veydan_configure` and handed to
  `webrtc-sys-build` as `LK_CUSTOM_WEBRTC`. One variable serves a build of
  several Android ABIs. Nothing is downloaded: the script pins the archive by
  sha256, the download of `webrtc-sys-build` checks nothing. A build with
  neither variable is refused (a panic that names the way out), so that the
  pin fails closed: otherwise the upstream `download_webrtc()` would fetch
  and link an unchecked archive on every host that forgot the environment (a
  release runner, a developer's shell). `tests/build_guard.rs` of
  `messenger-rtc` runs the compiled build script that way and expects the
  refusal.
- `VEYDAN_WEBRTC_CLANGXX` names the clang that compiles the C++ bridge
  (clang >= 21 for the hermetic libc++; on Linux the one of the Android NDK
  in data/toolchains, which targets the host too). `CXX` would send every
  C++ crate of the build to it.
- `CFLAGS`/`CXXFLAGS`/`CPPFLAGS` of `build-env.sh` (`-I data/dev-prefix/...`,
  for WebKit) are dropped for this crate when `VEYDAN_WEBRTC_DIR` is set, for
  the reason of patch 1.

## 3. `src/peer_connection.cpp`: `turns:` without a certificate check

A call node (`services/call`) is reached by its bare address and shows a
certificate signed by its own key, not by an authority; libwebrtc's
default `kTlsCertPolicySecure` refuses it and TURN over TLS never connects.
The patch sets `kTlsCertPolicyInsecureNoCheck` on an ICE server that has a
`turns:` URL. Decided 2026-10-07 (the plan of calls, stage 3): the node
was pinned by its id on the control channel the credentials came from, and
the media is under DTLS whatever the TURN transport is, so the TLS of the
relay adds nothing a check would protect. The crate's `IceServer` has no
field for the policy; adding one would also mean patching `libwebrtc`.

## The diff

```diff
diff -ru -x Cargo.toml.orig -x .cargo_vcs_info.json -x LICENSE -x PATCH.md -x .cargo-ok a/build.rs b/build.rs
--- a/build.rs	2006-07-24 09:21:28.000000000 +0800
+++ b/build.rs	2026-10-07 18:34:23.687524119 +0800
@@ -27,6 +27,7 @@
 
     println!("cargo:rerun-if-env-changed=LK_DEBUG_WEBRTC");
     println!("cargo:rerun-if-env-changed=LK_CUSTOM_WEBRTC");
+    veydan_configure(&target_os);
 
     let mut rust_files = vec![
         "src/peer_connection.rs",
@@ -185,6 +186,16 @@
             for lib_name in ["glib-2.0", "gobject-2.0", "gio-2.0"] {
                 let lib = pkg_config::Config::new().cargo_metadata(false).probe(lib_name).unwrap();
                 for path in lib.include_paths {
+                    // Veydan patch (PATCH.md): the pkg-config crate asks for
+                    // system cflags, so a prefix whose .pc files say
+                    // prefix=/usr yields -I/usr/include, and a -I beats the
+                    // -isystem of the hermetic libc++ below: <math.h> then
+                    // comes from glibc with its isfinite macros and libc++
+                    // fails to build. The compiler searches /usr/include on
+                    // its own.
+                    if path == std::path::Path::new("/usr/include") {
+                        continue;
+                    }
                     builder.include(path);
                 }
             }
@@ -534,7 +545,13 @@
     // changes the calling convention, not just layout: libwebrtc.a returns
     // std::unique_ptr in a register, while a GCC caller reads it back from an sret
     // slot the callee never wrote, yielding a garbage pointer at the first use.
-    if env::var_os("CXX").is_none() {
+    // Veydan patch (PATCH.md): the clang of the monorepo's own toolchain,
+    // named for this crate alone, so that the other C and C++ crates of the
+    // build keep the compiler of the system.
+    println!("cargo:rerun-if-env-changed=VEYDAN_WEBRTC_CLANGXX");
+    if let Some(clangxx) = env::var_os("VEYDAN_WEBRTC_CLANGXX") {
+        builder.compiler(clangxx);
+    } else if env::var_os("CXX").is_none() {
         if Command::new("clang++").arg("--version").output().is_err() {
             panic!(
                 "clang++ is required to build webrtc-sys on Linux: libwebrtc.a is built \
@@ -658,3 +675,54 @@
     let glib_path_config = glib_path_config.join("glib-2.0/include");
     builder.include(&glib_path_config);
 }
+
+/// Veydan patch (PATCH.md): where the prebuilt libwebrtc is, and the flags
+/// of the host that must not reach this crate.
+///
+/// `VEYDAN_WEBRTC_DIR` (scripts/webrtc-toolchain.sh) holds one unpacked
+/// archive per target under `<tag>/<os>-<arch>-<profile>/`; the one of the
+/// target being built is chosen here, so that a build of several Android
+/// ABIs needs no variable per target. `LK_CUSTOM_WEBRTC`, set by hand,
+/// still wins. Nothing is downloaded: the archive is pinned by hash by the
+/// script, and an upstream download would check none. So a build with
+/// neither variable is refused here, before webrtc-sys-build could fetch
+/// an unchecked archive (`download_webrtc` below, and again in
+/// `configure_jni_symbols` on Android): the pin fails closed.
+///
+/// `CFLAGS`/`CXXFLAGS` of the monorepo's build environment point at
+/// data/dev-prefix (the headers WebKit is built against). Any `-I` is
+/// searched before the `-isystem` of the hermetic libc++ of libwebrtc, so
+/// glibc's <math.h> would shadow the libc++ wrapper and the bridge would
+/// not compile. They are dropped for this crate only.
+fn veydan_configure(target_os: &str) {
+    println!("cargo:rerun-if-env-changed=VEYDAN_WEBRTC_DIR");
+    if env::var_os("LK_CUSTOM_WEBRTC").is_none() {
+        let Some(root) = env::var_os("VEYDAN_WEBRTC_DIR") else {
+            panic!(
+                "neither VEYDAN_WEBRTC_DIR nor LK_CUSTOM_WEBRTC is set, and webrtc-sys would download \
+                 libwebrtc with no check of its hash. Source scripts/build-env.sh (or \
+                 scripts/android/android-env.sh), or run scripts/webrtc-toolchain.sh <target> and \
+                 export VEYDAN_WEBRTC_DIR; a build without the engine of calls turns off the feature \
+                 `rtc` of messenger-runtime"
+            );
+        };
+        let dir = PathBuf::from(root)
+            .join(webrtc_sys_build::WEBRTC_TAG)
+            .join(webrtc_sys_build::webrtc_triple());
+        if !dir.join("webrtc.ninja").exists() {
+            panic!(
+                "libwebrtc for this target is not in {}: run scripts/webrtc-toolchain.sh <target> \
+                 (linux-x64, android-arm64, ...) from the root of the repository",
+                dir.display()
+            );
+        }
+        // Read by webrtc-sys-build (webrtc_dir, webrtc_defines,
+        // configure_jni_symbols) in this process.
+        env::set_var("LK_CUSTOM_WEBRTC", &dir);
+    }
+    if target_os == "linux" && env::var_os("VEYDAN_WEBRTC_DIR").is_some() {
+        for flags in ["CFLAGS", "CXXFLAGS", "CPPFLAGS"] {
+            env::remove_var(flags);
+        }
+    }
+}
diff -ru -x Cargo.toml.orig -x .cargo_vcs_info.json -x LICENSE -x PATCH.md -x .cargo-ok a/src/peer_connection.cpp b/src/peer_connection.cpp
--- a/src/peer_connection.cpp	2006-07-24 09:21:28.000000000 +0800
+++ b/src/peer_connection.cpp	2026-10-07 17:06:45.717052102 +0800
@@ -18,6 +18,7 @@
 #include "livekit/peer_connection_factory.h"
 
 #include <memory>
+#include <string_view>
 
 #include "api/data_channel_interface.h"
 #include "api/peer_connection_interface.h"
@@ -41,8 +42,19 @@
     ice_server.username = item.username.c_str();
     ice_server.password = item.password.c_str();
 
-    for (auto url : item.urls)
+    for (auto url : item.urls) {
       ice_server.urls.emplace_back(url.c_str());
+      // Veydan patch (PATCH.md): a `turns:` server is a call node reached by
+      // its bare address, and its certificate is signed by the node's own
+      // key, not by an authority. The node was already pinned by its id on
+      // the control channel the credentials came from, and the media stays
+      // under DTLS whatever the TURN transport is; so the TLS of the relay
+      // is accepted without a check of the certificate.
+      if (std::string_view(url.data(), url.size()).rfind("turns:", 0) == 0) {
+        ice_server.tls_cert_policy =
+            webrtc::PeerConnectionInterface::kTlsCertPolicyInsecureNoCheck;
+      }
+    }
 
     rtc_config.servers.push_back(ice_server);
   }
```

## 4. `src/adm_proxy.cpp`: `Init()` brings a terminated device back

libwebrtc (M13x, `pc/connection_context.h`, `MediaEngineReference`) runs
its media engine from the first PeerConnection of a factory to the last:
`WebRtcVoiceEngine::Init()` on the first, `Terminate()` after the last, and
`Init()` again on the next one. `Terminate()` stops the audio device module
and calls `Terminate()` on it; `Init()` calls `Init()` on it. The upstream
`AdmProxy::Terminate()` forwards to its sub ADMs (the synthetic one and the
platform's), but `AdmProxy::Init()` was a no-op ("the sub ADMs are
initialized at creation time"). So the second call of a process met a
terminated device: on the pushed path a synthetic ADM whose pump was gone
(silence), on Android a platform ADM whose `AudioDeviceBuffer` was freed by
its `Terminate()` (`adm_helpers.cc: Unable to access speaker/microphone`,
then SIGSEGV at 0x80 on `worker_thread 0` when the voice engine registered
its audio callback; tmp/calls-android/REPORT-w3.md). The patch makes
`Init()` initialize whichever sub ADM is not initialized, on the worker
thread, as the contract of `AudioDeviceModule` says (`Init()` after
`Terminate()`). `tests/loopback.rs` of `messenger-rtc`
(`a_second_pair_of_the_same_engine_carries_the_sound_again`) runs two pairs
of sessions in a row through one engine. Upstream `main` has the no-op
still (checked 2026-10-07); worth sending there.

```diff
--- a/src/adm_proxy.cpp	2006-07-24 09:21:28.000000000 +0800
+++ b/src/adm_proxy.cpp	2026-10-07 23:21:05.939959839 +0800
@@ -351,8 +351,30 @@
 }
 
 int32_t AdmProxy::Init() {
-  // Init is a no-op - the sub ADMs are initialized at creation time
-  return 0;
+  // Veydan patch (PATCH.md, 4): the sub ADMs are initialized at creation
+  // time, but Terminate() below terminates them, and libwebrtc calls
+  // Terminate() on its voice engine when the last PeerConnection goes and
+  // Init() again on the next one (MediaEngineReference). So Init() brings
+  // back whichever sub ADM is terminated; otherwise the second call of a
+  // process meets a terminated platform device (on Android its audio buffer
+  // is gone with it: "Unable to access speaker", then a null dereference
+  // when the voice engine registers its audio callback).
+  return RunOnWorker([this] {
+    RTC_DCHECK_RUN_ON(worker_thread_);
+    int32_t result = 0;
+    if (synthetic_adm_ && !synthetic_adm_->Initialized()) {
+      result = synthetic_adm_->Init();
+    }
+    if (platform_adm_ && !platform_adm_->Initialized()) {
+      int32_t platform_result = platform_adm_->Init();
+      if (platform_result != 0) {
+        RTC_LOG(LS_ERROR) << "AdmProxy::Init() - Platform ADM Init() failed with error="
+                          << platform_result;
+      }
+      if (result == 0) result = platform_result;
+    }
+    return result;
+  });
 }
 
 int32_t AdmProxy::Terminate() {
```

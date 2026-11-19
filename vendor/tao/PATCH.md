This is `tao` 0.35.3 from crates.io, with two changes on iOS.

1. `src/platform_impl/ios/view.rs`, `configuration_for_connecting_scene_session`:

```diff
-      Retained::as_ptr(&config) as _
+      Retained::autorelease_ptr(config) as _
```

The method handed UIKit a pointer to the `UISceneConfiguration` and dropped
its `Retained`, so the object could be freed before UIKit retained it. Built
with the SDK of iOS 27 the app takes the scene path (`Info.ios.plist` of
Space, `UIApplicationSupportsMultipleScenes`), and the TestFlight build of
Space 5.2.2 (72.2.0) died at launch: `EXC_BAD_ACCESS` in `objc_retain` under
`-[UIApplication _connectUISceneFromFBSScene:transitionContext:]`, on every
start. Whether it crashed depended on how the return value was claimed, so the
lab builds lived and the signed release build did not.

tao 0.36.0 makes the same change upstream.

2. `src/platform_impl/ios/app_state.rs`, `connect_scene`, and
`opened_on_connect` in `src/platform_impl/ios/scene.rs`: the URLs of
`UISceneConnectionOptions.URLContexts` are sent as `Event::Opened`.

On the scene path UIKit hands the URL that started the app (a `veydan://…`
tapped while the app was not running) in the connection options of the
scene, and calls `scene:openURLContexts:` only for URLs that come while the
scene is connected. tao 0.35.3 read the options only to pass them on with
`SceneRequested` for a scene after the first, so the link of a cold start
never became `RunEvent::Opened` and was lost (`crates/shell/src/links.rs`).
The event comes after `on_app_ready`, so Tauri's setup has run by then. A
scene handed on with `SceneRequested` still reads its own options. Upstream
tao (0.36, the `dev` branch as of 2026-11-19) does not read them either.
The property is read as nullable: iOS 27 gives nil, not an empty set, to an
app started from its icon, and objc2's typed getter panics on a nil (Space
5.2.2 rc.8 died at every start).

Tauri 2.11 takes tao 0.35, so the copy stays until Tauri moves to a tao that
has both. Then remove this folder, its line in `[patch.crates-io]` of the
root `Cargo.toml`, `vendor/tao` in `publish.shared` of `products.json` and
its entry in `VENDORED` of `scripts/authors-check.mjs`.

Left out of the package as published: `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`.

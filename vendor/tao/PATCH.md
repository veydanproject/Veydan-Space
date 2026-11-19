This is `tao` 0.35.3 from crates.io, with one change in
`src/platform_impl/ios/view.rs`, `configuration_for_connecting_scene_session`:

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

tao 0.36.0 makes the same change upstream; Tauri 2.11 takes tao 0.35, so the
copy stays until Tauri moves to a tao that has it. Then remove this folder,
its line in `[patch.crates-io]` of the root `Cargo.toml`, `vendor/tao` in
`publish.shared` of `products.json` and its entry in `VENDORED` of
`scripts/authors-check.mjs`.

Left out of the package as published: `.cargo_vcs_info.json`,
`Cargo.toml.orig`, `Cargo.lock`.

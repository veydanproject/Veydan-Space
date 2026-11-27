# The page calls the bridge by name: keep what @JavascriptInterface marks.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# The Rust side takes a link the system handed over by name (src/links.rs).
-keep class net.veydan.shell.IncomingLinks {
    public static java.lang.String take();
}

# The same for what another app shared (src/shares.rs): nothing in Kotlin
# calls take(), and R8 of a release left it out (the Galaxy Z Fold6 of the
# owner, 2026-11-27: the sheet never came; a debug build has no R8).
-keep class net.veydan.shell.IncomingShares {
    public static java.lang.String take();
}

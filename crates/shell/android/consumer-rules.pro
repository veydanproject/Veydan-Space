# The page calls the bridge by name: keep what @JavascriptInterface marks.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# The Rust side takes a link the system handed over by name (src/links.rs).
-keep class net.veydan.shell.IncomingLinks {
    public static java.lang.String take();
}

# The page calls the bridge by name: keep what @JavascriptInterface marks.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

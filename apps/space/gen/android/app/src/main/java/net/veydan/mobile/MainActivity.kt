package net.veydan.mobile

import android.os.Bundle
import android.webkit.WebView
import net.veydan.shell.ActivityDelegate

// What the activity does is the shell's (crates/shell/android, docs/platform-spec.md 13.4).
class MainActivity : TauriActivity() {
  private val delegate = ActivityDelegate(this)

  override fun onCreate(savedInstanceState: Bundle?) {
    delegate.onCreate()
    super.onCreate(savedInstanceState)
  }

  override fun onWebViewCreate(webView: WebView) {
    delegate.onWebViewCreate(webView)
  }
}

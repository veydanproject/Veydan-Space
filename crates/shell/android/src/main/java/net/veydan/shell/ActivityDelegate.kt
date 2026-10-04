// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.graphics.Color
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.ScriptHandler
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature

/**
 * What the MainActivity of every product does (docs/platform-spec.md 13.4):
 * edge-to-edge drawing, the insets of the system bars and the keyboard
 * handed to the page as CSS variables, and the bridge `VeydanChrome` the
 * page sets the look of the bars through. A base class is impossible —
 * `TauriActivity` is generated into the package of each product —, so the
 * activity passes `onCreate` and `onWebViewCreate` here.
 */
class ActivityDelegate(private val activity: ComponentActivity) {
  private var lastBars: Insets = Insets.NONE
  private var lastImeBottom: Int = 0
  private var insetsScriptHandler: ScriptHandler? = null

  /** Before `super.onCreate`: light icons and scrims until the page reports its theme. */
  fun onCreate() {
    activity.enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
    )
  }

  fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(ChromeBridge(), "VeydanChrome")

    ViewCompat.setOnApplyWindowInsetsListener(webView) { v, windowInsets ->
      lastBars = windowInsets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      lastImeBottom = windowInsets.getInsets(WindowInsetsCompat.Type.ime()).bottom
      injectInsets(v as WebView, lastBars, lastImeBottom)
      windowInsets
    }
    ViewCompat.requestApplyInsets(webView)
  }

  private fun injectInsets(webView: WebView, bars: Insets, imeBottom: Int) {
    val d = webView.resources.displayMetrics.density
    val top = bars.top / d
    val right = bars.right / d
    val bottom = bars.bottom / d
    val left = bars.left / d
    val kb = imeBottom / d
    val script =
      """
      (function(){
        var s=document.documentElement.style;
        s.setProperty('--sat','${top}px');
        s.setProperty('--sar','${right}px');
        s.setProperty('--sab','${bottom}px');
        s.setProperty('--sal','${left}px');
        s.setProperty('--kb','${kb}px');
        window.dispatchEvent(new Event('veydan-keyboard'));
      })();
      """.trimIndent()

    if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
      insetsScriptHandler?.remove()
      insetsScriptHandler = WebViewCompat.addDocumentStartJavaScript(webView, script, setOf("*"))
    }
    webView.evaluateJavascript(script, null)
  }

  private fun applyLightBars(light: Boolean) {
    val window = activity.window
    val controller = WindowCompat.getInsetsController(window, window.decorView)
    controller.isAppearanceLightStatusBars = light
    controller.isAppearanceLightNavigationBars = light
  }

  inner class ChromeBridge {
    @JavascriptInterface
    fun setLightBars(light: Boolean) {
      activity.runOnUiThread { applyLightBars(light) }
    }
  }
}

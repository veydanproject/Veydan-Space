// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.content.ClipboardManager
import android.graphics.Color
import android.graphics.Rect
import android.net.Uri
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
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray

/**
 * What the MainActivity of every product does (internal/platform-spec.md 13.4):
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
  private var webView: WebView? = null

  /** Whether the page takes pictures a keyboard inserts (the composer of a chat is focused): set through the bridge. */
  @Volatile private var takesPictures: Boolean = false

  /** Before `super.onCreate`: light icons and scrims until the page reports its theme. */
  fun onCreate() {
    activity.enableEdgeToEdge(
      statusBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
      navigationBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
    )
  }

  fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(ChromeBridge(), "VeydanChrome")
    receivePictures(webView)

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

  /**
   * Pictures a keyboard inserts (Gboard's clipboard, stickers, GIFs) or a
   * paste hands over: while the page takes them (`setTakesPictures`), their
   * `content://` URIs go to the page as the event `veydan-received-content`
   * and the rest of the payload (text) goes on as before; otherwise the
   * WebView handles all of it itself (the notes take pasted pictures their
   * own way). The URIs stay readable: the keyboard's grant lasts until it is
   * released, which nothing here does.
   */
  private fun receivePictures(webView: WebView) {
    ViewCompat.setOnReceiveContentListener(webView, arrayOf("image/*")) { view, payload ->
      // A keyboard's commit comes on the thread of the input connection, not the UI's: the
      // WebView is read and told only on the UI thread. `takesPictures` was set by its own page.
      if (!takesPictures) return@setOnReceiveContentListener payload
      val split = payload.partition { it.uri != null }
      val taken = split.first ?: return@setOnReceiveContentListener split.second
      val uris = JSONArray()
      val clip = taken.clip
      for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { uris.put(it.toString()) }
      val web = view as WebView
      if (uris.length() > 0) {
        web.post {
          if (ownPage(web)) {
            web.evaluateJavascript("window.dispatchEvent(new CustomEvent('veydan-received-content',{detail:$uris}))", null)
          }
        }
      }
      split.second
    }
  }

  /** The URI of the picture on the clipboard, if it holds one. Reading the clip itself makes Android 12+ say so in a toast: only on the user's tap. */
  private fun clipboardPicture(): Uri? {
    val cm = activity.getSystemService(ClipboardManager::class.java) ?: return null
    if (cm.primaryClipDescription?.hasMimeType("image/*") != true) return null
    val clip = cm.primaryClip ?: return null
    for (i in 0 until clip.itemCount) clip.getItemAt(i).uri?.let { return it }
    return null
  }

  /** `block` on the UI thread (the WebView is read only there), for a bridge call that answers; `fallback` after half a second. */
  private fun <T> onUi(fallback: T, block: (WebView) -> T): T {
    val out = AtomicReference(fallback)
    val done = CountDownLatch(1)
    activity.runOnUiThread {
      try {
        webView?.let { out.set(block(it)) }
      } finally {
        done.countDown()
      }
    }
    return if (done.await(500, TimeUnit.MILLISECONDS)) out.get() else fallback
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
      insetsScriptHandler = WebViewCompat.addDocumentStartJavaScript(webView, script, OWN_ORIGINS)
    }
    webView.evaluateJavascript(script, null)
  }

  private fun applyLightBars(light: Boolean) {
    val window = activity.window
    val controller = WindowCompat.getInsetsController(window, window.decorView)
    controller.isAppearanceLightStatusBars = light
    controller.isAppearanceLightNavigationBars = light
  }

  /**
   * The page in the window is the app's own: the bridge is in every frame
   * of whatever the WebView shows, and only the app may set the bars and
   * take the edge of the screen from "back". Read on the UI thread.
   */
  private fun ownPage(view: WebView): Boolean {
    val uri = view.url?.let(Uri::parse) ?: return false
    return (uri.scheme == "http" || uri.scheme == "https") && uri.host == HOST && uri.port == -1
  }

  inner class ChromeBridge {
    /** Whether the clipboard holds a picture: only its description is read, which Android does not announce. */
    @JavascriptInterface
    fun clipboardHasImage(): Boolean = onUi(false) { view ->
      ownPage(view) &&
        activity.getSystemService(ClipboardManager::class.java)?.primaryClipDescription?.hasMimeType("image/*") == true
    }

    /** The `content://` URI of the picture on the clipboard, read when the user asks for it; null without one. */
    @JavascriptInterface
    fun clipboardImage(): String? = onUi(null) { view -> if (ownPage(view)) clipboardPicture()?.toString() else null }

    /** Whether pictures a keyboard inserts go to the page (`veydan-received-content`): the composer of a chat, while it is focused. */
    @JavascriptInterface
    fun setTakesPictures(takes: Boolean) {
      takesPictures = takes && onUi(false) { view -> ownPage(view) }
    }

    @JavascriptInterface
    fun setLightBars(light: Boolean) {
      activity.runOnUiThread {
        val view = webView ?: return@runOnUiThread
        if (ownPage(view)) applyLightBars(light)
      }
    }

    /**
     * Where the page handles a drag that starts at the edge of the screen
     * itself (CSS pixels of the page); the system does not take it for
     * "back" there. An empty rectangle gives the edge back.
     */
    @JavascriptInterface
    fun setGestureExclusion(left: Double, top: Double, width: Double, height: Double) {
      activity.runOnUiThread {
        val view = webView ?: return@runOnUiThread
        if (!ownPage(view)) return@runOnUiThread
        val d = view.resources.displayMetrics.density
        val rects = if (width <= 0 || height <= 0) emptyList() else listOf(
          Rect((left * d).toInt(), (top * d).toInt(), ((left + width) * d).toInt(), ((top + height) * d).toInt()),
        )
        ViewCompat.setSystemGestureExclusionRects(view, rects)
      }
    }
  }

  private companion object {
    /** Where Tauri serves the pages on Android, a dev build too (it proxies the dev server). */
    const val HOST = "tauri.localhost"
    val OWN_ORIGINS = setOf("http://$HOST", "https://$HOST")
  }
}

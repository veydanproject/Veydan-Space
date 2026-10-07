// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.content.Context
import android.util.Log

/**
 * The media engine's one need of the Java side: the JVM and the
 * application context, handed over on a thread that came from Java before
 * the first engine of the process (`messenger_rtc::android` in the app's
 * Rust library, through `Java_net_veydan_call_Engine_init` of this plugin's
 * Rust side). The plugin's load, on the main thread, is such a place.
 *
 * The engine's Java classes (`livekit.org.webrtc.*`, libwebrtc.jar) are in
 * this library; the native side is in the app's library, which the app's
 * activity loaded before any plugin. A product without the engine has no
 * such native function: that is "no engine", not an error.
 */
internal object Engine {
  @Volatile
  private var ready: Boolean? = null

  /** Whether the engine may be made in this process. Idempotent. */
  fun start(context: Context): Boolean {
    ready?.let { return it }
    synchronized(this) {
      ready?.let { return it }
      val ok = try {
        init(context.applicationContext)
      } catch (e: UnsatisfiedLinkError) {
        Log.i(CallState.TAG, "no media engine in this build")
        false
      } catch (e: Throwable) {
        Log.w(CallState.TAG, "the media engine did not take the context: ${e.javaClass.simpleName}: ${e.message}")
        false
      }
      Log.i(CallState.TAG, "media engine ${if (ok) "ready" else "not ready"}")
      ready = ok
      return ok
    }
  }

  /** `Java_net_veydan_call_Engine_init` of the plugin's Rust side. */
  @JvmStatic
  private external fun init(context: Context): Boolean
}

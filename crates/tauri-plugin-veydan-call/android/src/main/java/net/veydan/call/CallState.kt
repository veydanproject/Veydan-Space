// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.util.Log
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import java.lang.ref.WeakReference

/** What the user pressed: `answer`, `decline`, `hangup`. */
internal data class Press(val callId: String, val action: String) {
  fun toJson(): JSObject = JSObject().put("callId", callId).put("action", action)
}

/**
 * What the plugin, the service, the receiver and the screen of a ringing
 * call share while the process lives.
 *
 * A press can come before the plugin exists: the process was dead, and the
 * press on the notification started it. Such a press waits here until the
 * app's Rust code registers its listener and asks with `takeActions`. A press
 * while a listener is in place goes to it at once.
 */
internal object CallState {
  const val TAG = "VeydanCall"

  const val EVENT_ACTION = "call_action"
  const val EVENT_ROUTE = "audio_route_changed"

  /** The presses nobody heard; a few at most, the oldest go first. */
  private const val MAX_WAITING = 8
  private val waiting = ArrayDeque<Press>()

  private var plugin: WeakReference<VeydanCallPlugin>? = null

  @Synchronized
  fun attach(plugin: VeydanCallPlugin) {
    this.plugin = WeakReference(plugin)
  }

  @Synchronized
  fun plugin(): VeydanCallPlugin? = plugin?.get()

  /** A press: to the listener when there is one, kept for later otherwise. */
  fun pressed(press: Press) {
    Log.i(TAG, "pressed ${press.action} on ${press.callId}")
    val heard = synchronized(this) {
      val p = plugin?.get()
      if (p != null && p.hasListener(EVENT_ACTION)) {
        p
      } else {
        waiting.addLast(press)
        while (waiting.size > MAX_WAITING) waiting.removeFirst()
        null
      }
    }
    heard?.trigger(EVENT_ACTION, press.toJson())
  }

  /** The presses kept so far, oldest first, once. */
  @Synchronized
  fun takeWaiting(): JSArray {
    val out = JSArray()
    for (press in waiting) out.put(press.toJson())
    waiting.clear()
    return out
  }

  /** The routes of the sound changed; nothing is kept, the app asks when it starts. */
  fun routesChanged(routes: JSObject) {
    plugin()?.trigger(EVENT_ROUTE, routes)
  }
}

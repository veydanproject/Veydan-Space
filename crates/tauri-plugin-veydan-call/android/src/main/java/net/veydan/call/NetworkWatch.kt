// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.util.Log

/**
 * The phone's word that its network changed: Wi-Fi gone and the mobile
 * data taking over, or the other way round, or another Wi-Fi. The media
 * engine notices a lost way only after seconds of silence, and a relay
 * socket on the old network may look open for much longer; the app, told
 * at once, restarts the call's way and makes the relay connections anew.
 *
 * The system's default network is watched. The first network seen after
 * the start is the one the app already uses: no word for it. Every
 * network that becomes the default after another one, or after a loss, is
 * a change. The callback comes on a thread of the system; the word goes
 * to the app's listener as it is (`CallState.networkChanged`).
 */
internal object NetworkWatch {
  const val EVENT = "network_changed"

  private var manager: ConnectivityManager? = null
  private var callback: ConnectivityManager.NetworkCallback? = null

  /** The default network last seen, or none after a loss. */
  @Volatile
  private var current: Network? = null
  @Volatile
  private var seenOne = false

  /** Starts watching; idempotent. Without the permission or the service, nothing is watched. */
  fun start(context: Context) {
    if (callback != null) return
    val cm = context.applicationContext.getSystemService(ConnectivityManager::class.java) ?: return
    val cb = object : ConnectivityManager.NetworkCallback() {
      override fun onAvailable(network: Network) {
        val before = current
        current = network
        if (!seenOne) {
          seenOne = true
          Log.i(CallState.TAG, "network: $network is the first")
          return
        }
        if (before == network) return
        Log.i(CallState.TAG, "network changed: ${before ?: "none"} -> $network")
        CallState.networkChanged(if (before == null) "back" else "other")
      }

      override fun onLost(network: Network) {
        if (current == network) {
          current = null
          Log.i(CallState.TAG, "network lost: $network")
        }
      }
    }
    try {
      cm.registerDefaultNetworkCallback(cb)
      manager = cm
      callback = cb
    } catch (e: Exception) {
      Log.w(CallState.TAG, "the network cannot be watched: ${e.javaClass.simpleName}: ${e.message}")
    }
  }

  fun stop() {
    val cb = callback ?: return
    try {
      manager?.unregisterNetworkCallback(cb)
    } catch (_: Exception) {
    }
    callback = null
    manager = null
  }
}

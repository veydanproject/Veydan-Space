// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.content.Context
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import java.io.File
import java.lang.ref.WeakReference

/**
 * What the push service and the plugin share while the process lives.
 *
 * All of it starts empty in a process the push service started by itself:
 * nobody is looking, nothing is live, so every push is shown.
 */
internal object PushState {
  const val TAG = "VeydanPush"

  /** The app is on the screen. Set by the plugin, which exists only with the app's window. */
  @Volatile
  var visible: Boolean = false
    private set

  /** The app receives messages by itself right now. Told by the app. */
  @Volatile
  var live: Boolean = false

  private const val MUTED_FILE = "push/muted"

  /**
   * The app wants no push shown: the part of it the pushes are for is
   * switched off. Kept in a file, because a process the push service starts
   * knows nothing else of the app; it holds whatever the push server still
   * sends until it forgets the phone.
   */
  fun muted(context: Context): Boolean = File(context.noBackupFilesDir, MUTED_FILE).exists()

  fun setMuted(context: Context, muted: Boolean) {
    val file = File(context.noBackupFilesDir, MUTED_FILE)
    if (muted) {
      file.parentFile?.mkdirs()
      if (!file.exists() && !file.createNewFile()) {
        throw IllegalStateException("the mute could not be kept")
      }
    } else if (file.exists() && !file.delete()) {
      throw IllegalStateException("the mute could not be lifted")
    }
  }

  /** The tap nobody has asked about yet. */
  private var tap: Tap? = null

  private var plugin: WeakReference<VeydanPushPlugin>? = null

  val screen = object : DefaultLifecycleObserver {
    override fun onStart(owner: LifecycleOwner) {
      visible = true
    }

    override fun onStop(owner: LifecycleOwner) {
      visible = false
    }
  }

  @Synchronized
  fun attach(plugin: VeydanPushPlugin) {
    this.plugin = WeakReference(plugin)
  }

  @Synchronized
  fun plugin(): VeydanPushPlugin? = plugin?.get()

  @Synchronized
  fun putTap(tap: Tap) {
    this.tap = tap
  }

  @Synchronized
  fun takeTap(): Tap? = tap.also { tap = null }
}

internal data class Tap(val type: String, val chat: String?)

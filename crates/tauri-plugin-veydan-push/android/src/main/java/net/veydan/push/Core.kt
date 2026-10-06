// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.content.Context
import android.util.Log
import org.json.JSONObject

/**
 * The messenger's own reading of a push: the app's Rust library, loaded
 * without the app. It opens the event with the keys and reads the
 * messenger's database, and says what the notification is about.
 *
 * The library is the product's: its name is the string resource
 * `veydan_core_library`, which build.gradle.kts reads from the `[lib]`
 * name of the product's Cargo.toml (internal/platform-spec.md 13.4).
 *
 * Loading the library starts nothing: its entry points wait for a window.
 * A build without the messenger has no such function at all, and that is
 * an error of linking, not an exception; both come back as null here.
 */
internal object Core {
  @Volatile
  private var loaded: Boolean? = null

  private fun load(context: Context): Boolean {
    loaded?.let { return it }
    synchronized(this) {
      loaded?.let { return it }
      val name = context.getString(R.string.veydan_core_library)
      val ok = try {
        System.loadLibrary(name)
        true
      } catch (e: Throwable) {
        Log.w(PushState.TAG, "the app's library $name did not load: ${e.javaClass.simpleName}")
        false
      }
      loaded = ok
      return ok
    }
  }

  /**
   * The outcome as JSON (`outcome`: `show` | `plain` | `quiet`), or null
   * when the library could not answer. `dataDir` is the messenger's
   * directory; `bundle` is the opened key bundle; `data` is the push's
   * data map as a JSON object.
   */
  fun describe(context: Context, bundle: ByteArray, data: Map<String, String>): JSONObject? {
    if (!load(context)) return null
    val dataDir = context.dataDir.resolve("data").resolve("messenger").absolutePath
    val push = JSONObject(data as Map<*, *>).toString()
    val answer = try {
      describe(dataDir, bundle, push)
    } catch (e: Throwable) {
      Log.w(PushState.TAG, "the core did not answer: ${e.javaClass.simpleName}")
      null
    } ?: return null
    return try {
      JSONObject(answer)
    } catch (e: Exception) {
      Log.w(PushState.TAG, "the core's answer is not JSON")
      null
    }
  }

  @JvmStatic
  private external fun describe(dataDir: String, bundle: ByteArray, push: String): String?
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.util.Log
import org.json.JSONObject

/**
 * The keeper of the channels this phone keeps (the module Communities,
 * crates/communities): what gives it a turn while the app is off the
 * screen. A push whose wrap carries the mark `channel` on the outside
 * (`pushtags::CHANNEL` of the messenger, the note `channel.wake` a
 * newcomer sends with a join) is shown to nobody; the periodic wake
 * (`KeeperJob`) does the same every 15 minutes. Both reach the app's Rust
 * code as the event `keeper_wake`, when the app runs.
 *
 * The keeper runs in the app's Rust code, which starts with the app's
 * window: a process the push service started by itself has no keeper to
 * wake, and the keeper takes its turn at the next start of the app.
 */
internal object Keeper {
  const val TAG = "VeydanKeeper"

  /** The event the app's Rust code listens to. */
  const val EVENT = "keeper_wake"

  /** The mark on the outside of the wrap: a tag of a push the module Communities owns. */
  private const val MARK = "channel"

  /** The push is the keeper's: a wrap (as a direct message) marked `channel` on the outside. */
  fun isWake(push: Push): Boolean {
    if (push.type != Push.TYPE_DM) return false
    val event = push.event ?: return false
    return try {
      val tags = JSONObject(event).optJSONArray("tags") ?: return false
      (0 until tags.length()).any { tags.optJSONArray(it)?.optString(0) == MARK }
    } catch (e: Exception) {
      false
    }
  }

  /** Gives the keeper a turn; what became of it, for the log. */
  fun wake(why: String): String {
    val plugin = PushState.plugin() ?: return "the app is not running: its keeper turns at the next start"
    return try {
      plugin.keeperWake(why)
      "the keeper takes a turn"
    } catch (e: Exception) {
      Log.w(TAG, "the keeper was not told: ${e.javaClass.simpleName}")
      "the keeper was not told"
    }
  }
}

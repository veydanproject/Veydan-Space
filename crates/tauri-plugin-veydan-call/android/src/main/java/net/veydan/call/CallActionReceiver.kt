// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import android.util.Log

/**
 * Decline and Hang up on a notification: they open nothing, so they come
 * here and not to a window. The process may have been dead; the press then
 * waits in `CallState` for the app.
 *
 * `RING` comes from the push handler of the app (the push plugin), in the
 * same process, when a push carried an invitation to a call and the app
 * is not up to ring for it: an explicit intent with the call
 * (`CallInfo.into`, `expiresAt` as a wall-clock time in milliseconds, see
 * `ringFromPush`). The receiver is not exported: nobody outside the app
 * can make the phone ring.
 */
class CallActionReceiver : BroadcastReceiver() {
  companion object {
    const val DECLINE = "net.veydan.call.DECLINE"
    const val HANGUP = "net.veydan.call.HANGUP"
    const val RING = "net.veydan.call.RING"
    /** With `RING`: when the invitation is no longer good, unix milliseconds. */
    const val EXTRA_EXPIRES_AT_WALL = "veydan_call_expires_at_wall"

    internal fun intent(context: Context, callId: String, action: String): PendingIntent {
      val intent = Intent(context, CallActionReceiver::class.java)
        .setAction(action)
        .setPackage(context.packageName)
        .putExtra(CallInfo.EXTRA_CALL_ID, callId)
      return PendingIntent.getBroadcast(
        context,
        action.hashCode(),
        intent,
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
    }
  }

  override fun onReceive(context: Context, intent: Intent) {
    val callId = intent.getStringExtra(CallInfo.EXTRA_CALL_ID)?.take(128) ?: return
    when (intent.action) {
      DECLINE -> Calls.declined(context, callId)
      HANGUP -> Calls.hungUp(context, callId)
      RING -> ringFromPush(context, intent)
    }
  }

  /**
   * A push said somebody calls. The wall-clock expiry of the invitation
   * becomes a point of the monotonic clock the ringing is limited by; one
   * already past rings for nothing and ends at once. A call that ended
   * here already, or a phone busy with another call, rings nothing
   * (`Calls.ringFromPush`).
   */
  private fun ringFromPush(context: Context, intent: Intent) {
    val info = CallInfo.from(intent) ?: return
    val wall = intent.getLongExtra(EXTRA_EXPIRES_AT_WALL, 0)
    val expiresAt = if (wall > 0) SystemClock.elapsedRealtime() + (wall - System.currentTimeMillis()) else 0
    Log.i(CallState.TAG, "a push rings ${info.callId}")
    Calls.ringFromPush(context, info.copy(expiresAt = expiresAt))
  }
}

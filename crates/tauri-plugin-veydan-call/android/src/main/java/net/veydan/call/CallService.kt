// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.Notification
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * The foreground service of a call: while it runs the system keeps the
 * process, its microphone and its camera, also with the app's window gone.
 * It does nothing by itself; it shows the notification `Calls` gives it,
 * with the types of the phase the call is in.
 *
 * Ringing, the type is `phoneCall` alone, which needs no permission the user
 * gives: the app has `MANAGE_OWN_CALLS`. The microphone and the camera are
 * added when the call goes on; Android lets a service take them only while
 * the app is in front, and an answer is given in front.
 *
 * Android 12 and later refuse to start the service from the background
 * (outside a high-priority push and a few other cases). The notification is
 * then shown without it; see `Calls`.
 *
 * Its life (a start on its way, running, ending) is kept by `ServiceLife`:
 * a start on its way is never stopped from outside, and an instance told
 * to end never takes a new notification.
 */
class CallService : Service() {
  companion object {
    /** What to show: the notification, the types of the service, the same notification without the call style. */
    private class Wanted(val notification: Notification, val types: Int, val plain: () -> Notification)

    private var wanted: Wanted? = null
    private val life = ServiceLife<CallService>()

    /**
     * Shows `notification` as the service's, starting the service when
     * needed. False when the system refused. True also for a start on its
     * way: the service shows the latest wanted notification when it comes,
     * or posts it by itself if the system then refuses it in front.
     */
    internal fun show(context: Context, notification: Notification, types: Int, plain: () -> Notification): Boolean {
      wanted = Wanted(notification, types, plain)
      return when (life.show()) {
        ServiceLife.Show.REUSE -> life.live()?.foreground() ?: false
        ServiceLife.Show.WAIT -> true
        ServiceLife.Show.START -> try {
          ContextCompat.startForegroundService(context, Intent(context, CallService::class.java))
          life.startAsked()
          true
        } catch (e: Exception) {
          // ForegroundServiceStartNotAllowedException and its kin.
          Log.w(CallState.TAG, "the call service may not start now: ${e.javaClass.simpleName}")
          wanted = null
          false
        }
      }
    }

    /**
     * Takes the service's notification away and ends the service. A start
     * still on its way is left to come: stopping it before it is in front
     * would make the system kill the app. It finds nothing wanted, shows a
     * placeholder for a moment and ends itself.
     */
    internal fun stop() {
      wanted = null
      when (life.stop()) {
        ServiceLife.Stop.END -> life.live()?.end()
        ServiceLife.Stop.LET_COME, ServiceLife.Stop.NOTHING -> Unit
      }
    }
  }

  override fun onCreate() {
    super.onCreate()
    life.created(this)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    life.commanded()
    if (!foreground()) {
      // Nothing to show any more, or the system refused: the service goes,
      // and the notification is shown without it.
      val left = wanted
      end()
      left?.let { CallNotices.post(this, it.notification, it.plain) }
    }
    return START_NOT_STICKY
  }

  /** Puts the wanted notification in front. False when there is none or the system refused it. */
  private fun foreground(): Boolean {
    val (notification, types) = wanted?.let { it.notification to it.types } ?: run {
      // A start asked for and then taken back: the system still wants to
      // see the service in front once, or it kills the app.
      placeholder()
      return false
    }
    return try {
      ServiceCompat.startForeground(this, CallNotices.ID, notification, types)
      true
    } catch (e: Exception) {
      Log.w(CallState.TAG, "the call service was refused (types $types): ${e.javaClass.simpleName}: ${e.message}")
      val phoneCall = phoneCallType()
      if (types != phoneCall) {
        // The microphone or the camera were refused: the call at least.
        try {
          ServiceCompat.startForeground(this, CallNotices.ID, notification, phoneCall)
          return true
        } catch (again: Exception) {
          Log.w(CallState.TAG, "the call service was refused again: ${again.javaClass.simpleName}")
        }
      }
      false
    }
  }

  private fun placeholder() {
    try {
      val notification = NotificationCompat.Builder(this, CallNotices.CHANNEL_ONGOING)
        .setSmallIcon(R.drawable.ic_stat_call)
        .setSilent(true)
        .build()
      ServiceCompat.startForeground(this, CallNotices.ID, notification, phoneCallType())
    } catch (e: Exception) {
      // Refused as well: nothing to undo.
    }
  }

  private fun phoneCallType(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL else 0

  /** Ends this instance; from now on it takes no notification, and a new show asks for a new start. */
  private fun end() {
    life.ending(this)
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    stopSelf()
  }

  override fun onDestroy() {
    life.destroyed(this)
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.app.Notification
import android.app.PendingIntent
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
import java.lang.ref.WeakReference

/**
 * The foreground service of the keeper of the channels, as the call's
 * (`CallService` of the call plugin): while it runs the system keeps the
 * process, so the keeper in the app's Rust code goes on letting readers in
 * with the app off the screen. It does nothing by itself; it shows the
 * notification «Хранитель канала работает». The app starts it while joins
 * stream in or an epoch waits, or while the owner keeps it on, and stops
 * it when the queue was empty for two minutes (crates/communities,
 * `background.rs`).
 *
 * The type is `dataSync`: the keeper hands keys out over the network. No
 * wake lock is taken beyond the service. Android 12 and later refuse to
 * start it from the background outside a high-priority push and a few
 * other cases; the start then answers false and the app asks again later.
 * Android 15 ends a data service after six hours a day (`onTimeout`).
 *
 * A start on its way is never stopped from outside: stopping it before it
 * is in front would make the system kill the app. It comes, shows the
 * notification for a moment, and ends itself.
 */
class KeeperService : Service() {
  companion object {
    /** The id of its notification. */
    private const val ID = 0x4b454550

    private enum class Life { IDLE, STARTING, RUNNING }

    private val lock = Any()
    private var life = Life.IDLE
    /** The service is wanted; a start on its way ends itself without it. */
    private var wanted = false
    private var live: WeakReference<KeeperService>? = null

    /** Starts the service; false when the system refused it now. */
    internal fun start(context: Context): Boolean {
      synchronized(lock) {
        wanted = true
        if (life != Life.IDLE) return true
        life = Life.STARTING
      }
      return try {
        ContextCompat.startForegroundService(context, Intent(context, KeeperService::class.java))
        true
      } catch (e: Exception) {
        // ForegroundServiceStartNotAllowedException and its kin.
        Log.w(Keeper.TAG, "the keeper's service may not start now: ${e.javaClass.simpleName}")
        synchronized(lock) {
          life = Life.IDLE
          wanted = false
        }
        false
      }
    }

    /** Ends the service; a start on its way is let come and ends itself. */
    internal fun stop() {
      val running = synchronized(lock) {
        wanted = false
        if (life == Life.RUNNING) live?.get() else null
      }
      running?.end()
    }
  }

  override fun onCreate() {
    super.onCreate()
    synchronized(lock) { live = WeakReference(this) }
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val shown = try {
      Notifier.ensureChannels(this)
      ServiceCompat.startForeground(this, ID, notification(), dataSyncType())
      true
    } catch (e: Exception) {
      Log.w(Keeper.TAG, "the keeper's service was refused: ${e.javaClass.simpleName}: ${e.message}")
      false
    }
    val keep = synchronized(lock) {
      if (shown && wanted) {
        life = Life.RUNNING
        true
      } else {
        wanted = false
        false
      }
    }
    if (!keep) end()
    return START_NOT_STICKY
  }

  private fun notification(): Notification {
    val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
      PendingIntent.getActivity(this, ID, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    return NotificationCompat.Builder(this, Notifier.CHANNEL_KEEPER)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setContentTitle(getString(R.string.veydan_push_keeper_title))
      .setContentText(getString(R.string.veydan_push_keeper_body))
      .setContentIntent(open)
      .setOngoing(true)
      .setSilent(true)
      .setShowWhen(false)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .build()
  }

  private fun dataSyncType(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0

  /** The day's time of a data service ran out (Android 15): it ends, the app asks again tomorrow. */
  override fun onTimeout(startId: Int, fgsType: Int) {
    Log.i(Keeper.TAG, "the keeper's service ran out of its time")
    synchronized(lock) { wanted = false }
    end()
  }

  /** Ends this instance; a new start asks the system again. */
  private fun end() {
    synchronized(lock) { life = Life.IDLE }
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    stopSelf()
  }

  override fun onDestroy() {
    synchronized(lock) {
      if (live?.get() === this) live = null
      life = Life.IDLE
    }
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log

/**
 * The periodic wake of the keeper of the channels: every 15 minutes, the
 * least the system allows, with a network, it gives the keeper a turn
 * (`Keeper.wake`), the net under the push. The app registers it while a
 * channel is kept on this phone and cancels it when the module stops.
 *
 * The system's `JobScheduler`, which needs no library: the plugin has no
 * WorkManager. While the job runs the system holds the processor and lets
 * the app on the network, also in Doze; the job ends after `WINDOW`, the
 * time a turn of the keeper takes, or at once when the app is not
 * running. Not kept across a restart of the phone: the app registers it
 * again at its next start.
 */
class KeeperJob : JobService() {
  companion object {
    private const val ID = 0x4b454551
    private const val PERIOD_MS = 15 * 60 * 1000L
    /** How long a woken keeper is given the network. */
    private const val WINDOW_MS = 30 * 1000L

    /** Registers (`on`) or cancels the periodic wake. */
    internal fun schedule(context: Context, on: Boolean) {
      val scheduler = context.getSystemService(JobScheduler::class.java) ?: return
      if (!on) {
        scheduler.cancel(ID)
        return
      }
      if (scheduler.getPendingJob(ID) != null) return
      val job = JobInfo.Builder(ID, ComponentName(context, KeeperJob::class.java))
        .setPeriodic(PERIOD_MS)
        .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
        .build()
      if (scheduler.schedule(job) != JobScheduler.RESULT_SUCCESS) {
        Log.w(Keeper.TAG, "the keeper's periodic wake was refused")
      }
    }
  }

  private val handler = Handler(Looper.getMainLooper())

  override fun onStartJob(params: JobParameters): Boolean {
    val running = PushState.plugin() != null
    Log.i(Keeper.TAG, "periodic wake: ${Keeper.wake("periodic")}")
    if (!running) return false
    handler.postDelayed({ jobFinished(params, false) }, WINDOW_MS)
    return true
  }

  override fun onStopJob(params: JobParameters): Boolean {
    handler.removeCallbacksAndMessages(null)
    return false
  }
}

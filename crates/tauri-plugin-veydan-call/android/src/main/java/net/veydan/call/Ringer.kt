// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.Ringtone
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.provider.Settings
import android.util.Log

/**
 * The ringtone and the vibration of a ringing call.
 *
 * The notification channel of calls is silent: the phone's own ringtone is
 * played here, so it starts and stops with the call and not with the
 * notification, and it is the one the user chose for phone calls.
 *
 * The phone's mode decides: silent rings not at all, vibrate only vibrates.
 * Do not disturb lets a call through only when it lets calls from anyone
 * through: the caller is nobody in the phone's contacts.
 */
internal object Ringer {
  private val main = Handler(Looper.getMainLooper())
  private var ringtone: Ringtone? = null
  private var vibrator: Vibrator? = null
  /** Before Android 9 a ringtone does not loop: it is started again. */
  private var again: Runnable? = null

  private val pattern = longArrayOf(0, 1000, 1000)

  fun start(context: Context) {
    main.post { startHere(context.applicationContext) }
  }

  fun stop() {
    main.post { stopHere() }
  }

  private fun startHere(context: Context) {
    stopHere()
    val audio = context.getSystemService(AudioManager::class.java) ?: return
    if (!allowedByDnd(context)) {
      Log.i(CallState.TAG, "ringing silently: do not disturb")
      return
    }
    when (audio.ringerMode) {
      AudioManager.RINGER_MODE_SILENT -> return
      AudioManager.RINGER_MODE_VIBRATE -> vibrate(context)
      else -> {
        ring(context)
        vibrate(context)
      }
    }
  }

  private fun allowedByDnd(context: Context): Boolean {
    val notifications = context.getSystemService(NotificationManager::class.java) ?: return true
    return when (notifications.currentInterruptionFilter) {
      NotificationManager.INTERRUPTION_FILTER_ALL, NotificationManager.INTERRUPTION_FILTER_UNKNOWN -> true
      NotificationManager.INTERRUPTION_FILTER_PRIORITY -> try {
        val policy = notifications.notificationPolicy
        policy.priorityCategories and NotificationManager.Policy.PRIORITY_CATEGORY_CALLS != 0 &&
          policy.priorityCallSenders == NotificationManager.Policy.PRIORITY_SENDERS_ANY
      } catch (e: Exception) {
        false
      }
      else -> false
    }
  }

  private fun ring(context: Context) {
    val uri = RingtoneManager.getActualDefaultRingtoneUri(context, RingtoneManager.TYPE_RINGTONE)
      ?: Settings.System.DEFAULT_RINGTONE_URI
    val tone = try {
      RingtoneManager.getRingtone(context, uri)
    } catch (e: Exception) {
      null
    } ?: return
    tone.audioAttributes = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
      .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
      .build()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      tone.isLooping = true
    } else {
      val loop = object : Runnable {
        override fun run() {
          if (ringtone === tone && !tone.isPlaying) tone.play()
          main.postDelayed(this, 1000)
        }
      }
      again = loop
      main.postDelayed(loop, 1000)
    }
    try {
      tone.play()
      ringtone = tone
    } catch (e: Exception) {
      Log.w(CallState.TAG, "the ringtone did not play: ${e.javaClass.simpleName}")
    }
  }

  private fun vibrate(context: Context) {
    val v = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      context.getSystemService(VibratorManager::class.java)?.defaultVibrator
    } else {
      @Suppress("DEPRECATION")
      context.getSystemService(Vibrator::class.java)
    } ?: return
    if (!v.hasVibrator()) return
    val effect = VibrationEffect.createWaveform(pattern, 0)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      v.vibrate(effect, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_RINGTONE))
    } else {
      @Suppress("DEPRECATION")
      v.vibrate(
        effect,
        AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build(),
      )
    }
    vibrator = v
  }

  private fun stopHere() {
    again?.let { main.removeCallbacks(it) }
    again = null
    ringtone?.let {
      try {
        it.stop()
      } catch (e: Exception) {
        // Already stopped.
      }
    }
    ringtone = null
    vibrator?.cancel()
    vibrator = null
  }
}

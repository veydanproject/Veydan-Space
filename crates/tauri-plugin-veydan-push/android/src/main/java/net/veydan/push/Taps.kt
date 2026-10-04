// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.core.app.OnNewIntentProvider
import androidx.startup.Initializer

/**
 * Catches the tap on a notification before the app's window has plugins.
 *
 * When the process died with the app still in the recent apps, a tap brings
 * the old task back: its window is created again with the intent it was
 * first started with, and the notification's intent comes to `onNewIntent`
 * right after, long before the app loads any plugin. So taps are caught for
 * the whole process, from the moment a window is created, and kept until
 * the page asks for them.
 */
class TapInitializer : Initializer<Unit> {
  override fun create(context: Context) {
    (context.applicationContext as? Application)?.registerActivityLifecycleCallbacks(Taps)
  }

  override fun dependencies(): List<Class<out Initializer<*>>> = emptyList()
}

internal object Taps : Application.ActivityLifecycleCallbacks {
  override fun onActivityCreated(activity: Activity, saved: Bundle?) {
    // A window brought back from saved state, or from the recent apps, was
    // started long ago: the intent it carries is not a new tap.
    val fromHistory = activity.intent?.flags?.and(Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0
    if (saved == null && !fromHistory) caught(activity.intent)
    (activity as? OnNewIntentProvider)?.addOnNewIntentListener { caught(it) }
  }

  private fun caught(intent: Intent?) {
    val tap = Notifier.readTap(intent) ?: return
    PushState.putTap(tap)
    // Nobody to tell yet on a cold start: the page asks when it is up.
    PushState.plugin()?.tapped()
  }

  override fun onActivityStarted(activity: Activity) {}
  override fun onActivityResumed(activity: Activity) {}
  override fun onActivityPaused(activity: Activity) {}
  override fun onActivityStopped(activity: Activity) {}
  override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
  override fun onActivityDestroyed(activity: Activity) {}
}

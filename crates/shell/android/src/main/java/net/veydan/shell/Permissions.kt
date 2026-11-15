// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import java.util.UUID

/**
 * The system's question for permissions, and another app's activity
 * launched for its result (the camera app, the system's Photo Picker),
 * from the app's window that is live now: the permissions of a call
 * (camera, microphone), of the gallery, the camera and the saving of a
 * picture (veydan-media), of the notifications (veydan-push). One per
 * process ([of]), whatever plugin asks.
 *
 * Not Tauri's `requestPermissionForAlias` and `startActivityForResult`:
 * their launchers are registered once, on the first window of the process
 * (`PluginManager.onActivityCreate` keeps the first and ignores the next).
 * A window closed while the process stays — "back", a swipe out of the
 * recent apps or the system while the call's service or a push holds the
 * process, a change of the font's size — and opened again, by the ringing
 * screen's answer too, left a launcher of a destroyed activity, and
 * launching it threw "unregistered ActivityResultLauncher" (ROO-1208, the
 * plugins of media and push ROO-1223).
 *
 * Here the window is followed through the application's activity
 * callbacks; a question waits until a window of the app is in front (the
 * ringing screen opens the app after the phone is unlocked) and is
 * registered on that window's own registry for the one answer, under a
 * key of its own: a window recreated from its saved state hands the answer
 * to a question of the window before it to the same key. One question at a time,
 * of all the plugins: the system answers a second question for permissions at
 * once with a no. Asked from any thread, answered on the main one.
 *
 * A key the registry restored with the window (recreated during the
 * question, or after the process was killed) got its answer while nothing
 * was registered for it: the answer waits as pending and the key stays
 * launched, so an unregister keeps it in the window's saved state for good.
 * [sweep] takes such keys out before every question, and hands a pending
 * answer to the question still waiting for it (the shot of the camera app
 * our window went away behind).
 */
class Permissions private constructor(first: Activity) : Application.ActivityLifecycleCallbacks {
  /**
   * `permissions` are of a question for them: none launched when all are granted, asked
   * again from the next window when the window went during it. Another app's activity is
   * not launched again: its answer comes to the next window under the key, or it was given up.
   */
  private class Question(
    val key: String,
    val intent: Intent,
    val permissions: Array<String>?,
    val answer: (ActivityResult?) -> Unit,
  ) {
    /** Launched from a window that went: answered from the registry of the next, or given up. */
    var away = false
    /** Its limit in the queue; one: a question back in it after its window went waits anew. */
    var limit: Runnable? = null
  }

  private val context: Context = first.applicationContext
  /** The app's window is of the class of the one the plugin came with (not the ringing screen, not a link's). */
  private val windowClass = first.javaClass
  private val main = Handler(Looper.getMainLooper())
  private var window: ComponentActivity? = first as? ComponentActivity
  private val waiting = ArrayDeque<Question>()
  private var asked: Question? = null
  private var askedIn: ComponentActivity? = null
  private var launcher: ActivityResultLauncher<Intent>? = null

  init {
    first.application.registerActivityLifecycleCallbacks(this)
  }

  fun granted(permissions: Array<String>): Boolean =
    permissions.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

  /**
   * `answer` gets whether all of `permissions` are granted: at once when
   * they are; after the system's question otherwise (a permission denied
   * for good is answered by the system without one); a no when no window
   * of the app comes in front within [WAIT_MS].
   */
  fun ask(permissions: Array<String>, answer: (Boolean) -> Unit) {
    main.post {
      if (granted(permissions)) {
        answer(true)
        return@post
      }
      val intent = ActivityResultContracts.RequestMultiplePermissions().createIntent(context, permissions)
      add(Question(newKey(), intent, permissions) { answer(granted(permissions)) })
    }
  }

  /**
   * Another app's activity for its result: `answer` gets it, a cancel when it
   * was given up or no window of the app came in front within [WAIT_MS], null
   * when no app of the phone takes `intent`.
   */
  fun launch(intent: Intent, answer: (ActivityResult?) -> Unit) {
    main.post { add(Question(newKey(), intent, null, answer)) }
  }

  private fun newKey() = "$KEY_PREFIX${UUID.randomUUID()}"

  private fun add(question: Question) {
    wait(question, last = true, WAIT_MS)
    next()
  }

  private fun wait(question: Question, last: Boolean, limitMs: Long) {
    if (last) waiting.addLast(question) else waiting.addFirst(question)
    question.limit?.let(main::removeCallbacks)
    val limit = Runnable {
      if (waiting.remove(question)) {
        Log.w(TAG, "no window of the app for ${question.intent.action}")
        question.answer(ActivityResult(Activity.RESULT_CANCELED, null))
      }
    }
    question.limit = limit
    main.postDelayed(limit, limitMs)
  }

  private fun next() {
    if (asked != null) return
    val window = window ?: return
    if (window.isFinishing || !window.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) return
    sweep(window)
    while (true) {
      val question = waiting.removeFirstOrNull() ?: return
      val permissions = question.permissions
      if (permissions != null && granted(permissions)) {
        question.answer(null)
        continue
      }
      if (question.away) {
        // The registry of this window had no answer under its key: the activity was given up.
        question.answer(ActivityResult(Activity.RESULT_CANCELED, null))
        continue
      }
      // The registry without a lifecycle: a register after the window started is allowed.
      val launcher = window.activityResultRegistry.register(
        question.key,
        ActivityResultContracts.StartActivityForResult(),
      ) { answered(question, it) }
      try {
        launcher.launch(question.intent)
      } catch (e: RuntimeException) {
        // No app for it (ActivityNotFoundException), or one the app may not start (SecurityException).
        Log.w(TAG, "${question.intent.action} not launched: ${e.message}")
        launcher.unregister()
        question.answer(null)
        continue
      }
      asked = question
      askedIn = window
      this.launcher = launcher
      return
    }
  }

  private fun answered(question: Question, result: ActivityResult) {
    if (asked !== question) return
    // Unregistered after the dispatch: an unregister inside its callback leaves the key in the registry's saved state.
    val done = launcher
    launcher = null
    asked = null
    askedIn = null
    if (done != null) main.post { done.unregister() }
    question.answer(result)
    main.post { next() }
  }

  /**
   * Takes our keys the registry restored out of it. The window is resumed:
   * the answers for it were dispatched before. A pending answer is the end of
   * the question of the window before: a question still waiting under its key
   * gets it, it is not asked again. The registry lists its keys only in its saved
   * state: [ActivityResultRegistry.onSaveInstanceState] is public, the names
   * are the format windows are restored from.
   */
  private fun sweep(window: ComponentActivity) {
    val registry = window.activityResultRegistry
    val state = Bundle().also { registry.onSaveInstanceState(it) }
    val keys = state.getStringArrayList("KEY_COMPONENT_ACTIVITY_REGISTERED_KEYS") ?: return
    val codes = state.getIntegerArrayList("KEY_COMPONENT_ACTIVITY_REGISTERED_RCS") ?: return
    for ((key, code) in keys.zip(codes)) {
      if (!key.startsWith(KEY_PREFIX)) continue
      var handed: ActivityResult? = null
      // The register hands over a pending answer at once.
      val stale = registry.register(key, ActivityResultContracts.StartActivityForResult()) { handed = it }
      val result = handed
      val question = waiting.firstOrNull { it.key == key }
      if (result != null && question != null) {
        waiting.remove(question)
        question.answer(result)
      }
      // A dispatch with a callback takes the key out of the launched ones; then the unregister drops it.
      registry.dispatchResult(code, Activity.RESULT_CANCELED, null)
      stale.unregister()
    }
  }

  private fun forget() {
    launcher?.unregister()
    launcher = null
    asked = null
    askedIn = null
  }

  private fun ours(activity: Activity): ComponentActivity? =
    if (activity.javaClass == windowClass) activity as? ComponentActivity else null

  override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) {
    ours(activity)?.let { window = it }
  }

  override fun onActivityResumed(activity: Activity) {
    val window = ours(activity) ?: return
    this.window = window
    // Called before the window's lifecycle is RESUMED (Android 10+): asked once it is.
    main.post { next() }
  }

  override fun onActivityDestroyed(activity: Activity) {
    if (window === activity) window = null
    if (askedIn === activity) {
      // Its answer goes to the registry of the next window: a question for permissions is asked
      // again from it; another app's activity is waited for (the user may still be in it).
      val question = asked
      forget()
      if (question != null) {
        question.away = question.permissions == null
        wait(question, last = false, if (question.away) AWAY_MS else WAIT_MS)
      }
    }
  }

  override fun onActivityStarted(activity: Activity) {}

  override fun onActivityPaused(activity: Activity) {}

  override fun onActivityStopped(activity: Activity) {}

  override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}

  companion object {
    private const val TAG = "VeydanPermissions"

    /** As long as an answered call waits for the app (Calls.ANSWER_LIMIT_MS of veydan-call). */
    const val WAIT_MS = 60_000L

    /** As long as another app's activity is waited for after our window went behind it. */
    private const val AWAY_MS = 30 * 60_000L

    /**
     * Of our keys; a question adds a UUID: a key restored from a process before must not hand its answer to a new question.
     * The name of the call's, where the class was before: a key restored from a build before is swept as well.
     * Of the activities launched too: one sweep.
     */
    private const val KEY_PREFIX = "net.veydan.call.permission."

    private var one: Permissions? = null

    /** The one of the process: one question at a time and one sweep of our keys, whatever plugin asks. `activity` is the app's window the plugin came with. */
    @Synchronized
    fun of(activity: Activity): Permissions = one ?: Permissions(activity).also { one = it }
  }
}

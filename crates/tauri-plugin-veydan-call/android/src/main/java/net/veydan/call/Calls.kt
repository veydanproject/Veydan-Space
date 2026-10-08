// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.Manifest
import android.app.Activity
import android.app.ActivityManager
import android.content.Context
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import android.view.WindowManager
import androidx.core.content.ContextCompat
import app.tauri.plugin.JSObject
import java.lang.ref.WeakReference

/**
 * The phases of a call on this phone, and everything that follows each:
 *
 * - ringing: the call notification, the ringing screen, the ringtone, the
 *   foreground service of a phone call;
 * - ongoing: the notification with Hang up, the service that also holds the
 *   microphone (and the camera), the sound in the mode of a conversation,
 *   the proximity sensor while the sound is at the ear;
 * - nothing: all of it let go.
 *
 * The app's Rust code moves a call from phase to phase. A press of the user
 * moves it too, at once, without waiting for the app: Decline and Hang up
 * end it here, Answer stops the ringing. The app learns of the press after.
 *
 * Runs on the main thread; the plugin and the receiver hand their work over.
 */
internal object Calls {
  private enum class Phase { RINGING, ANSWERED, ONGOING }

  private val main = Handler(Looper.getMainLooper())

  private var call: CallInfo? = null
  private var phase: Phase? = null
  private var since = 0L
  private var screen: WeakReference<Activity>? = null
  private var proximity: PowerManager.WakeLock? = null
  private var awake: PowerManager.WakeLock? = null
  private var app: Context? = null

  /**
   * A ringing call nobody ended, and an answer the app never took up, end by
   * themselves: the app may have died, and a phone must not ring forever.
   * The app's own timeouts are shorter (an invitation lives 45 seconds).
   */
  const val RING_LIMIT_MS = 120_000L
  private const val ANSWER_LIMIT_MS = 60_000L
  /** An awake processor at most this long, whatever the app forgets. */
  private const val AWAKE_LIMIT_MS = 4 * 60 * 60 * 1000L

  private val limit = Runnable {
    if (phase == Phase.RINGING || phase == Phase.ANSWERED) {
      Log.w(CallState.TAG, "the call ${call?.callId} was left $phase; ended here")
      app?.let { stop(it) }
    }
  }

  init {
    AudioRoutes.onChange = {
      updateProximity()
      app?.let { CallState.routesChanged(AudioRoutes.describe(it)) }
    }
  }

  fun ringing(callId: String): Boolean = phase == Phase.RINGING && call?.callId == callId

  /**
   * Rings. Answers what the phone could do:
   * `{ foreground, fullScreen, notifications, ringing }`.
   *
   * The ringtone plays only while the user has something to press (see
   * `RingRules`): the notification, or the page of the app while the app
   * is in front. Otherwise the phone stays silent and `ringing` is false.
   */
  fun showIncoming(context: Context, called: CallInfo): JSObject {
    val ctx = context.applicationContext
    app = ctx
    // The same call rings already: by a push before the app heard of it,
    // or the other way round. The ringing goes on as it is; a ringtone
    // started anew would stutter, and the limit stays the earlier one. A
    // call answered here that the app only now hears of stays answered:
    // the user pressed, and is not asked again.
    val same = call?.takeIf { (phase == Phase.RINGING || phase == Phase.ANSWERED) && it.callId == called.callId }
    if (same != null) {
      Log.i(CallState.TAG, "ringing ${called.callId} already ($phase)")
      return (lastShown ?: JSObject()).put("inFront", inFront())
    }
    if (phase == Phase.ONGOING) {
      // A second call while one goes on is the app's to answer "busy"
      // before it rings here; a call that rings anyway takes the phone over.
      Log.w(CallState.TAG, "a call rings while ${call?.callId} goes on; it takes the phone over")
      AudioRoutes.end()
      setProximity(false)
    }
    closeScreen()
    Ringer.stop()
    val info = called.copy(rungAt = SystemClock.elapsedRealtime())
    call = info
    phase = Phase.RINGING
    val notification = CallNotices.incoming(ctx, info)
    val plain = { CallNotices.incoming(ctx, info, styled = false) }
    val foreground = CallService.show(ctx, notification, phoneCall(), plain)
    val shown = foreground || CallNotices.post(ctx, notification, plain)
    val ringing = RingRules.rings(shown && CallNotices.ringingVisible(ctx), inFront())
    if (ringing) Ringer.start(ctx)
    main.removeCallbacks(limit)
    main.postDelayed(limit, info.ringLimitMs(SystemClock.elapsedRealtime()))
    val answer = JSObject()
      .put("foreground", foreground)
      .put("fullScreen", CallNotices.fullScreenAllowed(ctx))
      .put("notifications", CallNotices.allowed(ctx))
      .put("ringing", ringing)
    lastShown = answer
    answer.put("inFront", inFront())
    Log.i(CallState.TAG, "ringing ${info.callId} (video=${info.video}, hidden=${info.hidden}): $answer")
    return answer
  }

  /** What the last ringing answered, for the same call rung again. */
  private var lastShown: JSObject? = null

  /** The calls that ended here lately: a late push for one rings nothing. */
  private val over = RecentlyOver(RING_LIMIT_MS)

  /**
   * A push says somebody calls (`CallActionReceiver.RING`). Rings unless
   * the call is over here already or another call has the phone (see
   * `RingRules.pushRings`): the app, when it runs, decides for those.
   * Answers whether it rang.
   */
  fun ringFromPush(context: Context, called: CallInfo): Boolean {
    val busyWith = call?.callId?.takeIf { phase != null }
    if (!RingRules.pushRings(called.callId, over.isOver(called.callId, SystemClock.elapsedRealtime()), busyWith)) {
      Log.i(CallState.TAG, "a push rings ${called.callId}: not rung (over here, or busy with $busyWith)")
      return false
    }
    showIncoming(context, called)
    return true
  }

  /** The user looks at the app: its process is the one in front. */
  private fun inFront(): Boolean {
    val state = ActivityManager.RunningAppProcessInfo()
    ActivityManager.getMyMemoryState(state)
    return state.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
  }

  /**
   * The process starts (the plugin loads) knowing no call: a call
   * notification still there was left by a process that died, with its
   * limits and its ringing. It goes; the app shows a live call again.
   */
  fun loaded(context: Context) {
    if (phase != null || call != null) return
    CallNotices.cancel(context.applicationContext)
  }

  /**
   * The ringing screen was asked for a call that does not ring here. A
   * notification of a call this process never knew and too old to be live
   * is a leftover of a dead process: it goes.
   */
  fun notRinging(context: Context, info: CallInfo) {
    if (phase != null || call != null) return
    if (!RingAge.fresh(info.rungAt, SystemClock.elapsedRealtime(), RING_LIMIT_MS)) {
      Log.i(CallState.TAG, "a leftover ringing of ${info.callId} cleared")
      CallNotices.cancel(context.applicationContext)
    }
  }

  /**
   * The ringing ends without a word from the user: answered elsewhere,
   * cancelled, expired. The app also dismisses a call it ended without
   * the phone ever ringing for it (answered "busy", missed on a late
   * catch-up): remembered, so that a push for it rings nothing.
   */
  fun dismissIncoming(context: Context, callId: String?) {
    callId?.let { over.ended(it, SystemClock.elapsedRealtime()) }
    if (phase != Phase.RINGING && phase != Phase.ANSWERED) return
    if (callId != null && call?.callId != callId) return
    stop(context)
  }

  /**
   * A push says the call is over (`CallActionReceiver.DISMISS`): taken on
   * another device, declined there, or given up by the caller. The same
   * as `dismissIncoming`, and besides: a process that knows no call at
   * all was started by this very push, and a call notification still
   * there is a leftover of the process that rang and died (as `loaded`
   * reasons); it goes. The ringing of another call, and a call that goes
   * on, are not touched: the app ends those itself.
   */
  fun dismissFromPush(context: Context, callId: String) {
    Log.i(CallState.TAG, "a push ends the ringing of $callId (${phase ?: "nothing"} here${call?.let { ", ${it.callId}" } ?: ""})")
    if (phase == null && call == null) CallNotices.cancel(context.applicationContext)
    dismissIncoming(context, callId)
  }

  /**
   * Answer pressed, on the notification or on the ringing screen.
   *
   * A press on a call this process does not know comes from a notification
   * a dead process left. It still reaches the app while the ringing could
   * be live (the app knows whether the invitation is); an older one is
   * dropped with its notification, so that a call long gone is not answered.
   */
  fun answered(context: Context, info: CallInfo) {
    val ctx = context.applicationContext
    app = ctx
    val callId = info.callId
    if (call?.callId == callId && phase == Phase.RINGING) {
      Ringer.stop()
      closeScreen()
      phase = Phase.ANSWERED
      main.removeCallbacks(limit)
      main.postDelayed(limit, ANSWER_LIMIT_MS)
    } else if (phase == null && call == null) {
      CallNotices.cancel(ctx)
      if (!RingAge.fresh(info.rungAt, SystemClock.elapsedRealtime(), RING_LIMIT_MS)) {
        Log.w(CallState.TAG, "answer on ${info.callId}, which rang too long ago; dropped")
        return
      }
    }
    CallState.pressed(Press(callId, "answer"))
  }

  /**
   * Decline pressed. The call ends here, whatever this process knows of
   * it, and a push for it rings nothing more.
   *
   * The press reaches the app only when the app runs: a process the push
   * started has no runtime to sign and send `call.decline`, and a
   * receiver may start no window. The caller then rings on until the
   * invitation expires and records the call as unanswered, not declined.
   * A headless runtime of the push process for this one signal is a
   * later stage (the plan of calls).
   */
  fun declined(context: Context, callId: String) {
    over.ended(callId, SystemClock.elapsedRealtime())
    if (call == null || call?.callId == callId) stop(context)
    CallState.pressed(Press(callId, "decline"))
  }

  /** Hang up pressed. */
  fun hungUp(context: Context, callId: String) {
    over.ended(callId, SystemClock.elapsedRealtime())
    if (call == null || call?.callId == callId) stop(context)
    CallState.pressed(Press(callId, "hangup"))
  }

  /** The call goes on: the notification with Hang up, the microphone, the sound. */
  fun startOngoing(context: Context, info: CallInfo) {
    val ctx = context.applicationContext
    app = ctx
    main.removeCallbacks(limit)
    Ringer.stop()
    closeScreen()
    call = info
    if (phase != Phase.ONGOING) since = System.currentTimeMillis()
    phase = Phase.ONGOING
    AudioRoutes.begin(ctx, info.video)
    val notification = CallNotices.ongoing(ctx, info, since)
    var types = phoneCall()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      // A type whose permission the user has not given makes the system
      // refuse the whole service: it goes without it.
      if (granted(ctx, Manifest.permission.RECORD_AUDIO)) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
      if (info.video && granted(ctx, Manifest.permission.CAMERA)) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
    }
    val plain = { CallNotices.ongoing(ctx, info, since, styled = false) }
    if (!CallService.show(ctx, notification, types, plain)) CallNotices.post(ctx, notification, plain)
    updateProximity()
    Log.i(CallState.TAG, "ongoing ${info.callId} (video=${info.video})")
  }

  /** Everything let go. */
  fun stop(context: Context) {
    val ctx = context.applicationContext
    main.removeCallbacks(limit)
    Ringer.stop()
    closeScreen()
    AudioRoutes.end()
    setProximity(false)
    keepAwake(null, false)
    CallService.stop()
    CallNotices.cancel(ctx)
    call?.let {
      Log.i(CallState.TAG, "ended ${it.callId}")
      over.ended(it.callId, SystemClock.elapsedRealtime())
    }
    call = null
    phase = null
  }

  /** The ringing screen, closed when the ringing ends. */
  fun attachScreen(activity: Activity) {
    screen = WeakReference(activity)
  }

  fun detachScreen(activity: Activity) {
    if (screen?.get() === activity) screen = null
  }

  private fun closeScreen() {
    screen?.get()?.let { if (!it.isFinishing) it.finish() }
    screen = null
  }

  /** The processor awake and, with `activity`, its screen on. */
  fun keepAwake(activity: Activity?, on: Boolean) {
    activity?.window?.let {
      if (on) it.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
      else it.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }
    if (on) {
      val ctx = activity?.applicationContext ?: app ?: return
      if (awake == null) {
        awake = ctx.getSystemService(PowerManager::class.java)
          ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "veydan:call")
          ?.apply {
            setReferenceCounted(false)
            acquire(AWAKE_LIMIT_MS)
          }
      }
    } else {
      awake?.let { if (it.isHeld) it.release() }
      awake = null
    }
  }

  /** At the ear the screen goes dark: a call without video, its sound in the earpiece. */
  private fun updateProximity() {
    setProximity(phase == Phase.ONGOING && call?.video == false && AudioRoutes.current() == AudioRoutes.EARPIECE)
  }

  private fun setProximity(on: Boolean) {
    if (on) {
      if (proximity != null) return
      val power = app?.getSystemService(PowerManager::class.java) ?: return
      if (!power.isWakeLockLevelSupported(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK)) return
      proximity = power.newWakeLock(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK, "veydan:call-proximity").apply {
        setReferenceCounted(false)
        acquire(AWAKE_LIMIT_MS)
      }
      Log.i(CallState.TAG, "proximity sensor on")
    } else {
      val lock = proximity ?: return
      // The screen comes back when the phone leaves the ear, not at once.
      if (lock.isHeld) lock.release(PowerManager.RELEASE_FLAG_WAIT_FOR_NO_PROXIMITY)
      proximity = null
      Log.i(CallState.TAG, "proximity sensor off")
    }
  }

  private fun phoneCall(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL else 0

  private fun granted(context: Context, permission: String): Boolean =
    ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
}

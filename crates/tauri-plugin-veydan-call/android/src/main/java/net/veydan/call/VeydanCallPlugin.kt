// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.Manifest
import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.webkit.WebView
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class IncomingArgs {
  var callId: String? = null
  var name: String? = null
  var avatar: String? = null
  var video: Boolean = false
  /** Who calls stays off the lock screen; hidden when not said. */
  var hideOnLockscreen: Boolean = true
}

@InvokeArg
class DismissArgs {
  var callId: String? = null
}

@InvokeArg
class OngoingArgs {
  var callId: String? = null
  var name: String? = null
  var video: Boolean = false
  var hideOnLockscreen: Boolean = true
}

@InvokeArg
class GroupCallArgs {
  var callId: String? = null
  /** The chat of the group (`group:<id>`), which a tap opens. */
  var chatId: String? = null
  var name: String? = null
  var video: Boolean = false
  var hideOnLockscreen: Boolean = true
}

@InvokeArg
class RouteArgs {
  var route: String? = null
}

@InvokeArg
class AwakeArgs {
  var on: Boolean = false
}

@InvokeArg
class CameraArgs {
  /** `front` (the default) or `back`. */
  var facing: String? = null
  var width: Int = 640
  var height: Int = 360
}

@InvokeArg
class PermissionArgs {
  /** `camera` or `microphone`. */
  var permission: String? = null
}

private const val CAMERA = "camera"
private const val MICROPHONE = "microphone"

/**
 * The app's side of the call bridge. Called from the app's Rust code only.
 *
 * The permissions of a call are the phone's: the camera does not open and
 * the service does not hold the microphone without them. The app asks for
 * them here (`requestPermission`), where the window to ask from is.
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.CAMERA], alias = CAMERA),
    Permission(strings = [Manifest.permission.RECORD_AUDIO], alias = MICROPHONE),
  ]
)
class VeydanCallPlugin(private val activity: Activity) : Plugin(activity) {
  private val main = Handler(Looper.getMainLooper())

  override fun load(webView: WebView) {
    CallState.attach(this)
    CallNotices.ensureChannels(activity)
    // The engine takes the JVM here, on the main thread, before any call.
    Engine.start(activity)
    // A call notification a dead process left goes before anything rings.
    main.post { Calls.loaded(activity) }
    // A change of the network is told to the app at once (a lost way is
    // restarted without waiting for the engine to notice).
    NetworkWatch.start(activity)
  }

  /** Runs `work` on the main thread, where the call's state lives, and answers with what it returns. */
  private fun onMain(invoke: Invoke, work: () -> JSObject?) {
    main.post {
      try {
        val answer = work()
        if (answer != null) invoke.resolve(answer) else invoke.resolve()
      } catch (e: Exception) {
        Log.w(CallState.TAG, "call command failed: ${e.javaClass.simpleName}: ${e.message}")
        invoke.reject(e.message ?: e.javaClass.simpleName)
      }
    }
  }

  @Command
  fun showIncoming(invoke: Invoke) {
    val args = invoke.parseArgs(IncomingArgs::class.java)
    val callId = args.callId
    if (callId.isNullOrEmpty()) {
      invoke.reject("no call id")
      return
    }
    val call = CallInfo(callId.take(128), (args.name ?: "").take(200), args.avatar, args.video, args.hideOnLockscreen)
    onMain(invoke) { Calls.showIncoming(activity, call) }
  }

  @Command
  fun dismissIncoming(invoke: Invoke) {
    val callId = invoke.parseArgs(DismissArgs::class.java).callId
    onMain(invoke) {
      Calls.dismissIncoming(activity, callId)
      null
    }
  }

  /**
   * A call is on in a group: a quiet notification (no sound, no buttons)
   * unless the app is in front; a tap opens the group's chat. Answers
   * `{ shown }`.
   */
  @Command
  fun showGroupCall(invoke: Invoke) {
    val args = invoke.parseArgs(GroupCallArgs::class.java)
    val callId = args.callId
    val chatId = args.chatId
    if (callId.isNullOrEmpty() || chatId.isNullOrEmpty()) {
      invoke.reject("no call or chat id")
      return
    }
    val notice = GroupCallNotice(callId.take(128), chatId.take(128), (args.name ?: "").take(200), args.video, args.hideOnLockscreen)
    onMain(invoke) { JSObject().put("shown", Calls.showGroupCall(activity, notice)) }
  }

  @Command
  fun dismissGroupCall(invoke: Invoke) {
    val callId = invoke.parseArgs(DismissArgs::class.java).callId
    if (callId.isNullOrEmpty()) {
      invoke.reject("no call id")
      return
    }
    onMain(invoke) {
      Calls.dismissGroupCall(activity, callId)
      null
    }
  }

  @Command
  fun startOngoing(invoke: Invoke) {
    val args = invoke.parseArgs(OngoingArgs::class.java)
    val callId = args.callId
    if (callId.isNullOrEmpty()) {
      invoke.reject("no call id")
      return
    }
    val call = CallInfo(callId.take(128), (args.name ?: "").take(200), null, args.video, args.hideOnLockscreen)
    onMain(invoke) {
      Calls.startOngoing(activity, call)
      null
    }
  }

  @Command
  fun stop(invoke: Invoke) {
    onMain(invoke) {
      Calls.stop(activity)
      null
    }
  }

  @Command
  fun setAudioRoute(invoke: Invoke) {
    val route = invoke.parseArgs(RouteArgs::class.java).route
    if (route !in listOf(AudioRoutes.EARPIECE, AudioRoutes.SPEAKER, AudioRoutes.BLUETOOTH, AudioRoutes.WIRED)) {
      invoke.reject("not a route: $route")
      return
    }
    onMain(invoke) {
      if (!AudioRoutes.active) throw IllegalStateException("no call holds the sound")
      if (!AudioRoutes.set(route!!)) throw IllegalStateException("the phone has no $route now")
      AudioRoutes.describe(activity)
    }
  }

  @Command
  fun listAudioRoutes(invoke: Invoke) {
    onMain(invoke) { AudioRoutes.describe(activity) }
  }

  @Command
  fun keepAwake(invoke: Invoke) {
    val on = invoke.parseArgs(AwakeArgs::class.java).on
    onMain(invoke) {
      Calls.keepAwake(activity, on)
      null
    }
  }

  @Command
  fun startCamera(invoke: Invoke) {
    val args = invoke.parseArgs(CameraArgs::class.java)
    val facing = if (args.facing == Camera.BACK) Camera.BACK else Camera.FRONT
    onMain(invoke) {
      Camera.start(activity, facing, args.width, args.height)
      null
    }
  }

  @Command
  fun stopCamera(invoke: Invoke) {
    onMain(invoke) {
      Camera.stop()
      null
    }
  }

  @Command
  fun switchCamera(invoke: Invoke) {
    onMain(invoke) {
      Camera.switch()
      null
    }
  }

  @Command
  fun cameraStats(invoke: Invoke) {
    invoke.resolve(Camera.describe())
  }

  /**
   * The system's question for the camera or the microphone, when it was
   * never answered; answers `{ granted }` after it. A permission denied
   * for good is answered without a question: the system shows none.
   */
  @Command
  fun requestPermission(invoke: Invoke) {
    val alias = invoke.parseArgs(PermissionArgs::class.java).permission
    if (alias != CAMERA && alias != MICROPHONE) {
      invoke.reject("not a permission: $alias")
      return
    }
    if (getPermissionState(alias) == PermissionState.GRANTED) {
      permissionAnswered(invoke)
    } else {
      requestPermissionForAlias(alias, invoke, "permissionAnswered")
    }
  }

  @PermissionCallback
  fun permissionAnswered(invoke: Invoke) {
    val alias = invoke.parseArgs(PermissionArgs::class.java).permission
    invoke.resolve(JSObject().put("granted", alias != null && getPermissionState(alias) == PermissionState.GRANTED))
  }

  @Command
  fun takeActions(invoke: Invoke) {
    invoke.resolve(JSObject().put("actions", CallState.takeWaiting()))
  }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject

/**
 * Where the sound of a call goes: the earpiece, the loudspeaker, a Bluetooth
 * or a wired headset.
 *
 * During a call the phone is in the mode of a conversation
 * (`MODE_IN_COMMUNICATION`): the echo canceller of the phone works and the
 * volume keys set the call's volume. Android 12 and later choose the device
 * with `setCommunicationDevice`; older ones know only "loudspeaker on" and
 * "Bluetooth SCO on", and the rest follows from what is plugged in.
 *
 * Everything runs on the main thread.
 */
internal object AudioRoutes {
  const val EARPIECE = "earpiece"
  const val SPEAKER = "speaker"
  const val BLUETOOTH = "bluetooth"
  const val WIRED = "wired"

  private val main = Handler(Looper.getMainLooper())

  private var audio: AudioManager? = null
  private var focus: AudioFocusRequest? = null
  private var savedMode = AudioManager.MODE_NORMAL
  /** Before Android 12 the phone does not say which device it uses: the last choice. */
  private var chosen: String? = null
  private var communicationListener: AudioManager.OnCommunicationDeviceChangedListener? = null

  /** Called after every change of the route or of the routes there are. */
  var onChange: (() -> Unit)? = null

  val active: Boolean get() = audio != null

  private val devices = object : AudioDeviceCallback() {
    override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) {
      // A headset put on during the call takes the sound, as on a phone call.
      val kinds = added.mapNotNull { kindOf(it.type) }
      val take = when {
        BLUETOOTH in kinds -> BLUETOOTH
        WIRED in kinds -> WIRED
        else -> null
      }
      if (take != null && active) set(take)
      changed()
    }

    override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) {
      if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S && chosen != null &&
        removed.any { kindOf(it.type) == chosen }
      ) {
        chosen = null
      }
      changed()
    }
  }

  /** The call takes the sound: the mode, the focus and the first route. */
  fun begin(context: Context, video: Boolean) {
    if (active) return
    val am = context.applicationContext.getSystemService(AudioManager::class.java) ?: return
    audio = am
    savedMode = am.mode
    val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
      .setAudioAttributes(
        AudioAttributes.Builder()
          .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
          .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
          .build()
      )
      .build()
    am.requestAudioFocus(request)
    focus = request
    am.mode = AudioManager.MODE_IN_COMMUNICATION
    am.registerAudioDeviceCallback(devices, main)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      val listener = AudioManager.OnCommunicationDeviceChangedListener { changed() }
      am.addOnCommunicationDeviceChangedListener({ main.post(it) }, listener)
      communicationListener = listener
    }
    val available = available(am)
    val first = when {
      BLUETOOTH in available -> BLUETOOTH
      WIRED in available -> WIRED
      video || EARPIECE !in available -> SPEAKER
      else -> EARPIECE
    }
    set(first)
    Log.i(CallState.TAG, "sound: in communication, route $first of $available")
  }

  /** The call lets the sound go: everything as it was. */
  fun end() {
    val am = audio ?: return
    am.unregisterAudioDeviceCallback(devices)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      communicationListener?.let { am.removeOnCommunicationDeviceChangedListener(it) }
      communicationListener = null
      am.clearCommunicationDevice()
    } else {
      releaseOld(am)
    }
    am.mode = if (savedMode == AudioManager.MODE_IN_COMMUNICATION) AudioManager.MODE_NORMAL else savedMode
    focus?.let { am.abandonAudioFocusRequest(it) }
    focus = null
    chosen = null
    audio = null
    Log.i(CallState.TAG, "sound: back to normal")
    changed()
  }

  /** Sends the sound to `route`; false when the phone has no such device or no call holds the sound. */
  fun set(route: String): Boolean {
    val am = audio ?: return false
    val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      val device = am.availableCommunicationDevices.firstOrNull { kindOf(it.type) == route }
      device != null && am.setCommunicationDevice(device)
    } else {
      setOld(am, route)
    }
    if (ok) chosen = route
    Log.i(CallState.TAG, "sound: route $route ${if (ok) "taken" else "refused"}")
    changed()
    return ok
  }

  /** Before Android 12: the loudspeaker off and Bluetooth SCO let go. */
  @Suppress("DEPRECATION")
  private fun releaseOld(am: AudioManager) {
    am.isSpeakerphoneOn = false
    if (am.isBluetoothScoOn) {
      am.isBluetoothScoOn = false
      am.stopBluetoothSco()
    }
  }

  @Suppress("DEPRECATION")
  private fun setOld(am: AudioManager, route: String): Boolean {
    if (route !in available(am)) return false
    // The earpiece and a wired headset: nothing on, the phone picks the one plugged in.
    releaseOld(am)
    when (route) {
      SPEAKER -> am.isSpeakerphoneOn = true
      BLUETOOTH -> {
        am.startBluetoothSco()
        am.isBluetoothScoOn = true
      }
    }
    return true
  }

  /** The route in use, or null while no call holds the sound. */
  fun current(): String? {
    val am = audio ?: return null
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      return am.communicationDevice?.let { kindOf(it.type) }
    }
    return chosen
  }

  /** `{ current, available }`, as the Rust side reads it. */
  fun describe(context: Context): JSObject {
    val am = audio ?: context.applicationContext.getSystemService(AudioManager::class.java)
    val list = JSArray()
    am?.let { for (kind in available(it)) list.put(kind) }
    return JSObject().put("current", current()).put("available", list)
  }

  private fun available(am: AudioManager): List<String> {
    val kinds = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      am.availableCommunicationDevices.mapNotNull { kindOf(it.type) }
    } else {
      am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).mapNotNull { kindOf(it.type) } + SPEAKER
    }
    // In one order, so that a list can be compared and shown as it is.
    return listOf(EARPIECE, SPEAKER, WIRED, BLUETOOTH).filter { it in kinds }
  }

  private fun kindOf(type: Int): String? = when (type) {
    AudioDeviceInfo.TYPE_BUILTIN_EARPIECE -> EARPIECE
    AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> SPEAKER
    AudioDeviceInfo.TYPE_WIRED_HEADSET,
    AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
    AudioDeviceInfo.TYPE_USB_HEADSET -> WIRED
    AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
    AudioDeviceInfo.TYPE_HEARING_AID,
    AudioDeviceInfo.TYPE_BLE_HEADSET,
    AudioDeviceInfo.TYPE_BLE_SPEAKER -> BLUETOOTH
    else -> null
  }

  private fun changed() {
    onChange?.invoke()
  }
}

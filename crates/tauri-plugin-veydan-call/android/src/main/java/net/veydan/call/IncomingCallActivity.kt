// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.Activity
import android.app.KeyguardManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.Button
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The ringing call on the whole screen, also over the lock screen: the
 * full-screen intent of the call notification opens it, and so does a tap on
 * the notification.
 *
 * It is native and small on purpose. It must come up at once on a phone
 * that was asleep, and it shows nothing of the app: the messenger's window
 * never appears over the lock screen. Answer unlocks the phone (the system
 * asks for the PIN when there is one) and then opens the app.
 *
 * A call the messenger keeps off the lock screen (`CallInfo.hidden`) is
 * shown here without the caller's name and picture while the phone is
 * locked, and with them once it is not.
 *
 * Answer on the notification comes here too, without drawing anything:
 * the press is taken at once, before the app's window exists.
 */
class IncomingCallActivity : Activity() {
  companion object {
    const val SHOW = "net.veydan.call.SHOW"
    const val ANSWER = "net.veydan.call.ANSWER"

    internal fun intent(context: Context, call: CallInfo, action: String): PendingIntent {
      val intent = call.into(Intent(context, IncomingCallActivity::class.java))
        .setAction(action)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_USER_ACTION)
      return PendingIntent.getActivity(
        context,
        action.hashCode(),
        intent,
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
      setShowWhenLocked(true)
      setTurnScreenOn(true)
    } else {
      @Suppress("DEPRECATION")
      window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON)
    }
    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    handle(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    handle(intent)
  }

  override fun onDestroy() {
    Calls.detachScreen(this)
    super.onDestroy()
  }

  private fun handle(intent: Intent?) {
    val call = CallInfo.from(intent)
    if (call == null) {
      finish()
      return
    }
    when (intent?.action) {
      ANSWER -> answer(call)
      else -> {
        // A ringing that ended while the screen was on its way, or one a
        // dead process left behind.
        if (!Calls.ringing(call.callId)) {
          Calls.notRinging(this, call)
          finish()
          return
        }
        Calls.attachScreen(this)
        shown = call
        revealed = null
        render()
      }
    }
  }

  /** The call on screen, drawn again when the phone is unlocked under it. */
  private var shown: CallInfo? = null
  private var revealed: Boolean? = null

  override fun onResume() {
    super.onResume()
    render()
  }

  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) render()
  }

  /**
   * Draws the call. Who calls is left out while the phone is locked and the
   * messenger keeps them off the lock screen (`LockScreen`).
   */
  private fun render() {
    val call = shown ?: return
    val locked = getSystemService(KeyguardManager::class.java)?.isKeyguardLocked ?: true
    val reveal = LockScreen.reveals(call.hidden, locked)
    if (revealed == reveal) return
    revealed = reveal
    setContentView(layout(call, reveal))
  }

  private fun answer(call: CallInfo) {
    // This screen stays until the phone is unlocked; the answer would close it.
    Calls.detachScreen(this)
    Calls.answered(this, call)
    val keyguard = getSystemService(KeyguardManager::class.java)
    if (keyguard != null && keyguard.isKeyguardLocked) {
      keyguard.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
        override fun onDismissSucceeded() {
          openApp(call)
        }

        override fun onDismissCancelled() {
          // The call is answered; the app opens from the notification.
          finish()
        }

        override fun onDismissError() {
          finish()
        }
      })
    } else {
      openApp(call)
    }
  }

  private fun openApp(call: CallInfo) {
    CallNotices.launch(this, call.callId)?.let { startActivity(it) }
    finish()
  }

  private fun decline(call: CallInfo) {
    Calls.declined(this, call.callId)
    finish()
  }

  private fun dp(value: Float): Int =
    TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value, resources.displayMetrics).toInt()

  /** `reveal`: the caller's name and picture; without it the app's name and a blank face. */
  private fun layout(call: CallInfo, reveal: Boolean): View {
    val root = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_HORIZONTAL
      setBackgroundColor(Color.rgb(0x12, 0x16, 0x1c))
      setPadding(dp(24f), dp(96f), dp(24f), dp(64f))
    }
    val face = if (reveal) call else call.copy(name = "", avatar = null)
    root.addView(ImageView(this).apply {
      setImageBitmap(CallNotices.face(face))
    }, LinearLayout.LayoutParams(dp(128f), dp(128f)))
    root.addView(TextView(this).apply {
      text = if (reveal) CallNotices.label(this@IncomingCallActivity, call) else applicationInfo.loadLabel(packageManager)
      setTextColor(Color.WHITE)
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 28f)
      typeface = Typeface.DEFAULT_BOLD
      gravity = Gravity.CENTER
      maxLines = 2
      setPadding(0, dp(24f), 0, dp(8f))
    })
    root.addView(TextView(this).apply {
      text = getString(if (call.video) R.string.veydan_call_incoming_video else R.string.veydan_call_incoming)
      setTextColor(Color.rgb(0xb0, 0xb8, 0xc4))
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
      gravity = Gravity.CENTER
    })
    root.addView(View(this), LinearLayout.LayoutParams(0, 0, 1f))
    val buttons = LinearLayout(this).apply {
      orientation = LinearLayout.HORIZONTAL
      gravity = Gravity.CENTER
    }
    buttons.addView(
      round(getString(R.string.veydan_call_decline), Color.rgb(0xd9, 0x3b, 0x3b)) { decline(call) },
      LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
    )
    buttons.addView(
      round(getString(R.string.veydan_call_answer), Color.rgb(0x2e, 0xa0, 0x4f)) { answer(call) },
      LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
    )
    root.addView(buttons, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
    return root
  }

  private fun round(label: String, color: Int, press: () -> Unit): View {
    val column = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_HORIZONTAL
    }
    column.addView(Button(this).apply {
      background = GradientDrawable().apply {
        shape = GradientDrawable.OVAL
        setColor(color)
      }
      contentDescription = label
      setOnClickListener { press() }
    }, LinearLayout.LayoutParams(dp(72f), dp(72f)))
    // A vertical row gives a view without its own size the whole width:
    // the label is centred in it, under the button.
    column.addView(TextView(this).apply {
      text = label
      setTextColor(Color.WHITE)
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
      gravity = Gravity.CENTER_HORIZONTAL
      setPadding(0, dp(8f), 0, 0)
    })
    return column
  }
}

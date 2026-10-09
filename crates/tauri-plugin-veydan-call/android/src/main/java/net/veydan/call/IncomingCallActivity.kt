// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.Activity
import android.app.KeyguardManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Rect
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Build
import android.os.Bundle
import android.util.TypedValue
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowInsets
import android.view.WindowManager
import android.view.accessibility.AccessibilityNodeInfo
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
 * asks for the PIN when there is one) and then opens the app. Answer and
 * Decline are circles pulled toward the middle of the screen, or tapped
 * (`SwipeRule`).
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

    /** The hint of a swipe at rest; white while its circle is pressed. */
    private const val HINT = 0xFF8A93A0.toInt()
    /** How far along its way a circle must be let go to act. */
    private const val COMMIT = 0.5f

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
      clipChildren = false
    }
    // The window reaches under the navigation (edge to edge, forced from
    // Android 15): the circles keep above the bar and the home gesture.
    root.setOnApplyWindowInsetsListener { v, insets ->
      val bottom = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.mandatorySystemGestures()).bottom
      } else {
        @Suppress("DEPRECATION")
        val bars = insets.systemWindowInsetBottom
        @Suppress("DEPRECATION")
        val gestures = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) insets.mandatorySystemGestureInsets.bottom else 0
        maxOf(bars, gestures)
      }
      v.setPadding(v.paddingLeft, v.paddingTop, v.paddingRight, bottom + dp(32f))
      insets
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
    // The hints stand over their circles; the one pressed lights up.
    val hints = LinearLayout(this).apply {
      orientation = LinearLayout.HORIZONTAL
      setPadding(0, 0, 0, dp(16f))
    }
    val declineHint = hint(getString(R.string.veydan_call_swipe_decline))
    val answerHint = hint(getString(R.string.veydan_call_swipe_answer))
    hints.addView(declineHint, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
    hints.addView(answerHint, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
    root.addView(hints, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
    val buttons = LinearLayout(this).apply {
      orientation = LinearLayout.HORIZONTAL
      gravity = Gravity.CENTER
      clipChildren = false
    }
    buttons.addView(
      round(
        SwipeRule.Circle.DECLINE, getString(R.string.veydan_call_decline), R.drawable.ic_call_decline,
        Color.rgb(0xd9, 0x3b, 0x3b), declineHint,
      ) { decline(call) },
      LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
    )
    buttons.addView(
      round(
        SwipeRule.Circle.ANSWER, getString(R.string.veydan_call_answer),
        if (call.video) R.drawable.ic_call_answer_video else R.drawable.ic_call_answer,
        Color.rgb(0x2e, 0xa0, 0x4f), answerHint,
      ) { answer(call) },
      LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f),
    )
    root.addView(buttons, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
    return root
  }

  private fun hint(text: String): TextView = TextView(this).apply {
    this.text = text
    setTextColor(HINT)
    setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
    gravity = Gravity.CENTER_HORIZONTAL
    setPadding(dp(4f), 0, dp(4f), 0)
  }

  /**
   * A circle with its icon and label. It is pulled toward the middle to
   * act (`SwipeRule`): a phone in a pocket rarely does that. A tap does
   * not answer or decline (the owner's decision, 2026-10-09): it nudges the
   * circle the way it goes and lights up the hint. A screen reader or a
   * keyboard still clicks it, since a swipe is not theirs to make.
   */
  private fun round(
    circle: SwipeRule.Circle,
    label: String,
    icon: Int,
    color: Int,
    hint: TextView,
    press: () -> Unit,
  ): View {
    val column = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      gravity = Gravity.CENTER_HORIZONTAL
      clipChildren = false
    }
    val oval = GradientDrawable().apply {
      shape = GradientDrawable.OVAL
      setColor(color)
    }
    val caption = TextView(this).apply {
      text = label
      setTextColor(Color.WHITE)
      setTextSize(TypedValue.COMPLEX_UNIT_SP, 16f)
      gravity = Gravity.CENTER_HORIZONTAL
      setPadding(0, dp(8f), 0, 0)
      // The circle says it; the screen reader need not say it twice.
      importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }
    val button = ImageView(this).apply {
      // Without a Button's own press look: the ripple shows the touch.
      background = RippleDrawable(ColorStateList.valueOf(Color.argb(0x4d, 0xff, 0xff, 0xff)), oval, null)
      setImageResource(icon)
      scaleType = ImageView.ScaleType.FIT_CENTER
      // A 32dp icon in the 72dp circle.
      setPadding(dp(20f), dp(20f), dp(20f), dp(20f))
      contentDescription = label
      isClickable = true
      isFocusable = true
      // The click of a screen reader or a keyboard; a finger swipes.
      setOnClickListener { press() }
      accessibilityDelegate = object : View.AccessibilityDelegate() {
        override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
          super.onInitializeAccessibilityNodeInfo(host, info)
          info.className = Button::class.java.name
          info.addAction(AccessibilityNodeInfo.AccessibilityAction(AccessibilityNodeInfo.ACTION_CLICK, label))
        }
      }
      setOnTouchListener(Swipe(circle, hint, caption, press))
    }
    column.addView(button, LinearLayout.LayoutParams(dp(72f), dp(72f)))
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      // The back gesture of the screen's edges must not take the drag.
      // The home gesture at the bottom cannot be kept out: the insets keep
      // the circles above it.
      button.addOnLayoutChangeListener { v, _, _, _, _, _, _, _, _ ->
        v.systemGestureExclusionRects = listOf(Rect(0, 0, v.width, v.height))
      }
    }
    // A vertical row gives a view without its own size the whole width:
    // the label is centred in it, under the button.
    column.addView(caption)
    return column
  }

  /** The drag of a circle: it follows the finger its own way, as far as the middle of the row. */
  private inner class Swipe(
    private val circle: SwipeRule.Circle,
    private val hint: TextView,
    private val caption: TextView,
    private val press: () -> Unit,
  ) : View.OnTouchListener {
    private val slop = ViewConfiguration.get(this@IncomingCallActivity).scaledTouchSlop.toFloat()
    private var downX = 0f
    private var downY = 0f
    private var farthest = 0f
    private var travel = 0f

    override fun onTouch(v: View, e: MotionEvent): Boolean {
      when (e.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
          v.animate().cancel()
          downX = e.rawX
          downY = e.rawY
          farthest = 0f
          travel = travelOf(v)
          v.parent?.requestDisallowInterceptTouchEvent(true)
          v.drawableHotspotChanged(e.x, e.y)
          v.isPressed = true
          hint.setTextColor(Color.WHITE)
        }
        MotionEvent.ACTION_MOVE -> {
          val dx = e.rawX - downX
          farthest = maxOf(farthest, Math.abs(dx), Math.abs(e.rawY - downY))
          v.translationX = SwipeRule.offset(dx, circle, travel)
          caption.alpha = 1f - SwipeRule.progress(dx, circle, travel)
          if (!SwipeRule.isTap(farthest, slop)) v.isPressed = false
        }
        MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
          v.isPressed = false
          val dx = e.rawX - downX
          farthest = maxOf(farthest, Math.abs(dx), Math.abs(e.rawY - downY))
          val tap = e.actionMasked == MotionEvent.ACTION_UP && SwipeRule.isTap(farthest, slop)
          val acts = if (e.actionMasked == MotionEvent.ACTION_UP) {
            SwipeRule.decide(dx, circle, travel * COMMIT, travel)
          } else {
            null
          }
          when {
            tap -> nudge(v)
            acts != null -> {
              v.performHapticFeedback(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM else HapticFeedbackConstants.LONG_PRESS,
              )
              press()
            }
            else -> {
              back(v)
              hint.setTextColor(HINT)
            }
          }
        }
      }
      return true
    }

    /** A tap: the circle starts its way and comes back, and the hint stays lit a moment, to show what to do. */
    private fun nudge(v: View) {
      v.animate().translationX(SwipeRule.way(circle) * dp(16f)).setDuration(140).withEndAction {
        v.animate().translationX(0f).setDuration(220).start()
      }.start()
      caption.animate().alpha(1f).setDuration(200).start()
      hint.removeCallbacks(dim)
      hint.postDelayed(dim, 1500)
    }

    private val dim = Runnable { hint.setTextColor(HINT) }

    private fun back(v: View) {
      v.animate().translationX(0f).setDuration(200).start()
      caption.animate().alpha(1f).setDuration(200).start()
    }

    /** From the circle's centre, where it stands at rest, to the middle of the row of buttons. */
    private fun travelOf(v: View): Float {
      val column = v.parent as? View ?: return 0f
      val row = column.parent as? View ?: return 0f
      val centre = column.left + v.left + v.width / 2f
      return Math.abs(row.width / 2f - centre)
    }
  }
}

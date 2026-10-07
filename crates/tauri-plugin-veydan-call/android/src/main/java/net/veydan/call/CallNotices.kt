// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Shader
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.graphics.drawable.IconCompat
import java.io.File

/**
 * A call as the notifications and the ringing screen show it.
 *
 * `hidden`: who calls stays off the lock screen (the messenger's privacy
 * settings). `rungAt`: when it began to ring (`elapsedRealtime`, 0 for a
 * call that never rang here), so that a press that outlived the process
 * that rang can be told from a live one. `expiresAt`: when the invitation
 * is no longer good (`elapsedRealtime`, 0 for "the limit of the ringing"):
 * a call rung by a push, which the app may never hear the end of, stops
 * ringing by itself then.
 */
internal data class CallInfo(
  val callId: String,
  val name: String,
  val avatar: String?,
  val video: Boolean,
  val hidden: Boolean = false,
  val rungAt: Long = 0,
  val expiresAt: Long = 0,
) {
  fun into(intent: Intent): Intent = intent
    .putExtra(EXTRA_CALL_ID, callId)
    .putExtra(EXTRA_NAME, name)
    .putExtra(EXTRA_AVATAR, avatar)
    .putExtra(EXTRA_VIDEO, video)
    .putExtra(EXTRA_HIDDEN, hidden)
    .putExtra(EXTRA_RUNG_AT, rungAt)
    .putExtra(EXTRA_EXPIRES_AT, expiresAt)

  /** How long the ringing may go on from `now`: to the expiry, and never past the limit of a ringing. */
  fun ringLimitMs(now: Long): Long = RingAge.limit(expiresAt, now, Calls.RING_LIMIT_MS)

  companion object {
    const val EXTRA_CALL_ID = "veydan_call_id"
    const val EXTRA_NAME = "veydan_call_name"
    const val EXTRA_AVATAR = "veydan_call_avatar"
    const val EXTRA_VIDEO = "veydan_call_video"
    const val EXTRA_HIDDEN = "veydan_call_hidden"
    const val EXTRA_RUNG_AT = "veydan_call_rung_at"
    const val EXTRA_EXPIRES_AT = "veydan_call_expires_at"

    fun from(intent: Intent?): CallInfo? {
      val id = intent?.getStringExtra(EXTRA_CALL_ID) ?: return null
      return CallInfo(
        id.take(128),
        (intent.getStringExtra(EXTRA_NAME) ?: "").take(200),
        intent.getStringExtra(EXTRA_AVATAR),
        intent.getBooleanExtra(EXTRA_VIDEO, false),
        // Hidden unless told otherwise: an intent without the flag shows nobody.
        intent.getBooleanExtra(EXTRA_HIDDEN, true),
        intent.getLongExtra(EXTRA_RUNG_AT, 0),
        intent.getLongExtra(EXTRA_EXPIRES_AT, 0),
      )
    }
  }
}

/**
 * The notifications of a call: the ringing one (Answer, Decline, and the
 * screen over the lock screen) and the ongoing one (Hang up).
 *
 * Both are `CallStyle`: the system shows them as calls, first in the shade,
 * with the buttons in its own words. Android wants such a notification to
 * belong to a foreground service or to carry a full-screen screen; when the
 * service could not start and the system refuses the style, a plain
 * notification with the same buttons stands in.
 *
 * A call the messenger keeps off the lock screen (`CallInfo.hidden`) is
 * private there: the lock screen shows its public version, "Incoming call"
 * or "Call in progress" with the buttons, and no name or picture.
 */
internal object CallNotices {
  /**
   * The ringing call: loud in the eyes of the system, silent in its ears
   * (the Ringer rings). Its lock-screen visibility is left to each
   * notification, so that a private call stays private.
   */
  private const val CHANNEL_CALLS = "call_incoming"
  /** The first channel of ringing calls, made public on the lock screen; deleted. */
  private const val CHANNEL_CALLS_OLD = "calls"
  /** The call that goes on: in the shade, never popping up. */
  const val CHANNEL_ONGOING = "call_ongoing"

  /**
   * One notification for the call, whatever phase it is in, untagged: the
   * service shows its own under the same number, and one shown without the
   * service takes the same place.
   */
  const val ID = 0x5ca11

  fun ensureChannels(context: Context) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    val calls = NotificationChannel(CHANNEL_CALLS, context.getString(R.string.veydan_call_channel), NotificationManager.IMPORTANCE_HIGH).apply {
      setSound(null, null)
      enableVibration(false)
    }
    val ongoing = NotificationChannel(CHANNEL_ONGOING, context.getString(R.string.veydan_call_channel_ongoing), NotificationManager.IMPORTANCE_LOW).apply {
      setSound(null, null)
      enableVibration(false)
      setShowBadge(false)
    }
    manager.createNotificationChannels(listOf(calls, ongoing))
    if (manager.getNotificationChannel(CHANNEL_CALLS_OLD) != null) manager.deleteNotificationChannel(CHANNEL_CALLS_OLD)
  }

  fun allowed(context: Context): Boolean =
    NotificationManagerCompat.from(context).areNotificationsEnabled()

  /**
   * Does a ringing notification reach the user: the app's notifications are
   * on and the user has not turned the channel of calls off. Without it the
   * notification, the service's one included, is never seen, and neither is
   * its full-screen screen.
   */
  fun ringingVisible(context: Context): Boolean {
    if (!allowed(context)) return false
    ensureChannels(context)
    val channel = context.getSystemService(NotificationManager::class.java)?.getNotificationChannel(CHANNEL_CALLS)
    return channel != null && channel.importance != NotificationManager.IMPORTANCE_NONE
  }

  /** May the app take the screen of a locked phone with a ringing call. */
  fun fullScreenAllowed(context: Context): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE) return true
    return context.getSystemService(NotificationManager::class.java)?.canUseFullScreenIntent() ?: false
  }

  fun incoming(context: Context, call: CallInfo, styled: Boolean = true): Notification {
    ensureChannels(context)
    val screen = IncomingCallActivity.intent(context, call, IncomingCallActivity.SHOW)
    val answer = IncomingCallActivity.intent(context, call, IncomingCallActivity.ANSWER)
    val decline = CallActionReceiver.intent(context, call.callId, CallActionReceiver.DECLINE)
    val text = context.getString(if (call.video) R.string.veydan_call_incoming_video else R.string.veydan_call_incoming)
    val builder = NotificationCompat.Builder(context, CHANNEL_CALLS)
      .setSmallIcon(R.drawable.ic_stat_call)
      .setContentTitle(label(context, call))
      .setContentText(text)
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setPriority(NotificationCompat.PRIORITY_MAX)
      .setOngoing(true)
      .setAutoCancel(false)
      // The limit of the ringing, kept by the system: the process that
      // rang may die and take its own limit along. A notification of the
      // service is never timed out; the service lives with the process.
      .setTimeoutAfter(call.ringLimitMs(android.os.SystemClock.elapsedRealtime()))
      .setContentIntent(screen)
      .setFullScreenIntent(screen, true)
    if (call.hidden) {
      // The lock screen shows this one: the buttons, nobody's name.
      val onLock = NotificationCompat.Builder(context, CHANNEL_CALLS)
        .setSmallIcon(R.drawable.ic_stat_call)
        .setContentTitle(text)
        .setCategory(NotificationCompat.CATEGORY_CALL)
        .setContentIntent(screen)
        .addAction(0, context.getString(R.string.veydan_call_decline), decline)
        .addAction(0, context.getString(R.string.veydan_call_answer), answer)
        .build()
      builder.setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setPublicVersion(onLock)
    } else {
      builder.setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
    }
    if (styled) {
      builder.setStyle(
        NotificationCompat.CallStyle.forIncomingCall(person(context, call), decline, answer)
          .setIsVideo(call.video)
      )
    } else {
      builder
        .addAction(0, context.getString(R.string.veydan_call_decline), decline)
        .addAction(0, context.getString(R.string.veydan_call_answer), answer)
    }
    return builder.build()
  }

  fun ongoing(context: Context, call: CallInfo, since: Long, styled: Boolean = true): Notification {
    ensureChannels(context)
    val hangup = CallActionReceiver.intent(context, call.callId, CallActionReceiver.HANGUP)
    val builder = NotificationCompat.Builder(context, CHANNEL_ONGOING)
      .setSmallIcon(R.drawable.ic_stat_call)
      .setContentTitle(label(context, call))
      .setContentText(context.getString(R.string.veydan_call_ongoing))
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setWhen(since)
      .setUsesChronometer(true)
      .setShowWhen(true)
      .setContentIntent(openApp(context, call.callId))
    if (call.hidden) {
      val onLock = NotificationCompat.Builder(context, CHANNEL_ONGOING)
        .setSmallIcon(R.drawable.ic_stat_call)
        .setContentTitle(context.getString(R.string.veydan_call_ongoing))
        .setCategory(NotificationCompat.CATEGORY_CALL)
        .setWhen(since)
        .setUsesChronometer(true)
        .setShowWhen(true)
        .addAction(0, context.getString(R.string.veydan_call_hangup), hangup)
        .build()
      builder.setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setPublicVersion(onLock)
    } else {
      builder.setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
    }
    if (styled) {
      builder.setStyle(
        NotificationCompat.CallStyle.forOngoingCall(person(context, call), hangup)
          .setIsVideo(call.video)
      )
    } else {
      builder.addAction(0, context.getString(R.string.veydan_call_hangup), hangup)
    }
    return builder.build()
  }

  /**
   * Shows `notification` without the service. A call style the system
   * refuses there is shown plain: `plain` makes the same one without it.
   */
  fun post(context: Context, notification: Notification, plain: () -> Notification): Boolean {
    val manager = NotificationManagerCompat.from(context)
    return try {
      manager.notify(ID, notification)
      true
    } catch (e: IllegalArgumentException) {
      try {
        manager.notify(ID, plain())
        true
      } catch (e: SecurityException) {
        false
      }
    } catch (e: SecurityException) {
      // The permission was taken away between the check and the call.
      false
    }
  }

  fun cancel(context: Context) {
    NotificationManagerCompat.from(context).cancel(ID)
  }

  /** The app's own start, with the call it is about. */
  fun openApp(context: Context, callId: String?): PendingIntent? {
    val intent = launch(context, callId) ?: return null
    return PendingIntent.getActivity(
      context,
      REQUEST_OPEN,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
  }

  fun launch(context: Context, callId: String?): Intent? {
    val intent = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return null
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    callId?.let { intent.putExtra(CallInfo.EXTRA_CALL_ID, it) }
    return intent
  }

  private const val REQUEST_OPEN = 0x5ca10

  /**
   * Who the call is shown as: the name the app gave, or the app's own
   * when it gave none (its notifications say no content, or it has a
   * PIN; as a computer shows it).
   */
  fun label(context: Context, call: CallInfo): String =
    call.name.ifBlank { context.applicationInfo.loadLabel(context.packageManager).toString() }

  private fun person(context: Context, call: CallInfo): Person =
    Person.Builder()
      .setName(label(context, call))
      .setKey(call.callId)
      .setImportant(true)
      .setIcon(IconCompat.createWithBitmap(face(call)))
      .build()

  /** The caller's picture from the file the app gave, or the circle with initials. */
  fun face(call: CallInfo): Bitmap {
    call.avatar?.let { path ->
      try {
        val file = File(path)
        if (file.isFile && file.length() in 1..MAX_AVATAR_BYTES) {
          BitmapFactory.decodeFile(file.absolutePath)?.let { circle(it)?.let { c -> return c } }
        }
      } catch (e: Exception) {
        // The initials then.
      }
    }
    return initials(call.name)
  }

  private const val SIZE = 256
  private const val MAX_AVATAR_BYTES = 8L * 1024 * 1024

  private fun circle(source: Bitmap): Bitmap? {
    val side = minOf(source.width, source.height)
    if (side <= 0) return null
    val square = Bitmap.createBitmap(source, (source.width - side) / 2, (source.height - side) / 2, side, side)
    val scaled = Bitmap.createScaledBitmap(square, SIZE, SIZE, true)
    val out = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888)
    val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { shader = BitmapShader(scaled, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP) }
    Canvas(out).drawCircle(SIZE / 2f, SIZE / 2f, SIZE / 2f, paint)
    return out
  }

  /** Two letters and a hue from the name: the call has no key to colour by. */
  private fun initials(label: String): Bitmap {
    val bitmap = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(bitmap)
    val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    var h = 0L
    for (ch in label) h = (h * 31 + ch.code) and 0xffffffffL
    paint.color = Color.HSVToColor(floatArrayOf((h % 360).toFloat(), 0.55f, 0.72f))
    canvas.drawCircle(SIZE / 2f, SIZE / 2f, SIZE / 2f, paint)
    val text = letters(label)
    paint.color = Color.WHITE
    paint.textSize = SIZE * 0.42f
    paint.textAlign = Paint.Align.CENTER
    paint.isFakeBoldText = true
    val bounds = Rect()
    paint.getTextBounds(text, 0, text.length, bounds)
    canvas.drawText(text, SIZE / 2f, SIZE / 2f - bounds.exactCenterY(), paint)
    return bitmap
  }

  private fun letters(label: String): String {
    val words = label.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (words.isEmpty()) return "?"
    fun first(word: String) = String(Character.toChars(word.codePointAt(0)))
    return (if (words.size > 1) first(words[0]) + first(words[1]) else first(words[0])).uppercase()
  }
}

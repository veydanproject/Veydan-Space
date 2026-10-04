// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat

/**
 * Shows and removes notifications. Needs no window and no Rust: the words
 * are this phone's own strings, the facts come from the messenger's core
 * or, without it, from the push alone.
 *
 * One notification per chat, tagged with the chat; a new message joins the
 * ones already shown. All of them hang under one summary, so that a tap on
 * the summary opens the messenger too.
 */
internal object Notifier {
  private const val CHANNEL_DM = "dm"
  private const val CHANNEL_GROUPS = "groups"
  private const val CHANNEL_SERVICE = "service"

  const val EXTRA_TYPE = "veydan_push_type"
  const val EXTRA_CHAT = "veydan_push_chat"

  /** Notifications are told apart by their tag; the number is the same for all. */
  private const val ID = 1
  private const val GROUP = "net.veydan.messages"
  private const val TAG_SUMMARY = "summary"
  /** A direct message the phone could not open is one for all of them. */
  private const val TAG_DM_PLAIN = "dm"
  /** Messages the server only counted: one for all of them too. */
  private const val TAG_MORE = "more"
  private const val EXTRA_COUNT = "veydan_push_count"
  /** The album the last line of a chat tells of: its batch, whether of files, how many, its caption. */
  private const val EXTRA_ALBUM = "veydan_push_album"
  private const val EXTRA_ALBUM_FILES = "veydan_push_album_files"
  private const val EXTRA_ALBUM_N = "veydan_push_album_n"
  private const val EXTRA_ALBUM_CAPTION = "veydan_push_album_caption"
  private const val MAX_LINES = 25

  /** Channels are what the user sees in the system settings, one switch each. */
  fun ensureChannels(context: Context) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    manager.createNotificationChannels(
      listOf(
        NotificationChannel(CHANNEL_DM, context.getString(R.string.veydan_push_channel_dm), NotificationManager.IMPORTANCE_HIGH),
        NotificationChannel(CHANNEL_GROUPS, context.getString(R.string.veydan_push_channel_groups), NotificationManager.IMPORTANCE_HIGH),
        NotificationChannel(CHANNEL_SERVICE, context.getString(R.string.veydan_push_channel_service), NotificationManager.IMPORTANCE_DEFAULT),
      )
    )
  }

  fun allowed(context: Context): Boolean =
    NotificationManagerCompat.from(context).areNotificationsEnabled()

  /** A message with everything known about it. False when the system would not show it. */
  fun show(context: Context, notice: Notice): Boolean {
    if (!allowed(context)) return false
    ensureChannels(context)

    val tag = notice.chat ?: TAG_DM_PLAIN
    val group = notice.kind == "group"
    val face = IconCompat.createWithBitmap(Avatar.of(context, notice.sender, notice.senderKey, notice.picture))
    val sender = Person.Builder()
      .setName(notice.sender)
      .setKey(notice.senderKey)
      .setIcon(face)
      .build()
    val me = Person.Builder().setName(context.getString(R.string.veydan_push_me)).build()

    // The lines already on the screen for this chat stay; the new one joins
    // them. One more of an album that is coming joins its line instead,
    // which then says how many there are.
    val before = shown(context, tag)
    val style = before?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it) }
      ?: NotificationCompat.MessagingStyle(me)
    style.setGroupConversation(group)
    if (group) style.setConversationTitle(notice.title)
    val next = Album.of(notice.body, notice.count)
    val joined = next?.let { Album.last(before)?.join(it) }
    if (joined != null && style.messages.isNotEmpty()) style.messages.removeAt(style.messages.size - 1)
    val album = joined ?: next
    val text = if (album != null && album.n > 1) request(context, notice) + album.line(context) else line(context, notice)
    style.addMessage(text, System.currentTimeMillis(), sender)
    while (style.messages.size > MAX_LINES) style.messages.removeAt(0)

    val builder = NotificationCompat.Builder(context, if (group) CHANNEL_GROUPS else CHANNEL_DM)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setStyle(style)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setGroup(GROUP)
      .setAutoCancel(true)
      .setSilent(notice.muted)
      .setContentIntent(open(context, if (group) Push.TYPE_GROUP else Push.TYPE_DM, notice.chat))
      // What the last line is an album of, for the next of its files.
      .addExtras(album?.bundle() ?: Bundle())
    // A chat of its own in the system's eyes: its face becomes the
    // notification's, and it goes to the "Conversations" section.
    notice.chat?.let { chat ->
      val icon = if (group) IconCompat.createWithBitmap(Avatar.initials(notice.title, chat)) else face
      if (conversation(context, chat, notice.title, if (group) null else sender, icon, group)) builder.setShortcutId(chat)
    }
    if (notice.hideOnLockscreen) {
      builder
        .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
        .setPublicVersion(plainBuilder(context, PlainNotice(notice.kind, notice.chat, null, notice.muted, 1)).build())
    }
    return post(context, tag, builder) && summary(context)
  }

  /**
   * The chat as a long-lived shortcut: what Android wants before it shows a
   * notification as a conversation, with the chat's face instead of the
   * app's. False when the system would not take it.
   */
  private fun conversation(context: Context, chat: String, title: String, person: Person?, icon: IconCompat, group: Boolean): Boolean {
    val intent = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return false
    intent.putExtra(EXTRA_TYPE, if (group) Push.TYPE_GROUP else Push.TYPE_DM)
    intent.putExtra(EXTRA_CHAT, chat)
    val shortcut = ShortcutInfoCompat.Builder(context, chat)
      .setShortLabel(title.ifBlank { "Veydan" })
      .setLongLived(true)
      .setIcon(icon)
      .setIntent(intent)
      .apply { person?.let { setPerson(it) } }
      .build()
    return try {
      ShortcutManagerCompat.pushDynamicShortcut(context, shortcut)
      true
    } catch (e: Exception) {
      false
    }
  }

  /** Something came, and that is all: the phone has no key, or may say no more. */
  fun showPlain(context: Context, plain: PlainNotice): Boolean {
    if (!allowed(context)) return false
    ensureChannels(context)
    val tag = plain.chat ?: TAG_DM_PLAIN
    return post(context, tag, plainBuilder(context, plain)) && summary(context)
  }

  /**
   * More came than the server pushes one by one: how many, and nothing
   * else is known. Counted together with what such a notification already
   * says. It comes without a sound: the phone has rung enough by then, and
   * whether those chats are muted nobody here can tell.
   */
  fun showMore(context: Context, count: Int): Boolean {
    if (!allowed(context)) return false
    ensureChannels(context)
    val before = shown(context, TAG_MORE)?.extras?.getInt(EXTRA_COUNT) ?: 0
    val total = (before + count).coerceAtMost(9999)
    val title = if (total > 1) context.getString(R.string.veydan_push_many, total)
      else context.getString(R.string.veydan_push_one)
    val builder = NotificationCompat.Builder(context, CHANNEL_DM)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setContentTitle(title)
      .setNumber(total)
      .addExtras(Bundle().apply { putInt(EXTRA_COUNT, total) })
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setGroup(GROUP)
      .setAutoCancel(true)
      .setSilent(true)
      .setContentIntent(open(context, Push.TYPE_SYNC, null))
    return post(context, TAG_MORE, builder) && summary(context)
  }

  /** A word from the server itself: a test, a broadcast. */
  fun showService(context: Context, push: Push): Boolean {
    if (!allowed(context)) return false
    ensureChannels(context)
    val newer = push.version > Push.VERSION
    val (title, body) = when {
      newer -> context.getString(R.string.veydan_push_update_title) to context.getString(R.string.veydan_push_update_body)
      push.type == Push.TYPE_TEST -> context.getString(R.string.veydan_push_test_title) to context.getString(R.string.veydan_push_test_body)
      else -> context.getString(R.string.veydan_push_service_title) to (push.data ?: "")
    }
    val builder = NotificationCompat.Builder(context, CHANNEL_SERVICE)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setContentTitle(title)
      .setContentText(body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setCategory(NotificationCompat.CATEGORY_STATUS)
      .setAutoCancel(true)
      .setContentIntent(open(context, push.type, null))
    return post(context, "service:${push.trace}", builder)
  }

  private fun plainBuilder(context: Context, plain: PlainNotice): NotificationCompat.Builder {
    val group = plain.kind == "group"
    val title = when {
      group -> plain.title ?: context.getString(R.string.veydan_push_group)
      else -> context.getString(R.string.veydan_push_dm_title)
    }
    val body = if (plain.count > 1) context.getString(R.string.veydan_push_many, plain.count)
      else context.getString(R.string.veydan_push_one)
    return NotificationCompat.Builder(context, if (group) CHANNEL_GROUPS else CHANNEL_DM)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setContentTitle(title)
      .setContentText(body)
      .setNumber(plain.count)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setGroup(GROUP)
      .setAutoCancel(true)
      .setSilent(plain.muted)
      .setContentIntent(open(context, if (group) Push.TYPE_GROUP else Push.TYPE_DM, plain.chat))
  }

  /** "Message request · " before whatever a stranger sent. */
  private fun request(context: Context, notice: Notice): String =
    if (notice.kind == "request") context.getString(R.string.veydan_push_request) + " · " else ""

  /**
   * The one line of a message, in this phone's words: the same line the card
   * in the app makes (`push/wording.ts`) and a computer (`desktop_notify.rs`).
   */
  private fun line(context: Context, notice: Notice): String {
    val more = if (notice.count > 1) " " + context.getString(R.string.veydan_push_more, notice.count - 1) else ""
    val body = notice.body ?: return context.getString(R.string.veydan_push_one) + more
    val text = when (body) {
      is Body.Text -> body.text
      is Body.Link -> when (body.link) {
        "group" -> "🔗 " + (body.title?.let { context.getString(R.string.veydan_push_link_group, it) }
          ?: context.getString(R.string.veydan_push_link_group_nameless))
        "contact" -> "👤 " + (body.title?.let { context.getString(R.string.veydan_push_link_contact, it) }
          ?: context.getString(R.string.veydan_push_link_contact_nameless))
        else -> "🔗 " + (body.title ?: "")
      }
      is Body.Media -> media(context, body)
      is Body.Invite -> context.getString(R.string.veydan_push_invite, body.groupName)
      is Body.JoinRequest -> context.getString(R.string.veydan_push_join_request, body.groupName)
      is Body.Welcome -> context.getString(R.string.veydan_push_welcome, body.groupName)
    }
    return request(context, notice) + text + more
  }

  /** Files are told by their name; a picture, a video, a recording by what it is. */
  private fun media(context: Context, body: Body.Media): String {
    val (emoji, word) = when (body.kind) {
      "image" -> "📷" to R.string.veydan_push_photo
      "video" -> "🎬" to R.string.veydan_push_video
      "voice" -> "🎤" to R.string.veydan_push_voice
      "circle" -> "⭕" to R.string.veydan_push_circle
      "audio" -> "🎵" to R.string.veydan_push_audio
      else -> "📎" to R.string.veydan_push_file
    }
    if (word == R.string.veydan_push_file || word == R.string.veydan_push_audio) {
      val name = body.name ?: context.getString(word)
      return "$emoji " + (body.caption?.let { "$name · $it" } ?: name)
    }
    body.caption?.let { return "$emoji $it" }
    val length = body.durationMs?.let { " (${duration(it)})" } ?: ""
    return "$emoji ${context.getString(word)}$length"
  }

  /** `12400` → `0:12`; an hour and more as `1:02:03`. */
  private fun duration(ms: Long): String {
    val s = (ms + 500) / 1000
    val (h, m, sec) = Triple(s / 3600, s % 3600 / 60, s % 60)
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
  }

  /** Files sent together, as far as they came: an album, or a few documents. */
  private data class Album(val batch: String, val files: Boolean, val n: Int, val caption: String?) {
    fun join(next: Album): Album? =
      if (next.batch != batch) null else copy(files = files && next.files, n = n + next.n, caption = caption ?: next.caption)

    fun line(context: Context): String {
      val head = if (files) "📎 " + context.getString(R.string.veydan_push_files, n)
        else "🖼 " + context.getString(R.string.veydan_push_album, n)
      return caption?.let { "$head · $it" } ?: head
    }

    fun bundle() = Bundle().apply {
      putString(EXTRA_ALBUM, batch)
      putBoolean(EXTRA_ALBUM_FILES, files)
      putInt(EXTRA_ALBUM_N, n)
      caption?.let { putString(EXTRA_ALBUM_CAPTION, it) }
    }

    companion object {
      /**
       * The album a message belongs to. A push that stands for several
       * events names the last of them: the ones before it came in the same
       * moment, and are taken as files of the same album.
       */
      fun of(body: Body?, count: Int): Album? {
        if (body !is Body.Media || body.batch == null || body.kind == "voice" || body.kind == "circle") return null
        return Album(body.batch, body.kind == "file" || body.kind == "audio", count, body.caption)
      }

      /** What the last line of a notification on the screen is an album of. */
      fun last(shown: android.app.Notification?): Album? {
        val extras = shown?.extras ?: return null
        val batch = extras.getString(EXTRA_ALBUM) ?: return null
        return Album(batch, extras.getBoolean(EXTRA_ALBUM_FILES), extras.getInt(EXTRA_ALBUM_N, 1), extras.getString(EXTRA_ALBUM_CAPTION))
      }
    }
  }

  private fun shown(context: Context, tag: String): android.app.Notification? {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return null
    return manager.activeNotifications.firstOrNull { it.tag == tag && it.id == ID }?.notification
  }

  /** The one notification the group of them collapses into. */
  private fun summary(context: Context): Boolean {
    val builder = NotificationCompat.Builder(context, CHANNEL_DM)
      .setSmallIcon(R.drawable.ic_stat_veydan)
      .setGroup(GROUP)
      .setGroupSummary(true)
      // The summary never makes a sound: each chat decides for itself. A
      // muted chat's notification hands its alert to the summary, and a
      // summary that took it would ring for a chat the user silenced.
      .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
      .setOnlyAlertOnce(true)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setAutoCancel(true)
      .setContentIntent(open(context, Push.TYPE_DM, null))
    return post(context, TAG_SUMMARY, builder)
  }

  private fun post(context: Context, tag: String, builder: NotificationCompat.Builder): Boolean = try {
    NotificationManagerCompat.from(context).notify(tag, ID, builder.build())
    true
  } catch (e: SecurityException) {
    // The permission was taken away between the check and the call.
    false
  }

  /** The app's own start, with a note of what was tapped. */
  private fun open(context: Context, type: String, chat: String?): PendingIntent? {
    val intent = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return null
    intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    intent.putExtra(EXTRA_TYPE, type)
    chat?.let { intent.putExtra(EXTRA_CHAT, it) }
    return PendingIntent.getActivity(
      context,
      (chat ?: type).hashCode(),
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
  }

  /** A tag the app may ask to clear: a chat, or the one for direct messages it could not tell apart. */
  fun isClearable(key: String): Boolean = Push.isChatKey(key) || key == TAG_DM_PLAIN

  /** A notification about messages, as opposed to the summary and to the server's own words. */
  private fun aboutMessages(tag: String?): Boolean =
    tag != null && (Push.isChatKey(tag) || tag == TAG_DM_PLAIN || tag == TAG_MORE)

  /**
   * Removes the notification of one chat (`dm:<pubkey>`, `group:<id>`), or
   * every notification about messages. The summary goes when nothing is
   * left under it.
   */
  fun cancel(context: Context, key: String?) {
    val manager = context.getSystemService(NotificationManager::class.java) ?: return
    if (key != null) {
      manager.cancel(key, ID)
      // A direct chat opened is also the place a message the phone could
      // not open was about.
      if (key.startsWith("dm:")) manager.cancel(TAG_DM_PLAIN, ID)
      // The list of chats shows which of them have something new: all the
      // count of messages nobody named could say.
      if (key == TAG_DM_PLAIN) manager.cancel(TAG_MORE, ID)
    } else {
      for (shown in manager.activeNotifications) {
        if (aboutMessages(shown.tag)) manager.cancel(shown.tag, shown.id)
      }
    }
    val left = manager.activeNotifications.any { aboutMessages(it.tag) }
    if (!left) manager.cancel(TAG_SUMMARY, ID)
  }

  /** What a tap carried, if the intent is one of ours. The note is removed: it is read once. */
  fun readTap(intent: Intent?): Tap? {
    val type = intent?.getStringExtra(EXTRA_TYPE) ?: return null
    val chat = intent.getStringExtra(EXTRA_CHAT)?.takeIf { Push.isChatKey(it) }
    intent.removeExtra(EXTRA_TYPE)
    intent.removeExtra(EXTRA_CHAT)
    return Tap(type.take(32), chat)
  }
}

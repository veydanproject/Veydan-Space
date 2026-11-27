// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.core.content.IntentCompat
import java.io.File
import java.io.FileOutputStream
import org.json.JSONArray
import org.json.JSONObject

/**
 * «Поделиться» into Veydan from another app: the system's share sheet
 * (`ACTION_SEND`, `ACTION_SEND_MULTIPLE`; a chat of Direct Share with
 * `EXTRA_SHORTCUT_ID`) starts this activity with a text, files, or both
 * (crates/shell/src/shares.rs is the Rust side, the app's page picks the
 * chat). A product declares it in its manifest.
 *
 * The files come as `content://` URIs the system lets this activity read
 * while it lives, and only it: they are copied into
 * `cacheDir/share-inbox/<stamp>/` first, off the main thread, and the app is
 * started once the copies are made. The share itself (the text, the paths of
 * the copies) goes to `IncomingShares`, in memory only, as a link goes to
 * `IncomingLinks`: the intent that carries it never becomes the base intent
 * of a task the system keeps. The folder is deleted by the app once the share
 * is sent or given up (`shares_forget`); one the app never took is deleted
 * here at the next share, a day later.
 */
class ShareActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // A restored instance has nothing of its own to give: its grants are gone.
    if (savedInstanceState != null) {
      finishAndRemoveTask()
      return
    }
    val came = intent
    val streams = streamsOf(came)
    val text = ShareRules.textOf(came?.getCharSequenceExtra(Intent.EXTRA_TEXT))
    val subject = ShareRules.subjectOf(came?.getCharSequenceExtra(Intent.EXTRA_SUBJECT), text)
    val target = came?.getStringExtra(Intent.EXTRA_SHORTCUT_ID)?.takeIf { it.isNotBlank() }
    val kind = came?.type
    intent = Intent()
    if (streams.isEmpty() && text == null) {
      finishAndRemoveTask()
      return
    }
    val app = applicationContext
    Thread({
      val share = Inbox(app.cacheDir).take(this, streams, kind, text, subject, target)
      runOnUiThread {
        if (share != null) {
          IncomingShares.put(share.toString())
          IncomingShares.tell()
        }
        app.packageManager.getLaunchIntentForPackage(app.packageName)?.let { launch ->
          launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
          app.startActivity(launch)
        }
        finishAndRemoveTask()
      }
    }, "veydan-share").start()
  }

  /** The files of the share, `MAX_FILES` at most: one of SEND, the list of SEND_MULTIPLE. */
  private fun streamsOf(intent: Intent?): List<Uri> {
    if (intent == null) return emptyList()
    return when (intent.action) {
      Intent.ACTION_SEND -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
      Intent.ACTION_SEND_MULTIPLE ->
        IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)?.filterNotNull()?.take(ShareRules.MAX_FILES)
          ?: emptyList()
      else -> emptyList()
    }
  }
}

/**
 * The inbox of shared files in the app's cache: the copies of one share in
 * a folder of their own, named by the moment it came.
 */
internal class Inbox(cacheDir: File) {
  private val root = File(cacheDir, ShareRules.INBOX)

  /**
   * The share as the Rust side reads it (`shares.rs`, `Share`): the copies of
   * `streams` made, the names of those that could not be copied (too large,
   * gone before they were read) under `refused`. Null when nothing came of it.
   */
  fun take(activity: Activity, streams: List<Uri>, kind: String?, text: String?, subject: String?, target: String?): JSONObject? {
    val now = System.currentTimeMillis()
    sweep(now)
    val id = ShareRules.folderName(now)
    val dir = File(root, id)
    val files = JSONArray()
    val refused = JSONArray()
    if (streams.isNotEmpty()) dir.mkdirs()
    val resolver = activity.contentResolver
    val types = MimeTypeMap.getSingleton()
    for ((i, uri) in streams.withIndex()) {
      var display: String? = null
      try {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
          if (c.moveToFirst()) display = c.getString(0)
        }
      } catch (_: Exception) {
      }
      val mime = ShareRules.mimeOf(runCatching { resolver.getType(uri) }.getOrNull(), kind)
      val name = ShareRules.fileName(display ?: uri.lastPathSegment, mime, i) { types.getExtensionFromMimeType(it) }
      val out = free(dir, name)
      val size = copy(resolver, uri, out)
      if (size == null) {
        out.delete()
        refused.put(name)
        continue
      }
      files.put(JSONObject().put("path", out.absolutePath).put("name", out.name).put("mime", mime ?: JSONObject.NULL).put("size", size))
    }
    if (files.length() == 0 && dir.isDirectory) dir.deleteRecursively()
    if (files.length() == 0 && text == null) return null
    return JSONObject()
      .put("id", id)
      .put("text", text ?: JSONObject.NULL)
      .put("subject", subject ?: JSONObject.NULL)
      .put("target", target ?: JSONObject.NULL)
      .put("files", files)
      .put("refused", refused)
  }

  /** `name` in `dir`, or `name (2)`, `name (3)`… when files of a share share a name. */
  private fun free(dir: File, name: String): File {
    var f = File(dir, name)
    var n = 2
    while (f.exists()) {
      val dot = name.lastIndexOf('.')
      val stem = if (dot > 0) name.substring(0, dot) else name
      val ext = if (dot > 0) name.substring(dot) else ""
      f = File(dir, "$stem ($n)$ext")
      n++
    }
    return f
  }

  /** The bytes of `uri` into `out`; how many, or null when they could not be read or there were more than `MAX_FILE_BYTES`. */
  private fun copy(resolver: android.content.ContentResolver, uri: Uri, out: File): Long? {
    return try {
      resolver.openInputStream(uri)?.use { input ->
        FileOutputStream(out).use { output ->
          val buf = ByteArray(256 * 1024)
          var total = 0L
          while (true) {
            val n = input.read(buf)
            if (n < 0) break
            total += n
            if (total > ShareRules.MAX_FILE_BYTES) return null
            output.write(buf, 0, n)
          }
          total
        }
      }
    } catch (_: Exception) {
      null
    }
  }

  /** The folders no app took in a day go. */
  private fun sweep(now: Long) {
    root.listFiles()?.forEach { f -> if (ShareRules.stale(f.name, now)) f.deleteRecursively() }
  }
}

/** The share that waits for the Rust side (JSON of `Inbox.take`): the last one, in memory only. */
object IncomingShares {
  @Volatile private var pending: String? = null

  @Synchronized
  fun put(share: String) {
    pending = share
  }

  /**
   * Tells the running app a share waits, without it: the page asks for it
   * (`take`). Not loaded yet on a cold start, when the app takes the share
   * as it starts.
   */
  fun tell() {
    try {
      arrived()
    } catch (_: UnsatisfiedLinkError) {
    }
  }

  /** `Java_net_veydan_shell_IncomingShares_arrived` of crates/shell/src/shares.rs. */
  @JvmStatic private external fun arrived()

  /** The share that waits, taken: the next call gets nothing until another comes. */
  @JvmStatic
  @Synchronized
  fun take(): String? {
    val share = pending
    pending = null
    return share
  }
}

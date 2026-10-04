// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Shader
import android.util.Log
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * The face of the sender: their picture when they have one, the circle with
 * initials the app draws otherwise.
 *
 * A picture is an address on somebody's server. It is fetched once, briefly
 * (a push has seconds, not minutes), cut to a circle and kept in the app's
 * cache, so the next message from the same person costs nothing. The app
 * fetches the same address when it shows the chat list; the phone learns
 * nothing new by fetching it here.
 */
internal object Avatar {
  private const val SIZE = 192
  private const val DIR = "push-avatars"
  private const val TIMEOUT_MS = 2500
  private const val MAX_BYTES = 2 * 1024 * 1024
  private const val KEEP = 200

  fun of(context: Context, label: String, seed: String, picture: String?): Bitmap {
    picture?.let { url -> picture(context, url)?.let { return it } }
    return initials(label, seed)
  }

  private fun picture(context: Context, url: String): Bitmap? {
    if (!url.startsWith("https://")) return null
    val dir = File(context.cacheDir, DIR)
    val file = File(dir, sha256(url) + ".png")
    if (file.exists()) {
      BitmapFactory.decodeFile(file.absolutePath)?.let { return it }
      file.delete()
    }
    val raw = download(url) ?: return null
    val circle = circle(raw) ?: return null
    try {
      dir.mkdirs()
      file.outputStream().use { circle.compress(Bitmap.CompressFormat.PNG, 100, it) }
      prune(dir)
    } catch (e: Exception) {
      // Not kept this time: the next push fetches it again.
    }
    return circle
  }

  private fun download(address: String): Bitmap? = try {
    val connection = URL(address).openConnection() as HttpURLConnection
    connection.connectTimeout = TIMEOUT_MS
    connection.readTimeout = TIMEOUT_MS
    connection.instanceFollowRedirects = true
    try {
      if (connection.responseCode != 200) null
      else if (connection.url.protocol != "https") null
      else {
        val bytes = connection.inputStream.use { input ->
          val out = java.io.ByteArrayOutputStream()
          val buffer = ByteArray(16 * 1024)
          while (true) {
            val n = input.read(buffer)
            if (n < 0) break
            out.write(buffer, 0, n)
            if (out.size() > MAX_BYTES) return@use null
          }
          out.toByteArray()
        }
        bytes?.let { decode(it) }
      }
    } finally {
      connection.disconnect()
    }
  } catch (e: Exception) {
    Log.i(PushState.TAG, "the sender's picture did not come: ${e.javaClass.simpleName}")
    null
  }

  /** Decoded no larger than needed: a notification shows it at 48 dp. */
  private fun decode(bytes: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= SIZE && bounds.outHeight / (sample * 2) >= SIZE) sample *= 2
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
  }

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

  /** The oldest pictures go when there are too many. */
  private fun prune(dir: File) {
    val files = dir.listFiles() ?: return
    if (files.size <= KEEP) return
    files.sortedBy { it.lastModified() }.take(files.size - KEEP).forEach { it.delete() }
  }

  private fun sha256(s: String): String =
    MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

  /** As the app: two letters, a hue from the key. */
  fun initials(label: String, seed: String): Bitmap {
    val bitmap = Bitmap.createBitmap(SIZE, SIZE, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(bitmap)
    val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    paint.color = Color.HSVToColor(floatArrayOf(hue(seed.ifEmpty { label }), 0.55f, 0.72f))
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

  /** First letters of two words, or two of one; keys get a glyph. */
  fun letters(label: String): String {
    val s = label.trim()
    if (s.isEmpty()) return "?"
    if (s.startsWith("npub1", ignoreCase = true) || Regex("^[0-9a-f]{16,}", RegexOption.IGNORE_CASE).containsMatchIn(s)) return "#"
    val words = s.split(Regex("\\s+")).filter { it.isNotEmpty() }
    val pick = if (words.size > 1) first(words[0]) + first(words[1])
      else words[0].codePoints().limit(2).toArray().let { String(it, 0, it.size) }
    return pick.uppercase()
  }

  private fun first(word: String): String = String(Character.toChars(word.codePointAt(0)))

  /** The app's hash of the seed, so the colour matches the chat list. */
  private fun hue(seed: String): Float {
    var h = 0L
    for (ch in seed) h = (h * 31 + ch.code) and 0xffffffffL
    return (h % 360).toFloat()
  }
}

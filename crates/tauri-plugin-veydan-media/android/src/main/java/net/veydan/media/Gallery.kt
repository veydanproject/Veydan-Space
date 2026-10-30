// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.MediaStore
import android.util.Size
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest

// Columns of the table of files of MediaStore that the constants of
// MediaColumns name only from Android 10, though the table has had them
// since long before.
private const val DURATION = "duration"
private const val BUCKET_ID = "bucket_id"
private const val BUCKET_NAME = "bucket_display_name"

/** One photo or video of the grid. */
data class GalleryItem(
  val id: Long,
  val video: Boolean,
  /** The `content://` address `importPicked` reads. */
  val uri: String,
  val added: Long,
  val width: Int,
  val height: Int,
  /** Milliseconds, for a video. */
  val durationMs: Long,
)

data class GalleryPage(val items: List<GalleryItem>, val next: String?)

data class GalleryAlbum(val id: String, val title: String, val count: Int)

/**
 * The phone's photos and videos through MediaStore: pages of the newest,
 * by a cursor, and the albums (buckets). Only the rows are read, never a
 * file: a page of 100 costs one query. With a partial access (Android 14)
 * MediaStore itself gives only what the user chose.
 */
class Gallery(private val context: Context) {
  private val resolver: ContentResolver get() = context.contentResolver
  private val files: Uri = MediaStore.Files.getContentUri("external")

  private fun kindSelection(kind: String?): String {
    val image = MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
    val video = MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
    val type = MediaStore.Files.FileColumns.MEDIA_TYPE
    return when (kind) {
      "image" -> "$type = $image"
      "video" -> "$type = $video"
      else -> "$type IN ($image, $video)"
    }
  }

  fun page(cursor: String?, limit: Int, kind: String?, album: String?): GalleryPage {
    val added = MediaStore.MediaColumns.DATE_ADDED
    val id = MediaStore.MediaColumns._ID
    val where = StringBuilder(kindSelection(kind))
    val args = ArrayList<String>()
    if (!album.isNullOrEmpty()) {
      where.append(" AND $BUCKET_ID = ?")
      args.add(album)
    }
    PageCursor.of(cursor)?.let {
      where.append(" AND ").append(it.selection(added, id))
      args.addAll(it.args())
    }
    val n = pageLimit(limit)
    val projection = arrayOf(
      id,
      MediaStore.Files.FileColumns.MEDIA_TYPE,
      added,
      MediaStore.MediaColumns.WIDTH,
      MediaStore.MediaColumns.HEIGHT,
      DURATION,
    )
    val order = "$added DESC, $id DESC"
    val items = ArrayList<GalleryItem>(n)
    var more = false
    // One past the page tells whether another follows. Android 11+ takes the
    // bound as an argument and refuses it in the order; before, the other way.
    val rows = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      val query = Bundle().apply {
        putString(ContentResolver.QUERY_ARG_SQL_SELECTION, where.toString())
        putStringArray(ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS, args.toTypedArray())
        putString(ContentResolver.QUERY_ARG_SQL_SORT_ORDER, order)
        putInt(ContentResolver.QUERY_ARG_LIMIT, n + 1)
      }
      resolver.query(files, projection, query, null)
    } else {
      resolver.query(files, projection, where.toString(), args.toTypedArray(), "$order LIMIT ${n + 1}")
    }
    rows?.use { c ->
      val cId = c.getColumnIndexOrThrow(id)
      val cType = c.getColumnIndexOrThrow(MediaStore.Files.FileColumns.MEDIA_TYPE)
      val cAdded = c.getColumnIndexOrThrow(added)
      val cW = c.getColumnIndexOrThrow(MediaStore.MediaColumns.WIDTH)
      val cH = c.getColumnIndexOrThrow(MediaStore.MediaColumns.HEIGHT)
      val cD = c.getColumnIndexOrThrow(DURATION)
      while (c.moveToNext()) {
        if (items.size == n) {
          more = true
          break
        }
        val rowId = c.getLong(cId)
        val video = c.getInt(cType) == MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
        val base = if (video) MediaStore.Video.Media.EXTERNAL_CONTENT_URI else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        items.add(
          GalleryItem(
            id = rowId,
            video = video,
            uri = ContentUris.withAppendedId(base, rowId).toString(),
            added = c.getLong(cAdded),
            width = c.getInt(cW),
            height = c.getInt(cH),
            durationMs = if (video) c.getLong(cD) else 0,
          )
        )
      }
    }
    val last = items.lastOrNull()
    return GalleryPage(items, if (more && last != null) PageCursor(last.added, last.id).word() else null)
  }

  /** The albums, the largest first; one pass over two columns of the rows. */
  fun albums(): List<GalleryAlbum> {
    val bucket = BUCKET_ID
    val name = BUCKET_NAME
    val counts = LinkedHashMap<String, Pair<String, Int>>()
    resolver.query(files, arrayOf(bucket, name), kindSelection(null), null, null)?.use { c ->
      val cB = c.getColumnIndexOrThrow(bucket)
      val cN = c.getColumnIndexOrThrow(name)
      while (c.moveToNext()) {
        val b = c.getString(cB) ?: continue
        val prev = counts[b]
        counts[b] = Pair(prev?.first ?: (c.getString(cN) ?: ""), (prev?.second ?: 0) + 1)
      }
    }
    return counts.map { (id, v) -> GalleryAlbum(id, v.first, v.second) }.sortedByDescending { it.count }
  }
}

/** The app may no longer read the item: `gone` are the files of its thumbnails, deleted. */
class NoAccess(val gone: List<File>) : Exception("not_accessible")

/**
 * Thumbnails of the grid as JPEG files in `cacheDir/attach-thumbs/`, which
 * the app's server of files hands to the page by a name of its own pool:
 * never base64. At most about 50 MB on the disk, the least recently shown
 * going first; the folder is emptied when the app starts.
 */
class Thumbs(context: Context) {
  private val resolver = context.contentResolver
  val dir = File(context.cacheDir, "attach-thumbs")
  private val lru = ThumbLru(limit = 50L * 1024 * 1024, low = 40L * 1024 * 1024)

  /**
   * Empties the folder (the start of the app): it is moved aside at once,
   * so a thumbnail made right after is never deleted with it, and what was
   * moved goes in `later` (off the main thread).
   */
  fun clear(later: (Runnable) -> Unit) {
    lru.clear()
    val old = File(dir.parentFile, "${dir.name}.old-${System.nanoTime()}")
    if (dir.exists() && !dir.renameTo(old)) dir.deleteRecursively()
    dir.mkdirs()
    later(Runnable {
      old.deleteRecursively()
      // What an earlier start moved aside and did not finish.
      dir.parentFile?.listFiles { f -> f.name.startsWith("${dir.name}.old-") }?.forEach { it.deleteRecursively() }
    })
  }

  /**
   * The file of the thumbnail of `uri`, `side` pixels at most; made when it
   * is not there. MediaStore is asked first whether the app may read the
   * item now, kept or not: a partial access the user changed (another set
   * of photos, videos in place of photos) leaves no thumbnail of what it no
   * longer lets in. Then every one kept of the item is deleted, and
   * `NoAccess` names their files (the server of files forgets them).
   */
  fun of(uri: String, side: Int): File {
    val item = Uri.parse(uri)
    if (!readable(item)) throw NoAccess(drop(uri))
    val px = thumbSide(side)
    val name = nameOf(uri, px)
    val file = File(dir, name)
    if (lru.touch(name) && file.isFile) return file
    dir.mkdirs()
    val bitmap = load(item, px) ?: throw IllegalStateException("no thumbnail")
    // Two asks of the same tile at once write apart; the last to finish stands.
    val part = File(dir, "$name.${Thread.currentThread().id}.part")
    try {
      FileOutputStream(part).use { bitmap.compress(Bitmap.CompressFormat.JPEG, 82, it) }
    } finally {
      bitmap.recycle()
    }
    if (!part.renameTo(file)) {
      part.delete()
      throw IllegalStateException("thumbnail not written")
    }
    for (old in lru.add(name, file.length(), uri)) File(dir, old).delete()
    return file
  }

  /** Every thumbnail kept of `uri` deleted (the app may no longer read it); their files. */
  fun drop(uri: String): List<File> {
    val gone = lru.forget(uri).map { File(dir, it) }
    for (f in gone) f.delete()
    return gone
  }

  /**
   * Whether MediaStore gives the row of `uri` to the app now: with a
   * partial access only what the user chose has one; without the
   * permission of its kind the query is refused.
   */
  fun readable(uri: Uri): Boolean = try {
    resolver.query(uri, arrayOf(MediaStore.MediaColumns._ID), null, null, null)?.use { it.count > 0 } ?: false
  } catch (e: SecurityException) {
    false
  } catch (e: IllegalArgumentException) {
    false
  }

  private fun load(uri: Uri, px: Int): Bitmap? {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      return resolver.loadThumbnail(uri, Size(px, px), null)
    }
    // Android 8 and 9: the old thumbnails of MediaStore, else a sampled decode.
    val id = ContentUris.parseId(uri)
    val video = uri.toString().startsWith(MediaStore.Video.Media.EXTERNAL_CONTENT_URI.toString())
    @Suppress("DEPRECATION")
    val old = if (video) {
      MediaStore.Video.Thumbnails.getThumbnail(resolver, id, MediaStore.Video.Thumbnails.MINI_KIND, null)
    } else {
      MediaStore.Images.Thumbnails.getThumbnail(resolver, id, MediaStore.Images.Thumbnails.MINI_KIND, null)
    }
    if (old != null || video) return old
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, bounds) }
    var sample = 1
    while (bounds.outWidth / (sample * 2) >= px && bounds.outHeight / (sample * 2) >= px) sample *= 2
    val opts = BitmapFactory.Options().apply { inSampleSize = sample }
    return resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, opts) }
  }

  private fun nameOf(uri: String, px: Int): String {
    val digest = MessageDigest.getInstance("SHA-256").digest("$uri|$px".toByteArray())
    return digest.take(16).joinToString("") { "%02x".format(it) } + ".jpg"
  }
}

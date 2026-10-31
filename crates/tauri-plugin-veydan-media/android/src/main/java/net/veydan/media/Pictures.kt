// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

import android.content.ContentValues
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import android.media.MediaScannerConnection
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import java.io.File
import java.io.FileOutputStream

/**
 * A photo of a format not every client shows (HEIC/HEIF) written anew as a
 * JPEG by the phone's own decoder (platform-spec, «Общие форматы
 * вложений»): `ImageDecoder` turns it upright as its camera said and gives
 * pixels alone, so nothing of the source (place, phone, time) reaches the
 * JPEG, which `Bitmap.compress` writes without EXIF. A photo larger than
 * `maxPixels` is decoded smaller to it. The app calls this one photo at a
 * time (`PHOTO_SLOTS`): a decoded photo is held whole in memory.
 */
object Pictures {
  fun toJpeg(src: File, dst: File, maxPixels: Long, quality: Int) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) {
      throw UnsupportedOperationException("no ImageDecoder before Android 9")
    }
    val source = ImageDecoder.createSource(src)
    val bitmap = ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
      // In memory the app reads, of 8 bits in sRGB: a JPEG is no more, and a 10-bit HDR photo is not a 64-bit picture.
      decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
      decoder.setTargetColorSpace(ColorSpace.get(ColorSpace.Named.SRGB))
      val (w, h) = targetSize(info.size.width, info.size.height, maxPixels)
      if (w != info.size.width || h != info.size.height) decoder.setTargetSize(w, h)
    }
    try {
      FileOutputStream(dst).use { out ->
        if (!bitmap.compress(Bitmap.CompressFormat.JPEG, quality.coerceIn(1, 100), out)) {
          throw IllegalStateException("the JPEG was not written")
        }
        out.fd.sync()
      }
    } catch (e: Exception) {
      dst.delete()
      throw e
    } finally {
      bitmap.recycle()
    }
  }

  /** The album of the gallery the app's own pictures go to: Pictures/Veydan. */
  const val ALBUM = "Veydan"

  /**
   * A PNG the app made (a QR code of «Сохранить», ROO-297) into the
   * gallery, album `ALBUM`, as `name`. Android 10+ writes it through
   * MediaStore, which names a second one of the same name itself; before
   * that it is a file of the public Pictures (the caller has the permission
   * to write) that the media scanner is told about.
   */
  fun saveToGallery(context: Context, png: ByteArray, name: String) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      val resolver = context.contentResolver
      val values = ContentValues().apply {
        put(MediaStore.Images.Media.DISPLAY_NAME, name)
        put(MediaStore.Images.Media.MIME_TYPE, "image/png")
        put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/$ALBUM")
        put(MediaStore.Images.Media.IS_PENDING, 1)
      }
      val uri = resolver.insert(MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values)
        ?: throw IllegalStateException("the gallery took no picture")
      try {
        val out = resolver.openOutputStream(uri) ?: throw IllegalStateException("the picture was not written")
        out.use { it.write(png) }
        values.clear()
        values.put(MediaStore.Images.Media.IS_PENDING, 0)
        resolver.update(uri, values, null, null)
      } catch (e: Exception) {
        resolver.delete(uri, null, null)
        throw e
      }
      return
    }
    @Suppress("DEPRECATION")
    val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES), ALBUM)
    if (!dir.isDirectory && !dir.mkdirs()) throw IllegalStateException("no folder for the picture")
    val stem = name.substringBeforeLast('.')
    val ext = name.substringAfterLast('.', "png")
    var file = File(dir, name)
    var n = 1
    while (file.exists()) file = File(dir, "$stem (${n++}).$ext")
    try {
      FileOutputStream(file).use { out ->
        out.write(png)
        out.fd.sync()
      }
    } catch (e: Exception) {
      file.delete()
      throw e
    }
    MediaScannerConnection.scanFile(context, arrayOf(file.path), arrayOf("image/png"), null)
  }
}

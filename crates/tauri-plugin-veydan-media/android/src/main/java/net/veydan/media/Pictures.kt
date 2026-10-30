// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

import android.graphics.Bitmap
import android.graphics.ColorSpace
import android.graphics.ImageDecoder
import android.os.Build
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
}

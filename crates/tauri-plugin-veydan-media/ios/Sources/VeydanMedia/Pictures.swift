// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import ImageIO
import Photos
import UIKit
import UniformTypeIdentifiers

/**
 * Photos written anew, as Pictures.kt does on Android: a photo of a format
 * not every client shows (HEIC/HEIF, or anything else ImageIO reads) as a
 * JPEG; the camera's shot as an upright JPEG; a picture of the app into
 * the photo library.
 */
enum Pictures {
  /**
   * The photo at `src` as a JPEG of `quality` at `dst` (platform-spec,
   * «Общие форматы вложений»): ImageIO decodes it turned upright as its
   * camera said (`kCGImageSourceCreateThumbnailWithTransform`), smaller
   * when it is larger than `maxPixels`, and the JPEG is written from the
   * pixels alone: a `CGImage` carries no metadata, so nothing of the
   * source (place, phone, time) reaches it. Its colour profile stays.
   */
  static func toJpeg(src: URL, dst: URL, maxPixels: Int64, quality: Int) throws {
    let read = [kCGImageSourceShouldCache: false] as CFDictionary
    guard let source = CGImageSourceCreateWithURL(src as CFURL, read), CGImageSourceGetCount(source) > 0 else {
      throw MediaError("not a picture ImageIO reads")
    }
    guard let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
      let width = (props[kCGImagePropertyPixelWidth] as? NSNumber)?.intValue,
      let height = (props[kCGImagePropertyPixelHeight] as? NSNumber)?.intValue,
      width > 0, height > 0
    else {
      throw MediaError("a picture without a size")
    }
    let (w, h) = targetSize(width, height, maxPixels)
    let decode: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: max(w, h),
      kCGImageSourceShouldCacheImmediately: true,
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, decode as CFDictionary) else {
      throw MediaError("the picture was not decoded")
    }
    try writeJpeg(image, to: dst, quality: quality)
  }

  /** `image` as a JPEG at `dst`, with nothing but the quality as its properties. */
  static func writeJpeg(_ image: CGImage, to dst: URL, quality: Int) throws {
    guard let out = CGImageDestinationCreateWithURL(dst as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else {
      throw MediaError("the JPEG was not written")
    }
    let q = Double(min(max(quality, 1), 100)) / 100
    CGImageDestinationAddImage(out, image, [kCGImageDestinationLossyCompressionQuality: q] as CFDictionary)
    if !CGImageDestinationFinalize(out) {
      try? FileManager.default.removeItem(at: dst)
      throw MediaError("the JPEG was not written")
    }
  }

  /**
   * A shot of the camera as an upright JPEG at `dst`: `UIImagePickerController`
   * gives the sensor's pixels with an orientation beside them, which the
   * app's own steps would not all read. Drawn once the way it is held.
   */
  static func writeShot(_ shot: UIImage, to dst: URL) throws {
    let format = UIGraphicsImageRendererFormat()
    format.scale = shot.scale
    format.opaque = true
    let upright = UIGraphicsImageRenderer(size: shot.size, format: format).image { _ in
      shot.draw(in: CGRect(origin: .zero, size: shot.size))
    }
    guard let image = upright.cgImage else { throw MediaError("the shot was not drawn") }
    try writeJpeg(image, to: dst, quality: 92)
  }

  /**
   * A PNG of the app (a QR code of «Сохранить», ROO-297) into the photo
   * library, under `name`. iOS lets an app add without reading
   * (`.addOnly`, `NSPhotoLibraryAddUsageDescription`), asked at this tap
   * only; an album of its own would need the access to read, so the
   * picture goes among the others. `storage_denied` when the user says no,
   * as Android 9 says it.
   */
  static func save(png: Data, name: String, done: @escaping (MediaError?) -> Void) {
    let write = {
      PHPhotoLibrary.shared().performChanges({
        let options = PHAssetResourceCreationOptions()
        options.originalFilename = name
        PHAssetCreationRequest.forAsset().addResource(with: .photo, data: png, options: options)
      }) { ok, error in
        if ok {
          done(nil)
        } else {
          mediaLog("the picture was not saved: \(String(describing: error))")
          done(MediaError("the picture was not saved"))
        }
      }
    }
    switch PHPhotoLibrary.authorizationStatus(for: .addOnly) {
    case .authorized, .limited:
      write()
    case .notDetermined:
      PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
        if status == .authorized || status == .limited { write() } else { done(MediaError("storage_denied")) }
      }
    default:
      done(MediaError("storage_denied"))
    }
  }
}

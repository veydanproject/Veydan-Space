// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import CryptoKit
import Foundation
import Photos
import UIKit

/**
 * The phone's photos and videos through PhotoKit, as Gallery.kt reads
 * MediaStore: pages of the newest by a cursor, the albums, whether an
 * item may still be read, and an item written out as a file to import.
 * Only the assets' records are read for a page, never a file: a fetch of
 * PhotoKit is lazy, and a page reads its own items alone. With a limited
 * access PhotoKit itself gives only what the user chose.
 */
enum Gallery {
  static func status() -> LibraryStatus {
    switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
    case .authorized: return .authorized
    case .limited: return .limited
    case .notDetermined: return .notDetermined
    default: return .denied
    }
  }

  static func access() -> MediaAccess {
    return status().access
  }

  /** Photos, videos, or both; never the other kinds of the library (audio). */
  private static func kindPredicate(_ kind: String?) -> NSPredicate {
    let image = PHAssetMediaType.image.rawValue
    let video = PHAssetMediaType.video.rawValue
    switch kind {
    case "image": return NSPredicate(format: "mediaType == %ld", image)
    case "video": return NSPredicate(format: "mediaType == %ld", video)
    default: return NSPredicate(format: "mediaType == %ld OR mediaType == %ld", image, video)
    }
  }

  private static func millis(_ date: Date?) -> Int64 {
    guard let date = date else { return Int64.min }
    return Int64((date.timeIntervalSince1970 * 1000).rounded(.down))
  }

  /**
   * A page of the newest photos and videos after `cursor`: the items as
   * the Kotlin side answers them (`id`, `kind`, `path` = `ph://<id>`,
   * `width`, `height`, `durationMs` of a video) and the cursor of the next
   * page, none after the last. An album that is not there gives no item.
   */
  static func page(cursor word: String?, limit: Int, kind: String?, album: String?) -> (items: [[String: Any]], next: String?) {
    let options = PHFetchOptions()
    var predicates = [kindPredicate(kind)]
    let cursor = PageCursor.of(word)
    if let c = cursor {
      // Everything taken up to the cursor's millisecond; `start` skips what came before it.
      let bound = Date(timeIntervalSince1970: Double(c.created + 1) / 1000)
      predicates.append(NSPredicate(format: "creationDate < %@", bound as NSDate))
    }
    options.predicate = NSCompoundPredicate(andPredicateWithSubpredicates: predicates)
    options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
    let result: PHFetchResult<PHAsset>
    if let album = album, !album.isEmpty {
      guard let collection = PHAssetCollection.fetchAssetCollections(withLocalIdentifiers: [album], options: nil).firstObject else {
        return ([], nil)
      }
      result = PHAsset.fetchAssets(in: collection, options: options)
    } else {
      result = PHAsset.fetchAssets(with: options)
    }
    let count = result.count
    let first = cursor?.start(
      count: count, time: { millis(result.object(at: $0).creationDate) },
      id: { result.object(at: $0).localIdentifier }) ?? 0
    let n = pageLimit(limit)
    let end = min(first + n, count)
    var items: [[String: Any]] = []
    items.reserveCapacity(max(end - first, 0))
    var last: PHAsset?
    if first < end {
      for i in first..<end {
        let asset = result.object(at: i)
        let video = asset.mediaType == .video
        var item: [String: Any] = [
          "id": asset.localIdentifier,
          "kind": video ? "video" : "image",
          "path": assetUri(asset.localIdentifier),
          "width": asset.pixelWidth,
          "height": asset.pixelHeight,
        ]
        if video { item["durationMs"] = Int64((asset.duration * 1000).rounded()) }
        items.append(item)
        last = asset
      }
    }
    var next: String?
    if end < count, let last = last {
      next = PageCursor(created: millis(last.creationDate), id: last.localIdentifier).word()
    }
    return (items, next)
  }

  /** The smart albums offered beside the user's own; «Недавние» is the grid itself. */
  private static let smartAlbums: [PHAssetCollectionSubtype] = [
    .smartAlbumFavorites, .smartAlbumVideos, .smartAlbumSelfPortraits, .smartAlbumScreenshots,
    .smartAlbumLivePhotos, .smartAlbumPanoramas, .smartAlbumBursts, .smartAlbumSlomoVideos,
  ]

  /** The albums with a photo or a video in them, the largest first: `id` (the collection's), `title`, `count`. */
  static func albums() -> [[String: Any]] {
    let options = PHFetchOptions()
    options.predicate = kindPredicate(nil)
    var found: [(id: String, title: String, count: Int)] = []
    var seen = Set<String>()
    let add: (PHAssetCollection) -> Void = { collection in
      guard !seen.contains(collection.localIdentifier) else { return }
      seen.insert(collection.localIdentifier)
      let n = PHAsset.fetchAssets(in: collection, options: options).count
      if n > 0 { found.append((collection.localIdentifier, collection.localizedTitle ?? "", n)) }
    }
    for subtype in smartAlbums {
      PHAssetCollection.fetchAssetCollections(with: .smartAlbum, subtype: subtype, options: nil)
        .enumerateObjects { collection, _, _ in add(collection) }
    }
    PHAssetCollection.fetchAssetCollections(with: .album, subtype: .any, options: nil)
      .enumerateObjects { collection, _, _ in add(collection) }
    return found.sorted { $0.count > $1.count }.map { ["id": $0.id, "title": $0.title, "count": $0.count] }
  }

  /** The asset an address of the grid names, when the app may read it now. */
  static func asset(_ uri: String) -> PHAsset? {
    guard let id = assetId(uri) else { return nil }
    return PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil).firstObject
  }

  /** Which of the local identifiers `ids` the photo library gives the app now (one fetch). */
  static func present(_ ids: [String]) -> Set<String> {
    var there = Set<String>()
    PHAsset.fetchAssets(withLocalIdentifiers: ids, options: nil).enumerateObjects { asset, _, _ in
      there.insert(asset.localIdentifier)
    }
    return there
  }

  /**
   * The item at `uri` written out into a folder of its own in
   * `attach-picked/`, as the phone keeps it (a HEIC stays a HEIC: the app's
   * converter makes the JPEG), its edit when it was edited, under the name
   * the photo had (`IMG_0001.HEIC`). Fetched from iCloud when the phone
   * holds only its preview. Blocks until it is written: called off the
   * main thread. `not_accessible` when the app may no longer read it.
   */
  static func export(_ uri: String) throws -> URL {
    guard let asset = asset(uri) else { throw MediaError("not_accessible") }
    let resources = PHAssetResource.assetResources(for: asset)
    let video = asset.mediaType == .video
    let wanted: [PHAssetResourceType] = video ? [.fullSizeVideo, .video] : [.fullSizePhoto, .photo]
    guard let resource = wanted.lazy.compactMap({ type in resources.first { $0.type == type } }).first else {
      throw MediaError("not_accessible")
    }
    // An edit is named FullSizeRender: the name of the original says more.
    let original = resources.first { $0.type == (video ? .video : .photo) } ?? resource
    let stem = (safeName(original.originalFilename, fallback: video ? "video" : "photo") as NSString).deletingPathExtension
    let ext = (resource.originalFilename as NSString).pathExtension
    let name = safeName(ext.isEmpty ? stem : "\(stem).\(ext)", fallback: video ? "video.mov" : "photo.jpg")
    let dir = try Cache.pickedFolder()
    let file = dir.appendingPathComponent(name)
    let options = PHAssetResourceRequestOptions()
    options.isNetworkAccessAllowed = true
    let done = DispatchSemaphore(value: 0)
    var failure: Error?
    PHAssetResourceManager.default().writeData(for: resource, toFile: file, options: options) { error in
      failure = error
      done.signal()
    }
    done.wait()
    if let failure = failure {
      try? FileManager.default.removeItem(at: dir)
      mediaLog("an item of the grid was not written out: \(failure)")
      throw MediaError("not_accessible")
    }
    return file
  }
}

/**
 * Thumbnails of the grid as JPEG files in `attach-thumbs/` of the app's
 * cache, which the app's server of files hands to the page by a name of
 * its own pool: never base64. At most about 50 MB on the disk, the least
 * recently shown going first; the folder is emptied when the app starts.
 * As Thumbs of Gallery.kt.
 */
final class Thumbs {
  let dir = Cache.thumbs
  private let lru = ThumbLru(limit: 50 * 1024 * 1024, low: 40 * 1024 * 1024)
  private let images = PHCachingImageManager()

  /** Empties the folder (the start of the app); the old files go on `queue`. */
  func clear(on queue: OperationQueue) {
    lru.clear()
    Cache.empty(dir, on: queue)
  }

  enum Answer {
    case file(URL)
    /** The app may no longer read the item: its thumbnails were deleted, these were their files. */
    case gone([URL])
  }

  /**
   * The file of the thumbnail of `uri`, `side` pixels at most; made when it
   * is not there. The photo library is asked first whether the app may
   * read the item now, kept or not: a limited access the user changed
   * leaves no thumbnail of what it no longer lets in. Called off the main
   * thread (the request waits for its picture).
   */
  func of(_ uri: String, side: Int) throws -> Answer {
    guard let asset = Gallery.asset(uri) else { return .gone(drop(uri)) }
    let px = thumbSide(side)
    let name = Thumbs.nameOf(uri, px)
    let file = dir.appendingPathComponent(name)
    let fm = FileManager.default
    if lru.touch(name) && fm.fileExists(atPath: file.path) { return .file(file) }
    try Cache.make(dir)
    let options = PHImageRequestOptions()
    options.isSynchronous = true
    options.deliveryMode = .highQualityFormat
    options.resizeMode = .fast
    options.isNetworkAccessAllowed = true
    var picture: UIImage?
    images.requestImage(
      for: asset, targetSize: CGSize(width: px, height: px), contentMode: .aspectFit, options: options
    ) { image, _ in
      picture = image
    }
    guard let jpeg = picture?.jpegData(compressionQuality: 0.82) else { throw MediaError("no thumbnail") }
    // Two asks of the same tile at once write apart; the last to finish stands.
    let part = dir.appendingPathComponent("\(name).\(UUID().uuidString).part")
    try jpeg.write(to: part)
    do {
      if fm.fileExists(atPath: file.path) { try? fm.removeItem(at: file) }
      try fm.moveItem(at: part, to: file)
    } catch {
      try? fm.removeItem(at: part)
      throw MediaError("thumbnail not written")
    }
    for old in lru.add(name, bytes: Int64(jpeg.count), owner: uri) {
      try? fm.removeItem(at: dir.appendingPathComponent(old))
    }
    return .file(file)
  }

  /** Every thumbnail kept of `uri` deleted (the app may no longer read it); their files. */
  func drop(_ uri: String) -> [URL] {
    let gone = lru.forget(uri).map { dir.appendingPathComponent($0) }
    for file in gone { try? FileManager.default.removeItem(at: file) }
    return gone
  }

  private static func nameOf(_ uri: String, _ px: Int) -> String {
    let digest = SHA256.hash(data: Data("\(uri)|\(px)".utf8))
    return digest.prefix(16).map { String(format: "%02x", $0) }.joined() + ".jpg"
  }
}

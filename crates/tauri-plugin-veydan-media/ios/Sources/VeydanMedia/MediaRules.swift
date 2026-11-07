// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation

/*
 * The decisions of the gallery that need no phone, as MediaRules.kt makes
 * them on Android: what an answer of the photo library means for the grid,
 * what a tap on «Разрешить доступ» does, where a page of the grid starts,
 * how large a converted photo may be, which thumbnails go first. Foundation
 * alone, so that each rule reads as its Kotlin twin.
 */

/** What the grid may show: everything, what the user chose, nothing. The words are the Kotlin side's. */
enum MediaAccess: String {
  case full
  case partial
  case denied
}

/**
 * The photo library's answer, as the rules need it (`PHAuthorizationStatus`
 * for `.readWrite`, read in Gallery.swift): never asked, all of it, the
 * photos the user chose («Выбрать фото…»), refused (by the user, or by a
 * restriction of the phone).
 */
enum LibraryStatus {
  case notDetermined
  case authorized
  case limited
  case denied

  /** Never asked shows nothing yet, as a refusal does: the grid offers the question. */
  var access: MediaAccess {
    switch self {
    case .authorized: return .full
    case .limited: return .partial
    case .notDetermined, .denied: return .denied
    }
  }
}

/** What a tap on «Разрешить доступ» (or «Выбрать ещё») does. */
enum AskStep: Equatable {
  /** All is let in: nothing to ask. */
  case nothing
  /** The system's question, which iOS shows once in the app's life. */
  case question
  /** The system's chooser of the photos the app may see, to choose more (as Android 14 asks again). */
  case chooseMore
  /** Refused: iOS never asks again, so the app's page in Settings opens instead. */
  case settings
}

enum MediaPermissions {
  static func step(_ status: LibraryStatus) -> AskStep {
    switch status {
    case .authorized: return .nothing
    case .notDetermined: return .question
    case .limited: return .chooseMore
    case .denied: return .settings
    }
  }
}

/** The scheme of an item of the grid: a PhotoKit asset by its local identifier (lib.rs `ASSET_SCHEME`). */
let ASSET_SCHEME = "ph://"

/** The local identifier of the asset an address of the grid names; nil for anything else. */
func assetId(_ uri: String) -> String? {
  guard uri.hasPrefix(ASSET_SCHEME) else { return nil }
  let id = String(uri.dropFirst(ASSET_SCHEME.count))
  return id.isEmpty ? nil : id
}

/** The address of the grid of an asset. */
func assetUri(_ id: String) -> String {
  return ASSET_SCHEME + id
}

/**
 * Where a page of the newest photos and videos goes on: after the last item
 * of the page before, by the time it was taken (milliseconds) and then by
 * its place among those of the same millisecond, which the item names by
 * its identifier. Opaque to the page. A local identifier holds `/` but no
 * `:` before it, so the word splits at its first `:`.
 */
struct PageCursor: Equatable {
  let created: Int64
  let id: String

  func word() -> String {
    return "\(created):\(id)"
  }

  /** A cursor the page gave back; nil for the first page or a word that is not one. Photos older than 1970 have a time below 0. */
  static func of(_ word: String?) -> PageCursor? {
    guard let word = word, let colon = word.firstIndex(of: ":") else { return nil }
    guard let created = Int64(word[..<colon]), created < Int64.max else { return nil }
    let id = String(word[word.index(after: colon)...])
    if id.isEmpty { return nil }
    return PageCursor(created: created, id: id)
  }

  /**
   * Where the page after this cursor starts among `times` and `ids`, the
   * items of the fetch of everything taken up to the cursor's millisecond,
   * newest first: right after the cursor's item; when that item is gone, at
   * the first one older than its millisecond (those of the same
   * millisecond before it may come again: a rare double, never a gap).
   */
  func start(count: Int, time: (Int) -> Int64, id: (Int) -> String) -> Int {
    var i = 0
    while i < count {
      let t = time(i)
      if t < created { return i }
      if t == created && id(i) == self.id { return i + 1 }
      i += 1
    }
    return count
  }
}

/** The most items a page has: what the grid asks is bounded. */
let MAX_PAGE = 300

func pageLimit(_ asked: Int) -> Int {
  return min(max(asked, 1), MAX_PAGE)
}

/**
 * The size a photo is decoded at to stay within `maxPixels`: its own when
 * it fits, else both sides scaled by the same factor, rounded down.
 */
func targetSize(_ width: Int, _ height: Int, _ maxPixels: Int64) -> (Int, Int) {
  if width <= 0 || height <= 0 { return (width, height) }
  let pixels = Int64(width) * Int64(height)
  if pixels <= maxPixels { return (width, height) }
  let scale = (Double(maxPixels) / Double(pixels)).squareRoot()
  var w = max(Int((Double(width) * scale).rounded(.down)), 1)
  var h = max(Int((Double(height) * scale).rounded(.down)), 1)
  // Rounding never takes it over the bound.
  while Int64(w) * Int64(h) > maxPixels {
    if w >= h { w -= 1 } else { h -= 1 }
  }
  return (w, h)
}

/** The most addresses `readable` checks at once: past it, none is taken for readable. */
let MAX_READABLE = 300

/**
 * Which of `uris` the app may still read, in their order: an item of the
 * grid (`ph://`) whose asset the photo library gives now (`present`, asked
 * once with every identifier). Anything else is not of the grid and is
 * never taken for readable; past `MAX_READABLE` nothing is asked at all.
 */
func stillReadable(_ uris: [String], present: ([String]) -> Set<String>) -> [String] {
  if uris.count > MAX_READABLE { return [] }
  let ids = uris.compactMap(assetId)
  if ids.isEmpty { return [] }
  let there = present(ids)
  return uris.filter { uri in assetId(uri).map { there.contains($0) } ?? false }
}

/** The side of a thumbnail asked for, bounded: a tile of the grid, or the large one of the viewer. */
func thumbSide(_ asked: Int) -> Int {
  return min(max(asked, 64), 1280)
}

/**
 * The thumbnails kept on the disk, the least recently used going first
 * once they take more than `limit` bytes, down to `low`. Only names and
 * sizes: the files are the caller's. Each name is of an item (`owner`, its
 * `ph://` address): all of an item go at once when the app may no longer
 * read it (`forget`). Safe from any thread.
 */
final class ThumbLru {
  private let limit: Int64
  private let low: Int64
  private var sizes: [String: Int64] = [:]
  private var owners: [String: String] = [:]
  /** The names, the least recently used first. */
  private var order: [String] = []
  private var bytes: Int64 = 0
  private let lock = NSLock()

  init(limit: Int64, low: Int64) {
    self.limit = limit
    self.low = low
  }

  var total: Int64 {
    lock.lock()
    defer { lock.unlock() }
    return bytes
  }

  private func moveToEnd(_ name: String) {
    if let i = order.firstIndex(of: name) { order.remove(at: i) }
    order.append(name)
  }

  /** Whether `name` is kept; a kept one becomes the most recently used. */
  func touch(_ name: String) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard sizes[name] != nil else { return false }
    moveToEnd(name)
    return true
  }

  /** A thumbnail of `owner` written; the names to delete now, oldest first. */
  func add(_ name: String, bytes size: Int64, owner: String? = nil) -> [String] {
    lock.lock()
    defer { lock.unlock() }
    if let old = sizes[name] { bytes -= old }
    sizes[name] = size
    owners[name] = owner ?? name
    bytes += size
    moveToEnd(name)
    if bytes <= limit { return [] }
    var gone: [String] = []
    var kept: [String] = []
    for n in order {
      if bytes > low && n != name, let s = sizes[n] {
        bytes -= s
        sizes[n] = nil
        owners[n] = nil
        gone.append(n)
      } else {
        kept.append(n)
      }
    }
    order = kept
    return gone
  }

  /** Every thumbnail of `owner` leaves; their names, for the caller to delete. */
  func forget(_ owner: String) -> [String] {
    lock.lock()
    defer { lock.unlock() }
    let gone = owners.filter { $0.value == owner }.map { $0.key }
    for name in gone {
      if let s = sizes[name] { bytes -= s }
      sizes[name] = nil
      owners[name] = nil
    }
    if !gone.isEmpty {
      let set = Set(gone)
      order.removeAll { set.contains($0) }
    }
    return gone
  }

  func clear() {
    lock.lock()
    defer { lock.unlock() }
    sizes.removeAll()
    owners.removeAll()
    order.removeAll()
    bytes = 0
  }
}

/**
 * The name of a file the plugin writes for a name another app gave (the
 * Photo Picker's, a photo's original): its last segment, without what a
 * folder or a hidden file would make of it; `fallback` when nothing is left.
 */
func safeName(_ name: String?, fallback: String) -> String {
  let last = (name ?? "").split(separator: "/").last.map(String.init) ?? ""
  var kept = String.UnicodeScalarView()
  for scalar in last.unicodeScalars where !CharacterSet.controlCharacters.contains(scalar) && scalar != "\\" && scalar != ":" {
    kept.append(scalar)
  }
  let clean = String(kept)
  let trimmed = clean.trimmingCharacters(in: .whitespaces)
  if trimmed.isEmpty || trimmed.hasPrefix(".") { return fallback }
  return String(trimmed.prefix(120))
}

/**
 * `text` escaped for a part of a URL (the body of `sms:`, the subject and
 * body of `mailto:`, RFC 6068): every byte but the unreserved ones, so the
 * text can add no header nor addressee.
 */
func escapeForUrl(_ text: String) -> String {
  var allowed = CharacterSet()
  allowed.insert(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
  return text.addingPercentEncoding(withAllowedCharacters: allowed) ?? ""
}

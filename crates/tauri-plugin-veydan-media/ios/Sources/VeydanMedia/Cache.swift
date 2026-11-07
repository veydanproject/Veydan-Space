// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation

func mediaLog(_ message: String) {
  NSLog("VeydanMedia: %@", message)
}

/** An error of a command, as the Rust side gets its message (`camera_denied`, `no_app`, …). */
struct MediaError: Error {
  let message: String

  init(_ message: String) {
    self.message = message
  }
}

/**
 * The folders of the app's cache the plugin writes into. The root is
 * `Library/Caches/<bundle id>`, the bundle's own identifier (the Rust side
 * reads it from NSBundle too, `ios_cache_dir`: Tauri's `app_cache_dir` takes
 * the identifier of the config, which a sideloaded build does not carry): the app's Rust code
 * reads a file there without the scope of fs (`veydan_shell` source.rs),
 * and takes one of `attach-capture` and `attach-picked` for the phone's to
 * copy in (lib.rs `HANDED_DIRS`), as a `content://` on Android.
 */
enum Cache {
  static let root: URL = {
    let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
      ?? URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
    return caches.appendingPathComponent(Bundle.main.bundleIdentifier ?? "app", isDirectory: true)
  }()

  /** The camera's shots, until the app has copied them. */
  static let capture = root.appendingPathComponent("attach-capture", isDirectory: true)
  /** The Photo Picker's copies and the items of the grid written out, each in a folder of its own. */
  static let picked = root.appendingPathComponent("attach-picked", isDirectory: true)
  /** The thumbnails of the grid (Thumbs). */
  static let thumbs = root.appendingPathComponent("attach-thumbs", isDirectory: true)

  static func make(_ dir: URL) throws {
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
  }

  /**
   * A new folder of its own in `picked`, so that a copy keeps the name the
   * user's photo had (`IMG_0001.HEIC`) and two of the same name never meet.
   */
  static func pickedFolder() throws -> URL {
    let dir = picked.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try make(dir)
    return dir
  }

  /**
   * Empties `dir` (the start of the app): it is moved aside at once, so a
   * file written right after is never deleted with it, and what was moved
   * goes on `queue`, with what an earlier start moved aside and did not finish.
   */
  static func empty(_ dir: URL, on queue: OperationQueue) {
    let fm = FileManager.default
    let aside = dir.deletingLastPathComponent()
      .appendingPathComponent("\(dir.lastPathComponent).old-\(UUID().uuidString)", isDirectory: true)
    if fm.fileExists(atPath: dir.path) {
      do {
        try fm.moveItem(at: dir, to: aside)
      } catch {
        try? fm.removeItem(at: dir)
      }
    }
    try? make(dir)
    queue.addOperation {
      let parent = dir.deletingLastPathComponent()
      let prefix = "\(dir.lastPathComponent).old-"
      for name in (try? fm.contentsOfDirectory(atPath: parent.path)) ?? [] where name.hasPrefix(prefix) {
        try? fm.removeItem(at: parent.appendingPathComponent(name))
      }
    }
  }

  /** The file a path or a `file://` URL names, links resolved; nil for anything else. */
  static func file(_ pathOrUrl: String) -> URL? {
    let url: URL
    if pathOrUrl.lowercased().hasPrefix("file://") {
      guard let u = URL(string: pathOrUrl), u.isFileURL else { return nil }
      url = u
    } else if pathOrUrl.hasPrefix("/") {
      url = URL(fileURLWithPath: pathOrUrl)
    } else {
      return nil
    }
    return url.standardizedFileURL.resolvingSymlinksInPath()
  }

  /**
   * Deletes a file the plugin handed over, once the app holds its own copy:
   * a shot right in `capture`, or a copy in its own folder of `picked`
   * (the folder goes with it). Anything else is left alone.
   */
  static func forget(_ pathOrUrl: String) {
    guard let file = file(pathOrUrl) else { return }
    let name = file.lastPathComponent
    if name.isEmpty || name.hasPrefix(".") { return }
    let fm = FileManager.default
    let parent = file.deletingLastPathComponent()
    let capture = Cache.capture.standardizedFileURL.resolvingSymlinksInPath()
    let picked = Cache.picked.standardizedFileURL.resolvingSymlinksInPath()
    if parent.path == capture.path || parent.path == picked.path {
      try? fm.removeItem(at: file)
    } else if parent.deletingLastPathComponent().path == picked.path {
      try? fm.removeItem(at: parent)
    }
  }
}

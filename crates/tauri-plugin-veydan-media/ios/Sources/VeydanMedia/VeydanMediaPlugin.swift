// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import Foundation
import Photos
import Tauri
import UIKit
import WebKit

/*
 * The arguments of the commands, as the Rust side sends them (src/lib.rs).
 * Every field is optional and takes the Kotlin side's default when
 * missing, so that both sides read the same JSON the same way.
 */

struct PageArgs: Decodable {
  let cursor: String?
  let limit: Int?
  /** `image`, `video`, or both when absent. */
  let kind: String?
  let album: String?
}

struct ThumbArgs: Decodable {
  let uri: String?
  let side: Int?
}

struct ReadableArgs: Decodable {
  let uris: [String]?
}

struct CaptureArgs: Decodable {
  /** `photo` or `video`. */
  let kind: String?
}

struct CameraArgs: Decodable {
  /** A tap on «Разрешить доступ»: the system's question, or Settings. */
  let ask: Bool?
}

struct PickArgs: Decodable {
  let multiple: Bool?
  /** `image`, `video`, `media`. */
  let kind: String?
}

struct ConvertArgs: Decodable {
  let src: String?
  let dst: String?
  let maxPixels: Int64?
  let quality: Int?
}

struct UriArgs: Decodable {
  let uri: String?
}

struct ShareArgs: Decodable {
  /** `send` (the share sheet), `sms`, `mail`. */
  let via: String?
  let text: String?
  /** The subject of a letter. */
  let subject: String?
}

struct SavePictureArgs: Decodable {
  /** The PNG, as base64. */
  let data: String?
  let name: String?
}

/** What a command that tells nothing answers: `{}`, as the Kotlin side's `JSObject()`. */
private let NOTHING: JsonObject = [:]

/**
 * The phone's gallery for the sheet of the paperclip on iOS (stage Э5b):
 * the access to the photo library, pages of the newest photos and videos
 * and the albums, thumbnails as files, the camera, the system's Photo
 * Picker, photos of HEIC written as JPEG, a text into another app and a
 * picture into the library. Called from the app's Rust code only, by the
 * same commands and with the same answers as the Kotlin side
 * (VeydanMediaPlugin.kt); see src/lib.rs for the contract. What differs is
 * what an address is: `ph://<local identifier>` for an item of the grid,
 * the `file://` URL of a file in the app's cache for a shot or a pick, and
 * the extra command `export`, which writes an item of the grid out.
 *
 * Nothing is asked of the user before a tap: the access is only read until
 * the page asks for it (`request`), the camera's permission until the tile
 * of the camera is tapped (`capture`). Tauri calls a command on one queue
 * of its own for every plugin: what reads the library or the disk goes to
 * this plugin's queue, what shows a screen to the main thread.
 */
class VeydanMediaPlugin: Plugin {
  // PhotoKit, ImageIO and the disk off the queue that called, three at once as on Android.
  private let work: OperationQueue = {
    let queue = OperationQueue()
    queue.name = "net.veydan.media"
    queue.maxConcurrentOperationCount = 3
    queue.qualityOfService = .userInitiated
    return queue
  }()
  private let thumbs = Thumbs()
  /** The camera or the Photo Picker while it is open (its delegate is weak); on the main thread. */
  private var screen: NSObject?

  override func load(webview: WKWebView) {
    // What a run before left: thumbnails, and shots and picks never imported.
    thumbs.clear(on: work)
    Cache.empty(Cache.capture, on: work)
    Cache.empty(Cache.picked, on: work)
  }

  private func background(_ invoke: Invoke, _ job: @escaping () throws -> JsonObject) {
    work.addOperation {
      do {
        invoke.resolve(try job())
      } catch let error as MediaError {
        mediaLog("media command failed: \(error.message)")
        invoke.reject(error.message)
      } catch {
        mediaLog("media command failed: \(error)")
        invoke.reject("\(error)")
      }
    }
  }

  private func accessAnswer() -> JsonObject {
    return ["access": Gallery.access().rawValue]
  }

  // ─── Access ──────────────────────────────────────────────────────────────

  /** What the grid may show now (`full`, `partial`, `denied`); asks nothing. Read again whenever the page looks. */
  @objc func access(_ invoke: Invoke) {
    invoke.resolve(accessAnswer())
  }

  /**
   * The user tapped «Разрешить доступ» (or «Выбрать ещё»): the system's
   * question when it was never asked; the system's chooser of the photos
   * the app may see when it sees a part; the app's page in Settings when
   * it was refused (iOS asks once). Answers the access after it.
   */
  @objc func request(_ invoke: Invoke) {
    let answer = { invoke.resolve(self.accessAnswer()) }
    switch MediaPermissions.step(Gallery.status()) {
    case .nothing:
      answer()
    case .question:
      PHPhotoLibrary.requestAuthorization(for: .readWrite) { _ in answer() }
    case .chooseMore:
      DispatchQueue.main.async {
        guard let host = Screens.top() else {
          answer()
          return
        }
        PHPhotoLibrary.shared().presentLimitedLibraryPicker(from: host) { _ in answer() }
      }
    case .settings:
      DispatchQueue.main.async {
        if let url = URL(string: UIApplication.openSettingsURLString) {
          UIApplication.shared.open(url, options: [:], completionHandler: nil)
        }
        answer()
      }
    }
  }

  // ─── The grid ────────────────────────────────────────────────────────────

  /** A page of the newest photos and videos after `cursor`: `{ items, cursor }`, no cursor after the last. */
  @objc func recent(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PageArgs.self)
    background(invoke) {
      if Gallery.access() == .denied { return ["items": [[String: Any]]()] }
      let page = Gallery.page(cursor: args.cursor, limit: args.limit ?? 60, kind: args.kind, album: args.album)
      var answer: JsonObject = ["items": page.items]
      if let next = page.next { answer["cursor"] = next }
      return answer
    }
  }

  @objc func albums(_ invoke: Invoke) {
    background(invoke) {
      if Gallery.access() == .denied { return ["albums": [[String: Any]]()] }
      return ["albums": Gallery.albums()]
    }
  }

  /**
   * The file of the thumbnail of an item of the grid (`{ path }`, a path);
   * the app hands it to the page. An item out of the access now:
   * `{ gone }`, the files deleted of it, for the app to forget their addresses.
   */
  @objc func thumb(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ThumbArgs.self)
    guard let uri = args.uri, assetId(uri) != nil else {
      invoke.reject("not an item of the gallery")
      return
    }
    background(invoke) {
      switch try self.thumbs.of(uri, side: args.side ?? 256) {
      case .file(let file): return ["path": file.path]
      case .gone(let files): return ["gone": files.map { $0.path }]
      }
    }
  }

  /**
   * Which of the items chosen in the grid the app may still read
   * (`{ readable }`), each asked of the library by its identifier, whatever
   * album or page the grid shows; the thumbnails kept of the others are
   * deleted (`{ gone }`).
   */
  @objc func readable(_ invoke: Invoke) throws {
    let uris = try invoke.parseArgs(ReadableArgs.self).uris ?? []
    background(invoke) {
      let ok = stillReadable(uris, present: Gallery.present)
      let kept = Set(ok)
      var gone: [String] = []
      for uri in uris where !kept.contains(uri) && assetId(uri) != nil {
        gone.append(contentsOf: self.thumbs.drop(uri).map { $0.path })
      }
      return ["readable": ok, "gone": gone]
    }
  }

  /**
   * An item of the grid written out into `attach-picked/` to be imported
   * (iOS only: on Android the import reads the `content://` itself):
   * `{ path }`, its `file://` URL. `not_accessible` when the app may no
   * longer read it.
   */
  @objc func export(_ invoke: Invoke) throws {
    guard let uri = try invoke.parseArgs(UriArgs.self).uri, assetId(uri) != nil else {
      invoke.reject("not an item of the gallery")
      return
    }
    background(invoke) {
      let file = try Gallery.export(uri)
      return ["path": file.absoluteString]
    }
  }

  // ─── The camera ──────────────────────────────────────────────────────────

  /**
   * Whether the app may use the camera (`{ granted }`), read of the system:
   * the web view never says it (its query of the permission stays «prompt»),
   * so the tile of the camera asks here. With `ask` (a tap on «Разрешить
   * доступ»), the system's question when it was never asked, else the app's
   * page in Settings (iOS asks once). Without this the tile stayed «Разрешить
   * доступ» for good on an iPhone (2026-11-04).
   */
  @objc func camera(_ invoke: Invoke) throws {
    let ask = try invoke.parseArgs(CameraArgs.self).ask ?? false
    let status = AVCaptureDevice.authorizationStatus(for: .video)
    if status == .authorized || !ask {
      invoke.resolve(["granted": status == .authorized])
      return
    }
    if status == .notDetermined {
      CameraPermission.request { granted in invoke.resolve(["granted": granted]) }
      return
    }
    DispatchQueue.main.async {
      if let url = URL(string: UIApplication.openSettingsURLString) {
        UIApplication.shared.open(url, options: [:], completionHandler: nil)
      }
      invoke.resolve(["granted": false])
    }
  }

  /**
   * The camera takes a photo or a video into `attach-capture/`. Its
   * permission is asked first, on this tap. Answers `{ path }`, the
   * `file://` URL of the shot, or no path when it was given up;
   * `camera_denied`, `no_camera_app` (the simulator has none) refuse.
   */
  @objc func capture(_ invoke: Invoke) throws {
    let video = try invoke.parseArgs(CaptureArgs.self).kind == "video"
    DispatchQueue.main.async {
      guard self.screen == nil else {
        invoke.reject("busy")
        return
      }
      guard UIImagePickerController.isSourceTypeAvailable(.camera) else {
        invoke.reject("no_camera_app")
        return
      }
      CameraPermission.request { granted in
        guard granted else {
          invoke.reject("camera_denied")
          return
        }
        let camera = CameraScreen(invoke: invoke, video: video) { [weak self] in self?.screen = nil }
        guard camera.present() else {
          invoke.reject("no_camera_app")
          return
        }
        self.screen = camera
      }
    }
  }

  /**
   * A shot or a pick was imported (copied into the app's folder): its file
   * in the cache goes. Only a file of `attach-capture` or one of
   * `attach-picked`, given as a path or a `file://` URL.
   */
  @objc func forgetCapture(_ invoke: Invoke) throws {
    let uri = try invoke.parseArgs(UriArgs.self).uri
    background(invoke) {
      if let uri = uri { Cache.forget(uri) }
      return NOTHING
    }
  }

  // ─── The system's Photo Picker ───────────────────────────────────────────

  /** The system's Photo Picker (no access needed): `{ paths }`, `file://` URLs of the copies, none when given up. */
  @objc func pickSystem(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(PickArgs.self)
    DispatchQueue.main.async {
      guard self.screen == nil else {
        invoke.reject("busy")
        return
      }
      let picker = PickerScreen(invoke: invoke, multiple: args.multiple ?? true, kind: args.kind ?? "media") {
        [weak self] in self?.screen = nil
      }
      guard picker.present() else {
        invoke.reject("no_picker")
        return
      }
      self.screen = picker
    }
  }

  // ─── HEIC → JPEG ─────────────────────────────────────────────────────────

  /** The photo at `src` as a JPEG at `dst` (Pictures.swift); the app gives paths in its own folder. */
  @objc func convert(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ConvertArgs.self)
    guard let src = args.src, let dst = args.dst else {
      invoke.reject("no paths")
      return
    }
    background(invoke) {
      try Pictures.toJpeg(
        src: URL(fileURLWithPath: src), dst: URL(fileURLWithPath: dst),
        maxPixels: args.maxPixels ?? 50_000_000, quality: args.quality ?? 92)
      return NOTHING
    }
  }

  // ─── Sharing a text ──────────────────────────────────────────────────────

  /**
   * A text into another app: the share sheet (`send`), Messages (`sms`), a
   * letter (`mail`). Nothing comes back; `no_app` when the phone has none for it.
   */
  @objc func shareText(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(ShareArgs.self)
    guard let text = args.text, !text.isEmpty else {
      invoke.reject("no text")
      return
    }
    DispatchQueue.main.async {
      ShareText.share(via: args.via ?? "", text: text, subject: args.subject) { error in
        if let error = error { invoke.reject(error.message) } else { invoke.resolve(NOTHING) }
      }
    }
  }

  // ─── Save a picture ──────────────────────────────────────────────────────

  /** A PNG of the app (a QR code, ROO-297) into the photo library; `storage_denied` when the user says no. */
  @objc func savePicture(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(SavePictureArgs.self)
    guard let data = args.data, let png = Data(base64Encoded: data, options: .ignoreUnknownCharacters),
      let name = args.name, !name.isEmpty
    else {
      invoke.reject("no picture")
      return
    }
    Pictures.save(png: png, name: name) { error in
      if let error = error { invoke.reject(error.message) } else { invoke.resolve(NOTHING) }
    }
  }
}

@_cdecl("init_plugin_veydan_media")
func initPlugin() -> Plugin {
  return VeydanMediaPlugin()
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import Foundation
import PhotosUI
import Tauri
import UIKit
import UniformTypeIdentifiers

/*
 * The system's screens the plugin opens over the app: the camera, the
 * Photo Picker, the share sheet. Each is presented on the main thread from
 * the top-most controller over the app's webview, and answers once, when
 * the user is done or gave up, as the Kotlin side answers its activities.
 */

enum Screens {
  /** The controller to present from: the root of the key window (the webview's), or what it presents now. Tauri's own PluginManager keeps its controller internal. */
  static func top() -> UIViewController? {
    var vc = keyWindow()?.rootViewController
    while let shown = vc?.presentedViewController, !shown.isBeingDismissed {
      vc = shown
    }
    return vc
  }

  private static func keyWindow() -> UIWindow? {
    let windows = UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .flatMap { $0.windows }
    return windows.first { $0.isKeyWindow } ?? windows.first
  }

  /** An iPad shows a sheet as a popover, which needs a place: the middle of the app's view, without an arrow. */
  static func anchor(_ vc: UIViewController, on host: UIViewController) {
    guard let popover = vc.popoverPresentationController else { return }
    popover.sourceView = host.view
    popover.sourceRect = CGRect(x: host.view.bounds.midX, y: host.view.bounds.midY, width: 0, height: 0)
    popover.permittedArrowDirections = []
  }
}

/**
 * The camera, `photo` or `video` (`UIImagePickerController` of
 * `.camera`): the shot written into `attach-capture/`, answered as
 * `{ path }`, its `file://` URL (Android: a `content://` of the
 * FileProvider), or `{}` when it was given up.
 */
final class CameraScreen: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
  private let invoke: Invoke
  private let video: Bool
  private let finished: () -> Void
  private var answered = false

  init(invoke: Invoke, video: Bool, finished: @escaping () -> Void) {
    self.invoke = invoke
    self.video = video
    self.finished = finished
  }

  /** Opens the camera; false when there is none to open (the simulator) or nothing to open it over. */
  func present() -> Bool {
    guard UIImagePickerController.isSourceTypeAvailable(.camera), let host = Screens.top() else { return false }
    let type = video ? UTType.movie.identifier : UTType.image.identifier
    guard (UIImagePickerController.availableMediaTypes(for: .camera) ?? []).contains(type) else { return false }
    let picker = UIImagePickerController()
    picker.sourceType = .camera
    picker.mediaTypes = [type]
    picker.cameraCaptureMode = video ? .video : .photo
    if video { picker.videoQuality = .typeHigh }
    picker.delegate = self
    host.present(picker, animated: true)
    return true
  }

  private func answer(_ data: JsonObject?) {
    if answered { return }
    answered = true
    if let data = data { invoke.resolve(data) } else { invoke.resolve([String: Any?]()) }
    finished()
  }

  func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
    picker.dismiss(animated: true)
    answer(nil)
  }

  func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
    picker.dismiss(animated: true)
    let shot = info[.originalImage] as? UIImage
    let movie = info[.mediaURL] as? URL
    // Written off the main thread: a photo is drawn once more, a video moved.
    DispatchQueue.global(qos: .userInitiated).async {
      do {
        try Cache.make(Cache.capture)
        let name = UUID().uuidString
        let file: URL
        if self.video {
          guard let movie = movie else { throw MediaError("no video") }
          let ext = movie.pathExtension.isEmpty ? "mov" : movie.pathExtension.lowercased()
          file = Cache.capture.appendingPathComponent("\(name).\(ext)")
          // The picker's file is temporary: moved, else copied before it goes.
          do {
            try FileManager.default.moveItem(at: movie, to: file)
          } catch {
            try FileManager.default.copyItem(at: movie, to: file)
          }
        } else {
          guard let shot = shot else { throw MediaError("no photo") }
          file = Cache.capture.appendingPathComponent("\(name).jpg")
          try Pictures.writeShot(shot, to: file)
        }
        DispatchQueue.main.async { self.answer(["path": file.absoluteString]) }
      } catch {
        mediaLog("the shot was not written: \(error)")
        DispatchQueue.main.async { self.answer(nil) }
      }
    }
  }
}

/**
 * The system's Photo Picker (`PHPickerViewController`, which needs no
 * access to the library): every item picked copied into a folder of its
 * own in `attach-picked/` while the picker's file still exists, as the
 * phone keeps it (a HEIC stays a HEIC, `.current`), answered as
 * `{ paths }`, their `file://` URLs in the order picked; none when it was
 * given up. An item that could not be copied is left out.
 */
final class PickerScreen: NSObject, PHPickerViewControllerDelegate, UIAdaptivePresentationControllerDelegate {
  private let invoke: Invoke
  private let multiple: Bool
  private let kind: String
  private let finished: () -> Void
  private var answered = false

  init(invoke: Invoke, multiple: Bool, kind: String, finished: @escaping () -> Void) {
    self.invoke = invoke
    self.multiple = multiple
    self.kind = kind
    self.finished = finished
  }

  func present() -> Bool {
    guard let host = Screens.top() else { return false }
    var config = PHPickerConfiguration()
    switch kind {
    case "image": config.filter = .images
    case "video": config.filter = .videos
    default: config.filter = .any(of: [.images, .videos])
    }
    // 0 is no bound: the composer bounds what it takes.
    config.selectionLimit = multiple ? 0 : 1
    config.preferredAssetRepresentationMode = .current
    let picker = PHPickerViewController(configuration: config)
    picker.delegate = self
    picker.presentationController?.delegate = self
    host.present(picker, animated: true)
    return true
  }

  private func answer(_ paths: [String]) {
    if answered { return }
    answered = true
    invoke.resolve(["paths": paths])
    finished()
  }

  /** Swiped away: given up. */
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    answer([])
  }

  func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
    picker.dismiss(animated: true)
    if results.isEmpty {
      answer([])
      return
    }
    var copies = [URL?](repeating: nil, count: results.count)
    let group = DispatchGroup()
    let lock = NSLock()
    for (i, result) in results.enumerated() {
      let provider = result.itemProvider
      guard let type = PickerScreen.typeOf(provider) else { continue }
      group.enter()
      provider.loadFileRepresentation(forTypeIdentifier: type) { url, error in
        defer { group.leave() }
        guard let url = url else {
          mediaLog("a picked item was not read: \(String(describing: error))")
          return
        }
        do {
          let copy = try PickerScreen.copy(url, suggested: provider.suggestedName, type: type)
          lock.lock()
          copies[i] = copy
          lock.unlock()
        } catch {
          mediaLog("a picked item was not copied: \(error)")
        }
      }
    }
    group.notify(queue: .main) {
      self.answer(copies.compactMap { $0?.absoluteString })
    }
  }

  /**
   * The type to read an item as: a video as a movie, a photo as the image
   * type it has first (`public.heic` for an iPhone's photo), so that what
   * the phone keeps is what is copied.
   */
  private static func typeOf(_ provider: NSItemProvider) -> String? {
    if provider.hasItemConformingToTypeIdentifier(UTType.movie.identifier) { return UTType.movie.identifier }
    let image = provider.registeredTypeIdentifiers.first { id in
      UTType(id)?.conforms(to: .image) ?? false
    }
    if let image = image { return image }
    return provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) ? UTType.image.identifier : nil
  }

  /** The picker's file (gone when the handler returns) copied under the name the photo had, with the extension of its type. */
  private static func copy(_ url: URL, suggested: String?, type: String) throws -> URL {
    let ext = url.pathExtension.isEmpty ? (UTType(type)?.preferredFilenameExtension ?? "") : url.pathExtension
    let fallback = UTType(type)?.conforms(to: .movie) == true ? "video" : "photo"
    let stem = (safeName(suggested ?? url.lastPathComponent, fallback: fallback) as NSString).deletingPathExtension
    let name = safeName(ext.isEmpty ? stem : "\(stem).\(ext)", fallback: fallback)
    let dir = try Cache.pickedFolder()
    let copy = dir.appendingPathComponent(name)
    do {
      try FileManager.default.copyItem(at: url, to: copy)
    } catch {
      try? FileManager.default.removeItem(at: dir)
      throw error
    }
    return copy
  }
}

/** The camera's permission, asked on the tap of its tile (`NSCameraUsageDescription`); `done` on the main thread. */
enum CameraPermission {
  static func request(_ done: @escaping (Bool) -> Void) {
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized:
      done(true)
    case .notDetermined:
      AVCaptureDevice.requestAccess(for: .video) { granted in DispatchQueue.main.async { done(granted) } }
    default:
      done(false)
    }
  }
}

/**
 * A text into another app: the system's share sheet (`send`), Messages
 * with the text and no addressee (`sms`), a letter with a subject
 * (`mail`). The user picks the app and the addressee there; nothing comes
 * back. `no_app` when the phone has none for it (an iPad without
 * Messages, no mail account), as on Android.
 */
enum ShareText {
  static func share(via: String, text: String, subject: String?, done: @escaping (MediaError?) -> Void) {
    switch via {
    case "send":
      guard let host = Screens.top() else {
        done(MediaError("no_app"))
        return
      }
      let sheet = UIActivityViewController(activityItems: [text], applicationActivities: nil)
      Screens.anchor(sheet, on: host)
      host.present(sheet, animated: true) { done(nil) }
    case "sms":
      open("sms:&body=\(escapeForUrl(text))", done)
    case "mail":
      open("mailto:?subject=\(escapeForUrl(subject ?? ""))&body=\(escapeForUrl(text))", done)
    default:
      done(MediaError("unknown way"))
    }
  }

  private static func open(_ address: String, _ done: @escaping (MediaError?) -> Void) {
    guard let url = URL(string: address) else {
      done(MediaError("no_app"))
      return
    }
    UIApplication.shared.open(url, options: [:]) { opened in done(opened ? nil : MediaError("no_app")) }
  }
}

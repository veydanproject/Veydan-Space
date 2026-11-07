// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import Foundation
import Tauri
import UIKit

/**
 * Where the sound of a call goes: the earpiece, the loudspeaker, a
 * Bluetooth or a wired headset (the twin of AudioRoutes.kt).
 *
 * During a call the audio session is in the mode of a conversation
 * (`.playAndRecord` with `.voiceChat`, or `.videoChat` for a video call):
 * the echo canceller of the phone works and the volume keys set the call's
 * volume. The session is configured here when the call takes the sound
 * and activated by CallKit (`activated`), which is when the first route is
 * set; without CallKit the plugin activates it itself (`activateOwn`).
 *
 * The loudspeaker is an override of the output; a headset or the earpiece
 * is chosen by its microphone (`setPreferredInput`): on iOS the output of a
 * call follows the input it records from.
 *
 * Everything runs on the main thread.
 */
enum AudioRoutes {
  /** Called after every change of the route or of the routes there are. */
  static var onChange: (() -> Void)?

  /** A call holds the sound. */
  private(set) static var active = false
  private static var video = false
  /** The plugin activated the session itself (CallKit would not): it deactivates it too. */
  private static var ownActivation = false
  private static var observer: NSObjectProtocol?

  private static var session: AVAudioSession { AVAudioSession.sharedInstance() }

  /** The call takes the sound: the category and the mode; the route comes when the session is live. */
  static func begin(video: Bool) {
    if active { return }
    active = true
    AudioRoutes.video = video
    do {
      try session.setCategory(.playAndRecord, mode: video ? .videoChat : .voiceChat, options: [.allowBluetooth])
    } catch {
      callLog("sound: the category of a call was refused: \(error)")
    }
    observer = NotificationCenter.default.addObserver(
      forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main
    ) { note in
      AudioRoutes.routeChanged(note)
    }
    callLog("sound: in communication (video=\(video))")
  }

  /** The session is live (CallKit's `didActivate`, or `activateOwn`): the first route. */
  static func activated() {
    guard active else { return }
    let available = available()
    let first = Route.first(available: available, video: video)
    _ = set(first)
    callLog("sound: route \(first) of \(available)")
  }

  /**
   * CallKit would not take the call: the plugin activates the session
   * itself, so that the call still has its sound. Answers the session for
   * the engine (`RtcAudio.activated`), or nil when it would not activate.
   */
  static func activateOwn() -> AVAudioSession? {
    guard active else { return nil }
    do {
      try session.setActive(true)
      ownActivation = true
      activated()
      return session
    } catch {
      callLog("sound: the session would not activate: \(error)")
      return nil
    }
  }

  /** The call lets the sound go: the overrides undone, the session given back. */
  static func end() {
    guard active else { return }
    if let observer = observer {
      NotificationCenter.default.removeObserver(observer)
    }
    observer = nil
    try? session.overrideOutputAudioPort(.none)
    try? session.setPreferredInput(nil)
    if ownActivation {
      try? session.setActive(false, options: .notifyOthersOnDeactivation)
      ownActivation = false
    }
    active = false
    callLog("sound: back to normal")
    changed()
  }

  /** Sends the sound to `route`; false when the phone has no such route or no call holds the sound. */
  static func set(_ route: String) -> Bool {
    guard active, available().contains(route) else { return false }
    var ok = true
    do {
      switch route {
      case Route.speaker:
        try session.overrideOutputAudioPort(.speaker)
      case Route.earpiece:
        try session.overrideOutputAudioPort(.none)
        // The mode of a video call keeps the sound on the loudspeaker
        // whatever the input; at the ear it is a voice call's.
        if session.mode == .videoChat { try session.setMode(.voiceChat) }
        try session.setPreferredInput(port(inputs: [.builtInMic]))
      case Route.wired:
        try session.overrideOutputAudioPort(.none)
        // Headphones without a microphone take the output by themselves.
        try session.setPreferredInput(port(inputs: [.headsetMic, .usbAudio]) ?? port(inputs: [.builtInMic]))
      case Route.bluetooth:
        try session.overrideOutputAudioPort(.none)
        try session.setPreferredInput(port(inputs: [.bluetoothHFP, .bluetoothLE]))
      default:
        ok = false
      }
    } catch {
      callLog("sound: route \(route) refused: \(error)")
      ok = false
    }
    callLog("sound: route \(route) \(ok ? "taken" : "refused")")
    changed()
    return ok
  }

  /** The route in use, or nil while no call holds the sound. */
  static func current() -> String? {
    guard active else { return nil }
    for output in session.currentRoute.outputs {
      if let kind = outputKind(output.portType) { return kind }
    }
    return nil
  }

  /** `{ current, available }`, as the Rust side reads it. */
  static func describe() -> JsonObject {
    // A route of none is null, as the Kotlin side sends it.
    ["current": current().map { $0 as Any } ?? NSNull(), "available": available()]
  }

  /** The routes there are, in the one order of `Route.all`. */
  static func available() -> [String] {
    var kinds = Set<String>()
    // An iPad has no earpiece.
    if UIDevice.current.userInterfaceIdiom == .phone { kinds.insert(Route.earpiece) }
    kinds.insert(Route.speaker)
    for input in session.availableInputs ?? [] {
      if let kind = inputKind(input.portType) { kinds.insert(kind) }
    }
    // Headphones without a microphone are an output alone.
    for output in session.currentRoute.outputs {
      if let kind = outputKind(output.portType), kind == Route.wired || kind == Route.bluetooth {
        kinds.insert(kind)
      }
    }
    return Route.all.filter { kinds.contains($0) }
  }

  private static func port(inputs types: [AVAudioSession.Port]) -> AVAudioSessionPortDescription? {
    for type in types {
      if let port = session.availableInputs?.first(where: { $0.portType == type }) { return port }
    }
    return nil
  }

  private static func outputKind(_ type: AVAudioSession.Port) -> String? {
    switch type {
    case .builtInReceiver: return Route.earpiece
    case .builtInSpeaker: return Route.speaker
    case .headphones, .usbAudio: return Route.wired
    case .bluetoothHFP, .bluetoothA2DP, .bluetoothLE: return Route.bluetooth
    default: return nil
    }
  }

  private static func inputKind(_ type: AVAudioSession.Port) -> String? {
    switch type {
    case .headsetMic, .usbAudio: return Route.wired
    case .bluetoothHFP, .bluetoothLE: return Route.bluetooth
    default: return nil
    }
  }

  /**
   * The system changed the route. A headset put on during the call takes
   * the sound, as on a phone call; a choice whose device went is
   * forgotten. Every change is told.
   */
  private static func routeChanged(_ note: Notification) {
    guard active else { return }
    let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
    if AVAudioSession.RouteChangeReason(rawValue: raw) == .newDeviceAvailable {
      let available = available()
      let take: String? = available.contains(Route.bluetooth) ? Route.bluetooth : available.contains(Route.wired) ? Route.wired : nil
      if let take = take, set(take) { return }
    }
    changed()
  }

  private static func changed() {
    onChange?()
  }
}

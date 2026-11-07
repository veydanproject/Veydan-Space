// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Tauri
import UIKit

/** What the user pressed: `answer`, `decline`, `hangup`. */
struct Press {
  let callId: String
  let action: String

  var json: JsonObject { ["callId": callId, "action": action] }
}

/**
 * What the plugin, CallKit's delegate and the watchers share while the
 * process lives: the listeners of the app's Rust code and the presses
 * nobody heard (the twin of CallState.kt).
 *
 * A press can come before the app listens: CallKit may hand over an
 * answer before the Rust side has registered its listener. Such a press
 * waits here until the app asks with `takeActions`. A press while a
 * listener is in place goes to it at once.
 *
 * The listeners are kept here, not in Tauri's `Plugin`: its list is
 * private, and whether anyone listens decides between sending a press and
 * keeping it. `VeydanCallPlugin.registerListener` puts them here.
 *
 * Used on the main thread only.
 */
final class CallEvents {
  static let shared = CallEvents()

  static let action = "call_action"
  static let route = "audio_route_changed"
  static let network = "network_changed"

  /** The presses nobody heard; a few at most, the oldest go first. */
  private static let maxWaiting = 8
  private var waiting: [Press] = []
  private var listeners: [String: [Channel]] = [:]

  func listen(_ event: String, _ channel: Channel) {
    listeners[event, default: []].append(channel)
  }

  func remove(_ event: String, channelId: UInt64) {
    listeners[event]?.removeAll { $0.id == channelId }
  }

  private func hasListener(_ event: String) -> Bool {
    !(listeners[event]?.isEmpty ?? true)
  }

  private func send(_ event: String, _ data: JsonObject) {
    for channel in listeners[event] ?? [] {
      channel.send(data)
    }
  }

  /** A press: to the listener when there is one, kept for later otherwise. */
  func pressed(_ press: Press) {
    callLog("pressed \(press.action) on \(press.callId)")
    if hasListener(CallEvents.action) {
      send(CallEvents.action, press.json)
    } else {
      waiting.append(press)
      while waiting.count > CallEvents.maxWaiting { waiting.removeFirst() }
    }
  }

  /** The presses kept so far, oldest first, once. */
  func takeWaiting() -> [JsonObject] {
    let out = waiting.map { $0.json }
    waiting.removeAll()
    return out
  }

  /** The routes of the sound changed; nothing is kept, the app asks when it starts. */
  func routesChanged(_ routes: JsonObject) {
    send(CallEvents.route, routes)
  }

  /**
   * The phone's default network changed (`NetworkWatch`): `how` is `back`
   * after a loss, `other` for another network. Nothing is kept.
   */
  func networkChanged(_ how: String) {
    send(CallEvents.network, ["how": how])
  }
}

/** The plugin's lines in the system log, with one tag to find them by. */
func callLog(_ message: String) {
  NSLog("VeydanCall: %@", message)
}

/** An error of a command, as the Rust side gets its message. */
struct CallError: Error {
  let message: String

  init(_ message: String) {
    self.message = message
  }
}

/** The app as the phone names it: the name under its icon. */
func appName() -> String {
  let info = Bundle.main.infoDictionary
  if let name = info?["CFBundleDisplayName"] as? String, !name.isEmpty { return name }
  if let name = info?["CFBundleName"] as? String, !name.isEmpty { return name }
  return "Veydan"
}

/** The user looks at the app. On the main thread. */
func appInFront() -> Bool {
  UIApplication.shared.applicationState == .active
}

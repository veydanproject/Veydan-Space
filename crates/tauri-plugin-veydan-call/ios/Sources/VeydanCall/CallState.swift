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
 * nobody heard (the twin of CallState.kt); and, iOS alone, the VoIP
 * token of PushKit and the VoIP pushes nobody heard yet (`VoipPushes`).
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
  static let voipToken = "voip_token"
  static let voipPush = "voip_push"

  /** The presses nobody heard; a few at most, the oldest go first. */
  private static let maxWaiting = 8
  private var waiting: [Press] = []
  private var listeners: [String: [Channel]] = [:]

  /** The VoIP token PushKit gave this process (hex), and whether a change of it waits for a listener. */
  private(set) var currentVoipToken: String?
  private var voipTokenUntold = false
  /** The VoIP pushes nobody heard (the push started the process); a few at most. */
  private static let maxPushes = 4
  private var pushes: [(id: String, data: [String: String])] = []

  func listen(_ event: String, _ channel: Channel) {
    listeners[event, default: []].append(channel)
    if event == CallEvents.voipToken, voipTokenUntold {
      voipTokenUntold = false
      let data: JsonObject = ["token": currentVoipToken ?? ""]
      channel.send(data)
    }
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

  /**
   * PushKit gave a VoIP token, or took it back (nil): to the listener, or
   * to the first one that comes (the token comes at the launch, before
   * the app listens). The Rust side asks for it too (`getVoipToken`).
   */
  func voipTokenChanged(_ token: String?) {
    guard token != currentVoipToken else { return }
    currentVoipToken = token
    callLog(token == nil ? "PushKit took the VoIP token back" : "PushKit gave a VoIP token")
    if hasListener(CallEvents.voipToken) {
      send(CallEvents.voipToken, ["token": token ?? ""])
    } else {
      voipTokenUntold = true
    }
  }

  /**
   * A VoIP push whose placeholder rings (`CallKitBridge.ringFromVoipPush`):
   * to the listener when there is one, kept for `takeVoipPushes`
   * otherwise. Answers whether a listener had it.
   */
  func voipPushed(_ id: String, _ data: [String: String]) -> Bool {
    if hasListener(CallEvents.voipPush) {
      send(CallEvents.voipPush, ["pushId": id, "data": data])
      return true
    }
    pushes.append((id: id, data: data))
    while pushes.count > CallEvents.maxPushes { pushes.removeFirst() }
    return false
  }

  /** The VoIP pushes kept so far, oldest first, once. */
  func takeVoipPushes() -> [(id: String, data: [String: String])] {
    let out = pushes
    pushes.removeAll()
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

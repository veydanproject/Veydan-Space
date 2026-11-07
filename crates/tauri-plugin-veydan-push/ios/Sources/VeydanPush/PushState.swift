// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Tauri
import UIKit

/** What APNs answered to a registration. */
enum TokenResult {
  /** The device token as hex, the form the push server takes. */
  case token(String)
  case failed(String)
}

/**
 * What the plugin, the hooks of the app delegate and the delegate of the
 * notification center share while the process lives (the twin of
 * PushState.kt): the listeners of the app's Rust code, the tap nobody
 * asked about yet, the token of APNs, whether the app receives by itself,
 * the mute.
 *
 * The listeners are kept here, not in Tauri's `Plugin`: its list is
 * private, and a token renewed at the start, before the Rust side
 * listens, waits for the first listener (`VeydanPushPlugin.registerListener`
 * puts them here).
 *
 * Used on the main thread only.
 */
final class PushState {
  static let shared = PushState()

  static let tokenEvent = "token"
  static let tapEvent = "tap"
  /** The event the keeper of the channels listens to (Keeper.kt, `EVENT`). */
  static let keeperEvent = "keeper_wake"

  /** How long APNs is waited for: it answers in a moment, or never (no network, no entitlement). */
  private static let tokenWait: TimeInterval = 30

  /*
   * Kept in the app's defaults. The mute is the one thing the process a
   * push wakes has to know of the app; the token tells a renewal from the
   * token the push server has; `asked` is the user's agreement to pushes,
   * as `isAutoInitEnabled` keeps it for FCM on Android. When the extension
   * comes, the mute moves to the defaults of an app group it can read.
   */
  private static let mutedKey = "veydan_push.muted"
  private static let tokenKey = "veydan_push.token"
  private static let askedKey = "veydan_push.asked"

  private let defaults = UserDefaults.standard
  private var listeners: [String: [Channel]] = [:]

  /** The app receives messages by itself right now. Told by the app; false in a new process. */
  var live = false

  /** The tap nobody has asked about yet. */
  private var tap: Tap?

  /** The token APNs gave this process. */
  private var current: String?
  /** Those who wait for APNs' answer. */
  private var waiting: [(TokenResult) -> Void] = []
  private var timeout: DispatchWorkItem?
  /** A token renewed while nobody listened for tokens: told to the first listener. */
  private var untold = false

  // ─── Listeners ─────────────────────────────────────────────────────────

  func listen(_ event: String, _ channel: Channel) {
    listeners[event, default: []].append(channel)
    if event == PushState.tokenEvent, untold, let token = current {
      untold = false
      let data: JsonObject = ["token": token]
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

  // ─── The mute ──────────────────────────────────────────────────────────

  /**
   * The app wants no push shown: the part of it the pushes are for is
   * switched off, and the push server has not forgotten the phone yet.
   * Without the extension only what comes while the app is in front can
   * be held back (`PushCenter`); the system shows the rest.
   */
  var muted: Bool {
    get { defaults.bool(forKey: PushState.mutedKey) }
    set { defaults.set(newValue, forKey: PushState.mutedKey) }
  }

  // ─── Taps ──────────────────────────────────────────────────────────────

  /** A tap: kept for `takeTap`, and the app told when it listens (on a cold start nobody does yet). */
  func tapped(_ tap: Tap) {
    self.tap = tap
    send(PushState.tapEvent, [:])
  }

  func takeTap() -> Tap? {
    defer { tap = nil }
    return tap
  }

  // ─── The token ─────────────────────────────────────────────────────────

  /**
   * The user agreed to pushes before: the registration is made again at
   * every start, as Apple asks, so that a token APNs changed (a restore,
   * a reinstall) reaches the push server. Nothing is asked of APNs before
   * the first agreement.
   */
  func renewAtStart() {
    guard defaults.bool(forKey: PushState.askedKey) else { return }
    UIApplication.shared.registerForRemoteNotifications()
  }

  /** The token of this process at once, or APNs asked for it. */
  func askToken(_ done: @escaping (TokenResult) -> Void) {
    defaults.set(true, forKey: PushState.askedKey)
    if let token = current {
      done(.token(token))
      return
    }
    waiting.append(done)
    UIApplication.shared.registerForRemoteNotifications()
    if timeout == nil {
      let item = DispatchWorkItem { [weak self] in
        self?.timeout = nil
        self?.answer(.failed("APNs did not answer in \(Int(PushState.tokenWait)) s"))
      }
      timeout = item
      DispatchQueue.main.asyncAfter(deadline: .now() + PushState.tokenWait, execute: item)
    }
  }

  /** The token is given up: APNs stops delivering to this app until it is asked again. */
  func deleteToken() {
    defaults.set(false, forKey: PushState.askedKey)
    defaults.removeObject(forKey: PushState.tokenKey)
    current = nil
    untold = false
    UIApplication.shared.unregisterForRemoteNotifications()
  }

  /** APNs gave the token (application:didRegisterForRemoteNotificationsWithDeviceToken:). */
  func registered(_ deviceToken: Data) {
    let token = deviceToken.map { String(format: "%02x", $0) }.joined()
    let before = defaults.string(forKey: PushState.tokenKey)
    current = token
    defaults.set(token, forKey: PushState.tokenKey)
    if answer(.token(token)) { return }
    // A renewal at the start: the Rust side learns of a change as Android's
    // does from onNewToken, and keeps its own copy otherwise.
    guard token != before else { return }
    pushLog("APNs gave a new token")
    if hasListener(PushState.tokenEvent) {
      send(PushState.tokenEvent, ["token": token])
    } else {
      untold = true
    }
  }

  /** APNs refused (application:didFailToRegisterForRemoteNotificationsWithError:). */
  func failedToRegister(_ error: Error) {
    pushLog("APNs gave no token: \(error.localizedDescription)")
    answer(.failed(error.localizedDescription))
  }

  /** Answers those who wait; whether anyone did. */
  @discardableResult
  private func answer(_ result: TokenResult) -> Bool {
    timeout?.cancel()
    timeout = nil
    let asked = waiting
    waiting.removeAll()
    asked.forEach { $0(result) }
    return !asked.isEmpty
  }

  // ─── The keeper ────────────────────────────────────────────────────────

  /** The keeper of the channels takes a turn; whether anyone listened (the module may be off). */
  func keeperWake(_ why: String) -> Bool {
    guard hasListener(PushState.keeperEvent) else { return false }
    send(PushState.keeperEvent, ["why": why])
    return true
  }
}

/** The plugin's lines in the system log, with one tag to find them by. */
func pushLog(_ message: String) {
  NSLog("VeydanPush: %@", message)
}

/** An error of a command, as the Rust side gets its message. */
struct PushError: Error {
  let message: String

  init(_ message: String) {
    self.message = message
  }
}

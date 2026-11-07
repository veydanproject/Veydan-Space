// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Tauri
import UIKit
import UserNotifications
import WebKit

/*
 * The arguments of the commands, as the Rust side sends them (src/lib.rs).
 * Every field is optional and takes the Kotlin side's default when
 * missing, so that both sides read the same JSON the same way.
 */

struct ContextArgs: Decodable {
  let live: Bool?
}

struct MutedArgs: Decodable {
  let muted: Bool?
}

struct CancelArgs: Decodable {
  let key: String?
}

struct KeeperArgs: Decodable {
  let on: Bool?
}

struct KeysArgs: Decodable {
  /** Base64 of the bundle. */
  let bundle: String?
}

struct ListenerArgs: Decodable {
  let event: String
  let handler: Channel
}

struct RemoveListenerArgs: Decodable {
  let event: String
  let channelId: UInt64
}

/**
 * The app's side of the push bridge on iOS. Called from the app's Rust code
 * only, by the same commands and with the same answers as the Kotlin side
 * (VeydanPushPlugin.kt); see src/lib.rs for the contract.
 *
 * Tauri calls a command on a queue of its own; the state lives on the main
 * thread, where UIKit and the callbacks of APNs are, and every command
 * that touches it is carried there.
 */
class VeydanPushPlugin: Plugin {
  override func load(webview: WKWebView) {
    // Made at the plugin's creation already (`initPlugin`); a second call
    // does nothing.
    PushHooks.install()
  }

  /** Runs `work` on the main thread, and answers with what it returns. */
  private func onMain(_ invoke: Invoke, _ work: @escaping () throws -> JsonObject?) {
    DispatchQueue.main.async {
      do {
        if let answer = try work() {
          invoke.resolve(answer)
        } else {
          invoke.resolve()
        }
      } catch let error as PushError {
        pushLog("push command failed: \(error.message)")
        invoke.reject(error.message)
      } catch {
        pushLog("push command failed: \(error)")
        invoke.reject("\(error)")
      }
    }
  }

  private var appId: String {
    Bundle.main.bundleIdentifier ?? ""
  }

  /*
   * The listeners. Tauri's own `registerListener` keeps its channels where
   * this class cannot ask whether anyone listens, which decides whether a
   * token renewed at the start is sent or kept (`PushState`). Tauri's
   * dispatcher looks for `<command>:completionHandler:` before
   * `<command>:error:` (PluginManager.invoke in Tauri.swift), so these take
   * the commands `registerListener` and `removeListener` of this plugin.
   */

  /*
   * Answered at once, before the main thread has taken the listener: the
   * app registers listeners while it starts its modules, which the shell
   * does inside a block_on on the main thread, so an answer that waited
   * for the main thread would never come (the fifteenth run of the lab: a
   * white webview, the start never over). The events go out from the main
   * thread, after the registration queued here.
   */
  @objc func registerListener(_ invoke: Invoke, completionHandler: @escaping (NSError?) -> Void) {
    do {
      let args = try invoke.parseArgs(ListenerArgs.self)
      DispatchQueue.main.async { PushState.shared.listen(args.event, args.handler) }
      invoke.resolve()
      completionHandler(nil)
    } catch {
      completionHandler(error as NSError)
    }
  }

  @objc func removeListener(_ invoke: Invoke, completionHandler: @escaping (NSError?) -> Void) {
    do {
      let args = try invoke.parseArgs(RemoveListenerArgs.self)
      DispatchQueue.main.async { PushState.shared.remove(args.event, channelId: args.channelId) }
      invoke.resolve()
      completionHandler(nil)
    } catch {
      completionHandler(error as NSError)
    }
  }

  /**
   * What is known without a word to APNs: `{ available, permission, appId,
   * quiet? }`. APNs is part of the system, so pushes are always available
   * here; whether a token comes is `getToken`'s to say.
   */
  @objc func getState(_ invoke: Invoke) {
    let appId = self.appId
    UNUserNotificationCenter.current().getNotificationSettings { settings in
      var answer: JsonObject = [
        "available": true,
        "permission": Permission.name(settings.authorizationStatus),
        "appId": appId,
      ]
      if let quiet = Permission.quiet(settings) { answer["quiet"] = quiet }
      invoke.resolve(answer)
    }
  }

  /**
   * The settings of the app's notifications, where messages are made to
   * ring again: iOS 16 opens them at once, iOS 15 the app's page of the
   * settings, one tap away.
   */
  @objc func openSettings(_ invoke: Invoke) {
    DispatchQueue.main.async {
      var link = UIApplication.openSettingsURLString
      if #available(iOS 16.0, *) {
        link = UIApplication.openNotificationSettingsURLString
      }
      guard let url = URL(string: link) else {
        invoke.reject("the settings did not open: no address")
        return
      }
      UIApplication.shared.open(url) { opened in
        if opened {
          invoke.resolve()
        } else {
          invoke.reject("the settings did not open")
        }
      }
    }
  }

  /**
   * Registers with APNs; answers `{ appId, available, token? | reason,
   * detail }` when APNs does, with the token of this process at once when
   * it has one. The first call is the first contact with APNs: the app
   * makes it only after the user agreed to pushes.
   */
  @objc func getToken(_ invoke: Invoke) {
    let appId = self.appId
    DispatchQueue.main.async {
      PushState.shared.askToken { result in
        switch result {
        case .token(let token):
          invoke.resolve(["appId": appId, "available": true, "token": token])
        case .failed(let detail):
          invoke.resolve(["appId": appId, "available": false, "reason": "token_failed", "detail": detail])
        }
      }
    }
  }

  @objc func deleteToken(_ invoke: Invoke) {
    onMain(invoke) {
      PushState.shared.deleteToken()
      return nil
    }
  }

  @objc func permissionState(_ invoke: Invoke) {
    UNUserNotificationCenter.current().getNotificationSettings { settings in
      invoke.resolve(["state": Permission.name(settings.authorizationStatus)])
    }
  }

  /**
   * The system's question, when it was never answered; then `{ state }`.
   * Asked once more, the system shows nothing and answers what it knows.
   */
  @objc func requestPermission(_ invoke: Invoke) {
    let center = UNUserNotificationCenter.current()
    center.requestAuthorization(options: [.alert, .sound, .badge]) { _, error in
      if let error = error {
        pushLog("the question of notifications failed: \(error.localizedDescription)")
      }
      center.getNotificationSettings { settings in
        invoke.resolve(["state": Permission.name(settings.authorizationStatus)])
      }
    }
  }

  @objc func setContext(_ invoke: Invoke) throws {
    let live = try invoke.parseArgs(ContextArgs.self).live ?? false
    onMain(invoke) {
      PushState.shared.live = live
      return nil
    }
  }

  @objc func setMuted(_ invoke: Invoke) throws {
    let muted = try invoke.parseArgs(MutedArgs.self).muted ?? false
    onMain(invoke) {
      PushState.shared.muted = muted
      return nil
    }
  }

  /** `{ tap? }`: the tapped notification, once. */
  @objc func takeTap(_ invoke: Invoke) {
    onMain(invoke) {
      guard let tap = PushState.shared.takeTap() else { return [:] }
      return ["tap": tap.json]
    }
  }

  /** Removes the shown notifications of one chat (`dm`, `dm:<pubkey>`, `group:<id>`), or all about messages. */
  @objc func cancel(_ invoke: Invoke) throws {
    let key = try invoke.parseArgs(CancelArgs.self).key
    if let key = key, key != "dm", !Push.isChatKey(key) {
      invoke.reject("not a chat: \(key)")
      return
    }
    let center = UNUserNotificationCenter.current()
    center.getDeliveredNotifications { shown in
      let gone = shown.filter { Push.clearedBy(key, $0.request.content.userInfo) }.map { $0.request.identifier }
      if !gone.isEmpty {
        center.removeDeliveredNotifications(withIdentifiers: gone)
      }
      invoke.resolve()
    }
  }

  @objc func storeKeys(_ invoke: Invoke) throws {
    guard let b64 = try invoke.parseArgs(KeysArgs.self).bundle, !b64.isEmpty else {
      invoke.reject("no bundle")
      return
    }
    guard var bytes = Data(base64Encoded: b64) else {
      invoke.reject("the keys were not kept: not base64")
      return
    }
    do {
      try Keys.store(bytes)
      bytes.resetBytes(in: 0..<bytes.count)
      invoke.resolve()
    } catch let error as PushError {
      bytes.resetBytes(in: 0..<bytes.count)
      invoke.reject(error.message)
    } catch {
      bytes.resetBytes(in: 0..<bytes.count)
      invoke.reject("the keys were not kept: \(error)")
    }
  }

  @objc func clearKeys(_ invoke: Invoke) {
    Keys.clear()
    invoke.resolve()
  }

  /**
   * The keeper's foreground service of Android: iOS has none, so it never
   * runs (`{ running: false, supported: false }`). The keeper takes its
   * turns while the app runs, and at a background push.
   */
  @objc func keeperForeground(_ invoke: Invoke) {
    invoke.resolve(["running": false, "supported": false])
  }

  /**
   * The keeper's periodic wake: not on iOS yet, where it would be a
   * background task the system runs when it sees fit (BGAppRefreshTask, a
   * later stage). Answers `{ supported: false }`.
   */
  @objc func keeperPeriodic(_ invoke: Invoke) {
    invoke.resolve(["supported": false])
  }
}

/** The permission of notifications in the words of the Kotlin side (`PermissionState` of Tauri). */
enum Permission {
  static func name(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .notDetermined: return "prompt"
    case .denied: return "denied"
    case .authorized, .provisional, .ephemeral: return "granted"
    @unknown default: return "denied"
    }
  }

  /**
   * What keeps messages from ringing (`quiet` of the state), as Android
   * says it: `off` when notifications are not allowed, or allowed and
   * shown nowhere (not asked yet is off too, as on Android 13); `dm` when
   * they come without a sound, or quietly into the list (provisional): iOS
   * has no channels, and the first of Android's names stands for all of
   * them. None when they ring.
   */
  static func quiet(_ settings: UNNotificationSettings) -> String? {
    switch settings.authorizationStatus {
    case .notDetermined, .denied: return "off"
    case .provisional: return "dm"
    default: break
    }
    let shown = [settings.alertSetting, settings.lockScreenSetting, settings.notificationCenterSetting]
    if !shown.contains(.enabled) { return "off" }
    if settings.soundSetting != .enabled { return "dm" }
    return nil
  }
}

/** What the plugin puts in place at its load, once a process. */
enum PushHooks {
  private static var renewed = false

  /**
   * On the main thread. Called again at the load of the webview, which
   * finishes what found no app delegate the first time.
   */
  static func install() {
    PushCenter.install()
    // The token's renewal answers through the app delegate's hooks.
    guard AppDelegateHooks.install(), !renewed else { return }
    renewed = true
    PushState.shared.renewAtStart()
  }
}

/**
 * Made by Tauri's setup (`register_ios_plugin`), on the main thread and
 * inside application:didFinishLaunchingWithOptions:, before the webview
 * loads the plugin: the hooks go in now, so that the delegate of the
 * notification center is set before the launch is over and a tap that
 * launched the app reaches it.
 */
@_cdecl("init_plugin_veydan_push")
func initPlugin() -> Plugin {
  if Thread.isMainThread {
    PushHooks.install()
  } else {
    DispatchQueue.main.async { PushHooks.install() }
  }
  return VeydanPushPlugin()
}

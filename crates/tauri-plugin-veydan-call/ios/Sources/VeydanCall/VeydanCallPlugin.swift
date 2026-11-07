// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Tauri
import UIKit
import WebKit

/*
 * The arguments of the commands, as the Rust side sends them (src/lib.rs:
 * `Incoming`, `Ongoing`, `GroupCallNotice` and the small objects of each
 * command). Every field is optional and takes the Kotlin side's default
 * when missing, so that both sides read the same JSON the same way.
 */

struct IncomingArgs: Decodable {
  let callId: String?
  let name: String?
  /** A picture of the caller: CallKit shows none, so it is not read. */
  let avatar: String?
  let video: Bool?
  /** Who calls stays off the lock screen; hidden when not said. */
  let hideOnLockscreen: Bool?
}

struct DismissArgs: Decodable {
  let callId: String?
}

struct OngoingArgs: Decodable {
  let callId: String?
  let name: String?
  let video: Bool?
  let hideOnLockscreen: Bool?
}

struct GroupCallArgs: Decodable {
  let callId: String?
  /** The chat of the group (`group:<id>`), which a tap opens. */
  let chatId: String?
  let name: String?
  let video: Bool?
  let hideOnLockscreen: Bool?
}

struct RouteArgs: Decodable {
  let route: String?
}

struct AwakeArgs: Decodable {
  let on: Bool?
}

struct CameraArgs: Decodable {
  /** `front` (the default) or `back`. */
  let facing: String?
  let width: Int?
  let height: Int?
}

struct PermissionArgs: Decodable {
  /** `camera` or `microphone`. */
  let permission: String?
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
 * The app's side of the call bridge on iOS. Called from the app's Rust code
 * only, by the same commands and with the same answers as the Kotlin side
 * (VeydanCallPlugin.kt); see src/lib.rs for the contract.
 *
 * Tauri calls a command on a queue of its own; the state of a call lives
 * on the main thread, where CallKit's delegate, the audio session's
 * notifications and UIKit are, and every command is carried there.
 */
class VeydanCallPlugin: Plugin {
  override func load(webview: WKWebView) {
    callLog("loading")
    CallKitBridge.shared.load()
    // The engine waits for CallKit's audio session before it starts its sound.
    RtcAudio.prepare()
    // The notices of calls on in groups a dead process left go.
    GroupCallNotices.loaded()
    // A change of the network is told to the app at once (a lost way is
    // restarted without waiting for the engine to notice).
    NetworkWatch.start()
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
      } catch let error as CallError {
        callLog("call command failed: \(error.message)")
        invoke.reject(error.message)
      } catch {
        callLog("call command failed: \(error)")
        invoke.reject("\(error)")
      }
    }
  }

  /*
   * The listeners. Tauri's own `registerListener` keeps its channels where
   * this class cannot ask whether anyone listens, which decides between
   * sending a press and keeping it (`CallEvents`). Tauri's dispatcher
   * looks for `<command>:completionHandler:` before `<command>:error:`
   * (PluginManager.invoke in Tauri.swift), so these take the commands
   * `registerListener` and `removeListener` of this plugin.
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
      DispatchQueue.main.async { CallEvents.shared.listen(args.event, args.handler) }
      invoke.resolve()
      completionHandler(nil)
    } catch {
      completionHandler(error as NSError)
    }
  }

  @objc func removeListener(_ invoke: Invoke, completionHandler: @escaping (NSError?) -> Void) {
    do {
      let args = try invoke.parseArgs(RemoveListenerArgs.self)
      DispatchQueue.main.async { CallEvents.shared.remove(args.event, channelId: args.channelId) }
      invoke.resolve()
      completionHandler(nil)
    } catch {
      completionHandler(error as NSError)
    }
  }

  /** Rings through CallKit; answers `{ foreground, fullScreen, notifications, ringing, inFront, busyWith? }`. */
  @objc func showIncoming(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(IncomingArgs.self)
    guard let callId = args.callId, !callId.isEmpty else {
      invoke.reject("no call id")
      return
    }
    let call = CallInfo(
      callId: String(callId.prefix(128)), name: String((args.name ?? "").prefix(200)),
      video: args.video ?? false, hidden: args.hideOnLockscreen ?? true)
    DispatchQueue.main.async {
      CallKitBridge.shared.showIncoming(call) { answer in invoke.resolve(answer) }
    }
  }

  @objc func dismissIncoming(_ invoke: Invoke) throws {
    let callId = try invoke.parseArgs(DismissArgs.self).callId
    onMain(invoke) {
      CallKitBridge.shared.dismissIncoming(callId)
      return nil
    }
  }

  /**
   * A call is on in a group: a quiet notification (no sound, no buttons)
   * unless the app is in front; a tap opens the group's chat. Answers
   * `{ shown }`.
   */
  @objc func showGroupCall(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(GroupCallArgs.self)
    guard let callId = args.callId, !callId.isEmpty, let chatId = args.chatId, !chatId.isEmpty else {
      invoke.reject("no call or chat id")
      return
    }
    let notice = GroupCallNotice(
      callId: String(callId.prefix(128)), chatId: String(chatId.prefix(128)),
      name: String((args.name ?? "").prefix(200)), video: args.video ?? false,
      hidden: args.hideOnLockscreen ?? true)
    DispatchQueue.main.async {
      GroupCallNotices.show(notice) { shown in invoke.resolve(["shown": shown]) }
    }
  }

  @objc func dismissGroupCall(_ invoke: Invoke) throws {
    guard let callId = try invoke.parseArgs(DismissArgs.self).callId, !callId.isEmpty else {
      invoke.reject("no call id")
      return
    }
    onMain(invoke) {
      GroupCallNotices.dismiss(callId)
      return nil
    }
  }

  @objc func startOngoing(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(OngoingArgs.self)
    guard let callId = args.callId, !callId.isEmpty else {
      invoke.reject("no call id")
      return
    }
    let call = CallInfo(
      callId: String(callId.prefix(128)), name: String((args.name ?? "").prefix(200)),
      video: args.video ?? false, hidden: args.hideOnLockscreen ?? true)
    onMain(invoke) {
      CallKitBridge.shared.startOngoing(call)
      return nil
    }
  }

  @objc func stop(_ invoke: Invoke) {
    onMain(invoke) {
      CallKitBridge.shared.stop()
      return nil
    }
  }

  @objc func setAudioRoute(_ invoke: Invoke) throws {
    let route = try invoke.parseArgs(RouteArgs.self).route ?? ""
    guard Route.all.contains(route) else {
      invoke.reject("not a route: \(route)")
      return
    }
    onMain(invoke) {
      guard AudioRoutes.active else { throw CallError("no call holds the sound") }
      guard AudioRoutes.set(route) else { throw CallError("the phone has no \(route) now") }
      return AudioRoutes.describe()
    }
  }

  @objc func listAudioRoutes(_ invoke: Invoke) {
    onMain(invoke) { AudioRoutes.describe() }
  }

  @objc func keepAwake(_ invoke: Invoke) throws {
    let on = try invoke.parseArgs(AwakeArgs.self).on ?? false
    onMain(invoke) {
      CallKitBridge.shared.keepAwake(on)
      return nil
    }
  }

  @objc func startCamera(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CameraArgs.self)
    let facing = args.facing == Camera.back ? Camera.back : Camera.front
    onMain(invoke) {
      try Camera.shared.start(facing: facing, width: args.width ?? 640, height: args.height ?? 360)
      return nil
    }
  }

  @objc func stopCamera(_ invoke: Invoke) {
    onMain(invoke) {
      Camera.shared.stop()
      return nil
    }
  }

  @objc func switchCamera(_ invoke: Invoke) {
    onMain(invoke) {
      Camera.shared.switchFacing()
      return nil
    }
  }

  @objc func cameraStats(_ invoke: Invoke) {
    invoke.resolve(Camera.shared.describe())
  }

  /**
   * The system's question for the camera or the microphone, when it was
   * never answered; answers `{ granted }` after it. A permission denied
   * once is answered without a question: the system shows none.
   */
  @objc func requestPermission(_ invoke: Invoke) throws {
    let permission = try invoke.parseArgs(PermissionArgs.self).permission ?? ""
    guard permission == Permissions.camera || permission == Permissions.microphone else {
      invoke.reject("not a permission: \(permission)")
      return
    }
    DispatchQueue.main.async {
      Permissions.request(permission) { granted in invoke.resolve(["granted": granted]) }
    }
  }

  @objc func takeActions(_ invoke: Invoke) {
    onMain(invoke) { ["actions": CallEvents.shared.takeWaiting()] }
  }
}

@_cdecl("init_plugin_veydan_call")
func initPlugin() -> Plugin {
  return VeydanCallPlugin()
}

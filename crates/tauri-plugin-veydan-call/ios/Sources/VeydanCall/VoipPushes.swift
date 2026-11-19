// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import PushKit
import UIKit

/**
 * How long a placeholder of a VoIP push rings at most without a call
 * taking its place: the life of an invitation (`CALL_INVITE_TTL_SECS`,
 * 45 s from its making). The app answers much sooner once it runs with
 * its keys: it gives the opening of a push seven seconds and then ends
 * the placeholder itself (`DESCRIBE_WITHIN` of messenger-app's voip.rs).
 * This limit is for the rest: the runtime not up yet (a cold start), the
 * session not open (the app's PIN), the app gone.
 */
enum VoipLimits {
  static let placeholder: TimeInterval = 45
}

/**
 * PushKit: the second token of an iPhone, and the VoIP pushes the push
 * server sends to it, which are the invitations to a call alone (VPush
 * spec/protocol.md, «VoIP pushes»). iOS hands such a push to the app at
 * once, waking it or starting it in the background, and shows nothing
 * itself: the app reports it to CallKit before the handler returns
 * (`CallKitBridge.ringFromVoipPush`), or iOS kills it and, after a few
 * times, hands it no VoIP push at all.
 *
 * The registry is made at every launch, the background ones too: PushKit
 * delivers a push only to a registry that exists. Tauri's builder makes
 * this plugin before UIApplicationMain (`initPlugin`), so the registry is
 * made when UIKit says the launch is over; the plugin's load, which on the
 * scene path waits for a scene that a launch in the background never
 * connects, makes it too if it is not there yet.
 *
 * The token goes to the app's Rust code as the push plugin's token does
 * (`CallEvents.voipTokenChanged`, the `voip_token` event, `getVoipToken`):
 * hex, and empty when PushKit takes it back. The app puts it into the
 * registration at the push server.
 *
 * PushKit needs no entitlement beyond `aps-environment`, which the push
 * plugin writes; the background mode `voip` is in the app's Info.plist.
 *
 * On the main thread (the registry's queue).
 */
final class VoipPushes: NSObject, PKPushRegistryDelegate {
  static let shared = VoipPushes()

  private var registry: PKPushRegistry?
  private var launchObserver: NSObjectProtocol?

  /** At the plugin's creation, before UIApplicationMain: the registry is made at the end of the launch. */
  func startAtLaunch() {
    guard registry == nil, launchObserver == nil else { return }
    launchObserver = NotificationCenter.default.addObserver(
      forName: UIApplication.didFinishLaunchingNotification, object: nil, queue: nil
    ) { [weak self] _ in
      self?.start()
    }
  }

  /** The registry, once a process. */
  func start() {
    if let observer = launchObserver {
      NotificationCenter.default.removeObserver(observer)
      launchObserver = nil
    }
    guard registry == nil else { return }
    let registry = PKPushRegistry(queue: .main)
    registry.delegate = self
    registry.desiredPushTypes = [.voIP]
    self.registry = registry
    callLog("PushKit is asked for VoIP pushes")
  }

  // MARK: PKPushRegistryDelegate

  func pushRegistry(_ registry: PKPushRegistry, didUpdate pushCredentials: PKPushCredentials, for type: PKPushType) {
    guard type == .voIP else { return }
    let token = pushCredentials.token.map { String(format: "%02x", $0) }.joined()
    CallEvents.shared.voipTokenChanged(token.isEmpty ? nil : token)
  }

  func pushRegistry(_ registry: PKPushRegistry, didInvalidatePushTokenFor type: PKPushType) {
    guard type == .voIP else { return }
    CallEvents.shared.voipTokenChanged(nil)
  }

  /**
   * A VoIP push: reported to CallKit before `completion`, whatever it is
   * (`CallKitBridge.ringFromVoipPush`). PushKit gives this handler little
   * time: nothing is opened here.
   */
  func pushRegistry(
    _ registry: PKPushRegistry, didReceiveIncomingPushWith payload: PKPushPayload, for type: PKPushType,
    completion: @escaping () -> Void
  ) {
    guard type == .voIP else {
      completion()
      return
    }
    CallKitBridge.shared.ringFromVoipPush(VoipPush(payload.dictionaryPayload), reported: completion)
  }
}

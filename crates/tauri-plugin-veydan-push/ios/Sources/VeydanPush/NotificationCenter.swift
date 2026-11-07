// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import UserNotifications

/**
 * The delegate of the app's notification center: what is shown while the
 * app is in front, and the taps.
 *
 * This plugin builds no notification. An alert push is shown by the
 * system as it came: the app's string `push.new` ("New message", in the
 * language of the phone) under the app's name, grouped by its `thread-id`.
 * The Notification Service Extension, a later stage, will write the text
 * from the event as Android does; until then every message, a call's
 * invitation among them, reads "New message".
 *
 * - `willPresent`, the app in front: a push about messages is not shown
 *   while the app receives them by itself (`setContext`), none while it
 *   has muted pushes; the rest comes as a banner with its sound. The
 *   app's own notices (a call on in a group) are shown: their plugin shows
 *   none while the app is in front.
 * - `didReceive`: a tap is kept for `takeTap` and the app told with
 *   `tap`. A tap that launched the app comes here too: the delegate is set
 *   by Tauri's setup, which runs inside
 *   application:didFinishLaunchingWithOptions: (tao, without scenes), and
 *   the system hands the response to a delegate set before the launch is
 *   over. Nobody listens yet then; the page asks with `takeTap`.
 *
 * Nothing else in the app sets this delegate.
 */
final class PushCenter: NSObject, UNUserNotificationCenterDelegate {
  static let shared = PushCenter()

  static func install() {
    let center = UNUserNotificationCenter.current()
    if center.delegate !== shared {
      center.delegate = shared
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    let info = notification.request.content.userInfo
    DispatchQueue.main.async {
      guard let type = Push.kind(info) else {
        completionHandler([.banner, .list, .sound])
        return
      }
      let state = PushState.shared
      if state.muted {
        pushLog("push type=\(type) in front: not shown, muted by the app")
        completionHandler([])
      } else if Push.aboutMessage(type) && state.live {
        // The message is on the screen already.
        pushLog("push type=\(type) in front: not shown, the app is live")
        completionHandler([])
      } else {
        completionHandler([.banner, .list, .sound])
      }
    }
  }

  func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let info = response.notification.request.content.userInfo
    let opened = response.actionIdentifier == UNNotificationDefaultActionIdentifier
    DispatchQueue.main.async {
      if opened, let tap = Push.tap(info) {
        pushLog("tap type=\(tap.type)\(tap.chat == nil ? "" : " on a chat")")
        PushState.shared.tapped(tap)
      }
      completionHandler()
    }
  }
}

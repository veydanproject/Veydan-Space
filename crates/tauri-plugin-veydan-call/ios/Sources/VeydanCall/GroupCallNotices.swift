// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import UserNotifications

/** A call on in a group, as the app announces it. */
struct GroupCallNotice {
  let callId: String
  /** The chat of the group (`group:<id>`), which a tap opens. */
  let chatId: String
  let name: String
  let video: Bool
  let hidden: Bool
}

/**
 * The quiet notification of a call on in a group: the group's name over
 * "A call is on", no sound, no buttons; a tap opens the group's chat,
 * whose banner has Join. Not CallKit: nothing rings, and a call I am not
 * in is no call of this phone's.
 *
 * Shown once per call, not while the app is what the user looks at, and
 * not for a call the phone knows already (`GroupCallMemory`, kept in the
 * app's defaults across processes, as the Kotlin side keeps it in its
 * preferences).
 *
 * iOS shows a notification over the lock screen as the user's "Show
 * Previews" says, one version for all; it has no public version of a
 * notification of its own as Android has. So a call whose group stays off
 * the lock screen (`hidden`) never names the group: its notification says
 * only that a call is on, unlocked too.
 *
 * The tap is the app's to read: the notification carries the same extras
 * as the Kotlin side's (`veydan_push_type` = `group`, `veydan_push_chat`),
 * which the push side opens the chat by.
 *
 * Used on the main thread.
 */
enum GroupCallNotices {
  /** How long a call on in a group is remembered: longer than any call goes on. */
  private static let keptMs: Int64 = 24 * 60 * 60 * 1000
  private static let defaultsKey = "veydan_call.group_calls"
  private static var loadedMemory: GroupCallMemory?

  private static var memory: GroupCallMemory {
    if let memory = loadedMemory { return memory }
    let memory = GroupCallMemory.decode(UserDefaults.standard.string(forKey: defaultsKey), keptMs: keptMs)
    loadedMemory = memory
    return memory
  }

  private static func save() {
    UserDefaults.standard.set(memory.encode(), forKey: defaultsKey)
  }

  private static func identifier(_ callId: String) -> String {
    "veydan-call-group-\(callId)"
  }

  /**
   * The process starts: the notices of calls on in groups a dead process
   * showed go (the call may have ended while no process lived to take the
   * notice down; the app announces a call still on anew).
   */
  static func loaded() {
    let leftovers = memory.leftovers()
    guard !leftovers.isEmpty else { return }
    callLog("the notices of calls on in groups left by a dead process go: \(leftovers)")
    UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: leftovers.map(identifier))
    save()
  }

  /** Shows the notice of `notice` unless it is no news; `done` on the main thread with whether it was shown. */
  static func show(_ notice: GroupCallNotice, done: @escaping (Bool) -> Void) {
    if !memory.isNews(notice.callId, now: Clock.wallMs()) {
      callLog("a call is on in a group (\(notice.callId)); known here already, not shown")
      done(false)
      return
    }
    if appInFront() {
      callLog("a call is on in a group (\(notice.callId)); the app is in front, not shown")
      done(false)
      return
    }
    let center = UNUserNotificationCenter.current()
    center.getNotificationSettings { settings in
      let allowed = settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional
      guard allowed else {
        DispatchQueue.main.async { done(false) }
        return
      }
      let content = UNMutableNotificationContent()
      let text = Strings.groupOn(video: notice.video)
      if notice.hidden {
        content.title = text
      } else {
        content.title = notice.name.isEmpty ? appName() : notice.name
        content.body = text
      }
      content.sound = nil
      content.threadIdentifier = notice.chatId
      content.userInfo = ["veydan_push_type": "group", "veydan_push_chat": notice.chatId]
      if #available(iOS 15.0, *) {
        // Into the list, without lighting the screen or a sound.
        content.interruptionLevel = .passive
      }
      let request = UNNotificationRequest(identifier: GroupCallNotices.identifier(notice.callId), content: content, trigger: nil)
      center.add(request) { error in
        DispatchQueue.main.async {
          let shown = error == nil
          if shown {
            GroupCallNotices.memory.shown(notice.callId, now: Clock.wallMs())
            GroupCallNotices.save()
          }
          callLog("a call is on in a group (\(notice.callId), video=\(notice.video), hidden=\(notice.hidden)): shown=\(shown)")
          done(shown)
        }
      }
    }
  }

  /** The notice of a call on in a group goes: the call is over, or I am in it. */
  static func dismiss(_ callId: String) {
    let center = UNUserNotificationCenter.current()
    center.removeDeliveredNotifications(withIdentifiers: [identifier(callId)])
    center.removePendingNotificationRequests(withIdentifiers: [identifier(callId)])
    if memory.dismissed(callId) { save() }
  }

  /**
   * A call that went on here is one I was in: no news of it later,
   * whatever process hears of it (a room of a group call is announced
   * again at every seat taken, also after I leave it).
   */
  static func joined(_ callId: String) {
    memory.joined(callId, now: Clock.wallMs())
    save()
  }
}

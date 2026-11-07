// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation

/** A notification the user tapped: what the app opens (the twin of `Tap` of PushState.kt). */
struct Tap {
  let type: String
  /** `group:<id>` and the other keys of `Push.isChatKey`; none for a direct message the server cannot name. */
  let chat: String?

  var json: [String: Any] {
    var out: [String: Any] = ["type": type]
    if let chat = chat { out["chat"] = chat }
    return out
  }
}

/**
 * A push as APNs hands it over (services/push/spec/protocol.md, "Through
 * APNs"): the keys FCM carries as its data, every value a string, beside
 * `aps`. Everything here came from outside and is read so: what does not
 * look right is not taken (the rules of Push.kt).
 */
enum Push {
  static let typeDm = "dm"
  static let typeGroup = "group"
  static let typeSync = "sync"
  static let typeSealed = "sealed"

  /** The extras of a notification the app shows itself, as on Android (Notifier.kt, the call plugin's group notices). */
  static let extraType = "veydan_push_type"
  static let extraChat = "veydan_push_chat"

  /** The mark of a note to the keeper of a channel on the outside of the wrap (`pushtags::CHANNEL`, Keeper.kt). */
  private static let keeperMark = "channel"

  private static let typePattern = "^[a-z_]{1,32}$"
  private static let hex64Pattern = "^[0-9a-f]{64}$"
  /** As `CHAT` of ui/src/lib/messenger/push/bridge.ts and Push.kt. */
  private static let chatPattern = "^(dm:[0-9a-f]{64}(@[0-9a-f]{16})?|group:[0-9a-f]{64}|channel:[0-9a-f]{64})$"

  /**
   * The whole of `value` matches: ICU's `$` also matches before a line
   * break that ends the text, so a match that leaves one out is none.
   */
  private static func matches(_ value: String, _ pattern: String) -> Bool {
    value.range(of: pattern, options: .regularExpression) == value.startIndex..<value.endIndex
  }

  /** `dm:<pubkey>`, `dm:<pubkey>@<server id>`, `group:<id>` or `channel:<id>`: what a tap opens. */
  static func isChatKey(_ key: String) -> Bool {
    matches(key, chatPattern)
  }

  /** A push of the push server: a remote notification carries `aps`, one the app made does not. */
  static func isRemote(_ info: [AnyHashable: Any]) -> Bool {
    info["aps"] is [AnyHashable: Any]
  }

  /** The type of a push of the push server, when it is one and the type looks right. */
  static func kind(_ info: [AnyHashable: Any]) -> String? {
    guard isRemote(info), let type = info["type"] as? String, matches(type, typePattern) else { return nil }
    return type
  }

  /**
   * About messages, as opposed to a word of the server itself (`test`,
   * `broadcast`): what is not shown while the app shows the messages
   * itself. A sealed push of a private server is one of its relay's words,
   * which are about messages but for `test`; it cannot be opened here.
   */
  static func aboutMessage(_ type: String) -> Bool {
    type == typeDm || type == typeGroup || type == typeSync || type == typeSealed
  }

  /** The group a push is about: `group:<id>`. */
  static func groupChat(_ info: [AnyHashable: Any]) -> String? {
    guard kind(info) == typeGroup, let id = info["group_id"] as? String, matches(id, hex64Pattern) else { return nil }
    return "group:\(id)"
  }

  /**
   * What a tap on `info` opens: a notification the app showed carries the
   * extras Android's do; one the system showed for a push carries the
   * push, and names a chat only for a group (the server cannot tell who
   * wrote a direct message). Taps come from the system alone, for this
   * app's notifications: no secret is needed to tell them from a forgery,
   * as the intents of Android need.
   */
  static func tap(_ info: [AnyHashable: Any]) -> Tap? {
    if let type = info[extraType] as? String, !type.isEmpty {
      let chat = (info[extraChat] as? String).flatMap { isChatKey($0) ? $0 : nil }
      return Tap(type: String(type.prefix(32)), chat: chat)
    }
    guard let type = kind(info) else { return nil }
    // What a private server sealed stays sealed without the extension: a message.
    return Tap(type: type == typeSealed ? typeDm : type, chat: groupChat(info))
  }

  /**
   * Whether a delivered notification goes when the app takes the
   * notifications of `key` away (`cancel`, Notifier.cancel on Android):
   * nil for every one about messages; a group's for `group:<id>`; every
   * direct message for `dm:<pubkey>`, since none here names its sender,
   * and with them the counted rest (`sync`) for `dm`, the list of chats.
   * Only what the system showed for a push: the app's own notices (a call
   * on in a group) are their plugin's.
   */
  static func clearedBy(_ key: String?, _ info: [AnyHashable: Any]) -> Bool {
    guard let type = kind(info), aboutMessage(type) else { return false }
    guard let key = key else { return true }
    if key.hasPrefix("group:") { return groupChat(info) == key }
    let direct = type == typeDm || type == typeSealed
    if key == "dm" { return direct || type == typeSync }
    if key.hasPrefix("dm:") { return direct }
    return false
  }

  /**
   * A background push for the keeper of a channel: no alert,
   * `content-available`, a direct message (the server sends a direct
   * message this way only when it stands for a note to a keeper alone),
   * and, when the event came inside, the mark `channel` on its outside.
   */
  static func isKeeperWake(_ info: [AnyHashable: Any]) -> Bool {
    guard let aps = info["aps"] as? [AnyHashable: Any], aps["alert"] == nil,
      (aps["content-available"] as? NSNumber)?.intValue == 1,
      kind(info) == typeDm
    else { return false }
    guard let event = info["event"] as? String, !event.isEmpty else { return true }
    guard event.utf8.count <= 8192, let data = event.data(using: .utf8),
      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let tags = json["tags"] as? [Any]
    else { return false }
    return tags.contains { (($0 as? [Any])?.first as? String) == keeperMark }
  }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Security

/**
 * The keys the push handler opens messages with (the twin of Keys.kt),
 * kept in the Keychain: sealed by the system, readable once the phone has
 * been unlocked after its start (a push comes with the screen locked),
 * never in a backup and never on another device.
 *
 * What is in the bundle is the app's business: this side only keeps it
 * and throws it away when told. Nothing reads it yet: the Notification
 * Service Extension will, through an access group of the app's and the
 * extension's, which this item does not name yet.
 */
enum Keys {
  private static let service = "net.veydan.push"
  private static let account = "keys"

  private static var item: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }

  /** Keeps the bundle; a bundle kept before is replaced. */
  static func store(_ bundle: Data) throws {
    let values: [String: Any] = [
      kSecValueData as String: bundle,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
    ]
    var status = SecItemUpdate(item as CFDictionary, values as CFDictionary)
    if status == errSecItemNotFound {
      status = SecItemAdd(item.merging(values) { $1 } as CFDictionary, nil)
    }
    guard status == errSecSuccess else {
      throw PushError("the keys were not kept: OSStatus \(status)")
    }
  }

  /**
   * Throws the bundle away. The Keychain outlives the removal of the app:
   * what an earlier install kept stays until this is called.
   */
  static func clear() {
    let status = SecItemDelete(item as CFDictionary)
    if status != errSecSuccess && status != errSecItemNotFound {
      pushLog("the keys were not thrown away: OSStatus \(status)")
    }
  }
}

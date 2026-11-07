// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import UIKit

/**
 * The screen during a call: kept on (a video call is watched), and dark
 * at the ear while the sound is in the earpiece (the proximity sensor).
 *
 * iOS keeps the process of a call running by itself while CallKit and the
 * audio session hold it (the background modes `audio` and `voip` of the
 * app); what Android's wake lock does is the system's here, and only the
 * screen is the plugin's. Used on the main thread.
 */
enum KeepAwake {
  /** The screen on at most this long, whatever the app forgets (as the Kotlin side's wake lock). */
  private static let limit: TimeInterval = 4 * 60 * 60
  private static var expiry: DispatchWorkItem?

  /** The screen stays on (`on`), or sleeps as the user set it. */
  static func screen(_ on: Bool) {
    expiry?.cancel()
    expiry = nil
    UIApplication.shared.isIdleTimerDisabled = on
    if on {
      let item = DispatchWorkItem { KeepAwake.screen(false) }
      expiry = item
      DispatchQueue.main.asyncAfter(deadline: .now() + limit, execute: item)
    }
  }

  /** At the ear the screen goes dark (`on`); off, it lights again when the phone leaves the ear. */
  static func proximity(_ on: Bool) {
    let device = UIDevice.current
    if device.isProximityMonitoringEnabled == on { return }
    device.isProximityMonitoringEnabled = on
    // A device without the sensor keeps it off.
    callLog("proximity sensor \(device.isProximityMonitoringEnabled ? "on" : "off")")
  }
}

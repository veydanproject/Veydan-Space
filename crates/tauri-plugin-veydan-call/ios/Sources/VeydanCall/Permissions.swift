// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import Foundation

/**
 * The permissions of a call: the camera and the microphone. The system
 * asks the user once (the words of the question are the app's
 * `NSCameraUsageDescription` and `NSMicrophoneUsageDescription`); after
 * that the answer stands, and a no is changed only in the Settings app.
 * So a permission never answered is asked for, and any other is told as
 * it is, as the Kotlin side's `requestPermission` does.
 */
enum Permissions {
  static let camera = "camera"
  static let microphone = "microphone"

  /** Asks for `permission` when it was never answered; `done` on the main thread with whether it is granted. */
  static func request(_ permission: String, done: @escaping (Bool) -> Void) {
    let answer: (Bool) -> Void = { granted in DispatchQueue.main.async { done(granted) } }
    switch permission {
    case camera:
      switch AVCaptureDevice.authorizationStatus(for: .video) {
      case .authorized: answer(true)
      case .notDetermined: AVCaptureDevice.requestAccess(for: .video, completionHandler: answer)
      default: answer(false)
      }
    case microphone:
      if #available(iOS 17.0, *) {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: answer(true)
        case .undetermined: AVAudioApplication.requestRecordPermission(completionHandler: answer)
        default: answer(false)
        }
      } else {
        let session = AVAudioSession.sharedInstance()
        switch session.recordPermission {
        case .granted: answer(true)
        case .undetermined: session.requestRecordPermission(answer)
        default: answer(false)
        }
      }
    default:
      answer(false)
    }
  }
}

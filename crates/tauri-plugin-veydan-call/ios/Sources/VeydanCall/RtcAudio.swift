// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import Foundation

/**
 * libwebrtc's `RTCAudioSession`, told when CallKit hands the audio session
 * over and takes it back.
 *
 * The engine of calls plays and records through libwebrtc's own audio
 * device of iOS, which works on `RTCAudioSession`. With CallKit the system,
 * not the app, activates the audio session of a call (`didActivate`), and
 * a session started earlier by the engine would be cut off. So the engine
 * is put on manual audio at the plugin's load: it starts its audio unit
 * only once told the session is live (`isAudioEnabled`), and is told of
 * every activation and deactivation (`audioSessionDidActivate:`), as
 * libwebrtc's own CallKit example does.
 *
 * The class is reached through the Objective-C runtime, by name, without
 * a header: its code is in the static archive of libwebrtc that the Rust
 * side links into the same binary, and a Swift package has no bridging
 * header to import WebRTC's Objective-C headers with. The archive is built
 * without a prefix of its Objective-C names (`rtc_objc_prefix` unset in
 * vendor/webrtc-sys/libwebrtc/build_ios.sh), so the class is
 * `RTCAudioSession`. A build without the engine has no such class: then
 * nothing is done, and the audio session is the plugin's alone.
 *
 * Used on the main thread.
 */
enum RtcAudio {
  private static var shared: NSObject? {
    guard let cls = NSClassFromString("RTCAudioSession") else { return nil }
    let instance = NSSelectorFromString("sharedInstance")
    let meta: AnyObject = cls
    guard meta.responds(to: instance) else { return nil }
    return meta.perform(instance)?.takeUnretainedValue() as? NSObject
  }

  /** The engine waits for the session of a call before it starts its audio. At the plugin's load. */
  static func prepare() {
    guard let session = shared else {
      callLog("no RTCAudioSession linked: the audio session is the plugin's alone")
      return
    }
    setBool(session, "setUseManualAudio:", true)
    setBool(session, "setIsAudioEnabled:", false)
    callLog("RTCAudioSession on manual audio")
  }

  /** CallKit (or the plugin, without CallKit) activated `session`: the engine may start its audio. */
  static func activated(_ session: AVAudioSession) {
    guard let rtc = shared else { return }
    _ = rtc.perform(NSSelectorFromString("audioSessionDidActivate:"), with: session)
    setBool(rtc, "setIsAudioEnabled:", true)
  }

  /** CallKit deactivated `session`: the engine stops its audio. */
  static func deactivated(_ session: AVAudioSession) {
    guard let rtc = shared else { return }
    _ = rtc.perform(NSSelectorFromString("audioSessionDidDeactivate:"), with: session)
    setBool(rtc, "setIsAudioEnabled:", false)
  }

  /** The call is over: the engine stops its audio, whatever CallKit does next. */
  static func disable() {
    guard let rtc = shared else { return }
    setBool(rtc, "setIsAudioEnabled:", false)
  }

  /**
   * Calls a setter that takes a BOOL. `perform(_:with:)` would hand the
   * setter an object's address for its BOOL; the method is called through
   * its implementation instead, with the C signature it has.
   */
  private static func setBool(_ object: NSObject, _ name: String, _ value: Bool) {
    let selector = NSSelectorFromString(name)
    guard object.responds(to: selector) else {
      callLog("RTCAudioSession has no \(name)")
      return
    }
    typealias Setter = @convention(c) (AnyObject, Selector, Bool) -> Void
    let setter = unsafeBitCast(object.method(for: selector), to: Setter.self)
    setter(object, selector, value)
  }
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation

/*
 * The decisions of a call that do not need a phone to be made, kept apart
 * from the system's classes: Foundation alone. The rules shared with
 * Android are ports of CallRules.kt and say the same (its tests in
 * android/src/test are the reference); the classes that talk to iOS follow
 * what these say.
 *
 * Left out of the port, because iOS has nothing they decide: ServiceLife
 * (a foreground service), SwipeRule (the ringing screen is CallKit's) and
 * CameraOrientation (the rotation of a frame is computed for each frame
 * here, `CameraRotation`). Added here: the first route of the sound, the
 * size of the camera and the default network, which Kotlin decides inline.
 */

/**
 * Whether a ringing call rings, from what the user can see of it (see
 * `RingRules` of CallRules.kt). On iOS CallKit shows the call whenever it
 * takes it, over the lock screen and over the app alike, so `visible` is
 * whether CallKit took it.
 */
enum RingRules {
  static func rings(visible: Bool, inFront: Bool) -> Bool {
    visible || inFront
  }

  /**
   * Whether a push may ring `callId`: not a call that is `over` here, and
   * not while another call `busyWith` has the phone. The same call rung
   * again is left to `showIncoming`, which keeps its ringing.
   */
  static func pushRings(callId: String, over: Bool, busyWith: String?) -> Bool {
    !over && (busyWith == nil || busyWith == callId)
  }

  /**
   * Whether the app may ring a call while `ongoingWith` goes on on the
   * phone (nil: nothing goes on; a call that only rings is replaced). The
   * phone knows one call at a time: a ringing over a call under way would
   * end in `stop`, which takes the call under way down with it.
   */
  static func appRings(ongoingWith: String?) -> Bool {
    ongoingWith == nil
  }
}

/**
 * The calls that ended on this phone a short while ago, by id: a late
 * push for one of them rings nothing. A few ids, the oldest go first.
 * Times are milliseconds of a monotonic clock (`Clock.uptimeMs`).
 */
final class RecentlyOver {
  private let keptMs: Int64
  private let keep: Int
  private var list: [(callId: String, at: Int64)] = []

  init(keptMs: Int64, keep: Int = 16) {
    self.keptMs = keptMs
    self.keep = keep
  }

  /** `callId` ended at `now`. */
  func ended(_ callId: String, now: Int64) {
    list.removeAll { $0.callId == callId }
    list.append((callId: callId, at: now))
    while list.count > keep { list.removeFirst() }
  }

  /** Did `callId` end within the last `keptMs` before `now`. */
  func isOver(_ callId: String, now: Int64) -> Bool {
    list.contains { $0.callId == callId && now - $0.at >= 0 && now - $0.at <= keptMs }
  }
}

/**
 * The calls on in groups this phone has dealt with, across its processes:
 * the ones whose quiet notification was shown (`shown`), and the ones that
 * went on here, which I was in (`joined`). Kept by `GroupCallNotices` in
 * the app's defaults, in the same text as on Android, so that a process
 * started anew knows them.
 *
 * A call I was in is no news after I leave it (`isNews`). A notification
 * shown is shown once, until the call is over (`dismissed`) or the next
 * process starts (`leftovers`): the process that showed it may have died
 * with the call still on or over; it goes at the start, and the app shows
 * the call anew if it is still on.
 *
 * Times are a wall clock (milliseconds since 1970): the phone may restart
 * in between. A few entries, the oldest first.
 */
final class GroupCallMemory {
  enum How: String {
    case shown = "SHOWN"
    case joined = "JOINED"
  }

  struct Entry: Equatable {
    let callId: String
    let how: How
    let at: Int64
  }

  private let keptMs: Int64
  private let keep: Int
  private var calls: [Entry] = []

  init(keptMs: Int64, keep: Int = 32) {
    self.keptMs = keptMs
    self.keep = keep
  }

  /** The entries, oldest first (for keeping). */
  var entries: [Entry] { calls }

  /** The notification of `callId` was shown at `now`. A call I am in stays as such. */
  func shown(_ callId: String, now: Int64) { put(callId, .shown, now) }

  /** The call `callId` went on here at `now`: I was in it. */
  func joined(_ callId: String, now: Int64) { put(callId, .joined, now) }

  private func put(_ callId: String, _ how: How, _ now: Int64) {
    let was = calls.first { $0.callId == callId }
    // In it: the stronger word, kept; a notice shown of a call I was in changes nothing.
    let kept: How = was?.how == .joined ? .joined : how
    calls.removeAll { $0.callId == callId }
    calls.append(Entry(callId: callId, how: kept, at: now))
    while calls.count > keep { calls.removeFirst() }
  }

  /**
   * The notice of `callId` goes (the call is over, or I am in it): no
   * longer shown. What I was in stays known. Answers whether a notice was
   * known.
   */
  @discardableResult
  func dismissed(_ callId: String) -> Bool {
    let before = calls.count
    calls.removeAll { $0.callId == callId && $0.how == .shown }
    return calls.count != before
  }

  /**
   * Whether `callId` is news at `now`: neither shown nor joined within
   * `keptMs`. An entry from the future (the clock was set back since) is
   * known still.
   */
  func isNews(_ callId: String, now: Int64) -> Bool {
    !calls.contains { $0.callId == callId && now - $0.at <= keptMs }
  }

  /**
   * At the start of a process: the notifications a dead process left,
   * which go now and are forgotten as shown; what I was in stays known.
   */
  func leftovers() -> [String] {
    let shown = calls.filter { $0.how == .shown }.map { $0.callId }
    calls.removeAll { $0.how == .shown }
    return shown
  }

  /** The entries as one string, a line each; `decode` reads it back. */
  func encode() -> String {
    calls.map { "\($0.callId)\t\($0.how.rawValue)\t\($0.at)" }.joined(separator: "\n")
  }

  static func decode(_ text: String?, keptMs: Int64, keep: Int = 32) -> GroupCallMemory {
    let memory = GroupCallMemory(keptMs: keptMs, keep: keep)
    guard let text = text, !text.isEmpty else { return memory }
    for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
      let parts = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
      guard parts.count == 3, !parts[0].isEmpty,
        let how = How(rawValue: parts[1]), let at = Int64(parts[2])
      else { continue }
      memory.calls.removeAll { $0.callId == parts[0] }
      memory.calls.append(Entry(callId: parts[0], how: how, at: at))
    }
    while memory.calls.count > keep { memory.calls.removeFirst() }
    return memory
  }
}

/**
 * How long a ringing may go on, and whether a ringing could still be live
 * (see `RingAge` of CallRules.kt). CallKit ends the calls of a process that
 * dies, so iOS has no ringing left by a dead process; the limit is what is
 * used here.
 */
enum RingAge {
  /** Rung at `rungAt` (0 when unknown), is it still within `limitMs` at `now`. */
  static func fresh(rungAt: Int64, now: Int64, limitMs: Int64) -> Bool {
    rungAt > 0 && now >= rungAt && now - rungAt <= limitMs
  }

  /**
   * How long a ringing may go on from `now`: until `expiresAt` (0: no
   * expiry known) and never longer than `limitMs`; nothing for an
   * invitation that expired already.
   */
  static func limit(expiresAt: Int64, now: Int64, limitMs: Int64) -> Int64 {
    expiresAt <= 0 ? limitMs : min(max(expiresAt - now, 0), limitMs)
  }
}

/**
 * What a call shows over the lock screen. The messenger may ask to keep
 * who calls off it; CallKit then names the call "Incoming call" until the
 * phone is unlocked (`CallKitBridge` updates the name at the unlock).
 */
enum LockScreen {
  /** May the name of the caller be shown now. */
  static func reveals(hidden: Bool, locked: Bool) -> Bool {
    !hidden || !locked
  }
}

/** The routes of the sound, as the Rust side names them. */
enum Route {
  static let earpiece = "earpiece"
  static let speaker = "speaker"
  static let bluetooth = "bluetooth"
  static let wired = "wired"

  /** Every route, in the one order a list is given in. */
  static let all = [earpiece, speaker, wired, bluetooth]

  /**
   * The route a call starts on (as `AudioRoutes.begin` of the Kotlin side
   * chooses it): a headset when one is there, Bluetooth first; the
   * loudspeaker for a video call, or on a device without an earpiece; the
   * earpiece otherwise.
   */
  static func first(available: [String], video: Bool) -> String {
    if available.contains(bluetooth) { return bluetooth }
    if available.contains(wired) { return wired }
    if video || !available.contains(earpiece) { return speaker }
    return earpiece
  }
}

/** The size the camera is opened at. */
enum CameraFormats {
  /**
   * Which of `sizes` (landscape, as the camera lists its formats) to take
   * for `wanted`: the shape first, 16:9 when the wanted size is 16:9 and
   * 4:3 otherwise (any shape when the camera has none of it), then the
   * smallest at least as large as wanted, else the largest below it (the
   * Kotlin side's `FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER`). Nil for no
   * sizes. A bare nearest size would take any shape (a square picture at
   * the far end) and as many pixels.
   */
  static func pick(_ sizes: [(width: Int, height: Int)], wanted: (width: Int, height: Int)) -> Int? {
    let wide = wanted.width * 9 == wanted.height * 16
    let shaped = sizes.indices.filter { i in
      let s = sizes[i]
      return wide ? s.width * 9 == s.height * 16 : s.width * 3 == s.height * 4
    }
    let candidates = shaped.isEmpty ? Array(sizes.indices) : shaped
    let area = { (i: Int) in sizes[i].width * sizes[i].height }
    let larger = candidates.filter { sizes[$0].width >= wanted.width && sizes[$0].height >= wanted.height }
    if let best = larger.min(by: { area($0) < area($1) }) { return best }
    return candidates.max(by: { area($0) < area($1) })
  }
}

/**
 * The degrees clockwise a frame of the camera is to be turned to stand
 * upright, from how the phone is held: the sensor gives its pictures in
 * landscape, whichever way the phone is turned (the mapping of libwebrtc's
 * own `RTCCameraVideoCapturer`). Face up, face down and unknown keep the
 * last known way.
 */
enum CameraRotation {
  enum Held {
    case portrait
    case portraitUpsideDown
    /** Turned so that the top of the phone points left (home side right). */
    case landscapeLeft
    case landscapeRight
  }

  static func degrees(_ held: Held, front: Bool) -> Int {
    switch held {
    case .portrait: return 90
    case .portraitUpsideDown: return 270
    case .landscapeLeft: return front ? 180 : 0
    case .landscapeRight: return front ? 0 : 180
    }
  }
}

/**
 * The phone's default network, as `NetworkWatch` reports its changes (the
 * same reasoning as NetworkWatch.kt): the first network seen after the
 * start is the one the app uses already, no word for it; a network after
 * none is `back`, another one than before is `other`.
 */
struct DefaultNetwork {
  private var current: String?
  private var seenOne = false

  /** `key`: the default network now, nil for none. Answers the word, or nil for no change. */
  mutating func seen(_ key: String?) -> String? {
    guard let key = key else {
      current = nil
      return nil
    }
    let before = current
    current = key
    if !seenOne {
      seenOne = true
      return nil
    }
    if before == key { return nil }
    return before == nil ? "back" : "other"
  }
}

/** The clocks of the rules. */
enum Clock {
  /** Milliseconds of a clock that never goes back and runs on while the phone sleeps. */
  static func uptimeMs() -> Int64 {
    Int64(clock_gettime_nsec_np(CLOCK_MONOTONIC) / 1_000_000)
  }

  /** Milliseconds since 1970: a wall clock, for what outlives a restart. */
  static func wallMs() -> Int64 {
    Int64(Date().timeIntervalSince1970 * 1000)
  }
}

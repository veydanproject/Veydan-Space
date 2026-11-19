// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import AVFoundation
import CallKit
import Foundation
import Tauri
import UIKit

/** A call as the app hands it: its id, who it is with, whether with video, whether who calls stays off the lock screen. */
struct CallInfo {
  let callId: String
  let name: String
  let video: Bool
  let hidden: Bool
}

/**
 * The phases of a call on this phone, through CallKit (the twin of
 * Calls.kt):
 *
 * - ringing: CallKit's incoming call, over the lock screen and over the
 *   app, with the system's ringtone;
 * - answered: answered on CallKit's screen, the app not yet in the call;
 * - ongoing: CallKit's call under way, which keeps the process and the
 *   sound while the phone is locked or the app away, the audio session in
 *   the mode of a conversation, the proximity sensor while the sound is at
 *   the ear;
 * - nothing: all of it let go.
 *
 * The app's Rust code moves a call from phase to phase. A press of the
 * user on CallKit's screen moves it too, at once: Decline and End end it
 * here, Answer leaves the ringing. The app learns of the press after
 * (`CallEvents.pressed`).
 *
 * CallKit knows a call by a UUID of the app's choosing; the plugin knows
 * one call at a time, so one UUID is kept for it. Recents stay without
 * the calls of Veydan, and a call is never held, grouped or merged.
 *
 * The audio session belongs to CallKit during a call: it activates it,
 * the plugin and libwebrtc are told (`didActivate`, `RtcAudio`). Should
 * CallKit refuse a call the app starts, the call goes on without it, and
 * the plugin activates the session itself.
 *
 * A VoIP push rings before anybody knows which call it is (`VoipPushes`):
 * CallKit is told at once of a *placeholder*, a call with no id and no
 * name («Incoming call», or the app's name once the phone is unlocked),
 * and the push goes to the app, which opens it and names the call
 * (`voipRing`) or ends the placeholder (`voipEnd`). The app's own ring of
 * the call (`showIncoming`) takes its place as well, whichever comes
 * first. A press on a placeholder waits for its call: an Answer is the
 * call's answer once it is known, a Decline its decline. A placeholder no
 * call takes the place of ends as failed after `VoipLimits.placeholder`,
 * unless the app ends it sooner.
 *
 * Runs on the main thread (CallKit's delegate is called on the main queue).
 */
final class CallKitBridge: NSObject, CXProviderDelegate {
  static let shared = CallKitBridge()

  private enum Phase {
    case ringing
    case answered
    case ongoing
  }

  /**
   * A ringing call nobody ended, and an answer the app never took up, end
   * by themselves: the app may have stopped listening, and a phone must
   * not ring forever. The app's own timeouts are shorter (an invitation
   * lives 45 seconds).
   */
  static let ringLimit: TimeInterval = 120
  private static let answerLimit: TimeInterval = 60

  private var provider: CXProvider?
  private let controller = CXCallController()
  private var call: CallInfo?
  private var phase: Phase?
  /** CallKit's name of `call`, while CallKit has it. */
  private var uuid: UUID?
  /** The ends asked of CallKit by the app (`stop`), which are no press of the user. */
  private var endedByApp = Set<UUID>()
  private var limit: DispatchWorkItem?
  /** What the last ringing answered, for the same call rung again. */
  private var lastShown: JsonObject?
  /** The calls that ended here lately: a late push for one rings nothing. */
  private let over = RecentlyOver(keptMs: Int64(CallKitBridge.ringLimit * 1000))
  private var unlockObserver: NSObjectProtocol?
  /**
   * The VoIP push `call` is the placeholder of (its name: the UUID of
   * CallKit as a string), while no call has taken its place.
   */
  private var placeholder: String?
  /** The placeholders declined before their call was known: that call is declined once it is. */
  private var declinedPlaceholders: [String] = []
  /** The calls so declined: a ring of the app for one rings nothing, and declines it. */
  private let declined = RecentlyOver(keptMs: Int64(CallKitBridge.ringLimit * 1000))
  private let placeholders = PlaceholderGuard()

  /** CallKit's provider, made once, at the plugin's load. */
  func load() {
    guard provider == nil else { return }
    let provider = CXProvider(configuration: CallKitBridge.configuration())
    provider.setDelegate(self, queue: nil)
    self.provider = provider
    AudioRoutes.onChange = { [weak self] in
      self?.updateProximity()
      CallEvents.shared.routesChanged(AudioRoutes.describe())
    }
    // Unlocked while a hidden call rings or goes on: CallKit names who it is with now.
    unlockObserver = NotificationCenter.default.addObserver(
      forName: UIApplication.protectedDataDidBecomeAvailableNotification, object: nil, queue: .main
    ) { [weak self] _ in
      self?.updateName()
    }
  }

  private static func configuration() -> CXProviderConfiguration {
    let configuration: CXProviderConfiguration
    if #available(iOS 14.0, *) {
      // The name under the call is the app's own (CFBundleDisplayName).
      configuration = CXProviderConfiguration()
    } else {
      configuration = CXProviderConfiguration(localizedName: appName())
    }
    configuration.supportsVideo = true
    configuration.maximumCallGroups = 1
    configuration.maximumCallsPerCallGroup = 1
    // A call is named by the app's words, never by a number or an address.
    configuration.supportedHandleTypes = [.generic]
    // The phone's Recents would keep who called, out of the messenger's reach.
    configuration.includesCallsInRecents = false
    // No sound of the app's own: CallKit rings with the ringtone the user chose.
    configuration.ringtoneSound = nil
    return configuration
  }

  /** The user looks at the app. */
  private func inFront() -> Bool {
    appInFront()
  }

  /** Who a call is with, as CallKit may show it now (see `LockScreen`). */
  private func shownName(_ info: CallInfo) -> String {
    let locked = !UIApplication.shared.isProtectedDataAvailable
    if !LockScreen.reveals(hidden: info.hidden, locked: locked) {
      return phase == .ongoing ? appName() : Strings.incoming(video: info.video)
    }
    return info.name.isEmpty ? appName() : info.name
  }

  private func update(for info: CallInfo) -> CXCallUpdate {
    let update = CXCallUpdate()
    let name = shownName(info)
    update.remoteHandle = CXHandle(type: .generic, value: name)
    update.localizedCallerName = name
    update.hasVideo = info.video
    update.supportsHolding = false
    update.supportsGrouping = false
    update.supportsUngrouping = false
    update.supportsDTMF = false
    return update
  }

  private func updateName() {
    guard let info = call, let id = uuid else { return }
    provider?.reportCall(with: id, updated: update(for: info))
  }

  private func arm(_ seconds: TimeInterval) {
    limit?.cancel()
    let item = DispatchWorkItem { [weak self] in
      guard let self = self, self.phase == .ringing || self.phase == .answered else { return }
      if self.placeholder != nil {
        self.endPlaceholder(.failed, "no call took its place in time")
        return
      }
      callLog("the call \(self.call?.callId ?? "?") was left \(String(describing: self.phase)); ended here")
      self.stop()
    }
    limit = item
    DispatchQueue.main.asyncAfter(deadline: .now() + seconds, execute: item)
  }

  private func disarm() {
    limit?.cancel()
    limit = nil
  }

  /**
   * Rings. Answers what the phone could do: `{ foreground, fullScreen,
   * notifications, ringing, inFront }`, as the Rust side's `Shown` reads
   * it (see there for what the words mean on iOS); `busyWith` when a call
   * goes on, over which nothing rings.
   */
  func showIncoming(_ called: CallInfo, done: @escaping (JsonObject) -> Void) {
    load()
    // The placeholder of a VoIP push rings: this is its call (the app
    // heard the invitation before it opened the push), which takes its
    // place on the screen as it is.
    if placeholder != nil, uuid != nil, phase == .ringing || phase == .answered {
      var answer = adopt(called, ringFor: CallKitBridge.ringLimit)
      answer["inFront"] = inFront()
      done(answer)
      return
    }
    // Declined on the placeholder of its push before it was known: rings
    // nothing, and is declined now that the app knows it.
    if declined.isOver(called.callId, now: Clock.uptimeMs()) {
      callLog("\(called.callId) was declined on the placeholder of its push; not rung")
      CallEvents.shared.pressed(Press(callId: called.callId, action: "decline"))
      done(["foreground": false, "fullScreen": true, "notifications": true, "ringing": false, "inFront": inFront()])
      return
    }
    // The same call rings already: the ringing goes on as it is, and a
    // call answered here that the app only now hears of stays answered.
    if let same = call, phase == .ringing || phase == .answered, same.callId == called.callId {
      callLog("ringing \(called.callId) already (\(String(describing: phase)))")
      var answer = lastShown ?? [:]
      answer["inFront"] = inFront()
      done(answer)
      return
    }
    let ongoing = phase == .ongoing ? call?.callId : nil
    if !RingRules.appRings(ongoingWith: ongoing) {
      callLog("a call \(called.callId) rings while \(ongoing ?? "") goes on; not rung, the phone stays its")
      done([
        "foreground": false, "fullScreen": true, "notifications": true, "ringing": false,
        "inFront": inFront(), "busyWith": ongoing ?? "",
      ])
      return
    }
    // A call that rings already is replaced by the new one.
    if let old = uuid {
      provider?.reportCall(with: old, endedAt: nil, reason: .unanswered)
      if let previous = call { over.ended(previous.callId, now: Clock.uptimeMs()) }
    }
    disarm()
    let id = UUID()
    call = called
    phase = .ringing
    uuid = id
    lastShown = nil
    guard let provider = provider else {
      done(["foreground": false, "fullScreen": false, "notifications": false, "ringing": false, "inFront": inFront()])
      return
    }
    provider.reportNewIncomingCall(with: id, update: update(for: called)) { [weak self] error in
      DispatchQueue.main.async {
        guard let self = self else { return }
        let taken = error == nil
        if let error = error {
          // Do Not Disturb, a blocked caller, a device that cannot ring:
          // nothing rings, and the app shows the call its own way.
          callLog("CallKit would not ring \(called.callId): \(error)")
          if self.uuid == id {
            self.call = nil
            self.phase = nil
            self.uuid = nil
          }
        } else if self.uuid == id {
          self.arm(CallKitBridge.ringLimit)
        }
        // CallKit rings whenever it takes the call, in front or not
        // (`RingRules.rings` with the call visible), and nothing rings
        // when it does not: the plugin has no ringer of its own.
        let ringing = RingRules.rings(visible: taken, inFront: false)
        let answer: JsonObject = ["foreground": taken, "fullScreen": true, "notifications": true, "ringing": ringing]
        if taken { self.lastShown = answer }
        var out = answer
        out["inFront"] = self.inFront()
        callLog("ringing \(called.callId) (video=\(called.video), hidden=\(called.hidden)): \(out)")
        done(out)
      }
    }
  }

  // MARK: The placeholders of VoIP pushes

  /** What a placeholder is to CallKit: nobody, «Incoming call» while the phone is locked. */
  private static let nobody = CallInfo(callId: "", name: "", video: false, hidden: true)

  /**
   * A VoIP push (`VoipPushes`), reported to CallKit now, whatever it is:
   * Apple demands it of every VoIP push before its handler returns, and
   * `reported` (the handler's completion) is called once CallKit answered.
   *
   * - A call has the phone already (most likely this push is its own
   *   invitation, which the app rang first): reported under that call's
   *   UUID, which CallKit refuses as known; nothing changes on the screen.
   * - Not an invitation, a call going on without CallKit, or too many
   *   placeholders came to nothing this minute (`PlaceholderGuard`):
   *   reported and ended at once, without a ring.
   * - Otherwise a placeholder rings, and the push goes to the app.
   */
  func ringFromVoipPush(_ push: VoipPush, reported: @escaping () -> Void) {
    load()
    guard let provider = provider else {
      reported()
      return
    }
    if let current = uuid, let info = call, phase != nil {
      callLog("a VoIP push (\(push.trace)) while \(placeholder == nil ? info.callId : "a placeholder") has the phone; reported as that call")
      provider.reportNewIncomingCall(with: current, update: update(for: info)) { _ in
        DispatchQueue.main.async { reported() }
      }
      return
    }
    let id = UUID()
    var why: String?
    if !push.isInvite {
      why = "not an invitation"
    } else if phase != nil {
      why = "a call goes on without CallKit"
    } else if !placeholders.rings(now: Clock.uptimeMs()) {
      why = "too many came to nothing this minute"
    }
    if let why = why {
      callLog("a VoIP push (\(push.trace)): \(why); reported and ended at once")
      provider.reportNewIncomingCall(with: id, update: update(for: CallKitBridge.nobody)) { [weak self] error in
        DispatchQueue.main.async {
          if error == nil { self?.provider?.reportCall(with: id, endedAt: nil, reason: .failed) }
          reported()
        }
      }
      return
    }
    disarm()
    call = CallKitBridge.nobody
    phase = .ringing
    uuid = id
    lastShown = nil
    placeholder = id.uuidString
    provider.reportNewIncomingCall(with: id, update: update(for: CallKitBridge.nobody)) { [weak self] error in
      DispatchQueue.main.async {
        defer { reported() }
        guard let self = self, self.uuid == id else { return }
        if let error = error {
          // Do Not Disturb, a blocked caller: nothing rings, and the app
          // rings the call its own way when it hears it.
          callLog("CallKit would not ring the VoIP push (\(push.trace)): \(error)")
          self.call = nil
          self.phase = nil
          self.uuid = nil
          self.placeholder = nil
          return
        }
        let heard = CallEvents.shared.voipPushed(id.uuidString, push.data)
        self.arm(VoipLimits.placeholder)
        callLog("a VoIP push (\(push.trace)) rings as a placeholder; \(heard ? "the app opens it" : "it waits for the app")")
      }
    }
  }

  /**
   * The VoIP push `pushId` is the call `called`, which expires at
   * `expiresAt` (unix seconds, 0: not known). Its placeholder becomes the
   * call; one declined meanwhile declines the call; a call over here, or
   * expired, ends the placeholder.
   */
  func voipRing(_ pushId: String, _ called: CallInfo, expiresAt: Int64) {
    let now = Clock.uptimeMs()
    if let i = declinedPlaceholders.firstIndex(of: pushId) {
      declinedPlaceholders.remove(at: i)
      declined.ended(called.callId, now: now)
      over.ended(called.callId, now: now)
      callLog("the declined placeholder of a VoIP push was \(called.callId); declined")
      CallEvents.shared.pressed(Press(callId: called.callId, action: "decline"))
      return
    }
    guard placeholder == pushId, uuid != nil else {
      callLog("a VoIP push is \(called.callId), its placeholder gone already")
      return
    }
    if !RingRules.pushRings(callId: called.callId, over: over.isOver(called.callId, now: now), busyWith: nil) {
      endPlaceholder(.remoteEnded, "\(called.callId) is over here")
      return
    }
    let limitMs = RingAge.limit(
      expiresAt: expiresAt > 0 ? expiresAt * 1000 : 0, now: Clock.wallMs(),
      limitMs: Int64(CallKitBridge.ringLimit * 1000))
    guard limitMs > 0 else {
      endPlaceholder(.unanswered, "\(called.callId) expired")
      return
    }
    adopt(called, ringFor: TimeInterval(limitMs) / 1000)
  }

  /** The VoIP push `pushId` rings no call: its placeholder ends at once. */
  func voipEnd(_ pushId: String, overAlready: Bool) {
    declinedPlaceholders.removeAll { $0 == pushId }
    guard placeholder == pushId else { return }
    // Opened and found to be no call: what the guard counts. A placeholder
    // that only ran out of time (a cold start, the PIN) is not counted.
    if !overAlready { placeholders.cameToNothing(now: Clock.uptimeMs()) }
    endPlaceholder(overAlready ? .remoteEnded : .failed, "the app found no call to ring")
  }

  /**
   * The placeholder becomes `called`: CallKit names it, and rings on for
   * `seconds`; an Answer pressed on the placeholder is this call's now.
   * Answers what `showIncoming` would.
   */
  @discardableResult
  private func adopt(_ called: CallInfo, ringFor seconds: TimeInterval) -> JsonObject {
    let answered = phase == .answered
    call = called
    placeholder = nil
    if let id = uuid { provider?.reportCall(with: id, updated: update(for: called)) }
    arm(answered ? CallKitBridge.answerLimit : seconds)
    let answer: JsonObject = [
      "foreground": true, "fullScreen": true, "notifications": true,
      "ringing": RingRules.rings(visible: true, inFront: false),
    ]
    lastShown = answer
    callLog("the placeholder of a VoIP push is \(called.callId) (video=\(called.video), answered=\(answered))")
    if answered { CallEvents.shared.pressed(Press(callId: called.callId, action: "answer")) }
    return answer
  }

  /** The placeholder ends without a call: `reason` to CallKit, `why` to the log. */
  private func endPlaceholder(_ reason: CXCallEndedReason, _ why: String) {
    guard let id = uuid, placeholder != nil else { return }
    callLog("the placeholder of a VoIP push ends: \(why)")
    provider?.reportCall(with: id, endedAt: nil, reason: reason)
    release()
    call = nil
    phase = nil
    uuid = nil
    lastShown = nil
    placeholder = nil
  }

  /**
   * The ringing ends without a word from the user: answered elsewhere,
   * cancelled, expired. The app also dismisses a call it ended without
   * the phone ever ringing for it: remembered, so that a push for it
   * rings nothing.
   */
  func dismissIncoming(_ callId: String?) {
    if let callId = callId { over.ended(callId, now: Clock.uptimeMs()) }
    guard phase == .ringing || phase == .answered else { return }
    if let callId = callId, call?.callId != callId { return }
    stop()
  }

  /** The call goes on: CallKit's call under way, the sound of a conversation. */
  func startOngoing(_ info: CallInfo) {
    load()
    disarm()
    // A placeholder answered before its call was named is this call's:
    // the call answered on its screen goes on, not a new one.
    let answeredPlaceholder = placeholder != nil && phase == .answered
    let same = (call?.callId == info.callId || answeredPlaceholder) && uuid != nil
    placeholder = nil
    if !same, let old = uuid {
      // Another call's ringing goes: the phone is this call's now.
      provider?.reportCall(with: old, endedAt: nil, reason: .remoteEnded)
      uuid = nil
    }
    call = info
    phase = .ongoing
    AudioRoutes.begin(video: info.video)
    if same, let id = uuid {
      // Answered on CallKit's screen, or the camera turned on in the call:
      // the call is CallKit's already, and learns the name and the video.
      provider?.reportCall(with: id, updated: update(for: info))
    } else {
      let id = UUID()
      uuid = id
      let action = CXStartCallAction(call: id, handle: CXHandle(type: .generic, value: shownName(info)))
      action.isVideo = info.video
      controller.request(CXTransaction(action: action)) { [weak self] error in
        guard let error = error else { return }
        DispatchQueue.main.async { self?.goesOnWithoutCallKit(id, error) }
      }
    }
    updateProximity()
    // A call that went on here is one I was in: no news of it later.
    GroupCallNotices.joined(info.callId)
    callLog("ongoing \(info.callId) (video=\(info.video))")
  }

  /**
   * CallKit refused the call the app starts (another app's call holds the
   * phone, the system was busy): the call goes on without CallKit, the
   * plugin activating the sound itself; the call then stops with the
   * app's word, as it would with CallKit.
   */
  private func goesOnWithoutCallKit(_ id: UUID, _ error: Error) {
    callLog("CallKit would not start the call: \(error); it goes on without")
    guard uuid == id, phase == .ongoing else { return }
    uuid = nil
    if let session = AudioRoutes.activateOwn() { RtcAudio.activated(session) }
  }

  /** Everything let go: CallKit's call, the sound's mode and route, the screen. */
  func stop() {
    disarm()
    if let id = uuid {
      if phase == .ongoing {
        // The end of a call under way is asked as the user's would be; a
        // refusal (the call gone already) is reported instead.
        endedByApp.insert(id)
        controller.request(CXTransaction(action: CXEndCallAction(call: id))) { [weak self] error in
          guard error != nil else { return }
          DispatchQueue.main.async {
            self?.endedByApp.remove(id)
            self?.provider?.reportCall(with: id, endedAt: nil, reason: .remoteEnded)
          }
        }
      } else {
        provider?.reportCall(with: id, endedAt: nil, reason: .remoteEnded)
      }
    }
    release()
    if let info = call, !info.callId.isEmpty {
      callLog("ended \(info.callId)")
      over.ended(info.callId, now: Clock.uptimeMs())
    }
    call = nil
    phase = nil
    uuid = nil
    lastShown = nil
    placeholder = nil
  }

  /** The phone's side of the call let go; CallKit's call is ended by whoever called this. */
  private func release() {
    disarm()
    RtcAudio.disable()
    AudioRoutes.end()
    KeepAwake.proximity(false)
    KeepAwake.screen(false)
  }

  /** The screen stays on for the call (`on`), or sleeps as the user set it. */
  func keepAwake(_ on: Bool) {
    KeepAwake.screen(on)
  }

  /** At the ear the screen goes dark: a call without video, its sound in the earpiece. */
  private func updateProximity() {
    KeepAwake.proximity(phase == .ongoing && call?.video == false && AudioRoutes.current() == Route.earpiece)
  }

  // MARK: CXProviderDelegate

  /** CallKit dropped every call (its daemon restarted): the phone's side lets go, the app hears nothing. */
  func providerDidReset(_ provider: CXProvider) {
    callLog("CallKit reset")
    release()
    endedByApp.removeAll()
    call = nil
    phase = nil
    uuid = nil
    lastShown = nil
    placeholder = nil
  }

  /** Answer pressed on CallKit's screen. */
  func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
    guard action.callUUID == uuid, let info = call else {
      action.fail()
      return
    }
    if placeholder != nil {
      // Which call is not known yet: the answer waits for it (`adopt`),
      // and the placeholder's time runs on.
      phase = .answered
      AudioRoutes.begin(video: false)
      action.fulfill()
      callLog("the placeholder of a VoIP push answered; its call is answered once known")
      return
    }
    if phase == .ringing {
      phase = .answered
      arm(CallKitBridge.answerLimit)
    }
    // The audio session is configured before CallKit activates it.
    AudioRoutes.begin(video: info.video)
    action.fulfill()
    CallEvents.shared.pressed(Press(callId: info.callId, action: "answer"))
  }

  /**
   * Decline or End pressed: the call ends here, and a push for it rings
   * nothing more. An end the app asked for itself (`stop`) is no press.
   */
  func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
    if endedByApp.remove(action.callUUID) != nil {
      action.fulfill()
      return
    }
    guard action.callUUID == uuid, let info = call else {
      action.fulfill()
      return
    }
    if let pushId = placeholder {
      // Which call is not known yet: declined once it is (`voipRing`).
      declinedPlaceholders.append(pushId)
      while declinedPlaceholders.count > 4 { declinedPlaceholders.removeFirst() }
      release()
      call = nil
      phase = nil
      uuid = nil
      lastShown = nil
      placeholder = nil
      action.fulfill()
      callLog("the placeholder of a VoIP push declined; its call is declined once known")
      return
    }
    let press = phase == .ringing ? "decline" : "hangup"
    over.ended(info.callId, now: Clock.uptimeMs())
    release()
    call = nil
    phase = nil
    uuid = nil
    lastShown = nil
    action.fulfill()
    CallEvents.shared.pressed(Press(callId: info.callId, action: press))
  }

  /** The call the app starts (`startOngoing`): connected at once, the app's call is under way already. */
  func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
    guard action.callUUID == uuid, let info = call else {
      action.fail()
      return
    }
    provider.reportCall(with: action.callUUID, updated: update(for: info))
    provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: nil)
    action.fulfill()
    provider.reportOutgoingCall(with: action.callUUID, connectedAt: nil)
  }

  /**
   * Mute on CallKit's screen is refused: the microphone of a call is
   * muted on the app's page, and no event tells the app of a mute (the
   * Rust side's `Action` has answer, decline and hangup). A mute shown
   * on CallKit's screen that the engine did not do would be a lie.
   */
  func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
    action.fail()
  }

  /** A call is never held, nor are tones sent. */
  func provider(_ provider: CXProvider, perform action: CXSetHeldCallAction) {
    action.fail()
  }

  func provider(_ provider: CXProvider, perform action: CXPlayDTMFCallAction) {
    action.fail()
  }

  /** CallKit gave the call its audio session: the route, and the engine may start its sound. */
  func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
    callLog("CallKit activated the audio session")
    AudioRoutes.activated()
    RtcAudio.activated(audioSession)
    updateProximity()
  }

  /** CallKit took the audio session back (the call ended, or another app's call took it). */
  func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
    callLog("CallKit deactivated the audio session")
    RtcAudio.deactivated(audioSession)
  }

  func provider(_ provider: CXProvider, timedOutPerforming action: CXAction) {
    callLog("CallKit timed out on \(type(of: action))")
  }
}

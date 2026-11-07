// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import ObjectiveC
import UIKit

/**
 * The callbacks of APNs, which UIKit gives the app delegate alone.
 *
 * The app delegate is not the app's: Tauri's runtime on iOS (tao)
 * declares the class `AppDelegate` at run time with the launch, the scenes,
 * the URLs and the moves to and from the background, and forwards no
 * callback of remote notifications. So the three this plugin needs are put
 * on that class when the plugin loads, the way Firebase's swizzler and the
 * push plugins of Tauri's community do it:
 *
 * - application:didRegisterForRemoteNotificationsWithDeviceToken:
 * - application:didFailToRegisterForRemoteNotificationsWithError:
 * - application:didReceiveRemoteNotification:fetchCompletionHandler:
 *
 * A method the class has already (another plugin put it there) is wrapped:
 * its own code runs first, then this plugin's. UIKit reads which of these
 * its delegate answers when the delegate is set, so the delegate is set
 * again once they are in place.
 *
 * application:didFinishLaunchingWithOptions: is out of reach: the plugin
 * is made by Tauri's setup, which runs inside it (tao without scenes:
 * the setup runs from that call). A tap that launched the
 * app comes to the delegate of the notification center instead
 * (`PushCenter`), which is set within that same call.
 *
 * Used on the main thread.
 */
enum AppDelegateHooks {
  private static var installed = false
  /**
   * UIApplication does not keep its delegate; UIApplicationMain does, and
   * a delegate set again might be let go by it. Held here for the
   * process's life.
   */
  private static var delegate: UIApplicationDelegate?

  /**
   * How long a background push keeps the app awake for the keeper before
   * the system is told it is done: the system gives 30 seconds at most,
   * and one that keeps it longer is woken less often.
   */
  private static let keeperWindow: TimeInterval = 20

  private typealias TokenFn = @convention(c) (AnyObject, Selector, UIApplication, NSData) -> Void
  private typealias ErrorFn = @convention(c) (AnyObject, Selector, UIApplication, NSError) -> Void
  private typealias FetchDone = @convention(block) (UIBackgroundFetchResult) -> Void
  private typealias RemoteFn = @convention(c) (AnyObject, Selector, UIApplication, NSDictionary, @escaping FetchDone) -> Void

  /** Whether the hooks are in place. */
  @discardableResult
  static func install() -> Bool {
    guard !installed else { return true }
    let app = UIApplication.shared
    guard let delegate = app.delegate else {
      pushLog("the app has no delegate yet: no token, no background push")
      return false
    }
    // The class itself, not a subclass the key-value observing of
    // Foundation may have put under the object: `class` hides those.
    guard let cls: AnyClass = (delegate as? NSObject)?.classForCoder ?? object_getClass(delegate) else {
      pushLog("the app delegate has no class: no token, no background push")
      return false
    }
    installed = true
    self.delegate = delegate

    let onToken = #selector(UIApplicationDelegate.application(_:didRegisterForRemoteNotificationsWithDeviceToken:))
    hook(cls, onToken, types: "v@:@@") { original in
      let fn = original.map { unsafeBitCast($0, to: TokenFn.self) }
      let block: @convention(block) (AnyObject, UIApplication, NSData) -> Void = { this, application, token in
        fn?(this, onToken, application, token)
        PushState.shared.registered(token as Data)
      }
      return unsafeBitCast(block, to: AnyObject.self)
    }

    let onError = #selector(UIApplicationDelegate.application(_:didFailToRegisterForRemoteNotificationsWithError:))
    hook(cls, onError, types: "v@:@@") { original in
      let fn = original.map { unsafeBitCast($0, to: ErrorFn.self) }
      let block: @convention(block) (AnyObject, UIApplication, NSError) -> Void = { this, application, error in
        fn?(this, onError, application, error)
        PushState.shared.failedToRegister(error)
      }
      return unsafeBitCast(block, to: AnyObject.self)
    }

    let onPush = #selector(UIApplicationDelegate.application(_:didReceiveRemoteNotification:fetchCompletionHandler:))
    hook(cls, onPush, types: "v@:@@@?") { original in
      let fn = original.map { unsafeBitCast($0, to: RemoteFn.self) }
      let block: @convention(block) (AnyObject, UIApplication, NSDictionary, @escaping FetchDone) -> Void = {
        this, application, info, done in
        if let fn = fn {
          // The other code tells the system when it is done; this plugin
          // only wakes the keeper.
          fn(this, onPush, application, info, done)
          AppDelegateHooks.received(info as? [AnyHashable: Any] ?? [:], done: nil)
        } else {
          AppDelegateHooks.received(info as? [AnyHashable: Any] ?? [:], done: done)
        }
      }
      return unsafeBitCast(block, to: AnyObject.self)
    }

    // The delegate is left as it is: set to nil and back inside
    // application:didFinishLaunchingWithOptions: (where Tauri makes the
    // plugins), UIKit never finished the launch on the simulator — the
    // webview stayed white and no plugin after this one was loaded (the
    // thirteenth run of the lab). UIKit asks its delegate whether it answers
    // the callbacks of remote notifications when it delivers them.
    pushLog("the callbacks of APNs are on \(NSStringFromClass(cls))")
    return true
  }

  /**
   * Puts `selector` on `cls` as the block `make` returns, which is given
   * the method's former code when the class had one (its own or one it
   * inherits) to call it.
   */
  private static func hook(_ cls: AnyClass, _ selector: Selector, types: String, make: (IMP?) -> AnyObject) {
    let existing = class_getInstanceMethod(cls, selector)
    let imp = imp_implementationWithBlock(make(existing.map(method_getImplementation)))
    // Added to the class itself, so that a superclass stays as it was; a
    // method of the class's own is replaced in place.
    if !class_addMethod(cls, selector, imp, types), let existing = existing {
      method_setImplementation(existing, imp)
    }
  }

  /**
   * A push UIKit hands the delegate (one with `content-available`): the
   * keeper of the channels takes a turn when it is its push, and nothing
   * else is done with it here. `done`, when this plugin is the
   * one to tell the system, is called at once when nothing runs for it,
   * and after `keeperWindow` while the keeper works with the app off the
   * screen.
   */
  private static func received(_ info: [AnyHashable: Any], done: FetchDone?) {
    guard Push.isKeeperWake(info) else {
      done?(.noData)
      return
    }
    let woken = PushState.shared.keeperWake("push")
    pushLog(woken ? "a push for the keeper: it takes a turn" : "a push for the keeper: the module is off, it turns at its next start")
    guard let done = done else { return }
    if !woken || UIApplication.shared.applicationState == .active {
      done(woken ? .newData : .noData)
      return
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + keeperWindow) { done(.newData) }
  }
}

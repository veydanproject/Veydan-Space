// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation
import Network

/**
 * The phone's word that its network changed: Wi-Fi gone and the mobile
 * data taking over, or the other way round, or another Wi-Fi (the twin of
 * NetworkWatch.kt). The media engine notices a lost way only after seconds
 * of silence, and a relay socket on the old network may look open for
 * much longer; the app, told at once, restarts the call's way and makes
 * the relay connections anew.
 *
 * The path of the system's default route is watched. A network is named
 * by the interface the path prefers (type and name: `wifi en0`, `cellular
 * pdp_ip0`) and the IPv4 gateways it goes through, so that another Wi-Fi
 * on the same interface is another network; the gateways of IPv6, which
 * come and go on the same network, are left out. `DefaultNetwork` decides
 * which changes are told.
 */
enum NetworkWatch {
  private static var monitor: NWPathMonitor?
  private static let queue = DispatchQueue(label: "net.veydan.call.network")
  /** Touched on `queue` only. */
  private static var network = DefaultNetwork()

  /** Starts watching; idempotent. On the main thread. */
  static func start() {
    if monitor != nil { return }
    let watch = NWPathMonitor()
    watch.pathUpdateHandler = { path in
      let key = path.status == .satisfied ? NetworkWatch.key(of: path) : nil
      guard let how = NetworkWatch.network.seen(key) else { return }
      callLog("network changed (\(how)): \(key ?? "none")")
      DispatchQueue.main.async { CallEvents.shared.networkChanged(how) }
    }
    watch.start(queue: queue)
    monitor = watch
  }

  private static func key(of path: NWPath) -> String {
    var key = path.availableInterfaces.first.map { "\($0.type) \($0.name)" } ?? "unknown"
    if #available(iOS 14.0, *) {
      let gateways: [String] = path.gateways.compactMap { endpoint in
        if case let .hostPort(host, _) = endpoint, case .ipv4 = host { return "\(host)" }
        return nil
      }
      key += " via " + gateways.sorted().joined(separator: ",")
    }
    return key
  }
}

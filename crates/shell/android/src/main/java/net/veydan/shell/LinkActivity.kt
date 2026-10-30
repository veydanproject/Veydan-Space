// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * Takes a link of the product's scheme the system hands over (`veydan://…`
 * clicked in a browser, a mail, another app; crates/shell/src/links.rs) and
 * leaves no trace of it outside the memory of the process: a link of a group
 * or a channel carries its key.
 *
 * The intent that carries the link starts this activity, not MainActivity:
 * the intent that starts a task stays its base intent, which the system keeps
 * in its list of recent tasks, shows in its diagnostics and writes to disk.
 * This one has no history, no place in the recent tasks, a task of its own
 * that it removes on leaving; it puts the link in `IncomingLinks` and starts
 * MainActivity as the launcher does, with an intent that carries nothing.
 * The Rust side takes the link from there (`take`) when the app starts or,
 * already running, is told one came (`IncomingLinks.arrived`). A product
 * declares it in its manifest with its scheme.
 */
class LinkActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // A restored instance has no link of its own to give.
    if (savedInstanceState == null) {
      intent?.dataString?.let(IncomingLinks::put)
      IncomingLinks.tell()
    }
    intent = Intent()
    packageManager.getLaunchIntentForPackage(packageName)?.let { launch ->
      launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      startActivity(launch)
    }
    finishAndRemoveTask()
  }
}

/** The link that waits for the Rust side: the last one, in memory only. */
object IncomingLinks {
  /** More than any link the app reads; the Rust side cuts it the same way. */
  private const val MAX_HELD = 4096

  @Volatile private var pending: String? = null

  @Synchronized
  fun put(link: String) {
    var end = minOf(link.length, MAX_HELD)
    if (end < link.length && Character.isHighSurrogate(link[end - 1])) end -= 1
    pending = link.substring(0, end)
  }

  /**
   * Tells the running app a link waits, without the link: the page asks
   * for it (`take`). Not loaded yet on a cold start, when the app takes the
   * link as it starts.
   */
  fun tell() {
    try {
      arrived()
    } catch (_: UnsatisfiedLinkError) {
    }
  }

  /** `Java_net_veydan_shell_IncomingLinks_arrived` of crates/shell/src/links.rs. */
  @JvmStatic private external fun arrived()

  /** The link that waits, taken: the next call gets nothing until another comes. */
  @JvmStatic
  @Synchronized
  fun take(): String? {
    val link = pending
    pending = null
    return link
  }
}

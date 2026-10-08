// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * Receives pushes. The system starts it on its own, also when the app is
 * not running, so nothing here may count on the app's window or on its
 * runtime. What the push is about is worked out by the messenger's core,
 * loaded on its own, with the keys the app left for it; without the keys,
 * or when the core cannot say, the notification says only that something
 * came. While the app has muted pushes nothing is shown.
 *
 * Every push leaves one line in the log with its trace id, the same id the
 * server logs: what came and what was done with it, never what it said.
 */
class VeydanMessagingService : FirebaseMessagingService() {
  override fun onMessageReceived(message: RemoteMessage) {
    val push = Push.from(message.data)
    if (push == null) {
      Log.w(PushState.TAG, "push dropped: no type")
      return
    }
    Log.i(PushState.TAG, "push type=${push.type} trace=${push.trace}: ${handle(push)}")
  }

  private fun handle(push: Push): String {
    if (push.silent) {
      // Service pushes without a word to the user; their handling comes
      // with the features that need them.
      return "silent"
    }
    // The app switched the receiving part off; the server has not forgotten
    // the phone yet.
    if (PushState.muted(this)) {
      return "not shown, muted by the app"
    }
    if (!push.aboutMessage) {
      return if (Notifier.showService(this, push)) "shown" else "not shown, notifications are off"
    }
    // The user is looking at the app and the app gets messages by itself:
    // the message is already on the screen.
    if (PushState.visible && PushState.live) {
      return "not shown, the app is on the screen"
    }
    if (push.type == Push.TYPE_SYNC) {
      return if (Notifier.showMore(this, push.count)) "shown, ${push.count} more" else "not shown, notifications are off"
    }
    // Without keys the core still names the chat; nothing else (and
    // nothing at all of a call: the core answers `quiet`, `no_keys`).
    val bundle = Keys.read(this) ?: ByteArray(0)
    val answer = try {
      Core.describe(this, bundle, push.asData())
    } finally {
      bundle.fill(0)
    }
    return when (val outcome = answer?.let { Outcome.from(it) }) {
      null -> plain(push, PlainNotice.of(push), "no answer from the core")
      is Outcome.Show -> if (Notifier.show(this, outcome.notice)) "shown" else "not shown, notifications are off"
      is Outcome.Plain -> plain(push, outcome.plain, "the core says only that something came")
      is Outcome.Quiet -> "quiet: ${outcome.reason}"
      is Outcome.Call -> Notifier.ring(this, outcome.call)
      is Outcome.CallEnd -> Notifier.endRinging(this, outcome.callId)
      is Outcome.Error -> plain(push, PlainNotice.of(push), outcome.error)
    }
  }

  /**
   * Something came, and that is all that can be said: `notice` is what
   * the core said of it, or what the push alone says when the core did
   * not answer. Not of a call: the server marked the push as one, and a
   * call rings now or never. A "New message" for it would be the
   * notification of nothing (the phone had no keys, the event could not
   * be had, or the core found no call in it); the app shows a missed
   * call when it runs. The core answers `quiet` for every such case it
   * sees; this is the phone's word on the ones it does not.
   */
  private fun plain(push: Push, notice: PlainNotice, why: String): String {
    if (push.call) return "not shown, a call that could not be opened ($why)"
    return if (Notifier.showPlain(this, notice)) "shown plain ($why)" else "not shown, notifications are off ($why)"
  }

  override fun onNewToken(token: String) {
    Log.i(PushState.TAG, "the push service gave a new token")
    PushState.plugin()?.tokenChanged(token)
  }
}

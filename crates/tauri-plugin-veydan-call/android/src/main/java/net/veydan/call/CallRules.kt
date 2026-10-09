// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

/*
 * The decisions of a call that do not need a phone to be made: kept apart
 * from the system's classes so that a plain JVM test can check them
 * (src/test). The classes that talk to Android follow what these say.
 */

/**
 * Where the foreground service of a call is in its life, as this process
 * knows it, and what a new notification or a stop may do with it.
 *
 * A start of a foreground service is a promise to the system: the service
 * must reach `startForeground` once it runs. Stopping it before that (with
 * `stopService`, or `stopSelf` between `onCreate` and `onStartCommand`)
 * makes the system kill the app ("did not then call startForeground").
 * So a start asked for is never stopped from outside: it comes up, and with
 * nothing to show it shows a placeholder and ends itself.
 *
 * A service told to end goes away a little later (`onDestroy` comes after a
 * round trip to the system), taking its notification with it. It is never
 * given a new notification: a new start is asked for instead, and the
 * system makes a new service once the old one is gone.
 *
 * `S` is the service; a test uses anything.
 */
internal class ServiceLife<S : Any> {
  /** What `show` must do. */
  enum class Show {
    /** The running service takes the notification at once. */
    REUSE,
    /** A start is on its way: it shows the latest wanted notification when it comes. */
    WAIT,
    /** Nothing runs, or what runs is ending: a new start is needed. */
    START,
  }

  /** What `stop` must do. */
  enum class Stop {
    /** The running service ends itself. */
    END,
    /** A start is on its way: it finds nothing wanted and ends itself; nothing is stopped from outside. */
    LET_COME,
    /** Nothing runs, or it ends already. */
    NOTHING,
  }

  private var instance: S? = null
  /** A start was asked for and its `onStartCommand` has not come yet. */
  private var pending = false
  /** The running instance was told to end; it is gone at its `onDestroy`. */
  private var ending = false

  fun show(): Show = when {
    pending -> Show.WAIT
    instance != null && !ending -> Show.REUSE
    else -> Show.START
  }

  /** The system took a start (`startForegroundService` did not throw). */
  fun startAsked() {
    pending = true
  }

  fun stop(): Stop = when {
    pending -> Stop.LET_COME
    instance != null && !ending -> Stop.END
    else -> Stop.NOTHING
  }

  fun created(service: S) {
    instance = service
    ending = false
  }

  /** `onStartCommand` came: the start is kept, by `startForeground` or by the placeholder. */
  fun commanded() {
    pending = false
  }

  /** The instance was told to end (`stopForeground` and `stopSelf`). */
  fun ending(service: S) {
    if (instance === service) ending = true
  }

  fun destroyed(service: S) {
    if (instance === service) {
      instance = null
      ending = false
    }
  }

  /** The running instance that may take a notification, if any. */
  fun live(): S? = if (ending) null else instance
}

/**
 * Whether a ringing call rings, from what the user can see of it.
 *
 * The ringtone is played by the plugin, outside the notification system:
 * the volume keys and the shade do not stop it. It may play only while the
 * user has something to press. That is the call notification (and its
 * full-screen screen) when notifications reach the user, or the page of the
 * app itself while the app is in front (the messenger shows every call on
 * a page of its own; the plugin opens no screen over it). When neither is,
 * the phone stays silent and the app learns it from `Shown.ringing`.
 */
internal object RingRules {
  /**
   * `visible`: the notifications of calls reach the user (the app's
   * notifications and the channel are on) and the notification was shown,
   * by the service or by itself. `inFront`: the app is what the user looks at.
   */
  fun rings(visible: Boolean, inFront: Boolean): Boolean = visible || inFront

  /**
   * Whether a push may ring `callId`. A push comes late as often as not:
   * the app, alive in the background, hears the same invitation from the
   * relays, rings itself and may have ended the call already (the caller
   * gave up, Decline on the page, taken on another device) — such a call
   * is `over` here and is not rung again. With a call `busyWith` on the
   * phone (ringing, answered or going on) the app answers "busy" for any
   * other call; the push does not take the phone over for it. The same
   * call rung again is left to `showIncoming`, which keeps its ringing.
   */
  fun pushRings(callId: String, over: Boolean, busyWith: String?): Boolean =
    !over && (busyWith == null || busyWith == callId)

  /**
   * Whether the app may ring a call while `ongoingWith` goes on on the
   * phone (null: nothing goes on; a call that only rings is replaced, as
   * ever). The phone knows one call at a time: a ringing over a call
   * under way ends — Answer, Decline or the limit — in `stop`, which
   * takes the service, the notification, the sound's mode and the wake
   * lock of the call under way down with it, while the app keeps that
   * call. So nothing rings over a call that goes on; the app answers
   * busy for a second call of two before it rings here, and leaves a
   * call that comes in during a group call to the user's other devices.
   */
  fun appRings(ongoingWith: String?): Boolean = ongoingWith == null
}

/**
 * The calls that ended on this phone a short while ago, by id: a late
 * push for one of them rings nothing (see `RingRules.pushRings`). A few
 * ids for the life of a ringing; the oldest go first.
 */
internal class RecentlyOver(private val keptMs: Long, private val keep: Int = 16) {
  private val ended = ArrayDeque<Pair<String, Long>>()

  /** `callId` ended at `now` (`elapsedRealtime`). */
  fun ended(callId: String, now: Long) {
    ended.removeAll { it.first == callId }
    ended.addLast(callId to now)
    while (ended.size > keep) ended.removeFirst()
  }

  /** Did `callId` end within the last `keptMs` before `now`. */
  fun isOver(callId: String, now: Long): Boolean =
    ended.any { it.first == callId && now - it.second in 0..keptMs }
}

/**
 * The calls on in groups this phone has dealt with, across its processes:
 * the ones whose quiet notification was shown (`SHOWN`), and the ones
 * that went on here, which I was in (`JOINED`). Kept by `Calls` in the
 * app's preferences, so that a process started anew knows them: the app
 * keeps the same in its memory and forgets it when the process dies,
 * while the group announces its call again at every seat taken.
 *
 * A call I was in is no news after I leave it, whatever process hears of
 * it (`isNews`). A notification shown is shown once, until the call is
 * over (`dismissed`) or the next process starts (`leftovers`): the process
 * that showed it may have died with the call still on or over, and the
 * notification would stay in the shade to the user's tap either way; it
 * goes at the start, and the app shows the call anew if it is still on.
 *
 * Times are a wall clock (`currentTimeMillis`): the phone may reboot in
 * between. A few entries, the oldest first; a call id never comes back,
 * so an entry is good until it is pruned or `keptMs` old.
 */
internal class GroupCallMemory(private val keptMs: Long, private val keep: Int = 32) {
  enum class How { SHOWN, JOINED }

  data class Entry(val callId: String, val how: How, val at: Long)

  private val calls = ArrayDeque<Entry>()

  /** The entries, oldest first (for keeping). */
  fun entries(): List<Entry> = calls.toList()

  /** The notification of `callId` was shown at `now`. A call I am in stays as such. */
  fun shown(callId: String, now: Long) = put(callId, How.SHOWN, now)

  /** The call `callId` went on here at `now`: I was in it. */
  fun joined(callId: String, now: Long) = put(callId, How.JOINED, now)

  private fun put(callId: String, how: How, now: Long) {
    val was = calls.firstOrNull { it.callId == callId }
    // In it: the stronger word, kept; a notice shown of a call I was in changes nothing.
    val kept = if (was?.how == How.JOINED) How.JOINED else how
    calls.removeAll { it.callId == callId }
    calls.addLast(Entry(callId, kept, now))
    while (calls.size > keep) calls.removeFirst()
  }

  /**
   * The notice of `callId` goes (the call is over, or I am in it): no
   * longer shown. What I was in stays known: a call I joined is no news
   * after I leave it, and a call that is over never comes back under its
   * id. Answers whether a notice was known.
   */
  fun dismissed(callId: String): Boolean = calls.removeAll { it.callId == callId && it.how == How.SHOWN }

  /**
   * Whether `callId` is news at `now`: neither shown nor joined within
   * `keptMs`. An entry from the future (the clock was set back since) is
   * known still: it is this call's alone, and a notice shown again of a
   * call I was in is the worse mistake.
   */
  fun isNews(callId: String, now: Long): Boolean =
    calls.none { it.callId == callId && now - it.at <= keptMs }

  /**
   * At the start of a process: the notifications a dead process left,
   * which go now and are forgotten as shown (the call, if still on, is
   * news again); what I was in stays known.
   */
  fun leftovers(): List<String> {
    val shown = calls.filter { it.how == How.SHOWN }.map { it.callId }
    calls.removeAll { it.how == How.SHOWN }
    return shown
  }

  /** The entries as one string, for the preferences; `decode` reads it back. */
  fun encode(): String = calls.joinToString("\n") { "${it.callId}\t${it.how.name}\t${it.at}" }

  companion object {
    fun decode(text: String?, keptMs: Long, keep: Int = 32): GroupCallMemory {
      val memory = GroupCallMemory(keptMs, keep)
      if (text.isNullOrEmpty()) return memory
      for (line in text.split('\n')) {
        val parts = line.split('\t')
        if (parts.size != 3) continue
        val how = How.values().firstOrNull { it.name == parts[1] } ?: continue
        val at = parts[2].toLongOrNull() ?: continue
        if (parts[0].isEmpty()) continue
        memory.calls.removeAll { it.callId == parts[0] }
        memory.calls.addLast(Entry(parts[0], how, at))
      }
      while (memory.calls.size > keep) memory.calls.removeFirst()
      return memory
    }
  }
}

/**
 * A ringing notification may outlive the process that rang: the process
 * is killed, and with it the limit of the ringing and what it knew of the
 * call. A press on such a notification comes to a process that knows no
 * call. It is taken only while the ringing could still be live.
 *
 * Times are `SystemClock.elapsedRealtime()`: a change of the clock does not
 * move them, and a reboot (which resets them) takes the notifications away.
 */
internal object RingAge {
  /** Rung at `rungAt` (0 when unknown), is it still within `limitMs` at `now`. */
  fun fresh(rungAt: Long, now: Long, limitMs: Long): Boolean =
    rungAt > 0 && now >= rungAt && now - rungAt <= limitMs

  /**
   * How long a ringing may go on from `now`: until `expiresAt` (0: no
   * expiry known, the invitation's end is the app's to tell) and never
   * longer than `limitMs`. An invitation that expired already gives
   * nothing: the ringing ends at once.
   */
  fun limit(expiresAt: Long, now: Long, limitMs: Long): Long =
    if (expiresAt <= 0) limitMs else (expiresAt - now).coerceIn(0, limitMs)
}

/**
 * What a call shows over the lock screen. The messenger may ask to keep
 * who calls off it (its "hide on lock screen", or a PIN on the app). The
 * notifications then carry a public version without the name and the
 * picture, and the ringing screen shows neither while the phone is locked.
 */
internal object LockScreen {
  /** May the name and the picture of the caller be shown now. */
  fun reveals(hidden: Boolean, locked: Boolean): Boolean = !hidden || !locked
}

/**
 * The rotation the frames of the camera are turned for, following the
 * display. CameraX takes the rotation of the display once, when the use
 * case is made, and the camera lives on a lifecycle of its own that no
 * activity refreshes: a phone turned during a call would send its picture
 * lying on its side until the camera is switched. So the camera's class
 * tells this what the use case was made with (`bound`) and every change
 * of the display (`follow`), and sets the target of the use case to what
 * `follow` answers. Rotations are `Surface.ROTATION_*`.
 */
internal class CameraOrientation {
  private var applied = -1

  /** The use case was made for `rotation`. */
  fun bound(rotation: Int) {
    applied = rotation
  }

  /** The display has `rotation` now: the rotation to set, or null when the use case has it already. */
  fun follow(rotation: Int): Int? = if (rotation == applied) null else rotation.also { applied = it }
}

/**
 * The swipe of the two circles of the ringing screen. Decline stands on the
 * left and is pulled to the right; Answer stands on the right and is pulled
 * to the left: each toward the middle, as far as `maxTravel` (the distance
 * to the middle, so that the two never cross). A circle follows the finger
 * its own way only. Let go at `threshold` or further it acts; short of it
 * it goes back. A press that hardly moved is a tap (`isTap`): it acts on
 * nothing, the screen only shows which way to swipe.
 */
internal object SwipeRule {
  enum class Circle { DECLINE, ANSWER }

  /** The way `circle` goes: -1 to the left, 1 to the right. */
  fun way(circle: Circle): Float = if (circle == Circle.ANSWER) -1f else 1f

  /** Where `circle` stands for a drag of `dx` (pixels, right is positive). */
  fun offset(dx: Float, circle: Circle, maxTravel: Float): Float =
    (dx * way(circle)).coerceIn(0f, maxOf(maxTravel, 0f)) * way(circle)

  /** How far along its way `circle` is, 0 to 1. */
  fun progress(dx: Float, circle: Circle, maxTravel: Float): Float =
    if (maxTravel <= 0f) 0f else Math.abs(offset(dx, circle, maxTravel)) / maxTravel

  /**
   * What a release after a drag of `dx` does: the circle acts, or (null)
   * goes back. A threshold beyond the travel is the travel: a narrow screen
   * still lets the swipe through.
   */
  fun decide(dx: Float, circle: Circle, threshold: Float, maxTravel: Float): Circle? {
    if (maxTravel <= 0f) return null
    val along = Math.abs(offset(dx, circle, maxTravel))
    return if (along > 0f && along >= minOf(threshold, maxTravel)) circle else null
  }

  /** A press whose finger never went `slop` from where it came down. */
  fun isTap(farthest: Float, slop: Float): Boolean = farthest < slop
}

// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.call

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** The decisions of `CallRules.kt`, each against the case that once went wrong. */
class CallRulesTest {
  private class Service

  // ─── ServiceLife: the foreground service of a call ───────────────────────

  /**
   * show(A) then dismiss(A) before the service's onCreate: the start is
   * never stopped from outside (that crash: "did not then call
   * startForeground"); the service comes, finds nothing and ends itself.
   */
  @Test
  fun aStopBeforeTheServiceComesLetsTheStartCome() {
    val life = ServiceLife<Service>()
    assertEquals(ServiceLife.Show.START, life.show())
    life.startAsked()
    assertEquals(ServiceLife.Stop.LET_COME, life.stop())
    // Still on its way: a second stop does not stop it either.
    assertEquals(ServiceLife.Stop.LET_COME, life.stop())
    val service = Service()
    life.created(service)
    // Between onCreate and onStartCommand the start is not kept yet.
    assertEquals(ServiceLife.Stop.LET_COME, life.stop())
    life.commanded()
    life.ending(service)
    assertEquals(ServiceLife.Stop.NOTHING, life.stop())
    life.destroyed(service)
    assertEquals(ServiceLife.Show.START, life.show())
  }

  /** A second show while the start is on its way waits for it; no second start. */
  @Test
  fun aShowWhileTheStartIsOnItsWayWaits() {
    val life = ServiceLife<Service>()
    life.startAsked()
    assertEquals(ServiceLife.Show.WAIT, life.show())
    val service = Service()
    life.created(service)
    assertEquals(ServiceLife.Show.WAIT, life.show())
    life.commanded()
    assertEquals(ServiceLife.Show.REUSE, life.show())
    assertSame(service, life.live())
  }

  /**
   * stop(A) then show(B) before A's onDestroy: B is not given to the
   * ending service (its notification would go with it); a new start is
   * asked for.
   */
  @Test
  fun anEndingServiceIsNotReused() {
    val life = ServiceLife<Service>()
    life.startAsked()
    val old = Service()
    life.created(old)
    life.commanded()
    assertEquals(ServiceLife.Stop.END, life.stop())
    life.ending(old)
    assertNull(life.live())
    assertEquals(ServiceLife.Show.START, life.show())
    life.startAsked()
    // The old one goes, then the new one comes.
    life.destroyed(old)
    assertEquals(ServiceLife.Show.WAIT, life.show())
    val new = Service()
    life.created(new)
    life.commanded()
    assertEquals(ServiceLife.Show.REUSE, life.show())
    assertSame(new, life.live())
  }

  /** The old instance's onDestroy after the new one came leaves the new one alone. */
  @Test
  fun aLateDestroyOfTheOldServiceKeepsTheNewOne() {
    val life = ServiceLife<Service>()
    val old = Service()
    life.startAsked()
    life.created(old)
    life.commanded()
    life.ending(old)
    life.startAsked()
    val new = Service()
    life.created(new)
    life.commanded()
    life.ending(old)
    life.destroyed(old)
    assertSame(new, life.live())
    assertEquals(ServiceLife.Stop.END, life.stop())
  }

  /** A start the system refused leaves nothing on its way. */
  @Test
  fun aRefusedStartLeavesNothing() {
    val life = ServiceLife<Service>()
    assertEquals(ServiceLife.Show.START, life.show())
    // startForegroundService threw: startAsked is never called.
    assertEquals(ServiceLife.Stop.NOTHING, life.stop())
    assertEquals(ServiceLife.Show.START, life.show())
  }

  // ─── RingRules: no ringtone without something to press ───────────────────

  /** Notifications denied, the phone locked: silent. */
  @Test
  fun nothingVisibleInTheBackgroundStaysSilent() {
    assertFalse(RingRules.rings(visible = false, inFront = false))
  }

  @Test
  fun aVisibleNotificationRings() {
    assertTrue(RingRules.rings(visible = true, inFront = false))
    assertTrue(RingRules.rings(visible = true, inFront = true))
  }

  /** Notifications off but the app in front: its page shows the call, and the phone rings for it. */
  @Test
  fun inFrontWithoutNotificationsThePhoneRings() {
    assertTrue(RingRules.rings(visible = false, inFront = true))
  }

  // ─── A late push: the app rang and ended the call before the push came ───

  /**
   * Alice calls, the app (alive in the background) rings and Alice gives
   * up after 5 s; the push for the same invitation comes 8 s later. The
   * phone once rang again for a call that was over, and Answer led
   * nowhere. Over here: not rung. The same call, not over and nothing
   * else on the phone: rung (and `showIncoming` keeps a ringing one).
   */
  @Test
  fun aPushForACallThatEndedHereRingsNothing() {
    assertFalse(RingRules.pushRings("a", over = true, busyWith = null))
    assertTrue(RingRules.pushRings("a", over = false, busyWith = null))
    assertTrue(RingRules.pushRings("a", over = false, busyWith = "a"))
  }

  /** Bob talks with Carol; Alice's invitation comes by push: the app answers busy, the phone is not taken over. */
  @Test
  fun aPushDoesNotTakeThePhoneOverFromAnotherCall() {
    assertFalse(RingRules.pushRings("alice", over = false, busyWith = "carol"))
  }

  /**
   * Bob is in the room of a Trio (its service, sound mode and wake lock
   * on the phone); Alice calls him 1:1 and the app asks to ring it. Once
   * the ringing took the phone over, and its end (Decline, or the limit)
   * stopped everything of the room while the app kept Bob in it. Nothing
   * rings over a call that goes on; with nothing going on, it rings.
   */
  @Test
  fun theAppDoesNotRingOverACallThatGoesOn() {
    assertFalse(RingRules.appRings(ongoingWith = "trio-room"))
    assertTrue(RingRules.appRings(ongoingWith = null))
  }

  /** The ids that ended are kept for the life of a ringing, a few at a time, the oldest forgotten first. */
  @Test
  fun aCallIsOverForTheLifeOfARingingAndNoLonger() {
    val over = RecentlyOver(keptMs = 120_000, keep = 2)
    over.ended("a", now = 1_000)
    assertTrue(over.isOver("a", now = 1_000))
    assertTrue(over.isOver("a", now = 121_000))
    assertFalse(over.isOver("a", now = 121_001))
    assertFalse(over.isOver("b", now = 1_000))
    // A time from before the end (a clock that went back): not over.
    assertFalse(over.isOver("a", now = 999))
    over.ended("b", now = 2_000)
    over.ended("c", now = 3_000)
    assertFalse("the oldest made room", over.isOver("a", now = 3_000))
    assertTrue(over.isOver("b", now = 3_000))
    // Ended again: the time is the later one, and the id is kept once.
    over.ended("b", now = 4_000)
    assertTrue(over.isOver("b", now = 124_000))
    assertTrue(over.isOver("c", now = 4_000))
  }

  // ─── GroupCallMemory: the calls of groups across the processes ───────────

  /**
   * Bob joins the call of the Trio and leaves; the system kills the app;
   * the call goes on and somebody takes a seat, which the group announces
   * again: the new process, which had nothing in its memory, once showed
   * "Trio · A call is on" to the man who had just left it. The phone
   * remembers what went on here, as the preferences keep it; a notice
   * dismissed (he joins) does not take that away.
   */
  @Test
  fun aCallIWasInIsNoNewsAfterTheProcessDied() {
    val memory = GroupCallMemory(keptMs = 24 * 3_600_000L)
    assertTrue(memory.isNews("trio", now = 1_000))
    memory.joined("trio", now = 1_000)
    assertFalse(memory.isNews("trio", now = 2_000))
    // The next process reads what the last one wrote.
    val next = GroupCallMemory.decode(memory.encode(), keptMs = 24 * 3_600_000L)
    assertFalse(next.isNews("trio", now = 3_600_000L))
    assertFalse("the notice dismissed as I am in it: still no news", next.dismissed("trio"))
    assertFalse(next.isNews("trio", now = 3_600_000L))
    // A notice shown of a call I was in does not weaken the word; the call is on still, and the time moves on.
    next.shown("trio", now = 4_000_000L)
    assertEquals(listOf(GroupCallMemory.How.JOINED), next.entries().map { it.how })
    assertFalse(next.isNews("trio", now = 4_000_000L + 24 * 3_600_000L))
    assertTrue("the next day, the id is nobody's", next.isNews("trio", now = 4_000_000L + 24 * 3_600_000L + 1))
    assertFalse("a clock gone back: known still", next.isNews("trio", now = 500))
  }

  /**
   * The notice of a call is shown and the process dies; the call ends
   * while no process lives, or goes on. The notice once stayed in the
   * shade to the user's tap. At the next start it is a leftover: it goes,
   * and the call is news again (the app shows it anew if it is still on).
   * What I was in is not a leftover. Nothing read: nothing to do.
   */
  @Test
  fun aNoticeOfADeadProcessIsALeftover() {
    val memory = GroupCallMemory(keptMs = 24 * 3_600_000L)
    memory.shown("quartet", now = 1_000)
    memory.joined("trio", now = 2_000)
    assertFalse(memory.isNews("quartet", now = 3_000))
    assertEquals(listOf("quartet"), memory.leftovers())
    assertTrue(memory.isNews("quartet", now = 3_000))
    assertFalse(memory.isNews("trio", now = 3_000))
    assertTrue(memory.leftovers().isEmpty())
    assertTrue(GroupCallMemory.decode(null, keptMs = 1).leftovers().isEmpty())
    assertTrue(GroupCallMemory.decode("", keptMs = 1).leftovers().isEmpty())
  }

  /** The notice shown once is dismissed once, the ids are kept a few at a time, and a line that cannot be read is skipped. */
  @Test
  fun theMemoryOfGroupCallsStaysShortAndReadsWhatItCan() {
    val memory = GroupCallMemory(keptMs = 10_000, keep = 2)
    memory.shown("a", now = 1_000)
    assertTrue(memory.dismissed("a"))
    assertFalse("dismissed once", memory.dismissed("a"))
    assertTrue(memory.isNews("a", now = 1_000))
    memory.shown("a", now = 1_000)
    memory.shown("b", now = 2_000)
    memory.shown("c", now = 3_000)
    assertEquals(listOf("b", "c"), memory.entries().map { it.callId })
    assertTrue("the oldest made room", memory.isNews("a", now = 3_000))
    // Shown again: kept once, at the later time.
    memory.shown("b", now = 4_000)
    assertEquals(listOf("c", "b"), memory.entries().map { it.callId })
    assertFalse(memory.isNews("b", now = 14_000))
    assertTrue(memory.isNews("b", now = 14_001))
    val read = GroupCallMemory.decode("x\tSHOWN\t5\nbroken line\ny\tWHAT\t6\nz\tJOINED\tsoon\n\tSHOWN\t7\nw\tJOINED\t8", keptMs = 10_000)
    assertEquals(listOf("x" to GroupCallMemory.How.SHOWN, "w" to GroupCallMemory.How.JOINED), read.entries().map { it.callId to it.how })
    assertEquals(read.encode(), GroupCallMemory.decode(read.encode(), keptMs = 10_000).encode())
  }

  // ─── RingAge: a press that outlived the process that rang ────────────────

  @Test
  fun aPressWithinTheLimitIsLive() {
    assertTrue(RingAge.fresh(rungAt = 1_000, now = 1_000, limitMs = 120_000))
    assertTrue(RingAge.fresh(rungAt = 1_000, now = 121_000, limitMs = 120_000))
  }

  /** Answer hours later on a notification a dead process left: dropped. */
  @Test
  fun aPressAfterTheLimitIsStale() {
    assertFalse(RingAge.fresh(rungAt = 1_000, now = 121_001, limitMs = 120_000))
    assertFalse(RingAge.fresh(rungAt = 1_000, now = 1_000 + 3 * 60 * 60 * 1000L, limitMs = 120_000))
  }

  /** No time known, or a time from before a reboot: never live. */
  @Test
  fun anUnknownOrFutureRingIsStale() {
    assertFalse(RingAge.fresh(rungAt = 0, now = 5_000, limitMs = 120_000))
    assertFalse(RingAge.fresh(rungAt = 90_000, now = 5_000, limitMs = 120_000))
  }

  /** A call rung by a push stops ringing when its invitation expires, and never later than the limit. */
  @Test
  fun aRingingEndsAtTheExpiryOfTheInvitation() {
    assertEquals(120_000, RingAge.limit(expiresAt = 0, now = 5_000, limitMs = 120_000))
    assertEquals(40_000, RingAge.limit(expiresAt = 45_000, now = 5_000, limitMs = 120_000))
    assertEquals(120_000, RingAge.limit(expiresAt = 500_000, now = 5_000, limitMs = 120_000))
    // Expired on the way: nothing to ring for.
    assertEquals(0, RingAge.limit(expiresAt = 4_000, now = 5_000, limitMs = 120_000))
  }

  // ─── LockScreen: who calls, on a locked phone ────────────────────────────

  @Test
  fun aHiddenCallerIsNotShownWhileLocked() {
    assertFalse(LockScreen.reveals(hidden = true, locked = true))
  }

  @Test
  fun aHiddenCallerIsShownOnceUnlocked() {
    assertTrue(LockScreen.reveals(hidden = true, locked = false))
  }

  @Test
  fun aCallerNotHiddenIsShown() {
    assertTrue(LockScreen.reveals(hidden = false, locked = true))
    assertTrue(LockScreen.reveals(hidden = false, locked = false))
  }

  // ─── CameraOrientation: the frames follow the phone's turn ───────────────

  /**
   * A video call started in portrait, the phone turned to landscape
   * mid-call: the target of the use case follows the display (the peer
   * once saw the picture on its side until the camera was switched); the
   * display reporting the same rotation again changes nothing.
   */
  @Test
  fun aTurnOfThePhoneRetargetsTheCamera() {
    val o = CameraOrientation()
    o.bound(0)
    assertNull(o.follow(0))
    assertEquals(1, o.follow(1))
    assertNull(o.follow(1))
    assertEquals(0, o.follow(0))
  }

  /** A switch of the camera makes the use case anew, with the rotation the display has then. */
  @Test
  fun aRebindTakesTheRotationOfTheDisplay() {
    val o = CameraOrientation()
    o.bound(0)
    assertEquals(3, o.follow(3))
    o.bound(3)
    assertNull(o.follow(3))
    o.bound(0)
    assertEquals(3, o.follow(3))
  }
}

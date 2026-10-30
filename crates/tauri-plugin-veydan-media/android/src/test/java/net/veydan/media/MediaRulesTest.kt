// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The decisions of `MediaRules.kt`: what the answers of the system mean for the grid. */
class MediaRulesTest {
  private fun granted(vararg p: String): (String) -> Boolean = { it in p }

  // ─── The state of the access on each version ─────────────────────────────

  /** Android 14+: «Разрешить все» is full, «Разрешить выбранные» partial, «Не разрешать» denied. */
  @Test
  fun android14MapsItsThreeAnswers() {
    val p = MediaPermissions
    assertEquals(MediaAccess.FULL, p.access(34, granted(p.IMAGES, p.VIDEO, p.USER_SELECTED)))
    assertEquals(MediaAccess.FULL, p.access(35, granted(p.IMAGES, p.VIDEO)))
    assertEquals(MediaAccess.PARTIAL, p.access(34, granted(p.USER_SELECTED)))
    assertEquals(MediaAccess.PARTIAL, p.access(36, granted(p.USER_SELECTED)))
    assertEquals(MediaAccess.DENIED, p.access(34, granted()))
    // The storage of old phones means nothing here.
    assertEquals(MediaAccess.DENIED, p.access(34, granted(p.STORAGE)))
  }

  /** Android 13: photos and videos are asked apart; one of them shows a part. No «выбранные» there. */
  @Test
  fun android13HasTwoPermissions() {
    val p = MediaPermissions
    assertEquals(MediaAccess.FULL, p.access(33, granted(p.IMAGES, p.VIDEO)))
    assertEquals(MediaAccess.PARTIAL, p.access(33, granted(p.IMAGES)))
    assertEquals(MediaAccess.PARTIAL, p.access(33, granted(p.VIDEO)))
    assertEquals(MediaAccess.DENIED, p.access(33, granted(p.USER_SELECTED)))
    assertEquals(MediaAccess.DENIED, p.access(33, granted(p.STORAGE)))
  }

  /** Android 12 and older: the storage alone (`maxSdkVersion=32` in the manifests). */
  @Test
  fun android12AndOlderReadTheStorage() {
    val p = MediaPermissions
    assertEquals(MediaAccess.FULL, p.access(32, granted(p.STORAGE)))
    assertEquals(MediaAccess.FULL, p.access(26, granted(p.STORAGE)))
    assertEquals(MediaAccess.DENIED, p.access(30, granted(p.IMAGES, p.VIDEO)))
    assertEquals(MediaAccess.DENIED, p.access(26, granted()))
  }

  @Test
  fun eachVersionAsksWhatItsManifestDeclares() {
    val p = MediaPermissions
    assertArrayEquals(arrayOf(p.IMAGES, p.VIDEO, p.USER_SELECTED), p.asked(34))
    assertArrayEquals(arrayOf(p.IMAGES, p.VIDEO), p.asked(33))
    assertArrayEquals(arrayOf(p.STORAGE), p.asked(32))
  }

  @Test
  fun theWordsAreThoseOfThePage() {
    assertEquals(listOf("full", "partial", "denied"), MediaAccess.values().map { it.word })
  }

  // ─── Ask, or open the settings ──────────────────────────────────────────

  /**
   * Never asked: the question. Refused once: the system still asks (with a
   * rationale). Refused for good («не спрашивать»): no question can be
   * shown, the app's settings open. Partial: asked again, to choose more.
   */
  @Test
  fun aRefusalForGoodOpensTheSettings() {
    val p = MediaPermissions
    assertTrue(p.mayAsk(MediaAccess.DENIED, askedBefore = false, rationale = false))
    assertTrue(p.mayAsk(MediaAccess.DENIED, askedBefore = true, rationale = true))
    assertFalse(p.mayAsk(MediaAccess.DENIED, askedBefore = true, rationale = false))
    assertTrue(p.mayAsk(MediaAccess.PARTIAL, askedBefore = true, rationale = false))
    assertFalse(p.mayAsk(MediaAccess.FULL, askedBefore = true, rationale = false))
  }

  // ─── Pages ───────────────────────────────────────────────────────────────

  @Test
  fun aCursorGoesThereAndBack() {
    val c = PageCursor(1_730_000_000, 1_000_012_345)
    assertEquals("1730000000:1000012345", c.word())
    assertEquals(c, PageCursor.of(c.word()))
    assertArrayEquals(arrayOf("1730000000", "1730000000", "1000012345"), c.args())
    assertEquals("(date_added < ? OR (date_added = ? AND _id < ?))", c.selection("date_added", "_id"))
  }

  /** What is not a cursor starts from the newest, never a query of a page's words. */
  @Test
  fun aWordThatIsNoCursorIsTheFirstPage() {
    for (w in listOf(null, "", "1", "1:2:3", "a:1", "1:b", "-1:5", "1; DROP TABLE files:2")) {
      assertNull(w, PageCursor.of(w))
    }
  }

  @Test
  fun aPageIsBounded() {
    assertEquals(1, pageLimit(0))
    assertEquals(60, pageLimit(60))
    assertEquals(MAX_PAGE, pageLimit(100_000))
  }

  // ─── Sizes ───────────────────────────────────────────────────────────────

  @Test
  fun aPhotoWithinTheBoundKeepsItsSize() {
    assertEquals(Pair(4032, 3024), targetSize(4032, 3024, 50_000_000))
  }

  /** A 200 MP photo is made 50 MP, its proportions kept. */
  @Test
  fun aLargerPhotoIsMadeSmallerToTheBound() {
    val (w, h) = targetSize(16_320, 12_240, 50_000_000)
    assertTrue(w.toLong() * h <= 50_000_000)
    assertTrue(w.toLong() * h > 49_000_000)
    assertEquals(16_320.0 / 12_240.0, w.toDouble() / h, 0.01)
  }

  @Test
  fun aThumbnailIsOfATileOrOfTheViewer() {
    assertEquals(64, thumbSide(1))
    assertEquals(330, thumbSide(330))
    assertEquals(1280, thumbSide(10_000))
  }

  // ─── The thumbnails on the disk ──────────────────────────────────────────

  /** Past the bound the least recently shown go, down to the low mark; one shown again stays. */
  @Test
  fun theLeastRecentlyShownThumbnailsGoFirst() {
    val lru = ThumbLru(limit = 100, low = 60)
    assertTrue(lru.add("a", 30).isEmpty())
    assertTrue(lru.add("b", 30).isEmpty())
    assertTrue(lru.add("c", 30).isEmpty())
    assertTrue(lru.touch("a"))
    assertEquals(listOf("b", "c"), lru.add("d", 30))
    assertEquals(60, lru.total)
    assertTrue(lru.touch("a"))
    assertFalse(lru.touch("b"))
  }

  @Test
  fun aThumbnailWrittenAgainIsCountedOnce() {
    val lru = ThumbLru(limit = 100, low = 60)
    lru.add("a", 40)
    lru.add("a", 50)
    assertEquals(50, lru.total)
    lru.clear()
    assertEquals(0, lru.total)
    assertFalse(lru.touch("a"))
  }

  /**
   * An item the app may no longer read (a partial access changed, A → B):
   * every thumbnail of it leaves at once, the tile's and the viewer's; the
   * others stay, and one pushed out before is not named again.
   */
  @Test
  fun theThumbnailsOfAnItemOutOfTheAccessGoTogether() {
    val a = "content://media/external/images/media/1"
    val b = "content://media/external/video/media/2"
    val lru = ThumbLru(limit = 100, low = 60)
    lru.add("a-tile", 10, a)
    lru.add("a-viewer", 30, a)
    lru.add("b-tile", 10, b)
    assertEquals(setOf("a-tile", "a-viewer"), lru.forget(a).toSet())
    assertEquals(10, lru.total)
    assertFalse(lru.touch("a-tile"))
    assertFalse(lru.touch("a-viewer"))
    assertTrue(lru.touch("b-tile"))
    assertTrue(lru.forget(a).isEmpty())

    // Pushed out by the bound: forgotten by its item too.
    lru.add("c", 95, "c")
    assertTrue(lru.forget(b).isEmpty())
    assertEquals(95, lru.total)
  }

  /**
   * The choice is checked item by item, whatever album or page the grid
   * shows: only those MediaStore no longer gives leave; what is not of the
   * gallery is never taken for readable; a list past the bound asks nothing.
   */
  @Test
  fun theChoiceIsCheckedItemByItem() {
    val a = "content://media/external/images/media/1"
    val b = "content://media/external/video/media/2"
    val c = "content://media/external/images/media/3"
    val asked = ArrayList<String>()
    val now = setOf(a, c)
    val ok = stillReadable(listOf(a, b, "content://net.veydan.mobile.fileprovider/attach-capture/x.jpg", "/data/x.jpg", c)) {
      asked.add(it)
      it in now
    }
    assertEquals(listOf(a, c), ok)
    assertEquals(listOf(a, b, c), asked)
    assertTrue(stillReadable(emptyList()) { true }.isEmpty())
    var any = false
    assertTrue(stillReadable(List(MAX_READABLE + 1) { a }) { any = true; true }.isEmpty())
    assertFalse(any)
  }
}

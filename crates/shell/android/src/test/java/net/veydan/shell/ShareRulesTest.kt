// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.shell

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ShareRulesTest {
  private val ext = { mime: String -> mapOf("image/jpeg" to "jpg", "video/mp4" to "mp4", "application/pdf" to "pdf")[mime] }

  @Test
  fun `a shared file keeps the last segment of its name, cleaned`() {
    assertEquals("IMG_1.jpg", ShareRules.fileName("IMG_1.jpg", "image/jpeg", 0, ext))
    assertEquals("IMG_1.jpg", ShareRules.fileName("DCIM/Camera/IMG_1.jpg", "image/jpeg", 0, ext))
    assertEquals("photo.jpg", ShareRules.fileName("..\\..\\photo.jpg", null, 0, ext))
    assertEquals("ab.txt", ShareRules.fileName("a\u0000b.txt\n", null, 0, ext))
    assertEquals("Фото 写真.HEIC", ShareRules.fileName("  Фото 写真.HEIC ", null, 0, ext))
  }

  @Test
  fun `a file without a usable name is named by its place and kind`() {
    assertEquals("file-1.jpg", ShareRules.fileName(null, "image/jpeg", 0, ext))
    assertEquals("file-3.mp4", ShareRules.fileName("", "video/mp4", 2, ext))
    assertEquals("file-1", ShareRules.fileName(".hidden", null, 0, ext))
    assertEquals("file-1", ShareRules.fileName("..", null, 0, ext))
    assertEquals("file-2", ShareRules.fileName("/", "x/unknown", 1, ext))
  }

  @Test
  fun `a long name is cut`() {
    val name = ShareRules.fileName("a".repeat(400) + ".jpg", null, 0, ext)
    assertEquals(255, name.length)
  }

  @Test
  fun `the mime is the file's, else the share's when it names one kind`() {
    assertEquals("image/jpeg", ShareRules.mimeOf("image/jpeg", "image/*"))
    assertEquals("image/png", ShareRules.mimeOf(null, "image/png"))
    assertNull(ShareRules.mimeOf("image/*", "*/*"))
    assertNull(ShareRules.mimeOf("", null))
  }

  @Test
  fun `an inbox folder a day old is stale, a fresh one and one from the future are not and are`() {
    val now = 1_700_000_000_000L
    assertFalse(ShareRules.stale(ShareRules.folderName(now), now))
    assertFalse(ShareRules.stale(ShareRules.folderName(now - ShareRules.STALE_MS + 1), now))
    assertTrue(ShareRules.stale(ShareRules.folderName(now - ShareRules.STALE_MS - 1), now))
    // A clock set back: a folder far in the future is garbage too.
    assertTrue(ShareRules.stale(ShareRules.folderName(now + 2 * ShareRules.STALE_MS), now))
    assertTrue(ShareRules.stale("not-a-stamp", now))
    assertTrue(ShareRules.stale("", now))
  }

  @Test
  fun `the text is trimmed, cut and dropped when blank`() {
    assertNull(ShareRules.textOf(null))
    assertNull(ShareRules.textOf("  \n"))
    assertEquals("hello", ShareRules.textOf("  hello \n"))
    val long = "я".repeat(ShareRules.MAX_TEXT + 10)
    assertEquals(ShareRules.MAX_TEXT, ShareRules.textOf(long)!!.length)
    // Cut on a character: a surrogate pair is not split.
    val pairs = "😀".repeat(ShareRules.MAX_TEXT / 2 + 5)
    val cut = ShareRules.textOf(pairs)!!
    assertFalse(Character.isHighSurrogate(cut.last()))
  }

  @Test
  fun `the subject is kept only when it says more than the text`() {
    assertNull(ShareRules.subjectOf(null, "a"))
    assertNull(ShareRules.subjectOf("a", "a"))
    assertEquals("A title", ShareRules.subjectOf(" A title ", "the text"))
  }
}

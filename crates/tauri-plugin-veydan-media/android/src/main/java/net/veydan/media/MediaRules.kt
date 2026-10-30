// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.media

/**
 * The decisions of the gallery that need no phone: which permissions a
 * version of Android asks for, what their answers mean for the grid,
 * where a page of the grid starts, how large a converted photo may be.
 * Pure Kotlin, tested on the JVM (src/test).
 */

/** What the grid may show: everything, what the user chose (Android 14+), nothing. */
enum class MediaAccess(val word: String) {
  FULL("full"),
  PARTIAL("partial"),
  DENIED("denied"),
}

object MediaPermissions {
  const val IMAGES = "android.permission.READ_MEDIA_IMAGES"
  const val VIDEO = "android.permission.READ_MEDIA_VIDEO"
  const val USER_SELECTED = "android.permission.READ_MEDIA_VISUAL_USER_SELECTED"
  const val STORAGE = "android.permission.READ_EXTERNAL_STORAGE"

  /** Android 14: «Разрешить выбранные» (READ_MEDIA_VISUAL_USER_SELECTED). */
  const val SDK_SELECTED = 34
  /** Android 13: photos and videos are two permissions instead of the storage. */
  const val SDK_MEDIA = 33

  /** The permissions the grid asks for on `sdk`, as the manifests declare them. */
  fun asked(sdk: Int): Array<String> = when {
    sdk >= SDK_SELECTED -> arrayOf(IMAGES, VIDEO, USER_SELECTED)
    sdk >= SDK_MEDIA -> arrayOf(IMAGES, VIDEO)
    else -> arrayOf(STORAGE)
  }

  /**
   * What the grid may show, from what is granted. On Android 14 a user who
   * chose «Разрешить выбранные» gets only USER_SELECTED; on 13 photos
   * without videos (or the other way) shows a part as well.
   */
  fun access(sdk: Int, granted: (String) -> Boolean): MediaAccess = when {
    sdk >= SDK_MEDIA && granted(IMAGES) && granted(VIDEO) -> MediaAccess.FULL
    sdk >= SDK_MEDIA && (granted(IMAGES) || granted(VIDEO)) -> MediaAccess.PARTIAL
    sdk >= SDK_SELECTED && granted(USER_SELECTED) -> MediaAccess.PARTIAL
    sdk < SDK_MEDIA && granted(STORAGE) -> MediaAccess.FULL
    else -> MediaAccess.DENIED
  }

  /**
   * Whether a tap on «Разрешить доступ» can show the system's question.
   * Never asked: yes. Asked and refused once: the system shows a rationale
   * and asks again. Refused for good («Больше не спрашивать», or twice on
   * Android 11+): the system answers at once without a question, so the
   * app's settings are opened instead. A partial access is asked again to
   * choose more (Android 14 shows its chooser each time).
   */
  fun mayAsk(access: MediaAccess, askedBefore: Boolean, rationale: Boolean): Boolean = when (access) {
    MediaAccess.FULL -> false
    MediaAccess.PARTIAL -> true
    MediaAccess.DENIED -> !askedBefore || rationale
  }
}

/**
 * Where a page of the newest photos and videos goes on: after the last item
 * of the page before, by the time it was added and then by its id (two
 * shots of the same second keep their order). Opaque to the page.
 */
data class PageCursor(val added: Long, val id: Long) {
  fun word(): String = "$added:$id"

  companion object {
    /** A cursor the page gave back; `null` for the first page or a word that is not one. */
    fun of(word: String?): PageCursor? {
      val parts = word?.split(':') ?: return null
      if (parts.size != 2) return null
      val added = parts[0].toLongOrNull() ?: return null
      val id = parts[1].toLongOrNull() ?: return null
      if (added < 0 || id < 0) return null
      return PageCursor(added, id)
    }
  }

  /** The SQL that takes what comes after this cursor in the order `added DESC, id DESC`. */
  fun selection(addedColumn: String, idColumn: String): String =
    "($addedColumn < ? OR ($addedColumn = ? AND $idColumn < ?))"

  fun args(): Array<String> = arrayOf(added.toString(), added.toString(), id.toString())
}

/** The most items a page has: what the grid asks is bounded. */
const val MAX_PAGE = 300

fun pageLimit(asked: Int): Int = asked.coerceIn(1, MAX_PAGE)

/**
 * The size a photo is decoded at to stay within `maxPixels`: its own when
 * it fits, else both sides scaled by the same factor, rounded down.
 */
fun targetSize(width: Int, height: Int, maxPixels: Long): Pair<Int, Int> {
  if (width <= 0 || height <= 0) return Pair(width, height)
  val pixels = width.toLong() * height.toLong()
  if (pixels <= maxPixels) return Pair(width, height)
  val scale = Math.sqrt(maxPixels.toDouble() / pixels.toDouble())
  var w = Math.floor(width * scale).toInt().coerceAtLeast(1)
  var h = Math.floor(height * scale).toInt().coerceAtLeast(1)
  // Rounding never takes it over the bound.
  while (w.toLong() * h.toLong() > maxPixels) {
    if (w >= h) w-- else h--
  }
  return Pair(w, h)
}

/** The most addresses `readable` checks at once: past it, none is taken for readable. */
const val MAX_READABLE = 300

/**
 * Which of `uris` the app may still read, in their order: an item of the
 * gallery (`content://media/`) whose row MediaStore gives now (`ask`).
 * Anything else is not of the gallery and is never taken for readable;
 * past `MAX_READABLE` nothing is asked at all.
 */
fun stillReadable(uris: List<String>, ask: (String) -> Boolean): List<String> {
  if (uris.size > MAX_READABLE) return emptyList()
  return uris.filter { it.startsWith("content://media/") && ask(it) }
}

/** The side of a thumbnail asked for, bounded: a tile of the grid, or the large one of the viewer. */
fun thumbSide(asked: Int): Int = asked.coerceIn(64, 1280)

/**
 * The thumbnails kept on the disk, the least recently used going first
 * once they take more than `limit` bytes, down to `low`. Only names and
 * sizes: the files are the caller's. Each name is of an item (`owner`, its
 * `content://` address): all of an item go at once when the app may no
 * longer read it (`forget`).
 */
class ThumbLru(private val limit: Long, private val low: Long) {
  private val order = LinkedHashMap<String, Long>(64, 0.75f, true)
  private val owners = HashMap<String, String>()
  var total = 0L
    private set

  @Synchronized
  fun touch(name: String): Boolean = order[name] != null

  /** A thumbnail of `owner` written; the names to delete now, oldest first. */
  @Synchronized
  fun add(name: String, bytes: Long, owner: String = name): List<String> {
    order.remove(name)?.let { total -= it }
    order[name] = bytes
    owners[name] = owner
    total += bytes
    if (total <= limit) return emptyList()
    val gone = ArrayList<String>()
    val it = order.entries.iterator()
    while (total > low && it.hasNext()) {
      val e = it.next()
      if (e.key == name) continue
      total -= e.value
      gone.add(e.key)
      owners.remove(e.key)
      it.remove()
    }
    return gone
  }

  /** Every thumbnail of `owner` leaves; their names, for the caller to delete. */
  @Synchronized
  fun forget(owner: String): List<String> {
    val gone = owners.filterValues { it == owner }.keys.toList()
    for (name in gone) {
      order.remove(name)?.let { total -= it }
      owners.remove(name)
    }
    return gone
  }

  @Synchronized
  fun clear() {
    order.clear()
    owners.clear()
    total = 0
  }
}
